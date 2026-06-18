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

pub mod chunker;
mod consolidate;
mod edges;
mod embed;
mod error;
pub mod graph;
pub mod graph_rerank;
mod import_export;
mod maintenance;
pub mod memory;
mod operations;
mod recall_cache;
mod rooms;
mod types;

pub use chunker::{chunk_code, detect_language, extract_imports, CodeChunk};
pub use edges::{
    backlink_type_for, EdgeList, MemoryEdge, EDGE_REFERENCED_BY, EDGE_REFERENCES, EDGE_REPLIES_TO,
    EDGE_SUPERSEDES, EDGE_TAGGED_AS,
};
pub use graph::{build_meta_relationship, is_relationship_meta, Relationship, VALID_REL_TYPES};
pub use graph::{GraphEdge, GraphNode, GraphPath, GraphStats, GraphStore, GraphTriple};
pub use graph_rerank::{compute_graph_signals, rerank_with_graph, GraphRerankConfig, GraphSignals};
pub use memory::types::{
    AgingStatus, BulkDeleteResult, CleanupResult, ConsolidationResult, ContradictionResult,
    ExportEntry, ImportResult, Memory, MemoryTier, MemoryType, PruneResult, RecallStrategy,
    SearchResult, SimilarPair, StoreStats, TagInfo, DEFAULT_NAMESPACE,
};
pub use memory::{
    DocumentEntry, DocumentSection, Room, RoomDocument, RoomMemory, RoomStats, RoomSummary,
    TimeRange, TopicCluster,
};

pub use embed::{Embedder, OnnxEmbedder};
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

/// Resolved embedder configuration used by lazy backend dispatch.
///
/// Mirrors the CLI-side `EmbeddingConfig` but kept inside `uteke-core` so
/// the core library can construct OpenAI/Ollama backends without depending
/// on the CLI crate. Field values are sourced from the merged CLI config
/// (env vars + uteke.toml) by the caller, then further env-var overrides
/// take precedence at resolve time.
#[derive(Clone, Default)]
pub struct EmbeddingSettings {
    /// API key for OpenAI (or compatible). Empty = ONNX/Ollama.
    pub api_key: String,
    /// Custom endpoint. Empty = backend default.
    pub base_url: String,
    /// Model name. Empty = backend default.
    pub model: String,
    /// Force dims. 0 = backend/model default.
    pub dims: usize,
}

impl EmbeddingSettings {
    /// Merge caller-provided settings with env-var overrides. Env vars
    /// (UTEKE_EMBEDDING_*) win over the caller-supplied values; the caller
    /// is responsible for having already merged uteke.toml into the input.
    fn resolve_with_defaults(input: &EmbeddingSettings) -> Self {
        // Env vars win over caller-supplied values, but an explicitly empty
        // env var is treated as "unset" so it cannot clobber a non-empty
        // config-provided value (CodeCora finding: empty
        // UTEKE_EMBEDDING_API_KEY previously overwrote a populated
        // [embedding].api_key).
        let env_or = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let api_key = env_or("UTEKE_EMBEDDING_API_KEY")
            .or_else(|| env_or("OPENAI_API_KEY"))
            .unwrap_or_else(|| input.api_key.clone());
        let base_url = env_or("UTEKE_EMBEDDING_BASE_URL").unwrap_or_else(|| input.base_url.clone());
        let model = env_or("UTEKE_EMBEDDING_MODEL").unwrap_or_else(|| input.model.clone());
        let dims = std::env::var("UTEKE_EMBEDDING_DIMS")
            .ok()
            .filter(|v| !v.is_empty())
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(input.dims);
        Self {
            api_key,
            base_url,
            model,
            dims,
        }
    }
}

// Backward-compat alias removed — use EmbeddingSettings::resolve_with_defaults
// directly. The old EmbedderEnv struct was only used inside lib.rs and is
// now replaced by the public EmbeddingSettings API.

/// Uteke — AI agent memory engine.
///
/// Combines SQLite persistence, HNSW vector search, and ONNX embedding
/// into a single cohesive memory system.
pub struct Uteke {
    store: Store,
    index: RwLock<VectorIndex>,
    embedder: Mutex<Option<Box<dyn Embedder>>>,
    /// Embedding backend name ("onnx", "openai", "ollama", "custom"). Used by lazy init.
    embedder_backend: String,
    /// Caller-supplied embedding settings (from uteke.toml). Env vars still
    /// override these at resolve time.
    embedding_settings: EmbeddingSettings,
    tier_config: TierConfig,
    #[allow(dead_code)] // Stored for future per-store default threshold enforcement
    recall_config: RecallConfig,
    /// Graph-augmented reranking config (#378). Applied only for
    /// [`RecallStrategy::Graph`]. Defaults to enabled with subtle weights.
    graph_rerank_config: graph_rerank::GraphRerankConfig,
    /// Recall cache — avoids redundant embedding computation for repeated queries.
    recall_cache: recall_cache::RecallCache,
}

impl Uteke {
    /// Open or create a Uteke memory store.
    ///
    /// `path` can be a directory path (will create `uteke.db` inside)
    /// or a direct path to a `.sqlite` file.
    /// Use `:memory:` for an in-memory database (testing).
    ///
    /// The ONNX embedding model is loaded lazily on first use, so commands
    /// that don't need embedding (list, get, stats, tags, etc.) start instantly.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::open_with_backend(path, "onnx")
    }

    /// Open with a custom embedder (for testing).
    pub fn open_with_embedder(
        path: impl AsRef<Path>,
        embedder: impl Embedder + 'static,
    ) -> Result<Self, Error> {
        let (_db_str, store) = Self::open_store(path)?;
        Self::finish_open(
            store,
            Some(Box::new(embedder)),
            "custom".to_string(),
            TierConfig::default(),
            RecallConfig::default(),
            EmbeddingSettings::default(),
        )
    }

    /// Open with custom tier configuration.
    ///
    /// Allows overriding hot_days, warm_days, and hot_boost from the
    /// default 7/30/0.1 values. See [`TierConfig`].
    pub fn open_with_tier(path: impl AsRef<Path>, tier_config: TierConfig) -> Result<Self, Error> {
        let (_db_str, store) = Self::open_store(path)?;
        Self::finish_open(
            store,
            None,
            "onnx".to_string(),
            tier_config,
            RecallConfig::default(),
            EmbeddingSettings::default(),
        )
    }

    /// Open with custom tier configuration and embedding backend.
    pub fn open_with_tier_and_backend(
        path: impl AsRef<Path>,
        tier_config: TierConfig,
        backend: &str,
    ) -> Result<Self, Error> {
        let (_db_str, store) = Self::open_store(path)?;
        Self::finish_open(
            store,
            None,
            backend.to_string(),
            tier_config,
            RecallConfig::default(),
            EmbeddingSettings::default(),
        )
    }

    /// Open with custom recall configuration.
    pub fn open_with_recall(
        path: impl AsRef<Path>,
        recall_config: RecallConfig,
    ) -> Result<Self, Error> {
        let (_db_str, store) = Self::open_store(path)?;
        Self::finish_open(
            store,
            None,
            "onnx".to_string(),
            TierConfig::default(),
            recall_config,
            EmbeddingSettings::default(),
        )
    }

    /// Open with a specific embedding backend (e.g., "onnx").
    ///
    /// Future backends ("openai", "ollama") will be selectable here once implemented.
    pub fn open_with_backend(path: impl AsRef<Path>, backend: &str) -> Result<Self, Error> {
        let (_db_str, store) = Self::open_store(path)?;
        Self::finish_open(
            store,
            None,
            backend.to_string(),
            TierConfig::default(),
            RecallConfig::default(),
            EmbeddingSettings::default(),
        )
    }

    /// Open with caller-supplied embedding settings (CLI passes merged config).
    ///
    /// `backend` selects onnx/openai/ollama/custom. `settings` carries the
    /// api_key/base_url/model/dims resolved from uteke.toml; env vars still
    /// take precedence at first-embed resolve time.
    pub fn open_with_embedding(
        path: impl AsRef<Path>,
        backend: &str,
        settings: EmbeddingSettings,
        tier_config: TierConfig,
        recall_config: RecallConfig,
    ) -> Result<Self, Error> {
        let (_db_str, store) = Self::open_store(path)?;
        Self::finish_open_full(
            store,
            None,
            backend.to_string(),
            tier_config,
            recall_config,
            settings,
            graph_rerank::GraphRerankConfig::default(),
        )
    }

    /// Open with caller-supplied embedding settings **and** graph-reranking
    /// config. Used by the CLI to pass the merged `[recall]` graph weights
    /// (#378). Equivalent to [`Self::open_with_embedding`] with default graph
    /// reranking when graph reranking is not needed.
    pub fn open_with_embedding_and_graph(
        path: impl AsRef<Path>,
        backend: &str,
        settings: EmbeddingSettings,
        tier_config: TierConfig,
        recall_config: RecallConfig,
        graph_rerank_config: graph_rerank::GraphRerankConfig,
    ) -> Result<Self, Error> {
        let (_db_str, store) = Self::open_store(path)?;
        Self::finish_open_full(
            store,
            None,
            backend.to_string(),
            tier_config,
            recall_config,
            settings,
            graph_rerank_config,
        )
    }

    fn open_store(path: impl AsRef<Path>) -> Result<(String, Store), Error> {
        let db_path = path.as_ref();
        let db_str = resolve_db_path(db_path)?;
        let store = Store::open(&db_str)?;
        Ok((db_str, store))
    }

    fn finish_open(
        store: Store,
        embedder: Option<Box<dyn Embedder>>,
        embedder_backend: String,
        tier_config: TierConfig,
        recall_config: RecallConfig,
        embedding_settings: EmbeddingSettings,
    ) -> Result<Self, Error> {
        Self::finish_open_full(
            store,
            embedder,
            embedder_backend,
            tier_config,
            recall_config,
            embedding_settings,
            graph_rerank::GraphRerankConfig::default(),
        )
    }

    fn finish_open_full(
        store: Store,
        embedder: Option<Box<dyn Embedder>>,
        embedder_backend: String,
        tier_config: TierConfig,
        recall_config: RecallConfig,
        embedding_settings: EmbeddingSettings,
        graph_rerank_config: graph_rerank::GraphRerankConfig,
    ) -> Result<Self, Error> {
        // Determine index path: same directory as SQLite DB
        let index_path = store.path().map(|p| {
            let dir = p.parent().unwrap_or(Path::new("."));
            dir.join("uteke_index.usearch")
        });

        // Use dims from the provided embedder if available.
        // When lazy-initializing (embedder=None), validate backend and use known dims.
        let dims = match embedder.as_ref() {
            Some(e) => e.dims(),
            None => match embedder_backend.as_str() {
                "onnx" | "" | "custom" => crate::embed::OnnxEmbedder::dims(),
                "openai" => {
                    // User-configurable via uteke.toml or UTEKE_EMBEDDING_DIMS.
                    // Default 1536 (text-embedding-3-small).
                    let cfg = EmbeddingSettings::resolve_with_defaults(&embedding_settings);
                    if cfg.dims == 0 {
                        crate::embed::openai::DEFAULT_DIMS
                    } else {
                        cfg.dims
                    }
                }
                "ollama" => {
                    let cfg = EmbeddingSettings::resolve_with_defaults(&embedding_settings);
                    if cfg.dims == 0 {
                        crate::embed::ollama::DEFAULT_DIMS
                    } else {
                        cfg.dims
                    }
                }
                other => {
                    return Err(Error::Validation(format!(
                        "Unknown embedding backend: '{other}'. Supported: onnx, openai, ollama."
                    )));
                }
            },
        };

        let mut index = match &index_path {
            Some(path) => VectorIndex::load_or_create(path, dims)?,
            None => VectorIndex::new(dims)?,
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
            embedder_backend,
            embedding_settings,
            tier_config,
            recall_config,
            graph_rerank_config: graph_rerank_config.sanitized(),
            recall_cache: recall_cache::RecallCache::new(recall_cache::RecallCacheConfig::default()),
        })
    }

    /// Lazy-load the ONNX embedding engine on first use.
    ///
    /// Commands that don't need embedding (list, get, stats, tags, namespace,
    /// aging, export, forget) never trigger this, making them instant (~1ms)
    /// instead of waiting for model load (~2.5s).
    fn ensure_embedder(&self) -> Result<(), Error> {
        let mut guard = self
            .embedder
            .lock()
            .map_err(|_| Error::lock("embedder lock during lazy init"))?;
        if guard.is_none() {
            tracing::debug!(backend = %self.embedder_backend, "Lazy-initializing embedding backend");
            let backend = self.embedder_backend.as_str();
            let embedder: Box<dyn crate::embed::Embedder> = match backend {
                "onnx" | "" => Box::new(crate::embed::OnnxEmbedder::new()?),
                "custom" => {
                    return Err(Error::generic(
                        "Custom embedder backend set but no embedder was provided",
                    ));
                }
                "openai" => {
                    let cfg = EmbeddingSettings::resolve_with_defaults(&self.embedding_settings);
                    let model = if cfg.model.is_empty() {
                        crate::embed::openai::DEFAULT_MODEL.to_string()
                    } else {
                        cfg.model
                    };
                    let base_url = if cfg.base_url.is_empty() {
                        crate::embed::openai::DEFAULT_BASE_URL.to_string()
                    } else {
                        cfg.base_url
                    };
                    let dims = if cfg.dims == 0 {
                        crate::embed::openai::DEFAULT_DIMS
                    } else {
                        cfg.dims
                    };
                    Box::new(crate::embed::OpenAiEmbedder::new(
                        &cfg.api_key,
                        &model,
                        &base_url,
                        dims,
                    )?)
                }
                "ollama" => {
                    let cfg = EmbeddingSettings::resolve_with_defaults(&self.embedding_settings);
                    let model = if cfg.model.is_empty() {
                        crate::embed::ollama::DEFAULT_MODEL.to_string()
                    } else {
                        cfg.model
                    };
                    let base_url = if cfg.base_url.is_empty() {
                        crate::embed::ollama::DEFAULT_BASE_URL.to_string()
                    } else {
                        cfg.base_url
                    };
                    let dims = if cfg.dims == 0 {
                        crate::embed::ollama::DEFAULT_DIMS
                    } else {
                        cfg.dims
                    };
                    Box::new(crate::embed::OllamaEmbedder::new(&base_url, &model, dims)?)
                }
                other => {
                    return Err(Error::Validation(format!(
                        "Unknown embedding backend: '{other}'. Supported: onnx, openai, ollama."
                    )));
                }
            };

            // Dim mismatch detection (#337): refuse to silently mix vectors
            // from different backends in one index. Catch it at first use
            // so the user gets a clear error instead of garbage recall.
            //
            // Escape hatch: UTEKE_ALLOW_DIM_MISMATCH=1 skips the check so the
            // user can open the store with a different backend to run
            // `uteke repair` (which rebuilds vectors with the new backend).
            // Without this, a user who flips backend on an existing store
            // can never recover (CodeCora finding #154).
            let backend_dims = embedder.dims();
            let index_dims = self.index.read().map(|i| i.dims()).unwrap_or(backend_dims);
            if index_dims != backend_dims
                && std::env::var("UTEKE_ALLOW_DIM_MISMATCH").as_deref() != Ok("1")
            {
                return Err(Error::Validation(format!(
                    "Embedding dimension mismatch: index has {index_dims}d vectors but backend '{backend}' produces {backend_dims}d. \
                     Rebuild the index (`UTEKE_ALLOW_DIM_MISMATCH=1 uteke repair`) or switch backend."
                )));
            }

            *guard = Some(embedder);
        }
        Ok(())
    }

    /// Pin a memory so it never decays.
    pub fn pin(&self, id: &str) -> Result<bool, Error> {
        self.store.pin(id)
    }

    /// Unpin a memory.
    pub fn unpin(&self, id: &str) -> Result<bool, Error> {
        self.store.unpin(id)
    }

    /// Recalculate importance scores for all memories.
    pub fn recompute_importance(&self) -> Result<usize, Error> {
        self.store.recompute_importance()
    }

    /// Get a reference to the raw connection for graph operations.
    pub fn graph_store(&self) -> &rusqlite::Connection {
        &self.store.conn
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
            importance: 0.5,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
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
            importance: 0.5,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
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
            cache_hits: 100,
            cache_misses: 25,
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

    #[test]
    fn embedding_settings_defaults_empty() {
        let d = EmbeddingSettings::default();
        assert!(d.api_key.is_empty());
        assert!(d.base_url.is_empty());
        assert!(d.model.is_empty());
        assert_eq!(d.dims, 0);
    }

    #[test]
    fn embedding_settings_env_overrides_caller_config() {
        // Env vars win over caller-supplied settings.
        std::env::set_var("UTEKE_EMBEDDING_API_KEY", "sk-env-wins");
        std::env::set_var("UTEKE_EMBEDDING_MODEL", "env-model");
        let input = EmbeddingSettings {
            api_key: "sk-config".to_string(),
            base_url: "https://config.example.com".to_string(),
            model: "config-model".to_string(),
            dims: 1024,
        };
        let merged = EmbeddingSettings::resolve_with_defaults(&input);
        std::env::remove_var("UTEKE_EMBEDDING_API_KEY");
        std::env::remove_var("UTEKE_EMBEDDING_MODEL");
        // Env overrides
        assert_eq!(merged.api_key, "sk-env-wins");
        assert_eq!(merged.model, "env-model");
        // Non-overridden fields fall through from the caller config.
        assert_eq!(merged.base_url, "https://config.example.com");
        assert_eq!(merged.dims, 1024);
    }

    #[test]
    fn embedding_settings_empty_env_does_not_overwrite_config() {
        // Explicitly empty env var must NOT clobber a non-empty config value
        // (CodeCora finding: std::env::var returns Ok("") for empty vars).
        std::env::set_var("UTEKE_EMBEDDING_API_KEY", "");
        std::env::set_var("UTEKE_EMBEDDING_MODEL", "");
        let input = EmbeddingSettings {
            api_key: "sk-from-config".to_string(),
            base_url: "https://config.example.com".to_string(),
            model: "config-model".to_string(),
            dims: 1536,
        };
        let merged = EmbeddingSettings::resolve_with_defaults(&input);
        std::env::remove_var("UTEKE_EMBEDDING_API_KEY");
        std::env::remove_var("UTEKE_EMBEDDING_MODEL");
        assert_eq!(
            merged.api_key, "sk-from-config",
            "empty env must not clobber config"
        );
        assert_eq!(
            merged.model, "config-model",
            "empty env must not clobber config"
        );
    }

    #[test]
    fn embedding_settings_config_used_when_env_absent() {
        std::env::remove_var("UTEKE_EMBEDDING_API_KEY");
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("UTEKE_EMBEDDING_BASE_URL");
        std::env::remove_var("UTEKE_EMBEDDING_MODEL");
        std::env::remove_var("UTEKE_EMBEDDING_DIMS");
        let input = EmbeddingSettings {
            api_key: "sk-config-only".to_string(),
            base_url: "https://from-toml.example.com".to_string(),
            model: "from-toml-model".to_string(),
            dims: 2048,
        };
        let merged = EmbeddingSettings::resolve_with_defaults(&input);
        assert_eq!(merged.api_key, "sk-config-only");
        assert_eq!(merged.base_url, "https://from-toml.example.com");
        assert_eq!(merged.model, "from-toml-model");
        assert_eq!(merged.dims, 2048);
    }
}
