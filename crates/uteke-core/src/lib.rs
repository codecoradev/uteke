//! Uteke Core — persistent memory library for AI agents.
//!
//! # Example
//! ```ignore
//! use uteke_core::Uteke;
//!
//! let uteke = Uteke::open("~/.uteke/db.sqlite")?;
//! let id = uteke.remember("important context", &["tag1"], None)?;
//! let results = uteke.recall("query", 5, None)?;
//! ```

mod consolidate;
mod embed;
mod error;
mod import_export;
mod maintenance;
pub mod memory;
mod operations;
mod types;

pub use memory::types::{
    AgingStatus, BulkDeleteResult, CleanupResult, ConsolidationResult, ContradictionResult,
    ExportEntry, ImportResult, Memory, MemoryTier, MemoryType, PruneResult, RecallStrategy,
    SearchResult, SimilarPair, StoreStats, TagInfo, DEFAULT_NAMESPACE,
};

pub use error::{format_bytes, Error};
pub use types::{DoctorCheck, DoctorReport, DoctorStatus, RepairReport, VerifyReport};

/// Maximum memory content length (characters).
pub const MAX_CONTENT_LENGTH: usize = 10_000;
/// Maximum number of tags per memory.
pub const MAX_TAGS_COUNT: usize = 20;
/// Maximum single tag length (characters).
pub const MAX_TAG_LENGTH: usize = 50;
/// Maximum payload size for server API (bytes).
pub const MAX_PAYLOAD_SIZE: usize = 1_048_576; // 1MB

/// Validate input parameters before processing.
pub fn validate_input(content: &str, tags: &[impl AsRef<str>]) -> Result<(), Error> {
    if content.trim().is_empty() {
        return Err(Error::Validation("Content must not be empty".into()));
    }
    if content.len() > MAX_CONTENT_LENGTH {
        return Err(Error::Validation(format!(
            "Content too long: {} chars (max {})",
            content.len(),
            MAX_CONTENT_LENGTH
        )));
    }
    if tags.len() > MAX_TAGS_COUNT {
        return Err(Error::Validation(format!(
            "Too many tags: {} (max {})",
            tags.len(),
            MAX_TAGS_COUNT
        )));
    }
    for tag in tags {
        let t = tag.as_ref();
        if t.is_empty() {
            return Err(Error::Validation("Tags must not be empty".into()));
        }
        if t.len() > MAX_TAG_LENGTH {
            return Err(Error::Validation(format!(
                "Tag too long: {} chars (max {})",
                t.len(),
                MAX_TAG_LENGTH
            )));
        }
    }
    Ok(())
}

use embed::EmbeddingEngine;
use memory::store::Store;
use memory::VectorIndex;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Configuration for memory tier thresholds.
///
/// Controls how memories are classified into hot/warm/cold tiers
/// and how hot memories are boosted in recall scoring.
///
/// Defaults match the hardcoded values used before config wiring (#127).
#[derive(Debug, Clone, Copy)]
pub struct TierConfig {
    /// Days before memory moves from hot → warm.
    pub hot_days: i64,
    /// Days before memory moves from warm → cold.
    pub warm_days: i64,
    /// Score boost added to hot memories during recall.
    pub hot_boost: f64,
}

impl Default for TierConfig {
    fn default() -> Self {
        Self {
            hot_days: 7,
            warm_days: 30,
            hot_boost: 0.1,
        }
    }
}

/// Configuration for recall threshold.
#[derive(Debug, Clone, Copy)]
pub struct RecallConfig {
    /// Minimum cosine similarity score for recall results. 0.0 = no filter.
    pub min_score: f32,
}

impl Default for RecallConfig {
    fn default() -> Self {
        Self { min_score: 0.0 }
    }
}

/// Resolve uteke data directory.
///
/// Uses `UTEKE_HOME` environment variable when set, otherwise falls back to
/// `~/.uteke`. This allows Docker containers and custom deployments to
/// override the default storage location.
///
/// ```text
/// UTEKE_HOME=/data   → /data
/// (not set)           → ~/.uteke
/// ```
pub fn uteke_home() -> Result<PathBuf, Error> {
    if let Ok(home) = std::env::var("UTEKE_HOME") {
        Ok(PathBuf::from(home))
    } else {
        dirs::home_dir()
            .ok_or_else(|| {
                Error::generic("Cannot determine home directory. Set UTEKE_HOME or HOME.")
            })
            .map(|p| p.join(".uteke"))
    }
}

/// Uteke — AI agent memory engine.
///
/// Combines SQLite persistence, HNSW vector search, and ONNX embedding
/// into a single cohesive memory system.
pub struct Uteke {
    store: Store,
    index: RwLock<VectorIndex>,
    embedder: Mutex<EmbeddingEngine>,
    tier_config: TierConfig,
    #[allow(dead_code)] // Stored for future per-store default threshold enforcement
    recall_config: RecallConfig,
}

impl Uteke {
    /// Open or create a Uteke memory store.
    ///
    /// `path` can be a directory path (will create `uteke.db` inside)
    /// or a direct path to a `.sqlite` file.
    /// Use `:memory:` for an in-memory database (testing).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let (_db_str, store) = Self::open_store(path)?;
        let embedder = EmbeddingEngine::new()?;
        Self::finish_open(
            store,
            embedder,
            TierConfig::default(),
            RecallConfig::default(),
        )
    }

    /// Open with a custom embedder (for testing).
    pub fn open_with_embedder(
        path: impl AsRef<Path>,
        embedder: EmbeddingEngine,
    ) -> Result<Self, Error> {
        let (_db_str, store) = Self::open_store(path)?;
        Self::finish_open(
            store,
            embedder,
            TierConfig::default(),
            RecallConfig::default(),
        )
    }

    /// Open with custom tier configuration.
    ///
    /// Allows overriding hot_days, warm_days, and hot_boost from the
    /// default 7/30/0.1 values. See [`TierConfig`].
    pub fn open_with_tier(path: impl AsRef<Path>, tier_config: TierConfig) -> Result<Self, Error> {
        let (_db_str, store) = Self::open_store(path)?;
        let embedder = EmbeddingEngine::new()?;
        Self::finish_open(store, embedder, tier_config, RecallConfig::default())
    }

    /// Open with custom recall configuration.
    pub fn open_with_recall(
        path: impl AsRef<Path>,
        recall_config: RecallConfig,
    ) -> Result<Self, Error> {
        let (_db_str, store) = Self::open_store(path)?;
        let embedder = EmbeddingEngine::new()?;
        Self::finish_open(store, embedder, TierConfig::default(), recall_config)
    }

    fn open_store(path: impl AsRef<Path>) -> Result<(String, Store), Error> {
        let db_path = path.as_ref();
        let db_str = resolve_db_path(db_path)?;
        let store = Store::open(&db_str)?;
        Ok((db_str, store))
    }

    fn finish_open(
        store: Store,
        embedder: EmbeddingEngine,
        tier_config: TierConfig,
        recall_config: RecallConfig,
    ) -> Result<Self, Error> {
        // Determine index path: same directory as SQLite DB
        let index_path = store.path().map(|p| {
            let dir = p.parent().unwrap_or(Path::new("."));
            dir.join("uteke_index.usearch")
        });

        let mut index = match &index_path {
            Some(path) => VectorIndex::load_or_create(path, EmbeddingEngine::dims())?,
            None => VectorIndex::new(EmbeddingEngine::dims())?,
        };

        // If index is empty but SQLite has memories, build from SQLite (migration)
        if index.is_empty() {
            let all_memories = store.load_all(None)?;
            if !all_memories.is_empty() {
                let items: Vec<(String, Vec<f32>)> = all_memories
                    .into_iter()
                    .map(|m| (m.id, m.embedding))
                    .collect();
                index.build(&items)?;
                index.save().ok(); // Persist after migration build
            }
        }

        Ok(Self {
            store,
            index: RwLock::new(index),
            embedder: Mutex::new(embedder),
            tier_config,
            recall_config,
        })
    }
}

/// Resolve a path to a database string.
fn resolve_db_path(db_path: &Path) -> Result<String, Error> {
    if db_path.to_str() == Some(":memory:") {
        return Ok(":memory:".to_string());
    }

    if db_path.is_dir() || db_path.extension().is_none() {
        std::fs::create_dir_all(db_path).map_err(Error::Io)?;
        // Set directory permissions to owner-only (0700) on Unix
        #[cfg(unix)]
        {
            let p: &std::path::Path = db_path;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700)).ok();
        }
        Ok(db_path.join("uteke.db").to_string_lossy().to_string())
    } else {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::Io)?;
            // Set directory permissions to owner-only (0700) on Unix
            #[cfg(unix)]
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).ok();
        }
        Ok(db_path.to_string_lossy().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_types_serialization() {
        let now = chrono::Utc::now();
        let m = Memory {
            id: "test-id".to_string(),
            content: "hello".to_string(),
            embedding: vec![0.1; 768],
            tags: vec!["a".to_string(), "b".to_string()],
            metadata: serde_json::json!({"key": "value"}),
            created_at: now,
            updated_at: now,
            namespace: DEFAULT_NAMESPACE.to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
        };

        let json = serde_json::to_string(&m).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Embedding is skipped in JSON output (skip_serializing)
        assert!(
            v.get("embedding").is_none(),
            "embedding should not be in JSON output"
        );

        // Other fields should serialize correctly
        assert_eq!(v["id"], m.id);
        assert_eq!(v["content"], m.content);
        assert_eq!(v["tags"].as_array().unwrap().len(), 2);

        // Deserialization produces empty embedding (expected — populated programmatically)
        let restored: Memory = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, m.id);
        assert_eq!(restored.content, m.content);
        assert_eq!(restored.tags, m.tags);
        assert!(restored.embedding.is_empty());
    }

    #[test]
    fn test_search_result_type() {
        let now = chrono::Utc::now();
        let m = Memory {
            id: "sr-test".to_string(),
            content: "test content".to_string(),
            embedding: vec![0.0; 768],
            tags: vec![],
            metadata: serde_json::Value::Null,
            created_at: now,
            updated_at: now,
            namespace: DEFAULT_NAMESPACE.to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
        };

        let sr = SearchResult {
            memory: m,
            score: 0.95,
        };
        assert_eq!(sr.score, 0.95);
        assert_eq!(sr.memory.id, "sr-test");
    }

    #[test]
    fn test_store_stats_type() {
        let stats = StoreStats {
            total_memories: 42,
            unique_tags: 5,
            db_size_bytes: 1024,
            hot: 10,
            warm: 15,
            cold: 17,
        };

        let json = serde_json::to_string(&stats).unwrap();
        let restored: StoreStats = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.total_memories, 42);
        assert_eq!(restored.unique_tags, 5);
    }

    #[test]
    fn test_resolve_db_path_memory() {
        let path = std::path::Path::new(":memory:");
        assert_eq!(resolve_db_path(path).unwrap(), ":memory:");
    }

    #[test]
    fn test_uteke_home_with_env() {
        std::env::set_var("UTEKE_HOME", "/tmp/custom_home");
        let home = uteke_home().unwrap_or_else(|_| PathBuf::from("/tmp/.uteke"));
        assert_eq!(home.to_string_lossy(), "/tmp/custom_home");
        std::env::remove_var("UTEKE_HOME");
    }

    #[test]
    fn test_memory_tier_from_last_accessed() {
        use crate::memory::types::MemoryTier;

        let now = chrono::Utc::now();
        let long_ago = now - chrono::Duration::days(60);
        let recent = now - chrono::Duration::days(3);

        assert_eq!(
            MemoryTier::from_last_accessed(None, 7, 30),
            MemoryTier::Cold
        );
        assert_eq!(
            MemoryTier::from_last_accessed(Some(recent), 7, 30),
            MemoryTier::Hot
        );
        assert_eq!(
            MemoryTier::from_last_accessed(Some(long_ago), 7, 30),
            MemoryTier::Cold
        );
    }

    #[test]
    fn test_memory_type_enum() {
        use crate::memory::types::MemoryType;
        assert_eq!(MemoryType::from_str_opt("fact"), Some(MemoryType::Fact));
        assert_eq!(
            MemoryType::from_str_opt("procedure"),
            Some(MemoryType::Procedure)
        );
        assert_eq!(
            MemoryType::from_str_opt("preference"),
            Some(MemoryType::Preference)
        );
        assert_eq!(
            MemoryType::from_str_opt("decision"),
            Some(MemoryType::Decision)
        );
        assert_eq!(
            MemoryType::from_str_opt("context"),
            Some(MemoryType::Context)
        );
        assert_eq!(MemoryType::from_str_opt("unknown"), None);

        assert_eq!(MemoryType::Fact.as_str(), "fact");
        assert!(MemoryType::Fact.has_temporal_validity());
        assert!(!MemoryType::Procedure.has_temporal_validity());
    }

    #[test]
    fn test_recall_config_default() {
        let config = RecallConfig::default();
        assert!((config.min_score - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_bulk_delete_result_serialization() {
        let result = BulkDeleteResult {
            deleted: 3,
            ids: vec!["a".to_string(), "b".to_string(), "c".to_string()],
        };
        let json = serde_json::to_string(&result).unwrap();
        let restored: BulkDeleteResult = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.deleted, 3);
        assert_eq!(restored.ids.len(), 3);
    }

    // These tests require ONNX embedding model (not available in CI)
    #[test]
    #[ignore]
    fn test_recall_threshold_filters_low_scores() {
        let uteke = Uteke::open(":memory:").unwrap();

        // Store a memory
        let _id = uteke
            .remember("test content about rust programming", &[], None, None)
            .unwrap();

        // Recall with min_score=0.0 should return results
        let results = uteke
            .recall("rust programming", 5, None, None, 0.0)
            .unwrap();
        assert!(!results.is_empty());

        // Recall with very high min_score should return empty
        let results = uteke
            .recall("completely unrelated quantum physics", 5, None, None, 0.99)
            .unwrap();
        assert!(
            results.is_empty(),
            "Expected empty results with 0.99 threshold, got {}",
            results.len()
        );
    }

    #[test]
    #[ignore]
    fn test_recall_threshold_zero_returns_all() {
        let uteke = Uteke::open(":memory:").unwrap();
        let _id = uteke
            .remember("some content here", &[], None, None)
            .unwrap();

        // min_score=0.0 should return results (backward compatible)
        let results = uteke.recall("content", 5, None, None, 0.0).unwrap();
        assert!(!results.is_empty(), "Expected results with 0.0 threshold");
    }

    #[test]
    #[ignore]
    fn test_recall_threshold_specific_score() {
        let uteke = Uteke::open(":memory:").unwrap();
        let _id = uteke
            .remember(
                "Rust is a systems programming language focused on safety",
                &[],
                None,
                None,
            )
            .unwrap();

        // Same content query should have high score and pass moderate threshold
        let results = uteke
            .recall("Rust programming language safety", 5, None, None, 0.5)
            .unwrap();
        assert!(
            !results.is_empty(),
            "Expected results with 0.5 threshold for matching query"
        );

        // Verify each result actually meets the threshold
        for r in &results {
            assert!(
                r.score >= 0.5,
                "Result score {} is below threshold 0.5",
                r.score
            );
        }
    }

    #[test]
    #[ignore]
    fn test_recall_config_stored_but_override_per_call() {
        // Open with recall config min_score=0.5
        let config = RecallConfig { min_score: 0.5 };
        let uteke = Uteke::open_with_recall(":memory:", config).unwrap();

        let _id = uteke
            .remember("test content about rust programming", &[], None, None)
            .unwrap();

        // Per-call min_score=0.0 should still work (overrides config)
        let results = uteke
            .recall("rust programming", 5, None, None, 0.0)
            .unwrap();
        assert!(!results.is_empty());
    }
}
