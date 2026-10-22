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

impl Uteke {
    /// Apply what can change after the store is open: the Jaccard weight, the
    /// dream thresholds, the lifecycle config and, when it is complete (api
    /// key, base URL and model), the embedding fallback. `tier` and the
    /// recall thresholds that the constructor takes are not touched here.
    pub fn apply_settings(&mut self, settings: &EngineSettings) {
        self.set_jaccard_weight(settings.recall.jaccard_weight);
        self.set_dream_config(settings.dream);
        self.set_lifecycle_config(settings.lifecycle);
        if settings.embed_fallback.is_configured() {
            self.set_fallback_settings(settings.embed_fallback.clone());
        }
    }
}
