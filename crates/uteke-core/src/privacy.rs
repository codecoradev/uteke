//! Secret detection for the write path (#1464, part 1).
//!
//! [`redact`] finds common secret shapes in text and replaces the secret part
//! with a masked form that keeps the shape: `ghp_[REDACTED:github-token]`,
//! `Bearer [REDACTED:bearer-token]`, `API_KEY=[REDACTED:env-secret]`. It reports
//! what it removed (a kind and a count, never the value) so the caller can set
//! `metadata.redacted` and write an audit entry.
//!
//! Precision comes first: the filter will run by default, so a pattern is only
//! here when its shape is specific enough to rarely hit ordinary text. The one
//! generic rule, `NAME=value` assignments, needs a secret-like name, a long
//! value with enough entropy, and ignores hashes, UUIDs, paths and obvious
//! placeholders. A caller-supplied allowlist (regexes) keeps a value that
//! matches any entry. [`redact`] is idempotent.
//!
//! Nothing here touches storage; wiring into `remember`, import, room remember
//! and update is part 2.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::{Captures, Regex};

use crate::Error;

/// How many secrets of one kind were masked.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RedactionCount {
    pub kind: &'static str,
    pub count: usize,
}

/// Result of [`redact`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redaction {
    /// The text with every detected secret masked.
    pub text: String,
    /// What was masked, ordered by kind. Empty when the text was clean.
    pub redactions: Vec<RedactionCount>,
}

impl Redaction {
    /// Whether nothing was masked.
    pub fn is_clean(&self) -> bool {
        self.redactions.is_empty()
    }

    /// Total number of masked secrets.
    pub fn total(&self) -> usize {
        self.redactions.iter().map(|r| r.count).sum()
    }
}

/// Compile the `[privacy] allowlist` entries; the first invalid one is an error.
pub fn compile_allowlist(patterns: &[String]) -> Result<Vec<Regex>, Error> {
    patterns
        .iter()
        .map(|p| {
            Regex::new(p).map_err(|e| {
                Error::Validation(format!("invalid privacy allowlist regex '{p}': {e}"))
            })
        })
        .collect()
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
}

/// Content and metadata ready to store, plus what the filter did.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub content: String,
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
        let untouched = |metadata| Prepared {
            content: content.to_string(),
            metadata,
            redactions: Vec::new(),
        };
        if self.mode == PrivacyMode::Off {
            return Ok(untouched(metadata));
        }
        let r = redact(content, &self.allowlist);
        if r.is_clean() {
            return Ok(untouched(metadata));
        }
        if self.mode == PrivacyMode::Strict {
            return Err(Error::SecretDetected {
                kinds: r.redactions.iter().map(|x| x.kind.to_string()).collect(),
            });
        }
        Ok(Prepared {
            content: r.text,
            metadata: with_redaction_flags(metadata, &r.redactions),
            redactions: r.redactions,
        })
    }
}

/// Merge `redacted: true` and `redactions` into `metadata`. `None` becomes a new
/// object; a metadata value that is not an object is left alone (there is
/// nowhere to merge), and the caller still learns of the redaction from the
/// returned counts.
pub fn with_redaction_flags(
    metadata: Option<serde_json::Value>,
    redactions: &[RedactionCount],
) -> Option<serde_json::Value> {
    if redactions.is_empty() {
        return metadata;
    }
    // A JSON `null` (an import row with `"metadata": null`) is no metadata.
    let mut value = match metadata {
        None | Some(serde_json::Value::Null) => serde_json::json!({}),
        Some(v) => v,
    };
    if let serde_json::Value::Object(map) = &mut value {
        map.insert("redacted".into(), serde_json::json!(true));
        map.insert(
            "redactions".into(),
            serde_json::to_value(redactions).unwrap_or_default(),
        );
    }
    Some(value)
}

/// `metadata` without the `redacted` and `redactions` keys, or `None` when it
/// has neither (nothing to change). The flags describe the CURRENT content, so
/// an update to clean content drops the ones an earlier version left; the
/// `redacted` timeline event keeps the history.
pub fn without_redaction_flags(metadata: &serde_json::Value) -> Option<serde_json::Value> {
    let serde_json::Value::Object(map) = metadata else {
        return None;
    };
    if !map.contains_key("redacted") && !map.contains_key("redactions") {
        return None;
    }
    let mut map = map.clone();
    map.remove("redacted");
    map.remove("redactions");
    Some(serde_json::Value::Object(map))
}

/// `new` with the redaction flags of `old` carried over, for an update that
/// replaces the metadata but leaves the (still masked) content alone. `None`
/// when `old` has no flags, `new` is not an object, or `new` sets its own.
pub fn carry_redaction_flags(
    new: &serde_json::Value,
    old: &serde_json::Value,
) -> Option<serde_json::Value> {
    let (serde_json::Value::Object(new_map), serde_json::Value::Object(old_map)) = (new, old)
    else {
        return None;
    };
    if new_map.contains_key("redacted") || new_map.contains_key("redactions") {
        return None;
    }
    let mut merged = new_map.clone();
    let mut carried = false;
    for key in ["redacted", "redactions"] {
        if let Some(v) = old_map.get(key) {
            merged.insert(key.to_string(), v.clone());
            carried = true;
        }
    }
    carried.then_some(serde_json::Value::Object(merged))
}

/// A whole-secret rule: `mask` builds the replacement from the match, or
/// returns `None` to leave this match alone (a placeholder, prose).
struct Rule {
    kind: &'static str,
    pattern: &'static str,
    mask: fn(&Captures) -> Option<String>,
}

fn has_digit(s: &str) -> bool {
    s.bytes().any(|b| b.is_ascii_digit())
}

/// Base64-looking: a digit, `+`, `/`, `=` or mixed case. Plain words
/// (`authentication`, `credentials`) are not.
fn base64_shape(s: &str) -> bool {
    has_digit(s)
        || s.contains(['+', '/', '='])
        || (s.bytes().any(|b| b.is_ascii_uppercase()) && s.bytes().any(|b| b.is_ascii_lowercase()))
}

/// Order matters: a JWT sent as `Bearer <jwt>` is reported as a bearer token.
/// Private key blocks are handled first by [`mask_private_keys`].
const RULES: &[Rule] = &[
    Rule {
        kind: "aws-access-key",
        pattern: r"\b(AKIA|ASIA)[0-9A-Z]{16}\b",
        // AWS's documented example key is not a credential.
        mask: |c| {
            (!c[0].contains("EXAMPLE")).then(|| format!("{}[REDACTED:aws-access-key]", &c[1]))
        },
    },
    Rule {
        kind: "github-token",
        pattern: r"\b(gh[pousr]_)[A-Za-z0-9]{36,}\b",
        mask: |c| Some(format!("{}[REDACTED:github-token]", &c[1])),
    },
    Rule {
        kind: "github-token",
        pattern: r"\b(github_pat_)[A-Za-z0-9_]{22,}\b",
        mask: |c| Some(format!("{}[REDACTED:github-token]", &c[1])),
    },
    Rule {
        kind: "npm-token",
        pattern: r"\b(npm_)[A-Za-z0-9]{36}\b",
        mask: |c| Some(format!("{}[REDACTED:npm-token]", &c[1])),
    },
    Rule {
        kind: "api-key",
        pattern: r"\b(sk-)[A-Za-z0-9_-]{20,}\b",
        // A real key is one long random run (`sk-proj-<24+ characters>`, a
        // 32+ hex body) with mixed case and a digit; a hyphenated name such as
        // `sk-learn-preprocessing-pipeline-v2` or `sk-SK-Wavenet-A-Neural2-Female`
        // has no long run.
        mask: |c| {
            let body = &c[0][c[1].len()..];
            let long_run = body.split(['-', '_']).any(|part| part.len() >= 20);
            let mixed = body.bytes().any(|b| b.is_ascii_uppercase())
                && body.bytes().any(|b| b.is_ascii_lowercase())
                && has_digit(body);
            let hex_body = body.len() >= 32
                && body
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            ((long_run && mixed) || hex_body).then(|| format!("{}[REDACTED:api-key]", &c[1]))
        },
    },
    Rule {
        kind: "api-key",
        pattern: r"\b((?:sk|rk)_(?:live|test)_)[A-Za-z0-9]{16,}\b",
        mask: |c| Some(format!("{}[REDACTED:api-key]", &c[1])),
    },
    Rule {
        kind: "webhook-secret",
        pattern: r"\b(whsec_)[A-Za-z0-9]{24,}\b",
        mask: |c| Some(format!("{}[REDACTED:webhook-secret]", &c[1])),
    },
    Rule {
        kind: "slack-token",
        pattern: r"\b(xox[abprse]-|xapp-)[A-Za-z0-9-]{20,}",
        mask: |c| Some(format!("{}[REDACTED:slack-token]", &c[1])),
    },
    Rule {
        kind: "google-api-key",
        pattern: r"\b(AIza)[0-9A-Za-z_-]{35}\b",
        mask: |c| Some(format!("{}[REDACTED:google-api-key]", &c[1])),
    },
    Rule {
        kind: "basic-auth",
        pattern: r#"(?i)(authorization["']?\s*(?:=>|[:=])\s*["']?basic[ \t]+)([A-Za-z0-9+/]{12,}={0,2})"#,
        mask: |c| {
            (base64_shape(&c[2]) && !is_placeholder(&c[2]))
                .then(|| format!("{}[REDACTED:basic-auth]", &c[1]))
        },
    },
    Rule {
        kind: "auth-token",
        pattern: r#"(?i)(authorization["']?\s*(?:=>|[:=])\s*["']?(?:token|bot|apikey|sso-key)[ \t]+)([A-Za-z0-9._~+/=-]{16,})"#,
        mask: |c| {
            (has_digit(&c[2]) && !is_placeholder(&c[2]))
                .then(|| format!("{}[REDACTED:auth-token]", &c[1]))
        },
    },
    Rule {
        kind: "basic-auth",
        pattern: r#"(?i)((?:-u|--user)[ \t]+[^\s:'"]+:)([^\s'"]{3,})"#,
        mask: |c| {
            (!c[2].starts_with(['$', '<']) && !is_placeholder(&c[2]))
                .then(|| format!("{}[REDACTED:basic-auth]", &c[1]))
        },
    },
    Rule {
        kind: "cli-secret",
        pattern: r#"(?i)\b(docker\s+login\b[^\n]*?\s-p[ \t]+)([^\s"']{6,})"#,
        mask: |c| {
            (!c[2].starts_with(['$', '<', '-']) && !is_placeholder(&c[2]))
                .then(|| format!("{}[REDACTED:cli-secret]", &c[1]))
        },
    },
    Rule {
        kind: "cli-secret",
        pattern: r#"(?i)\b(mysql(?:dump|admin)?\b[^\n]*?\s-p)([^\s"'=-][^\s"']{5,})"#,
        mask: |c| {
            (!c[2].starts_with(['$', '<']) && !is_placeholder(&c[2]))
                .then(|| format!("{}[REDACTED:cli-secret]", &c[1]))
        },
    },
    Rule {
        kind: "cli-secret",
        pattern: r#"(?i)(--(?:password|passwd|token|api-key|apikey|secret|access-token)(?:=|[ \t]+))([^\s"']{6,})"#,
        mask: |c| {
            (!c[2].starts_with(['$', '<', '-', '/', '~']) && !is_placeholder(&c[2]))
                .then(|| format!("{}[REDACTED:cli-secret]", &c[1]))
        },
    },
    Rule {
        kind: "url-password",
        pattern: r"\b([A-Za-z][A-Za-z0-9+.-]*://)([^\s:/@]*)(:)([^\s/?#&]{3,})(@)",
        // The password runs to the last `@` of the authority, so a raw `@` or
        // `/` in it is covered. Ports, placeholders and the usual dev defaults
        // (`postgres:postgres@`) are not secrets.
        mask: |c| {
            let (user, pw) = (&c[2], &c[4]);
            let default = [
                "password", "pass", "secret", "admin", "root", "postgres", "changeme", "guest",
            ]
            .contains(&pw.to_ascii_lowercase().as_str());
            let skip = pw.bytes().all(|b| b.is_ascii_digit())
                || pw == user
                || default
                || is_placeholder(pw)
                || is_placeholder(&c[1]);
            (!skip).then(|| format!("{}{}{}[REDACTED:url-password]{}", &c[1], user, &c[3], &c[5]))
        },
    },
    Rule {
        kind: "bearer-token",
        // Behind an explicit `Authorization:` header the digit is not required.
        pattern: r#"(?i)(authorization["']?\s*(?:=>|[:=])\s*["']?bearer[ \t]+)([A-Za-z0-9._~+/=%-]{16,})"#,
        mask: |c| {
            (base64_shape(&c[2]) && !is_placeholder(&c[2]))
                .then(|| format!("{}[REDACTED:bearer-token]", &c[1]))
        },
    },
    Rule {
        kind: "bearer-token",
        pattern: r"(?i)\b(bearer[ \t]+)([A-Za-z0-9._~+/=%-]{16,})",
        // A real bearer token has a digit; `Bearer tokens/credentials` and
        // `Bearer YOUR_ACCESS_TOKEN_HERE` are prose and placeholders.
        mask: |c| {
            (has_digit(&c[2]) && !is_placeholder(&c[2]))
                .then(|| format!("{}[REDACTED:bearer-token]", &c[1]))
        },
    },
    Rule {
        kind: "jwt",
        // The signature may be empty (`alg: none`): the claims are still data.
        pattern: r"\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*",
        mask: |_| Some("[REDACTED:jwt]".to_string()),
    },
];

/// The name and separator of `NAME=value`, `NAME: value`, `NAME := value`,
/// `'NAME' => 'value'`, `"NAME": "value"`, escaped JSON (`\"NAME\":\"value\"`).
/// The value is read by [`assignment_value`]; whether the name is secret-like
/// and the value looks like a secret is decided in [`mask_assignments`].
const ASSIGNMENT: &str =
    r#"(?m)\b([A-Za-z_][A-Za-z0-9_.-]*)(\\?["']?\s*(?::=|=>|[:=|])\s*\\?["']?)"#;

/// Names that are a secret word by themselves. A bare `key`, `pass`, `token` or
/// `secret` is NOT here: they are ordinary words (a `key` field in a Postman
/// header, a Terraform backend, a type annotation `token: &str`).
const BARE_SECRET_WORDS: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "passphrase",
    "apikey",
    "secretkey",
    "accesskey",
    "privatekey",
    "authtoken",
    "accesstoken",
    "credentials",
    "credential",
];
/// Words a compound name ends with (`API_KEY`, `auth_token`, `DB_PASS`).
const COMPOUND_SECRET_WORDS: &[&str] = &["key", "token", "secret", "pass", "auth"];
/// Suffixes of a single-segment name (`PGPASSWORD`).
const UNAMBIGUOUS_SUFFIXES: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "apikey",
    "secretkey",
    "privatekey",
    "accesskey",
    "authtoken",
    "accesstoken",
];
const PASSWORD_WORDS: &[&str] = &["password", "passwd", "pwd", "pass", "passphrase"];
/// Trailing segments that only qualify a name: `API_KEY_V2`, `SECRET_KEY_BASE`,
/// `DB_PASSWORD_PROD`.
const QUALIFIER_WORDS: &[&str] = &[
    "base",
    "prod",
    "production",
    "dev",
    "development",
    "staging",
    "stage",
    "test",
    "live",
    "new",
    "old",
    "backup",
    "local",
    "primary",
    "secondary",
];
/// A segment before the last one that makes the name something else:
/// `public_key`, `cache_key`, `next_token`, `page_token`, `idempotency_key`.
const NOT_SECRET_PREFIXES: &[&str] = &[
    "public",
    "publishable",
    "site",
    "cache",
    "sort",
    "partition",
    "index",
    "shard",
    "lookup",
    "foreign",
    "checksum",
    "next",
    "page",
    "continuation",
    "cursor",
    "sync",
    "idempotency",
    "release",
    "object",
];
/// A segment that says the value is a hash or identifier, so a hex value is not
/// a secret: `COMMIT_KEY`, `GIT_SHA_KEY`. Only used for a `key` name; a hex
/// `GITHUB_TOKEN` or `ENCRYPTION_KEY` is a secret.
const HASHY_WORDS: &[&str] = &[
    "commit", "sha", "hash", "checksum", "digest", "etag", "rev", "revision", "uuid", "git", "id",
];

/// The lowercase segments of a name, split at `_`, `.`, `-` and camelCase
/// boundaries: `GITHUB_TOKEN` is `github, token`; `apiKey` is `api, key`.
fn name_segments(name: &str) -> Vec<String> {
    let mut segs = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in name.chars() {
        if matches!(ch, '_' | '.' | '-') {
            if !cur.is_empty() {
                segs.push(std::mem::take(&mut cur).to_ascii_lowercase());
            }
            prev_lower = false;
            continue;
        }
        if ch.is_ascii_uppercase() && prev_lower && !cur.is_empty() {
            segs.push(std::mem::take(&mut cur).to_ascii_lowercase());
        }
        prev_lower = ch.is_ascii_lowercase();
        cur.push(ch);
    }
    if !cur.is_empty() {
        segs.push(cur.to_ascii_lowercase());
    }
    segs
}

fn is_version_segment(s: &str) -> bool {
    !s.is_empty()
        && (s.bytes().all(|b| b.is_ascii_digit())
            || (s.starts_with('v') && s.len() > 1 && s[1..].bytes().all(|b| b.is_ascii_digit())))
}

/// `Some(is_password)` when `segs` name a secret: a bare secret word, a single
/// segment ending in an unambiguous one, or a compound name whose last segment
/// (after dropping version and environment qualifiers) is a secret word and
/// that carries no segment turning it into something else.
fn secret_name_kind(segs: &[String], equals_sign: bool) -> Option<bool> {
    let mut segs: Vec<&str> = segs.iter().map(String::as_str).collect();
    while segs.len() > 1
        && segs
            .last()
            .is_some_and(|l| QUALIFIER_WORDS.contains(l) || is_version_segment(l))
    {
        segs.pop();
    }
    let (&last, before) = segs.split_last()?;
    let password = PASSWORD_WORDS.contains(&last)
        || (before.is_empty() && last.ends_with("password") || last.ends_with("passwd"));
    if before.is_empty() {
        // `token=...` and `key=...` with an equals sign are parameters (a query
        // string, an env line); `token:` and `key:` are ordinary words.
        let bare = BARE_SECRET_WORDS.contains(&last)
            || UNAMBIGUOUS_SUFFIXES.iter().any(|s| last.ends_with(s))
            || (equals_sign && ["key", "token", "secret"].contains(&last));
        return bare.then_some(password);
    }
    if before.iter().any(|s| NOT_SECRET_PREFIXES.contains(s)) {
        return None;
    }
    (COMPOUND_SECRET_WORDS.contains(&last)
        || BARE_SECRET_WORDS.contains(&last)
        || UNAMBIGUOUS_SUFFIXES.iter().any(|s| last.ends_with(s)))
    .then_some(password)
}

/// Shannon entropy of `s` in bits per character.
fn entropy(s: &str) -> f64 {
    let mut counts: BTreeMap<char, usize> = BTreeMap::new();
    for c in s.chars() {
        *counts.entry(c).or_default() += 1;
    }
    let n = s.chars().count() as f64;
    counts
        .values()
        .map(|&k| {
            let p = k as f64 / n;
            -p * p.log2()
        })
        .sum()
}

/// How many of lowercase, uppercase, digit and (ASCII) punctuation characters
/// `s` has.
fn char_classes(s: &str) -> usize {
    [
        s.bytes().any(|b| b.is_ascii_lowercase()),
        s.bytes().any(|b| b.is_ascii_uppercase()),
        s.bytes().any(|b| b.is_ascii_digit()),
        s.bytes().any(|b| b.is_ascii_punctuation()),
    ]
    .iter()
    .filter(|b| **b)
    .count()
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_uuid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5
        && [8, 4, 4, 4, 12]
            .iter()
            .zip(&parts)
            .all(|(len, p)| p.len() == *len && is_hex(p))
}

/// Words that mark a value as an example, not a credential.
fn is_placeholder(v: &str) -> bool {
    let lower = v.to_ascii_lowercase();
    v.contains("[REDACTED")
        || [
            "example",
            "your",
            "placeholder",
            "changeme",
            "change-me",
            "xxxx",
            "****",
            "dummy",
            "sample",
            "replace",
            "generate",
            "insert",
            "todo",
            "_here",
            "-here",
            "abcdef",
            "123456",
        ]
        .iter()
        .any(|w| lower.contains(w))
}

/// Words joined by `.` or `_` (or by `-` for a non-password), all lowercase or
/// all uppercase and without digits: a variable, field path or setting name
/// (`config.embedding.api_key`, `VAULT_API_KEY_NAME`), not a secret. Hyphenated
/// lowercase words are a passphrase when the name says password.
fn is_identifier_like(v: &str, password: bool) -> bool {
    let separators: &[char] = if password {
        &['.', '_']
    } else {
        &['.', '_', '-']
    };
    let words: Vec<&str> = v.split(separators).collect();
    // For a key or token a lowercase word may end in a short number (`v2`,
    // `node20`, `credentials-v2`); an all-digit or digit-first part is not a word.
    let word = |w: &str| {
        let letters = w.bytes().take_while(u8::is_ascii_lowercase).count();
        letters > 0 && w[letters..].len() <= 3 && w[letters..].bytes().all(|b| b.is_ascii_digit())
    };
    words.len() > 1
        && words.iter().all(|w| {
            w.len() > 1
                && (w.bytes().all(|b| b.is_ascii_lowercase())
                    || w.bytes().all(|b| b.is_ascii_uppercase())
                    || (!password && word(w)))
        })
}

/// `2026-10-16...`
fn is_date(v: &str) -> bool {
    let b = v.as_bytes();
    b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
}

/// `1.2.3`, `v2.3.1-beta.2`.
fn is_semver(v: &str) -> bool {
    let core = v.trim_start_matches('v');
    let head = core.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = head.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// A 26-character ULID.
fn is_ulid(v: &str) -> bool {
    v.len() == 26
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b.is_ascii_uppercase() && !b"ILOU".contains(&b)))
}

/// `local@domain.tld` and nothing else: an address, not a password with an `@`.
fn is_email(v: &str) -> bool {
    let Some((local, domain)) = v.split_once('@') else {
        return false;
    };
    let tld = domain.rsplit('.').next().unwrap_or("");
    !local.is_empty()
        && local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._%+-".contains(&b))
        && domain.contains('.')
        && domain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-.".contains(&b))
        && tld.len() >= 2
        && tld.bytes().all(|b| b.is_ascii_alphabetic())
}

/// `$VAR`, `${VAR}`, `$(cmd)` and the shell default `-$VAR`, as the WHOLE
/// value. A password may contain `$` (`Pa$Word2026!x`), but not like this.
fn is_variable_reference(v: &str) -> bool {
    let v = v.trim_start_matches('-');
    v.strip_prefix('$').is_some_and(|rest| {
        rest.starts_with(['{', '('])
            || rest
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_uppercase() || b == b'_')
    })
}

/// `min-length=12`: a lowercase setting name, `=`, a short plain value.
fn is_setting(v: &str) -> bool {
    v.split_once('=').is_some_and(|(k, val)| {
        !k.is_empty()
            && k.bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'-' || b == b'_')
            && !val.is_empty()
            && val
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    })
}

/// A relative or absolute path: lowercase segments, or a file extension. A
/// base64 secret has upper-case letters and no extension.
fn is_path_like(v: &str) -> bool {
    if !v.contains('/') {
        return false;
    }
    let segs: Vec<&str> = v.split('/').filter(|s| !s.is_empty()).collect();
    let has_ext = v
        .rsplit('/')
        .next()
        .and_then(|f| f.rsplit_once('.'))
        .is_some_and(|(stem, ext)| {
            !stem.is_empty()
                && (1..=6).contains(&ext.len())
                && ext.bytes().all(|b| b.is_ascii_alphanumeric())
        });
    has_ext
        || (segs.len() >= 2
            && segs.iter().all(|s| {
                s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
            }))
}

/// An identifier with structure, not a secret: ARNs, digests, fingerprints,
/// `0x` addresses and key IDs, `ssh-` key lines.
fn is_structured_identifier(v: &str) -> bool {
    v.starts_with("arn:")
        || v.starts_with("sha256:")
        || v.starts_with("SHA256:")
        || v.starts_with("sha512-")
        || v.starts_with("ssh-")
        || (v.starts_with("0x") && is_hex(&v[2..]))
        || v.split(':').count() >= 4
}

/// A value an assignment rule must leave alone.
fn is_benign_value(v: &str, segs: &[String], password: bool) -> bool {
    // Hex belongs to a hash-named `key` (`COMMIT_KEY`); a hex token, password
    // or encryption key is a secret.
    let hashy_key = segs.last().is_some_and(|l| l == "key")
        && segs.iter().any(|s| HASHY_WORDS.contains(&s.as_str()));
    // An AWS secret access key is 40 base64 characters and may start with `/`.
    let aws_shape = v.len() == 40
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+/".contains(&b))
        && char_classes(v) >= 3;
    v.contains("::")
        || !v.is_ascii()
        || is_variable_reference(v)
        || is_placeholder(v)
        || is_uuid(v)
        || is_date(v)
        || is_semver(v)
        || is_ulid(v)
        || is_email(v)
        || is_identifier_like(v, password)
        || is_structured_identifier(v)
        || (hashy_key && is_hex(v))
        || (v.starts_with('~') && !aws_shape)
        || (v.starts_with('/') && !aws_shape && !v.contains(['+', '=']))
        || (is_path_like(v) && !aws_shape)
        || v.starts_with(['<', '{', '%'])
        || v.starts_with("./")
        || v.starts_with("../")
        || v.contains("://")
        || (password && is_setting(v))
        || v.chars().nth(1) == Some(':') && v.contains('\\')
}

/// Whether `value`, assigned to a secret-like name, looks like a secret.
fn looks_like_secret(value: &str, segs: &[String], password: bool) -> bool {
    if is_benign_value(value, segs, password) {
        return false;
    }
    let (classes, bits) = (char_classes(value), entropy(value));
    if password {
        // A human password is short and weak: any non-benign 8+ character value
        // with a digit or a symbol, or hyphenated words (a passphrase).
        value.len() >= 8
            && (has_digit(value)
                || value.bytes().any(|b| b.is_ascii_punctuation())
                || value.contains('-'))
            && bits >= 2.5
    } else {
        // A key or token is long and random, and (outside plain words and
        // camelCase code references) has a digit.
        value.len() >= 16 && bits >= 3.0 && classes >= 2 && has_digit(value)
    }
}

fn rules() -> &'static [(Regex, &'static Rule)] {
    static COMPILED: OnceLock<Vec<(Regex, &'static Rule)>> = OnceLock::new();
    COMPILED.get_or_init(|| {
        RULES
            .iter()
            .map(|r| (Regex::new(r.pattern).expect("built-in privacy pattern"), r))
            .collect()
    })
}

fn assignment() -> &'static Regex {
    static COMPILED: OnceLock<Regex> = OnceLock::new();
    COMPILED.get_or_init(|| Regex::new(ASSIGNMENT).expect("built-in privacy assignment pattern"))
}

fn private_key_begin() -> &'static Regex {
    static COMPILED: OnceLock<Regex> = OnceLock::new();
    COMPILED.get_or_init(|| {
        Regex::new(r"-----BEGIN ([A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?)-----")
            .expect("built-in privacy private key pattern")
    })
}

/// A line of a PEM body: base64, or an RFC 1421 header (`Proc-Type: 4,ENCRYPTED`).
fn is_pem_line(line: &str) -> bool {
    // A quoted mail or Markdown block prefixes each line with `>`.
    // A quoted mail, a comment or a string literal wraps each line.
    let line = line.trim_matches(|c: char| c.is_whitespace() || "\"'`+,;>#/*".contains(c));
    line.is_empty()
        || (line.len() >= 4
            && line
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b)))
        || (line.contains(": ")
            && line
                .split(": ")
                .next()
                .is_some_and(|k| k.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-')))
}

/// How far past a BEGIN line the matching END line is looked for first (a block
/// made only of key lines is read to its end).
const MAX_PEM_BYTES: usize = 16 * 1024;
/// The most a block of key lines is followed to its end.
const MAX_PEM_WIDENED_BYTES: usize = 1 << 20;
/// How much of the text after `name=` is read first as the value (one that
/// runs past it is read to its true end).
const MAX_VALUE_BYTES: usize = 4096;

/// The longest prefix of `s` that is at most `max` bytes and ends on a char
/// boundary. Bounds the work a scan does per match.
fn bounded(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Whether `body` (the text between BEGIN and END) is key material: only PEM
/// lines, at least one long base64 line. A sentence that mentions both markers
/// is not.
fn is_pem_body(body: &str) -> bool {
    // A key inside a JSON string or a log line has literal `\n`, `\r\n` or
    // `\r` for each line break.
    let body = body
        .replace("\\r\\n", "\n")
        .replace("\\n", "\n")
        .replace("\\r", "\n")
        .replace("\\/", "/");
    let base64ish = |l: &str| {
        let l = l.trim_matches(|c: char| c.is_whitespace() || "\"'`+,;>#/*".contains(c));
        l.len() >= 32
            && l.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
    };
    if body.lines().all(is_pem_line) && body.lines().any(base64ish) {
        return true;
    }
    // Fail closed: prose has no unbroken 64-character base64 run, key material
    // does, whatever else surrounds it.
    if body.split_whitespace().any(|t| {
        t.len() >= 64
            && t.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
    }) {
        return true;
    }
    // Line breaks collapsed into spaces: every token is base64 and the long
    // ones carry most of the text.
    let tokens: Vec<&str> = body.split_whitespace().collect();
    let all_b64 = tokens.iter().all(|t| {
        t.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
    });
    let total: usize = tokens.iter().map(|t| t.len()).sum();
    let long: usize = tokens
        .iter()
        .filter(|t| t.len() >= 16)
        .map(|t| t.len())
        .sum();
    !tokens.is_empty() && all_b64 && tokens.iter().any(|t| t.len() >= 32) && long * 10 >= total * 8
}

/// Replace each private key block whole. A block with its matching END line is
/// masked when its body is key material; a block that was cut off (no END) is
/// masked from BEGIN through the key lines that follow. A sentence that merely
/// mentions a BEGIN or END line is left alone.
fn mask_private_keys(text: &str, allowed: &dyn Fn(&str) -> bool, count: &mut usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for caps in private_key_begin().captures_iter(text) {
        let Some(begin) = caps.get(0) else {
            continue;
        };
        if begin.start() < last {
            continue;
        }
        let end_marker = format!("-----END {}-----", &caps[1]);
        // A key is a few KiB; do not search the rest of a huge text per BEGIN.
        let full = &text[begin.end()..];
        let mut after = bounded(full, MAX_PEM_BYTES);
        // A longer block (every line so far is key material) is read whole.
        if after.len() < full.len()
            && !after.contains(&end_marker)
            // The last line of the window may be cut mid-way.
            && after
                .rsplit_once('\n')
                .is_some_and(|(whole, _)| whole.lines().all(is_pem_line))
        {
            after = bounded(full, MAX_PEM_WIDENED_BYTES);
        }
        let end = match after.find(&end_marker) {
            Some(rel) if is_pem_body(&after[..rel]) => begin.end() + rel + end_marker.len(),
            Some(_) => continue,
            None => {
                // Cut off: key lines directly after BEGIN, up to the first line
                // that is not one.
                let mut consumed = 0;
                for line in after.split_inclusive('\n') {
                    if !is_pem_line(line) {
                        break;
                    }
                    consumed += line.len();
                }
                let body = &after[..consumed];
                if !is_pem_body(body) {
                    continue;
                }
                begin.end() + body.trim_end().len()
            }
        };
        if allowed(&text[begin.start()..end]) {
            continue;
        }
        out.push_str(&text[last..begin.start()]);
        out.push_str("[REDACTED:private-key]");
        last = end;
        *count += 1;
    }
    out.push_str(&text[last..]);
    out
}

/// The value of an assignment starting at `start`, as `(value_start, value_end)`
/// byte offsets with surrounding decoration trimmed, and the offset where the
/// scan resumes. A quoted value runs to its closing quote on the same line (so
/// it may hold spaces); an unquoted one ends at whitespace, a quote, `,` or a
/// bracket, and is cut where `&name=` or `;name=` starts the next pair.
fn assignment_value(text: &str, start: usize, sep: &str, password: bool) -> (usize, usize, usize) {
    // The window only bounds the work: a value that runs to its end is read
    // again over the whole text, so a long secret is never cut short.
    let window = bounded(&text[start..], MAX_VALUE_BYTES);
    let found = assignment_value_in(window, start, sep, password);
    if found.2 - start == window.len() && window.len() < text.len() - start {
        return assignment_value_in(&text[start..], start, sep, password);
    }
    found
}

fn assignment_value_in(
    rest: &str,
    start: usize,
    sep: &str,
    password: bool,
) -> (usize, usize, usize) {
    let quote = sep
        .trim_end()
        .chars()
        .last()
        .filter(|c| matches!(c, '"' | '\''));
    let mut end = 0;
    let mut quoted = false;
    if let Some(q) = quote {
        if let Some(close) = rest.find(['\n', q]) {
            if rest[close..].starts_with(q) {
                end = close;
                quoted = true;
            }
        }
    }
    if !quoted {
        end = rest
            .find(|c: char| {
                c.is_whitespace()
                    || matches!(c, '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>')
                    // A password may hold `,` and `\`; other values end there.
                    || (!password && matches!(c, ',' | '\\'))
            })
            .unwrap_or(rest.len());
        // `a=b&c=d`, `a=b;c=d`: the value ends where the next pair begins.
        let value = &rest[..end];
        for (i, _) in value.match_indices(['&', ';', ',']) {
            let tail = &value[i + 1..];
            let name_len = tail
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
                .unwrap_or(tail.len());
            if name_len > 0 && tail[name_len..].starts_with('=') {
                end = i;
                break;
            }
        }
    }
    let raw = &rest[..end];
    // `Value.` ends a sentence; backticks and curly quotes wrap a value.
    let left = raw.len() - raw.trim_start_matches(['`', '“', '‘']).len();
    let core = raw[left..].trim_end_matches(['.', ',', '`', '”', '’', '\\']);
    (start + left, start + left + core.len(), start + end)
}

/// Mask the value of every `NAME=value` style assignment whose name is
/// secret-like and whose value looks like a secret. Pairs that are not secret
/// are skipped one separator at a time, so a secret next to them (`MODE=prod;
/// API_KEY=...`, a URL with `?api_key=...`) is still found.
fn mask_assignments(text: &str, allowed: &dyn Fn(&str) -> bool, count: &mut usize) -> String {
    let re = assignment();
    let mut out = String::with_capacity(text.len());
    let (mut pos, mut copied) = (0, 0);
    while pos < text.len() {
        let Some(caps) = re.captures_at(text, pos) else {
            break;
        };
        let Some(whole) = caps.get(0) else {
            break;
        };
        pos = whole.end().max(pos + 1);
        let segs = name_segments(&caps[1]);
        let equals_sign = caps[2]
            .trim_matches(|c: char| c == '"' || c == '\'' || c == '\\' || c.is_whitespace())
            == "=";
        let Some(password) = secret_name_kind(&segs, equals_sign) else {
            continue;
        };
        let (vs, ve, resume) = assignment_value(text, whole.end(), &caps[2], password);
        let value = &text[vs..ve];
        if vs < copied
            || value.len() < 8
            || !looks_like_secret(value, &segs, password)
            || allowed(value)
            || allowed(&text[whole.start()..ve])
        {
            // A value read past the window is not read again from inside it.
            if resume - whole.end() > MAX_VALUE_BYTES {
                pos = pos.max(resume);
            }
            continue;
        }
        out.push_str(&text[copied..vs]);
        out.push_str("[REDACTED:env-secret]");
        copied = ve;
        pos = resume.max(ve);
        *count += 1;
    }
    out.push_str(&text[copied..]);
    out
}

/// Mask every secret found in `input`; a match that any `allowlist` regex also
/// matches is kept as it is.
pub fn redact(input: &str, allowlist: &[Regex]) -> Redaction {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let allowed = |s: &str| allowlist.iter().any(|a| a.is_match(s));

    let mut keys = 0;
    let mut text = mask_private_keys(input, &allowed, &mut keys);
    if keys > 0 {
        counts.insert("private-key", keys);
    }

    for (re, rule) in rules() {
        text = re
            .replace_all(&text, |c: &Captures| {
                if allowed(&c[0]) {
                    return c[0].to_string();
                }
                match (rule.mask)(c) {
                    Some(masked) => {
                        *counts.entry(rule.kind).or_default() += 1;
                        masked
                    }
                    None => c[0].to_string(),
                }
            })
            .into_owned();
    }

    let mut env = 0;
    text = mask_assignments(&text, &allowed, &mut env);
    if env > 0 {
        *counts.entry("env-secret").or_default() += env;
    }

    Redaction {
        text,
        redactions: counts
            .into_iter()
            .map(|(kind, count)| RedactionCount { kind, count })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake secrets are assembled at run time so the repository itself holds no
    /// string a secret scanner would flag.
    fn gh() -> String {
        format!("{}{}", "ghp_", "aB3dE5gH7jK9mN1pQ3rS5tU7vW9xY1zA3bC5")
    }
    fn aws() -> String {
        format!("{}{}", "AKIA", "IOSFODNN7EXAMPLQ")
    }
    fn jwt() -> String {
        [
            "eyJhbGciOiJIUzI1NiJ9",
            "eyJzdWIiOiIxMjM0NTY3ODkwIn0",
            "dBjftJeZ4CVPmB92K27uhbUJU1p1r",
        ]
        .join(".")
    }
    fn pem() -> String {
        format!(
            "-----BEGIN {} KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC\nabcdefgh\n-----END {} KEY-----",
            "RSA PRIVATE", "RSA PRIVATE"
        )
    }

    fn sample() -> String {
        ["Zq8vN2xL5mP9", "aR4tW7yB3cD6"].concat()
    }

    fn kinds(r: &Redaction) -> Vec<(&'static str, usize)> {
        r.redactions.iter().map(|x| (x.kind, x.count)).collect()
    }

    #[test]
    fn github_tokens_keep_their_prefix() {
        let r = redact(&format!("use {} for the repo", gh()), &[]);
        assert_eq!(r.text, "use ghp_[REDACTED:github-token] for the repo");
        assert_eq!(kinds(&r), vec![("github-token", 1)]);
        let pat = format!(
            "{}{}",
            "github_pat_", "11ABCDEFG0abcdefghij_klmnopqrstuvwxyz0123456789"
        );
        let r = redact(&pat, &[]);
        assert_eq!(r.text, "github_pat_[REDACTED:github-token]");
    }

    #[test]
    fn aws_access_keys_keep_their_prefix() {
        let r = redact(&format!("key id {}", aws()), &[]);
        assert_eq!(r.text, "key id AKIA[REDACTED:aws-access-key]");
        assert_eq!(kinds(&r), vec![("aws-access-key", 1)]);
        // Wrong length is not a key.
        assert!(redact("AKIAshort", &[]).is_clean());
    }

    #[test]
    fn bearer_headers_and_jwts() {
        let r = redact(&format!("Authorization: Bearer {}", jwt()), &[]);
        assert_eq!(r.text, "Authorization: Bearer [REDACTED:bearer-token]");
        let r = redact(&format!("cookie {}", jwt()), &[]);
        assert_eq!(r.text, "cookie [REDACTED:jwt]");
        assert!(redact("Bearer token handling is documented", &[]).is_clean());
    }

    #[test]
    fn private_key_blocks_are_removed_whole() {
        let r = redact(&format!("before\n{}\nafter", pem()), &[]);
        assert_eq!(r.text, "before\n[REDACTED:private-key]\nafter");
        assert_eq!(kinds(&r), vec![("private-key", 1)]);
        assert!(
            redact(
                "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----",
                &[]
            )
            .is_clean()
        );
    }

    #[test]
    fn api_keys_with_the_sk_prefix() {
        let sample = format!("{}{}", "sk-", "proj-Ab3dE5gH7jK9mN1pQ3rS5tU7");
        let r = redact(&sample, &[]);
        assert_eq!(r.text, "sk-[REDACTED:api-key]");
        assert!(redact("sk-short", &[]).is_clean());
        assert!(redact("task-description-for-the-weekly-sync-meeting", &[]).is_clean());
    }

    #[test]
    fn secret_named_assignments_with_a_high_entropy_value() {
        let value = sample();
        for (input, expected) in [
            (
                format!("API_KEY={value}"),
                "API_KEY=[REDACTED:env-secret]".to_string(),
            ),
            (
                format!("export DB_PASSWORD=\"{value}\""),
                "export DB_PASSWORD=\"[REDACTED:env-secret]".to_string() + "\"",
            ),
            (
                format!("{{\"client_secret\": \"{value}\"}}"),
                "{\"client_secret\": \"[REDACTED:env-secret]\"}".to_string(),
            ),
            (
                format!("auth_token: {value}"),
                "auth_token: [REDACTED:env-secret]".to_string(),
            ),
        ] {
            let r = redact(&input, &[]);
            assert_eq!(r.text, expected, "{input}");
            assert_eq!(kinds(&r), vec![("env-secret", 1)], "{input}");
        }
    }

    #[test]
    fn assignments_that_are_not_secrets_are_left_alone() {
        for input in [
            "TOKEN_LIMIT=10000",
            "PASSWORD_MIN_LENGTH=12",
            "API_KEY=short",
            "KEY_FILE=/etc/ssl/private/server.key",
            "SECRET_PATH=./config/secret.yaml",
            "SSH_KEY=~/.ssh/id_ed25519_work_laptop",
            "API_KEY=${VAULT_API_KEY_PRODUCTION_ENV}",
            "api_key=<your-api-key-goes-here-ok>",
            "API_KEY=your_api_key_goes_here_please",
            "TOKEN=aaaaaaaaaaaaaaaaaaaaaaaa",
            "COMMIT_KEY=3f786850e387550fdab836ed7e6dc881de23001b",
            "CACHE_KEY=2f9c1d3e-8a4b-4c6d-9e1f-0a1b2c3d4e5f",
            "TOKEN_URL=https://auth.example.com/oauth/token/endpoint",
            "keyboard shortcuts: press the key combination again to toggle",
        ] {
            let r = redact(input, &[]);
            assert!(r.is_clean(), "false positive on {input:?}: {}", r.text);
            assert_eq!(r.text, input);
        }
    }

    #[test]
    fn the_allowlist_keeps_a_matching_value() {
        let allow = compile_allowlist(&["^ghp_aB3".to_string()]).unwrap();
        let input = format!("token {}", gh());
        let r = redact(&input, &allow);
        assert!(r.is_clean());
        assert_eq!(r.text, input);
        // An allowlist entry that does not match changes nothing.
        let other = compile_allowlist(&["^nothing$".to_string()]).unwrap();
        assert_eq!(redact(&input, &other).total(), 1);
        assert!(compile_allowlist(&["(unclosed".to_string()]).is_err());
    }

    #[test]
    fn counts_are_per_kind_and_the_filter_is_idempotent() {
        let input = format!(
            "{} and {} and {} plus API_KEY={}",
            gh(),
            gh(),
            aws(),
            sample()
        );
        let once = redact(&input, &[]);
        assert_eq!(
            kinds(&once),
            vec![
                ("aws-access-key", 1),
                ("env-secret", 1),
                ("github-token", 2)
            ]
        );
        assert_eq!(once.total(), 4);
        let twice = redact(&once.text, &[]);
        assert!(twice.is_clean(), "second pass changed {}", twice.text);
        assert_eq!(twice.text, once.text);
    }

    #[test]
    fn the_secret_value_does_not_survive_in_the_output() {
        let input = format!("{} {} {} {}", gh(), aws(), jwt(), pem());
        let r = redact(&input, &[]);
        for secret in [
            gh(),
            aws(),
            jwt(),
            "MIIEvQIBADANBgkqhkiG9w0BAQEFAASC".to_string(),
        ] {
            assert!(!r.text.contains(&secret), "{secret} survived: {}", r.text);
        }
    }

    /// `docs/privacy-filter-cases.json` is the shared test-vector file uteke-cloud
    /// pins parity against (uteke-cloud#218): every case must hold here too.
    #[test]
    fn the_shared_cases_file_matches_the_implementation() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/privacy-filter-cases.json"
        );
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).expect("cases file")).unwrap();
        assert_eq!(doc["version"], 3);
        let cases = doc["cases"].as_array().unwrap();
        assert!(cases.len() >= 150, "only {} cases", cases.len());
        let mut masked = 0;
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let input: String = case["input"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_str().unwrap())
                .collect();
            let r = redact(&input, &[]);
            // A case without `expected` must come back unchanged.
            let expected_text = case["expected"].as_str().unwrap_or(&input);
            assert_eq!(r.text, expected_text, "{name}");
            let kinds: Vec<(String, u64)> = r
                .redactions
                .iter()
                .map(|x| (x.kind.to_string(), x.count as u64))
                .collect();
            let expected: Vec<(String, u64)> = case["kinds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|k| (k[0].as_str().unwrap().to_string(), k[1].as_u64().unwrap()))
                .collect();
            assert_eq!(kinds, expected, "{name}");
            if !expected.is_empty() {
                masked += 1;
            }
            // The expected text is a fixed point of the filter.
            assert!(redact(&r.text, &[]).is_clean(), "{name}: not idempotent");
        }
        assert!(
            masked >= 10,
            "the file must exercise every kind, not only clean text"
        );
    }

    #[test]
    fn names_are_split_into_segments_and_must_end_in_a_secret_word() {
        let segs = |n: &str| name_segments(n);
        assert_eq!(segs("GITHUB_TOKEN"), vec!["github", "token"]);
        assert_eq!(segs("apiKey"), vec!["api", "key"]);
        assert_eq!(segs("db.password"), vec!["db", "password"]);
        // (name, separator is `=`, expected: Some(is_password) or None)
        for (name, eq, expected) in [
            ("API_KEY", false, Some(false)),
            ("auth-token", false, Some(false)),
            ("clientSecret", false, Some(false)),
            ("API_KEY_V2", false, Some(false)),
            ("SECRET_KEY_BASE", false, Some(false)),
            ("REDIS_AUTH", false, Some(false)),
            ("AZURE_COSMOS_PRIMARY_KEY", false, Some(false)),
            ("DB_PASSWORD", false, Some(true)),
            ("DB_PASSWORD_PROD", false, Some(true)),
            ("DB_PASS", false, Some(true)),
            ("PGPASSWORD", false, Some(true)),
            ("password", false, Some(true)),
            ("passphrase", false, Some(true)),
            ("credentials", false, Some(false)),
            // A bare generic word is an ordinary word, except with `=`.
            ("key", false, None),
            ("token", false, None),
            ("secret", false, None),
            ("pass", false, None),
            ("PASS", true, None),
            ("key", true, Some(false)),
            ("token", true, Some(false)),
            ("secret", true, Some(false)),
            // Not a name that ENDS in a secret word.
            ("monkey", true, None),
            ("Tokenizer", true, None),
            ("Secretary", false, None),
            ("keyword", true, None),
            ("token_expires_at", false, None),
            ("key_contact", false, None),
            ("PASSWORD_MIN_LENGTH", false, None),
            // A qualifier that makes it something else.
            ("public_key", false, None),
            ("cache_key", false, None),
            ("next_token", false, None),
            ("page_token", false, None),
            ("idempotency_key", false, None),
        ] {
            assert_eq!(
                secret_name_kind(&name_segments(name), eq),
                expected,
                "{name} (equals: {eq})"
            );
        }
    }

    #[test]
    fn entropy_matches_known_values() {
        assert_eq!(entropy("aaaa"), 0.0);
        assert!((entropy("abab") - 1.0).abs() < 1e-9);
        assert!((entropy("abcd") - 2.0).abs() < 1e-9);
    }

    fn cfg(mode: PrivacyMode) -> PrivacyConfig {
        PrivacyConfig {
            mode,
            allowlist: Vec::new(),
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
    fn a_key_with_stray_text_is_still_masked_and_hostile_input_stays_fast() {
        let (b, e) = (
            format!("-----BEGIN {} KEY-----", "RSA PRIVATE"),
            format!("-----END {} KEY-----", "RSA PRIVATE"),
        );
        let run = "MIIEowIBAAKCAQEAu1SU1LfVLPHCozMxH2Mo4lgOEePzNm0tRgeLezV6ffAt0gun";
        // Escaped slashes and stray words around the base64 run.
        let t = format!("{b} note: {run}\\/{run} {e}");
        assert_eq!(redact(&t, &[]).text, "[REDACTED:private-key]");
        // A sentence naming both markers stays.
        let prose = format!("starts with {b} and ends with {e} markers");
        assert_eq!(redact(&prose, &[]).text, prose);

        // Many candidates in one long token must not take quadratic time.
        let many = "password=".repeat(20_000);
        let started = std::time::Instant::now();
        let _ = redact(&many, &[]);
        let many_begin = format!("{b} ").repeat(5_000);
        let _ = redact(&many_begin, &[]);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "scan took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn values_and_keys_longer_than_the_scan_windows_are_masked_whole() {
        let long = "Ab3dE5gH7jK9".repeat(1_000); // 12 KB, no separator
        let out = redact(&format!("password={long} next"), &[]).text;
        assert_eq!(out, "password=[REDACTED:env-secret] next");

        let (b, e) = (
            format!("-----BEGIN {} KEY-----", "RSA PRIVATE"),
            format!("-----END {} KEY-----", "RSA PRIVATE"),
        );
        let line = "MIIEowIBAAKCAQEAu1SU1LfVLPHCozMxH2Mo4lgOEePzNm0tRgeLezV6ffAt0gun";
        let body = format!("{line}\n").repeat(600); // ~39 KB
        let out = redact(&format!("a\n{b}\n{body}{e}\nz"), &[]).text;
        assert_eq!(out, "a\n[REDACTED:private-key]\nz");
    }

    #[test]
    fn a_key_wrapped_in_string_literals_or_comments_is_masked() {
        let (b, e) = (
            format!("-----BEGIN {} KEY-----", "RSA PRIVATE"),
            format!("-----END {} KEY-----", "RSA PRIVATE"),
        );
        let run = "MIIEowIBAAKCAQEAu1SU1LfVLPHCozMxH2Mo4lg";
        let java = format!("String k = \"{b}\\n\" +\n  \"{run}\\n\" +\n  \"{e}\";");
        assert_eq!(
            redact(&java, &[]).text,
            "String k = \"[REDACTED:private-key]\";"
        );
        let comment = format!("# {b}\n# {run}\n# {e}");
        assert_eq!(redact(&comment, &[]).text, "# [REDACTED:private-key]");
    }
}
