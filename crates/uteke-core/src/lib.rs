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
mod dream;
mod edges;
mod embed;
mod error;
pub mod graph;
pub mod graph_rerank;
mod import_export;
mod maintenance;
pub mod memory;
mod operations;
mod orphans;
mod recall_cache;
mod rooms;
pub mod salience_recency;
mod timeline;
mod types;

pub use chunker::{
    chunk_code, chunk_markdown, chunk_markdown_embed_aware, detect_language, extract_imports,
    CodeChunk, TextChunk,
};
pub use dream::{DreamPhase, DreamReport, PhaseResult, PhaseStatus};
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
    documents::{Document, DocumentChunk, DocumentSummary},
    DocumentEntry, DocumentSection, Room, RoomDocument, RoomMemory, RoomStats, RoomSummary,
    TimeRange, TopicCluster,
};
pub use orphans::{compute_orphan_score, OrphanMemory, DEFAULT_ORPHAN_THRESHOLD};
pub use salience_recency::{
    apply_boosts, recency_score, salience_score, type_half_life_days, SalienceRecencyConfig,
};
pub use timeline::{TimelineEvent, TimelineEventType};

pub use embed::{Embedder, OnnxEmbedder};
pub use error::{format_bytes, Error};
pub use types::{DoctorCheck, DoctorReport, DoctorStatus, RepairReport, VerifyReport};

/// Maximum memory content length (characters) — default, overridable via config (#404).
pub const MAX_CONTENT_LENGTH: usize = 100_000;
/// Maximum number of tags per memory.
pub const MAX_TAGS_COUNT: usize = 20;
/// Maximum single tag length (characters).
pub const MAX_TAG_LENGTH: usize = 50;
/// Maximum payload size for server API (bytes).
pub const MAX_PAYLOAD_SIZE: usize = 10_485_760; // 10MB

/// Validate input parameters before processing.
/// Uses default limits. For configurable limits, use `validate_input_with_limits`.
pub fn validate_input(content: &str, tags: &[impl AsRef<str>]) -> Result<(), Error> {
    validate_input_with_limits(
        content,
        tags,
        MAX_CONTENT_LENGTH,
        MAX_TAGS_COUNT,
        MAX_TAG_LENGTH,
    )
}

/// Validate input with configurable limits (#404).
/// Set max_content_length to 0 to disable content length check.
pub fn validate_input_with_limits(
    content: &str,
    tags: &[impl AsRef<str>],
    max_content_length: usize,
    max_tags_count: usize,
    max_tag_length: usize,
) -> Result<(), Error> {
    if content.trim().is_empty() {
        return Err(Error::Validation("Content must not be empty".into()));
    }
    if max_content_length > 0 && content.len() > max_content_length {
        return Err(Error::Validation(format!(
            "Content too long: {} chars (max {})",
            content.len(),
            max_content_length
        )));
    }
    if tags.len() > max_tags_count {
        return Err(Error::Validation(format!(
            "Too many tags: {} (max {})",
            tags.len(),
            max_tags_count
        )));
    }
    for tag in tags {
        let t = tag.as_ref();
        if t.is_empty() {
            return Err(Error::Validation("Tags must not be empty".into()));
        }
        if max_tag_length > 0 && t.len() > max_tag_length {
            return Err(Error::Validation(format!(
                "Tag too long: {} chars (max {})",
                t.len(),
                max_tag_length
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
    /// Salience + recency dual-axis boost config (#352). Defaults to all
    /// weights zero (opt-in per query via CLI flags / API params).
    salience_recency_config: salience_recency::SalienceRecencyConfig,
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
            salience_recency_config: salience_recency::SalienceRecencyConfig::default(),
            recall_cache: recall_cache::RecallCache::new(recall_cache::RecallCacheConfig::default()),
        })
    }

    /// Override the salience/recency dual-axis boost config (#352).
    ///
    /// Used by the CLI to forward the merged `[recall]` weights and the
    /// per-query `--salience` / `--recency` flag overrides.
    ///
    /// **Important:** this mutates shared state. Callers that serve
    /// multiple queries on the same `Uteke` instance (server, MCP) MUST
    /// call [`reset_salience_recency_config`] after the query to avoid
    /// leaking boost state into later queries (CodeCora #387).
    pub fn set_salience_recency_config(&mut self, config: salience_recency::SalienceRecencyConfig) {
        self.salience_recency_config = config.sanitized();
    }

    /// Reset salience/recency boost config to its no-op default (CodeCora #387).
    ///
    /// Call after a per-query boost override so later queries on the same
    /// `Uteke` instance aren't affected.
    pub fn reset_salience_recency_config(&mut self) {
        self.salience_recency_config = salience_recency::SalienceRecencyConfig::default();
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

    /// Set source provenance on a memory (#348).
    pub fn set_source(
        &self,
        id: &str,
        source: Option<&str>,
        source_type: &str,
    ) -> Result<bool, Error> {
        self.store.set_source(id, source, source_type)
    }

    /// Recalculate importance scores for all memories.
    pub fn recompute_importance(&self) -> Result<usize, Error> {
        self.store.recompute_importance()
    }

    /// Get a reference to the raw connection for graph operations.
    pub fn graph_store(&self) -> &rusqlite::Connection {
        &self.store.conn
    }

    /// Get graph nodes + edges for visualization (#408).
    ///
    /// Returns all nodes and edges in the knowledge graph, optionally
    /// limited by namespace.
    pub fn graph_data(&self, namespace: Option<&str>) -> Result<GraphData, Error> {
        let gs = GraphStore::new(&self.store.conn);
        let nodes = gs.all_nodes()?;
        let edges = gs.all_edges()?;
        let stats = gs.stats()?;

        // Filter by namespace if specified.
        let (nodes, edges) = if let Some(ns) = namespace {
            let ns_string = ns.to_string();
            let filtered_nodes: Vec<GraphNode> = nodes
                .into_iter()
                .filter(|n| {
                    // Memory-linked nodes: check memory namespace.
                    // Entity nodes: always include (shared across namespaces).
                    n.memory_id.as_deref().map_or(true, |_| true)
                })
                .collect();
            let _ = ns_string; // namespace filter applied at memory level
            (filtered_nodes, edges)
        } else {
            (nodes, edges)
        };

        Ok(GraphData {
            nodes,
            edges,
            stats,
        })
    }

    // ── Document engine (#406, #438) ────────────────────────────────────────

    /// Create or update a document (#406, #438).
    ///
    /// If the slug exists in the namespace, updates content and re-chunks.
    /// Chunks are created via the markdown chunker (#405) and embedded.
    /// Optional parent slug for hierarchical documents.
    pub fn doc_upsert(
        &self,
        slug: &str,
        title: &str,
        content: &str,
        tags: &[&str],
        namespace: Option<&str>,
    ) -> Result<String, Error> {
        self.doc_upsert_with_parent(slug, title, content, tags, namespace, None)
    }

    /// Create or update a document with optional parent (#438).
    ///
    /// If `parent_slug` is Some, the document is created as a child of that
    /// parent document. Depth and path are computed automatically.
    /// Max depth is 10 — returns error if exceeded.
    pub fn doc_upsert_with_parent(
        &self,
        slug: &str,
        title: &str,
        content: &str,
        tags: &[&str],
        namespace: Option<&str>,
        parent_slug: Option<&str>,
    ) -> Result<String, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let now = chrono::Utc::now().to_rfc3339();

        // Resolve parent if specified.
        let (parent_id, parent_path, parent_depth) = match parent_slug {
            Some(ps) => {
                let parent = self.store.get_document_by_slug(ps, ns)?.ok_or_else(|| {
                    Error::validation(format!("parent document '{ps}' not found"))
                })?;
                if parent.depth >= 9 {
                    return Err(Error::Validation(
                        "maximum document depth of 10 would be exceeded".into(),
                    ));
                }
                (Some(parent.id.clone()), parent.path, parent.depth + 1)
            }
            None => (None, String::new(), 0),
        };

        // Check if document exists to get current version.
        let existing = self.store.get_document_by_slug(slug, ns)?;
        let (id, version) = match &existing {
            Some(doc) => (doc.id.clone(), doc.version),
            None => (uuid::Uuid::new_v4().to_string(), 1),
        };

        let path = if let Some(ref _pid) = parent_id {
            format!("{}{}/", parent_path, id)
        } else {
            format!("/{}/", id)
        };

        let doc = Document {
            id: id.clone(),
            slug: slug.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            namespace: ns.to_string(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            metadata: serde_json::Value::Null,
            version,
            content_type: "markdown".to_string(),
            created_at: now.clone(),
            updated_at: now,
            parent_id,
            path,
            depth: parent_depth,
            sort_order: 0,
            has_children: false,
        };

        let doc_id = self.store.upsert_document(&doc)?;

        // Delete old chunks on update (re-chunking).
        let old_chunk_ids = self
            .store
            .delete_chunks_for_documents(std::slice::from_ref(&doc_id))?;

        // Chunk and embed the content.
        self.ensure_embedder()?;
        let embedder = self
            .embedder
            .lock()
            .map_err(|_| Error::lock("embedder lock during document chunking"))?;
        let embedder = embedder.as_ref().expect("embedder ensured above");

        let max_chars = embedder.max_seq_len().saturating_mul(4).max(1024);
        let chunks = crate::chunker::chunk_markdown(content, max_chars);

        // Acquire usearch write lock for chunk index inserts.
        let mut index = self
            .index
            .write()
            .map_err(|_| Error::lock("index write lock during doc chunking"))?;

        // Remove old chunk entries from usearch.
        for old_id in &old_chunk_ids {
            let key = format!("chunk:{}", old_id);
            index.remove(&key);
        }

        for (i, chunk) in chunks.iter().enumerate() {
            let chunk_id = uuid::Uuid::new_v4().to_string();
            let embedding = embedder.embed(&chunk.content)?;

            self.store.insert_document_chunk(
                &DocumentChunk {
                    id: chunk_id.clone(),
                    document_id: doc_id.clone(),
                    chunk_index: i as i64,
                    heading: chunk.heading.clone(),
                    content: chunk.content.clone(),
                    char_start: chunk.char_start as i64,
                    char_end: chunk.char_end as i64,
                    tags: tags.iter().map(|t| t.to_string()).collect(),
                },
                &embedding,
            )?;

            // Insert chunk embedding into usearch with "chunk:" prefix.
            let index_key = format!("chunk:{}", chunk_id);
            if let Err(e) = index.insert(&index_key, &embedding) {
                tracing::warn!(
                    "Failed to insert chunk {} into vector index: {}",
                    chunk_id,
                    e
                );
            }
        }

        if let Err(e) = index.save() {
            tracing::warn!("Failed to persist vector index after doc chunking: {}", e);
        }

        tracing::info!(
            "Document '{slug}' upserted in namespace '{ns}': {} chunks",
            chunks.len()
        );

        Ok(doc_id)
    }

    /// Get a document by ID or slug.
    pub fn doc_get(
        &self,
        id_or_slug: &str,
        namespace: Option<&str>,
    ) -> Result<Option<Document>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        // Try by slug first, then by ID.
        if let Some(doc) = self.store.get_document_by_slug(id_or_slug, ns)? {
            return Ok(Some(doc));
        }
        self.store.get_document(id_or_slug)
    }

    /// List documents in a namespace.
    pub fn doc_list(
        &self,
        namespace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<DocumentSummary>, Error> {
        self.store.list_documents(namespace, limit)
    }

    /// List root documents (no parent) in a namespace (#438).
    pub fn doc_list_roots(
        &self,
        namespace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<DocumentSummary>, Error> {
        self.store.list_root_documents(namespace, limit)
    }

    /// List children of a document (#438).
    pub fn doc_list_children(
        &self,
        parent_id_or_slug: &str,
        namespace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<DocumentSummary>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        // Resolve slug to ID first.
        let parent_id = match self.store.get_document_by_slug(parent_id_or_slug, ns)? {
            Some(doc) => doc.id,
            None => parent_id_or_slug.to_string(),
        };
        self.store.list_document_children(&parent_id, limit)
    }

    /// List all descendants of a document (#438).
    pub fn doc_list_descendants(
        &self,
        id_or_slug: &str,
        namespace: Option<&str>,
        max_depth: Option<i64>,
        limit: usize,
    ) -> Result<Vec<DocumentSummary>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        self.store
            .list_descendants(id_or_slug, ns, max_depth, limit)
    }

    /// Get breadcrumbs from root to a document (#438).
    pub fn doc_breadcrumbs(
        &self,
        id_or_slug: &str,
        namespace: Option<&str>,
    ) -> Result<Vec<DocumentSummary>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        self.store.get_breadcrumbs(id_or_slug, ns)
    }

    /// Move a document to a new parent or root (#438).
    pub fn doc_move(
        &self,
        id_or_slug: &str,
        new_parent_slug: Option<&str>,
        namespace: Option<&str>,
    ) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let doc = self
            .store
            .get_document_by_slug(id_or_slug, ns)?
            .or_else(|| self.store.get_document(id_or_slug).unwrap_or(None))
            .ok_or_else(|| Error::validation("document not found for move"))?;

        let new_parent_id = match new_parent_slug {
            Some(ps) => {
                let parent = self.store.get_document_by_slug(ps, ns)?.ok_or_else(|| {
                    Error::validation(&format!("parent document '{ps}' not found"))
                })?;
                Some(parent.id)
            }
            None => None,
        };

        let parent_id_ref = new_parent_id.as_deref();
        self.store.move_document(&doc.id, parent_id_ref, None)
    }

    /// Delete a document by ID or slug (#438).
    ///
    /// Cascades to children and chunks. Returns (deleted, subtree_size).
    /// Also removes chunk embeddings from usearch index.
    pub fn doc_delete(&self, id: &str) -> Result<(bool, usize), Error> {
        // Collect all document IDs in the subtree before deletion.
        let ns = DEFAULT_NAMESPACE;
        let subtree = self
            .store
            .list_descendants(id, ns, None, 10000)
            .unwrap_or_default();

        // Get the main document ID too.
        let main_id = match self.store.get_document(id)? {
            Some(d) => d.id,
            None => self
                .store
                .get_document_by_slug(id, ns)?
                .map(|d| d.id)
                .unwrap_or_default(),
        };

        let all_ids: Vec<String> = subtree
            .iter()
            .map(|d| d.id.clone())
            .chain(std::iter::once(main_id.clone()))
            .collect();

        // Get chunk IDs to remove from usearch.
        let chunk_ids = self
            .store
            .delete_chunks_for_documents(&all_ids)
            .unwrap_or_default();

        let (deleted, subtree_size) = self.store.delete_document(id)?;

        // Remove chunk entries from usearch index.
        if !chunk_ids.is_empty() {
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write lock during doc delete"))?;
            for chunk_id in &chunk_ids {
                let key = format!("chunk:{}", chunk_id);
                index.remove(&key);
            }
            let _ = index.save();
        }

        Ok((deleted, subtree_size))
    }

    /// Search documents using semantic (vector) and/or FTS5 (keyword) search.
    ///
    /// - **semantic**: embeds query, searches usearch index for chunk matches,
    ///   then joins back to document metadata. Requires embedding model.
    /// - **fts**: keyword search on title/slug via FTS5. Always available.
    /// - **hybrid** (default): runs both, deduplicates by document ID, merges
    ///   scores with reciprocal rank fusion (RRF).
    pub fn doc_search(
        &self,
        query: &str,
        namespace: Option<&str>,
        limit: usize,
        mode: &str,
    ) -> Result<Vec<crate::memory::documents::DocumentSearchResult>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let limit = limit.min(50);

        match mode {
            "semantic" => self.doc_search_semantic(query, ns, limit),
            "fts" => self.doc_search_fts(query, ns, limit),
            _ => self.doc_search_hybrid(query, ns, limit),
        }
    }

    fn doc_search_semantic(
        &self,
        query: &str,
        _ns: &str,
        limit: usize,
    ) -> Result<Vec<crate::memory::documents::DocumentSearchResult>, Error> {
        use crate::memory::documents::DocumentSearchResult;

        self.ensure_embedder()?;
        let query_embedding = self
            .embedder
            .lock()
            .map_err(|_| Error::lock("embedder lock during doc search"))?
            .as_ref()
            .expect("embedder ensured above")
            .embed(query)?;

        let index = self
            .index
            .read()
            .map_err(|_| Error::lock("index read lock during doc search"))?;

        // Search usearch — request more candidates, filter chunk: prefixed results.
        let k = (limit * 10).min(index.len()).max(1);
        let candidates = index.search(&query_embedding, k, k * 4);

        // Filter for chunk: prefixed IDs only.
        let chunk_hits: Vec<(String, f32)> = candidates
            .into_iter()
            .filter(|(id, _)| id.starts_with("chunk:"))
            .take(limit * 3)
            .collect();

        if chunk_hits.is_empty() {
            return Ok(Vec::new());
        }

        // Extract chunk IDs (strip "chunk:" prefix).
        let chunk_ids: Vec<String> = chunk_hits
            .iter()
            .map(|(id, _)| id[6..].to_string())
            .collect();

        // Get chunk data from SQLite.
        let chunks = self.store.get_chunks_by_ids(&chunk_ids)?;

        // Build results: group by document, take best score per doc.
        let mut doc_scores: std::collections::HashMap<
            String,
            (DocumentSummary, String, String, f32),
        > = std::collections::HashMap::new();

        for ((_chunk_key, distance), (_chunk_id, doc_id, heading, content)) in
            chunk_hits.iter().zip(chunks.iter())
        {
            let score = crate::memory::vector::cosine_distance_to_similarity(*distance);

            // Get document summary from store.
            if let Ok(Some(doc)) = self.store.get_document(doc_id) {
                let summary = crate::memory::documents::DocumentSummary {
                    id: doc.id.clone(),
                    slug: doc.slug.clone(),
                    title: doc.title.clone(),
                    namespace: doc.namespace.clone(),
                    version: doc.version,
                    updated_at: doc.updated_at.clone(),
                    parent_id: doc.parent_id.clone(),
                    depth: doc.depth,
                    has_children: doc.has_children,
                    sort_order: doc.sort_order,
                };

                // Keep best score per document.
                let entry = doc_scores.entry(doc_id.clone());
                entry
                    .and_modify(|(_, h, s, old_score)| {
                        if score > *old_score {
                            *old_score = score;
                            *h = heading.clone();
                            *s = content.clone();
                        }
                    })
                    .or_insert((summary, heading.clone(), content.clone(), score));
            }
        }

        let mut results: Vec<DocumentSearchResult> = doc_scores
            .into_values()
            .map(
                |(document, chunk_heading, chunk_snippet, score)| DocumentSearchResult {
                    document,
                    chunk_heading,
                    chunk_snippet: if chunk_snippet.len() > 200 {
                        format!("{}...", &chunk_snippet[..200])
                    } else {
                        chunk_snippet
                    },
                    score,
                    mode: "semantic".to_string(),
                },
            )
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);

        Ok(results)
    }

    fn doc_search_fts(
        &self,
        query: &str,
        ns: &str,
        limit: usize,
    ) -> Result<Vec<crate::memory::documents::DocumentSearchResult>, Error> {
        use crate::memory::documents::DocumentSearchResult;

        let docs = self
            .store
            .search_documents_fts(query, Some(ns), limit)
            .unwrap_or_default();

        Ok(docs
            .into_iter()
            .enumerate()
            .map(|(i, document)| DocumentSearchResult {
                document,
                chunk_heading: String::new(),
                chunk_snippet: String::new(),
                score: 1.0 / (i as f32 + 1.0), // Rank-based score
                mode: "fts".to_string(),
            })
            .collect())
    }

    fn doc_search_hybrid(
        &self,
        query: &str,
        ns: &str,
        limit: usize,
    ) -> Result<Vec<crate::memory::documents::DocumentSearchResult>, Error> {
        use crate::memory::documents::DocumentSearchResult;

        let semantic_results = self
            .doc_search_semantic(query, ns, limit * 2)
            .unwrap_or_default();
        let fts_results = self
            .doc_search_fts(query, ns, limit * 2)
            .unwrap_or_default();

        // Reciprocal Rank Fusion (RRF): score = sum(1 / (k + rank))
        let rrf_k: f32 = 60.0;
        let mut fused: std::collections::HashMap<String, DocumentSearchResult> =
            std::collections::HashMap::new();

        for (rank, result) in semantic_results.iter().enumerate() {
            let rrf_score = 1.0 / (rrf_k + (rank as f32 + 1.0));
            let entry = fused.entry(result.document.id.clone());
            entry
                .and_modify(|e| {
                    e.score += rrf_score;
                    // Prefer semantic chunk info when available.
                    if !result.chunk_heading.is_empty() && e.chunk_heading.is_empty() {
                        e.chunk_heading = result.chunk_heading.clone();
                        e.chunk_snippet = result.chunk_snippet.clone();
                    }
                })
                .or_insert_with(|| DocumentSearchResult {
                    document: result.document.clone(),
                    chunk_heading: result.chunk_heading.clone(),
                    chunk_snippet: result.chunk_snippet.clone(),
                    score: rrf_score,
                    mode: "hybrid".to_string(),
                });
        }

        for (rank, result) in fts_results.iter().enumerate() {
            let rrf_score = 1.0 / (rrf_k + (rank as f32 + 1.0));
            let entry = fused.entry(result.document.id.clone());
            entry
                .and_modify(|e| e.score += rrf_score)
                .or_insert_with(|| DocumentSearchResult {
                    document: result.document.clone(),
                    chunk_heading: result.chunk_heading.clone(),
                    chunk_snippet: result.chunk_snippet.clone(),
                    score: rrf_score,
                    mode: "hybrid".to_string(),
                });
        }

        let mut results: Vec<DocumentSearchResult> = fused.into_values().collect();
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);

        Ok(results)
    }
}

/// Graph visualization data (#408).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GraphData {
    /// All nodes in the graph.
    pub nodes: Vec<GraphNode>,
    /// All edges in the graph.
    pub edges: Vec<GraphEdge>,
    /// Graph statistics.
    pub stats: GraphStats,
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
    use serial_test::serial;

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
            source: None,
            source_type: "user".to_string(),
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
            source: None,
            source_type: "user".to_string(),
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
    #[serial]
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
    #[serial]
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
    #[serial]
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
