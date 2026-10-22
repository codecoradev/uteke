//! Embedder trait — abstraction over embedding backends.
//!
//! Implementations:
//! - [`OnnxEmbedder`](crate::embed::OnnxEmbedder) — local ONNX EmbeddingGemma (default) *(requires `onnx` feature)*
//! - [`OllamaEmbedder`](crate::embed::OllamaEmbedder) — Ollama API
//! - [`OpenAiEmbedder`](crate::embed::OpenAiEmbedder) — OpenAI API
//! - [`FallbackEmbedder`](crate::embed::FallbackEmbedder) — passthrough (no-op, zero vectors)
//!
//! Future backends (OpenAI, Ollama, etc.) implement this trait without
//! touching recall/remember code.

use crate::Error;

/// Trait for embedding backends.
///
/// All embedding operations in uteke go through this trait, enabling different
/// backends (ONNX local, OpenAI API, Ollama, etc.) to be plugged in without
/// modifying recall/remember logic.
///
/// Implementations must be `Send + Sync` to work with `Mutex<Box<dyn Embedder>>`.
pub trait Embedder: Send + Sync {
    /// Embed a text string, returning an f32 vector.
    fn embed(&self, text: &str) -> Result<Vec<f32>, Error>;

    /// Embedding dimensions (e.g., 768 for EmbeddingGemma).
    fn dims(&self) -> usize;

    /// Maximum sequence length in tokens.
    fn max_seq_len(&self) -> usize;

    /// Human-readable name for this backend.
    fn name(&self) -> &str;

    /// What produced the vectors: the backend, the model, the dimensions and
    /// (for a remote backend) a fingerprint of the endpoint. Two embedders
    /// with the same identity return interchangeable vectors; the embedding
    /// cache is keyed by it, so a changed model, size or endpoint never serves
    /// the vector of the old one. The default (`name:dims`) suits a backend
    /// with one fixed model.
    fn identity(&self) -> String {
        format!("{}:{}", self.name(), self.dims())
    }
}
