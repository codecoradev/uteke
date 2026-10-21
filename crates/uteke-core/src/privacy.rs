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

/// `NAME=value`, `NAME: value`, `"NAME": "value"` with a secret-like name. The
/// value ends at whitespace, a quote, `,` or `;` (captured, so code such as
/// `self.api_key.clone()` or `key = make_key(x)` never matches: the value is
/// followed by `(`).
const ASSIGNMENT: &str = r#"(?im)\b([A-Za-z0-9_.-]*(?:key|token|secret|passw(?:or)?d|pwd)[A-Za-z0-9_.-]*)(["']?\s*[:=]\s*["']?)([^\s"',;()\[\]{}<>]{16,})(["',;\s]|$)"#;

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

/// Words joined by `.`, `_` or `-`, all lowercase or all uppercase and without
/// digits: a variable, field path or setting name (`config.embedding.api_key`,
/// `VAULT_API_KEY_NAME`), not a secret.
fn is_identifier_like(v: &str) -> bool {
    let words: Vec<&str> = v.split(['.', '_', '-']).collect();
    words.len() > 1
        && words.iter().all(|w| {
            w.len() > 1
                && (w.bytes().all(|b| b.is_ascii_lowercase())
                    || w.bytes().all(|b| b.is_ascii_uppercase()))
        })
}

/// A value an assignment rule must leave alone: hashes, UUIDs, paths, URLs,
/// environment references, placeholders and already-masked text.
fn is_benign_value(v: &str) -> bool {
    let lower = v.to_ascii_lowercase();
    v.contains("[REDACTED")
        || v.contains("::")
        || is_identifier_like(v)
        || is_uuid(v)
        || (is_hex(v) && matches!(v.len(), 40 | 64))
        || v.starts_with(['/', '~', '$', '<', '{', '%'])
        || v.starts_with("./")
        || v.starts_with("../")
        || v.contains("://")
        || v.chars().nth(1) == Some(':') && v.contains('\\')
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
        ]
        .iter()
        .any(|w| lower.contains(w))
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
            after = full;
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

    text = assignment()
        .replace_all(&text, |c: &Captures| {
            let value = &c[3];
            if is_benign_value(value) || entropy(value) < 3.0 || allowed(value) || allowed(&c[0]) {
                return c[0].to_string();
            }
            *counts.entry("env-secret").or_default() += 1;
            format!("{}{}[REDACTED:env-secret]{}", &c[1], &c[2], &c[4])
        })
        .into_owned();

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
        assert!(cases.len() >= 100, "only {} cases", cases.len());
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
            masked >= 40,
            "the file must exercise every kind, not only clean text"
        );
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
