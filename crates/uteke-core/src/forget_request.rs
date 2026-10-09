//! Typed `forget` request shared by the HTTP server, the MCP server and the
//! CLI-via-server client (#1343, `forget` slice).
//!
//! Each surface used to parse the target (id or tag), validate and resolve
//! the id (full UUID or short prefix) and decide what "not found" means on
//! its own. This module owns that once: a surface decodes its wire format
//! into a [`ForgetInput`], picks a [`ForgetPolicy`], calls
//! [`ForgetRequest::decode`] and, for an id target, [`ForgetRequest::resolve`].
//!
//! This is an extraction, not a unification. Where the surfaces disagree
//! today the disagreement is a [`ForgetPolicy`] value, not a decision:
//!
//! | aspect                       | HTTP `DELETE /forget`                 | MCP `uteke_forget`                  | CLI via server               |
//! |------------------------------|---------------------------------------|-------------------------------------|------------------------------|
//! | id charset check             | none (`%`/`_` reach the LIKE scan)    | hex digits and `-` only (#1328)     | what the user typed is sent  |
//! | full UUID                    | kept verbatim (case, form)            | normalised to hyphenated lowercase  | verbatim, server decides     |
//! | unknown prefix               | 404 `Memory not found: {id}`          | error `No memory matches id prefix '{id}'` | server's 404          |
//! | full UUID that is not stored | 404 (existence check, #762)           | core is called directly             | server's 404                 |
//! | ambiguous prefix             | 400, core text                        | error, same core text               | server's 400                 |
//! | tag target                   | `?tag=` (+ optional `namespace`)      | not offered                         | `--tag`, namespace `default` unless `--namespace` |
//! | namespace for an id          | ignored                               | ignored                             | not sent                     |
//! | tag without namespace        | default namespace only (core)         | n/a                                 | n/a (always sent)            |
//! | both `id` and `tag`          | `id` wins                             | n/a                                 | `id` wins                    |
//! | neither                      | 400 `Provide ?id= or ?tag= parameter` | error `Missing 'id'`                | local usage error            |
//! | read-only token              | blocked (403, #409)                   | NOT gated                           | server decides               |
//! | confirmation prompt          | none                                  | none                                | local forget prompts, via-server does not |
//!
//! The read-only token gate lives in the HTTP request pipeline (a method and
//! path check before any handler runs), so it cannot be a decode-time rule;
//! it is recorded as [`ForgetPolicy::read_only_token_may_forget`] so the
//! disagreement is visible and pinned by tests.

use crate::Uteke;
use std::fmt;

/// What a surface accepts as a memory id before it touches the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdCharset {
    /// Anything: a non-UUID value goes straight to the prefix scan, where
    /// `%` and `_` act as `LIKE` wildcards (HTTP today).
    Any,
    /// Hex digits and `-` only (MCP, #1328); keeps wildcards out of the scan.
    HexAndDash,
}

/// How a full, parseable UUID is passed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullIdForm {
    /// Exactly the string the caller sent (HTTP today).
    Verbatim,
    /// Lowercase hyphenated, whatever form was sent (MCP today).
    Hyphenated,
}

/// The wording of "this prefix matches nothing".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnmatchedPrefixText {
    /// `Memory not found: {id}`
    MemoryNotFound,
    /// `No memory matches id prefix '{id}'`
    NoPrefixMatch,
}

/// Where today's surfaces disagree, per surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForgetPolicy {
    pub id_charset: IdCharset,
    pub full_id_form: FullIdForm,
    pub unmatched_prefix_text: UnmatchedPrefixText,
    /// Look the resolved id up before deleting and report "not found" when
    /// it is absent (#762). Without it a full UUID goes straight to core.
    pub require_existing: bool,
    /// Message when the request names neither an id nor a tag.
    pub missing_target_text: &'static str,
    /// Whether a read-only token may forget. Documentation only: the gate is
    /// applied by the HTTP pipeline, not by decode.
    pub read_only_token_may_forget: bool,
}

impl ForgetPolicy {
    /// `DELETE /forget`.
    pub const HTTP: Self = Self {
        id_charset: IdCharset::Any,
        full_id_form: FullIdForm::Verbatim,
        unmatched_prefix_text: UnmatchedPrefixText::MemoryNotFound,
        require_existing: true,
        missing_target_text: "Provide ?id= or ?tag= parameter",
        read_only_token_may_forget: false,
    };
    /// MCP `uteke_forget`.
    pub const MCP: Self = Self {
        id_charset: IdCharset::HexAndDash,
        full_id_form: FullIdForm::Hyphenated,
        unmatched_prefix_text: UnmatchedPrefixText::NoPrefixMatch,
        require_existing: false,
        missing_target_text: "Missing 'id'",
        read_only_token_may_forget: true,
    };
}

/// Raw, undecoded forget arguments as a surface read them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForgetInput {
    pub id: Option<String>,
    pub tag: Option<String>,
    /// Only meaningful with `tag`; an id target ignores it.
    pub namespace: Option<String>,
}

impl ForgetInput {
    /// Parse the query string of `DELETE /forget` (the part after `?`).
    ///
    /// Same rules the handler always had: `&`-separated `key=value` pairs, a
    /// pair without `=` is dropped, the last duplicate wins, and values are
    /// NOT percent-decoded.
    pub fn from_http_query(query: &str) -> Self {
        let mut input = Self::default();
        for pair in query.split('&') {
            let mut kv = pair.splitn(2, '=');
            let (Some(key), Some(value)) = (kv.next(), kv.next()) else {
                continue;
            };
            match key {
                "id" => input.id = Some(value.to_string()),
                "tag" => input.tag = Some(value.to_string()),
                "namespace" => input.namespace = Some(value.to_string()),
                _ => {}
            }
        }
        input
    }

    /// The path-and-query of `DELETE /forget`, for clients that call the
    /// server (CLI via server). `encode` percent-encodes one value. An id
    /// wins over a tag, as on the server; `None` when neither is set.
    /// Nothing is validated or rewritten here: the server owns that.
    pub fn to_http_path(&self, encode: impl Fn(&str) -> String) -> Option<String> {
        if let Some(id) = &self.id {
            Some(format!("/forget?id={}", encode(id)))
        } else {
            let tag = self.tag.as_ref()?;
            let mut path = format!("/forget?tag={}", encode(tag));
            if let Some(ns) = &self.namespace {
                path.push_str(&format!("&namespace={}", encode(ns)));
            }
            Some(path)
        }
    }
}

/// Why a forget request failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgetErrorKind {
    /// No id and no tag (HTTP 400).
    MissingTarget,
    /// The id failed the policy's charset check.
    InvalidId,
    /// No memory matches (HTTP 404).
    NotFound,
    /// The prefix lookup itself failed, e.g. ambiguous (HTTP 400).
    Lookup,
}

/// A decode or resolution failure. The text is the exact message the
/// surfaces return; [`ForgetErrorKind`] says which HTTP status it maps to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgetRequestError {
    kind: ForgetErrorKind,
    message: String,
}

impl ForgetRequestError {
    pub fn kind(&self) -> ForgetErrorKind {
        self.kind
    }
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ForgetRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ForgetRequestError {}

/// A decoded forget request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForgetRequest {
    /// One memory, by full id or prefix; not yet resolved.
    Id(String),
    /// Every memory with the tag; `namespace: None` means the default
    /// namespace (core `DEFAULT_NAMESPACE`), not every namespace.
    Tag {
        tag: String,
        namespace: Option<String>,
    },
}

impl ForgetRequest {
    /// Decode `input`: an id wins over a tag, and neither is an error.
    pub fn decode(input: ForgetInput, policy: ForgetPolicy) -> Result<Self, ForgetRequestError> {
        if let Some(id) = input.id {
            Ok(Self::Id(id))
        } else if let Some(tag) = input.tag {
            Ok(Self::Tag {
                tag,
                namespace: input.namespace,
            })
        } else {
            Err(ForgetRequestError {
                kind: ForgetErrorKind::MissingTarget,
                message: policy.missing_target_text.to_string(),
            })
        }
    }

    /// Resolve an id target to the full id to delete (`Ok(None)` for a tag
    /// target, which has no single id).
    pub fn resolve(
        &self,
        uteke: &Uteke,
        policy: ForgetPolicy,
    ) -> Result<Option<String>, ForgetRequestError> {
        match self {
            Self::Id(id) => resolve_memory_id(uteke, id, policy).map(Some),
            Self::Tag { .. } => Ok(None),
        }
    }
}

/// Resolve a full UUID or id prefix to the id to act on, under `policy`.
///
/// Order, as every surface always did it: charset check, full-UUID shortcut,
/// prefix scan (unknown and ambiguous prefixes error), then the optional
/// existence check.
pub fn resolve_memory_id(
    uteke: &Uteke,
    id: &str,
    policy: ForgetPolicy,
) -> Result<String, ForgetRequestError> {
    if policy.id_charset == IdCharset::HexAndDash
        && (id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
    {
        return Err(ForgetRequestError {
            kind: ForgetErrorKind::InvalidId,
            message: format!("Invalid memory id '{id}' (expected a UUID or hex prefix)"),
        });
    }
    // A well-formed full UUID skips the prefix scan; anything else (including
    // a 36-char non-UUID string) takes the prefix path.
    let resolved = match uuid::Uuid::parse_str(id) {
        Ok(uuid) => match policy.full_id_form {
            FullIdForm::Verbatim => id.to_string(),
            FullIdForm::Hyphenated => uuid.hyphenated().to_string(),
        },
        Err(_) => match uteke.resolve_id_prefix(id) {
            Ok(Some(full)) => full,
            Ok(None) => {
                return Err(ForgetRequestError {
                    kind: ForgetErrorKind::NotFound,
                    message: match policy.unmatched_prefix_text {
                        UnmatchedPrefixText::MemoryNotFound => format!("Memory not found: {id}"),
                        UnmatchedPrefixText::NoPrefixMatch => {
                            format!("No memory matches id prefix '{id}'")
                        }
                    },
                });
            }
            Err(e) => {
                return Err(ForgetRequestError {
                    kind: ForgetErrorKind::Lookup,
                    message: e.to_string(),
                });
            }
        },
    };
    if policy.require_existing && uteke.get_by_id(&resolved).ok().flatten().is_none() {
        return Err(ForgetRequestError {
            kind: ForgetErrorKind::NotFound,
            message: format!("Memory not found: {id}"),
        });
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::Memory;

    fn scratch() -> (Uteke, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "forget-req-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let uteke = Uteke::open(dir.join("t.db").to_str().unwrap()).unwrap();
        (uteke, dir)
    }

    fn seed_with(uteke: &Uteke, id: &str, ns: &str) {
        let now = chrono::Utc::now();
        let m = Memory {
            id: id.to_string(),
            content: "forget request probe".to_string(),
            embedding: vec![0.21; 768],
            tags: vec!["probe".to_string()],
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
            namespace: ns.to_string(),
            access_count: 0,
            recall_count: 0,
            last_accessed: None,
            deprecated: false,
            deprecated_at: None,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
            importance: 0.5,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
            source: None,
            source_type: "user".to_string(),
            author_type: "agent".to_string(),
        };
        uteke.store().insert(&m).unwrap();
    }

    fn input(id: Option<&str>, tag: Option<&str>, ns: Option<&str>) -> ForgetInput {
        ForgetInput {
            id: id.map(String::from),
            tag: tag.map(String::from),
            namespace: ns.map(String::from),
        }
    }

    #[test]
    fn query_parsing_keeps_the_old_handler_rules() {
        assert_eq!(
            ForgetInput::from_http_query("id=abc"),
            input(Some("abc"), None, None)
        );
        assert_eq!(
            ForgetInput::from_http_query("tag=t&namespace=n"),
            input(None, Some("t"), Some("n"))
        );
        // Last duplicate wins, a pair without '=' is dropped, unknown keys
        // are ignored, values are not percent-decoded, '=' stays in values.
        assert_eq!(
            ForgetInput::from_http_query("id=a&id=b&flag&x=1&tag=a%20b=c"),
            input(Some("b"), Some("a%20b=c"), None)
        );
        assert_eq!(ForgetInput::from_http_query(""), ForgetInput::default());
        // An empty value is still a value.
        assert_eq!(
            ForgetInput::from_http_query("id="),
            input(Some(""), None, None)
        );
    }

    #[test]
    fn id_wins_over_tag_and_neither_is_an_error() {
        for p in [ForgetPolicy::HTTP, ForgetPolicy::MCP] {
            assert_eq!(
                ForgetRequest::decode(input(Some("a"), Some("t"), Some("n")), p).unwrap(),
                ForgetRequest::Id("a".into())
            );
        }
        assert_eq!(
            ForgetRequest::decode(input(None, Some("t"), None), ForgetPolicy::HTTP).unwrap(),
            ForgetRequest::Tag {
                tag: "t".into(),
                namespace: None
            }
        );
        let e =
            ForgetRequest::decode(input(None, None, Some("n")), ForgetPolicy::HTTP).unwrap_err();
        assert_eq!(e.message(), "Provide ?id= or ?tag= parameter");
        assert_eq!(e.kind(), ForgetErrorKind::MissingTarget);
        let e = ForgetRequest::decode(input(None, None, None), ForgetPolicy::MCP).unwrap_err();
        assert_eq!(e.message(), "Missing 'id'");
    }

    #[test]
    fn http_path_encoding_matches_the_old_cli_urls() {
        let enc = |s: &str| s.replace(' ', "%20");
        assert_eq!(
            input(Some("ab 1"), Some("t"), Some("n")).to_http_path(enc),
            Some("/forget?id=ab%201".to_string())
        );
        assert_eq!(
            input(None, Some("my tag"), Some("work")).to_http_path(enc),
            Some("/forget?tag=my%20tag&namespace=work".to_string())
        );
        assert_eq!(input(None, None, Some("n")).to_http_path(enc), None);
    }

    #[test]
    fn policies_encode_the_known_disagreements() {
        let flags = |p: ForgetPolicy| (p.read_only_token_may_forget, p.require_existing);
        assert_eq!(flags(ForgetPolicy::HTTP), (false, true));
        assert_eq!(flags(ForgetPolicy::MCP), (true, false));
        assert_eq!(ForgetPolicy::HTTP.id_charset, IdCharset::Any);
        assert_eq!(ForgetPolicy::HTTP.full_id_form, FullIdForm::Verbatim);
        assert_eq!(ForgetPolicy::MCP.id_charset, IdCharset::HexAndDash);
        assert_eq!(ForgetPolicy::MCP.full_id_form, FullIdForm::Hyphenated);
    }

    #[test]
    fn resolution_per_policy() {
        let (uteke, dir) = scratch();
        let id = uuid::Uuid::new_v4().to_string();
        seed_with(&uteke, &id, "ns-a");
        let short: String = id.chars().take(8).collect();
        let upper = id.to_uppercase();
        let simple = id.replace('-', "");
        let absent = uuid::Uuid::new_v4().to_string();

        for p in [ForgetPolicy::HTTP, ForgetPolicy::MCP] {
            assert_eq!(resolve_memory_id(&uteke, &id, p).unwrap(), id);
            assert_eq!(resolve_memory_id(&uteke, &short, p).unwrap(), id);
        }
        // Full-UUID form: MCP normalises, HTTP keeps it verbatim and lets the
        // existence check decide.
        assert_eq!(
            resolve_memory_id(&uteke, &simple, ForgetPolicy::MCP).unwrap(),
            id
        );
        assert_eq!(
            resolve_memory_id(&uteke, &upper, ForgetPolicy::MCP).unwrap(),
            id
        );
        let e = resolve_memory_id(&uteke, &simple, ForgetPolicy::HTTP).unwrap_err();
        assert_eq!(e.kind(), ForgetErrorKind::NotFound);
        assert_eq!(e.message(), format!("Memory not found: {simple}"));
        // Absent full UUID: HTTP 404, MCP passes it on to core.
        let e = resolve_memory_id(&uteke, &absent, ForgetPolicy::HTTP).unwrap_err();
        assert_eq!(e.message(), format!("Memory not found: {absent}"));
        assert_eq!(
            resolve_memory_id(&uteke, &absent, ForgetPolicy::MCP).unwrap(),
            absent
        );
        // Unknown prefix wording.
        let e = resolve_memory_id(&uteke, "ffffffff", ForgetPolicy::HTTP).unwrap_err();
        assert_eq!(
            (e.kind(), e.message()),
            (ForgetErrorKind::NotFound, "Memory not found: ffffffff")
        );
        let e = resolve_memory_id(&uteke, "ffffffff", ForgetPolicy::MCP).unwrap_err();
        assert_eq!(
            (e.kind(), e.message()),
            (
                ForgetErrorKind::NotFound,
                "No memory matches id prefix 'ffffffff'"
            )
        );
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn charset_check_is_mcp_only() {
        let (uteke, dir) = scratch();
        let id = uuid::Uuid::new_v4().to_string();
        seed_with(&uteke, &id, "ns-a");
        for bad in ["", "zz", "%", "a_b", "abc%"] {
            let e = resolve_memory_id(&uteke, bad, ForgetPolicy::MCP).unwrap_err();
            assert_eq!(e.kind(), ForgetErrorKind::InvalidId, "{bad:?}");
            assert_eq!(
                e.message(),
                format!("Invalid memory id '{bad}' (expected a UUID or hex prefix)")
            );
        }
        // HTTP lets the wildcard reach the scan: with exactly one memory,
        // `%` resolves to it (pinned, not endorsed: owner decision).
        assert_eq!(
            resolve_memory_id(&uteke, "%", ForgetPolicy::HTTP).unwrap(),
            id
        );
        // `zz` is just an unknown prefix there.
        let e = resolve_memory_id(&uteke, "zz", ForgetPolicy::HTTP).unwrap_err();
        assert_eq!(e.kind(), ForgetErrorKind::NotFound);
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ambiguous_prefix_has_the_same_text_on_both_policies() {
        let (uteke, dir) = scratch();
        seed_with(&uteke, "abcd0000-0000-4000-8000-000000000001", "n");
        seed_with(&uteke, "abcd0000-0000-4000-8000-000000000002", "n");
        let texts: Vec<String> = [ForgetPolicy::HTTP, ForgetPolicy::MCP]
            .into_iter()
            .map(|p| {
                let e = resolve_memory_id(&uteke, "abcd", p).unwrap_err();
                assert_eq!(e.kind(), ForgetErrorKind::Lookup);
                e.message().to_string()
            })
            .collect();
        assert_eq!(texts[0], texts[1]);
        assert_eq!(
            texts[0],
            "Validation error: Ambiguous ID prefix 'abcd' — matches 2 memories. Use a longer prefix."
        );
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tag_target_has_no_single_id() {
        let (uteke, dir) = scratch();
        let req = ForgetRequest::Tag {
            tag: "t".into(),
            namespace: None,
        };
        assert_eq!(req.resolve(&uteke, ForgetPolicy::HTTP).unwrap(), None);
        let by_id = ForgetRequest::Id("ffffffff".into());
        assert!(by_id.resolve(&uteke, ForgetPolicy::HTTP).is_err());
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }
}
