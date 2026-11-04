//! Read-only views over captured sessions (#1470 c): the turns of one session in
//! `seq` order, the list of sessions, and the memories a session produced.
//!
//! Extraction stores facts with the source `capture:<session>:<batch>`; the plan
//! for the viewer also names `observation:<session>`. Both link back to a session.

use crate::error::Error;
use rusqlite::params;
use serde::Serialize;

/// One captured session: how many turns the queue still holds for it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionSummary {
    pub session_id: String,
    pub turns: usize,
    pub first_at: String,
    pub last_at: String,
}

/// One captured turn.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionTurn {
    pub seq: i64,
    pub role: String,
    pub content: String,
    pub captured_at: String,
    pub status: String,
}

/// A live memory whose source points at a session.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionMemory {
    pub id: String,
    pub content: String,
    pub namespace: String,
    pub created_at: String,
    pub source: String,
}

/// The session a memory `source` points at, if it is a capture source.
///
/// `capture:<session>:<batch>` (the batch is the last segment; a session id may
/// itself contain `:`) or `observation:<session>`.
pub fn session_of_source(source: &str) -> Option<&str> {
    if let Some(rest) = source.strip_prefix("observation:") {
        return Some(rest).filter(|s| !s.is_empty());
    }
    let rest = source.strip_prefix("capture:")?;
    let (session, _batch) = rest.rsplit_once(':')?;
    Some(session).filter(|s| !s.is_empty())
}

/// `?1` = `observation:<session>`, `?2`/`?3` = the half-open range of sources that
/// start with `capture:<session>:`. Equality and range predicates on `source` use
/// `idx_memories_source` (v25), unlike a `substr` prefix test, and take the
/// session id literally (no LIKE wildcards). `+deprecated` keeps the planner off
/// the low-selectivity `deprecated` index (it picks it over `source` when the
/// table has no statistics). The `instr` guard keeps a longer
/// session id (`a:b`) out of the view of its prefix (`a`): after the prefix only
/// the batch id (no ':') may follow.
const SESSION_MEMORIES_SQL: &str = "SELECT id, content, namespace, created_at, source FROM memories
     WHERE +deprecated = 0
       AND (source = ?1
            OR (source >= ?2 AND source < ?3
                AND instr(substr(source, length(?2) + 1), ':') = 0))
     ORDER BY created_at, id";

impl crate::Uteke {
    /// Sessions in the capture queue, most recently active first.
    pub fn capture_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>, Error> {
        let mut stmt = self
            .graph_store()
            .prepare(
                "SELECT session_id, COUNT(*), MIN(captured_at), MAX(captured_at)
                 FROM observations GROUP BY session_id
                 ORDER BY MAX(captured_at) DESC, session_id LIMIT ?1",
            )
            .map_err(|e| Error::db("capture sessions", e))?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(SessionSummary {
                    session_id: r.get(0)?,
                    turns: r.get::<_, i64>(1)? as usize,
                    first_at: r.get(2)?,
                    last_at: r.get(3)?,
                })
            })
            .map_err(|e| Error::db("capture sessions", e))?;
        rows.collect::<Result<_, _>>()
            .map_err(|e| Error::db("capture sessions", e))
    }

    /// The turns of one session in `seq` order (at most `limit`).
    pub fn session_timeline(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<SessionTurn>, Error> {
        let mut stmt = self
            .graph_store()
            .prepare(
                "SELECT seq, role, content, captured_at, status FROM observations
                 WHERE session_id = ?1 ORDER BY seq LIMIT ?2",
            )
            .map_err(|e| Error::db("session timeline", e))?;
        let rows = stmt
            .query_map(params![session_id, limit as i64], |r| {
                Ok(SessionTurn {
                    seq: r.get(0)?,
                    role: r.get(1)?,
                    content: r.get(2)?,
                    captured_at: r.get(3)?,
                    status: r.get(4)?,
                })
            })
            .map_err(|e| Error::db("session timeline", e))?;
        rows.collect::<Result<_, _>>()
            .map_err(|e| Error::db("session timeline", e))
    }

    /// Live memories whose source is `observation:<session>` or
    /// `capture:<session>:<batch>`, oldest first.
    pub fn session_memories(&self, session_id: &str) -> Result<Vec<SessionMemory>, Error> {
        let mut stmt = self
            .graph_store()
            .prepare(SESSION_MEMORIES_SQL)
            .map_err(|e| Error::db("session memories", e))?;
        let rows = stmt
            .query_map(
                params![
                    format!("observation:{session_id}"),
                    format!("capture:{session_id}:"),
                    // ';' is the character after ':', so the pair bounds every
                    // source that starts with `capture:<session>:`.
                    format!("capture:{session_id};"),
                ],
                |r| {
                    Ok(SessionMemory {
                        id: r.get(0)?,
                        content: r.get(1)?,
                        namespace: r.get(2)?,
                        created_at: r.get(3)?,
                        source: r.get(4)?,
                    })
                },
            )
            .map_err(|e| Error::db("session memories", e))?;
        rows.collect::<Result<_, _>>()
            .map_err(|e| Error::db("session memories", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Uteke;
    use crate::capture::Role;

    fn open() -> Uteke {
        Uteke::open_with_backend(":memory:", None).expect("in-memory store")
    }

    #[test]
    fn source_parsing_covers_both_forms() {
        assert_eq!(session_of_source("observation:abc"), Some("abc"));
        assert_eq!(session_of_source("capture:abc:0123"), Some("abc"));
        // a session id with a colon keeps it: the batch is the last segment
        assert_eq!(session_of_source("capture:a:b:0123"), Some("a:b"));
        assert_eq!(session_of_source("observation:"), None);
        assert_eq!(session_of_source("capture:abc"), None);
        assert_eq!(session_of_source("user"), None);
        assert_eq!(session_of_source("capture::b"), None);
    }

    #[test]
    fn timeline_is_in_seq_order_and_scoped_to_the_session() {
        let u = open();
        u.capture("s1", Role::User, "first", None, None).unwrap();
        u.capture("s2", Role::User, "other", None, None).unwrap();
        u.capture("s1", Role::Assistant, "second", None, None)
            .unwrap();
        u.capture("s1", Role::Tool, "third", None, None).unwrap();

        let t = u.session_timeline("s1", 100).unwrap();
        let got: Vec<(i64, &str, &str)> = t
            .iter()
            .map(|x| (x.seq, x.role.as_str(), x.content.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (1, "user", "first"),
                (2, "assistant", "second"),
                (3, "tool", "third")
            ]
        );
        assert!(t.iter().all(|x| x.status == "pending"));
        assert_eq!(u.session_timeline("s1", 2).unwrap().len(), 2);
        assert!(u.session_timeline("none", 10).unwrap().is_empty());
    }

    #[test]
    fn sessions_list_counts_turns() {
        let u = open();
        u.capture("s1", Role::User, "a", None, None).unwrap();
        u.capture("s1", Role::User, "b", None, None).unwrap();
        u.capture("s2", Role::User, "c", None, None).unwrap();
        let mut got: Vec<(String, usize)> = u
            .capture_sessions(10)
            .unwrap()
            .into_iter()
            .map(|s| (s.session_id, s.turns))
            .collect();
        got.sort();
        assert_eq!(got, [("s1".to_string(), 2), ("s2".to_string(), 1)]);
        assert_eq!(u.capture_sessions(1).unwrap().len(), 1);
    }

    #[test]
    fn memories_link_back_by_both_source_forms_and_skip_deprecated() {
        let u = open();
        let conn = u.graph_store();
        let id_of = |content: &str| -> String { u.remember(content, &[], None, None).unwrap() };
        let obs = id_of("from observation");
        let cap = id_of("from capture");
        let other = id_of("another session");
        let prefix = id_of("session s10 must not match s1");
        let gone = id_of("deprecated one");
        for (id, src) in [
            (&obs, "observation:s1"),
            (&cap, "capture:s1:batch0001"),
            (&other, "capture:s2:batch0002"),
            (&prefix, "capture:s10:batch0003"),
            (&gone, "capture:s1:batch0004"),
        ] {
            conn.execute(
                "UPDATE memories SET source = ?1 WHERE id = ?2",
                params![src, id],
            )
            .unwrap();
        }
        conn.execute(
            "UPDATE memories SET deprecated = 1 WHERE id = ?1",
            params![gone],
        )
        .unwrap();

        let mut got: Vec<String> = u
            .session_memories("s1")
            .unwrap()
            .into_iter()
            .map(|m| m.content)
            .collect();
        got.sort();
        assert_eq!(got, ["from capture", "from observation"]);
    }

    /// Stores `source` on a fresh memory and returns nothing; helper for the
    /// session-match tests below.
    fn with_sources(u: &Uteke, rows: &[(&str, &str)]) {
        let conn = u.graph_store();
        for (content, src) in rows {
            let id = u.remember(content, &[], None, None).unwrap();
            conn.execute(
                "UPDATE memories SET source = ?1 WHERE id = ?2",
                params![src, id],
            )
            .unwrap();
        }
    }

    fn contents(u: &Uteke, session: &str) -> Vec<String> {
        let mut got: Vec<String> = u
            .session_memories(session)
            .unwrap()
            .into_iter()
            .map(|m| m.content)
            .collect();
        got.sort();
        got
    }

    #[test]
    fn a_session_id_with_a_colon_does_not_leak_into_its_prefix_session() {
        let u = open();
        with_sources(
            &u,
            &[
                ("a one", "capture:a:batch0001"),
                ("a obs", "observation:a"),
                ("a:b one", "capture:a:b:batch0002"),
                ("a:b obs", "observation:a:b"),
                ("a:b:c one", "capture:a:b:c:batch0003"),
            ],
        );
        assert_eq!(contents(&u, "a"), ["a obs", "a one"]);
        assert_eq!(contents(&u, "a:b"), ["a:b obs", "a:b one"]);
        assert_eq!(contents(&u, "a:b:c"), ["a:b:c one"]);
    }

    #[test]
    fn wildcards_and_quotes_in_a_session_id_are_literal() {
        let u = open();
        with_sources(
            &u,
            &[
                ("pct", "capture:100%:batch0001"),
                ("any", "capture:100xyz:batch0002"),
                ("under", "capture:s_1:batch0003"),
                ("other under", "capture:sa1:batch0004"),
                ("quote", "capture:it's:batch0005"),
                ("semi", "capture:a;b:batch0006"),
            ],
        );
        assert_eq!(contents(&u, "100%"), ["pct"]);
        assert_eq!(contents(&u, "s_1"), ["under"]);
        assert_eq!(contents(&u, "it's"), ["quote"]);
        assert_eq!(contents(&u, "a;b"), ["semi"]);
        assert!(contents(&u, "%").is_empty());
    }

    #[test]
    fn the_session_lookup_is_served_by_a_source_index() {
        let u = open();
        let conn = u.graph_store();
        let plan: Vec<String> = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {SESSION_MEMORIES_SQL}"))
            .unwrap()
            .query_map(params!["observation:s", "capture:s:", "capture:s;"], |r| {
                r.get::<_, String>(3)
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let plan = plan.join(" | ");
        assert!(plan.contains("idx_memories_source"), "plan: {plan}");
        assert!(!plan.contains("SCAN memories"), "plan: {plan}");
    }
}
