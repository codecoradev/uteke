//! Running a decoded update request (#1751, after #1516, #1343 and #1439).
//!
//! The CLI, `PUT /memory` and `uteke_update` each hand-wrote the same
//! sequence: parse the caller's validity, check it, run the field update,
//! write the validity only after the fields went through (#1686), then move
//! the namespace. The ordering is an invariant of the interface, so it lives
//! here once. A transport decodes its input into an [`UpdateRequest`], calls
//! [`Uteke::run_update`] and renders the [`UpdateRun`] or the [`Error`].
//!
//! Order, and what a failure leaves behind:
//! 1. [`UpdateRequest::decode`] parses the validity (a bad interval is a
//!    validation error) and refuses a request that changes nothing.
//! 2. The validity is checked against the stored memory (missing memory,
//!    deprecated memory, interval that would not start before it ends)
//!    before anything is written, so a refused interval changes nothing.
//! 3. The field update runs (skipped when the request is validity only). A
//!    refused field leaves the validity untouched.
//! 4. The validity is written, then the namespace is moved. Both come after
//!    the fields, so a failure there is reported as an error with the earlier
//!    steps already stored; it is never retried or rolled back.
//!
//! A memory that does not exist is [`Error::NotFound`] with the text
//! `Memory not found: <id>` at every step.

use crate::memory::types::Validity;
use crate::privacy::RedactionCount;
use crate::{Error, Uteke};

/// The fields of an update as a transport decoded them, before validation.
#[derive(Debug, Clone, Default)]
pub struct UpdateInput {
    pub id: String,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
    pub importance: Option<f64>,
    pub pinned: Option<bool>,
    pub memory_type: Option<String>,
    pub namespace: Option<String>,
    /// RFC3339, parsed by [`UpdateRequest::decode`].
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

/// A decoded update: the validity is parsed and the request changes something.
#[derive(Debug, Clone)]
pub struct UpdateRequest {
    pub id: String,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
    pub importance: Option<f64>,
    pub pinned: Option<bool>,
    pub memory_type: Option<String>,
    pub namespace: Option<String>,
    pub validity: Option<Validity>,
}

impl UpdateRequest {
    /// Parse the validity and refuse an update that changes nothing. Field
    /// values (importance range, memory type, content) are validated when the
    /// update runs, against the stored memory.
    pub fn decode(input: UpdateInput) -> Result<Self, Error> {
        let validity = Validity::parse(input.valid_from.as_deref(), input.valid_until.as_deref())?;
        let req = Self {
            id: input.id,
            content: input.content,
            tags: input.tags,
            metadata: input.metadata,
            importance: input.importance,
            pinned: input.pinned,
            memory_type: input.memory_type,
            namespace: input.namespace,
            validity,
        };
        if !req.has_fields() && req.namespace.is_none() && req.validity.is_none() {
            return Err(Error::Validation(
                "No fields to update. Provide at least one of: content, tags, metadata, importance, pinned, memory_type, namespace, valid_from, valid_until"
                    .to_string(),
            ));
        }
        Ok(req)
    }

    /// Whether any field the field update writes is set (everything but the
    /// namespace and the validity).
    fn has_fields(&self) -> bool {
        self.content.is_some()
            || self.tags.is_some()
            || self.metadata.is_some()
            || self.importance.is_some()
            || self.pinned.is_some()
            || self.memory_type.is_some()
    }

    /// The request sets a validity and nothing else the field update writes.
    pub fn only_validity(&self) -> bool {
        self.validity.is_some() && !self.has_fields()
    }
}

/// What a successful update did.
#[derive(Debug, Clone, Default)]
pub struct UpdateRun {
    /// What the privacy filter masked in the new content, tags or metadata
    /// (#1464). Empty for a validity-only update.
    pub redactions: Vec<RedactionCount>,
}

impl UpdateRun {
    /// Whether the privacy filter masked anything in this update.
    pub fn redacted(&self) -> bool {
        !self.redactions.is_empty()
    }

    /// What was masked, for a human (`github-token x1`).
    pub fn redaction_summary(&self) -> Option<String> {
        (!self.redactions.is_empty()).then(|| {
            self.redactions
                .iter()
                .map(|r| format!("{} x{}", r.kind, r.count))
                .collect::<Vec<_>>()
                .join(", ")
        })
    }
}

fn missing(id: &str) -> Error {
    Error::not_found(format!("Memory not found: {id}"))
}

impl Uteke {
    /// Run `req`: check the validity, update the fields, then write the
    /// validity and move the namespace. See the module docs for the order.
    pub fn run_update(&self, req: &UpdateRequest) -> Result<UpdateRun, Error> {
        let id = req.id.as_str();
        if let Some(v) = &req.validity
            && !self.check_validity(id, v)?
        {
            return Err(missing(id));
        }

        let redactions = if req.only_validity() {
            Vec::new()
        } else {
            let outcome = self.update_memory_detailed(
                id,
                req.content.as_deref(),
                req.tags.as_deref(),
                req.metadata.as_ref(),
                req.importance,
                req.pinned,
                req.memory_type.as_deref(),
            )?;
            if !outcome.updated {
                return Err(missing(id));
            }
            outcome.redactions
        };

        if let Some(v) = &req.validity
            && !self.set_validity(id, v)?
        {
            return Err(missing(id));
        }
        // Namespace move (#1181): a plain column change after the fields, no
        // re-embed needed.
        if let Some(ns) = req.namespace.as_deref()
            && !self.move_memory(id, ns)?
        {
            return Err(missing(id));
        }
        Ok(UpdateRun { redactions })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::open_keyword;

    fn input(id: &str, f: impl FnOnce(&mut UpdateInput)) -> UpdateInput {
        let mut i = UpdateInput {
            id: id.to_string(),
            ..Default::default()
        };
        f(&mut i);
        i
    }

    fn run(u: &Uteke, id: &str, f: impl FnOnce(&mut UpdateInput)) -> Result<UpdateRun, Error> {
        u.run_update(&UpdateRequest::decode(input(id, f))?)
    }

    fn seed(u: &Uteke) -> String {
        u.remember("update probe content", &["t"], None, None)
            .unwrap()
    }

    const FROM: &str = "2025-01-01T00:00:00Z";
    const UNTIL: &str = "2099-01-01T00:00:00Z";

    #[test]
    fn decode_refuses_a_bad_interval_and_an_empty_request() {
        assert!(matches!(
            UpdateRequest::decode(input("x", |i| i.valid_from = Some("soon".into()))),
            Err(Error::Validation(_))
        ));
        assert!(matches!(
            UpdateRequest::decode(input("x", |i| {
                i.valid_from = Some(UNTIL.into());
                i.valid_until = Some(FROM.into());
            })),
            Err(Error::Validation(_))
        ));
        let e = UpdateRequest::decode(input("x", |_| {})).unwrap_err();
        assert!(e.to_string().contains("No fields to update"), "{e}");
    }

    #[test]
    fn fields_and_validity_land_together() {
        let u = open_keyword();
        let id = seed(&u);
        run(&u, &id, |i| {
            i.importance = Some(0.9);
            i.pinned = Some(true);
            i.valid_from = Some(FROM.into());
            i.valid_until = Some(UNTIL.into());
        })
        .unwrap();
        let m = u.get_by_id(&id).unwrap().unwrap();
        assert!((m.importance - 0.9).abs() < 1e-9 && m.pinned);
        assert_eq!(
            m.valid_from.unwrap().to_rfc3339(),
            "2025-01-01T00:00:00+00:00"
        );
        assert_eq!(
            m.valid_until.unwrap().to_rfc3339(),
            "2099-01-01T00:00:00+00:00"
        );
    }

    /// #1686: a field the update refuses must not leave a new validity behind.
    #[test]
    fn a_refused_field_leaves_the_validity_unchanged() {
        let u = open_keyword();
        let id = seed(&u);
        for f in [
            (|i: &mut UpdateInput| i.importance = Some(5.0)) as fn(&mut UpdateInput),
            |i| i.memory_type = Some("nonsense".into()),
        ] {
            let err = run(&u, &id, |i| {
                f(i);
                i.valid_until = Some(UNTIL.into());
            })
            .unwrap_err();
            assert!(matches!(err, Error::Validation(_)), "{err}");
            let m = u.get_by_id(&id).unwrap().unwrap();
            assert!(m.valid_until.is_none(), "validity written despite {err}");
        }
    }

    /// The validity is checked before the field update: a refused interval
    /// (here a deprecated memory) leaves the fields untouched too.
    #[test]
    fn a_refused_validity_leaves_the_fields_unchanged() {
        let u = open_keyword();
        let id = seed(&u);
        u.store.deprecate(&id).unwrap();
        let before = u.get_by_id(&id).unwrap().unwrap();
        let err = run(&u, &id, |i| {
            i.content = Some("new content".into());
            i.importance = Some(0.9);
            i.valid_until = Some(UNTIL.into());
        })
        .unwrap_err();
        assert!(err.to_string().contains("deprecated"), "{err}");
        let m = u.get_by_id(&id).unwrap().unwrap();
        assert_eq!(m.content, "update probe content");
        assert!((m.importance - 0.9).abs() > 1e-9);
        assert_eq!(m.valid_until, before.valid_until);

        // An interval that does not fit the stored one is refused the same way.
        let id2 = seed_other(&u);
        run(&u, &id2, |i| i.valid_from = Some(FROM.into())).unwrap();
        let err = run(&u, &id2, |i| {
            i.importance = Some(0.8);
            i.valid_until = Some("2020-01-01T00:00:00Z".into());
        })
        .unwrap_err();
        assert!(matches!(err, Error::Validation(_)), "{err}");
        let m = u.get_by_id(&id2).unwrap().unwrap();
        assert!((m.importance - 0.8).abs() > 1e-9, "fields written");
    }

    fn seed_other(u: &Uteke) -> String {
        u.remember("another probe about rust", &["t"], None, None)
            .unwrap()
    }

    #[test]
    fn only_validity_skips_the_field_update() {
        let u = open_keyword();
        let id = seed(&u);
        let before = u.get_by_id(&id).unwrap().unwrap();
        let r = run(&u, &id, |i| i.valid_from = Some(FROM.into())).unwrap();
        assert!(!r.redacted());
        let after = u.get_by_id(&id).unwrap().unwrap();
        assert_eq!(
            after.valid_from.unwrap().to_rfc3339(),
            "2025-01-01T00:00:00+00:00"
        );
        assert_eq!(after.updated_at, before.updated_at, "no field update ran");
        assert!(
            UpdateRequest::decode(input(&id, |i| i.valid_from = Some(FROM.into())))
                .unwrap()
                .only_validity()
        );
        assert!(
            !UpdateRequest::decode(input(&id, |i| {
                i.valid_from = Some(FROM.into());
                i.pinned = Some(true);
            }))
            .unwrap()
            .only_validity()
        );
    }

    #[test]
    fn a_missing_memory_is_not_found_at_every_step() {
        let u = open_keyword();
        let id = "00000000-0000-4000-8000-000000000001";
        for f in [
            (|i: &mut UpdateInput| i.valid_from = Some(FROM.into())) as fn(&mut UpdateInput),
            |i| i.importance = Some(0.5),
            |i| i.namespace = Some("other".into()),
            |i| {
                i.importance = Some(0.5);
                i.valid_from = Some(FROM.into());
            },
        ] {
            let err = run(&u, id, f).unwrap_err();
            assert!(matches!(err, Error::NotFound(_)), "{err}");
            assert!(matches!(&err, Error::NotFound(m) if m == &format!("Memory not found: {id}")));
        }
    }

    #[test]
    fn namespace_moves_after_the_fields() {
        let u = open_keyword();
        let id = seed(&u);
        run(&u, &id, |i| {
            i.namespace = Some("other".into());
            i.pinned = Some(true);
        })
        .unwrap();
        let m = u.get_by_id(&id).unwrap().unwrap();
        assert_eq!(m.namespace, "other");
        assert!(m.pinned);

        // Validity only plus a namespace still moves.
        let id2 = seed_other(&u);
        run(&u, &id2, |i| {
            i.namespace = Some("third".into());
            i.valid_until = Some(UNTIL.into());
        })
        .unwrap();
        let m = u.get_by_id(&id2).unwrap().unwrap();
        assert_eq!(m.namespace, "third");
        assert!(m.valid_until.is_some());
    }

    /// Partial failure: the fields and the validity are stored before the
    /// namespace move, so a refused namespace reports the error and keeps them.
    #[test]
    fn a_refused_namespace_keeps_the_earlier_steps() {
        let u = open_keyword();
        let id = seed(&u);
        let err = run(&u, &id, |i| {
            i.pinned = Some(true);
            i.valid_until = Some(UNTIL.into());
            i.namespace = Some(" ".into());
        })
        .unwrap_err();
        assert!(matches!(err, Error::Validation(_)), "{err}");
        let m = u.get_by_id(&id).unwrap().unwrap();
        assert!(m.pinned && m.valid_until.is_some());
        assert_eq!(m.namespace, crate::DEFAULT_NAMESPACE);
    }
}
