//! Shared test helpers: an embedder-equipped `Uteke` that needs no ONNX
//! runtime (CI has none), so recall paths can be tested end to end.

/// Deterministic bag-of-words embedder (hashed into 64 dims) so recall can
/// run without the ONNX model: texts sharing words get similar vectors.
pub(crate) struct KeywordEmbedder;

impl crate::embed::Embedder for KeywordEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, crate::Error> {
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
        Ok(v.into_iter().map(|x| x / n).collect())
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

pub(crate) fn open_keyword() -> crate::Uteke {
    let (_db, store) = crate::Uteke::open_store(":memory:").expect("open_store");
    crate::Uteke::finish_open_full(
        store,
        Some(Box::new(KeywordEmbedder)),
        "test-keyword".to_string(),
        crate::TierConfig::default(),
        crate::RecallConfig::default(),
        crate::EmbeddingSettings::default(),
        crate::graph_rerank::GraphRerankConfig::default(),
        None,
    )
    .expect("finish_open_full")
}
