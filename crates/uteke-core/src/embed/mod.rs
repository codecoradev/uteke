//! Embedding engine components.

pub mod cache;
pub mod embed_trait;
#[cfg(feature = "onnx")]
pub mod engine;
pub(crate) mod factory;
pub mod fallback;
pub mod ollama;
pub mod openai;
#[cfg(feature = "onnx")]
pub mod ort_init;

pub use cache::{CachingEmbedder, EmbeddingCache};
pub use embed_trait::Embedder;
#[cfg(feature = "onnx")]
pub use engine::OnnxEmbedder;
pub use fallback::FallbackEmbedder;
pub use ollama::OllamaEmbedder;
pub use openai::OpenAiEmbedder;

/// Backward-compat alias — old code referencing `EmbeddingEngine` continues to work.
#[cfg(feature = "onnx")]
#[allow(dead_code)]
pub type EmbeddingEngine = OnnxEmbedder;

/// A short, stable fingerprint of the parts that identify an endpoint (for an
/// embedder identity). Hashed so a URL with credentials in it never lands in
/// the cache database in the clear.
pub(crate) fn fingerprint(parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for p in parts {
        hasher.update(p.as_bytes());
        hasher.update(b"\0");
    }
    hasher
        .finalize()
        .iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Validate a user-supplied base_url: must be http(s):// and absolute.
///
/// Rejects empty / schemeless / malformed URLs early so callers get a
/// clear validation error at construct time instead of a confusing HTTP
/// failure later (CodeCora finding #155).
pub(crate) fn validate_base_url(base_url: &str) -> Result<(), crate::Error> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err(crate::Error::Validation(
            "base_url must not be empty".into(),
        ));
    }
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return Err(crate::Error::Validation(format!(
            "base_url must start with 'http://' or 'https://' (got '{trimmed}')"
        )));
    }
    if reqwest::Url::parse(trimmed).is_err() {
        return Err(crate::Error::Validation(format!(
            "base_url is not a valid URL: '{trimmed}'"
        )));
    }
    Ok(())
}
