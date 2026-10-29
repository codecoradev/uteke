//! Core memory operations, split by concern (#1519): `write`, `recall`, `forget`,
//! `listing`, `lookup`, `tags`, `update` and `time_travel`. The shared types and the
//! embedding retry live here.

use crate::error::Error;
use crate::index_sync::{self, SyncPolicy};
use crate::memory::types::{
    BulkDeleteResult, DEFAULT_NAMESPACE, Memory, MemoryTier, NamespaceDeleteResult,
    NamespaceRenameResult, RecallStrategy, SearchResult, TagInfo,
};
use crate::memory::vector::cosine_distance_to_similarity;
use std::sync::Mutex;
use std::time::Duration;

mod forget;
mod listing;
mod lookup;
mod recall;
mod tags;
mod time_travel;
mod update;
mod write;

pub(crate) use recall::rrf_fuse_weighted;

/// Retry embedding generation with exponential backoff (#621).
///
/// Embedding failures silently drop vector entries, causing the vector
/// index to desync from SQLite. This helper retries up to 3 times with
/// 200ms, 400ms, then 800ms delays between attempts.
///
/// If the embedder slot is empty (it should not be after `ensure_embedder`),
/// returns an embedding error instead of panicking. Validation errors are
/// returned immediately: they can never succeed on retry.
pub(crate) fn retry_embed(
    embedder: &Mutex<Option<std::sync::Arc<dyn crate::embed::Embedder>>>,
    text: &str,
) -> Result<Vec<f32>, Error> {
    const MAX_RETRIES: usize = 3;
    let mut delay = Duration::from_millis(200);

    for attempt in 0..MAX_RETRIES {
        // Clone the handle and release the slot lock: the call itself (a
        // network round trip for a remote backend) must not block other
        // embeds (#1518).
        let handle = embedder
            .lock()
            .map_err(|_| Error::lock("embedder lock during remember"))?
            .clone();
        let Some(handle) = handle else {
            return Err(Error::embed_msg("No embedder configured"));
        };
        match handle.embed(text) {
            Ok(embedding) => return Ok(embedding),
            Err(e @ Error::Validation(_)) => return Err(e),
            Err(e) => {
                if attempt < MAX_RETRIES - 1 {
                    tracing::warn!(
                        "Embedding attempt {}/{} failed: {}. Retrying in {:?}...",
                        attempt + 1,
                        MAX_RETRIES,
                        e,
                        delay
                    );
                    std::thread::sleep(delay);
                    delay *= 2;
                } else {
                    tracing::error!(
                        "Embedding failed after {} attempts: {}. \
                         Memory will be stored WITHOUT vector embedding. \
                         Run `uteke repair` to rebuild index.",
                        MAX_RETRIES,
                        e
                    );
                    return Err(e);
                }
            }
        }
    }
    unreachable!()
}

/// Result of a remember operation with honest embedding-write reporting (#1273).
///
/// `embedding_written` is `false` when the memory row was stored (SQLite +
/// FTS5, keyword-searchable) but the vector entry is NOT durably usable:
/// embedding generation failed after retries, the index insert failed, or
/// index persistence failed after retries. Such rows are recoverable via
/// `uteke repair` / `POST /repair`. `warning` carries a human-readable
/// reason whenever `embedding_written` is `false` due to a FAILURE.
/// Note: a store with no embedder backend configured (#1166) also reports
/// `embedding_written: false` — by configuration, not failure — with no
/// warning.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RememberOutcome {
    /// ID of the stored memory.
    pub id: String,
    /// Whether the vector embedding is durably usable for semantic search.
    pub embedding_written: bool,
    /// Failure reason when the embedding could not be persisted.
    pub warning: Option<String>,
    /// Secrets the privacy filter masked in the content of THIS call (#1464): a
    /// kind and a count each, never a value. Empty when nothing was masked. On a
    /// dedup hit nothing new is stored, so the stored memory keeps its own flags.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub redactions: Vec<crate::privacy::RedactionCount>,
}

/// What an update did (#1464): whether the memory was found and updated, and
/// what the privacy filter masked in the new content, tags or metadata.
#[derive(Debug, Clone, Default)]
pub struct UpdateOutcome {
    pub updated: bool,
    pub redactions: Vec<crate::privacy::RedactionCount>,
}

impl UpdateOutcome {
    /// Whether the privacy filter masked anything in this update.
    pub fn redacted(&self) -> bool {
        !self.redactions.is_empty()
    }

    /// What was masked, for a human (see [`RememberOutcome::redaction_summary`]).
    pub fn redaction_summary(&self) -> Option<String> {
        redaction_summary(&self.redactions)
    }
}

fn redaction_summary(redactions: &[crate::privacy::RedactionCount]) -> Option<String> {
    (!redactions.is_empty()).then(|| {
        redactions
            .iter()
            .map(|r| format!("{} x{}", r.kind, r.count))
            .collect::<Vec<_>>()
            .join(", ")
    })
}

impl RememberOutcome {
    /// Whether the privacy filter masked anything in the stored content.
    pub fn redacted(&self) -> bool {
        !self.redactions.is_empty()
    }

    /// What was masked, for a human: `github-token x1, jwt x2`. `None` when
    /// nothing was.
    pub fn redaction_summary(&self) -> Option<String> {
        redaction_summary(&self.redactions)
    }
}

/// Fusion weights benchmark-tuned on LongMemEval fast50 (#1123):
/// plateau [1.7, 1.9] → R@5 0.98; 1.7 chosen mid-plateau.
/// k=60 matches the k used by recall_rrf.
pub(crate) const FUSION_W_VECTOR: f64 = 1.7;
pub(crate) const FUSION_W_HYBRID: f64 = 1.0;
pub(crate) const FUSION_RRF_K: f64 = 60.0;

#[cfg(test)]
mod retry_embed_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Fake embedder that counts calls and fails the first `failures` calls
    /// with the error built by `make_err`, then succeeds.
    struct ScriptedEmbedder {
        calls: Arc<AtomicUsize>,
        failures: usize,
        make_err: fn() -> Error,
    }

    impl crate::embed::Embedder for ScriptedEmbedder {
        fn embed(&self, _text: &str) -> Result<Vec<f32>, Error> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n < self.failures {
                Err((self.make_err)())
            } else {
                Ok(vec![0.0; 4])
            }
        }
        fn dims(&self) -> usize {
            4
        }
        fn max_seq_len(&self) -> usize {
            8
        }
        fn name(&self) -> &str {
            "scripted"
        }
    }

    fn scripted(
        failures: usize,
        make_err: fn() -> Error,
    ) -> (
        Mutex<Option<Arc<dyn crate::embed::Embedder>>>,
        Arc<AtomicUsize>,
    ) {
        let calls = Arc::new(AtomicUsize::new(0));
        let embedder: Arc<dyn crate::embed::Embedder> = Arc::new(ScriptedEmbedder {
            calls: Arc::clone(&calls),
            failures,
            make_err,
        });
        (Mutex::new(Some(embedder)), calls)
    }

    #[test]
    fn validation_error_is_not_retried() {
        let (embedder, calls) = scripted(usize::MAX, || Error::Validation("bad input".into()));
        let err = retry_embed(&embedder, "hello").unwrap_err();
        assert!(matches!(err, Error::Validation(_)), "{err}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn transient_error_is_retried_then_succeeds() {
        let (embedder, calls) = scripted(1, || Error::embed_msg("timeout"));
        let embedding = retry_embed(&embedder, "hello").unwrap();
        assert_eq!(embedding.len(), 4);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// #1518: the slot lock is held to look at the handle, never across the
    /// embed call, so two slow embeds run at the same time.
    #[test]
    fn two_embeds_run_concurrently() {
        use std::sync::atomic::AtomicUsize;
        struct Slow {
            inflight: Arc<AtomicUsize>,
            peak: Arc<AtomicUsize>,
        }
        impl crate::embed::Embedder for Slow {
            fn embed(&self, _text: &str) -> Result<Vec<f32>, Error> {
                let now = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(150));
                self.inflight.fetch_sub(1, Ordering::SeqCst);
                Ok(vec![0.0; 4])
            }
            fn dims(&self) -> usize {
                4
            }
            fn max_seq_len(&self) -> usize {
                8
            }
            fn name(&self) -> &str {
                "slow"
            }
        }
        let peak = Arc::new(AtomicUsize::new(0));
        let slot: Mutex<Option<Arc<dyn crate::embed::Embedder>>> =
            Mutex::new(Some(Arc::new(Slow {
                inflight: Arc::new(AtomicUsize::new(0)),
                peak: Arc::clone(&peak),
            })));
        std::thread::scope(|s| {
            for _ in 0..2 {
                s.spawn(|| retry_embed(&slot, "x").unwrap());
            }
        });
        assert_eq!(peak.load(Ordering::SeqCst), 2, "the embeds were serialised");
    }

    #[test]
    fn no_embedder_is_an_error_not_a_panic() {
        let embedder: Mutex<Option<std::sync::Arc<dyn crate::embed::Embedder>>> = Mutex::new(None);
        let err = retry_embed(&embedder, "hello").unwrap_err();
        assert!(matches!(err, Error::Embedding { .. }), "{err:?}");
        assert!(err.to_string().contains("No embedder"), "{err}");
    }
}
