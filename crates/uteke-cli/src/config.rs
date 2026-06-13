//! Configuration management for uteke CLI.
//!
//! Layered config resolution: CLI args > project `.uteke/uteke.toml` > global `~/.uteke/uteke.toml` > defaults.
//! Migrates legacy `config.toml` → `uteke.toml` on load.

use std::path::PathBuf;

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
            path: "~/.uteke".to_string(),
            namespace: "default".to_string(),
        }
    }
}

/// Embedding model configuration.
#[derive(serde::Deserialize, Clone)]
#[serde(default)]
pub struct EmbeddingConfig {
    /// Embedding backend: "onnx" (default), future: "openai", "ollama".
    pub backend: String,
    /// Embedding model name.
    pub model: String,
    /// Maximum sequence length for the embedding model.
    pub max_seq_length: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            backend: "onnx".to_string(),
            model: "embeddinggemma-q4".to_string(),
            max_seq_length: 256,
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
    /// Maximum age in days before pruning.
    pub max_age_days: u32,
    /// Maximum number of cold memories to keep.
    pub max_cold_count: usize,
}

impl Default for AgingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_age_days: 180,
            max_cold_count: 10000,
        }
    }
}

/// Recall / search threshold configuration.
#[derive(serde::Deserialize, Clone)]
#[serde(default)]
pub struct RecallConfig {
    /// Minimum cosine similarity score for recall results. Results below are filtered out.
    pub min_score: f64,
    /// Strict mode threshold (higher, for critical queries).
    pub min_score_strict: f64,
}

impl Default for RecallConfig {
    fn default() -> Self {
        Self {
            min_score: 0.3,
            min_score_strict: 0.5,
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

/// Full uteke configuration, loaded from `uteke.toml`.
#[derive(serde::Deserialize, Default, Clone)]
#[serde(default)]
pub struct Config {
    pub store: StoreConfig,
    pub embedding: EmbeddingConfig,
    pub tier: TierConfig,
    pub logging: LoggingConfig,
    pub aging: AgingConfig,
    pub recall: RecallConfig,
    pub server: ServerConfig,
}

impl Config {
    /// Load config with layered resolution:
    /// 1. Defaults
    /// 2. Global `~/.uteke/uteke.toml`
    /// 3. Project `.uteke/uteke.toml`
    ///
    /// Each layer overrides the previous. Legacy `config.toml` is migrated
    /// to `uteke.toml` when found.
    pub fn load() -> Self {
        let mut config = Self::default();

        // Migrate legacy config.toml → uteke.toml at global location
        migrate_legacy_global();

        // Layer 1: global ~/.uteke/uteke.toml
        if let Some(global_path) = global_config_path() {
            config = config.merge_from_file(&global_path);
        }

        // Layer 2: project .uteke/uteke.toml
        if let Ok(cwd) = std::env::current_dir() {
            let project_path = cwd.join(".uteke").join("uteke.toml");
            config = config.merge_from_file(&project_path);
        }

        config
    }

    /// Merge values from a TOML file on top of this config.
    /// Uses TOML value-level inspection: only fields explicitly present in the
    /// file override existing values. Missing fields keep their current value.
    fn merge_from_file(mut self, path: &std::path::Path) -> Self {
        if !path.exists() {
            return self;
        }
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Cannot read config {}: {e}", path.display());
                return self;
            }
        };

        // Parse raw TOML table to inspect which keys are explicitly present
        let raw: toml::Value = match toml::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Invalid config {}: {e}", path.display());
                return self;
            }
        };

        let table = match raw.as_table() {
            Some(t) => t,
            None => return self,
        };

        // Also parse as typed Config for safe value extraction
        let overlay: Config = match toml::from_str(&content) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Invalid config {}: {e}", path.display());
                return self;
            }
        };

        // Merge store section
        if let Some(store) = table.get("store").and_then(|v| v.as_table()) {
            if store.contains_key("path") {
                self.store.path = overlay.store.path;
            }
            if store.contains_key("namespace") {
                self.store.namespace = overlay.store.namespace;
            }
        }

        // Merge embedding section
        if let Some(emb) = table.get("embedding").and_then(|v| v.as_table()) {
            if emb.contains_key("backend") {
                self.embedding.backend = overlay.embedding.backend.clone();
            }
            if emb.contains_key("model") {
                self.embedding.model = overlay.embedding.model;
            }
            if emb.contains_key("max_seq_length") {
                self.embedding.max_seq_length = overlay.embedding.max_seq_length;
            }
        }

        // Merge tier section
        if let Some(tier) = table.get("tier").and_then(|v| v.as_table()) {
            if tier.contains_key("hot_days") {
                self.tier.hot_days = overlay.tier.hot_days;
            }
            if tier.contains_key("warm_days") {
                self.tier.warm_days = overlay.tier.warm_days;
            }
            if tier.contains_key("hot_boost") {
                self.tier.hot_boost = overlay.tier.hot_boost;
            }
        }

        // Merge logging section
        if let Some(log) = table.get("logging").and_then(|v| v.as_table()) {
            if log.contains_key("level") {
                self.logging.level = overlay.logging.level;
            }
            if log.contains_key("file") {
                self.logging.file = overlay.logging.file;
            }
        }

        // Merge aging section
        if let Some(aging) = table.get("aging").and_then(|v| v.as_table()) {
            if aging.contains_key("enabled") {
                self.aging.enabled = overlay.aging.enabled;
            }
            if aging.contains_key("max_age_days") {
                self.aging.max_age_days = overlay.aging.max_age_days;
            }
            if aging.contains_key("max_cold_count") {
                self.aging.max_cold_count = overlay.aging.max_cold_count;
            }
        }

        // Merge recall section
        if let Some(recall) = table.get("recall").and_then(|v| v.as_table()) {
            if recall.contains_key("min_score") {
                self.recall.min_score = overlay.recall.min_score;
            }
            if recall.contains_key("min_score_strict") {
                self.recall.min_score_strict = overlay.recall.min_score_strict;
            }
        }

        // Merge server section
        if let Some(server) = table.get("server").and_then(|v| v.as_table()) {
            if server.contains_key("enabled") {
                self.server.enabled = overlay.server.enabled;
            }
            if server.contains_key("host") {
                self.server.host = overlay.server.host;
            }
            if server.contains_key("port") {
                self.server.port = overlay.server.port;
            }
        }

        self
    }

    /// Ensure the global uteke directory exists and return its path.
    pub fn ensure_dirs() -> PathBuf {
        let base = dirs::home_dir()
            .expect("Cannot determine home directory")
            .join(".uteke");
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
# path = "~/.uteke"
# namespace = "default"

[embedding]
# backend = "onnx"  # future: "openai", "ollama"
# model = "embeddinggemma-q4"
# max_seq_length = 256

[tier]
# hot_days = 7
# warm_days = 30
# hot_boost = 0.1

[logging]
# level = "warn"
# file = ""

[aging]
# enabled = false
# max_age_days = 180
# max_cold_count = 10000

[recall]
# min_score = 0.3
# min_score_strict = 0.5

[server]
# enabled = false
# host = "127.0.0.1"
# port = 8767
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
        let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
        let config_path = home.join(".uteke").join("uteke.toml");

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

/// Migrate legacy `~/.uteke/config.toml` → `~/.uteke/uteke.toml`.
fn migrate_legacy_global() {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return,
    };
    let base = home.join(".uteke");
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
fn migrate_content(old: &str) -> String {
    // Old format had flat keys like `store_path`, `namespace`.
    // New format uses sections. Attempt best-effort conversion.
    let mut out = String::from("# Migrated from config.toml\n");
    let mut store_section = String::new();
    let mut embedding_section = String::new();

    for line in old.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "store_path" => {
                    store_section.push_str(&format!("path = {value}\n"));
                }
                "namespace" => {
                    store_section.push_str(&format!("namespace = {value}\n"));
                }
                "model" => {
                    embedding_section.push_str(&format!("model = {value}\n"));
                }
                "max_seq_length" => {
                    embedding_section.push_str(&format!("max_seq_length = {value}\n"));
                }
                _ => {
                    // Unknown key — pass through
                    out.push_str(line);
                    out.push('\n');
                }
            }
        } else if trimmed.starts_with('[') {
            // Section header — pass through (new-format sections)
            out.push_str(line);
            out.push('\n');
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

/// Return the global config path `~/.uteke/uteke.toml` if home dir is known.
fn global_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".uteke").join("uteke.toml"))
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

    #[test]
    fn default_config_values() {
        let cfg = Config::default();
        assert_eq!(cfg.store.path, "~/.uteke");
        assert_eq!(cfg.store.namespace, "default");
        assert_eq!(cfg.embedding.model, "embeddinggemma-q4");
        assert_eq!(cfg.embedding.backend, "onnx");
        assert_eq!(cfg.embedding.max_seq_length, 256);
        assert_eq!(cfg.tier.hot_days, 7);
        assert_eq!(cfg.tier.warm_days, 30);
        assert!((cfg.tier.hot_boost - 0.1).abs() < f64::EPSILON);
        assert_eq!(cfg.logging.level, "warn");
        assert!(cfg.logging.file.is_empty());
        assert!(!cfg.aging.enabled);
        assert_eq!(cfg.aging.max_age_days, 180);
        assert_eq!(cfg.aging.max_cold_count, 10000);
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
        assert_eq!(cfg.store.path, "~/.uteke");
        assert_eq!(cfg.embedding.model, "embeddinggemma-q4");
        assert_eq!(cfg.embedding.backend, "onnx");
        assert_eq!(cfg.embedding.max_seq_length, 256);
        assert_eq!(cfg.tier.hot_days, 7);
        assert!(!cfg.aging.enabled);
    }

    #[test]
    fn merge_file_overrides_non_defaults() {
        let base = Config::default();
        let toml = r#"
[store]
path = "/custom/store"
namespace = "prod"

[embedding]
model = "other-model"
"#;
        let tmp = std::env::temp_dir().join("uteke_test_merge.toml");
        std::fs::write(&tmp, toml).unwrap();
        let merged = base.merge_from_file(&tmp);
        std::fs::remove_file(&tmp).ok();

        assert_eq!(merged.store.path, "/custom/store");
        assert_eq!(merged.store.namespace, "prod");
        assert_eq!(merged.embedding.model, "other-model");
        // Unchanged defaults
        assert_eq!(merged.embedding.max_seq_length, 256);
        assert_eq!(merged.tier.hot_days, 7);
    }

    #[test]
    fn merge_nonexistent_file_returns_self() {
        let cfg = Config::default();
        let merged = cfg
            .clone()
            .merge_from_file(std::path::Path::new("/no/such/file.toml"));
        assert_eq!(merged.store.path, cfg.store.path);
    }

    #[test]
    fn expand_tilde() {
        let expanded = Config::expand_tilde("~/foo");
        assert!(!expanded.starts_with('~'));
        assert!(expanded.ends_with("foo"));

        let no_tilde = Config::expand_tilde("/absolute/path");
        assert_eq!(no_tilde, "/absolute/path");
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
        assert!((cfg.min_score - 0.3).abs() < f64::EPSILON);
        assert!((cfg.min_score_strict - 0.5).abs() < f64::EPSILON);
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
        let tmp = std::env::temp_dir().join("uteke_test_merge_recall.toml");
        std::fs::write(&tmp, toml).unwrap();
        let merged = Config::default().merge_from_file(&tmp);
        std::fs::remove_file(&tmp).ok();

        assert!((merged.recall.min_score - 0.6).abs() < f64::EPSILON);
        assert!((merged.recall.min_score_strict - 0.8).abs() < f64::EPSILON);
        // Other config untouched
        assert_eq!(merged.store.path, "~/.uteke");
        assert_eq!(merged.embedding.model, "embeddinggemma-q4");
        assert_eq!(merged.tier.hot_days, 7);
        assert_eq!(merged.logging.level, "warn");
        assert!(!merged.aging.enabled);
        assert!(!merged.server.enabled);
    }

    #[test]
    fn merge_from_file_all_sections() {
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
        let tmp = std::env::temp_dir().join("uteke_test_all_sections.toml");
        std::fs::write(&tmp, toml).unwrap();
        let merged = Config::default().merge_from_file(&tmp);
        std::fs::remove_file(&tmp).ok();

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
    fn merge_from_file_invalid_toml() {
        let tmp = std::env::temp_dir().join("uteke_test_invalid.toml");
        std::fs::write(&tmp, "this is not valid toml [[[[").unwrap();
        let merged = Config::default().merge_from_file(&tmp);
        std::fs::remove_file(&tmp).ok();
        // Should return defaults when file is invalid
        assert_eq!(merged.store.path, "~/.uteke");
    }

    #[test]
    fn merge_embedding_backend() {
        let toml = r#"
[embedding]
backend = "ollama"
"#;
        let tmp = std::env::temp_dir().join("uteke_test_backend_merge.toml");
        std::fs::write(&tmp, toml).unwrap();
        let merged = Config::default().merge_from_file(&tmp);
        std::fs::remove_file(&tmp).ok();
        assert_eq!(merged.embedding.backend, "ollama");
        // Other embedding fields stay default
        assert_eq!(merged.embedding.model, "embeddinggemma-q4");
        assert_eq!(merged.embedding.max_seq_length, 256);
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
}
