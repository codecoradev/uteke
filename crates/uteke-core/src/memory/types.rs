//! Core types for the Uteke memory engine.

use serde::{Deserialize, Serialize};

/// Default namespace for memories without explicit namespace.
pub const DEFAULT_NAMESPACE: &str = "default";

/// A stored memory with content, embedding, tags, and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Unique identifier (UUID v4).
    pub id: String,
    /// The text content of the memory.
    pub content: String,
    /// 768-dimensional embedding vector.
    pub embedding: Vec<f32>,
    /// Tags for categorization.
    pub tags: Vec<String>,
    /// Arbitrary JSON metadata.
    pub metadata: serde_json::Value,
    /// When this memory was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When this memory was last updated.
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Namespace for multi-agent isolation.
    #[serde(default = "default_namespace")]
    pub namespace: String,
}

fn default_namespace() -> String {
    DEFAULT_NAMESPACE.to_string()
}

/// A search result with relevance score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The matched memory.
    pub memory: Memory,
    /// Cosine similarity score (0.0–1.0).
    pub score: f32,
}

/// Statistics about the memory store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreStats {
    /// Total number of memories.
    pub total_memories: usize,
    /// Number of unique tags.
    pub unique_tags: usize,
    /// Database file size in bytes.
    pub db_size_bytes: u64,
}

/// Lightweight export format — no embedding vector (re-embedded on import).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportEntry {
    /// The text content.
    pub content: String,
    /// Tags for categorization.
    pub tags: Vec<String>,
    /// Arbitrary JSON metadata.
    pub metadata: serde_json::Value,
    /// When this memory was originally created.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Result of an import operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    /// Number of memories imported.
    pub imported: usize,
    /// Number of entries skipped (duplicate or invalid).
    pub skipped: usize,
}
