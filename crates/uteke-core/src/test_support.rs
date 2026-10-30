//! Shared test helpers: an embedder-equipped `Uteke` that needs no ONNX
//! runtime (CI has none), so recall paths can be tested end to end.
//! Public only behind the `test-support` feature, for the isolation suites of
//! the server, MCP and CLI crates (#1467).

use std::sync::{Arc, Mutex};

/// The bag-of-words vector of `text`, hashed into 64 dimensions: texts sharing
/// words get similar vectors, so recall can run without the ONNX model.
fn keyword_vector(text: &str) -> Vec<f32> {
    let mut v = vec![0.0_f32; 64];
    for w in text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let h = w.bytes().fold(2166136261_u32, |h, b| {
            (h ^ u32::from(b)).wrapping_mul(16777619)
        });
        v[(h % 64) as usize] += 1.0;
    }
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    v.into_iter().map(|x| x / n).collect()
}

/// Deterministic bag-of-words embedder (64 dims) so recall can run without the
/// ONNX model: texts sharing words get similar vectors.
pub struct KeywordEmbedder;

impl crate::embed::Embedder for KeywordEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, crate::Error> {
        Ok(keyword_vector(text))
    }
    fn dims(&self) -> usize {
        64
    }
    fn max_seq_len(&self) -> usize {
        128
    }
    fn name(&self) -> &str {
        "keyword-test-embedder"
    }
}

/// What a [`RecordingEmbedder`] was asked to embed, in order.
pub type Seen = Arc<Mutex<Vec<String>>>;

/// A [`KeywordEmbedder`] that records every text it is asked to embed and can
/// fail its first calls, to test how the core handles an embedder that is
/// slow to come up or keeps failing (retry, FTS5-only storage, the warning).
pub struct RecordingEmbedder {
    seen: Seen,
    failures_left: Mutex<usize>,
}

impl RecordingEmbedder {
    /// An embedder that always succeeds, and the record of what it embedded.
    pub fn new() -> (Self, Seen) {
        Self::failing_first(0)
    }

    /// An embedder whose first `n` calls fail with an embedding error. The
    /// failed calls are recorded too.
    pub fn failing_first(n: usize) -> (Self, Seen) {
        let seen: Seen = Arc::default();
        (
            Self {
                seen: seen.clone(),
                failures_left: Mutex::new(n),
            },
            seen,
        )
    }
}

impl crate::embed::Embedder for RecordingEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, crate::Error> {
        self.seen.lock().unwrap().push(text.to_string());
        let mut left = self.failures_left.lock().unwrap();
        if *left > 0 {
            *left -= 1;
            return Err(crate::Error::embed_msg("transient embedding failure"));
        }
        Ok(keyword_vector(text))
    }
    fn dims(&self) -> usize {
        64
    }
    fn max_seq_len(&self) -> usize {
        128
    }
    fn name(&self) -> &str {
        "recording-test-embedder"
    }
}

/// An in-memory store that embeds with `embedder`. Production constructors
/// build the real backend; this is the seam where a test chooses its own
/// (no ONNX runtime needed), for tests in every crate.
pub fn open_with_embedder(embedder: Box<dyn crate::embed::Embedder>) -> crate::Uteke {
    open_with_embedder_at(":memory:", embedder)
}

/// [`open_with_embedder`] on a database file, for tests that inspect the file.
pub fn open_with_embedder_at(
    path: impl AsRef<std::path::Path>,
    embedder: Box<dyn crate::embed::Embedder>,
) -> crate::Uteke {
    let (_db, store) = crate::Uteke::open_store(path).expect("open_store");
    crate::Uteke::finish_open_full(
        store,
        Some(embedder),
        "test-embedder".to_string(),
        crate::TierConfig::default(),
        crate::RecallConfig::default(),
        crate::EmbeddingSettings::default(),
        crate::graph_rerank::GraphRerankConfig::default(),
        None,
    )
    .expect("finish_open_full")
}

pub fn open_keyword() -> crate::Uteke {
    open_keyword_at(":memory:")
}

/// [`open_keyword`] on a database file, for tests that inspect the file.
pub fn open_keyword_at(path: impl AsRef<std::path::Path>) -> crate::Uteke {
    open_with_embedder_at(path, Box::new(KeywordEmbedder))
}

/// Query that matches every record of [`seed_two_scopes`] in both namespaces.
pub const ISOLATION_QUERY: &str = "quartz meridian orchid ledger";
/// The two namespaces of [`seed_two_scopes`].
pub const ISOLATION_SCOPES: [&str; 2] = ["alpha", "beta"];

/// A store with two namespaces (`alpha`, `beta`) whose memories share the SAME
/// vocabulary, so every retrieval surface has hits in both scopes for
/// [`ISOLATION_QUERY`] (#1467). Per namespace: three plain memories tagged
/// `project:<ns>` + `shared`, and one memory in room `room-<ns>` (author
/// `tester`). One global document, `shared-doc`, shares the vocabulary too
/// (documents have no namespace by design, #1271).
pub fn seed_two_scopes() -> crate::Uteke {
    let u = open_keyword();
    for scope in ISOLATION_SCOPES {
        let project = format!("project:{scope}");
        for i in 0..3 {
            u.remember(
                &format!("{ISOLATION_QUERY} note {i} of {scope}"),
                &[project.as_str(), "shared"],
                None,
                Some(scope),
            )
            .expect("remember");
        }
        u.remember_in_room(
            &format!("{ISOLATION_QUERY} room entry of {scope}"),
            &[project.as_str(), "shared"],
            None,
            Some(scope),
            "fact",
            &format!("room-{scope}"),
            "tester",
        )
        .expect("remember_in_room");
    }
    u.doc_upsert(
        "shared-doc",
        "Shared",
        &format!("{ISOLATION_QUERY} reference text"),
        &[],
        None,
    )
    .expect("doc_upsert");
    u
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_supplied_embedder_is_the_one_a_write_uses() {
        let (embedder, seen) = RecordingEmbedder::new();
        let u = open_with_embedder(Box::new(embedder));
        u.remember("a note about rust", &[], None, None).unwrap();
        assert_eq!(seen.lock().unwrap().as_slice(), ["a note about rust"]);
    }

    #[test]
    fn a_failing_embedder_fails_its_first_calls_only_and_records_each() {
        let (embedder, seen) = RecordingEmbedder::failing_first(1);
        use crate::embed::Embedder;
        assert!(embedder.embed("one").is_err());
        assert_eq!(embedder.embed("two").unwrap().len(), 64);
        assert_eq!(seen.lock().unwrap().as_slice(), ["one", "two"]);
    }

    #[test]
    fn the_keyword_vector_ranks_a_shared_word_above_an_unrelated_text() {
        let q = keyword_vector("rust memory");
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        assert!(dot(&q, &keyword_vector("rust safety")) > dot(&q, &keyword_vector("tax deadline")));
    }

    #[test]
    fn open_keyword_at_keeps_data_in_the_given_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        open_keyword_at(&path)
            .remember("kept", &[], None, None)
            .unwrap();
        assert!(path.exists());
    }
}
