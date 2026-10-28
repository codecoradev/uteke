//! The dimension of a store's vectors is decided once, when it is opened
//! (#1540): the backend (or the injected embedder) says what it produces, the
//! stored vectors and the index file say what the store holds, and a
//! disagreement is an error at open with an actionable message, not a late and
//! confusing failure at the first embedding. `UTEKE_ALLOW_DIM_MISMATCH=1`
//! skips the check so a store can be opened with another backend to run
//! `uteke repair`, which rebuilds the vectors.

use crate::Error;

/// What the store holds, as far as it is known.
pub(crate) struct Held {
    /// Width of the vectors already stored in SQLite, if any.
    pub stored: Option<usize>,
    /// Width of the index file that was loaded, if there was one with content.
    pub index: Option<usize>,
}

/// `wanted` is the width the backend produces. Errors when the store holds
/// vectors of another width, unless `allow_mismatch`.
pub(crate) fn verify(
    wanted: usize,
    held: &Held,
    backend: &str,
    allow_mismatch: bool,
) -> Result<(), Error> {
    if allow_mismatch {
        return Ok(());
    }
    let holds = held
        .stored
        .filter(|s| *s != wanted)
        .map(|s| ("stored vectors", s))
        .or_else(|| {
            held.index
                .filter(|i| *i != wanted)
                .map(|i| ("the vector index", i))
        });
    match holds {
        None => Ok(()),
        Some((what, found)) => Err(Error::Validation(format!(
            "Embedding dimension mismatch: {what} are {found}d but the '{backend}' backend produces {wanted}d. \
             Switch back to the backend (or `dims`) the store was built with, or rebuild its vectors with the new one: \
             `UTEKE_ALLOW_DIM_MISMATCH=1 uteke repair`."
        ))),
    }
}

/// `UTEKE_ALLOW_DIM_MISMATCH=1`.
pub(crate) fn mismatch_allowed() -> bool {
    std::env::var("UTEKE_ALLOW_DIM_MISMATCH").as_deref() == Ok("1")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(stored: Option<usize>, index: Option<usize>) -> Held {
        Held { stored, index }
    }

    #[test]
    fn a_store_built_with_one_width_is_refused_by_another_backend() {
        let err = verify(1536, &held(Some(768), None), "openai", false).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("768d") && msg.contains("1536d") && msg.contains("openai"),
            "{msg}"
        );
        assert!(
            msg.contains("UTEKE_ALLOW_DIM_MISMATCH=1 uteke repair"),
            "{msg}"
        );
        let err = verify(1536, &held(None, Some(768)), "openai", false).unwrap_err();
        assert!(err.to_string().contains("the vector index are 768d"));
    }

    #[test]
    fn matching_empty_or_unknown_widths_pass() {
        assert!(verify(768, &held(Some(768), Some(768)), "onnx", false).is_ok());
        assert!(verify(768, &held(None, None), "onnx", false).is_ok());
    }

    #[test]
    fn the_escape_hatch_skips_the_check() {
        assert!(verify(1536, &held(Some(768), Some(768)), "openai", true).is_ok());
    }

    fn open_as(path: &std::path::Path, backend: &str, dims: usize) -> Result<crate::Uteke, Error> {
        crate::Uteke::open_with_embedding_and_graph(
            path,
            backend,
            crate::EmbeddingSettings {
                dims,
                api_key: "k".into(),
                ..Default::default()
            },
            crate::TierConfig::default(),
            crate::RecallConfig::default(),
            crate::graph_rerank::GraphRerankConfig::default(),
            None,
        )
    }

    #[test]
    #[serial_test::serial]
    fn opening_a_store_with_a_backend_of_another_width_is_refused_at_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uteke.db");
        {
            // Built with a 64-wide embedder.
            let u = crate::test_support::open_keyword_at(&path);
            u.remember("a stored memory", &[], None, None).unwrap();
            u.shutdown().unwrap();
        }
        let err = open_as(&path, "openai", 1536).err().expect("must refuse");
        let msg = err.to_string();
        assert!(msg.contains("64d") && msg.contains("1536d"), "{msg}");
        assert!(
            msg.contains("UTEKE_ALLOW_DIM_MISMATCH=1 uteke repair"),
            "{msg}"
        );
        // Nothing was touched: the same width still opens.
        assert!(open_as(&path, "openai", 64).is_ok());
    }

    #[test]
    #[serial_test::serial]
    #[allow(unsafe_code)] // env mutation, serialized with the other env tests
    fn the_escape_hatch_still_opens_the_store_for_repair() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uteke.db");
        {
            let u = crate::test_support::open_keyword_at(&path);
            u.remember("a stored memory", &[], None, None).unwrap();
            u.shutdown().unwrap();
        }
        unsafe { std::env::set_var("UTEKE_ALLOW_DIM_MISMATCH", "1") };
        let opened = open_as(&path, "openai", 1536);
        unsafe { std::env::remove_var("UTEKE_ALLOW_DIM_MISMATCH") };
        assert!(opened.is_ok(), "{:?}", opened.err());
    }
}
