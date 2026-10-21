//! Shared test helpers: an embedder-equipped `Uteke` that needs no ONNX
//! runtime (CI has none), so recall paths can be tested end to end.
//! Public only behind the `test-support` feature, for the isolation suites of
//! the server, MCP and CLI crates (#1467).

/// Deterministic bag-of-words embedder (hashed into 64 dims) so recall can
/// run without the ONNX model: texts sharing words get similar vectors.
pub struct KeywordEmbedder;

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

pub fn open_keyword() -> crate::Uteke {
    open_keyword_at(":memory:")
}

/// [`open_keyword`] on a database file, for tests that inspect the file.
pub fn open_keyword_at(path: impl AsRef<std::path::Path>) -> crate::Uteke {
    let (_db, store) = crate::Uteke::open_store(path).expect("open_store");
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
