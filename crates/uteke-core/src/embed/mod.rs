//! Embedding engine components.

pub mod embed_trait;
pub mod engine;
pub mod ollama;
pub mod openai;

pub use embed_trait::Embedder;
pub use engine::OnnxEmbedder;
pub use ollama::OllamaEmbedder;
pub use openai::OpenAiEmbedder;

/// Backward-compat alias — old code referencing `EmbeddingEngine` continues to work.
#[allow(dead_code)]
pub type EmbeddingEngine = OnnxEmbedder;
