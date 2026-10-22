//! The engine sections of `uteke.toml` as one value (#1515): what the CLI, the
//! server and MCP read once and apply to a `Uteke` the same way.

use crate::{DreamConfig, FallbackSettings, LifecycleConfig, RecallSettings, TierConfig, Uteke};

/// `[tier]`, `[recall]`, `[dream]`, `[lifecycle]` and `[embed_fallback]`. Every
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
}

impl EngineSettings {
    /// Read the five sections the way every binary does: the global
    /// `uteke.toml`, then the project file (its untrusted keys stripped unless
    /// `UTEKE_TRUST_PROJECT_CONFIG` is set), then the `UTEKE_RECALL_*`,
    /// `UTEKE_GRAPH_*` and `UTEKE_EMBED_FALLBACK_*` environment variables.
    /// An unreadable or invalid file is skipped with a warning (defaults).
    pub fn load() -> Self {
        let mut engine = crate::config_layers::resolve_standard::<EngineSettings>().value;
        engine.recall = engine.recall.with_env_overrides();
        engine.embed_fallback = engine.embed_fallback.with_env_overrides();
        engine
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
        if settings.embed_fallback.is_configured() {
            self.set_fallback_settings(settings.embed_fallback.clone());
        }
    }
}
