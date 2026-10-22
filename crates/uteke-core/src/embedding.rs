//! The one way the engine asks its embedder for a vector (#1538).
//!
//! Every embedding call goes through here, so lazy initialisation, the embedder
//! lock and the retry live in one place and no call site can reach the embedder
//! slot without having initialised it:
//!
//! - [`Uteke::embed`] initialises the embedder if needed and retries a transient
//!   failure (3 attempts, 200/400/800 ms); the caller decides what a final
//!   failure means (an error, or FTS5-only storage for a write).
//! - [`Uteke::embed_once`] is the same without the retry, for a bulk loop that
//!   reports each failed row instead of waiting on a dead backend per row.
//!
//! The lock is held for one embed call and never across a loop.

use crate::{Error, Uteke};

impl Uteke {
    /// Embed `text`, initialising the embedder on first use and retrying a
    /// transient failure. A validation error is returned at once.
    pub(crate) fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
        self.ensure_embedder()?;
        crate::operations::retry_embed(&self.embedder, text)
    }

    /// Embed `text` once: no retry, for loops that count failures per row.
    pub(crate) fn embed_once(&self, text: &str) -> Result<Vec<f32>, Error> {
        self.ensure_embedder()?;
        self.with_embedder(|e| e.embed(text))
    }

    /// The embedder's maximum sequence length, initialising it if needed.
    pub(crate) fn embedder_max_seq_len(&self) -> Result<usize, Error> {
        self.ensure_embedder()?;
        self.with_embedder(|e| Ok(e.max_seq_len()))
    }

    fn with_embedder<T>(
        &self,
        f: impl FnOnce(&dyn crate::embed::Embedder) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let guard = self
            .embedder
            .lock()
            .map_err(|_| Error::lock("embedder lock"))?;
        let embedder = guard
            .as_deref()
            .ok_or_else(|| Error::embed_msg("No embedder configured"))?;
        f(embedder)
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::{RecordingEmbedder, Seen, open_with_embedder};

    fn store(failures: usize) -> (crate::Uteke, Seen) {
        let (e, seen) = RecordingEmbedder::failing_first(failures);
        (open_with_embedder(Box::new(e)), seen)
    }

    #[test]
    fn a_query_retries_a_transient_failure_like_a_write() {
        let (u, seen) = store(1);
        u.recall("rust", 5, None, None, 0.0, None, None).unwrap();
        assert_eq!(seen.lock().unwrap().len(), 2, "one failure, one retry");
    }

    #[test]
    fn document_search_and_embed_text_retry_too() {
        let (u, seen) = store(1);
        u.doc_search_semantic("rust", 3).unwrap();
        assert_eq!(seen.lock().unwrap().len(), 2);
        let (u, seen) = store(1);
        assert_eq!(u.embed_text("rust").unwrap().len(), 64);
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_persistent_failure_is_an_error_after_three_attempts() {
        let (u, seen) = store(3);
        assert!(u.embed_text("rust").is_err());
        assert_eq!(seen.lock().unwrap().len(), 3);
    }

    #[test]
    fn embed_once_does_not_retry() {
        let (u, seen) = store(1);
        assert!(u.embed_once("rust").is_err());
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_write_that_cannot_embed_is_fts5_only_with_a_warning_not_an_error() {
        let (u, _) = store(3);
        let out = u.remember_detailed("kept anyway", &[], None, None).unwrap();
        assert!(!out.embedding_written);
        assert!(out.warning.is_some());
    }

    #[test]
    fn reembed_repairs_rows_with_one_attempt_each() {
        let (u, seen) = store(0);
        u.remember("one", &[], None, None).unwrap();
        let before = seen.lock().unwrap().len();
        u.graph_store()
            .execute("UPDATE memories SET embedding = NULL", rusqlite::params![])
            .unwrap();
        let report = u.reembed_missing().unwrap();
        assert_eq!(report.reembedded, 1);
        assert_eq!(seen.lock().unwrap().len(), before + 1);
    }
}
