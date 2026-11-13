//! The engine sections of `uteke.toml` as one value (#1515): what the CLI, the
//! server and MCP read once and apply to a `Uteke` the same way.

use crate::{DreamConfig, FallbackSettings, LifecycleConfig, RecallSettings, TierConfig, Uteke};

/// `[tier]`, `[recall]`, `[dream]`, `[lifecycle]`, `[embed_fallback]` and
/// `[doc_similarity]` (and the other sections below). Every
/// section and every key is optional: what a file omits keeps the core
/// default. Embed it in a larger config with `#[serde(flatten)]`.
#[derive(Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct EngineSettings {
    /// Hot/warm/cold tiers. Open-time: pass it to the constructor.
    pub tier: TierConfig,
    pub recall: RecallSettings,
    pub dream: DreamConfig,
    pub lifecycle: LifecycleConfig,
    pub embed_fallback: FallbackSettings,
    /// `[capture]` (#1466). Global file only: listed in
    /// `UNTRUSTED_PROJECT_KEYS`, so a cloned repository cannot turn it on.
    pub capture: crate::capture::CaptureSettings,
    /// `[rooms]` (#1715): whether a new room id must be `category:title`.
    /// Global file only (`UNTRUSTED_PROJECT_KEYS`): a cloned repository cannot
    /// loosen it.
    pub rooms: crate::room_naming::RoomNamingSettings,
    /// `[doc_similarity]` (#1730): thresholds of document similarity.
    pub doc_similarity: crate::doc_similarity::DocSimilaritySettings,
}

impl EngineSettings {
    /// Read the five sections the way every binary does: the global
    /// `uteke.toml`, then the project file (its untrusted keys stripped unless
    /// `UTEKE_TRUST_PROJECT_CONFIG` is set), then the `UTEKE_RECALL_*`,
    /// `UTEKE_GRAPH_*` and `UTEKE_EMBED_FALLBACK_*` environment variables.
    /// An unreadable or invalid file is skipped with a warning (defaults).
    pub fn load() -> Self {
        crate::config_layers::resolve_standard::<EngineSettings>()
            .value
            .with_env_overrides()
    }

    /// The `UTEKE_RECALL_*`, `UTEKE_GRAPH_*` and `UTEKE_EMBED_FALLBACK_*`
    /// overrides from `lookup` on top of the file values: the one place they
    /// are applied. Setting a value is idempotent, so applying it twice is safe.
    pub fn with_env_overrides_from(mut self, lookup: impl Fn(&str) -> Option<String>) -> Self {
        self.recall = self.recall.with_overrides_from(&lookup);
        self.embed_fallback = self.embed_fallback.with_overrides_from(&lookup);
        self
    }

    /// [`with_env_overrides_from`](Self::with_env_overrides_from) over the
    /// process environment.
    pub fn with_env_overrides(self) -> Self {
        self.with_env_overrides_from(|k| std::env::var(k).ok())
    }
}

impl Uteke {
    /// The `[recall]` settings in force: what a request falls back to when it
    /// gives no similarity floor and no strategy.
    pub fn recall_settings(&self) -> &RecallSettings {
        &self.recall_settings
    }

    /// Apply what can change after the store is open: the tier and graph-rerank
    /// settings, the Jaccard weight, the recall defaults, the dream thresholds,
    /// the lifecycle config and, when it is complete (api key, base URL and
    /// model), the embedding fallback. The embedding backend and the vector
    /// engine are choices of the constructor and are not touched here.
    pub fn apply_settings(&mut self, settings: &EngineSettings) {
        self.tier_config = settings.tier;
        self.graph_rerank_config = settings.recall.graph_rerank().sanitized();
        self.recall_settings = settings.recall.clone().sanitized();
        self.set_jaccard_weight(settings.recall.jaccard_weight);
        self.set_dream_config(settings.dream);
        self.set_lifecycle_config(settings.lifecycle);
        self.set_capture_settings(settings.capture);
        self.set_room_naming_settings(settings.rooms);
        self.set_doc_similarity_settings(settings.doc_similarity);
        if settings.embed_fallback.is_configured() {
            self.set_fallback_settings(settings.embed_fallback.clone());
        }
    }
}
