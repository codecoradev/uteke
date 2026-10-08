//! Keeping the vector index in step with SQLite — one policy, one place.
//!
//! SQLite is the source of truth; the vector index is a derived, repairable
//! structure (`uteke repair` rebuilds it). Every mutating operation therefore
//! commits to SQLite first and then mirrors the change into the index. That
//! mirroring used to be hand-written at ~60 call sites with five different
//! failure policies (swallowed, retried inline, retried through a helper,
//! warn-only, propagated). This module owns the policy:
//!
//! * inserts are retried a bounded number of times, except for errors that
//!   can never succeed (a dimension mismatch is deterministic);
//! * persisting (`save`) is retried a bounded number of times;
//! * the caller gets a [`SyncOutcome`] that says what actually happened, and
//!   decides whether that is an error (a `forget` that could not persist the
//!   index) or a warning (a `remember` stored FTS5-only).
//!
//! The functions work on a `&mut impl IndexWriter` the caller already holds
//! the write lock for, so the store write and the index write stay ordered
//! under one lock. [`IndexWriter`] is the seam: production uses
//! [`crate::memory::vector::VectorIndex`], tests use a recording/failing fake.

use crate::Error;
use crate::memory::vector::VectorIndex;
use std::time::Duration;

/// The operations the sync policy needs from an index.
pub(crate) trait IndexWriter {
    fn insert(&mut self, id: &str, embedding: &[f32]) -> Result<(), Error>;
    /// Returns `false` when `id` was not in the index.
    fn remove(&mut self, id: &str) -> bool;
    fn save(&mut self) -> Result<(), Error>;
    /// Save only if enough has accumulated (per-operation hot paths, #1322).
    /// Adapters without batching just save every time.
    fn save_deferred(&mut self) -> Result<(), Error> {
        self.save()
    }
    fn dims(&self) -> usize;
}

impl IndexWriter for VectorIndex {
    fn insert(&mut self, id: &str, embedding: &[f32]) -> Result<(), Error> {
        VectorIndex::insert(self, id, embedding)
    }
    fn remove(&mut self, id: &str) -> bool {
        VectorIndex::remove(self, id)
    }
    fn save(&mut self) -> Result<(), Error> {
        VectorIndex::save(self)
    }
    fn save_deferred(&mut self) -> Result<(), Error> {
        VectorIndex::save_if_due(self)
    }
    fn dims(&self) -> usize {
        VectorIndex::dims(self)
    }
}

/// How hard to try before giving up on one index operation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SyncPolicy {
    pub attempts: u32,
    pub backoff: Duration,
}

impl SyncPolicy {
    /// Production policy: 3 attempts, 200 ms apart (#621, #1273).
    pub(crate) const STANDARD: SyncPolicy = SyncPolicy {
        attempts: 3,
        backoff: Duration::from_millis(200),
    };
}

/// What actually happened to the index. SQLite has already been written by
/// the time this is produced, so none of these are fatal by themselves.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SyncOutcome {
    /// The vector insert failed (after retries). The row stays FTS5-searchable.
    pub insert_error: Option<String>,
    /// The index could not be written to disk (after retries). The in-memory
    /// index is correct; the on-disk copy is stale until the next save.
    pub persist_error: Option<String>,
    /// Removals requested for ids that were not in the index.
    pub missing: usize,
    attempts: u32,
}

impl SyncOutcome {
    #[cfg(test)]
    pub(crate) fn is_clean(&self) -> bool {
        self.insert_error.is_none() && self.persist_error.is_none()
    }

    /// One-line description of the first failure, for "honest outcome"
    /// reporting (`RememberOutcome.vector_error`, warnings).
    pub(crate) fn error_message(&self) -> Option<String> {
        let after = if self.attempts > 1 {
            format!(" after {} attempts", self.attempts)
        } else {
            String::new()
        };
        if let Some(e) = &self.insert_error {
            return Some(format!("vector insert failed{after}: {e}"));
        }
        self.persist_error
            .as_ref()
            .map(|e| format!("index persist failed{after}: {e}"))
    }

    /// Log a warning when the on-disk index could not be saved; for callers
    /// where a stale file is repairable and not worth failing the operation.
    pub(crate) fn warn_if_stale(&self, what: &str) {
        if let Some(msg) = self.error_message() {
            tracing::warn!("{what}: {msg}. `uteke repair` / `verify` resync the index.");
        }
    }

    /// For operations whose SQLite change is already committed and where an
    /// unsaved index must be surfaced to the caller as an error.
    pub(crate) fn into_delete_result(self, what: &str) -> Result<(), Error> {
        match self.error_message() {
            None => Ok(()),
            Some(msg) => {
                tracing::error!(
                    "{what}: SQLite rows changed but the vector index is stale ({msg}). \
                     Run `uteke repair` to resync."
                );
                Err(Error::embed_msg(format!(
                    "{what}: database rows were changed, but the vector index could not be \
                     updated: {msg}. Run `uteke repair` to resync the index."
                )))
            }
        }
    }
}

fn retry<T>(
    policy: &SyncPolicy,
    what: &str,
    mut op: impl FnMut() -> Result<T, Error>,
) -> Result<T, Error> {
    let mut attempt = 1;
    loop {
        match op() {
            Ok(v) => return Ok(v),
            // Deterministic failures (bad input) can never succeed on retry, and
            // the backoff sleeps while the caller holds the index lock (#1322).
            Err(e) if attempt < policy.attempts && !matches!(e, Error::Validation(_)) => {
                tracing::warn!(
                    "{what}: attempt {attempt}/{} failed: {e}. Retrying...",
                    policy.attempts
                );
                std::thread::sleep(policy.backoff);
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Save the index to disk with the standard retry. `Err` carries the last
/// error after all attempts.
pub(crate) fn persist<I: IndexWriter>(index: &mut I, policy: &SyncPolicy) -> Result<(), Error> {
    retry(policy, "index save", || index.save())
}

/// Like [`persist`] but lets the index batch: used by per-operation paths
/// (`remember`, `forget`) so a burst of writes does not rewrite the whole
/// index each time (#1322). Pending changes are flushed by `shutdown()`,
/// the index's `Drop`, and the server's periodic flush.
pub(crate) fn persist_deferred<I: IndexWriter>(
    index: &mut I,
    policy: &SyncPolicy,
) -> Result<(), Error> {
    retry(policy, "index save", || index.save_deferred())
}

/// Insert (or replace) `id` and persist the index.
///
/// A dimension mismatch is deterministic and is reported immediately instead
/// of being retried. An empty embedding means "no embedder configured": the
/// row simply has no vector entry and the outcome is clean.
pub(crate) fn upsert<I: IndexWriter>(
    index: &mut I,
    id: &str,
    embedding: &[f32],
    policy: &SyncPolicy,
) -> SyncOutcome {
    let mut outcome = SyncOutcome {
        attempts: policy.attempts,
        ..SyncOutcome::default()
    };
    if embedding.is_empty() {
        return outcome;
    }
    if embedding.len() != index.dims() {
        outcome.attempts = 1; // deterministic: never retried
        outcome.insert_error = Some(format!(
            "embedding has {} dimensions, index expects {}",
            embedding.len(),
            index.dims()
        ));
        return outcome;
    }
    if let Err(e) = retry(policy, &format!("vector insert id={id}"), || {
        index.insert(id, embedding)
    }) {
        tracing::warn!(
            "Vector insert failed for id={id}: {e}. Row stays FTS5-only; run `uteke repair` to backfill."
        );
        outcome.insert_error = Some(e.to_string());
        return outcome;
    }
    if let Err(e) = persist_deferred(index, policy) {
        tracing::warn!(
            "Failed to persist vector index after insert id={id}: {e}. \
             The entry can be rebuilt via `uteke repair`."
        );
        outcome.persist_error = Some(e.to_string());
    }
    outcome
}

/// Insert without persisting, for batch paths that save once at the end
/// (import, re-embed). Deterministic dimension errors are not retried.
pub(crate) fn insert_unsaved<I: IndexWriter>(
    index: &mut I,
    id: &str,
    embedding: &[f32],
    policy: &SyncPolicy,
) -> Result<(), Error> {
    if embedding.len() != index.dims() {
        return Err(Error::embed_msg(format!(
            "embedding has {} dimensions, index expects {}",
            embedding.len(),
            index.dims()
        )));
    }
    retry(policy, &format!("vector insert id={id}"), || {
        index.insert(id, embedding)
    })
}

/// Remove without persisting (batch paths and rollbacks). Returns `false`
/// when `id` was not in the index.
pub(crate) fn remove_unsaved<I: IndexWriter>(index: &mut I, id: &str) -> bool {
    index.remove(id)
}

/// Remove every id from the index and persist once.
pub(crate) fn remove_ids<'a, I: IndexWriter>(
    index: &mut I,
    ids: impl IntoIterator<Item = &'a str>,
    policy: &SyncPolicy,
) -> SyncOutcome {
    let mut outcome = SyncOutcome {
        attempts: policy.attempts,
        ..SyncOutcome::default()
    };
    for id in ids {
        if !index.remove(id) {
            outcome.missing += 1;
        }
    }
    if outcome.missing > 0 {
        tracing::debug!(
            "{} index entr{} not found during removal (ok if never embedded)",
            outcome.missing,
            if outcome.missing == 1 { "y" } else { "ies" }
        );
    }
    if let Err(e) = persist_deferred(index, policy) {
        outcome.persist_error = Some(e.to_string());
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const FAST: SyncPolicy = SyncPolicy {
        attempts: 3,
        backoff: Duration::ZERO,
    };

    /// Second adapter at the seam: records calls and fails on demand.
    #[derive(Default)]
    struct FakeIndex {
        dims: usize,
        present: HashSet<String>,
        insert_calls: u32,
        save_calls: u32,
        insert_failures_left: u32,
        save_failures_left: u32,
        insert_validation_error: bool,
    }

    impl FakeIndex {
        fn new(dims: usize) -> Self {
            Self {
                dims,
                ..Self::default()
            }
        }
    }

    impl IndexWriter for FakeIndex {
        fn insert(&mut self, id: &str, _e: &[f32]) -> Result<(), Error> {
            self.insert_calls += 1;
            if self.insert_validation_error {
                return Err(Error::validation("deterministic"));
            }
            if self.insert_failures_left > 0 {
                self.insert_failures_left -= 1;
                return Err(Error::embed_msg("insert boom"));
            }
            self.present.insert(id.to_string());
            Ok(())
        }
        fn remove(&mut self, id: &str) -> bool {
            self.present.remove(id)
        }
        fn save(&mut self) -> Result<(), Error> {
            self.save_calls += 1;
            if self.save_failures_left > 0 {
                self.save_failures_left -= 1;
                return Err(Error::embed_msg("save boom"));
            }
            Ok(())
        }
        fn dims(&self) -> usize {
            self.dims
        }
    }

    // #1322: a deterministic (validation) error is never retried, so the
    // backoff sleep cannot run under the index lock for something that can
    // never succeed.
    #[test]
    fn upsert_does_not_retry_validation_errors() {
        let mut idx = FakeIndex::new(2);
        idx.insert_validation_error = true;
        let out = upsert(&mut idx, "a", &[0.0, 1.0], &FAST);
        assert!(out.insert_error.is_some());
        assert_eq!(idx.insert_calls, 1, "validation errors must not be retried");
    }

    #[test]
    fn upsert_clean_inserts_and_saves_once() {
        let mut idx = FakeIndex::new(2);
        let out = upsert(&mut idx, "a", &[0.0, 1.0], &FAST);
        assert!(out.is_clean());
        assert!(idx.present.contains("a"));
        assert_eq!((idx.insert_calls, idx.save_calls), (1, 1));
    }

    #[test]
    fn upsert_retries_transient_insert_failure() {
        let mut idx = FakeIndex::new(2);
        idx.insert_failures_left = 2;
        let out = upsert(&mut idx, "a", &[0.0, 1.0], &FAST);
        assert!(out.is_clean());
        assert_eq!(idx.insert_calls, 3);
    }

    #[test]
    fn upsert_gives_up_after_policy_attempts_and_skips_save() {
        let mut idx = FakeIndex::new(2);
        idx.insert_failures_left = 99;
        let out = upsert(&mut idx, "a", &[0.0, 1.0], &FAST);
        assert_eq!(idx.insert_calls, 3);
        assert_eq!(
            idx.save_calls, 0,
            "nothing to persist when the insert failed"
        );
        assert!(out.insert_error.is_some() && out.persist_error.is_none());
        assert!(
            out.error_message()
                .unwrap()
                .contains("vector insert failed after 3 attempts")
        );
    }

    #[test]
    fn dimension_mismatch_is_not_retried() {
        let mut idx = FakeIndex::new(4);
        let out = upsert(&mut idx, "a", &[0.0, 1.0], &FAST);
        assert_eq!(
            idx.insert_calls, 0,
            "deterministic error: no insert, no retries"
        );
        assert!(
            out.error_message()
                .unwrap()
                .starts_with("vector insert failed: embedding has 2 dimensions")
        );
    }

    #[test]
    fn empty_embedding_is_a_clean_noop() {
        let mut idx = FakeIndex::new(2);
        let out = upsert(&mut idx, "a", &[], &FAST);
        assert!(out.is_clean());
        assert_eq!((idx.insert_calls, idx.save_calls), (0, 0));
    }

    #[test]
    fn persist_failure_is_reported_not_swallowed() {
        let mut idx = FakeIndex::new(2);
        idx.save_failures_left = 99;
        let out = upsert(&mut idx, "a", &[0.0, 1.0], &FAST);
        assert_eq!(idx.save_calls, 3);
        assert!(
            idx.present.contains("a"),
            "in-memory index is still correct"
        );
        assert!(out.insert_error.is_none() && out.persist_error.is_some());
        assert!(
            out.error_message()
                .unwrap()
                .contains("index persist failed after 3 attempts")
        );
    }

    #[test]
    fn remove_ids_counts_missing_and_persists_once() {
        let mut idx = FakeIndex::new(2);
        idx.present.insert("a".into());
        idx.present.insert("b".into());
        let out = remove_ids(&mut idx, ["a", "b", "ghost"], &FAST);
        assert!(out.is_clean());
        assert_eq!(out.missing, 1);
        assert!(idx.present.is_empty());
        assert_eq!(idx.save_calls, 1, "one save for the whole batch");
    }

    #[test]
    fn delete_result_is_an_error_only_when_persist_failed() {
        let mut idx = FakeIndex::new(2);
        idx.present.insert("a".into());
        assert!(
            remove_ids(&mut idx, ["a"], &FAST)
                .into_delete_result("forget")
                .is_ok()
        );

        idx.present.insert("b".into());
        idx.save_failures_left = 99;
        let err = remove_ids(&mut idx, ["b"], &FAST)
            .into_delete_result("forget")
            .unwrap_err()
            .to_string();
        assert!(err.contains("uteke repair"), "{err}");
    }

    #[test]
    fn insert_unsaved_never_saves_and_rejects_wrong_dims() {
        let mut idx = FakeIndex::new(2);
        insert_unsaved(&mut idx, "a", &[0.0, 1.0], &FAST).unwrap();
        assert_eq!((idx.insert_calls, idx.save_calls), (1, 0));
        assert!(idx.present.contains("a"));

        let err = insert_unsaved(&mut idx, "b", &[0.0], &FAST).unwrap_err();
        assert!(err.to_string().contains("dimensions"));
        assert_eq!(
            idx.insert_calls, 1,
            "wrong dimension: no insert attempt, no retries"
        );
    }

    #[test]
    fn insert_unsaved_retries_transient_failures_then_persist_saves_once() {
        let mut idx = FakeIndex::new(2);
        idx.insert_failures_left = 2;
        insert_unsaved(&mut idx, "a", &[0.0, 1.0], &FAST).unwrap();
        assert_eq!(idx.insert_calls, 3);
        persist(&mut idx, &FAST).unwrap();
        assert_eq!(idx.save_calls, 1);
    }

    #[test]
    fn works_with_the_real_vector_index() {
        let mut idx = VectorIndex::new(4).unwrap();
        let out = upsert(&mut idx, "a", &[1.0, 0.0, 0.0, 0.0], &FAST);
        assert!(out.is_clean());
        assert_eq!(idx.len(), 1);
        let out = remove_ids(&mut idx, ["a", "ghost"], &FAST);
        assert!(out.is_clean());
        assert_eq!((idx.len(), out.missing), (0, 1));
    }
}
