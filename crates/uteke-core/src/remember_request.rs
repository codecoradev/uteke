//! Typed `remember` request shared by the HTTP server, the MCP server and the
//! CLI-via-server client (#1343, `remember` slice).
//!
//! Same shape as [`crate::recall_request`]: a surface decodes its wire format
//! into a [`RememberInput`] (raw), picks a [`RememberPolicy`], and calls
//! [`RememberRequest::decode`]. This is an extraction, not a unification:
//! where the surfaces disagree today the disagreement is a policy value.
//!
//! | aspect                    | HTTP `/remember`            | MCP `uteke_remember`        | CLI via server            |
//! |---------------------------|-----------------------------|-----------------------------|---------------------------|
//! | content/tags validation   | at decode (400)             | at decode (#1439)           | server validates          |
//! | `type` omitted            | auto-inference              | auto-inference (#1439)      | omitted when empty        |
//! | unknown `type`            | 400 at decode               | error at decode (#1439)     | server 400                |
//! | `type` in metadata        | copied as `"type"`          | copied as `"type"` (#1439)  | copied (server decodes)   |
//! | metadata object           | merged, plus entity/etc.    | not accepted (always none)  | built from flags, sent    |
//! | entity/category fields    | honoured (into metadata)    | not accepted                | folded into `metadata`    |
//! | valid_from/valid_until    | honoured (into metadata)    | not accepted                | not sent                  |
//! | detect_contradiction      | honoured                    | not accepted                | forwarded when set        |
//! | source/source_type/author_type | honoured               | not accepted                | forwarded                 |
//! | author                    | honoured, validated (1-128) | honoured, also room author  | forwarded                 |
//! | room                      | rejected as unknown key     | routes to room write        | refused client-side       |
//! | timestamp                 | rejected as unknown key     | not accepted                | applied to the content    |
//! | namespace omitted         | core default                | core default                | `"default"` always sent   |
//!
//! Room remember is a separate operation and is not part of this module.

use crate::memory::types::{MemoryType, validate_author, validate_author_type};
use serde_json::{Map, Value};
use std::fmt;

/// Where today's surfaces disagree, per surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RememberPolicy {
    /// Validate content and tags when decoding (HTTP). Otherwise the core
    /// write validates them and the error carries the caller's own prefix.
    pub validate_content_and_tags: bool,
    /// Validate an explicit `type` when decoding (HTTP).
    pub validate_type: bool,
    /// Type used when the request gives none. `None` leaves it to the core's
    /// auto-inference.
    pub default_type: Option<&'static str>,
    /// Copy an explicit `type` into the metadata map (HTTP).
    pub type_in_metadata: bool,
}

impl RememberPolicy {
    /// `POST /remember`.
    pub const HTTP: Self = Self {
        validate_content_and_tags: true,
        validate_type: true,
        default_type: None,
        type_in_metadata: true,
    };
    /// MCP `uteke_remember`: the HTTP rules (#1439).
    pub const MCP: Self = Self::HTTP;
}

/// Raw, undecoded remember arguments as a surface read them.
#[derive(Debug, Clone, Default)]
pub struct RememberInput {
    pub content: String,
    pub tags: Vec<String>,
    pub namespace: Option<String>,
    pub memory_type: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub detect_contradiction: bool,
    pub entity: Option<String>,
    pub category: Option<String>,
    /// Caller-supplied metadata; only a JSON object is used (HTTP merges it,
    /// the CLI sends it as built).
    pub metadata: Option<Value>,
    pub source: Option<String>,
    pub source_type: Option<String>,
    pub author_type: Option<String>,
    /// Room id: MCP routes the write to a room. `POST /remember` takes none, so
    /// [`Self::to_http_body`] never sends it.
    pub room: Option<String>,
    /// Who wrote the memory (#1427 phase 2): the memory's own `author`, also
    /// the room link's author when `room` is set.
    pub author: Option<String>,
}

impl RememberInput {
    /// The `POST /remember` body, for clients that call the server (CLI via
    /// server). Nothing is validated or rewritten here: the server owns that.
    /// `content`, `tags` and `namespace` are always present; the others only
    /// when set (`type` only when non-empty, `detect_contradiction` only when
    /// true, `metadata` only when a non-empty object). `entity`, `category`,
    /// `valid_from` and `valid_until` are not part of this encoding: the CLI
    /// folds entity/category into `metadata`. `room` is never sent: the server
    /// has no such key (#1439). `author` is accepted since #1427 phase 2.
    pub fn to_http_body(&self) -> Value {
        let mut body = serde_json::json!({
            "content": self.content,
            "tags": self.tags,
            "namespace": self.namespace,
        });
        if let Some(at) = &self.author_type {
            body["author_type"] = serde_json::json!(at);
        }
        if let Some(t) = self.memory_type.as_deref().filter(|t| !t.is_empty()) {
            body["type"] = serde_json::json!(t);
        }
        if self.detect_contradiction {
            body["detect_contradiction"] = serde_json::json!(true);
        }
        if let Some(Value::Object(m)) = &self.metadata
            && !m.is_empty()
        {
            body["metadata"] = Value::Object(m.clone());
        }
        if let Some(author) = &self.author {
            body["author"] = serde_json::json!(author);
        }
        if let Some(src) = &self.source {
            body["source"] = serde_json::json!(src);
        }
        if let Some(st) = &self.source_type {
            body["source_type"] = serde_json::json!(st);
        }
        body
    }
}

/// Metadata the CLI builds from `--entity`, `--category` and repeated
/// `--meta` flags, the same on a local store and through a server (#1439).
///
/// A `key:value` pair becomes a number, a boolean or a string; a pair with no
/// `:` is the key set to `true`; `rel:type:target` pairs collect into a
/// `relationships` array. A `--meta` key wins over `--entity` / `--category`.
pub fn metadata_from_cli_flags(
    entity: Option<&str>,
    category: Option<&str>,
    pairs: &[String],
) -> Option<Value> {
    let mut map = Map::new();
    if let Some(e) = entity {
        map.insert("entity".into(), Value::String(e.to_string()));
    }
    if let Some(c) = category {
        map.insert("category".into(), Value::String(c.to_string()));
    }
    let mut relationships: Vec<Value> = Vec::new();
    for pair in pairs {
        if let Some((rel_type, target)) = crate::is_relationship_meta(pair) {
            relationships.push(serde_json::json!({"type": rel_type, "target": target}));
        } else if let Some((key, value)) = pair.split_once(':') {
            let val = if let Ok(n) = value.parse::<f64>() {
                Value::from(n)
            } else if value == "true" {
                Value::Bool(true)
            } else if value == "false" {
                Value::Bool(false)
            } else {
                Value::String(value.to_string())
            };
            map.insert(key.to_string(), val);
        } else {
            map.insert(pair.clone(), Value::Bool(true));
        }
    }
    if !relationships.is_empty() {
        map.insert("relationships".into(), Value::Array(relationships));
    }
    if map.is_empty() {
        None
    } else {
        Some(Value::Object(map))
    }
}

/// A validation failure. The text is the exact message the surface returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RememberRequestError(String);

impl RememberRequestError {
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RememberRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RememberRequestError {}

/// A decoded remember request.
#[derive(Debug, Clone, PartialEq)]
pub struct RememberRequest {
    pub content: String,
    pub tags: Vec<String>,
    /// `None` = the core's default namespace.
    pub namespace: Option<String>,
    /// After the policy default; `None` = auto-infer.
    pub memory_type: Option<String>,
    /// `None` when empty (the core write takes no empty map).
    pub metadata: Option<Value>,
    pub detect_contradiction: bool,
    pub source: Option<String>,
    pub source_type: Option<String>,
    pub author_type: Option<String>,
    pub room: Option<String>,
    pub author: Option<String>,
}

impl RememberRequest {
    /// Decode `input` under `policy`. Errors surface in the order the HTTP
    /// handler always reported them: content/tags, `author_type`, `type`.
    pub fn decode(
        input: RememberInput,
        policy: RememberPolicy,
    ) -> Result<Self, RememberRequestError> {
        if policy.validate_content_and_tags {
            crate::validate_input(&input.content, &input.tags)
                .map_err(|e| RememberRequestError(e.to_string()))?;
        }

        let mut meta = Map::new();
        if policy.type_in_metadata
            && let Some(t) = &input.memory_type
        {
            meta.insert("type".into(), Value::String(t.clone()));
        }
        // HTTP only: these fields exist on its wire format; MCP leaves them
        // unset, so the metadata stays empty there.
        for (key, value) in [
            ("valid_from", &input.valid_from),
            ("valid_until", &input.valid_until),
            ("entity", &input.entity),
            ("category", &input.category),
        ] {
            if let Some(v) = value {
                meta.insert(key.into(), Value::String(v.clone()));
            }
        }
        // Caller-supplied metadata object, merged last (#682).
        if let Some(Value::Object(extra)) = &input.metadata {
            for (k, v) in extra {
                meta.insert(k.clone(), v.clone());
            }
        }
        let metadata = if meta.is_empty() {
            None
        } else {
            Some(Value::Object(meta))
        };

        // Validate before any write (#1083): an invalid value must reject the
        // whole request, not leave a stored memory behind.
        if let Some(at) = input.author_type.as_deref() {
            validate_author_type(at).map_err(|e| RememberRequestError(e.to_string()))?;
        }
        // `author` (#1427 phase 2): free text, trimmed. With a room it is also
        // the room link's author; without one it is the memory's own.
        let author = input
            .author
            .as_deref()
            .map(|a| validate_author(a).map(str::to_string))
            .transpose()
            .map_err(|e| RememberRequestError(e.to_string()))?;
        if policy.validate_type
            && let Some(t) = input.memory_type.as_deref()
            && MemoryType::from_str_opt(t).is_none()
        {
            return Err(RememberRequestError(format!(
                "Unknown memory type '{t}'. Valid types: fact, procedure, preference, decision, context, note, insight, reference, event"
            )));
        }

        let memory_type = input
            .memory_type
            .or_else(|| policy.default_type.map(str::to_string));

        Ok(Self {
            content: input.content,
            tags: input.tags,
            namespace: input.namespace,
            memory_type,
            metadata,
            detect_contradiction: input.detect_contradiction,
            source: input.source,
            source_type: input.source_type,
            author_type: input.author_type,
            room: input.room,
            author,
        })
    }

    /// Tags as the core write functions take them.
    pub fn tag_refs(&self) -> Vec<&str> {
        self.tags.iter().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(content: &str) -> RememberInput {
        RememberInput {
            content: content.to_string(),
            ..RememberInput::default()
        }
    }

    #[test]
    fn blank_content_is_rejected_at_decode_on_every_policy() {
        for p in [RememberPolicy::HTTP, RememberPolicy::MCP] {
            let e = RememberRequest::decode(input("  "), p).unwrap_err();
            assert_eq!(e.message(), "Validation error: Content must not be empty");
        }
    }

    #[test]
    fn http_too_many_tags_rejected_with_core_text() {
        let mut i = input("x");
        i.tags = (0..1000).map(|n| format!("t{n}")).collect();
        let e = RememberRequest::decode(i, RememberPolicy::HTTP).unwrap_err();
        assert!(
            e.message().starts_with("Validation error: Too many tags"),
            "{e}"
        );
    }

    #[test]
    fn an_omitted_type_is_left_to_inference_on_every_policy() {
        for p in [RememberPolicy::HTTP, RememberPolicy::MCP] {
            let r = RememberRequest::decode(input("x"), p).unwrap();
            assert_eq!(r.memory_type, None);
            assert_eq!(r.metadata, None);
        }
    }

    #[test]
    fn an_explicit_type_wins_and_is_copied_to_metadata_on_every_policy() {
        let mut i = input("x");
        i.memory_type = Some("decision".into());
        for p in [RememberPolicy::HTTP, RememberPolicy::MCP] {
            let r = RememberRequest::decode(i.clone(), p).unwrap();
            assert_eq!(r.memory_type.as_deref(), Some("decision"));
            assert_eq!(r.metadata, Some(json!({"type": "decision"})));
        }
    }

    #[test]
    fn an_unknown_type_is_rejected_at_decode_on_every_policy() {
        let mut i = input("x");
        i.memory_type = Some("bogus".into());
        for p in [RememberPolicy::HTTP, RememberPolicy::MCP] {
            let e = RememberRequest::decode(i.clone(), p).unwrap_err();
            assert_eq!(
                e.message(),
                "Unknown memory type 'bogus'. Valid types: fact, procedure, preference, decision, context, note, insight, reference, event"
            );
        }
    }

    #[test]
    fn error_order_is_content_then_author_then_type() {
        let mut i = input("");
        i.author_type = Some("robot".into());
        i.memory_type = Some("bogus".into());
        let e = RememberRequest::decode(i.clone(), RememberPolicy::HTTP).unwrap_err();
        assert_eq!(e.message(), "Validation error: Content must not be empty");
        i.content = "x".into();
        let e = RememberRequest::decode(i.clone(), RememberPolicy::HTTP).unwrap_err();
        assert_eq!(
            e.message(),
            "Validation error: Invalid author_type 'robot'. Valid values: human, agent"
        );
        i.author_type = None;
        assert!(
            RememberRequest::decode(i, RememberPolicy::HTTP)
                .unwrap_err()
                .message()
                .starts_with("Unknown memory type")
        );
    }

    #[test]
    fn metadata_merges_fields_then_object_which_wins() {
        let mut i = input("x");
        i.entity = Some("e".into());
        i.category = Some("c".into());
        i.valid_from = Some("2026-01-01".into());
        i.valid_until = Some("2027-01-01".into());
        i.metadata = Some(json!({"project": "uteke", "entity": "override"}));
        let r = RememberRequest::decode(i, RememberPolicy::HTTP).unwrap();
        assert_eq!(
            r.metadata,
            Some(json!({
                "valid_from": "2026-01-01", "valid_until": "2027-01-01",
                "entity": "override", "category": "c", "project": "uteke"
            }))
        );
    }

    #[test]
    fn non_object_or_empty_metadata_is_none() {
        let mut i = input("x");
        i.metadata = Some(json!("str"));
        assert_eq!(
            RememberRequest::decode(i.clone(), RememberPolicy::HTTP)
                .unwrap()
                .metadata,
            None
        );
        i.metadata = Some(json!({}));
        assert_eq!(
            RememberRequest::decode(i, RememberPolicy::HTTP)
                .unwrap()
                .metadata,
            None
        );
    }

    #[test]
    fn passthrough_fields_survive() {
        let mut i = input("x");
        i.tags = vec!["a".into(), "b".into()];
        i.namespace = Some("ns".into());
        i.source = Some("s".into());
        i.source_type = Some("st".into());
        i.author_type = Some("human".into());
        i.detect_contradiction = true;
        let r = RememberRequest::decode(i, RememberPolicy::HTTP).unwrap();
        assert_eq!(r.tag_refs(), vec!["a", "b"]);
        assert_eq!(r.namespace.as_deref(), Some("ns"));
        assert_eq!(r.source.as_deref(), Some("s"));
        assert_eq!(r.source_type.as_deref(), Some("st"));
        assert_eq!(r.author_type.as_deref(), Some("human"));
        assert!(r.detect_contradiction);
    }

    /// #1427 phase 2: `author` is validated before any write, under every
    /// policy, and trimmed; the error order is content, author_type, author, type.
    #[test]
    fn author_is_validated_and_trimmed() {
        for policy in [RememberPolicy::HTTP, RememberPolicy::MCP] {
            let mut i = input("x");
            i.author = Some("  claude-code  ".into());
            assert_eq!(
                RememberRequest::decode(i, policy)
                    .unwrap()
                    .author
                    .as_deref(),
                Some("claude-code")
            );
            for bad in ["", "   ", "tab\there", &"y".repeat(129)] {
                let mut i = input("x");
                i.author = Some(bad.to_string());
                let e = RememberRequest::decode(i, policy).unwrap_err().to_string();
                assert!(e.starts_with("Validation error: author"), "{bad:?}: {e}");
            }
            let mut i = input("x");
            i.author = Some("y".repeat(128));
            assert!(
                RememberRequest::decode(i, policy).is_ok(),
                "128 is the bound"
            );
        }
        let mut i = input("x");
        i.author_type = Some("robot".into());
        i.author = Some("".into());
        i.memory_type = Some("nope".into());
        let e = RememberRequest::decode(i, RememberPolicy::HTTP)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("author_type"),
            "author_type is reported before author: {e}"
        );
    }

    #[test]
    fn http_body_minimal_and_full() {
        let mut i = input("hi");
        i.namespace = Some("default".into());
        assert_eq!(
            i.to_http_body(),
            json!({"content": "hi", "tags": [], "namespace": "default"})
        );
        i.tags = vec!["t".into()];
        i.memory_type = Some("note".into());
        i.detect_contradiction = true;
        i.author_type = Some("human".into());
        i.metadata = metadata_from_cli_flags(Some("e"), Some("c"), &["k:v:w".into(), "bad".into()]);
        i.room = Some("r".into());
        i.author = Some("a".into());
        i.source = Some("s".into());
        i.source_type = Some("st".into());
        assert_eq!(
            i.to_http_body(),
            json!({
                "content": "hi", "tags": ["t"], "namespace": "default",
                "author_type": "human",
                "type": "note", "detect_contradiction": true,
                "metadata": {"entity": "e", "category": "c", "k": "v:w", "bad": true},
                "author": "a", "source": "s", "source_type": "st"
            })
        );
    }

    #[test]
    fn http_body_omits_empty_type_false_flag_and_empty_metadata() {
        let mut i = input("hi");
        i.memory_type = Some(String::new());
        i.metadata = metadata_from_cli_flags(None, None, &[]);
        assert_eq!(i.metadata, None);
        assert_eq!(
            i.to_http_body(),
            json!({"content": "hi", "tags": [], "namespace": null})
        );
    }

    #[test]
    fn cli_meta_pairs_are_typed_and_relationships_collected() {
        let m = metadata_from_cli_flags(
            Some("e"),
            None,
            &[
                "n:3".into(),
                "ok:true".into(),
                "off:false".into(),
                "s:text:with:colons".into(),
                "flag".into(),
                "entity:override".into(),
            ],
        )
        .unwrap();
        assert_eq!(
            m,
            json!({"entity": "override", "n": 3.0, "ok": true, "off": false,
                   "s": "text:with:colons", "flag": true})
        );
        let rel = metadata_from_cli_flags(None, None, &["rel:supersedes:abc".into()]).unwrap();
        assert_eq!(
            rel,
            json!({"relationships": [{"type": "supersedes", "target": "abc"}]})
        );
    }

    #[test]
    fn display_writes_the_exact_message() {
        let e = RememberRequest::decode(input("  "), RememberPolicy::HTTP).unwrap_err();
        assert_eq!(e.to_string(), "Validation error: Content must not be empty");
    }
}
