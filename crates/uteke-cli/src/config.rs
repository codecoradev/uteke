//! Configuration management for uteke CLI.
//!
//! Layered config resolution: CLI args > project `.uteke/uteke.toml` > global `{uteke_home}/uteke.toml` > defaults.
//! Migrates legacy `config.toml` → `uteke.toml` on load.

use std::path::PathBuf;
use uteke_core::config_layers::Layers;

// ── Config sections ─────────────────────────────────────────────────────────

/// Store configuration.
#[derive(serde::Deserialize, Clone)]
#[serde(default)]
pub struct StoreConfig {
    /// Base directory for the memory store.
    pub path: String,
    /// Namespace for multi-agent isolation.
    pub namespace: String,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            path: "~/.codecora/uteke".to_string(),
            namespace: "default".to_string(),
        }
    }
}

/// Vector engine configuration (#1168).
///
/// Which engine the index runs on when BOTH are compiled in. Slim builds
/// (one engine) ignore this — the compiled-in engine always runs.
#[derive(serde::Deserialize, Clone, Default)]
pub struct VectorConfig {
    /// "usearch" or "vecq". Empty = compiled-in default (usearch when present).
    pub backend: String,
}

/// Embedding model configuration.
#[derive(serde::Deserialize, Clone)]
#[serde(default)]
pub struct EmbeddingConfig {
    /// Embedding backend: "onnx" (default), "openai", "ollama".
    pub backend: String,
    /// Embedding model name.
    pub model: String,
    /// Maximum sequence length for the embedding model.
    pub max_seq_length: usize,
    /// API key for backends that require one (OpenAI). Leave empty for ONNX/Ollama.
    /// Can also be supplied via UTEKE_EMBEDDING_API_KEY / OPENAI_API_KEY.
    pub api_key: String,
    /// Custom endpoint URL. Empty string = backend default.
    /// - OpenAI: https://api.openai.com/v1
    /// - Ollama: http://localhost:11434
    /// - Azure OpenAI: your endpoint base
    pub base_url: String,
    /// Embedding endpoint path appended to base_url. Empty string = "/embeddings" (OpenAI standard).
    /// Override for non-standard OpenAI-compatible APIs, e.g. CodeCora Embed uses "/embed" (#473).
    /// Can also be supplied via UTEKE_EMBEDDING_ENDPOINT_PATH.
    pub endpoint_path: String,
    /// Embedding dimensions. 0 = use backend/model default.
    /// Override only when you know your model's output dim.
    pub dims: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            backend: "onnx".to_string(),
            model: "embeddinggemma-q4".to_string(),
            max_seq_length: 2048,
            api_key: String::new(),
            base_url: String::new(),
            endpoint_path: String::new(),
            dims: 0,
        }
    }
}

impl EmbeddingConfig {
    /// Supported embedding backends.
    pub const SUPPORTED_BACKENDS: &'static [&'static str] = &["onnx", "openai", "ollama"];

    /// Validate the backend field.
    ///
    /// Returns an error message if the backend is not recognized.
    pub fn validate_backend(&self) -> Result<(), String> {
        if Self::SUPPORTED_BACKENDS.contains(&self.backend.as_str()) {
            Ok(())
        } else {
            Err(format!(
                "Unsupported embedding backend: '{}'. Supported: {}",
                self.backend,
                Self::SUPPORTED_BACKENDS.join(", ")
            ))
        }
    }
}

/// LLM fact-extraction configuration for `import --extract` (opt-in).
///
/// Re-exported from uteke-core so the server can share the same config.
pub type ExtractionConfig = uteke_core::extraction::ExtractionConfig;

/// Embedding fallback configuration for cloud API when local ONNX fails.
///
/// Entirely optional — all fields default to empty. When all fields are empty,
/// no fallback is configured and local ONNX errors propagate normally.
/// Env vars (UTEKE_EMBED_FALLBACK_*) win over toml values.
#[derive(serde::Deserialize, Clone, Default)]
#[serde(default)]
pub struct EmbedFallbackConfig {
    /// API key for the fallback embedding endpoint.
    /// Env: UTEKE_EMBED_FALLBACK_API_KEY
    pub api_key: String,
    /// Base URL (e.g. "https://your-modal-app.modal.run").
    /// Env: UTEKE_EMBED_FALLBACK_BASE_URL
    pub base_url: String,
    /// Endpoint path appended to base_url. Empty = "/embeddings".
    /// Env: UTEKE_EMBED_FALLBACK_ENDPOINT_PATH
    pub endpoint_path: String,
    /// Model name for the fallback embedding endpoint.
    /// Env: UTEKE_EMBED_FALLBACK_MODEL
    pub model: String,
}

impl EmbedFallbackConfig {
    /// The same settings as the core type, for `is_configured` and the env
    /// overrides, so the rules live in one place (#1355).
    fn to_core(&self) -> uteke_core::FallbackSettings {
        uteke_core::FallbackSettings {
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            endpoint_path: self.endpoint_path.clone(),
            model: self.model.clone(),
        }
    }

    /// Check if fallback is fully configured (api_key, base_url, AND model).
    /// Warns on partial config — partial config will be rejected by the core library.
    pub fn is_configured(&self) -> bool {
        self.to_core().is_configured()
    }

    /// Resolve with env var overrides. Env vars win over toml values.
    fn resolve_with_env(self) -> Self {
        let r = self.to_core().with_env_overrides();
        Self {
            api_key: r.api_key,
            base_url: r.base_url,
            endpoint_path: r.endpoint_path,
            model: r.model,
        }
    }
}

/// Tier configuration for hot/warm/cold memory tiers.
#[derive(serde::Deserialize, Clone)]
#[serde(default)]
pub struct TierConfig {
    /// Days before memory moves from hot to warm.
    pub hot_days: u32,
    /// Days before memory moves from warm to cold.
    pub warm_days: u32,
    /// Score boost for hot memories.
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

/// Logging configuration.
#[derive(serde::Deserialize, Clone)]
#[serde(default)]
pub struct LoggingConfig {
    /// Log level: trace, debug, info, warn, error.
    pub level: String,
    /// Optional log file path. Empty = stderr only.
    pub file: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "warn".to_string(),
            file: String::new(),
        }
    }
}

/// Aging / eviction configuration.
#[derive(serde::Deserialize, Clone)]
#[serde(default)]
pub struct AgingConfig {
    /// Enable automatic aging of old memories.
    pub enabled: bool,
    /// Maximum age in days before pruning (default: 365).
    pub max_age_days: u32,
    /// Maximum access count for a memory to be considered "cold" (default: 10).
    /// Only memories accessed fewer than this many times AND older than
    /// max_age_days are candidates for cleanup.
    pub max_access_count: u32,
    /// Maximum number of cold memories to keep before triggering cleanup
    /// (default: 1000).
    pub max_cold_count: usize,
}

impl Default for AgingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_age_days: 365,
            max_access_count: 10,
            max_cold_count: 1000,
        }
    }
}

/// Recall / search threshold configuration.
#[derive(serde::Deserialize, Clone)]
#[serde(default)]
pub struct RecallConfig {
    /// Minimum similarity score for recall results. Results below are filtered out.
    /// Default 0.0 (#1223): since fusion RRF became the default strategy (0.16.0),
    /// returned scores are rank-based (~0.0-0.2), so a legacy cosine-era default of
    /// 0.3 silently emptied default recalls. Threshold remains opt-in via config,
    /// `--min`, or `--strict` (0.5).
    pub min_score: f64,
    /// Strict mode threshold (higher, for critical queries).
    pub min_score_strict: f64,
    /// Default recall strategy for the `recall` command when `--strategy` is
    /// not given. One of: `vector`, `fts5`, `hybrid`, `graph`. Default
    /// `hybrid` (RRF: vector + FTS5, R@5 98.0% vs vector-only 85.4% on
    /// LongMemEval-S); `graph` enables graph-augmented reranking (#378).
    pub default_strategy: String,
    /// Weight for the edge-density boost applied by the `graph` strategy.
    /// 0.0 disables; 0.1 is subtle (default).
    pub graph_density_weight: f32,
    /// Weight for the incoming-edge authority boost applied by the `graph`
    /// strategy. 0.0 disables; 0.1 is subtle (default).
    pub graph_authority_weight: f32,
    /// Feature flag for graph-augmented reranking. When `false`, the `graph`
    /// strategy behaves like `hybrid` (no boost applied).
    pub graph_rerank_enabled: bool,
    /// Weight for the salience boost applied when the `--salience` flag is
    /// passed to `recall` (#352). 0.0 disables; 0.15 is the default.
    pub salience_weight: f32,
    /// Weight for the recency boost applied when the `--recency` flag is
    /// passed to `recall` (#352). 0.0 disables; 0.15 is the default.
    pub recency_weight: f32,
    /// Weight for the Jaccard token reranking boost (#719).
    /// Applied post-RRF as an additive signal based on query-content token
    /// overlap. 0.0 disables (default); 0.10-0.15 recommended.
    pub jaccard_weight: f32,
}

impl Default for RecallConfig {
    fn default() -> Self {
        Self {
            min_score: 0.0,
            min_score_strict: 0.5,
            // Fusion (weighted RRF: vector×1.7 + hybrid×1) is the default
            // strategy since 0.16.0 (#1123). Benchmark: fast50 R@5 0.98 vs
            // 0.9267 hybrid. Explicit config still overrides.
            default_strategy: "fusion".to_string(),
            graph_density_weight: 0.1,
            graph_authority_weight: 0.1,
            graph_rerank_enabled: true,
            salience_weight: 0.15,
            recency_weight: 0.15,
            jaccard_weight: 0.0,
        }
    }
}

// ── Top-level config ────────────────────────────────────────────────────────

/// Server configuration.
#[derive(serde::Deserialize, Clone)]
#[serde(default)]
pub struct ServerConfig {
    /// Enable server mode.
    pub enabled: bool,
    /// Bind host.
    pub host: String,
    /// Bind port.
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".to_string(),
            port: 8767,
        }
    }
}

/// Maintenance daemon configuration (#442).
/// Controls auto-aging and auto-dream background tasks in the server.
#[derive(serde::Deserialize, Clone)]
#[serde(default)]
pub struct MaintenanceConfig {
    /// Enable auto-aging: periodically clean up cold, stale memories.
    pub auto_aging_enabled: bool,
    /// Auto-aging interval in hours (default: 24).
    pub auto_aging_interval_hours: u64,
    /// Enable auto-dream: periodically run dream cycle (lint → dedup → orphans).
    pub auto_dream_enabled: bool,
    /// Auto-dream interval in days (default: 7).
    pub auto_dream_interval_days: u64,
}

impl Default for MaintenanceConfig {
    fn default() -> Self {
        Self {
            auto_aging_enabled: false,
            auto_aging_interval_hours: 24,
            auto_dream_enabled: true,
            auto_dream_interval_days: 7,
        }
    }
}

/// Dream pipeline thresholds (#731). All phases in the dream cycle
/// (contradiction scan, dedup/consolidate, orphan detection) read from
/// these values instead of hardcoded constants.
#[derive(serde::Deserialize, Clone, Copy)]
#[serde(default)]
pub struct DreamConfig {
    /// Cosine similarity threshold for contradiction scan. Memories with
    /// similarity ABOVE this value are NOT contradictions. Default: 0.6.
    pub contradict_similarity_threshold: f32,
    /// Minimum Jaccard index for tag overlap to consider contradiction.
    /// Default: 0.4 (up from original 0.3 to reduce false positives).
    pub contradict_tag_jaccard_min: f32,
    /// Maximum memories loaded for O(n²) contradiction scan. Default: 200.
    pub contradict_max_memories: usize,
    /// Cosine similarity threshold for dedup/consolidate. Memories with
    /// similarity ABOVE this value are merge candidates. Default: 0.92.
    pub dedup_threshold: f32,
    /// Importance threshold for orphan detection. Memories below this AND
    /// with no edges are flagged as orphans. Default: 0.15 (safer than 0.3).
    pub orphan_importance_threshold: f64,
}

impl Default for DreamConfig {
    fn default() -> Self {
        Self {
            contradict_similarity_threshold: 0.6,
            contradict_tag_jaccard_min: 0.4,
            contradict_max_memories: 200,
            dedup_threshold: 0.92,
            orphan_importance_threshold: 0.15,
        }
    }
}

/// Memory lifecycle configuration (#928).
///
/// Controls how memories transition through their lifecycle:
/// `ACTIVE → DEPRECATED (hidden, restorable) → PRUNED (hard delete)`.
///
/// All destructive operations route through soft-delete when
/// `soft_delete_only = true` (the default).
#[derive(serde::Deserialize, Clone)]
#[serde(default)]
pub struct LifecycleConfig {
    /// Route all destructive ops through `deprecate()` instead of `delete()`.
    /// Default: true. Set to `false` to restore pre-v0.13 hard-delete behavior.
    pub soft_delete_only: bool,
    /// Whether auto lifecycle cycles are enabled. Default: true.
    pub auto_aging_enabled: bool,
    /// Hours between automatic lifecycle cycles (server mode). Default: 168 (weekly).
    pub auto_aging_interval_hours: u64,
    /// Minimum age in days before eligible for deprecation. Default: 90.
    pub min_age_days: u32,
    /// Maximum access count for "cold" eligibility. Default: 3.
    pub max_access_count: u32,
    /// Max percentage of total memories deprecable per cycle. Default: 1.0.
    pub max_deprecate_percent: f64,
    /// Minimum deprecations per cycle. Default: 1.
    pub min_deprecate_per_cycle: usize,
    /// Hard ceiling on deprecations per cycle. Default: 50.
    pub max_deprecate_per_cycle: usize,
    /// Days after deprecation before eligible for pruning (hard delete). Default: 30.
    pub deprecated_ttl_days: u32,
    /// Auto-prune expired deprecated memories during cycles. Default: true.
    pub auto_prune_enabled: bool,
    /// Dream dedup phase uses soft-delete. Default: true.
    pub dream_dedup_soft_delete: bool,
    /// Dream compact phase uses soft-delete. Default: true.
    pub dream_compact_soft_delete: bool,
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            soft_delete_only: true,
            auto_aging_enabled: true,
            auto_aging_interval_hours: 168,
            min_age_days: 90,
            max_access_count: 3,
            max_deprecate_percent: 1.0,
            min_deprecate_per_cycle: 1,
            max_deprecate_per_cycle: 50,
            deprecated_ttl_days: 30,
            auto_prune_enabled: true,
            dream_dedup_soft_delete: true,
            dream_compact_soft_delete: true,
        }
    }
}

impl LifecycleConfig {
    /// Convert CLI config to core config.
    pub fn to_core(&self) -> uteke_core::LifecycleConfig {
        uteke_core::LifecycleConfig {
            soft_delete_only: self.soft_delete_only,
            auto_aging_enabled: self.auto_aging_enabled,
            auto_aging_interval_hours: self.auto_aging_interval_hours,
            min_age_days: self.min_age_days,
            max_access_count: self.max_access_count,
            max_deprecate_percent: self.max_deprecate_percent,
            min_deprecate_per_cycle: self.min_deprecate_per_cycle,
            max_deprecate_per_cycle: self.max_deprecate_per_cycle,
            deprecated_ttl_days: self.deprecated_ttl_days,
            auto_prune_enabled: self.auto_prune_enabled,
            dream_dedup_soft_delete: self.dream_dedup_soft_delete,
            dream_compact_soft_delete: self.dream_compact_soft_delete,
        }
    }
}

/// Full uteke configuration, loaded from `uteke.toml`.
#[derive(serde::Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub store: StoreConfig,
    #[serde(default)]
    pub vector: VectorConfig,
    pub embedding: EmbeddingConfig,
    pub extraction: ExtractionConfig,
    pub embed_fallback: EmbedFallbackConfig,
    pub tier: TierConfig,
    pub logging: LoggingConfig,
    pub aging: AgingConfig,
    pub recall: RecallConfig,
    pub server: ServerConfig,
    pub limits: LimitsConfig,
    pub maintenance: MaintenanceConfig,
    pub dream: DreamConfig,
    pub lifecycle: LifecycleConfig,
    /// Enable background update notification on startup. Default: true.
    /// Set to `false` to disable.
    #[serde(default = "default_true")]
    pub update_check: bool,
    /// Show a one-line "Like Uteke?" footer after `uteke doctor` succeeds
    /// (#1246). Interactive terminals only. Default: true. Set to `false`
    /// to disable.
    #[serde(default = "default_true")]
    pub doctor_footer: bool,
}

/// Serde default helper: returns `true`.
fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            store: StoreConfig::default(),
            vector: VectorConfig::default(),
            embedding: EmbeddingConfig::default(),
            extraction: ExtractionConfig::default(),
            embed_fallback: EmbedFallbackConfig::default(),
            tier: TierConfig::default(),
            logging: LoggingConfig::default(),
            aging: AgingConfig::default(),
            recall: RecallConfig::default(),
            server: ServerConfig::default(),
            limits: LimitsConfig::default(),
            maintenance: MaintenanceConfig::default(),
            dream: DreamConfig::default(),
            lifecycle: LifecycleConfig::default(),
            update_check: true,
            doctor_footer: true,
        }
    }
}

/// Configurable limits (#404).
/// All limits can be overridden via config or env vars.
#[derive(serde::Deserialize, Clone)]
#[serde(default)]
pub struct LimitsConfig {
    /// Maximum memory content length in characters. Set to 0 to disable.
    pub max_content_length: usize,
    /// Maximum number of tags per memory.
    pub max_tags_count: usize,
    /// Maximum single tag length in characters.
    pub max_tag_length: usize,
    /// Maximum payload size for server API in bytes.
    pub max_payload_size: usize,
    /// Default recall limit when --limit not specified.
    pub default_recall_limit: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        // Env var overrides with fallback to defaults.
        Self {
            max_content_length: env_or("UTEKE_MAX_CONTENT_LENGTH", 100_000),
            max_tags_count: env_or("UTEKE_MAX_TAGS_COUNT", 20),
            max_tag_length: env_or("UTEKE_MAX_TAG_LENGTH", 50),
            max_payload_size: env_or("UTEKE_MAX_PAYLOAD_SIZE", 10_485_760),
            default_recall_limit: env_or("UTEKE_DEFAULT_RECALL_LIMIT", 5),
        }
    }
}

/// Opt-in escape hatch: allow the project-local `.uteke/uteke.toml` to set
/// endpoints, API keys and the server address.
fn project_config_trusted() -> bool {
    matches!(
        std::env::var("UTEKE_TRUST_PROJECT_CONFIG").as_deref(),
        Ok("1") | Ok("true")
    )
}

impl Config {
    /// Load config with layered resolution:
    /// 1. Defaults
    /// 2. Global `{uteke_home}/uteke.toml`
    /// 3. Project `.uteke/uteke.toml`
    ///
    /// Each layer overrides the previous. Legacy `config.toml` is migrated
    /// to `uteke.toml` when found.
    pub fn load() -> Self {
        // Migrate legacy config.toml → uteke.toml at global location
        migrate_legacy_global();

        let global = global_config_path();
        let project = std::env::current_dir()
            .ok()
            .map(|cwd| cwd.join(".uteke").join("uteke.toml"));
        let config = Self::from_layers(&Layers {
            global: global.as_deref(),
            project: project.as_deref(),
            trust_project: project_config_trusted(),
        });

        // Layer 3: environment variables (override config file)
        config.apply_env_overrides()
    }

    /// Resolve the global and project-local config files into a `Config`.
    ///
    /// Merge precedence, the untrusted-project-key policy and per-layer
    /// validation live in `uteke_core::config_layers`, shared with the server,
    /// so a new key needs only a field here — no hand-written merge line.
    pub fn from_layers(layers: &Layers<'_>) -> Self {
        uteke_core::config_layers::resolve::<Config>(layers).value
    }

    /// Apply environment variable overrides on top of config file values.
    ///
    /// Resolution order (highest priority first):
    /// 1. CLI flags
    /// 2. Environment variables (UTEKE_*)
    /// 3. Config file (uteke.toml)
    /// 4. Built-in defaults
    fn apply_env_overrides(mut self) -> Self {
        // Logging
        if let Ok(v) = std::env::var("UTEKE_LOG_LEVEL") {
            self.logging.level = v;
        }

        // Server
        if let Ok(v) = std::env::var("UTEKE_SERVER_HOST") {
            self.server.host = v;
        }
        if let Ok(v) = std::env::var("UTEKE_SERVER_PORT") {
            match v.parse::<u16>() {
                Ok(port) => self.server.port = port,
                Err(_) => {
                    tracing::warn!("Invalid UTEKE_SERVER_PORT='{v}', ignoring (expected 0-65535)")
                }
            }
        }

        // Recall thresholds (must be 0.0-1.0)
        if let Ok(v) = std::env::var("UTEKE_RECALL_MIN_SCORE") {
            match v.parse::<f64>() {
                Ok(score) if (0.0..=1.0).contains(&score) => self.recall.min_score = score,
                Ok(_) => tracing::warn!(
                    "UTEKE_RECALL_MIN_SCORE='{v}' out of range, ignoring (expected 0.0-1.0)"
                ),
                Err(_) => tracing::warn!(
                    "Invalid UTEKE_RECALL_MIN_SCORE='{v}', ignoring (expected 0.0-1.0)"
                ),
            }
        }
        if let Ok(v) = std::env::var("UTEKE_RECALL_MIN_SCORE_STRICT") {
            match v.parse::<f64>() {
                Ok(score) if (0.0..=1.0).contains(&score) => self.recall.min_score_strict = score,
                Ok(_) => tracing::warn!(
                    "UTEKE_RECALL_MIN_SCORE_STRICT='{v}' out of range, ignoring (expected 0.0-1.0)"
                ),
                Err(_) => tracing::warn!(
                    "Invalid UTEKE_RECALL_MIN_SCORE_STRICT='{v}', ignoring (expected 0.0-1.0)"
                ),
            }
        }

        // Graph-augmented reranking overrides (#378)
        if let Ok(v) = std::env::var("UTEKE_RECALL_STRATEGY") {
            if matches!(
                v.as_str(),
                "vector" | "fts5" | "hybrid" | "graph" | "fusion"
            ) {
                self.recall.default_strategy = v;
            } else {
                tracing::warn!(
                    "Invalid UTEKE_RECALL_STRATEGY='{v}', ignoring (expected vector|fts5|hybrid|graph|fusion)"
                );
            }
        }
        if let Ok(v) = std::env::var("UTEKE_GRAPH_DENSITY_WEIGHT") {
            match v.parse::<f32>() {
                Ok(w) if (0.0..=1.0).contains(&w) => self.recall.graph_density_weight = w,
                Ok(_) => tracing::warn!(
                    "UTEKE_GRAPH_DENSITY_WEIGHT='{v}' out of range, ignoring (expected 0.0-1.0)"
                ),
                Err(_) => tracing::warn!(
                    "Invalid UTEKE_GRAPH_DENSITY_WEIGHT='{v}', ignoring (expected 0.0-1.0)"
                ),
            }
        }
        if let Ok(v) = std::env::var("UTEKE_GRAPH_AUTHORITY_WEIGHT") {
            match v.parse::<f32>() {
                Ok(w) if (0.0..=1.0).contains(&w) => self.recall.graph_authority_weight = w,
                Ok(_) => tracing::warn!(
                    "UTEKE_GRAPH_AUTHORITY_WEIGHT='{v}' out of range, ignoring (expected 0.0-1.0)"
                ),
                Err(_) => tracing::warn!(
                    "Invalid UTEKE_GRAPH_AUTHORITY_WEIGHT='{v}', ignoring (expected 0.0-1.0)"
                ),
            }
        }
        if let Ok(v) = std::env::var("UTEKE_GRAPH_RERANK_ENABLED") {
            match v.to_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => self.recall.graph_rerank_enabled = true,
                "0" | "false" | "no" | "off" => self.recall.graph_rerank_enabled = false,
                _ => tracing::warn!(
                    "Invalid UTEKE_GRAPH_RERANK_ENABLED='{v}', ignoring (expected true/false)"
                ),
            }
        }

        // Vector engine override (#1168). Ignored when the requested engine
        // is not compiled in (resolution falls back with a warning in core).
        if let Ok(v) = std::env::var("UTEKE_VECTOR_BACKEND") {
            if !v.is_empty() {
                self.vector.backend = v;
            }
        }

        // Embedding backend overrides (#337)
        if let Ok(v) = std::env::var("UTEKE_EMBEDDING_BACKEND") {
            if !v.is_empty() {
                self.embedding.backend = v;
            }
        }
        if let Ok(v) = std::env::var("UTEKE_EMBEDDING_MODEL") {
            if !v.is_empty() {
                self.embedding.model = v;
            }
        }
        // API key: prefer UTEKE_EMBEDDING_API_KEY, then OPENAI_API_KEY fallback.
        // An explicitly empty env var is treated as unset so it cannot clobber
        // a non-empty [embedding].api_key from uteke.toml (CodeCora finding).
        if let Ok(v) = std::env::var("UTEKE_EMBEDDING_API_KEY") {
            if !v.is_empty() {
                self.embedding.api_key = v;
            }
        } else if let Ok(v) = std::env::var("OPENAI_API_KEY") {
            if !v.is_empty() {
                self.embedding.api_key = v;
            }
        }
        if let Ok(v) = std::env::var("UTEKE_EMBEDDING_BASE_URL") {
            if !v.is_empty() {
                self.embedding.base_url = v;
            }
        }
        if let Ok(v) = std::env::var("UTEKE_EMBEDDING_ENDPOINT_PATH") {
            if !v.is_empty() {
                self.embedding.endpoint_path = v;
            }
        }
        if let Ok(v) = std::env::var("UTEKE_EMBEDDING_DIMS") {
            match v.parse::<usize>() {
                Ok(d) => self.embedding.dims = d,
                Err(_) => tracing::warn!(
                    "Invalid UTEKE_EMBEDDING_DIMS='{v}', ignoring (expected integer)"
                ),
            }
        }
        if let Ok(v) = std::env::var("UTEKE_MAX_SEQ_LENGTH") {
            match v.parse::<usize>() {
                Ok(len) if len > 0 => self.embedding.max_seq_length = len,
                Ok(_) | Err(_) => tracing::warn!(
                    "Invalid UTEKE_MAX_SEQ_LENGTH='{v}', ignoring (expected positive integer)"
                ),
            }
        }

        // Extraction (import --extract). All optional; inert unless --extract.
        if let Ok(v) = std::env::var("UTEKE_EXTRACTION_MODEL") {
            if !v.is_empty() {
                self.extraction.model = v;
            }
        }
        if let Ok(v) = std::env::var("UTEKE_EXTRACTION_API_KEY") {
            if !v.is_empty() {
                self.extraction.api_key = v;
            }
        }
        if let Ok(v) = std::env::var("UTEKE_EXTRACTION_BASE_URL") {
            if !v.is_empty() {
                self.extraction.base_url = v;
            }
        }
        if let Ok(v) = std::env::var("UTEKE_EXTRACTION_ENDPOINT_PATH") {
            if !v.is_empty() {
                self.extraction.endpoint_path = v;
            }
        }
        if let Ok(v) = std::env::var("UTEKE_EXTRACTION_MAX_FACTS") {
            match v.parse::<usize>() {
                Ok(n) => self.extraction.max_facts = n,
                Err(_) => tracing::warn!(
                    "Invalid UTEKE_EXTRACTION_MAX_FACTS='{v}', ignoring (expected integer)"
                ),
            }
        }

        // Embed fallback (cloud API when local ONNX fails). All optional.
        self.embed_fallback = self.embed_fallback.clone().resolve_with_env();

        self
    }

    /// Ensure the global uteke directory exists and return its path.
    pub fn ensure_dirs() -> PathBuf {
        let base = uteke_core::uteke_home().expect("Cannot determine uteke home directory");
        std::fs::create_dir_all(&base).ok();
        std::fs::create_dir_all(base.join("models")).ok();
        base
    }

    /// Write a default `uteke.toml` at the global location if none exists.
    pub fn write_default_config() {
        let base = Self::ensure_dirs();
        let config_path = base.join("uteke.toml");
        if config_path.exists() {
            return;
        }
        let default = r#"# Uteke configuration
# See https://github.com/codecoradev/uteke for documentation

[store]
# path = "~/.codecora/uteke"
# namespace = "default"

[embedding]
# backend = "onnx"  # future: "openai", "ollama"
# model = "embeddinggemma-q4"
# max_seq_length = 2048

[tier]
# hot_days = 7
# warm_days = 30
# hot_boost = 0.1

[logging]
# level = "warn"
# file = ""

[aging]
# Aging controls which old, rarely-accessed memories get cleaned up.
# A memory is a cleanup candidate ONLY if ALL conditions are met:
#   - older than max_age_days
#   - access_count < max_access_count
#   - not pinned
#   - not deprecated
#   - not accessed since max_age_days ago
# enabled = false
# max_age_days = 365
# max_access_count = 10
# max_cold_count = 1000

[recall]
# min_score = 0.0  # default since 0.17.x (#1223): fusion scores are rank-based; 0.3 is a legacy cosine-era threshold
# min_score_strict = 0.5
# default_strategy = "fusion"  # vector | fts5 | hybrid | graph | fusion
# graph_density_weight = 0.1
# graph_authority_weight = 0.1
# graph_rerank_enabled = true
# jaccard_weight = 0.0  # Post-RRF token overlap boost (#719). 0=off, 0.10-0.15 recommended
# salience_weight = 0.0  # Post-RRF importance/pin boost. 0=off, 0.05-0.10 recommended
# recency_weight = 0.0   # Post-RRF time-decay boost. 0=off, 0.05-0.10 recommended

[embed_fallback]
# Fallback when embedding model fails or is unavailable (#598)
# enabled = true
# strategy = "hash"  # hash | random | zero

[extraction]
# Content extraction settings for document chunking
# max_chunk_size = 512
# overlap = 50

[server]
# enabled = false
# host = "127.0.0.1"
# port = 8767

[limits]
# Configurable limits (#404). Override via config or env vars:
# UTEKE_MAX_CONTENT_LENGTH, UTEKE_MAX_TAGS_COUNT, etc.
# max_content_length = 100000  # Set to 0 to disable
# max_tags_count = 20
# max_tag_length = 50
# max_payload_size = 10485760  # 10MB
# default_recall_limit = 5

[maintenance]
# Auto-maintenance daemon (runs in server background)
# auto_aging_enabled = false      # Opt-in — auto-delete should be explicit
# auto_aging_interval_hours = 24   # Daily (not every 6h)
# auto_dream_enabled = true        # Run dream cycle (lint → dedup → orphans)
# auto_dream_interval_days = 7     # Weekly (not every 3d)

[dream]
# Dream pipeline thresholds — all phases read from these values
# contradict_similarity_threshold = 0.6   # Cosine > this → NOT contradiction
# contradict_tag_jaccard_min = 0.4       # Tag overlap ≥ this → consider contradict
# contradict_max_memories = 200          # O(n²) scan limit
# dedup_threshold = 0.92                # Cosine > this → merge candidate
# orphan_importance_threshold = 0.15     # Importance < this → orphan candidate
"#;
        std::fs::write(&config_path, default).ok();
    }

    /// Expand `~` in a path string to the actual home directory.
    pub fn expand_tilde(path: &str) -> String {
        if path.starts_with("~/") {
            dirs::home_dir()
                .map(|h| {
                    let rest = &path[2..];
                    h.join(rest).to_string_lossy().to_string()
                })
                .unwrap_or_else(|| path.to_string())
        } else {
            path.to_string()
        }
    }

    /// Set the default namespace in the global config file.
    /// Creates or updates the `[store]` section's `namespace` key.
    pub fn set_default_namespace(name: &str) -> Result<(), String> {
        // Must be the same file `load()` reads via global_config_path(), or the write
        // lands in a file nothing consults and the command is a silent no-op.
        let config_path = global_config_path().ok_or("Cannot determine uteke home directory")?;

        // Read existing config or start fresh
        let content = if config_path.exists() {
            std::fs::read_to_string(&config_path)
                .map_err(|e| format!("Failed to read config: {e}"))?
        } else {
            String::new()
        };

        let updated = set_namespace_in_toml(&content, name);
        std::fs::write(&config_path, updated)
            .map_err(|e| format!("Failed to write config: {e}"))?;

        Ok(())
    }
}

// ── Legacy migration ────────────────────────────────────────────────────────

/// Migrate legacy `{uteke_home}/config.toml` → `{uteke_home}/uteke.toml`.
/// Read an env var as a type T, falling back to default if unset or invalid.
fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn migrate_legacy_global() {
    let base = match uteke_core::uteke_home() {
        Ok(b) => b,
        Err(_) => return,
    };
    let legacy = base.join("config.toml");
    let modern = base.join("uteke.toml");

    if legacy.exists() && !modern.exists() {
        tracing::info!(
            "Migrating legacy config {} → {}",
            legacy.display(),
            modern.display()
        );
        if let Ok(content) = std::fs::read_to_string(&legacy) {
            // Try to parse old format and rewrite as new format
            let new_content = migrate_content(&content);
            if std::fs::write(&modern, &new_content).is_ok() {
                // Rename old file as backup
                let backup = base.join("config.toml.bak");
                let _ = std::fs::rename(&legacy, &backup);
            }
        }
    }
}

/// Convert old `config.toml` content to new `uteke.toml` format.
///
/// The legacy format had FLAT top-level keys (`store_path`, `namespace`,
/// `model`, `max_seq_length`). Only keys that appear BEFORE the first
/// `[section]` header are legacy: once a header has been seen, everything below
/// is already new-format and passes through untouched. (Treating a `model`
/// inside `[extraction]` as the legacy embedding model, or a `namespace` inside
/// `[store]` as a flat key, relocated them into extra `[store]`/`[embedding]`
/// tables at the end — duplicate tables, i.e. invalid TOML — #1332.)
fn migrate_content(old: &str) -> String {
    let mut out = String::from("# Migrated from config.toml\n");
    let mut store_section = String::new();
    let mut embedding_section = String::new();

    let has_section = |name: &str| old.lines().any(|l| l.trim() == format!("[{name}]"));
    let (has_store, has_embedding) = (has_section("store"), has_section("embedding"));

    let mut in_section = false;
    for line in old.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && !in_section && trimmed.ends_with(']') {
            in_section = true;
        }
        if in_section || trimmed.is_empty() || trimmed.starts_with('#') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            // Continuation of a multi-line value, or a stray line: keep it.
            out.push_str(line);
            out.push('\n');
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let (target, has_target, new_key, table) = match key {
            "store_path" => (&mut store_section, has_store, "path", "store"),
            "namespace" => (&mut store_section, has_store, "namespace", "store"),
            "model" => (&mut embedding_section, has_embedding, "model", "embedding"),
            "max_seq_length" => (
                &mut embedding_section,
                has_embedding,
                "max_seq_length",
                "embedding",
            ),
            _ => {
                // Unknown top-level key — pass through
                out.push_str(line);
                out.push('\n');
                continue;
            }
        };
        if has_target {
            // The file already defines that table: the new-format value wins and
            // we must not emit the table a second time.
            out.push_str(&format!(
                "# legacy `{key}` ignored: a [{table}] section already exists\n"
            ));
        } else {
            target.push_str(&format!("{new_key} = {value}\n"));
        }
    }

    if !store_section.is_empty() {
        out.push_str("[store]\n");
        out.push_str(&store_section);
    }
    if !embedding_section.is_empty() {
        out.push_str("[embedding]\n");
        out.push_str(&embedding_section);
    }

    out
}

/// Return the global config path `{uteke_home}/uteke.toml`.
fn global_config_path() -> Option<PathBuf> {
    uteke_core::uteke_home().ok().map(|h| h.join("uteke.toml"))
}

/// Update or insert the namespace value in a TOML config string.
/// Preserves all other content.
fn set_namespace_in_toml(content: &str, namespace: &str) -> String {
    let mut in_store_section = false;
    let mut found_namespace_key = false;
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    for line in lines.iter_mut() {
        let trimmed = line.trim();
        if trimmed == "[store]" {
            in_store_section = true;
            continue;
        }
        if trimmed.starts_with('[') && trimmed != "[store]" {
            in_store_section = false;
        }
        if in_store_section && trimmed.starts_with("namespace") {
            *line = format!("namespace = \"{namespace}\"");
            found_namespace_key = true;
            break;
        }
    }

    if !found_namespace_key {
        // Need to insert namespace into [store] section
        if let Some(pos) = lines.iter().position(|l| l.trim() == "[store]") {
            lines.insert(pos + 1, format!("namespace = \"{namespace}\""));
        } else {
            // No [store] section exists — append one
            lines.push(String::new());
            lines.push("[store]".to_string());
            lines.push(format!("namespace = \"{namespace}\""));
        }
    }

    lines.join("\n")
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Resolve `toml` as the global layer (every key allowed).
    fn from_toml(toml: &str) -> Config {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uteke.toml");
        std::fs::write(&path, toml).unwrap();
        Config::from_layers(&Layers {
            global: Some(&path),
            project: None,
            trust_project: false,
        })
    }

    /// Resolve `toml` as the project-local layer on top of nothing.
    fn from_project_toml(toml: &str, trust_project: bool) -> Config {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uteke.toml");
        std::fs::write(&path, toml).unwrap();
        Config::from_layers(&Layers {
            global: None,
            project: Some(&path),
            trust_project,
        })
    }

    #[test]
    fn default_config_values() {
        let cfg = Config::default();
        assert_eq!(cfg.store.path, "~/.codecora/uteke");
        assert_eq!(cfg.store.namespace, "default");
        assert_eq!(cfg.embedding.model, "embeddinggemma-q4");
        assert_eq!(cfg.embedding.backend, "onnx");
        assert_eq!(cfg.embedding.max_seq_length, 2048);
        assert_eq!(cfg.tier.hot_days, 7);
        assert_eq!(cfg.tier.warm_days, 30);
        assert!((cfg.tier.hot_boost - 0.1).abs() < f64::EPSILON);
        assert_eq!(cfg.logging.level, "warn");
        assert!(cfg.logging.file.is_empty());
        assert!(!cfg.aging.enabled);
        assert_eq!(cfg.aging.max_age_days, 365);
        assert_eq!(cfg.aging.max_access_count, 10);
        assert_eq!(cfg.aging.max_cold_count, 1000);
    }

    #[test]
    fn parse_full_config() {
        let toml = r#"
[store]
path = "/data/uteke"
namespace = "test-ns"

[embedding]
backend = "openai"
model = "custom-model"
max_seq_length = 512

[tier]
hot_days = 3
warm_days = 14
hot_boost = 0.2

[logging]
level = "debug"
file = "/tmp/uteke.log"

[aging]
enabled = true
max_age_days = 90
max_cold_count = 5000
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.store.path, "/data/uteke");
        assert_eq!(cfg.store.namespace, "test-ns");
        assert_eq!(cfg.embedding.model, "custom-model");
        assert_eq!(cfg.embedding.backend, "openai");
        assert_eq!(cfg.embedding.max_seq_length, 512);
        assert_eq!(cfg.tier.hot_days, 3);
        assert_eq!(cfg.tier.warm_days, 14);
        assert!((cfg.tier.hot_boost - 0.2).abs() < f64::EPSILON);
        assert_eq!(cfg.logging.level, "debug");
        assert_eq!(cfg.logging.file, "/tmp/uteke.log");
        assert!(cfg.aging.enabled);
        assert_eq!(cfg.aging.max_age_days, 90);
        assert_eq!(cfg.aging.max_cold_count, 5000);
    }

    #[test]
    fn parse_partial_config() {
        let toml = r#"
[store]
namespace = "my-ns"

[logging]
level = "info"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        // Specified values
        assert_eq!(cfg.store.namespace, "my-ns");
        assert_eq!(cfg.logging.level, "info");
        // Everything else is default
        assert_eq!(cfg.store.path, "~/.codecora/uteke");
        assert_eq!(cfg.embedding.model, "embeddinggemma-q4");
        assert_eq!(cfg.embedding.backend, "onnx");
        assert_eq!(cfg.embedding.max_seq_length, 2048);
        assert_eq!(cfg.tier.hot_days, 7);
        assert!(!cfg.aging.enabled);
    }

    #[test]
    fn merge_file_overrides_non_defaults() {
        let toml = r#"
[store]
path = "/custom/store"
namespace = "prod"

[embedding]
model = "other-model"
"#;
        let merged = from_toml(toml);

        assert_eq!(merged.store.path, "/custom/store");
        assert_eq!(merged.store.namespace, "prod");
        assert_eq!(merged.embedding.model, "other-model");
        // Unchanged defaults
        assert_eq!(merged.embedding.max_seq_length, 2048);
        assert_eq!(merged.tier.hot_days, 7);
    }

    #[test]
    fn merge_nonexistent_file_returns_self() {
        let cfg = Config::default();
        let missing = std::path::Path::new("/no/such/file.toml");
        let merged = Config::from_layers(&Layers {
            global: Some(missing),
            project: Some(missing),
            trust_project: false,
        });
        assert_eq!(merged.store.path, cfg.store.path);
    }

    #[test]
    fn merge_all_sections_from_file() {
        // Regression test for #856: verify all 12 sections are merged.
        let toml = r#"
[recall]
salience_weight = 0.25
recency_weight = 0.20

[embed_fallback]
api_key = "sk-test"
base_url = "https://embed.example.com"
model = "text-embed"

[extraction]
model = "gpt-4o"
max_facts = 30

[limits]
max_content_length = 50000
default_recall_limit = 10

[maintenance]
auto_aging_enabled = true
auto_aging_interval_hours = 12
auto_dream_enabled = false

[dream]
dedup_threshold = 0.95
orphan_importance_threshold = 0.10
"#;
        let merged = from_toml(toml);

        // recall (salience/recency_weight were the original missing fields)
        assert!((merged.recall.salience_weight - 0.25).abs() < f32::EPSILON);
        assert!((merged.recall.recency_weight - 0.20).abs() < f32::EPSILON);
        // embed_fallback
        assert_eq!(merged.embed_fallback.api_key, "sk-test");
        assert_eq!(merged.embed_fallback.base_url, "https://embed.example.com");
        assert_eq!(merged.embed_fallback.model, "text-embed");
        // extraction
        assert_eq!(merged.extraction.model, "gpt-4o");
        assert_eq!(merged.extraction.max_facts, 30);
        // limits
        assert_eq!(merged.limits.max_content_length, 50000);
        assert_eq!(merged.limits.default_recall_limit, 10);
        // maintenance
        assert!(merged.maintenance.auto_aging_enabled);
        assert_eq!(merged.maintenance.auto_aging_interval_hours, 12);
        assert!(!merged.maintenance.auto_dream_enabled);
        // dream
        assert!((merged.dream.dedup_threshold - 0.95).abs() < f32::EPSILON);
        assert!((merged.dream.orphan_importance_threshold - 0.10).abs() < f64::EPSILON);
    }

    #[test]
    fn expand_tilde() {
        let expanded = Config::expand_tilde("~/foo");
        assert!(!expanded.starts_with('~'));
        assert!(expanded.ends_with("foo"));

        let no_tilde = Config::expand_tilde("/absolute/path");
        assert_eq!(no_tilde, "/absolute/path");
    }

    /// #1332: a flat file that ALSO has sections used to gain a second
    /// `[store]`/`[embedding]` table (invalid TOML) and misplace `model`.
    #[test]
    fn migrate_content_mixed_flat_and_sectioned_stays_valid_toml() {
        let old = r#"namespace = "legacy-ns"
model = "legacy-model"

[store]
path = "/data/mem"
namespace = "new-ns"

[embedding]
model = "new-model"

[extraction]
model = "gpt-4o"
"#;
        let migrated = migrate_content(old);
        let parsed: toml::Value = toml::from_str(&migrated)
            .unwrap_or_else(|e| panic!("migrated config must parse: {e}\n{migrated}"));
        assert_eq!(parsed["store"]["namespace"].as_str(), Some("new-ns"));
        assert_eq!(parsed["store"]["path"].as_str(), Some("/data/mem"));
        assert_eq!(parsed["embedding"]["model"].as_str(), Some("new-model"));
        assert_eq!(
            parsed["extraction"]["model"].as_str(),
            Some("gpt-4o"),
            "a model key inside [extraction] is not the legacy embedding model"
        );
    }

    #[test]
    fn migrate_content_keeps_multiline_values_and_unknown_keys() {
        let old = "custom = 1\nlist = [\n  \"a\",\n  \"b\",\n]\nnamespace = \"x\"\n";
        let migrated = migrate_content(old);
        let parsed: toml::Value =
            toml::from_str(&migrated).unwrap_or_else(|e| panic!("must parse: {e}\n{migrated}"));
        assert_eq!(parsed["custom"].as_integer(), Some(1));
        assert_eq!(parsed["list"].as_array().map(|a| a.len()), Some(2));
        assert_eq!(parsed["store"]["namespace"].as_str(), Some("x"));
    }

    #[test]
    fn migrate_content_old_format() {
        let old = r#"store_path = "/data/mem"
namespace = "agent1"
"#;
        let migrated = migrate_content(old);
        assert!(migrated.contains("[store]"));
        assert!(migrated.contains("path = \"/data/mem\""));
        assert!(migrated.contains("namespace = \"agent1\""));
    }

    #[test]
    fn set_namespace_in_toml_existing_section() {
        let content = "[store]\npath = \"~/.uteke\"\n# namespace = \"default\"\n\n[logging]\nlevel = \"warn\"\n";
        let result = set_namespace_in_toml(content, "my-agent");
        assert!(result.contains("namespace = \"my-agent\""));
        assert!(result.contains("[store]"));
        assert!(result.contains("[logging]"));
    }

    #[test]
    fn set_namespace_in_toml_no_store_section() {
        let content = "[logging]\nlevel = \"warn\"\n";
        let result = set_namespace_in_toml(content, "new-ns");
        assert!(result.contains("[store]"));
        assert!(result.contains("namespace = \"new-ns\""));
        assert!(result.contains("[logging]"));
    }

    #[test]
    fn set_namespace_in_toml_empty_content() {
        let content = "";
        let result = set_namespace_in_toml(content, "empty-ns");
        assert!(result.contains("[store]"));
        assert!(result.contains("namespace = \"empty-ns\""));
    }

    #[test]
    fn set_namespace_in_toml_update_existing() {
        let content = "[store]\nnamespace = \"old-ns\"\npath = \"~/.uteke\"\n";
        let result = set_namespace_in_toml(content, "new-ns");
        assert!(result.contains("namespace = \"new-ns\""));
        assert!(!result.contains("namespace = \"old-ns\""));
    }

    #[test]
    fn default_recall_config() {
        let cfg = RecallConfig::default();
        assert!((cfg.min_score - 0.0).abs() < f64::EPSILON);
        assert!((cfg.min_score_strict - 0.5).abs() < f64::EPSILON);
        // Fusion (vector×1.7 + hybrid×1 weighted RRF) is the default
        // strategy since 0.16.0 (#1123).
        assert_eq!(cfg.default_strategy, "fusion");
    }

    #[test]
    fn parse_recall_config() {
        let toml = r#"
[recall]
min_score = 0.45
min_score_strict = 0.7
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert!((cfg.recall.min_score - 0.45).abs() < f64::EPSILON);
        assert!((cfg.recall.min_score_strict - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn merge_recall_config() {
        let toml = r#"
[recall]
min_score = 0.6
min_score_strict = 0.8
"#;
        let merged = from_toml(toml);

        assert!((merged.recall.min_score - 0.6).abs() < f64::EPSILON);
        assert!((merged.recall.min_score_strict - 0.8).abs() < f64::EPSILON);
        // Other config untouched
        assert_eq!(merged.store.path, "~/.codecora/uteke");
        assert_eq!(merged.embedding.model, "embeddinggemma-q4");
        assert_eq!(merged.tier.hot_days, 7);
        assert_eq!(merged.logging.level, "warn");
        assert!(!merged.aging.enabled);
        assert!(!merged.server.enabled);
    }

    #[test]
    fn merge_all_sections_in_one_file() {
        let toml = r#"
[store]
path = "/custom/path"
namespace = "full-test"

[embedding]
backend = "onnx"
model = "custom-embed"
max_seq_length = 512

[tier]
hot_days = 5
warm_days = 21
hot_boost = 0.3

[logging]
level = "trace"
file = "/tmp/test.log"

[aging]
enabled = true
max_age_days = 60
max_cold_count = 2000

[recall]
min_score = 0.4
min_score_strict = 0.65

[server]
enabled = true
host = "0.0.0.0"
port = 9999
"#;
        let merged = from_toml(toml);

        assert_eq!(merged.store.path, "/custom/path");
        assert_eq!(merged.store.namespace, "full-test");
        assert_eq!(merged.embedding.model, "custom-embed");
        assert_eq!(merged.embedding.backend, "onnx");
        assert_eq!(merged.embedding.max_seq_length, 512);
        assert_eq!(merged.tier.hot_days, 5);
        assert_eq!(merged.tier.warm_days, 21);
        assert!((merged.tier.hot_boost - 0.3).abs() < f64::EPSILON);
        assert_eq!(merged.logging.level, "trace");
        assert_eq!(merged.logging.file, "/tmp/test.log");
        assert!(merged.aging.enabled);
        assert_eq!(merged.aging.max_age_days, 60);
        assert_eq!(merged.aging.max_cold_count, 2000);
        assert!((merged.recall.min_score - 0.4).abs() < f64::EPSILON);
        assert!((merged.recall.min_score_strict - 0.65).abs() < f64::EPSILON);
        assert!(merged.server.enabled);
        assert_eq!(merged.server.host, "0.0.0.0");
        assert_eq!(merged.server.port, 9999);
    }

    #[test]
    fn project_config_cannot_redirect_endpoints() {
        // Dummy credential line assembled at runtime (not a real secret).
        let cred_line = format!("{} = \"{}\"", "api_key", "k".repeat(12));
        let toml = format!(
            r#"
[embedding]
backend = "openai"
model = "evil-model"
base_url = "https://evil.example/v1"
endpoint_path = "/steal"
{cred_line}

[extraction]
base_url = "https://evil.example/x"

[server]
enabled = true
host = "evil.example"
port = 1
"#
        );
        let trusted = Config::default();
        let merged = from_project_toml(&toml, false);

        // Sensitive fields keep the trusted (global/default) values...
        assert_eq!(merged.embedding.backend, trusted.embedding.backend);
        assert_eq!(merged.embedding.base_url, trusted.embedding.base_url);
        assert_eq!(
            merged.embedding.endpoint_path,
            trusted.embedding.endpoint_path
        );
        assert_eq!(merged.embedding.api_key, trusted.embedding.api_key);
        assert_eq!(merged.extraction.base_url, trusted.extraction.base_url);
        assert_eq!(merged.server.host, trusted.server.host);
        assert_eq!(merged.server.port, trusted.server.port);
        assert_eq!(merged.server.enabled, trusted.server.enabled);
        // ...while harmless tuning from the project file still applies.
        assert_eq!(merged.embedding.model, "evil-model");

        // Explicit opt-in lets the project file set them.
        let trusted_project = from_project_toml(&toml, true);
        assert_eq!(
            trusted_project.embedding.base_url,
            "https://evil.example/v1"
        );
        assert_eq!(trusted_project.server.host, "evil.example");
    }

    #[test]
    fn merge_applies_lifecycle_toplevel_and_aging_access_count() {
        let toml = r#"
update_check = false
doctor_footer = false

[aging]
max_access_count = 7

[lifecycle]
soft_delete_only = false
min_age_days = 11
deprecated_ttl_days = 5
"#;
        let defaults = Config::default();
        let merged = from_toml(toml);

        assert!(!merged.update_check);
        assert!(!merged.doctor_footer);
        assert_eq!(merged.aging.max_access_count, 7);
        assert!(!merged.lifecycle.soft_delete_only);
        assert_eq!(merged.lifecycle.min_age_days, 11);
        assert_eq!(merged.lifecycle.deprecated_ttl_days, 5);
        // Keys absent from the file keep their defaults.
        assert_eq!(
            merged.lifecycle.max_deprecate_per_cycle,
            defaults.lifecycle.max_deprecate_per_cycle
        );
        assert_eq!(
            merged.lifecycle.auto_aging_enabled,
            defaults.lifecycle.auto_aging_enabled
        );
    }

    #[test]
    fn invalid_toml_layer_is_skipped() {
        let merged = from_toml("this is not valid toml [[[[");
        // Should return defaults when file is invalid
        assert_eq!(merged.store.path, "~/.codecora/uteke");
    }

    #[test]
    fn merge_embedding_backend() {
        let toml = r#"
[embedding]
backend = "ollama"
"#;
        let merged = from_toml(toml);
        assert_eq!(merged.embedding.backend, "ollama");
        // Other embedding fields stay default
        assert_eq!(merged.embedding.model, "embeddinggemma-q4");
        assert_eq!(merged.embedding.max_seq_length, 2048);
    }

    #[test]
    fn merge_embedding_full_openai_config() {
        let toml = r#"
[embedding]
backend = "openai"
model = "text-embedding-3-large"
api_key = "sk-test-123"
base_url = "https://my-proxy.example.com/v1"
dims = 3072
max_seq_length = 8191
"#;
        let merged = from_toml(toml);
        assert_eq!(merged.embedding.backend, "openai");
        assert_eq!(merged.embedding.model, "text-embedding-3-large");
        assert_eq!(merged.embedding.api_key, "sk-test-123");
        assert_eq!(merged.embedding.base_url, "https://my-proxy.example.com/v1");
        assert_eq!(merged.embedding.dims, 3072);
        assert_eq!(merged.embedding.max_seq_length, 8191);
    }

    #[test]
    #[serial_test::serial]
    fn env_overrides_embedding_backend() {
        unsafe {
            std::env::set_var("UTEKE_EMBEDDING_BACKEND", "openai");
        }
        unsafe {
            std::env::set_var("UTEKE_EMBEDDING_API_KEY", "sk-env-test");
        }
        unsafe {
            std::env::set_var("UTEKE_EMBEDDING_MODEL", "text-embedding-3-small");
        }
        unsafe {
            std::env::set_var("UTEKE_EMBEDDING_DIMS", "1536");
        }
        let cfg = Config::default().apply_env_overrides();
        unsafe {
            std::env::remove_var("UTEKE_EMBEDDING_BACKEND");
        }
        unsafe {
            std::env::remove_var("UTEKE_EMBEDDING_API_KEY");
        }
        unsafe {
            std::env::remove_var("UTEKE_EMBEDDING_MODEL");
        }
        unsafe {
            std::env::remove_var("UTEKE_EMBEDDING_DIMS");
        }
        assert_eq!(cfg.embedding.backend, "openai");
        assert_eq!(cfg.embedding.api_key, "sk-env-test");
        assert_eq!(cfg.embedding.model, "text-embedding-3-small");
        assert_eq!(cfg.embedding.dims, 1536);
    }

    #[test]
    #[serial_test::serial]
    fn env_openai_api_key_fallback() {
        unsafe {
            std::env::remove_var("UTEKE_EMBEDDING_API_KEY");
        }
        unsafe {
            std::env::set_var("OPENAI_API_KEY", "sk-fallback");
        }
        let cfg = Config::default().apply_env_overrides();
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
        assert_eq!(cfg.embedding.api_key, "sk-fallback");
    }

    #[test]
    #[serial_test::serial]
    fn env_uteke_api_key_wins_over_openai() {
        unsafe {
            std::env::set_var("UTEKE_EMBEDDING_API_KEY", "sk-uteke");
        }
        unsafe {
            std::env::set_var("OPENAI_API_KEY", "sk-openai");
        }
        let cfg = Config::default().apply_env_overrides();
        unsafe {
            std::env::remove_var("UTEKE_EMBEDDING_API_KEY");
        }
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
        assert_eq!(cfg.embedding.api_key, "sk-uteke");
    }

    #[test]
    #[serial_test::serial]
    fn env_invalid_dims_ignored() {
        unsafe {
            std::env::set_var("UTEKE_EMBEDDING_DIMS", "not-a-number");
        }
        let cfg = Config::default().apply_env_overrides();
        unsafe {
            std::env::remove_var("UTEKE_EMBEDDING_DIMS");
        }
        assert_eq!(cfg.embedding.dims, 0, "invalid dims should keep default 0");
    }

    #[test]
    fn migrate_content_with_model_key() {
        let old = r#"store_path = "/data/mem"
model = "gemma-q4"
max_seq_length = 128
"#;
        let migrated = migrate_content(old);
        assert!(migrated.contains("[store]"));
        assert!(migrated.contains("[embedding]"));
        assert!(migrated.contains("model = \"gemma-q4\""));
        assert!(migrated.contains("max_seq_length = 128"));
    }

    #[test]
    fn expand_tilde_no_home() {
        // Just verify it doesn't panic
        let _ = Config::expand_tilde("/absolute/path");
    }

    #[test]
    #[serial_test::serial]
    fn env_override_log_level() {
        unsafe {
            std::env::set_var("UTEKE_LOG_LEVEL", "debug");
        }
        let cfg = Config::default().apply_env_overrides();
        unsafe {
            std::env::remove_var("UTEKE_LOG_LEVEL");
        }
        assert_eq!(cfg.logging.level, "debug");
    }

    #[test]
    #[serial_test::serial]
    fn env_override_server() {
        unsafe {
            std::env::set_var("UTEKE_SERVER_HOST", "0.0.0.0");
        }
        unsafe {
            std::env::set_var("UTEKE_SERVER_PORT", "9999");
        }
        let cfg = Config::default().apply_env_overrides();
        unsafe {
            std::env::remove_var("UTEKE_SERVER_HOST");
        }
        unsafe {
            std::env::remove_var("UTEKE_SERVER_PORT");
        }
        assert_eq!(cfg.server.host, "0.0.0.0");
        assert_eq!(cfg.server.port, 9999);
    }

    #[test]
    #[serial_test::serial]
    fn env_override_recall() {
        unsafe {
            std::env::set_var("UTEKE_RECALL_MIN_SCORE", "0.7");
        }
        unsafe {
            std::env::set_var("UTEKE_RECALL_MIN_SCORE_STRICT", "0.85");
        }
        let cfg = Config::default().apply_env_overrides();
        unsafe {
            std::env::remove_var("UTEKE_RECALL_MIN_SCORE");
        }
        unsafe {
            std::env::remove_var("UTEKE_RECALL_MIN_SCORE_STRICT");
        }
        assert!((cfg.recall.min_score - 0.7).abs() < f64::EPSILON);
        assert!((cfg.recall.min_score_strict - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    #[serial_test::serial]
    fn env_override_invalid_port_ignored() {
        unsafe {
            std::env::set_var("UTEKE_SERVER_PORT", "not-a-number");
        }
        let cfg = Config::default().apply_env_overrides();
        unsafe {
            std::env::remove_var("UTEKE_SERVER_PORT");
        }
        // Invalid value should be ignored — keeps default
        assert_eq!(cfg.server.port, 8767);
    }

    #[test]
    #[serial_test::serial]
    fn env_override_no_vars_uses_defaults() {
        // Ensure no env vars are set
        unsafe {
            std::env::remove_var("UTEKE_LOG_LEVEL");
        }
        unsafe {
            std::env::remove_var("UTEKE_SERVER_HOST");
        }
        unsafe {
            std::env::remove_var("UTEKE_SERVER_PORT");
        }
        unsafe {
            std::env::remove_var("UTEKE_RECALL_MIN_SCORE");
        }
        let cfg = Config::default().apply_env_overrides();
        assert_eq!(cfg.logging.level, "warn");
        assert_eq!(cfg.server.host, "127.0.0.1");
        assert_eq!(cfg.server.port, 8767);
        assert!((cfg.recall.min_score - 0.0).abs() < f64::EPSILON);
    }

    // ── #1078 P0 batch 3: score range guards, strategy validation, graph
    // weights, embed fallback — none previously covered. ─────────────────

    #[test]
    #[serial_test::serial]
    fn env_override_min_score_out_of_range_ignored() {
        unsafe {
            std::env::set_var("UTEKE_RECALL_MIN_SCORE", "1.5");
        }
        let cfg = Config::default().apply_env_overrides();
        unsafe {
            std::env::remove_var("UTEKE_RECALL_MIN_SCORE");
        }
        assert!((cfg.recall.min_score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    #[serial_test::serial]
    fn env_override_min_score_not_a_number_ignored() {
        unsafe {
            std::env::set_var("UTEKE_RECALL_MIN_SCORE", "banana");
        }
        let cfg = Config::default().apply_env_overrides();
        unsafe {
            std::env::remove_var("UTEKE_RECALL_MIN_SCORE");
        }
        assert!((cfg.recall.min_score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    #[serial_test::serial]
    fn env_override_strategy_valid_values_accepted() {
        for v in ["vector", "fts5", "hybrid", "graph", "fusion"] {
            unsafe {
                std::env::set_var("UTEKE_RECALL_STRATEGY", v);
            }
            let cfg = Config::default().apply_env_overrides();
            assert_eq!(cfg.recall.default_strategy, v, "strategy {v} rejected");
        }
        unsafe {
            std::env::remove_var("UTEKE_RECALL_STRATEGY");
        }
    }

    #[test]
    #[serial_test::serial]
    fn env_override_strategy_invalid_ignored() {
        unsafe {
            std::env::set_var("UTEKE_RECALL_STRATEGY", "quantum");
        }
        let cfg = Config::default().apply_env_overrides();
        unsafe {
            std::env::remove_var("UTEKE_RECALL_STRATEGY");
        }
        // Invalid strategy keeps the fusion default
        assert_eq!(cfg.recall.default_strategy, "fusion");
    }

    #[test]
    #[serial_test::serial]
    fn env_override_graph_weights_valid() {
        unsafe {
            std::env::set_var("UTEKE_GRAPH_DENSITY_WEIGHT", "0.5");
            std::env::set_var("UTEKE_GRAPH_AUTHORITY_WEIGHT", "0.7");
        }
        let cfg = Config::default().apply_env_overrides();
        unsafe {
            std::env::remove_var("UTEKE_GRAPH_DENSITY_WEIGHT");
            std::env::remove_var("UTEKE_GRAPH_AUTHORITY_WEIGHT");
        }
        assert!((cfg.recall.graph_density_weight - 0.5).abs() < f32::EPSILON);
        assert!((cfg.recall.graph_authority_weight - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    #[serial_test::serial]
    fn env_override_graph_weights_out_of_range_ignored() {
        unsafe {
            std::env::set_var("UTEKE_GRAPH_DENSITY_WEIGHT", "-1.0");
            std::env::set_var("UTEKE_GRAPH_AUTHORITY_WEIGHT", "2.0");
        }
        let cfg = Config::default().apply_env_overrides();
        unsafe {
            std::env::remove_var("UTEKE_GRAPH_DENSITY_WEIGHT");
            std::env::remove_var("UTEKE_GRAPH_AUTHORITY_WEIGHT");
        }
        assert!((cfg.recall.graph_density_weight - 0.1).abs() < f32::EPSILON);
        assert!((cfg.recall.graph_authority_weight - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    #[serial_test::serial]
    fn env_override_embed_fallback_all_fields() {
        unsafe {
            std::env::set_var("UTEKE_EMBED_FALLBACK_API_KEY", "fb-key");
            std::env::set_var("UTEKE_EMBED_FALLBACK_BASE_URL", "https://fb.example");
            std::env::set_var("UTEKE_EMBED_FALLBACK_ENDPOINT_PATH", "/v1/e");
            std::env::set_var("UTEKE_EMBED_FALLBACK_MODEL", "fb-model");
        }
        let cfg = Config::default().apply_env_overrides();
        unsafe {
            std::env::remove_var("UTEKE_EMBED_FALLBACK_API_KEY");
            std::env::remove_var("UTEKE_EMBED_FALLBACK_BASE_URL");
            std::env::remove_var("UTEKE_EMBED_FALLBACK_ENDPOINT_PATH");
            std::env::remove_var("UTEKE_EMBED_FALLBACK_MODEL");
        }
        assert_eq!(cfg.embed_fallback.api_key, "fb-key");
        assert_eq!(cfg.embed_fallback.base_url, "https://fb.example");
        assert_eq!(cfg.embed_fallback.endpoint_path, "/v1/e");
        assert_eq!(cfg.embed_fallback.model, "fb-model");
        assert!(cfg.embed_fallback.is_configured());
    }

    #[test]
    fn embed_fallback_is_configured_matrix() {
        // Nothing set — not configured
        let none = EmbedFallbackConfig {
            api_key: String::new(),
            base_url: String::new(),
            endpoint_path: String::new(),
            model: String::new(),
        };
        assert!(!none.is_configured());

        // Partial (2 of 3 required) — still not configured
        let partial = EmbedFallbackConfig {
            api_key: "k".into(),
            base_url: "u".into(),
            endpoint_path: String::new(),
            model: String::new(),
        };
        assert!(!partial.is_configured());

        // All three required set — configured (endpoint_path optional)
        let full = EmbedFallbackConfig {
            api_key: "k".into(),
            base_url: "u".into(),
            endpoint_path: String::new(),
            model: "m".into(),
        };
        assert!(full.is_configured());
    }

    // ── #1078 P0 batch 5: mutation survivors — migrate_content,
    // global_config_path, set_namespace_in_toml. Previously untested.

    #[test]
    fn migrate_content_moves_embedding_keys_to_section() {
        let old = "store_path = ~/.uteke\nmodel = gemma\nmax_seq_length = 512\n";
        let out = migrate_content(old);
        assert!(
            out.contains("[store]\npath = ~/.uteke"),
            "store path: {out}"
        );
        assert!(
            out.contains("[embedding]\nmodel = gemma\nmax_seq_length = 512"),
            "embedding keys must move to [embedding]: {out}"
        );
    }

    #[test]
    fn migrate_content_passes_through_unknown_and_sections() {
        let old = "# comment\nunknown_key = 1\n[new_section]\nfoo = bar\n";
        let out = migrate_content(old);
        assert!(out.contains("# comment"));
        assert!(out.contains("unknown_key = 1"));
        assert!(out.contains("[new_section]\nfoo = bar"));
        assert!(!out.contains("[store]"), "no store keys: {out}");
        assert!(!out.contains("[embedding]"), "no embedding keys: {out}");
    }

    #[test]
    #[serial_test::serial]
    fn global_config_path_respects_uteke_home() {
        unsafe {
            std::env::set_var("UTEKE_HOME", "/tmp/uteke-home-test");
        }
        let p = global_config_path().expect("UTEKE_HOME set → Some");
        assert_eq!(p, PathBuf::from("/tmp/uteke-home-test/uteke.toml"));
        unsafe {
            std::env::remove_var("UTEKE_HOME");
        }
    }

    #[test]
    fn set_namespace_rewrites_only_store_section() {
        // namespace in [other] must NOT be rewritten (guards the != and &&
        // mutants: section-boundary tracking).
        let content = "[store]\nnamespace = \"old\"\npath = p\n[other]\nnamespace = \"keep\"\n";
        let out = set_namespace_in_toml(content, "new");
        assert!(out.contains("namespace = \"new\""));
        assert!(
            out.contains("namespace = \"keep\""),
            "[other] namespace untouched: {out}"
        );
        assert!(out.contains("path = p"), "other keys preserved: {out}");
    }

    #[test]
    fn set_namespace_inserts_immediately_after_store_header() {
        // namespace must land right after [store], not before it (guards
        // the pos+1 insert-position mutant).
        let content = "[store]\npath = p\n";
        let out = set_namespace_in_toml(content, "ns1");
        let lines: Vec<&str> = out.lines().collect();
        let pos = lines.iter().position(|l| *l == "[store]").unwrap();
        assert_eq!(
            lines[pos + 1],
            "namespace = \"ns1\"",
            "insert after header: {out}"
        );
        assert!(out.contains("path = p"), "existing keys kept: {out}");
    }

    #[test]
    fn set_namespace_appends_store_section_when_missing() {
        let out = set_namespace_in_toml("top = 1\n", "ns2");
        assert!(
            out.contains("[store]\nnamespace = \"ns2\""),
            "appended [store]: {out}"
        );
        assert!(out.contains("top = 1"));
    }

    #[test]
    fn set_namespace_replaces_existing_value_in_place() {
        let out = set_namespace_in_toml("[store]\nnamespace = \"a\"\n", "b");
        assert!(out.contains("namespace = \"b\""));
        assert!(!out.contains("\"a\""), "old value gone: {out}");
    }
}
