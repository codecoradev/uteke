//! Rejected-value tombstones (#1597).
//!
//! A memory forgotten (or superseded) with the reason `wrong` leaves a
//! tombstone: the SHA-256 of its normalised text. Every write path checks it
//! through `commit_memory`, so the same wrong value cannot be stored again,
//! even after the deprecated row was pruned. Ordinary forgets leave none.
//! Design: `docs/plans/2026-11-01-rejected-value-tombstone-1597.md`.

use crate::error::Error;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// Characters of the preview stored with a tombstone.
const PREVIEW_CHARS: usize = 120;

/// A recorded rejection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    /// SHA-256 of the normalised text.
    pub hash: String,
    /// The memory that was rejected (it may have been pruned since).
    pub memory_id: Option<String>,
    /// The reason as given, e.g. `wrong: the port is 8767`.
    pub reason: String,
    /// The first characters of the text.
    pub preview: String,
    pub created_at: String,
}

/// Whether a forget / supersede reason marks the value as wrong: `wrong`, or
/// `wrong:` followed by a note, any letter case.
pub fn is_wrong_reason(reason: &str) -> bool {
    let r = reason.trim().to_lowercase();
    r == "wrong" || r.starts_with("wrong:")
}

/// The text a tombstone compares: NFKC, case folded (Unicode lowercase),
/// invisible characters removed and whitespace collapsed to single spaces.
pub fn normalise(text: &str) -> String {
    let folded: String = text
        .nfkc()
        .flat_map(char::to_lowercase)
        .filter(|c| !is_invisible(*c))
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Zero-width and other format characters that render as nothing.
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}'
            | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{FFA0}'
            | '\u{E0000}'..='\u{E007F}' | '\u{E0100}'..='\u{E01EF}'
    )
}

/// The tombstone key of a text.
pub fn tombstone_hash(text: &str) -> String {
    crate::provenance::content_hash(&normalise(text))
}

/// Refuse `content` when it matches a tombstone in `conn`.
pub(crate) fn check(conn: &rusqlite::Connection, content: &str) -> Result<(), Error> {
    let reason: Option<String> = conn
        .query_row(
            "SELECT reason FROM rejected_values WHERE hash = ?1",
            params![tombstone_hash(content)],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| Error::db("check rejected value", e))?;
    match reason {
        Some(reason) => Err(Error::RejectedValue { reason }),
        None => Ok(()),
    }
}

/// Record a tombstone for `memory` through `conn` (an open transaction when
/// the caller has one). A value rejected twice keeps its first tombstone.
pub(crate) fn record(
    conn: &rusqlite::Connection,
    memory: &crate::Memory,
    reason: &str,
) -> Result<(), Error> {
    let preview: String = memory.content.chars().take(PREVIEW_CHARS).collect();
    conn.execute(
        "INSERT OR IGNORE INTO rejected_values (hash, memory_id, reason, preview, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            tombstone_hash(&memory.content),
            memory.id,
            reason.trim(),
            preview,
            chrono::Utc::now().to_rfc3339()
        ],
    )
    .map_err(|e| Error::db("record tombstone", e))?;
    Ok(())
}

/// Restore tombstones (structural import). Existing ones win. Returns the
/// number added.
pub(crate) fn insert_all(conn: &rusqlite::Connection, items: &[Tombstone]) -> Result<usize, Error> {
    let mut n = 0;
    // A hash that is not a SHA-256 can never match a write: drop it.
    for t in items
        .iter()
        .filter(|t| t.hash.len() == 64 && t.hash.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        n += conn
            .execute(
                "INSERT OR IGNORE INTO rejected_values (hash, memory_id, reason, preview, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![t.hash, t.memory_id, t.reason, t.preview, t.created_at],
            )
            .map_err(|e| Error::db("restore tombstone", e))?;
    }
    Ok(n)
}

impl crate::Uteke {
    /// Refuse `content` when it matches a tombstone (#1597).
    pub(crate) fn check_not_rejected(&self, content: &str) -> Result<(), Error> {
        check(self.graph_store(), content)
    }

    /// Record a tombstone for the current content of memory `id`. A value
    /// rejected twice keeps its first tombstone.
    pub(crate) fn record_tombstone(&self, id: &str, reason: &str) -> Result<(), Error> {
        let memory = self
            .store
            .get_by_id(id)?
            .ok_or_else(|| Error::not_found(format!("Memory not found: {id}")))?;
        record(self.graph_store(), &memory, reason)
    }

    /// Remove the tombstone of `content`, whichever memory recorded it.
    /// Returns the number removed (0 or 1).
    pub(crate) fn remove_tombstone_of(&self, content: &str) -> Result<usize, Error> {
        self.graph_store()
            .execute(
                "DELETE FROM rejected_values WHERE hash = ?1",
                params![tombstone_hash(content)],
            )
            .map_err(|e| Error::db("remove tombstone", e))
    }

    /// Every tombstone, newest first.
    pub fn tombstones(&self) -> Result<Vec<Tombstone>, Error> {
        let mut stmt = self
            .graph_store()
            .prepare(
                "SELECT hash, memory_id, reason, preview, created_at FROM rejected_values
                 ORDER BY created_at DESC, hash",
            )
            .map_err(|e| Error::db("prepare tombstone list", e))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Tombstone {
                    hash: r.get(0)?,
                    memory_id: r.get(1)?,
                    reason: r.get(2)?,
                    preview: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })
            .map_err(|e| Error::db("list tombstones", e))?;
        rows.collect::<Result<_, _>>()
            .map_err(|e| Error::db("read tombstones", e))
    }

    /// Remove the tombstone named by `key`: a hash prefix (at least 8 hex
    /// characters, unambiguous) or the id of the rejected memory. Returns the
    /// number removed.
    pub fn remove_tombstone(&self, key: &str) -> Result<usize, Error> {
        let key = key.trim();
        if key.is_empty() {
            return Err(Error::validation(
                "Give a tombstone hash prefix or a memory id",
            ));
        }
        let conn = self.graph_store();
        let by_memory = conn
            .execute(
                "DELETE FROM rejected_values WHERE memory_id = ?1",
                params![key],
            )
            .map_err(|e| Error::db("remove tombstone by memory", e))?;
        if by_memory > 0 {
            return Ok(by_memory);
        }
        if key.len() < 8 || !key.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(0);
        }
        let prefix = format!("{}%", key.to_ascii_lowercase());
        let matches: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM rejected_values WHERE hash LIKE ?1",
                params![prefix],
                |r| r.get(0),
            )
            .map_err(|e| Error::db("count tombstones", e))?;
        if matches > 1 {
            return Err(Error::validation(format!(
                "Tombstone prefix '{key}' is ambiguous ({matches} matches); give more characters"
            )));
        }
        conn.execute(
            "DELETE FROM rejected_values WHERE hash LIKE ?1",
            params![prefix],
        )
        .map_err(|e| Error::db("remove tombstone", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reason_wrong_marks_a_value() {
        assert!(is_wrong_reason("wrong"));
        assert!(is_wrong_reason("  Wrong: the port is 8767 "));
        assert!(!is_wrong_reason("outdated"));
        assert!(!is_wrong_reason("wrongly filed"));
        assert!(!is_wrong_reason(""));
    }

    #[test]
    fn normalisation_folds_case_width_whitespace_and_invisible_characters() {
        let base = normalise("The port is 8767");
        assert_eq!(base, "the port is 8767");
        for variant in [
            "THE PORT IS 8767",
            "  the   port\tis\n8767 ",
            "the p\u{200B}ort is 8767",
            "the port is \u{FF18}\u{FF17}\u{FF16}\u{FF17}", // full-width digits
            "\u{FEFF}the port is 8767",
            "the port\u{00A0}is 8767",
            "the port is 8767\u{FE0F}",  // variation selector
            "the port\u{E0020} is 8767", // tag character
        ] {
            assert_eq!(normalise(variant), base, "{variant:?}");
            assert_eq!(tombstone_hash(variant), tombstone_hash("The port is 8767"));
        }
        assert_ne!(tombstone_hash("the port is 8768"), tombstone_hash(&base));
    }
}

/// The tombstone across the write paths (#1597).
#[cfg(test)]
mod write_path_tests {
    use crate::Error;
    use crate::test_support::open_keyword;

    fn rejected<T: std::fmt::Debug>(r: Result<T, Error>) -> String {
        match r {
            Err(Error::RejectedValue { reason }) => reason,
            other => panic!("expected RejectedValue, got {other:?}"),
        }
    }

    fn wrong_value(u: &crate::Uteke) -> String {
        let id = u
            .remember("The deploy target is staging-2", &[], None, None)
            .unwrap();
        u.forget_with_reason(&id, Some("wrong: it is staging-3"))
            .unwrap();
        id
    }

    #[test]
    fn remember_room_remember_and_update_refuse_a_wrong_value() {
        let u = open_keyword();
        wrong_value(&u);
        assert_eq!(
            rejected(u.remember("the deploy target is   STAGING-2", &[], None, None)),
            "wrong: it is staging-3"
        );
        u.create_room("ops", None, "default").unwrap();
        rejected(u.remember_in_room(
            "The deploy target is staging-2",
            &[],
            None,
            None,
            "fact",
            "ops",
            "bot",
        ));
        let other = u.remember("unrelated", &[], None, None).unwrap();
        rejected(u.update_memory(
            &other,
            Some("The deploy target is staging-2"),
            None,
            None,
            None,
            None,
            None,
        ));
        // a different value is fine
        u.remember("The deploy target is staging-3", &[], None, None)
            .unwrap();
    }

    #[test]
    fn an_ordinary_forget_records_nothing() {
        let u = open_keyword();
        let id = u.remember("short-lived note", &[], None, None).unwrap();
        u.forget_with_reason(&id, Some("outdated")).unwrap();
        assert!(u.tombstones().unwrap().is_empty());
        u.remember("short-lived note", &[], None, None).unwrap();
    }

    #[test]
    fn supersede_with_reason_wrong_rejects_the_old_value() {
        let u = open_keyword();
        let old = u
            .remember("Cache TTL is 5 minutes", &[], None, None)
            .unwrap();
        let new = u
            .remember("Cache TTL is 15 minutes", &[], None, None)
            .unwrap();
        u.supersede(&old, &new, Some("wrong")).unwrap();
        rejected(u.remember("cache ttl is 5 minutes", &[], None, None));
        let t = u.tombstones().unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].memory_id.as_deref(), Some(old.as_str()));
        assert_eq!(t[0].preview, "Cache TTL is 5 minutes");
    }

    #[test]
    fn the_tombstone_survives_prune() {
        let u = open_keyword();
        let id = wrong_value(&u);
        u.graph_store()
            .execute(
                "UPDATE memories SET deprecated_at = '2020-01-01T00:00:00Z', updated_at = '2020-01-01T00:00:00Z' WHERE id = ?1",
                [&id],
            )
            .unwrap();
        let pruned = u.prune(30, None, false).unwrap();
        assert_eq!(pruned.pruned, 1);
        assert!(u.get_by_id(&id).unwrap().is_none(), "the row is gone");
        rejected(u.remember("The deploy target is staging-2", &[], None, None));
    }

    #[test]
    fn promote_and_remove_lift_the_tombstone() {
        let u = open_keyword();
        let id = wrong_value(&u);
        assert!(u.promote(&id).unwrap());
        assert!(u.tombstones().unwrap().is_empty());

        let id2 = u.remember("Region is eu-west-9", &[], None, None).unwrap();
        u.forget_with_reason(&id2, Some("wrong")).unwrap();
        let hash = u.tombstones().unwrap()[0].hash.clone();
        assert_eq!(
            u.remove_tombstone("abc").unwrap(),
            0,
            "too short to be a prefix"
        );
        assert_eq!(u.remove_tombstone(&hash[..10]).unwrap(), 1);
        u.remember("Region is eu-west-9", &[], None, None).unwrap();
    }

    #[test]
    fn import_skips_and_counts_a_wrong_value() {
        let u = open_keyword();
        wrong_value(&u);
        let r = u
            .import(
                "{\"content\":\"The deploy target is staging-2\"}\n{\"content\":\"fresh fact\"}\n",
                None,
            )
            .unwrap();
        assert_eq!((r.imported, r.skipped, r.rejected_tombstone), (1, 1, 1));
    }

    #[test]
    fn a_full_bundle_carries_the_tombstones_and_skips_their_rows() {
        let src = open_keyword();
        wrong_value(&src);
        src.remember("kept fact", &[], None, None).unwrap();
        let bundle = src.export_full().unwrap();
        assert!(bundle.contains("\"rejected_value\""));

        let dst = open_keyword();
        // the rejected value was already in the target: its row is skipped
        let mut tampered = bundle.clone();
        tampered.push_str(
            "\n{\"type\":\"memory\",\"id\":\"01900000-0000-7000-8000-000000000001\",\"content\":\"the deploy target is staging-2\",\"tags\":[],\"metadata\":{},\"created_at\":\"t\",\"updated_at\":\"t\",\"namespace\":\"default\",\"access_count\":0,\"memory_type\":\"fact\",\"importance\":0.5,\"pinned\":false,\"content_type\":\"text\",\"source_type\":\"user\"}\n",
        );
        let report = dst.import_full(&tampered).unwrap();
        assert_eq!(report["imported"]["rejected_values"], 1, "{report}");
        assert_eq!(report["imported"]["rejected_tombstone"], 1, "{report}");
        assert_eq!(dst.tombstones().unwrap().len(), 1);
        rejected(dst.remember("The deploy target is staging-2", &[], None, None));
        assert_eq!(dst.count(None).unwrap(), 1, "only the kept fact");
    }

    #[test]
    fn a_full_import_skips_the_room_links_edges_and_events_of_a_rejected_row() {
        let src = open_keyword();
        src.create_room("ops", None, "default").unwrap();
        let x = src
            .remember_in_room(
                "The deploy target is staging-2",
                &[],
                None,
                None,
                "fact",
                "ops",
                "bot",
            )
            .unwrap()
            .id;
        let y = src
            .remember("deploys run on fridays", &[], None, None)
            .unwrap();
        src.link_memories(&x, &y, "related").unwrap();
        let bundle = src.export_full().unwrap();

        let dst = open_keyword();
        wrong_value(&dst);
        let report = dst.import_full(&bundle).expect("the import does not abort");
        assert_eq!(report["imported"]["rejected_tombstone"], 1, "{report}");
        assert!(dst.get_by_id(&x).unwrap().is_none());
        assert!(dst.get_by_id(&y).unwrap().is_some());
        let orphans: i64 = dst
            .graph_store()
            .query_row(
                "SELECT (SELECT COUNT(*) FROM room_memories WHERE memory_id = ?1)
                      + (SELECT COUNT(*) FROM memory_edges WHERE source_id = ?1 OR target_id = ?1)
                      + (SELECT COUNT(*) FROM timeline_events WHERE memory_id = ?1)",
                [&x],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orphans, 0);
    }

    #[test]
    fn promote_lifts_a_tombstone_recorded_by_a_duplicate() {
        let u = open_keyword();
        let a = u.remember("Queue size is 64", &[], None, None).unwrap();
        // a copy that existed before any tombstone (e.g. from an old import)
        let mut copy = u.get_by_id(&a).unwrap().unwrap();
        copy.id = uuid::Uuid::now_v7().to_string();
        u.store().insert(&copy).unwrap();
        let b = copy.id;
        u.forget_with_reason(&a, Some("wrong")).unwrap();
        u.forget_with_reason(&b, Some("wrong")).unwrap();
        assert_eq!(u.tombstones().unwrap().len(), 1);
        assert!(u.promote(&b).unwrap());
        assert!(u.tombstones().unwrap().is_empty());
        u.remember("Queue size is 64", &[], None, None).unwrap();
    }

    #[test]
    fn supersede_records_the_tombstone_with_the_supersession() {
        let u = open_keyword();
        let old = u.remember("Timeout is 3s", &[], None, None).unwrap();
        let new = u.remember("Timeout is 30s", &[], None, None).unwrap();
        u.supersede(&old, &new, Some("wrong: typo")).unwrap();
        assert!(u.get_by_id(&old).unwrap().unwrap().deprecated);
        assert_eq!(u.tombstones().unwrap()[0].reason, "wrong: typo");
    }
}
