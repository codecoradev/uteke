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
    /// The `category:title` naming warning for a room this write created
    /// (#1715, stage 1). `None` without a room, for an existing room, and for
    /// a new room whose id has that shape.
    pub room_warning: Option<String>,
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
        let mut room_warning = None;
        let outcome = if let Some(room) = req.room.as_deref() {
            let author = req.author.as_deref().unwrap_or("anonymous");
            let written = self.remember_in_room_reporting(
                content,
                &tags,
                metadata,
                namespace,
                explicit.unwrap_or_else(inferred),
                room,
                author,
            )?;
            room_warning = written.room_warning;
            written.outcome
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
        if let Some(v) = &req.validity {
            note("validity", self.set_validity(&id, v));
        }
        Ok(RememberRun {
            outcome,
            contradiction,
            provenance_warnings: warnings,
            room_warning,
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

    /// #1715 stage 1: a write that creates a room outside `category:title`
    /// carries the naming warning; a write into an existing room, an alias of
    /// one, a conventional new room, or no room carries none.
    #[test]
    fn a_room_created_by_a_write_is_warned_only_outside_category_title() {
        let u = open_keyword();
        let write = |n: usize, room: Option<&str>| {
            u.run_remember(&req(|i| {
                i.content = format!("We decided to use SQLite in step {n}");
                i.room = room.map(str::to_string);
            }))
            .unwrap()
        };

        let first = write(1, Some("plain-room"));
        let w = first
            .room_warning
            .as_deref()
            .expect("new plain room warned");
        assert_eq!(
            Some(w.to_string()),
            crate::room_naming::room_id_warning("plain-room")
        );
        assert!(u.get_room("plain-room").unwrap().is_some());

        assert_eq!(
            write(2, Some("plain-room")).room_warning,
            None,
            "existing room"
        );
        assert_eq!(
            write(3, Some("lab:custom")).room_warning,
            None,
            "conventional id"
        );
        assert_eq!(write(4, None).room_warning, None, "no room");

        u.rename_room("plain-room", "ops:plain").unwrap();
        let via_alias = write(5, Some("plain-room"));
        assert_eq!(via_alias.room_warning, None, "alias of an existing room");
        assert_eq!(
            u.list_rooms(None).unwrap().len(),
            2,
            "no room created via alias"
        );
    }

    /// #1715 stage 3, prepared and off by default: with
    /// `[rooms] require_category_title` on, a NEW canonical id of another shape
    /// is refused on create, rename and a room write, before anything is
    /// stored. Existing rooms and aliases keep working.
    #[test]
    fn required_shape_refuses_only_new_canonical_ids() {
        use crate::room_naming::RoomNamingSettings;
        let mut u = open_keyword();
        // Made before the rule is switched on: legacy rooms.
        u.create_room("legacy", None, "default").unwrap();
        u.create_room("old-name", None, "default").unwrap();
        u.set_room_naming_settings(RoomNamingSettings {
            require_category_title: true,
        });
        assert!(u.room_naming_settings().require_category_title);

        let invalid = |r: Result<(), Error>, id: &str| match r {
            Err(Error::InvalidRoomId { id: got }) => assert_eq!(got, id),
            other => panic!("expected InvalidRoomId for {id}, got {other:?}"),
        };
        invalid(u.create_room("plain", None, "default"), "plain");
        assert!(u.get_room("plain").unwrap().is_none());
        u.create_room("lab:custom", None, "default").unwrap();

        invalid(
            u.rename_room("legacy", "still-plain").map(|_| ()),
            "still-plain",
        );
        assert!(u.get_room("legacy").unwrap().is_some(), "nothing renamed");
        u.rename_room("old-name", "ops:new-name").unwrap();

        let before = u.stats(None).unwrap().total_memories;
        let refused = u.run_remember(&req(|i| {
            i.content = "We decided to use SQLite in a fresh plain room".into();
            i.room = Some("fresh-plain".into());
        }));
        assert!(
            matches!(refused, Err(Error::InvalidRoomId { .. })),
            "{refused:?}"
        );
        assert_eq!(
            u.stats(None).unwrap().total_memories,
            before,
            "nothing stored"
        );
        assert!(u.get_room("fresh-plain").unwrap().is_none());

        // An existing legacy room and an alias are written to as before.
        for (n, room) in ["legacy", "old-name"].into_iter().enumerate() {
            let run = u
                .run_remember(&req(|i| {
                    i.content = format!("We decided to use SQLite in legacy step {n}");
                    i.room = Some(room.into());
                }))
                .unwrap();
            assert_eq!(run.room_warning, None);
        }
        // A conventional new room is created without a warning.
        let run = u
            .run_remember(&req(|i| {
                i.content = "We decided to use SQLite in a new team room".into();
                i.room = Some("team:new".into());
            }))
            .unwrap();
        assert_eq!(run.room_warning, None);
        assert!(u.get_room("team:new").unwrap().is_some());
    }

    #[test]
    fn remember_in_room_reporting_says_whether_it_created_the_room() {
        let u = open_keyword();
        let a = u
            .remember_in_room_reporting("Fact one about quartz", &[], None, None, "fact", "r", "a")
            .unwrap();
        assert!(a.room_created);
        assert!(a.room_warning.is_some());
        let b = u
            .remember_in_room_reporting("Fact two about orchid", &[], None, None, "fact", "r", "a")
            .unwrap();
        assert!(!b.room_created);
        assert_eq!(b.room_warning, None);
        let c = u
            .remember_in_room_reporting(
                "Fact three about ledger",
                &[],
                None,
                None,
                "fact",
                "team:x",
                "a",
            )
            .unwrap();
        assert!(c.room_created);
        assert_eq!(c.room_warning, None);
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

    // #1598: caller validity. Timestamps are whole seconds so the RFC3339
    // round trip through SQLite compares equal.
    fn t(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn validity(from: Option<&str>, until: Option<&str>) -> crate::memory::types::Validity {
        crate::memory::types::Validity {
            from: from.map(t),
            until: until.map(t),
        }
    }

    #[test]
    fn wire_validity_lands_in_the_columns_and_keeps_the_metadata_copy() {
        // #1598, Amendment 3 of the core contract: columns, plus the metadata
        // copy for one release.
        let r = req(|i| {
            i.valid_from = Some("2026-01-01T00:00:00Z".into());
            i.valid_until = Some("2027-01-01T00:00:00Z".into());
        });
        let u = open_keyword();
        let m = stored(&u, &u.run_remember(&r).unwrap());
        assert_eq!(m.valid_from, Some(t("2026-01-01T00:00:00Z")));
        assert_eq!(m.valid_until, Some(t("2027-01-01T00:00:00Z")));
        assert_eq!(m.metadata["valid_from"], "2026-01-01T00:00:00Z");
        assert_eq!(m.metadata["valid_until"], "2027-01-01T00:00:00Z");
    }

    #[test]
    fn caller_validity_is_written_to_the_columns() {
        let u = open_keyword();
        let mut r = req(|_| {});
        r.validity = Some(validity(
            Some("2025-03-01T00:00:00Z"),
            Some("2030-01-01T00:00:00Z"),
        ));
        let run = u.run_remember(&r).unwrap();
        assert!(run.provenance_warnings.is_empty());
        let m = stored(&u, &run);
        assert_eq!(m.valid_from, Some(t("2025-03-01T00:00:00Z")));
        assert_eq!(m.valid_until, Some(t("2030-01-01T00:00:00Z")));
        // Without validity the write time stays the start, no end.
        let plain = stored(
            &u,
            &u.run_remember(&req(|i| i.content = "no validity".into()))
                .unwrap(),
        );
        assert_eq!(plain.valid_from, Some(plain.created_at));
        assert_eq!(plain.valid_until, None);
    }

    #[test]
    fn set_validity_keeps_the_unset_side_and_refuses_empty_intervals() {
        let u = open_keyword();
        let id = u.run_remember(&req(|_| {})).unwrap().outcome.id;
        let both = validity(Some("2025-01-01T00:00:00Z"), Some("2026-01-01T00:00:00Z"));
        assert!(u.set_validity(&id, &both).unwrap());

        // Only the end changes; the stored start is kept.
        assert!(
            u.set_validity(&id, &validity(None, Some("2026-06-01T00:00:00Z")))
                .unwrap()
        );
        let m = u.get_by_id(&id).unwrap().unwrap();
        assert_eq!(m.valid_from, Some(t("2025-01-01T00:00:00Z")));
        assert_eq!(m.valid_until, Some(t("2026-06-01T00:00:00Z")));

        // An end before the stored start, and an equal start and end, are refused.
        for bad in [
            validity(None, Some("2024-01-01T00:00:00Z")),
            validity(Some("2026-06-01T00:00:00Z"), None),
        ] {
            let e = u.set_validity(&id, &bad).unwrap_err().to_string();
            assert!(e.contains("must be before"), "{e}");
        }
        let m = u.get_by_id(&id).unwrap().unwrap();
        assert_eq!(m.valid_until, Some(t("2026-06-01T00:00:00Z")));

        assert!(!u.set_validity("no-such-id", &both).unwrap());
    }

    #[test]
    fn set_validity_refuses_a_deprecated_memory() {
        let u = open_keyword();
        let id = u.run_remember(&req(|_| {})).unwrap().outcome.id;
        u.soft_forget(&id, "outdated").unwrap();
        let e = u
            .set_validity(&id, &validity(Some("2025-01-01T00:00:00Z"), None))
            .unwrap_err()
            .to_string();
        assert!(e.contains("deprecated"), "{e}");
    }

    #[test]
    fn deprecation_keeps_an_earlier_caller_end_and_cuts_a_later_one() {
        let u = open_keyword();
        let ended = u
            .run_remember(&req(|i| i.content = "the port was 8080".into()))
            .unwrap()
            .outcome
            .id;
        let open_ended = u
            .run_remember(&req(|i| i.content = "the port is 8767".into()))
            .unwrap()
            .outcome
            .id;
        let future = u
            .run_remember(&req(|i| i.content = "the lease runs to 2099".into()))
            .unwrap()
            .outcome
            .id;
        let past = validity(Some("2020-01-01T00:00:00Z"), Some("2021-01-01T00:00:00Z"));
        assert!(u.set_validity(&ended, &past).unwrap());
        assert!(
            u.set_validity(&future, &validity(None, Some("2099-01-01T00:00:00Z")))
                .unwrap()
        );

        for id in [&ended, &open_ended, &future] {
            u.soft_forget(id, "outdated").unwrap();
        }
        // The fact had already ended: the caller's end is kept.
        let m = u.get_by_id(&ended).unwrap().unwrap();
        assert_eq!(m.valid_until, Some(t("2021-01-01T00:00:00Z")));
        assert!(m.deprecated_at.unwrap() > t("2021-01-01T00:00:00Z"));
        // No caller end (every row written before #1598): unchanged, the
        // deprecation time.
        let m = u.get_by_id(&open_ended).unwrap().unwrap();
        assert_eq!(m.valid_until, m.deprecated_at);
        // An end still in the future is cut to the deprecation time.
        let m = u.get_by_id(&future).unwrap().unwrap();
        assert_eq!(m.valid_until, m.deprecated_at);
    }
}
