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
    #[serde(skip_serializing)]
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
    /// How many times this memory has been accessed (recall, get).
    #[serde(default)]
    pub access_count: u32,
    /// When this memory was last accessed.
    #[serde(default)]
    pub last_accessed: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether this memory has been superseded by a newer one.
    #[serde(default)]
    pub deprecated: bool,
    /// When this fact became valid (temporal metadata).
    #[serde(default)]
    pub valid_from: Option<chrono::DateTime<chrono::Utc>>,
    /// When this fact was invalidated (temporal metadata).
    #[serde(default)]
    pub valid_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Memory type: fact, procedure, preference, decision, context.
    #[serde(default = "default_memory_type")]
    pub memory_type: String,
}

fn default_namespace() -> String {
    DEFAULT_NAMESPACE.to_string()
}

fn default_memory_type() -> String {
    "fact".to_string()
}

/// A search result with relevance score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The matched memory.
    pub memory: Memory,
    /// Cosine similarity score (0.0–1.0).
    pub score: f32,
}

/// Memory tier based on access recency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryTier {
    /// Accessed within last 7 days — boosted in recall.
    Hot,
    /// Accessed within last 30 days — normal recall.
    Warm,
    /// Not accessed in 30+ days — lower priority.
    Cold,
}

impl MemoryTier {
    /// Determine tier from last_accessed timestamp.
    pub fn from_last_accessed(last_accessed: Option<chrono::DateTime<chrono::Utc>>) -> Self {
        let Some(la) = last_accessed else {
            return MemoryTier::Cold;
        };
        let age = chrono::Utc::now() - la;
        if age.num_days() <= 7 {
            MemoryTier::Hot
        } else if age.num_days() <= 30 {
            MemoryTier::Warm
        } else {
            MemoryTier::Cold
        }
    }
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
    /// Number of hot memories (accessed within 7 days).
    pub hot: usize,
    /// Number of warm memories (accessed within 30 days).
    pub warm: usize,
    /// Number of cold memories (not accessed in 30+ days).
    pub cold: usize,
}

/// Result of a bulk delete operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkDeleteResult {
    /// Number of memories deleted.
    pub deleted: usize,
    /// IDs of deleted memories.
    pub ids: Vec<String>,
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

/// A tag with its usage count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagInfo {
    /// Tag name.
    pub name: String,
    /// Number of memories using this tag.
    pub count: usize,
}

/// Aging status — breakdown of memories by access tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgingStatus {
    /// Total memories in namespace.
    pub total: usize,
    /// Hot memories (accessed within 7 days).
    pub hot: usize,
    /// Warm memories (accessed within 30 days but not hot).
    pub warm: usize,
    /// Cold memories (not accessed in 30+ days).
    pub cold: usize,
    /// Memories that have never been accessed.
    pub never_accessed: usize,
}

/// Result of a cleanup operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupResult {
    /// Number of memories deleted.
    pub deleted: usize,
}

/// Result of a prune operation (auto-forget with decay policy).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneResult {
    /// Number of memories pruned.
    pub pruned: usize,
    /// IDs of pruned memories.
    pub ids: Vec<String>,
    /// Number of memories deprecated (contradicted).
    pub deprecated: usize,
    /// IDs of deprecated memories.
    pub deprecated_ids: Vec<String>,
}

/// Result of a contradiction check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContradictionResult {
    /// Whether a contradiction was detected.
    pub contradicted: bool,
    /// ID of the existing memory that was deprecated.
    pub deprecated_id: Option<String>,
    /// Similarity score that triggered the contradiction.
    pub similarity: f32,
}

/// Memory type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryType {
    /// A factual statement (has temporal validity).
    Fact,
    /// A procedure or how-to (doesn't expire).
    Procedure,
    /// A user preference (doesn't expire).
    Preference,
    /// A design or architecture decision (may be superseded).
    Decision,
    /// Contextual information (session-scoped, may expire).
    Context,
}

impl MemoryType {
    /// Parse from string.
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "fact" => Some(Self::Fact),
            "procedure" => Some(Self::Procedure),
            "preference" => Some(Self::Preference),
            "decision" => Some(Self::Decision),
            "context" => Some(Self::Context),
            _ => None,
        }
    }

    /// Convert to string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Procedure => "procedure",
            Self::Preference => "preference",
            Self::Decision => "decision",
            Self::Context => "context",
        }
    }

    /// Whether this memory type has temporal validity.
    pub fn has_temporal_validity(&self) -> bool {
        matches!(self, Self::Fact | Self::Decision | Self::Context)
    }
}

/// Result of a consolidation (deduplication) operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationResult {
    /// Number of duplicate pairs found.
    pub duplicates_found: usize,
    /// Number of memories merged (older duplicates removed).
    pub merged: usize,
    /// IDs of removed duplicate memories.
    pub removed_ids: Vec<String>,
    /// Kept memory IDs (one per duplicate group).
    pub kept_ids: Vec<String>,
}

/// A pair of similar memories (potential duplicate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarPair {
    /// ID of the first (older) memory.
    pub id_a: String,
    /// Content preview of the first memory.
    pub content_a: String,
    /// ID of the second (newer) memory.
    pub id_b: String,
    /// Content preview of the second memory.
    pub content_b: String,
    /// Cosine similarity score.
    pub similarity: f32,
}
