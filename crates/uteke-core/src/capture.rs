//! Raw observation capture with deferred extraction (#1466).
//!
//! Capture is the hot path: one SQLite insert per turn, no embedder, no
//! network. Extraction is the cold path: a run takes up to `budget` sessions
//! with pending turns, asks a [`FactSource`] (the LLM extractor, or the
//! offline rules) for facts, and stores them through `remember`, so the
//! privacy filter, the duplicate gate and rejected-value tombstones (#1597)
//! apply. Design: `docs/plans/2026-10-29-raw-capture-1466.md`.

use crate::error::Error;
use crate::extraction::ExtractedFact;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// A tool result is kept as a summary: at most this many characters (#1466,
/// owner decision 2026-11-01). Large tool output is noise for extraction.
pub const TOOL_SUMMARY_CHARS: usize = 2000;

/// Longest accepted session id.
const MAX_SESSION_ID: usize = 200;

/// `[capture]` in the global `uteke.toml`. A project file cannot set it (a
/// cloned repository must not turn capture on).
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct CaptureSettings {
    /// Accept captured turns. Off by default: capture is opt-in.
    pub enabled: bool,
    /// Minutes between scheduled extraction runs (server).
    pub interval_minutes: u64,
    /// Sessions extracted per run (LLM calls per run).
    pub budget_per_run: usize,
    /// Days a done observation is kept before it is deleted.
    pub retention_days: u32,
    /// Extraction attempts per session before it is marked failed.
    pub max_attempts: u32,
    /// Minutes after which a session left `extracting` (a crashed run) is
    /// handed back to the queue.
    pub lease_minutes: u64,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: 60,
            budget_per_run: 10,
            retention_days: 30,
            max_attempts: 3,
            lease_minutes: 15,
        }
    }
}

/// Who said a captured turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    /// A tool result, kept as a summary of [`TOOL_SUMMARY_CHARS`].
    Tool,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }
}

impl std::str::FromStr for Role {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        match s.trim().to_ascii_lowercase().as_str() {
            "user" => Ok(Role::User),
            "assistant" => Ok(Role::Assistant),
            "tool" => Ok(Role::Tool),
            other => Err(Error::validation(format!(
                "Unknown role '{other}'. Use user, assistant or tool"
            ))),
        }
    }
}

/// What [`crate::Uteke::capture`] did.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureOutcome {
    pub id: String,
    /// The same turn was already in the queue; nothing was added.
    pub duplicate: bool,
    /// The privacy filter masked a secret in the turn.
    pub redacted: bool,
}

/// Queue counts, by turn.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct QueueStatus {
    pub pending: usize,
    pub extracting: usize,
    pub done: usize,
    pub failed: usize,
    /// Sessions with at least one pending turn.
    pub sessions_pending: usize,
}

/// What one extraction run did.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ExtractionRunReport {
    pub sessions_done: usize,
    /// Sessions that reached `max_attempts` in this run.
    pub sessions_failed: usize,
    pub facts_stored: usize,
    /// Facts refused by a rejected-value tombstone or the privacy filter.
    pub facts_refused: usize,
    /// Sessions handed back after an expired lease.
    pub reclaimed: usize,
    /// Done turns deleted by the retention rule.
    pub pruned: usize,
    /// Why the run stopped early (the extractor failed: rate limit or a
    /// provider error). The session goes back to the queue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped: Option<String>,
    /// Sessions still pending after the run.
    pub sessions_pending: usize,
}

/// Where facts come from: the LLM extractor or the offline rules.
pub trait FactSource {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, Error>;
}

impl FactSource for crate::extraction::Extractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, Error> {
        crate::extraction::Extractor::extract(self, text)
    }
}

/// Rule-based extraction, no network (`crate::offline_extraction`).
pub struct OfflineFacts {
    pub max_facts: usize,
}

impl FactSource for OfflineFacts {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, Error> {
        Ok(
            crate::offline_extraction::extract_facts(text, self.max_facts)
                .into_iter()
                .map(ExtractedFact::flat)
                .collect(),
        )
    }
}

/// Captured conversations go to an LLM only when the user chose one: mode
/// `llm`, or no mode at all with an API key. Any other mode (`offline`, the
/// `none` that onboarding writes, an unknown value) keeps them local.
pub fn sends_to_llm(cfg: &crate::extraction::ExtractionConfig) -> bool {
    match cfg.mode.as_str() {
        "llm" => true,
        "" => !cfg.api_key.is_empty(),
        _ => false,
    }
}

/// The fact source of an extraction run (CLI, server schedule): the LLM
/// extractor when [`sends_to_llm`], else the offline rules.
pub fn fact_source(
    cfg: &crate::extraction::ExtractionConfig,
) -> Result<Box<dyn FactSource>, Error> {
    if sends_to_llm(cfg) {
        let mut llm = cfg.clone();
        llm.mode = "llm".to_string();
        Ok(Box::new(crate::extraction::Extractor::new(&llm)?))
    } else {
        Ok(Box::new(OfflineFacts {
            max_facts: cfg.max_facts,
        }))
    }
}

/// How far a failing schedule backs off: at most this many intervals.
pub const MAX_BACKOFF_INTERVALS: u32 = 16;

/// Wait before the next scheduled run: the interval after a run that went
/// through, double the previous wait after one that failed (rate limit,
/// provider down), up to [`MAX_BACKOFF_INTERVALS`] intervals.
pub fn next_run_delay(
    interval: std::time::Duration,
    previous: std::time::Duration,
    failed: bool,
) -> std::time::Duration {
    if failed {
        previous
            .max(interval)
            .saturating_mul(2)
            .min(interval.saturating_mul(MAX_BACKOFF_INTERVALS))
    } else {
        interval
    }
}

/// `text` without a leading `user:`, `assistant:` or `tool:` label.
fn strip_role_label(text: &str) -> &str {
    let t = text.trim_start();
    for label in ["user:", "assistant:", "tool:"] {
        // Compare bytes: slicing `t` at the label length could split a
        // multibyte character. A matched ASCII label ends on a boundary.
        if t.as_bytes()
            .get(..label.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(label.as_bytes()))
        {
            return t[label.len()..].trim_start();
        }
    }
    t
}

/// Dedup key of a turn: the same text from the same role in the same session.
fn turn_hash(session_id: &str, role: Role, content: &str) -> String {
    crate::provenance::content_hash(&format!("{session_id}\u{0}{}\u{0}{content}", role.as_str()))
}

fn ago(minutes: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::minutes(minutes)).to_rfc3339()
}

impl crate::Uteke {
    /// Append one turn to the capture queue (#1466). The privacy filter runs
    /// first, so a secret never sits in the queue in clear text. Capturing
    /// the same turn again is a no-op (`duplicate`). No embedder, no network.
    /// The facts extracted from the turn are stored in `namespace` (the
    /// caller's, recorded now so a later run does not depend on its own).
    pub fn capture(
        &self,
        session_id: &str,
        role: Role,
        content: &str,
        source: Option<&str>,
        namespace: Option<&str>,
    ) -> Result<CaptureOutcome, Error> {
        let session_id = session_id.trim();
        if session_id.is_empty() || session_id.chars().count() > MAX_SESSION_ID {
            return Err(Error::validation(format!(
                "A session id is required (1 to {MAX_SESSION_ID} characters)"
            )));
        }
        if content.trim().is_empty() {
            return Err(Error::validation("Nothing to capture: the turn is empty"));
        }
        let text: String = if role == Role::Tool && content.chars().count() > TOOL_SUMMARY_CHARS {
            let mut s: String = content.chars().take(TOOL_SUMMARY_CHARS).collect();
            s.push_str(" [...]");
            s
        } else {
            content.to_string()
        };
        let prepared = self.privacy_config.prepare_with_tags(&text, &[], None)?;
        let redacted = !prepared.redactions.is_empty();
        let text = prepared.content;
        let hash = turn_hash(session_id, role, &text);

        let conn = self.graph_store();
        // INSERT OR IGNORE: two hooks capturing the same turn at once both
        // succeed, one of them as the duplicate.
        let inserted = conn
            .execute(
                "INSERT OR IGNORE INTO observations (id, session_id, seq, role, content, content_hash, source, captured_at, namespace)
                 VALUES (?1, ?2,
                         (SELECT COALESCE(MAX(seq), 0) + 1 FROM observations WHERE session_id = ?2),
                         ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    uuid::Uuid::now_v7().to_string(),
                    session_id,
                    role.as_str(),
                    text,
                    hash,
                    source,
                    chrono::Utc::now().to_rfc3339(),
                    namespace
                ],
            )
            .map_err(|e| Error::db("capture insert", e))?;
        let id: String = conn
            .query_row(
                "SELECT id FROM observations WHERE content_hash = ?1",
                params![hash],
                |r| r.get(0),
            )
            .map_err(|e| Error::db("capture lookup", e))?;
        Ok(CaptureOutcome {
            id,
            duplicate: inserted == 0,
            redacted,
        })
    }

    /// Queue counts.
    pub fn capture_status(&self) -> Result<QueueStatus, Error> {
        let conn = self.graph_store();
        let mut status = QueueStatus::default();
        let mut stmt = conn
            .prepare("SELECT status, COUNT(*) FROM observations GROUP BY status")
            .map_err(|e| Error::db("capture status", e))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| Error::db("capture status", e))?;
        for row in rows {
            let (s, n) = row.map_err(|e| Error::db("capture status", e))?;
            let n = n as usize;
            match s.as_str() {
                "pending" => status.pending = n,
                "extracting" => status.extracting = n,
                "done" => status.done = n,
                "failed" => status.failed = n,
                _ => {}
            }
        }
        status.sessions_pending = self.sessions_pending()?;
        Ok(status)
    }

    fn sessions_pending(&self) -> Result<usize, Error> {
        self.graph_store()
            .query_row(
                "SELECT COUNT(DISTINCT session_id) FROM observations WHERE status = 'pending'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .map_err(|e| Error::db("count pending sessions", e))
    }

    /// One extraction run (#1466): delete done and failed turns past the
    /// retention, hand back batches whose lease expired, then extract up to
    /// `budget` sessions, oldest first, one extractor call each. An extractor
    /// error (rate limit, provider down) stops the run: the turns go back to
    /// the queue, or are marked failed after `max_attempts`.
    ///
    /// Each claim is a batch: its id is written on the turns, which keep it
    /// until they are done. A crashed batch is claimed again as the same
    /// batch, without turns that arrived since, and its facts carry the source
    /// `capture:<session>:<batch>`. A run that finds live facts with that
    /// source (a crash after storing some or all of them, before marking
    /// done) withdraws them and extracts the batch again, so its facts are
    /// stored whole and never twice. The claim is one conditional UPDATE, so
    /// two runs never work on the same batch.
    pub fn run_capture_extraction(
        &self,
        facts: &dyn FactSource,
        budget: usize,
        settings: &CaptureSettings,
    ) -> Result<ExtractionRunReport, Error> {
        let conn = self.graph_store();
        let mut report = ExtractionRunReport {
            pruned: conn
                .execute(
                    "DELETE FROM observations WHERE status IN ('done', 'failed') AND finished_at < ?1",
                    params![ago(i64::from(settings.retention_days) * 24 * 60)],
                )
                .map_err(|e| Error::db("capture retention", e))?,
            ..Default::default()
        };
        let lease = ago(settings.lease_minutes as i64);
        report.reclaimed = conn
            .query_row(
                "SELECT COUNT(DISTINCT batch_id) FROM observations
                 WHERE status = 'extracting' AND claimed_at < ?1",
                params![lease],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|e| Error::db("capture reclaim", e))? as usize;
        // A batch that keeps crashing the run (it never reaches the error
        // path) fails like one that keeps erroring.
        conn.execute(
            "UPDATE observations
             SET status = CASE WHEN attempts >= ?2 THEN 'failed' ELSE 'pending' END,
                 claimed_at = NULL,
                 finished_at = CASE WHEN attempts >= ?2 THEN ?3 ELSE NULL END
             WHERE status = 'extracting' AND claimed_at < ?1",
            params![
                lease,
                settings.max_attempts,
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .map_err(|e| Error::db("capture reclaim", e))?;

        let sessions: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT session_id FROM observations WHERE status = 'pending'
                     GROUP BY session_id ORDER BY MIN(captured_at), session_id LIMIT ?1",
                )
                .map_err(|e| Error::db("capture sessions", e))?;
            let rows = stmt
                .query_map(params![budget as i64], |r| r.get::<_, String>(0))
                .map_err(|e| Error::db("capture sessions", e))?;
            rows.collect::<Result<_, _>>()
                .map_err(|e| Error::db("capture sessions", e))?
        };

        for session in sessions {
            if let Err(stop) = self.extract_session(facts, &session, settings, &mut report)? {
                report.stopped = Some(stop);
                break;
            }
        }
        report.sessions_pending = self.sessions_pending()?;
        Ok(report)
    }

    /// Claim the next batch of `session`: the pending turns of an unfinished
    /// batch when there is one (a crash or an extractor error left it), else
    /// every pending turn under a new batch id. `None` when another run
    /// claimed it first.
    fn claim_batch(&self, session: &str) -> Result<Option<String>, Error> {
        let conn = self.graph_store();
        let old: Option<String> = conn
            .query_row(
                "SELECT batch_id FROM observations
                 WHERE session_id = ?1 AND status = 'pending' AND batch_id IS NOT NULL
                 ORDER BY seq LIMIT 1",
                params![session],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| Error::db("capture claim", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        let (batch, claimed) = match old {
            Some(b) => {
                let n = conn
                    .execute(
                        "UPDATE observations SET status = 'extracting', claimed_at = ?2, attempts = attempts + 1
                         WHERE batch_id = ?1 AND status = 'pending'",
                        params![b, now],
                    )
                    .map_err(|e| Error::db("capture claim", e))?;
                (b, n)
            }
            None => {
                let b = uuid::Uuid::now_v7().simple().to_string();
                let n = conn
                    .execute(
                        "UPDATE observations SET status = 'extracting', claimed_at = ?3, attempts = attempts + 1, batch_id = ?2
                         WHERE session_id = ?1 AND status = 'pending' AND batch_id IS NULL",
                        params![session, b, now],
                    )
                    .map_err(|e| Error::db("capture claim", e))?;
                (b, n)
            }
        };
        Ok((claimed > 0).then_some(batch))
    }

    /// Extract one batch of a session. The outer `Result` is a database
    /// failure; the inner `Err` is an extractor failure that stops the run.
    fn extract_session(
        &self,
        facts: &dyn FactSource,
        session: &str,
        settings: &CaptureSettings,
        report: &mut ExtractionRunReport,
    ) -> Result<Result<(), String>, Error> {
        let Some(batch) = self.claim_batch(session)? else {
            return Ok(Ok(()));
        };
        let conn = self.graph_store();
        let turns: Vec<(String, String, Option<String>)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT role, content, namespace FROM observations
                     WHERE batch_id = ?1 AND status = 'extracting' ORDER BY seq",
                )
                .map_err(|e| Error::db("capture turns", e))?;
            let rows = stmt
                .query_map(params![batch], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(|e| Error::db("capture turns", e))?;
            rows.collect::<Result<_, _>>()
                .map_err(|e| Error::db("capture turns", e))?
        };
        let source = format!("capture:{session}:{}", &batch[..batch.len().min(16)]);
        // Facts of this batch still live from an earlier attempt that never
        // reached "done" (a crash, possibly halfway through storing): take
        // them back, so the batch is extracted and stored whole, once.
        let leftovers: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT id FROM memories WHERE source = ?1 AND deprecated = 0")
                .map_err(|e| Error::db("capture batch check", e))?;
            let rows = stmt
                .query_map(params![source], |r| r.get(0))
                .map_err(|e| Error::db("capture batch check", e))?;
            rows.collect::<Result<_, _>>()
                .map_err(|e| Error::db("capture batch check", e))?
        };
        for id in &leftovers {
            self.soft_forget(id, "capture batch rerun")?;
        }

        {
            let text = turns
                .iter()
                .map(|(role, content, _)| format!("{role}: {content}"))
                .collect::<Vec<_>>()
                .join("\n");
            // The namespace the session was captured in (its first turn).
            let namespace = turns.first().and_then(|t| t.2.clone());
            let outcome = match facts.extract(&text) {
                Ok(found) => {
                    let mut stored = Vec::new();
                    let mut failure = None;
                    for fact in &found {
                        match self.store_captured_fact(fact, &source, namespace.as_deref()) {
                            Ok(Some(id)) => stored.push(id),
                            Ok(None) => {}
                            // A refusal of this one fact: a tombstone, strict
                            // privacy, or a fact the write rejects as invalid.
                            // Retrying would give the same answer.
                            Err(
                                e @ (Error::RejectedValue { .. }
                                | Error::SecretDetected { .. }
                                | Error::Validation(_)),
                            ) => {
                                tracing::debug!("Captured fact refused: {e}");
                                report.facts_refused += 1
                            }
                            // Database, embedder or lock: may pass next time,
                            // so the batch is retried whole.
                            Err(e) => {
                                failure = Some(format!("storing a fact failed: {e}"));
                                break;
                            }
                        }
                    }
                    match failure {
                        None => {
                            report.facts_stored += stored.len();
                            Ok(())
                        }
                        // Take back what this attempt stored, so the retry
                        // extracts the batch whole.
                        Some(why) => {
                            for id in &stored {
                                if let Err(e) = self.soft_forget(id, "capture batch retried") {
                                    tracing::warn!("Failed to withdraw captured fact {id}: {e}");
                                }
                            }
                            Err(why)
                        }
                    }
                }
                Err(e) => Err(e.to_string()),
            };
            if let Err(why) = outcome {
                // Per turn: a turn that joined this batch later has fewer
                // attempts and goes back to pending.
                conn.execute(
                    "UPDATE observations
                     SET status = CASE WHEN attempts >= ?2 THEN 'failed' ELSE 'pending' END,
                         claimed_at = NULL,
                         finished_at = CASE WHEN attempts >= ?2 THEN ?3 ELSE NULL END
                     WHERE batch_id = ?1 AND status = 'extracting'",
                    params![
                        batch,
                        settings.max_attempts,
                        chrono::Utc::now().to_rfc3339()
                    ],
                )
                .map_err(|e| Error::db("capture release", e))?;
                let failed: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM observations WHERE batch_id = ?1 AND status = 'failed'",
                        params![batch],
                        |r| r.get(0),
                    )
                    .map_err(|e| Error::db("capture release", e))?;
                if failed > 0 {
                    report.sessions_failed += 1;
                }
                return Ok(Err(why));
            }
        }
        conn.execute(
            "UPDATE observations SET status = 'done', finished_at = ?2
             WHERE batch_id = ?1 AND status = 'extracting'",
            params![batch, chrono::Utc::now().to_rfc3339()],
        )
        .map_err(|e| Error::db("capture done", e))?;
        report.sessions_done += 1;
        Ok(Ok(()))
    }

    /// Store one extracted fact like `import --extract` does: its type,
    /// scene tag and priority when the extractor gave them, and the source.
    /// `None` when the fact matched a stored memory (left as it is).
    fn store_captured_fact(
        &self,
        fact: &ExtractedFact,
        source: &str,
        namespace: Option<&str>,
    ) -> Result<Option<String>, Error> {
        // The session text labels each turn ("user: ..."); a rule-based
        // extractor copies the label into the fact.
        let content = strip_role_label(&fact.content);
        if content.trim().is_empty() {
            return Ok(None);
        }
        let fact = &ExtractedFact {
            content: content.to_string(),
            ..fact.clone()
        };
        let mut tags = vec!["capture".to_string()];
        if let Some(scene) = &fact.scene {
            tags.push(format!("scene:{scene}"));
        }
        let tag_refs: Vec<&str> = tags.iter().map(String::as_str).collect();
        let started = chrono::Utc::now();
        let id = match fact.fact_type.as_deref() {
            Some(t) if crate::memory::types::MemoryType::from_str_opt(t).is_some() => {
                self.remember_typed(&fact.content, &tag_refs, None, namespace, t)?
            }
            _ => self.remember(&fact.content, &tag_refs, None, namespace)?,
        };
        // A duplicate of a stored memory (the write returned an older row):
        // leave that memory as it is.
        let created = self.store.get_by_id(&id)?.map(|m| m.created_at);
        if created.is_none_or(|c| c < started) {
            return Ok(None);
        }
        if let Some(p) = fact.priority.filter(|p| (0.0..=1.0).contains(p)) {
            self.set_importance(&id, p)?;
        }
        self.set_source(&id, Some(source), "capture")?;
        Ok(Some(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::open_keyword;
    use std::sync::Mutex;

    /// A fact source that returns one fact per line of the session text and
    /// can fail its first calls; it records every call.
    struct Fake {
        calls: Mutex<Vec<String>>,
        fail_first: Mutex<usize>,
    }

    impl Fake {
        fn new(fail_first: usize) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_first: Mutex::new(fail_first),
            }
        }
        fn calls(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl FactSource for Fake {
        fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, Error> {
            self.calls.lock().unwrap().push(text.to_string());
            let mut left = self.fail_first.lock().unwrap();
            if *left > 0 {
                *left -= 1;
                return Err(Error::generic("Extraction endpoint returned HTTP 429"));
            }
            Ok(text
                .lines()
                .filter_map(|l| l.strip_prefix("user: "))
                .map(|l| ExtractedFact::flat(format!("fact: {l}")))
                .collect())
        }
    }

    fn settings() -> CaptureSettings {
        CaptureSettings {
            enabled: true,
            ..Default::default()
        }
    }

    fn rows(u: &crate::Uteke, sql: &str) -> i64 {
        u.graph_store().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn the_same_turn_twice_is_one_row_and_tool_output_is_summarised() {
        let u = open_keyword();
        let a = u
            .capture("s1", Role::User, "deploy to staging", None, None)
            .unwrap();
        let b = u
            .capture("s1", Role::User, "deploy to staging", None, None)
            .unwrap();
        assert!(!a.duplicate && b.duplicate);
        assert_eq!(a.id, b.id);
        // same text from another role or session is a new turn
        assert!(
            !u.capture("s1", Role::Assistant, "deploy to staging", None, None)
                .unwrap()
                .duplicate
        );
        assert!(
            !u.capture("s2", Role::User, "deploy to staging", None, None)
                .unwrap()
                .duplicate
        );

        u.capture(
            "s1",
            Role::Tool,
            &"x".repeat(TOOL_SUMMARY_CHARS + 500),
            None,
            None,
        )
        .unwrap();
        let len = rows(
            &u,
            "SELECT length(content) FROM observations WHERE role = 'tool'",
        );
        assert_eq!(len as usize, TOOL_SUMMARY_CHARS + " [...]".len());
        assert_eq!(
            rows(
                &u,
                "SELECT MAX(seq) FROM observations WHERE session_id = 's1'"
            ),
            3
        );

        assert!(u.capture(" ", Role::User, "x", None, None).is_err());
        assert!(u.capture("s1", Role::User, "   ", None, None).is_err());
        assert!("bot".parse::<Role>().is_err());
    }

    #[test]
    fn a_secret_is_masked_before_it_is_queued() {
        let u = open_keyword();
        let token = format!("{}{}", "ghp_", "aB3dE5gH7jK9mN1pQ3rS5tU7vW9xY1zA3bC5");
        let out = u
            .capture(
                "s1",
                Role::User,
                &format!("my token is {token}"),
                None,
                None,
            )
            .unwrap();
        assert!(out.redacted);
        let stored: String = u
            .graph_store()
            .query_row("SELECT content FROM observations", [], |r| r.get(0))
            .unwrap();
        assert!(!stored.contains("aB3dE5gH7jK9"), "{stored}");
    }

    #[test]
    fn a_run_extracts_up_to_the_budget_oldest_session_first() {
        let u = open_keyword();
        u.capture("old", Role::User, "the db is postgres", None, None)
            .unwrap();
        u.capture("old", Role::Assistant, "noted", None, None)
            .unwrap();
        u.capture("new", Role::User, "the cache is redis", None, None)
            .unwrap();
        let fake = Fake::new(0);
        let r = u.run_capture_extraction(&fake, 1, &settings()).unwrap();
        assert_eq!(
            (r.sessions_done, r.facts_stored, r.sessions_pending),
            (1, 1, 1)
        );
        assert_eq!(fake.calls(), 1);
        assert!(
            fake.calls.lock().unwrap()[0].starts_with("user: the db is postgres\nassistant: noted")
        );
        let stored = u.search("postgres", 5, None, None).unwrap();
        assert_eq!(stored.len(), 1);
        assert!(stored[0].memory.tags.contains(&"capture".to_string()));
        assert!(
            stored[0]
                .memory
                .source
                .as_deref()
                .unwrap()
                .starts_with("capture:old:")
        );

        let r = u.run_capture_extraction(&fake, 5, &settings()).unwrap();
        assert_eq!((r.sessions_done, r.sessions_pending), (1, 0));
        let s = u.capture_status().unwrap();
        assert_eq!((s.pending, s.done, s.sessions_pending), (0, 3, 0));
    }

    #[test]
    fn an_extractor_error_stops_the_run_and_the_session_is_retried_then_failed() {
        let u = open_keyword();
        u.capture("a", Role::User, "first", None, None).unwrap();
        u.capture("b", Role::User, "second", None, None).unwrap();
        let fake = Fake::new(3);
        let s = CaptureSettings {
            max_attempts: 2,
            ..settings()
        };
        let r = u.run_capture_extraction(&fake, 5, &s).unwrap();
        assert!(r.stopped.as_deref().unwrap().contains("429"));
        assert_eq!(
            (fake.calls(), r.sessions_done, r.sessions_pending),
            (1, 0, 2)
        );
        // second attempt of session a fails too: it reaches max_attempts
        let r = u.run_capture_extraction(&fake, 5, &s).unwrap();
        assert_eq!(r.sessions_failed, 1);
        // the run after that skips the failed session and gets b through
        // (one more failure for b first)
        let r = u.run_capture_extraction(&fake, 5, &s).unwrap();
        assert!(r.stopped.is_some());
        let r = u.run_capture_extraction(&fake, 5, &s).unwrap();
        assert_eq!((r.sessions_done, r.facts_stored), (1, 1));
        let st = u.capture_status().unwrap();
        assert_eq!((st.failed, st.done, st.pending), (1, 1, 0));
    }

    #[test]
    fn an_expired_lease_is_reclaimed_and_a_live_one_is_not() {
        let u = open_keyword();
        u.capture("crashed", Role::User, "x", None, None).unwrap();
        u.capture("running", Role::User, "y", None, None).unwrap();
        u.graph_store()
            .execute(
                "UPDATE observations SET status = 'extracting', claimed_at = ?1, attempts = 1, batch_id = 'b1' WHERE session_id = 'crashed'",
                [ago(60)],
            )
            .unwrap();
        u.graph_store()
            .execute(
                "UPDATE observations SET status = 'extracting', claimed_at = ?1, attempts = 1, batch_id = 'b2' WHERE session_id = 'running'",
                [ago(1)],
            )
            .unwrap();
        let fake = Fake::new(0);
        let r = u.run_capture_extraction(&fake, 5, &settings()).unwrap();
        assert_eq!((r.reclaimed, r.sessions_done), (1, 1));
        assert_eq!(
            rows(
                &u,
                "SELECT COUNT(*) FROM observations WHERE session_id = 'running' AND status = 'extracting'"
            ),
            1
        );
    }

    #[test]
    fn a_rerun_after_a_crash_does_not_store_the_facts_twice() {
        let u = open_keyword();
        u.capture("s", Role::User, "the region is eu-west-1", None, None)
            .unwrap();
        u.capture("s", Role::User, "the zone is c", None, None)
            .unwrap();
        let fake = Fake::new(0);
        u.run_capture_extraction(&fake, 5, &settings()).unwrap();
        // crash halfway: one of the two facts was stored, then the process died
        let second: String = u
            .graph_store()
            .query_row(
                "SELECT id FROM memories WHERE content LIKE '%zone%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        u.graph_store()
            .execute("DELETE FROM memories WHERE id = ?1", [&second])
            .unwrap();
        u.graph_store()
            .execute(
                "UPDATE observations SET status = 'extracting', claimed_at = ?1, finished_at = NULL",
                [ago(60)],
            )
            .unwrap();
        let r = u.run_capture_extraction(&fake, 5, &settings()).unwrap();
        assert_eq!(
            (r.sessions_done, r.facts_stored),
            (1, 2),
            "extracted whole again"
        );
        let live: i64 = rows(
            &u,
            "SELECT COUNT(*) FROM memories WHERE source LIKE 'capture:%' AND deprecated = 0",
        );
        assert_eq!(live, 2, "each fact once");
    }

    #[test]
    fn done_turns_past_the_retention_are_deleted() {
        let u = open_keyword();
        u.capture("s", Role::User, "keep me short", None, None)
            .unwrap();
        let fake = Fake::new(0);
        u.run_capture_extraction(&fake, 5, &settings()).unwrap();
        u.graph_store()
            .execute(
                "UPDATE observations SET finished_at = ?1",
                [ago(31 * 24 * 60)],
            )
            .unwrap();
        let r = u.run_capture_extraction(&fake, 5, &settings()).unwrap();
        assert_eq!(r.pruned, 1);
        assert_eq!(rows(&u, "SELECT COUNT(*) FROM observations"), 0);
    }

    #[test]
    fn a_fact_forgotten_as_wrong_is_not_stored_again() {
        let u = open_keyword();
        let id = u
            .remember("fact: the port is 9999", &[], None, None)
            .unwrap();
        u.forget_with_reason(&id, Some("wrong")).unwrap();
        u.capture("s", Role::User, "the port is 9999", None, None)
            .unwrap();
        let r = u
            .run_capture_extraction(&Fake::new(0), 5, &settings())
            .unwrap();
        assert_eq!(
            (r.facts_stored, r.facts_refused, r.sessions_done),
            (0, 1, 1)
        );
    }

    #[test]
    fn a_batch_is_claimed_once() {
        let u = open_keyword();
        u.capture("s", Role::User, "x", None, None).unwrap();
        let first = u.claim_batch("s").unwrap();
        assert!(first.is_some());
        assert_eq!(
            u.claim_batch("s").unwrap(),
            None,
            "a second run gets nothing"
        );
    }

    #[test]
    fn a_crashed_batch_is_rerun_without_new_turns_and_without_storing_twice() {
        let u = open_keyword();
        u.capture("s", Role::User, "the region is eu-west-1", None, None)
            .unwrap();
        let fake = Fake::new(0);
        u.run_capture_extraction(&fake, 5, &settings()).unwrap();
        // crash after storing, before "done", then a new turn arrives
        u.graph_store()
            .execute(
                "UPDATE observations SET status = 'extracting', claimed_at = ?1, finished_at = NULL",
                [ago(60)],
            )
            .unwrap();
        u.capture("s", Role::User, "the zone is b", None, None)
            .unwrap();
        let before = u.count(None).unwrap();
        let r = u.run_capture_extraction(&fake, 5, &settings()).unwrap();
        // the old batch alone is extracted again; its fact is replaced, not doubled
        assert_eq!((r.reclaimed, r.sessions_done, r.facts_stored), (1, 1, 1));
        assert_eq!(fake.calls(), 2);
        assert!(!fake.calls.lock().unwrap()[1].contains("zone"));
        assert_eq!(u.count(None).unwrap(), before);
        // the new turn is its own batch on the next run
        let r = u.run_capture_extraction(&fake, 5, &settings()).unwrap();
        assert_eq!((r.sessions_done, r.facts_stored), (1, 1));
        assert!(fake.calls.lock().unwrap()[2].starts_with("user: the zone is b"));
    }

    #[test]
    fn a_batch_that_keeps_crashing_the_run_is_failed_at_max_attempts() {
        let u = open_keyword();
        u.capture("s", Role::User, "poison", None, None).unwrap();
        let s = CaptureSettings {
            max_attempts: 2,
            ..settings()
        };
        for _ in 0..2 {
            u.claim_batch("s").unwrap().unwrap();
            // the run dies: the lease expires
            u.graph_store()
                .execute("UPDATE observations SET claimed_at = ?1", [ago(60)])
                .unwrap();
            u.run_capture_extraction(&Fake::new(0), 0, &s).unwrap();
        }
        assert_eq!(u.capture_status().unwrap().failed, 1);
    }

    #[test]
    fn attempts_are_counted_per_turn() {
        let u = open_keyword();
        u.capture("s", Role::User, "old", None, None).unwrap();
        let s = CaptureSettings {
            max_attempts: 2,
            ..settings()
        };
        u.run_capture_extraction(&Fake::new(1), 5, &s).unwrap(); // old: 1 attempt
        u.capture("s", Role::User, "new", None, None).unwrap();
        u.run_capture_extraction(&Fake::new(1), 5, &s).unwrap(); // old batch again: failed
        let failed: i64 = rows(
            &u,
            "SELECT COUNT(*) FROM observations WHERE status = 'failed'",
        );
        let new_pending: i64 = rows(
            &u,
            "SELECT COUNT(*) FROM observations WHERE content = 'new' AND status = 'pending' AND attempts = 0",
        );
        assert_eq!((failed, new_pending), (1, 1));
    }

    #[test]
    fn failed_turns_are_deleted_after_the_retention_too() {
        let u = open_keyword();
        u.capture("s", Role::User, "x", None, None).unwrap();
        let s = CaptureSettings {
            max_attempts: 1,
            ..settings()
        };
        u.run_capture_extraction(&Fake::new(1), 5, &s).unwrap();
        assert_eq!(u.capture_status().unwrap().failed, 1);
        u.graph_store()
            .execute(
                "UPDATE observations SET finished_at = ?1",
                [ago(31 * 24 * 60)],
            )
            .unwrap();
        let r = u.run_capture_extraction(&Fake::new(0), 5, &s).unwrap();
        assert_eq!(r.pruned, 1);
    }

    #[test]
    fn a_fact_that_matches_a_stored_memory_does_not_retag_it() {
        // an embedder, so the duplicate gate returns the stored memory
        let (emb, _) = crate::test_support::RecordingEmbedder::new();
        let u = crate::test_support::open_with_embedder(Box::new(emb));
        let id = u.remember("fact: shared", &[], None, None).unwrap();
        u.capture("s", Role::User, "shared", None, None).unwrap();
        u.run_capture_extraction(&Fake::new(0), 5, &settings())
            .unwrap();
        let m = u.get_by_id(&id).unwrap().unwrap();
        assert_ne!(m.source_type, "capture");
    }

    /// Returns a good fact, then one whose insert fails with a database
    /// error (a trigger stands in for a full disk), until `fixed`.
    struct HalfBroken {
        fixed: Mutex<bool>,
    }

    impl FactSource for HalfBroken {
        fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, Error> {
            let mut facts = vec![ExtractedFact::flat("the build uses cargo")];
            if !*self.fixed.lock().unwrap() {
                facts.push(ExtractedFact::flat("boom fact"));
            }
            Ok(facts)
        }
    }

    #[test]
    fn a_failed_store_withdraws_the_batch_and_the_retry_stores_it_whole() {
        let u = open_keyword();
        u.capture("s", Role::User, "how do we build", None, None)
            .unwrap();
        u.graph_store()
            .execute_batch(
                "CREATE TEMP TRIGGER boom BEFORE INSERT ON memories WHEN NEW.content = 'boom fact'
                 BEGIN SELECT RAISE(ABORT, 'disk is busy'); END;",
            )
            .unwrap();
        let src = HalfBroken {
            fixed: Mutex::new(false),
        };
        let r = u.run_capture_extraction(&src, 5, &settings()).unwrap();
        assert!(
            r.stopped
                .as_deref()
                .unwrap()
                .contains("storing a fact failed")
        );
        assert_eq!((r.sessions_done, r.facts_stored), (0, 0));
        assert_eq!(
            u.search("cargo", 5, None, None).unwrap().len(),
            0,
            "withdrawn"
        );
        assert_eq!(u.capture_status().unwrap().pending, 1);

        *src.fixed.lock().unwrap() = true;
        u.graph_store().execute_batch("DROP TRIGGER boom;").unwrap();
        let r = u.run_capture_extraction(&src, 5, &settings()).unwrap();
        assert_eq!((r.sessions_done, r.facts_stored), (1, 1));
        assert_eq!(u.search("cargo", 5, None, None).unwrap().len(), 1);
    }

    #[test]
    fn facts_land_in_the_namespace_the_session_was_captured_in() {
        let u = open_keyword();
        u.capture("s", Role::User, "the queue is kafka", None, Some("team-a"))
            .unwrap();
        u.run_capture_extraction(&Fake::new(0), 5, &settings())
            .unwrap();
        let hits = u.search("kafka", 5, None, Some("team-a")).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory.namespace, "team-a");
        assert!(
            u.search("kafka", 5, None, Some("default"))
                .unwrap()
                .is_empty()
        );
    }

    /// Facts the write can never accept.
    struct Junk;

    impl FactSource for Junk {
        fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, Error> {
            Ok(vec![
                ExtractedFact::flat("user:"),
                ExtractedFact::flat("   "),
                ExtractedFact::flat("x".repeat(200_000)),
                ExtractedFact::flat("the cache is redis"),
            ])
        }
    }

    #[test]
    fn empty_and_invalid_facts_do_not_fail_the_batch() {
        let u = open_keyword();
        u.capture("s", Role::User, "anything", None, None).unwrap();
        let r = u.run_capture_extraction(&Junk, 5, &settings()).unwrap();
        assert_eq!(r.stopped, None);
        assert_eq!((r.sessions_done, r.facts_stored), (1, 1));
        assert!(r.facts_refused >= 1, "the oversized fact is refused");
    }

    #[test]
    fn the_role_label_is_not_part_of_a_fact() {
        // a multibyte character where the label would end must not panic
        assert_eq!(strip_role_label("userпривет"), "userпривет");
        assert_eq!(strip_role_label("meetïng notes"), "meetïng notes");
        assert_eq!(strip_role_label("assistant:é"), "é");
        assert_eq!(
            strip_role_label("user: the db is postgres"),
            "the db is postgres"
        );
        assert_eq!(strip_role_label("  Assistant:noted"), "noted");
        assert_eq!(strip_role_label("users: plural"), "users: plural");
        assert_eq!(strip_role_label("plain"), "plain");
    }

    #[test]
    fn capture_is_off_by_default() {
        assert!(!CaptureSettings::default().enabled);
        assert_eq!(CaptureSettings::default().retention_days, 30);
    }

    #[test]
    fn the_llm_is_used_only_when_extraction_names_one() {
        use crate::extraction::ExtractionConfig;
        let cfg = |mode: &str, key: &str| ExtractionConfig {
            mode: mode.into(),
            api_key: key.into(),
            ..Default::default()
        };
        assert!(!sends_to_llm(&cfg("", "")), "unconfigured: offline rules");
        assert!(sends_to_llm(&cfg("", "sk-test")));
        assert!(sends_to_llm(&cfg("llm", "")));
        assert!(!sends_to_llm(&cfg("offline", "sk-test")));
        assert!(
            !sends_to_llm(&cfg("none", "sk-test")),
            "onboarding's none stays local"
        );
        assert!(!sends_to_llm(&cfg("typo", "sk-test")));
    }

    #[test]
    fn a_failing_schedule_doubles_its_wait_up_to_the_cap_and_resets_after_success() {
        use std::time::Duration;
        let hour = Duration::from_secs(3600);
        assert_eq!(next_run_delay(hour, hour, false), hour);
        assert_eq!(next_run_delay(hour, hour, true), hour * 2);
        assert_eq!(next_run_delay(hour, hour * 2, true), hour * 4);
        assert_eq!(next_run_delay(hour, hour * 8, true), hour * 16);
        assert_eq!(
            next_run_delay(hour, hour * 16, true),
            hour * MAX_BACKOFF_INTERVALS,
            "capped"
        );
        assert_eq!(next_run_delay(hour, hour * 16, false), hour, "reset");
        // A previous wait shorter than the interval still doubles the interval.
        assert_eq!(next_run_delay(hour, Duration::ZERO, true), hour * 2);
    }
}
