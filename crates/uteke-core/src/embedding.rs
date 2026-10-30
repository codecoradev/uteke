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
//! The slot lock is held to look at the handle, never across an embed call.

use crate::{Error, Uteke};

/// A handle that embeds text ahead of a request, WITHOUT the store (#1518).
///
/// A server serialises requests on one store lock. The embedding of a recall
/// or a remember is the slow part (a network round trip for a remote backend)
/// and does not need the store, so the server asks this handle to embed first,
/// outside the lock. The result lands in the embedder's cache, and the real
/// call under the lock then finds it there. Only an embedder that caches
/// ([`crate::embed::Embedder::caches_results`]) gets a handle.
///
/// Warming must never send the embedder more than the real call would: a
/// content goes through the same validation and privacy filter as a write
/// ([`Self::warm_content`]), so a secret is masked, and a strict-mode refusal
/// sends nothing.
#[derive(Clone)]
pub struct EmbeddingWarmer {
    embedder: std::sync::Arc<dyn crate::embed::Embedder>,
    privacy: crate::privacy::PrivacyConfig,
}

impl EmbeddingWarmer {
    /// Warm the vector of a recall query. A blank query is a request error
    /// and embeds nothing.
    pub fn warm_query(&self, query: &str) {
        if !query.trim().is_empty() {
            self.warm(query);
        }
    }

    /// Warm the vector of a memory about to be written: only the text the
    /// write itself would embed. Content the write would refuse (validation,
    /// strict privacy mode) is not embedded, a secret is masked first, and
    /// JSON content (flattened before embedding) is left to the real call.
    pub fn warm_content(&self, content: &str) {
        if crate::validate_input(content, &[] as &[&str]).is_err() {
            return;
        }
        let Ok(prepared) = self.privacy.prepare(content, None) else {
            return;
        };
        if crate::memory::crud::detect_content_type(&prepared.content) == "json" {
            return;
        }
        self.warm(&prepared.content);
    }

    /// Embed `text` once so its vector is cached. A failure is ignored: the
    /// real call retries and reports it.
    fn warm(&self, text: &str) {
        if let Err(e) = self.embedder.embed(text) {
            tracing::debug!("embedding warm-up failed, the request will retry: {e}");
        }
    }
}

impl Uteke {
    /// A warm-up handle once the embedder is built and caches its results.
    /// It never initialises the embedder, so it is cheap to ask for.
    pub fn embedding_warmer(&self) -> Option<EmbeddingWarmer> {
        let handle = self.embedder.lock().ok()?.clone()?;
        handle.caches_results().then(|| EmbeddingWarmer {
            embedder: handle,
            privacy: self.privacy_config.clone(),
        })
    }

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
        // The handle is cloned and the slot lock released before the call, so a
        // slow backend never blocks another embed (#1518).
        let handle = self
            .embedder
            .lock()
            .map_err(|_| Error::lock("embedder lock"))?
            .clone()
            .ok_or_else(|| Error::embed_msg("No embedder configured"))?;
        f(handle.as_ref())
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
    fn a_warmer_exists_only_for_an_embedder_that_caches() {
        // The recording embedder keeps no cache: no warm-up handle.
        let (u, seen) = store(0);
        u.embed_text("x").unwrap();
        assert!(u.embedding_warmer().is_none());
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn warming_makes_the_real_embed_a_cache_hit() {
        let (inner, seen) = RecordingEmbedder::failing_first(0);
        let dir = std::env::temp_dir().join(format!("warm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cache = crate::embed::cache::EmbeddingCache::open(&dir.join("c.db")).unwrap();
        let cached = crate::embed::CachingEmbedder::with_cache(Box::new(inner), cache);
        let u = open_with_embedder(Box::new(cached));
        let warmer = u
            .embedding_warmer()
            .expect("a caching embedder is warmable");
        warmer.warm_query("what is rust");
        assert_eq!(seen.lock().unwrap().len(), 1, "the warm-up embedded once");
        u.embed_text("what is rust").unwrap();
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "the real call was a cache hit"
        );
        drop(u);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The embedder's own sequence length is what chunk sizing reads.
    #[test]
    fn the_max_sequence_length_comes_from_the_embedder() {
        let u = crate::test_support::open_keyword();
        assert_eq!(u.embedder_max_seq_len().unwrap(), 128);
    }

    fn warmable() -> (crate::Uteke, Seen, std::path::PathBuf) {
        let (inner, seen) = RecordingEmbedder::failing_first(0);
        let dir = std::env::temp_dir().join(format!(
            "warm-content-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cache = crate::embed::cache::EmbeddingCache::open(&dir.join("c.db")).unwrap();
        let cached = crate::embed::CachingEmbedder::with_cache(Box::new(inner), cache);
        (open_with_embedder(Box::new(cached)), seen, dir)
    }

    /// A content is embedded as a write would embed it: plain text yes, JSON
    /// (flattened by the write) and content the write refuses no.
    #[test]
    fn warm_content_embeds_plain_text_and_skips_json_and_invalid_content() {
        let (u, seen, dir) = warmable();
        let w = u.embedding_warmer().unwrap();
        w.warm_content("the quokka is a small marsupial");
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            ["the quokka is a small marsupial".to_string()]
        );
        w.warm_content("{\"animal\": \"quokka\"}");
        w.warm_content("   ");
        assert_eq!(seen.lock().unwrap().len(), 1, "json or blank was embedded");
        drop(u);
        std::fs::remove_dir_all(&dir).ok();
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
