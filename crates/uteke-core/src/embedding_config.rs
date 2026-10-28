//! The `[embedding]` section of `uteke.toml` and its one resolution (#1537).
//!
//! The CLI and the server read the same section and used to resolve it in two
//! copies (backend list, validation, env precedence, default model) that
//! disagreed. [`EmbeddingSection::resolve`] is the single place that turns the
//! file section plus the `UTEKE_EMBEDDING_*` environment into the backend name
//! and the [`EmbeddingSettings`] the store opens with.

use crate::EmbeddingSettings;

/// `[embedding]` in `uteke.toml`. Empty / zero means "not set": the backend
/// default applies (a model that is not set is the backend's default model).
#[derive(serde::Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct EmbeddingSection {
    /// `onnx` (default), `openai` or `ollama`.
    pub backend: String,
    pub model: String,
    pub api_key: String,
    pub base_url: String,
    pub endpoint_path: String,
    /// Output dimensions. 0 = the backend's default.
    pub dims: usize,
    /// Accepted so existing files keep loading; it has never had any effect.
    pub max_seq_length: Option<usize>,
}

/// `[vector]` in `uteke.toml`.
#[derive(serde::Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct VectorSection {
    /// `usearch` or `vecq`. Empty = the compiled-in default; `UTEKE_VECTOR_BACKEND`
    /// (read by the core when the store is opened) wins over it.
    pub backend: String,
}

/// The two sections that decide how a store is opened: `[embedding]` and
/// `[vector]`. What a binary without its own config type (MCP) reads.
#[derive(serde::Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct StoreSettings {
    pub embedding: EmbeddingSection,
    pub vector: VectorSection,
}

impl StoreSettings {
    /// Read both sections the way every binary does: the global `uteke.toml`,
    /// then the project file with its untrusted keys (the embedding backend and
    /// credentials, endpoints) stripped unless `UTEKE_TRUST_PROJECT_CONFIG` is
    /// set. An unreadable or invalid file is skipped with a warning.
    pub fn load() -> Self {
        crate::config_layers::resolve_standard::<StoreSettings>().value
    }
}

/// What a store is opened with: the backend name and its settings.
#[derive(Clone)]
pub struct ResolvedEmbedding {
    pub backend: String,
    pub settings: EmbeddingSettings,
}

impl EmbeddingSection {
    /// Backends the core can initialise.
    pub const SUPPORTED_BACKENDS: &'static [&'static str] = &["onnx", "openai", "ollama"];

    /// Resolve with the process environment.
    pub fn resolve(&self) -> Result<ResolvedEmbedding, String> {
        self.resolve_from(|name| std::env::var(name).ok())
    }

    /// [`Self::resolve`] with an injectable environment lookup (tests).
    ///
    /// Precedence: `UTEKE_EMBEDDING_BACKEND` (non-empty, trimmed) wins over the
    /// file's `backend`, which wins over `onnx`; an empty or blank value counts
    /// as unset. The other `UTEKE_EMBEDDING_*` variables are applied by
    /// [`EmbeddingSettings::with_overrides_from`]. An unsupported backend is an
    /// error with an actionable message (#1399).
    pub fn resolve_from(
        &self,
        get: impl Fn(&str) -> Option<String>,
    ) -> Result<ResolvedEmbedding, String> {
        let non_blank =
            |v: Option<String>| v.map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
        let backend = non_blank(get("UTEKE_EMBEDDING_BACKEND"))
            .or_else(|| non_blank(Some(self.backend.clone())))
            .unwrap_or_else(|| "onnx".to_string());
        if !Self::SUPPORTED_BACKENDS.contains(&backend.as_str()) {
            return Err(format!(
                "Unsupported embedding backend '{backend}'. Supported: {}. \
                 Set UTEKE_EMBEDDING_BACKEND or [embedding] backend in uteke.toml.",
                Self::SUPPORTED_BACKENDS.join(", ")
            ));
        }
        if self.max_seq_length.is_some() || get("UTEKE_MAX_SEQ_LENGTH").is_some() {
            tracing::warn!(
                "[embedding] max_seq_length / UTEKE_MAX_SEQ_LENGTH has no effect and is ignored"
            );
        }
        let settings = EmbeddingSettings {
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            endpoint_path: self.endpoint_path.clone(),
            model: self.model.clone(),
            dims: self.dims,
        }
        .with_overrides_from(get);
        Ok(ResolvedEmbedding { backend, settings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn a_project_file_cannot_choose_the_backend_through_store_settings() {
        use crate::config_layers::{Layers, resolve};
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("g.toml");
        let project = dir.path().join("p.toml");
        std::fs::write(
            &global,
            "[embedding]\nbackend = \"onnx\"\n[vector]\nbackend = \"usearch\"\n",
        )
        .unwrap();
        std::fs::write(
            &project,
            "[embedding]\nbackend = \"openai\"\nbase_url = \"https://evil.example\"\nmodel = \"m\"\n",
        )
        .unwrap();
        let layers = Layers {
            global: Some(&global),
            project: Some(&project),
            trust_project: false,
        };
        let s = resolve::<StoreSettings>(&layers).value;
        assert_eq!(s.embedding.backend, "onnx");
        assert_eq!(s.embedding.base_url, "");
        assert_eq!(s.embedding.model, "m", "a model name is not a credential");
        assert_eq!(s.vector.backend, "usearch");
    }

    #[test]
    fn store_settings_read_both_sections_and_default_when_absent() {
        let s: StoreSettings =
            toml::from_str("[embedding]\nbackend = \"ollama\"\n[vector]\nbackend = \"vecq\"\n")
                .unwrap();
        assert_eq!(s.embedding.backend, "ollama");
        assert_eq!(s.vector.backend, "vecq");
        let empty: StoreSettings = toml::from_str("").unwrap();
        assert_eq!(empty.embedding.backend, "");
        assert_eq!(empty.vector.backend, "");
    }

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn nothing_set_is_onnx_with_no_model() {
        let r = EmbeddingSection::default().resolve_from(env(&[])).unwrap();
        assert_eq!(r.backend, "onnx");
        assert_eq!(r.settings.model, "");
        assert_eq!(r.settings.dims, 0);
    }

    #[test]
    fn an_openai_section_without_a_model_leaves_the_model_to_the_backend() {
        let s = EmbeddingSection {
            backend: "openai".into(),
            api_key: "k".into(),
            base_url: "http://x/v1".into(),
            dims: 768,
            ..Default::default()
        };
        let r = s.resolve_from(env(&[])).unwrap();
        assert_eq!(r.backend, "openai");
        assert_eq!(r.settings.model, "", "the backend picks its own default");
        assert_eq!(r.settings.dims, 768);
    }

    #[test]
    fn the_environment_wins_and_an_empty_value_counts_as_unset() {
        let s = EmbeddingSection {
            backend: "ollama".into(),
            model: "file-model".into(),
            api_key: "file-key".into(),
            ..Default::default()
        };
        let r = s
            .resolve_from(env(&[
                ("UTEKE_EMBEDDING_BACKEND", " openai "),
                ("UTEKE_EMBEDDING_MODEL", "env-model"),
                ("UTEKE_EMBEDDING_API_KEY", ""),
                ("UTEKE_EMBEDDING_DIMS", "256"),
            ]))
            .unwrap();
        assert_eq!(r.backend, "openai", "trimmed, env over file");
        assert_eq!(r.settings.model, "env-model");
        assert_eq!(r.settings.api_key, "file-key", "an empty variable is unset");
        assert_eq!(r.settings.dims, 256);
    }

    #[test]
    fn openai_api_key_is_the_fallback_for_the_embedding_key() {
        let r = EmbeddingSection::default()
            .resolve_from(env(&[
                ("OPENAI_API_KEY", "o"),
                ("UTEKE_EMBEDDING_API_KEY", "u"),
            ]))
            .unwrap();
        assert_eq!(r.settings.api_key, "u");
        let r = EmbeddingSection::default()
            .resolve_from(env(&[("OPENAI_API_KEY", "o")]))
            .unwrap();
        assert_eq!(r.settings.api_key, "o");
    }

    #[test]
    fn a_blank_backend_is_unset_and_an_unknown_one_is_an_error() {
        let blank = EmbeddingSection {
            backend: "  ".into(),
            ..Default::default()
        };
        assert_eq!(blank.resolve_from(env(&[])).unwrap().backend, "onnx");
        let bad = EmbeddingSection {
            backend: "cohere".into(),
            ..Default::default()
        };
        let err = bad.resolve_from(env(&[])).err().unwrap();
        assert!(
            err.contains("cohere") && err.contains("onnx, openai, ollama"),
            "{err}"
        );
    }

    #[test]
    fn a_non_numeric_dims_is_ignored() {
        let s = EmbeddingSection {
            dims: 512,
            ..Default::default()
        };
        let r = s
            .resolve_from(env(&[("UTEKE_EMBEDDING_DIMS", "many")]))
            .unwrap();
        assert_eq!(r.settings.dims, 512);
    }
}
