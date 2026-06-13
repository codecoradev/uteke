//! Embedding engine components.

pub mod embed_trait;
pub mod engine;

pub use embed_trait::Embedder;
pub use engine::OnnxEmbedder;

/// Backward-compat alias — old code referencing `EmbeddingEngine` continues to work.
#[allow(dead_code)]
pub type EmbeddingEngine = OnnxEmbedder;
