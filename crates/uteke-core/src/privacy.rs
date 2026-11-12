//! The privacy filter of the write path (#1464).
//!
//! The secret detector itself lives in the small `uteke-privacy` crate (#1732),
//! so `tole` can share the same ruleset without taking `uteke-core`'s weight;
//! this module re-exports it and adds what is specific to uteke: the
//! `[privacy]` settings, the modes, and the filter as `remember`, import, room
//! remember and update apply it (metadata flags, `strict` refusal).
//!
//! [`redact`] finds common secret shapes in text and replaces the secret part
//! with a masked form that keeps the shape: `ghp_[REDACTED:github-token]`,
//! `Bearer [REDACTED:bearer-token]`, `API_KEY=[REDACTED:env-secret]`. It reports
//! what it removed (a kind and a count, never the value).

use std::collections::BTreeMap;

use regex::Regex;

use crate::Error;

pub use uteke_privacy::{Policy, PolicyError, Redacted, RedactionCount, redact_with_allowlist};

/// Result of [`redact`] (the shared crate's [`Redacted`]).
pub type Redaction = Redacted;

/// Mask every secret found in `input`; a match that any `allowlist` regex also
/// matches is kept as it is.
pub fn redact(input: &str, allowlist: &[Regex]) -> Redaction {
    redact_with_allowlist(input, allowlist)
}

/// Compile the `[privacy] allowlist` entries; the first invalid one is an error.
pub fn compile_allowlist(patterns: &[String]) -> Result<Vec<Regex>, Error> {
    uteke_privacy::compile_allowlist(patterns).map_err(|e| Error::Validation(e.to_string()))
}

/// What the write path does with a secret (`[privacy] mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrivacyMode {
    /// Mask the secret, store the rest, flag the memory (the default).
    #[default]
    Redact,
    /// Refuse the write with [`Error::SecretDetected`].
    Strict,
    /// Store the content as given.
    Off,
}

impl PrivacyMode {
    /// Parse `redact`, `strict` or `off` (case-insensitive).
    pub fn parse(s: &str) -> Result<Self, Error> {
        match s.trim().to_ascii_lowercase().as_str() {
            "redact" => Ok(Self::Redact),
            "strict" => Ok(Self::Strict),
            "off" => Ok(Self::Off),
            other => Err(Error::Validation(format!(
                "invalid privacy mode '{other}' (expected redact, strict or off)"
            ))),
        }
    }
}

/// The privacy filter as the write path applies it.
#[derive(Debug, Clone, Default)]
pub struct PrivacyConfig {
    pub mode: PrivacyMode,
    /// Values matching any of these regexes are kept as they are.
    pub allowlist: Vec<Regex>,
    /// Keep a rejected-value tombstone as a hash only: no preview of the
    /// forgotten text (#1669). Off by default.
    pub tombstone_hash_only: bool,
}

/// `[privacy]` in `uteke.toml`.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct PrivacySettings {
    /// `redact` (default), `strict` or `off`.
    pub mode: Option<String>,
    /// Regexes; a value matching any of them is kept as it is.
    pub allowlist: Vec<String>,
    /// Keep a preview (the first 120 characters, filtered) with a
    /// rejected-value tombstone. `false` keeps the hash only. Default `true`.
    pub tombstone_preview: Option<bool>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct PrivacyFile {
    privacy: PrivacySettings,
}

impl PrivacyConfig {
    /// Build the config from `[privacy]` settings. An unknown mode or an
    /// invalid regex is an error; callers that must keep running use
    /// [`load_config`], which falls back to the safe default.
    pub fn from_settings(settings: &PrivacySettings) -> Result<Self, Error> {
        Ok(Self {
            mode: match settings.mode.as_deref() {
                Some(m) => PrivacyMode::parse(m)?,
                None => PrivacyMode::default(),
            },
            allowlist: compile_allowlist(&settings.allowlist)?,
            tombstone_hash_only: settings.tombstone_preview == Some(false),
        })
    }
}

/// The privacy config of this installation: `[privacy]` from the global
/// `uteke.toml` (the project-local file is never trusted for it, so a cloned
/// repository cannot switch the filter off), then `UTEKE_PRIVACY_MODE`.
/// A bad value is logged and replaced by the default (`redact`): the filter
/// never turns itself off because of a typo.
pub fn load_config() -> PrivacyConfig {
    let mut settings = crate::config_layers::resolve_standard::<PrivacyFile>()
        .value
        .privacy;
    if let Ok(mode) = std::env::var("UTEKE_PRIVACY_MODE") {
        settings.mode = Some(mode);
    }
    config_or_default(&settings)
}

fn config_or_default(settings: &PrivacySettings) -> PrivacyConfig {
    // Each setting falls back on its own: a bad allowlist entry must not
    // lose a `strict` mode, and a bad mode must not drop the allowlist.
    let mode = match settings.mode.as_deref().map(PrivacyMode::parse) {
        Some(Ok(m)) => m,
        Some(Err(e)) => {
            tracing::warn!("Invalid [privacy] mode, using redact: {e}");
            PrivacyMode::default()
        }
        None => PrivacyMode::default(),
    };
    let allowlist = settings
        .allowlist
        .iter()
        .filter_map(|p| match compile_allowlist(std::slice::from_ref(p)) {
            Ok(mut v) => v.pop(),
            Err(e) => {
                tracing::warn!("Ignoring [privacy] allowlist entry: {e}");
                None
            }
        })
        .collect();
    PrivacyConfig {
        mode,
        allowlist,
        tombstone_hash_only: settings.tombstone_preview == Some(false),
    }
}

/// Content and metadata ready to store, plus what the filter did.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub content: String,
    /// The tags as given, with any secret masked.
    pub tags: Vec<String>,
    pub metadata: Option<serde_json::Value>,
    /// Empty when nothing was masked.
    pub redactions: Vec<RedactionCount>,
}

impl PrivacyConfig {
    /// Apply the filter to `content` before it is embedded or stored.
    /// `Redact` masks secrets and merges `redacted: true` and `redactions`
    /// (`[{kind, count}]`, never a value) into the metadata; `Strict` returns
    /// [`Error::SecretDetected`] when there is anything to mask; `Off` and
    /// clean content pass through unchanged.
    pub fn prepare(
        &self,
        content: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<Prepared, Error> {
        self.prepare_with_tags(content, &[], metadata)
    }

    /// [`prepare`](Self::prepare) for a whole write: the content, the tags and
    /// every string value in the metadata are filtered, and the counts are
    /// merged into one `redactions` list.
    pub fn prepare_with_tags(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
    ) -> Result<Prepared, Error> {
        let untouched = |metadata| Prepared {
            content: content.to_string(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            metadata,
            redactions: Vec::new(),
        };
        if self.mode == PrivacyMode::Off {
            return Ok(untouched(metadata));
        }
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        let r = redact(content, &self.allowlist);
        merge_counts(&mut counts, &r.counts);
        let tags_out: Vec<String> = tags
            .iter()
            .map(|t| {
                let r = redact(t, &self.allowlist);
                merge_counts(&mut counts, &r.counts);
                r.text
            })
            .collect();
        let metadata_out = metadata.as_ref().map(|m| self.mask_json(m, &mut counts));
        if counts.is_empty() {
            return Ok(untouched(metadata));
        }
        let redactions: Vec<RedactionCount> = counts
            .into_iter()
            .map(|(kind, count)| RedactionCount { kind, count })
            .collect();
        if self.mode == PrivacyMode::Strict {
            return Err(Error::SecretDetected {
                kinds: redactions.iter().map(|x| x.kind.to_string()).collect(),
            });
        }
        Ok(Prepared {
            content: r.text,
            tags: tags_out,
            metadata: with_redaction_flags(metadata_out, &redactions),
            redactions,
        })
    }

    /// Mask a free-text note that is kept after a secret may have been typed
    /// into it (a tombstone's reason or preview). Never refuses: `strict`
    /// masks here too, since the forget itself must go through. `off` keeps
    /// the text as given. Text the filter cannot process is withheld.
    pub fn mask_note(&self, text: &str) -> String {
        if self.mode == PrivacyMode::Off {
            return text.to_string();
        }
        let redact = PrivacyConfig {
            mode: PrivacyMode::Redact,
            allowlist: self.allowlist.clone(),
            tombstone_hash_only: self.tombstone_hash_only,
        };
        redact
            .mask_text(text)
            .map(|(t, _)| t)
            .unwrap_or_else(|_| "[withheld]".to_string())
    }

    /// Filter one free-text field (the `source` of a memory). Same modes as
    /// [`prepare`](Self::prepare); returns the text to store and what was masked.
    pub fn mask_text(&self, text: &str) -> Result<(String, Vec<RedactionCount>), Error> {
        let p = self.prepare_with_tags("", &[text], None)?;
        Ok((p.tags.into_iter().next().unwrap_or_default(), p.redactions))
    }

    /// Filter the metadata of an update that has no new content. Same modes as
    /// [`prepare`](Self::prepare); the flags are not added here.
    pub fn mask_metadata(
        &self,
        metadata: &serde_json::Value,
    ) -> Result<(serde_json::Value, Vec<RedactionCount>), Error> {
        if self.mode == PrivacyMode::Off {
            return Ok((metadata.clone(), Vec::new()));
        }
        let mut counts = BTreeMap::new();
        let masked = self.mask_json(metadata, &mut counts);
        let redactions: Vec<RedactionCount> = counts
            .into_iter()
            .map(|(kind, count)| RedactionCount { kind, count })
            .collect();
        if self.mode == PrivacyMode::Strict && !redactions.is_empty() {
            return Err(Error::SecretDetected {
                kinds: redactions.iter().map(|x| x.kind.to_string()).collect(),
            });
        }
        Ok((masked, redactions))
    }

    /// Filter the tags of an update.
    pub fn mask_tags(&self, tags: &[String]) -> Result<(Vec<String>, Vec<RedactionCount>), Error> {
        let refs: Vec<&str> = tags.iter().map(String::as_str).collect();
        let p = self.prepare_with_tags("", &refs, None)?;
        Ok((p.tags, p.redactions))
    }

    /// `value` with every string inside it filtered (keys are left alone).
    fn mask_json(
        &self,
        value: &serde_json::Value,
        counts: &mut BTreeMap<&'static str, usize>,
    ) -> serde_json::Value {
        use serde_json::Value;
        match value {
            Value::String(s) => {
                let r = redact(s, &self.allowlist);
                merge_counts(counts, &r.counts);
                Value::String(r.text)
            }
            Value::Array(items) => {
                Value::Array(items.iter().map(|v| self.mask_json(v, counts)).collect())
            }
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), self.mask_json(v, counts)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

fn merge_counts(into: &mut BTreeMap<&'static str, usize>, from: &[RedactionCount]) {
    for c in from {
        *into.entry(c.kind).or_default() += c.count;
    }
}

pub use crate::redaction_flags::{
    carry_redaction_flags, with_redaction_flags, without_redaction_flags,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake secret assembled at run time so the repository holds no scanner bait.
    fn gh() -> String {
        format!("{}{}", "ghp_", "aB3dE5gH7jK9mN1pQ3rS5tU7vW9xY1zA3bC5")
    }

    fn cfg(mode: PrivacyMode) -> PrivacyConfig {
        PrivacyConfig {
            mode,
            allowlist: Vec::new(),
            ..Default::default()
        }
    }

    #[test]
    fn modes_parse_and_default_to_redact() {
        assert_eq!(PrivacyMode::default(), PrivacyMode::Redact);
        assert_eq!(PrivacyMode::parse("strict").unwrap(), PrivacyMode::Strict);
        assert_eq!(PrivacyMode::parse("off").unwrap(), PrivacyMode::Off);
        assert!(PrivacyMode::parse("maybe").is_err());
    }

    #[test]
    fn prepare_follows_the_mode() {
        let text = format!("token {}", gh());
        let masked = cfg(PrivacyMode::Redact).prepare(&text, None).unwrap();
        assert!(masked.content.contains("[REDACTED:github-token]"));
        assert_eq!(masked.metadata.unwrap()["redacted"], true);
        assert_eq!(masked.redactions.len(), 1);

        let off = cfg(PrivacyMode::Off).prepare(&text, None).unwrap();
        assert_eq!(off.content, text);
        assert!(off.redactions.is_empty() && off.metadata.is_none());

        match cfg(PrivacyMode::Strict).prepare(&text, None) {
            Err(Error::SecretDetected { kinds }) => assert_eq!(kinds, ["github-token"]),
            other => panic!("expected SecretDetected, got {other:?}"),
        }
        let clean = cfg(PrivacyMode::Strict)
            .prepare("nothing here", None)
            .unwrap();
        assert_eq!(clean.content, "nothing here");
    }

    #[test]
    fn the_flag_helpers_add_drop_and_carry_the_flags() {
        let counts = [RedactionCount {
            kind: "jwt",
            count: 2,
        }];
        let flagged = with_redaction_flags(Some(serde_json::json!({"k": "v"})), &counts).unwrap();
        assert_eq!(flagged["k"], "v");
        assert_eq!(flagged["redacted"], true);
        assert_eq!(flagged["redactions"][0]["kind"], "jwt");
        assert_eq!(flagged["redactions"][0]["count"], 2);

        let clean = without_redaction_flags(&flagged).unwrap();
        assert_eq!(clean, serde_json::json!({"k": "v"}));
        assert!(without_redaction_flags(&clean).is_none(), "nothing to drop");

        let carried = carry_redaction_flags(&serde_json::json!({"n": 1}), &flagged).unwrap();
        assert_eq!(carried["n"], 1);
        assert_eq!(carried["redacted"], true);
        // The caller's own flags are never overridden.
        assert!(carry_redaction_flags(&serde_json::json!({"redacted": false}), &flagged).is_none());
    }

    #[test]
    fn settings_become_a_config_and_bad_values_fall_back_one_by_one() {
        let ok = PrivacyConfig::from_settings(&PrivacySettings {
            mode: Some("Strict".into()),
            allowlist: vec!["^dev-".into()],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(ok.mode, PrivacyMode::Strict);
        assert_eq!(ok.allowlist.len(), 1);
        assert!(
            PrivacyConfig::from_settings(&PrivacySettings {
                mode: Some("maybe".into()),
                allowlist: vec![],
                ..Default::default()
            })
            .is_err()
        );

        // A bad mode keeps the allowlist and never turns the filter off; a bad
        // regex keeps the mode.
        let c = config_or_default(&PrivacySettings {
            mode: Some("none".into()),
            allowlist: vec!["^dev-".into()],
            ..Default::default()
        });
        assert_eq!((c.mode, c.allowlist.len()), (PrivacyMode::Redact, 1));
        let c = config_or_default(&PrivacySettings {
            mode: Some("strict".into()),
            allowlist: vec!["(".into(), "^dev-".into()],
            ..Default::default()
        });
        assert_eq!((c.mode, c.allowlist.len()), (PrivacyMode::Strict, 1));
        assert!(!c.tombstone_hash_only, "previews are kept by default");
        let hash_only = PrivacySettings {
            tombstone_preview: Some(false),
            ..Default::default()
        };
        assert!(config_or_default(&hash_only).tombstone_hash_only);
        assert!(
            PrivacyConfig::from_settings(&hash_only)
                .unwrap()
                .tombstone_hash_only
        );
    }

    #[test]
    fn privacy_is_read_from_the_global_file_only() {
        use crate::config_layers::{Layers, resolve};
        let dir = std::env::temp_dir().join(format!("uteke-privacy-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let global = dir.join("global.toml");
        let project = dir.join("project.toml");
        std::fs::write(&global, "[privacy]\nmode = \"strict\"\n").unwrap();
        std::fs::write(
            &project,
            "[privacy]\nmode = \"off\"\nallowlist = [\".*\"]\n",
        )
        .unwrap();
        let r = resolve::<PrivacyFile>(&Layers {
            global: Some(&global),
            project: Some(&project),
            trust_project: false,
        });
        assert_eq!(r.value.privacy.mode.as_deref(), Some("strict"));
        assert!(r.value.privacy.allowlist.is_empty());
        assert!(r.ignored_untrusted.contains(&"privacy".to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }
}
