//! Embedding engine components.

pub mod embed_trait;
pub mod engine;
pub mod fallback;
pub mod ollama;
pub mod openai;

pub use embed_trait::Embedder;
pub use engine::OnnxEmbedder;
pub use fallback::FallbackEmbedder;
pub use ollama::OllamaEmbedder;
pub use openai::OpenAiEmbedder;

/// Backward-compat alias — old code referencing `EmbeddingEngine` continues to work.
#[allow(dead_code)]
pub type EmbeddingEngine = OnnxEmbedder;

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
