//! One open-and-configure sequence for every binary (#1754).
//!
//! The CLI, the server and MCP each wrote "resolve the config, open the store,
//! apply the settings" by hand, and the copies drifted: a different order of
//! `apply_settings` and `set_privacy_config`, a different `RecallConfig` handed
//! to the constructor, env overrides applied (and sanitized) in more than one
//! place, and the `[recall]` defaults reaching the recall path by two routes.
//!
//! [`OpenConfig`] is that sequence, once:
//!
//! 1. **Resolve** the sections that decide how a store is opened and tuned
//!    (`[tier]`, `[recall]`, `[dream]`, `[lifecycle]`, `[embed_fallback]`,
//!    `[capture]`, `[rooms]`, `[doc_similarity]`, `[embedding]`, `[vector]`,
//!    `[privacy]`) from the layered files ([`crate::config_layers`]: precedence
//!    and trust) and the environment (`UTEKE_RECALL_*`, `UTEKE_GRAPH_*`,
//!    `UTEKE_EMBED_FALLBACK_*`, `UTEKE_EMBEDDING_*`, `UTEKE_PRIVACY_MODE`).
//!    Each section family is read on its own, so a type error in one never
//!    discards the others.
//! 2. **Open** the store with the open-time choices (embedder, vector engine,
//!    tier, graph rerank weights, the engine-level similarity floor).
//! 3. **Apply** what can change after opening ([`Uteke::apply_settings`]), then
//!    set the privacy filter. The two touch disjoint state, so the order has no
//!    observable effect; it is fixed (settings, then privacy) so it stays one
//!    line.
//!
//! The `[recall]` defaults a request falls back to are read from the opened
//! store ([`Uteke::recall_settings`]) by every transport.
//!
//! What stays in each binary: the server's bind address, tokens and CORS, the
//! CLI's paths and flags.

use crate::config_layers::{Layers, resolve};
use crate::embedding_config::{ResolvedEmbedding, StoreSettings};
use crate::privacy::{PrivacyConfig, PrivacyFile};
use crate::{EngineSettings, Error, Uteke};
use std::path::Path;

/// Everything needed to open a store and configure it, resolved.
#[derive(Clone)]
pub struct OpenConfig {
    /// The engine sections with the environment on top.
    pub engine: EngineSettings,
    /// The embedder the store opens with (`[embedding]` + `UTEKE_EMBEDDING_*`).
    pub embedding: ResolvedEmbedding,
    /// `[vector] backend` (empty = the compiled-in default). `UTEKE_VECTOR_BACKEND`
    /// wins over it and is read when the store is opened.
    pub vector_backend: String,
    /// The `[privacy]` filter with `UTEKE_PRIVACY_MODE` on top.
    pub privacy: PrivacyConfig,
}

impl OpenConfig {
    /// Resolve from the standard layers (`{uteke_home}/uteke.toml`, then the
    /// project file, untrusted unless `UTEKE_TRUST_PROJECT_CONFIG`) and the
    /// process environment. `Err` is an actionable message (an unsupported
    /// embedding backend), for the caller to print and exit on.
    pub fn load() -> Result<Self, String> {
        crate::config_layers::with_standard_layers(|layers| {
            Self::from_layers(layers, |k| std::env::var(k).ok())
        })
    }

    /// Resolve from explicit layers and an injectable environment (tests).
    pub fn from_layers(
        layers: &Layers<'_>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let engine = resolve::<EngineSettings>(layers).value;
        let store = resolve::<StoreSettings>(layers).value;
        let privacy = resolve::<PrivacyFile>(layers).value.privacy;
        let embedding = store.embedding.resolve_from(&env)?;
        Ok(Self {
            engine: engine.with_env_overrides_from(&env),
            embedding,
            vector_backend: store.vector.backend,
            privacy: crate::privacy::config_with_env(privacy, &env),
        })
    }

    /// Open the store at `path` and configure it: open-time choices through the
    /// constructor, then [`Uteke::apply_settings`], then the privacy filter.
    pub fn open(&self, path: impl AsRef<Path>) -> Result<Uteke, Error> {
        let mut uteke = Uteke::open_with_embedding_and_graph(
            path,
            &self.embedding.backend,
            self.embedding.settings.clone(),
            self.engine.tier,
            self.engine.recall.recall_config(),
            self.engine.recall.graph_rerank(),
            // UTEKE_VECTOR_BACKEND (higher precedence) is read inside the core.
            Some(self.vector_backend.as_str()),
        )?;
        uteke.apply_settings(&self.engine);
        uteke.set_privacy_config(self.privacy.clone());
        Ok(uteke)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::PrivacyMode;
    use std::path::PathBuf;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn resolve_toml(
        global: &str,
        project: &str,
        trust: bool,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<OpenConfig, String> {
        let dir = tempfile::tempdir().unwrap();
        let g = write(dir.path(), "g.toml", global);
        let p = write(dir.path(), "p.toml", project);
        OpenConfig::from_layers(
            &Layers {
                global: Some(&g),
                project: Some(&p),
                trust_project: trust,
            },
            env,
        )
    }

    #[test]
    fn defaults_when_nothing_is_configured() {
        let c = resolve_toml("", "", false, no_env).unwrap();
        assert_eq!(c.embedding.backend, "onnx");
        assert_eq!(c.vector_backend, "");
        assert_eq!(c.privacy.mode, PrivacyMode::default());
        assert_eq!(
            (c.engine.recall.min_score, c.engine.recall.min_score_strict),
            (0.0, 0.5)
        );
        assert!(!c.engine.embed_fallback.is_configured());
    }

    #[test]
    fn the_project_file_wins_per_key_over_the_global_one() {
        let c = resolve_toml(
            "[recall]\nmin_score = 0.2\ndefault_strategy = \"vector\"\n[tier]\nhot_days = 3\n",
            "[recall]\nmin_score = 0.4\n",
            false,
            no_env,
        )
        .unwrap();
        assert_eq!(c.engine.recall.min_score, 0.4, "project overrides global");
        assert_eq!(
            c.engine.recall.default_strategy, "vector",
            "a key the project omits keeps the global value"
        );
        assert_eq!(c.engine.tier.hot_days, 3);
    }

    #[test]
    fn an_untrusted_project_file_cannot_set_credentials_privacy_or_the_embedder() {
        let project = "[embedding]\nbackend = \"openai\"\nbase_url = \"https://evil.example\"\n\
                       model = \"m\"\n[embed_fallback]\napi_key = \"k\"\n\
                       base_url = \"https://evil.example\"\nmodel = \"m\"\n\
                       [privacy]\nmode = \"off\"\n[recall]\nmin_score = 0.3\n";
        let c = resolve_toml("[privacy]\nmode = \"strict\"\n", project, false, no_env).unwrap();
        assert_eq!(c.embedding.backend, "onnx");
        assert!(c.embedding.settings.base_url.is_empty());
        assert_eq!(
            c.embedding.settings.model, "m",
            "a model name is not a credential"
        );
        assert!(!c.engine.embed_fallback.is_configured());
        assert_eq!(c.privacy.mode, PrivacyMode::Strict);
        assert_eq!(
            c.engine.recall.min_score, 0.3,
            "tuning is allowed per project"
        );

        let trusted = resolve_toml("", project, true, no_env).unwrap();
        assert_eq!(trusted.embedding.backend, "openai");
        assert!(trusted.engine.embed_fallback.is_configured());
        assert_eq!(trusted.privacy.mode, PrivacyMode::Off);
    }

    #[test]
    fn the_environment_wins_over_the_files() {
        let env = |k: &str| {
            Some(
                match k {
                    "UTEKE_RECALL_MIN_SCORE" => "0.6",
                    "UTEKE_GRAPH_DENSITY_WEIGHT" => "0.25",
                    "UTEKE_EMBED_FALLBACK_API_KEY" => "env-key",
                    "UTEKE_EMBEDDING_BACKEND" => "ollama",
                    "UTEKE_PRIVACY_MODE" => "off",
                    _ => return None,
                }
                .to_string(),
            )
        };
        let c = resolve_toml(
            "[recall]\nmin_score = 0.2\n[embed_fallback]\nbase_url = \"https://file\"\nmodel = \"m\"\n\
             [embedding]\nbackend = \"onnx\"\n[privacy]\nmode = \"strict\"\n",
            "",
            false,
            env,
        )
        .unwrap();
        assert_eq!(c.engine.recall.min_score, 0.6);
        assert_eq!(c.engine.recall.graph_rerank().density_weight, 0.25);
        assert!(
            c.engine.embed_fallback.is_configured(),
            "env completes the file"
        );
        assert_eq!(c.engine.embed_fallback.api_key, "env-key");
        assert_eq!(c.embedding.backend, "ollama");
        assert_eq!(c.privacy.mode, PrivacyMode::Off);
    }

    #[test]
    fn an_unsupported_backend_is_an_actionable_error() {
        let err = resolve_toml("[embedding]\nbackend = \"cohere\"\n", "", false, no_env)
            .err()
            .expect("must fail");
        assert!(
            err.contains("cohere") && err.contains("UTEKE_EMBEDDING_BACKEND"),
            "{err}"
        );
    }

    #[test]
    fn a_type_error_in_one_family_does_not_discard_the_others() {
        let c = resolve_toml(
            "[privacy]\nmode = 5\n[recall]\nmin_score = 0.3\n[vector]\nbackend = \"vecq\"\n",
            "",
            false,
            no_env,
        )
        .unwrap();
        assert_eq!(c.engine.recall.min_score, 0.3);
        assert_eq!(c.vector_backend, "vecq");
        assert_eq!(c.privacy.mode, PrivacyMode::default());
    }

    #[test]
    fn open_applies_the_settings_and_the_privacy_filter() {
        let c = resolve_toml(
            "[recall]\nmin_score = 0.25\ndefault_strategy = \"typo\"\n[privacy]\nmode = \"strict\"\n",
            "",
            false,
            no_env,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let u = c.open(dir.path().join("uteke.db")).unwrap();
        assert_eq!(u.recall_settings().min_score, 0.25);
        assert_eq!(
            u.recall_settings().default_strategy,
            "hybrid",
            "an unknown strategy is replaced once, for every transport"
        );
        assert_eq!(u.privacy_config().mode, PrivacyMode::Strict);
    }

    #[test]
    fn apply_settings_and_privacy_do_not_depend_on_their_order() {
        // The server used to set privacy before applying the settings, the CLI
        // and MCP after. The two touch disjoint state.
        let c = resolve_toml(
            "[recall]\nmin_score = 0.25\n[privacy]\nmode = \"strict\"\n",
            "",
            false,
            no_env,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut u = c.open(dir.path().join("a.db")).unwrap();
        // privacy first, then the settings
        u.set_privacy_config(c.privacy.clone());
        u.apply_settings(&c.engine);
        assert_eq!(u.privacy_config().mode, PrivacyMode::Strict);
        assert_eq!(u.recall_settings().min_score, 0.25);
        // settings first, then privacy
        u.apply_settings(&c.engine);
        u.set_privacy_config(c.privacy.clone());
        assert_eq!(u.privacy_config().mode, PrivacyMode::Strict);
        assert_eq!(u.recall_settings().min_score, 0.25);
    }
}
