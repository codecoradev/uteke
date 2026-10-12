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
//! | content/tags validation   | at decode (400)             | inside the core call        | server validates          |
//! | `type` omitted            | auto-inference              | stored as `fact`, no infer  | omitted when empty        |
//! | unknown `type`            | 400 at decode               | core error (`Failed: ...`)  | server 400                |
//! | `type` in metadata        | copied as `"type"`          | not copied                  | not copied                |
//! | metadata object           | merged, plus entity/etc.    | not accepted (always none)  | built from flags, sent    |
//! | entity/category fields    | honoured (into metadata)    | not accepted                | folded into `metadata`    |
//! | valid_from/valid_until    | honoured (into metadata)    | not accepted                | not sent                  |
//! | detect_contradiction      | honoured                    | not accepted                | forwarded when set        |
//! | source/source_type/author_type | honoured               | not accepted                | forwarded                 |
//! | room/author               | rejected as unknown keys    | `room` routes to room write | sent (server rejects)     |
//! | timestamp                 | rejected as unknown key     | not accepted                | sent (server rejects)     |
//! | namespace omitted         | core default                | core default                | `"default"` always sent   |
//!
//! Room remember is a separate operation and is not part of this module.

use crate::memory::types::{MemoryType, validate_author_type};
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
    /// MCP `uteke_remember`.
    pub const MCP: Self = Self {
        validate_content_and_tags: false,
        validate_type: false,
        default_type: Some("fact"),
        type_in_metadata: false,
    };
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
    /// Room id (MCP routes the write to a room; the CLI forwards it).
    pub room: Option<String>,
    /// Room author (MCP) / forwarded by the CLI.
    pub author: Option<String>,
    /// CLI `--timestamp`. Only [`Self::to_http_body`] reads it.
    pub timestamp: Option<String>,
}

impl RememberInput {
    /// The `POST /remember` body, for clients that call the server (CLI via
    /// server). Nothing is validated or rewritten here: the server owns that.
    /// `content`, `tags` and `namespace` are always present; the others only
    /// when set (`type` only when non-empty, `detect_contradiction` only when
    /// true, `metadata` only when a non-empty object). `entity`, `category`,
    /// `valid_from` and `valid_until` are not part of this encoding: the CLI
    /// folds entity/category into `metadata`. `timestamp`, `room` and `author`
    /// are forwarded even though the server rejects them as unknown keys
    /// today; that is current behaviour, listed as an owner decision.
    pub fn to_http_body(&self) -> Value {
        let mut body = serde_json::json!({
            "content": self.content,
            "tags": self.tags,
            "namespace": self.namespace,
        });
        if let Some(ts) = &self.timestamp {
            body["timestamp"] = serde_json::json!(ts);
        }
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
        if let Some(room) = &self.room {
            body["room"] = serde_json::json!(room);
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
/// `--meta key:value` flags. A pair without `:` is dropped silently.
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
    for pair in pairs {
        if let Some((key, value)) = pair.split_once(':') {
            map.insert(key.to_string(), Value::String(value.to_string()));
        }
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
            author: input.author,
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
    fn http_blank_content_rejected_mcp_defers_to_core() {
        let e = RememberRequest::decode(input("  "), RememberPolicy::HTTP).unwrap_err();
        assert_eq!(e.message(), "Validation error: Content must not be empty");
        assert!(RememberRequest::decode(input("  "), RememberPolicy::MCP).is_ok());
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
    fn default_type_is_none_on_http_and_fact_on_mcp() {
        let h = RememberRequest::decode(input("x"), RememberPolicy::HTTP).unwrap();
        let m = RememberRequest::decode(input("x"), RememberPolicy::MCP).unwrap();
        assert_eq!(h.memory_type, None);
        assert_eq!(m.memory_type.as_deref(), Some("fact"));
        assert_eq!(m.metadata, None);
    }

    #[test]
    fn explicit_type_wins_and_only_http_copies_it_to_metadata() {
        let mut i = input("x");
        i.memory_type = Some("decision".into());
        let h = RememberRequest::decode(i.clone(), RememberPolicy::HTTP).unwrap();
        let m = RememberRequest::decode(i, RememberPolicy::MCP).unwrap();
        assert_eq!(h.memory_type.as_deref(), Some("decision"));
        assert_eq!(h.metadata, Some(json!({"type": "decision"})));
        assert_eq!(m.memory_type.as_deref(), Some("decision"));
        assert_eq!(m.metadata, None);
    }

    #[test]
    fn unknown_type_rejected_on_http_only_at_decode() {
        let mut i = input("x");
        i.memory_type = Some("bogus".into());
        let e = RememberRequest::decode(i.clone(), RememberPolicy::HTTP).unwrap_err();
        assert_eq!(
            e.message(),
            "Unknown memory type 'bogus'. Valid types: fact, procedure, preference, decision, context, note, insight, reference, event"
        );
        assert_eq!(
            RememberRequest::decode(i, RememberPolicy::MCP)
                .unwrap()
                .memory_type
                .as_deref(),
            Some("bogus")
        );
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
        i.timestamp = Some("2026-01-01T00:00:00Z".into());
        assert_eq!(
            i.to_http_body(),
            json!({
                "content": "hi", "tags": ["t"], "namespace": "default",
                "timestamp": "2026-01-01T00:00:00Z", "author_type": "human",
                "type": "note", "detect_contradiction": true,
                "metadata": {"entity": "e", "category": "c", "k": "v:w"},
                "room": "r", "author": "a", "source": "s", "source_type": "st"
            })
        );
    }

    #[test]
    fn http_body_omits_empty_type_false_flag_and_empty_metadata() {
        let mut i = input("hi");
        i.memory_type = Some(String::new());
        i.metadata = metadata_from_cli_flags(None, None, &["nocolon".into()]);
        assert_eq!(i.metadata, None);
        assert_eq!(
            i.to_http_body(),
            json!({"content": "hi", "tags": [], "namespace": null})
        );
    }
}
