//! The `[recall]` section of `uteke.toml` (#1515): one schema, one set of
//! defaults and one set of `UTEKE_*` overrides for the CLI and the server.

use crate::{GraphRerankConfig, RecallConfig};

/// Recall / search tuning read from `[recall]`. Every key is optional; an
/// omitted key keeps its default.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct RecallSettings {
    /// Minimum similarity score for recall results. Results below are filtered out.
    /// Default 0.0 (#1223): since fusion RRF became the default strategy (0.16.0),
    /// returned scores are rank-based (~0.0-0.2), so a legacy cosine-era default of
    /// 0.3 silently emptied default recalls. Threshold remains opt-in via config,
    /// `--min`, or `--strict` (0.5).
    pub min_score: f64,
    /// Strict mode threshold (higher, for critical queries).
    pub min_score_strict: f64,
    /// Default recall strategy when a request does not name one. One of
    /// `vector`, `fts5`, `hybrid`, `graph`, `fusion` (default: weighted RRF,
    /// the default since 0.16.0, #1123).
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

impl Default for RecallSettings {
    fn default() -> Self {
        Self {
            min_score: 0.0,
            min_score_strict: 0.5,
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

/// The strategy names `default_strategy` and `UTEKE_RECALL_STRATEGY` accept.
const STRATEGIES: &[&str] = &["vector", "fts5", "hybrid", "graph", "fusion"];

impl RecallSettings {
    /// The graph-rerank weights and flag.
    pub fn graph_rerank(&self) -> GraphRerankConfig {
        GraphRerankConfig {
            density_weight: self.graph_density_weight,
            authority_weight: self.graph_authority_weight,
            enabled: self.graph_rerank_enabled,
        }
    }

    /// The engine-level similarity floor (`min_score`).
    pub fn recall_config(&self) -> RecallConfig {
        RecallConfig {
            min_score: self.min_score as f32,
        }
    }

    /// Apply the `UTEKE_RECALL_*` and `UTEKE_GRAPH_*` overrides from `lookup`.
    /// A value that is out of range or does not parse is logged and ignored.
    pub fn with_overrides_from(mut self, lookup: impl Fn(&str) -> Option<String>) -> Self {
        // A number from 0.0 to 1.0, as `f32` or `f64` (the key decides).
        fn unit<T>(name: &str, lookup: &impl Fn(&str) -> Option<String>) -> Option<T>
        where
            T: std::str::FromStr + PartialOrd + From<u8>,
        {
            let v = lookup(name)?;
            match v.parse::<T>() {
                Ok(x) if (T::from(0)..=T::from(1)).contains(&x) => Some(x),
                Ok(_) => {
                    tracing::warn!("{name}='{v}' out of range, ignoring (expected 0.0-1.0)");
                    None
                }
                Err(_) => {
                    tracing::warn!("Invalid {name}='{v}', ignoring (expected 0.0-1.0)");
                    None
                }
            }
        }

        if let Some(x) = unit::<f64>("UTEKE_RECALL_MIN_SCORE", &lookup) {
            self.min_score = x;
        }
        if let Some(x) = unit::<f64>("UTEKE_RECALL_MIN_SCORE_STRICT", &lookup) {
            self.min_score_strict = x;
        }
        if let Some(v) = lookup("UTEKE_RECALL_STRATEGY") {
            if STRATEGIES.contains(&v.as_str()) {
                self.default_strategy = v;
            } else {
                tracing::warn!(
                    "Invalid UTEKE_RECALL_STRATEGY='{v}', ignoring (expected vector|fts5|hybrid|graph|fusion)"
                );
            }
        }
        if let Some(x) = unit::<f32>("UTEKE_GRAPH_DENSITY_WEIGHT", &lookup) {
            self.graph_density_weight = x;
        }
        if let Some(x) = unit::<f32>("UTEKE_GRAPH_AUTHORITY_WEIGHT", &lookup) {
            self.graph_authority_weight = x;
        }
        if let Some(v) = lookup("UTEKE_GRAPH_RERANK_ENABLED") {
            match v.to_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => self.graph_rerank_enabled = true,
                "0" | "false" | "no" | "off" => self.graph_rerank_enabled = false,
                _ => tracing::warn!(
                    "Invalid UTEKE_GRAPH_RERANK_ENABLED='{v}', ignoring (expected true/false)"
                ),
            }
        }
        self
    }

    /// [`with_overrides_from`](Self::with_overrides_from) over the process
    /// environment.
    pub fn with_env_overrides(self) -> Self {
        self.with_overrides_from(|k| std::env::var(k).ok())
    }
}
