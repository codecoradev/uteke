//! Running a decoded remember request (#1516, #1439).
//!
//! Each transport used to pick the write entry point (room, contradiction,
//! typed, plain) and then run the provenance follow-ups (`source`,
//! `author_type`, `author`) itself, in its own order and with its own failure
//! policy. [`Uteke::run_remember`] does the whole sequence once; a transport
//! decodes, calls it and renders the [`RememberRun`].
//!
//! Failure policy: every provenance value is validated when the request is
//! decoded (`author_type`, `author`) or before the write (`source` under the
//! strict privacy mode), so a follow-up can only fail on a database error. The
//! memory is already stored by then, so the call still succeeds and the
//! failure is reported in [`RememberRun::provenance_warnings`] (and logged),
//! never as an error that would invite a duplicate retry.

use crate::{ContradictionResult, Error, MemoryType, RememberOutcome, RememberRequest, Uteke};

/// Similarity above which a new memory contradicts an older one.
const CONTRADICTION_THRESHOLD: f32 = 0.65;

/// What a remember request produced.
#[derive(Debug)]
pub struct RememberRun {
    pub outcome: RememberOutcome,
    /// Set when `detect_contradiction` was requested.
    pub contradiction: Option<ContradictionResult>,
    /// Provenance follow-ups that failed after the memory was stored.
    pub provenance_warnings: Vec<String>,
}

impl Uteke {
    /// Store `req` and record its provenance. A room write links the memory to
    /// `req.room` with `req.author` (default `anonymous`) as the link author.
    pub fn run_remember(&self, req: &RememberRequest) -> Result<RememberRun, Error> {
        // #1464: the source is stored after the memory, so strict mode has to
        // refuse it before anything is written.
        if let Some(src) = req.source.as_deref() {
            self.privacy_config().mask_text(src)?;
        }

        let tags = req.tag_refs();
        let namespace = req.namespace.as_deref();
        let metadata = req.metadata.clone();
        let content = req.content.as_str();
        let explicit = req.memory_type.as_deref();
        let inferred = || MemoryType::infer_stored_name(content);

        let mut contradiction = None;
        let outcome = if let Some(room) = req.room.as_deref() {
            let author = req.author.as_deref().unwrap_or("anonymous");
            self.remember_in_room(
                content,
                &tags,
                metadata,
                namespace,
                explicit.unwrap_or_else(inferred),
                room,
                author,
            )?
        } else if req.detect_contradiction {
            let (outcome, found) = self.remember_with_contradiction(
                content,
                &tags,
                metadata,
                namespace,
                Some(explicit.unwrap_or_else(inferred)),
                true,
                CONTRADICTION_THRESHOLD,
            )?;
            contradiction = Some(found);
            outcome
        } else if let Some(t) = explicit {
            self.remember_typed_detailed(content, &tags, metadata, namespace, t)?
        } else {
            self.remember_detailed(content, &tags, metadata, namespace)?
        };

        let id = outcome.id.clone();
        let mut warnings = Vec::new();
        let mut note = |what: &str, result: Result<bool, Error>| {
            if let Err(e) = result {
                tracing::error!("Failed to set {what} for {id}: {e}");
                warnings.push(format!("{what}: {e}"));
            }
        };
        if req.source.is_some() || req.source_type.is_some() {
            let st = req.source_type.as_deref().unwrap_or("user");
            note("source", self.set_source(&id, req.source.as_deref(), st));
        }
        if let Some(at) = req.author_type.as_deref() {
            note("author_type", self.set_author_type(&id, at));
        }
        if let Some(a) = req.author.as_deref() {
            note("author", self.set_author(&id, Some(a)));
        }
        Ok(RememberRun {
            outcome,
            contradiction,
            provenance_warnings: warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::open_keyword;
    use crate::{RememberInput, RememberRequest};

    fn req(f: impl FnOnce(&mut RememberInput)) -> RememberRequest {
        let mut i = RememberInput {
            content: "We decided to use SQLite".into(),
            ..RememberInput::default()
        };
        f(&mut i);
        RememberRequest::decode(i).unwrap()
    }

    fn stored(u: &Uteke, run: &RememberRun) -> crate::Memory {
        u.get_by_id(&run.outcome.id).unwrap().unwrap()
    }

    #[test]
    fn the_type_is_explicit_or_inferred_on_every_entry_point() {
        let u = open_keyword();
        for (n, (set, want)) in [
            (
                Box::new(|_: &mut RememberInput| {}) as Box<dyn Fn(&mut RememberInput)>,
                "decision",
            ),
            (Box::new(|i| i.memory_type = Some("note".into())), "note"),
            (Box::new(|i| i.room = Some("r".into())), "decision"),
            (Box::new(|i| i.detect_contradiction = true), "decision"),
        ]
        .into_iter()
        .enumerate()
        {
            // Distinct content per case: identical content is a dedup hit.
            let run = u
                .run_remember(&req(|i| {
                    i.content = format!("We decided to use SQLite for {want} case {n}");
                    set(i);
                }))
                .unwrap();
            assert_eq!(stored(&u, &run).memory_type, want);
        }
    }

    #[test]
    fn provenance_is_recorded_after_the_write() {
        let u = open_keyword();
        let run = u
            .run_remember(&req(|i| {
                i.source = Some("https://example.com".into());
                i.source_type = Some("url".into());
                i.author_type = Some("human".into());
                i.author = Some("  alice ".into());
            }))
            .unwrap();
        let m = stored(&u, &run);
        assert_eq!(m.source.as_deref(), Some("https://example.com"));
        assert_eq!(m.source_type, "url");
        assert_eq!(m.author_type, "human");
        assert_eq!(m.author.as_deref(), Some("alice"));
        assert!(run.provenance_warnings.is_empty());
    }

    #[test]
    fn a_source_type_alone_defaults_to_no_source() {
        let u = open_keyword();
        let run = u
            .run_remember(&req(|i| i.source_type = Some("file".into())))
            .unwrap();
        let m = stored(&u, &run);
        assert_eq!((m.source, m.source_type.as_str()), (None, "file"));
    }

    #[test]
    fn a_room_write_links_with_the_author_or_anonymous() {
        let u = open_keyword();
        let named = u
            .run_remember(&req(|i| {
                i.room = Some("r1".into());
                i.author = Some("bob".into());
            }))
            .unwrap();
        assert_eq!(stored(&u, &named).author.as_deref(), Some("bob"));
        let anon = u
            .run_remember(&req(|i| {
                i.content = "another note".into();
                i.room = Some("r1".into());
            }))
            .unwrap();
        assert_eq!(stored(&u, &anon).author, None);
        assert_eq!(u.recall_room("r1", None, 0).unwrap().len(), 2);
    }

    #[test]
    fn a_contradiction_check_reports_its_result() {
        let u = open_keyword();
        let run = u
            .run_remember(&req(|i| i.detect_contradiction = true))
            .unwrap();
        assert!(run.contradiction.is_some());
        let plain = u
            .run_remember(&req(|i| i.content = "plain".into()))
            .unwrap();
        assert!(plain.contradiction.is_none());
    }
}
