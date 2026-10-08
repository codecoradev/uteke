//! Layered `uteke.toml` resolution — one policy shared by the CLI and the server.
//!
//! The same file used to be merged by two hand-written implementations with
//! different semantics (the CLI listed every key by hand; the server replaced
//! whole sections), and the "do not trust the project-local file" rule existed
//! twice. That produced real bugs: whole config sections silently ignored,
//! and a project file wiping the global `[server]` auth settings.
//!
//! This module owns the rules, independent of any schema:
//!
//! * **Precedence**: global file, then the project-local file on top.
//! * **Per-key merge**: tables merge key by key; any other value (including
//!   arrays) from the higher layer replaces the lower one. Keys a layer does
//!   not mention keep the lower layer's value.
//! * **Trust**: the project-local file lives in the working tree, which may be
//!   an untrusted clone, so [`UNTRUSTED_PROJECT_KEYS`] (credentials, endpoints,
//!   the embedding backend, the whole `[server]` section) are stripped from it
//!   *before* merging unless the caller explicitly trusts it.
//! * **Validation**: every layer must deserialize into the schema on its own;
//!   an unreadable or invalid layer is skipped with a warning instead of
//!   discarding the others.
//!
//! Callers choose the schema type `T` (`#[serde(default)]` everywhere, so a
//! missing key falls back to its default) and add their own environment
//! overrides on top of the result.

use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use toml::{Table, Value};

/// Dotted keys the project-local config may not set: they decide where
/// credentials or memory text are sent, or how the server authenticates.
/// A bare section name (`server`) covers every key in that section.
pub const UNTRUSTED_PROJECT_KEYS: &[&str] = &[
    "embedding.backend",
    "embedding.api_key",
    "embedding.base_url",
    "embedding.endpoint_path",
    "embed_fallback.api_key",
    "embed_fallback.base_url",
    "embed_fallback.endpoint_path",
    "extraction.api_key",
    "extraction.base_url",
    "extraction.endpoint_path",
    "server",
];

/// The files to resolve. Missing files are fine.
#[derive(Debug, Clone, Default)]
pub struct Layers<'a> {
    /// `{uteke_home}/uteke.toml`
    pub global: Option<&'a Path>,
    /// `$CWD/.uteke/uteke.toml`
    pub project: Option<&'a Path>,
    /// Allow the project file to set [`UNTRUSTED_PROJECT_KEYS`] (explicit opt-in).
    pub trust_project: bool,
}

/// What happened while resolving, for logging and tests.
#[derive(Debug)]
pub struct Resolved<T> {
    pub value: T,
    /// Keys dropped from the project file because it is untrusted.
    pub ignored_untrusted: Vec<String>,
    /// Layers skipped because they could not be read or did not match the schema.
    pub skipped: Vec<PathBuf>,
}

/// Merge `overlay` into `base`: tables recurse, everything else is replaced.
pub fn deep_merge(base: &mut Table, overlay: Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(base_table)), Value::Table(overlay_table)) => {
                deep_merge(base_table, overlay_table);
            }
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// Remove [`UNTRUSTED_PROJECT_KEYS`] from a project-local table. Returns the
/// dotted names that were actually present (and therefore dropped).
pub fn strip_untrusted(table: &mut Table) -> Vec<String> {
    let mut removed = Vec::new();
    for key in UNTRUSTED_PROJECT_KEYS {
        match key.split_once('.') {
            None => {
                if table.remove(*key).is_some() {
                    removed.push((*key).to_string());
                }
            }
            Some((section, leaf)) => {
                if let Some(Value::Table(section_table)) = table.get_mut(section) {
                    if section_table.remove(leaf).is_some() {
                        removed.push((*key).to_string());
                    }
                }
            }
        }
    }
    removed
}

fn read_layer(path: &Path) -> Option<Table> {
    if !path.exists() {
        return None;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Cannot read config {}: {e}", path.display());
            return None;
        }
    };
    match toml::from_str::<Table>(&content) {
        Ok(table) => Some(table),
        Err(e) => {
            tracing::warn!("Invalid config {}: {e}", path.display());
            None
        }
    }
}

/// Resolve the layers into a `T`.
pub fn resolve<T: DeserializeOwned + Default>(layers: &Layers<'_>) -> Resolved<T> {
    let mut merged = Table::new();
    let mut ignored_untrusted = Vec::new();
    let mut skipped = Vec::new();

    let ordered = [(layers.global, false), (layers.project, true)];
    for (path, is_project) in ordered {
        let Some(path) = path else { continue };
        let Some(mut table) = read_layer(path) else {
            if path.exists() {
                skipped.push(path.to_path_buf());
            }
            continue;
        };
        if is_project && !layers.trust_project {
            let removed = strip_untrusted(&mut table);
            if !removed.is_empty() {
                tracing::warn!(
                    "Ignoring {} from project config {} (untrusted: set them in the global \
                     config, or set UTEKE_TRUST_PROJECT_CONFIG=1 to allow)",
                    removed.join(", "),
                    path.display()
                );
                ignored_untrusted.extend(removed);
            }
        }
        // Each layer must stand on its own against the schema.
        if let Err(e) = Value::Table(table.clone()).try_into::<T>() {
            tracing::warn!("Invalid config {}: {e}", path.display());
            skipped.push(path.to_path_buf());
            continue;
        }
        deep_merge(&mut merged, table);
    }

    let value = Value::Table(merged).try_into::<T>().unwrap_or_else(|e| {
        tracing::warn!("Merged config does not match the schema ({e}); using defaults");
        T::default()
    });
    Resolved {
        value,
        ignored_untrusted,
        skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(default)]
    struct Schema {
        top: bool,
        store: Store,
        embedding: Embedding,
        server: Server,
        recall: Recall,
    }
    impl Default for Schema {
        fn default() -> Self {
            Self {
                top: true,
                store: Store::default(),
                embedding: Embedding::default(),
                server: Server::default(),
                recall: Recall::default(),
            }
        }
    }
    #[derive(Debug, Deserialize, PartialEq, Default)]
    #[serde(default)]
    struct Store {
        path: String,
        namespace: String,
    }
    #[derive(Debug, Deserialize, PartialEq, Default)]
    #[serde(default)]
    struct Embedding {
        backend: String,
        model: String,
        base_url: String,
        api_key: String,
    }
    #[derive(Debug, Deserialize, PartialEq, Default)]
    #[serde(default)]
    struct Server {
        host: String,
        auth_token: String,
        cors_origins: Vec<String>,
    }
    #[derive(Debug, Deserialize, PartialEq, Default)]
    #[serde(default)]
    struct Recall {
        min_score: f64,
        strategy: String,
    }

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    fn layers<'a>(g: Option<&'a Path>, p: Option<&'a Path>, trust: bool) -> Layers<'a> {
        Layers {
            global: g,
            project: p,
            trust_project: trust,
        }
    }

    #[test]
    fn deep_merge_recurses_into_tables_and_replaces_other_values() {
        let mut base: Table = toml::from_str(
            "a = 1\n[s]\nx = 1\ny = 2\nlist = [1, 2]\n[s.inner]\nk = \"base\"\nkeep = true",
        )
        .unwrap();
        let overlay: Table =
            toml::from_str("a = 9\n[s]\ny = 20\nlist = [3]\n[s.inner]\nk = \"over\"").unwrap();
        deep_merge(&mut base, overlay);
        let expected: Table = toml::from_str(
            "a = 9\n[s]\nx = 1\ny = 20\nlist = [3]\n[s.inner]\nk = \"over\"\nkeep = true",
        )
        .unwrap();
        assert_eq!(base, expected);
    }

    #[test]
    fn project_overrides_global_per_key() {
        let d = tempfile::tempdir().unwrap();
        let g = write(
            d.path(),
            "g.toml",
            "[store]\npath = \"/g\"\nnamespace = \"g\"\n[recall]\nmin_score = 0.3\nstrategy = \"vector\"",
        );
        let p = write(d.path(), "p.toml", "[recall]\nmin_score = 0.5");
        let r = resolve::<Schema>(&layers(Some(&g), Some(&p), false));
        assert_eq!(r.value.recall.min_score, 0.5, "project wins");
        assert_eq!(
            r.value.recall.strategy, "vector",
            "a project key that is absent keeps the global value"
        );
        assert_eq!(r.value.store.path, "/g");
        assert!(r.value.top, "untouched keys keep their schema default");
    }

    #[test]
    fn untrusted_project_keys_are_stripped_before_merging() {
        let d = tempfile::tempdir().unwrap();
        let g = write(
            d.path(),
            "g.toml",
            "[embedding]\nbackend = \"onnx\"\nbase_url = \"https://global.example\"\n[server]\nauth_token = \"global\"\nhost = \"0.0.0.0\"\ncors_origins = [\"https://app.example\"]",
        );
        let p = write(
            d.path(),
            "p.toml",
            "[embedding]\nbackend = \"openai\"\nbase_url = \"https://evil.example\"\napi_key = \"stolen\"\nmodel = \"tuned\"\n[server]\nauth_token = \"evil\"\nhost = \"evil.example\"",
        );
        let r = resolve::<Schema>(&layers(Some(&g), Some(&p), false));
        assert_eq!(r.value.embedding.backend, "onnx");
        assert_eq!(r.value.embedding.base_url, "https://global.example");
        assert_eq!(r.value.embedding.api_key, "");
        assert_eq!(
            r.value.embedding.model, "tuned",
            "harmless keys still apply"
        );
        assert_eq!(r.value.server.auth_token, "global");
        assert_eq!(r.value.server.host, "0.0.0.0");
        assert_eq!(r.value.server.cors_origins, vec!["https://app.example"]);
        for key in [
            "embedding.backend",
            "embedding.base_url",
            "embedding.api_key",
            "server",
        ] {
            assert!(r.ignored_untrusted.contains(&key.to_string()), "{key}");
        }
    }

    #[test]
    fn global_layer_may_set_sensitive_keys() {
        let d = tempfile::tempdir().unwrap();
        let g = write(
            d.path(),
            "g.toml",
            "[embedding]\napi_key = \"ok\"\n[server]\nhost = \"h\"",
        );
        let r = resolve::<Schema>(&layers(Some(&g), None, false));
        assert_eq!(r.value.embedding.api_key, "ok");
        assert_eq!(r.value.server.host, "h");
        assert!(r.ignored_untrusted.is_empty());
    }

    #[test]
    fn explicit_trust_lets_the_project_file_set_everything() {
        let d = tempfile::tempdir().unwrap();
        let p = write(
            d.path(),
            "p.toml",
            "[server]\nhost = \"proj\"\n[embedding]\nbackend = \"ollama\"",
        );
        let r = resolve::<Schema>(&layers(None, Some(&p), true));
        assert_eq!(r.value.server.host, "proj");
        assert_eq!(r.value.embedding.backend, "ollama");
        assert!(r.ignored_untrusted.is_empty());
    }

    #[test]
    fn a_project_file_without_server_section_never_wipes_the_global_one() {
        let d = tempfile::tempdir().unwrap();
        let g = write(d.path(), "g.toml", "[server]\nauth_token = \"t\"");
        let p = write(d.path(), "p.toml", "[recall]\nmin_score = 0.1");
        let r = resolve::<Schema>(&layers(Some(&g), Some(&p), false));
        assert_eq!(r.value.server.auth_token, "t");
    }

    #[test]
    fn invalid_layers_are_skipped_not_fatal() {
        let d = tempfile::tempdir().unwrap();
        let g = write(d.path(), "g.toml", "[recall]\nstrategy = \"fusion\"");
        let broken_toml = write(d.path(), "p1.toml", "this is not valid toml [[[[");
        let r = resolve::<Schema>(&layers(Some(&g), Some(&broken_toml), false));
        assert_eq!(r.value.recall.strategy, "fusion");
        assert_eq!(r.skipped, vec![broken_toml.clone()]);

        // Valid TOML, wrong type for a schema field.
        let wrong_type = write(d.path(), "p2.toml", "[recall]\nmin_score = \"high\"");
        let r = resolve::<Schema>(&layers(Some(&g), Some(&wrong_type), false));
        assert_eq!(
            r.value.recall.strategy, "fusion",
            "other layers still apply"
        );
        assert_eq!(r.value.recall.min_score, 0.0, "bad value did not leak in");
        assert_eq!(r.skipped, vec![wrong_type]);
    }

    #[test]
    fn missing_files_resolve_to_schema_defaults() {
        let d = tempfile::tempdir().unwrap();
        let nope = d.path().join("nope.toml");
        let r = resolve::<Schema>(&layers(Some(&nope), Some(&nope), false));
        assert_eq!(r.value, Schema::default());
        assert!(r.skipped.is_empty() && r.ignored_untrusted.is_empty());
    }
}
