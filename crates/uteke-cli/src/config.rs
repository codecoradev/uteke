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
    /// Embedding model name.
    pub model: String,
    /// Maximum sequence length for the embedding model.
    pub max_seq_length: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
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
    /// Missing/empty values in the file don't overwrite existing ones.
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
        let overlay: Config = match toml::from_str(&content) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Invalid config {}: {e}", path.display());
                return self;
            }
        };

        // Store: only override non-default values
        if overlay.store.path != StoreConfig::default().path {
            self.store.path = overlay.store.path;
        }
        if overlay.store.namespace != StoreConfig::default().namespace {
            self.store.namespace = overlay.store.namespace;
        }

        // Embedding
        if overlay.embedding.model != EmbeddingConfig::default().model {
            self.embedding.model = overlay.embedding.model;
        }
        if overlay.embedding.max_seq_length != EmbeddingConfig::default().max_seq_length {
            self.embedding.max_seq_length = overlay.embedding.max_seq_length;
        }

        // Tier
        if overlay.tier.hot_days != TierConfig::default().hot_days {
            self.tier.hot_days = overlay.tier.hot_days;
        }
        if overlay.tier.warm_days != TierConfig::default().warm_days {
            self.tier.warm_days = overlay.tier.warm_days;
        }
        if (overlay.tier.hot_boost - TierConfig::default().hot_boost).abs() > f64::EPSILON {
            self.tier.hot_boost = overlay.tier.hot_boost;
        }

        // Logging
        if overlay.logging.level != LoggingConfig::default().level {
            self.logging.level = overlay.logging.level;
        }
        if !overlay.logging.file.is_empty() {
            self.logging.file = overlay.logging.file;
        }

        // Aging
        if overlay.aging.enabled != AgingConfig::default().enabled {
            self.aging.enabled = overlay.aging.enabled;
        }
        if overlay.aging.max_age_days != AgingConfig::default().max_age_days {
            self.aging.max_age_days = overlay.aging.max_age_days;
        }
        if overlay.aging.max_cold_count != AgingConfig::default().max_cold_count {
            self.aging.max_cold_count = overlay.aging.max_cold_count;
        }

        // Server
        if overlay.server.enabled != ServerConfig::default().enabled {
            self.server.enabled = overlay.server.enabled;
        }
        if overlay.server.host != ServerConfig::default().host {
            self.server.host = overlay.server.host;
        }
        if overlay.server.port != ServerConfig::default().port {
            self.server.port = overlay.server.port;
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
# See https://github.com/ajianaz/uteke for documentation

[store]
# path = "~/.uteke"
# namespace = "default"

[embedding]
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
}
