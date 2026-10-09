//! Typed `recall` request shared by the HTTP server, the MCP server and the
//! CLI-via-server client (#1343, first slice).
//!
//! Each surface used to parse, default, clamp and validate its own recall
//! arguments. This module owns that logic once: a surface decodes its wire
//! format into a [`RecallInput`] (raw, every field optional), picks a
//! [`RecallPolicy`], and calls [`RecallRequest::decode`].
//!
//! This is an extraction, not a unification. Where the surfaces disagree
//! today the disagreement is encoded as a [`RecallPolicy`] value rather than
//! resolved:
//!
//! | aspect                  | HTTP `/recall`      | MCP `uteke_recall`        | CLI via server        |
//! |-------------------------|---------------------|---------------------------|-----------------------|
//! | limit default           | 5                   | 5                         | 5 (clap), always sent |
//! | limit cap               | 100 (#903)          | none                      | none (server caps)    |
//! | blank query             | rejected            | accepted                  | accepted (server 400) |
//! | empty `tags: []`        | no filter           | filter matching nothing   | sent, so no filter    |
//! | namespace omitted       | all namespaces      | all namespaces            | `"default"` only      |
//! | `strict`, config floor  | honoured            | not accepted              | `--strict` forwarded  |
//! | entity/category/enrich  | honoured            | not accepted              | forwarded             |
//! | at/after/before         | honoured            | not accepted              | `--at` forwarded      |
//!
//! `namespace: None` means "search every namespace" (core, #448); the CLI
//! resolves its own namespace to `Some("default")` before encoding.

use crate::memory::types::{RecallStrategy, SearchType};
use chrono::{DateTime, Utc};
use std::fmt;

/// Result count when the caller gives no `limit`.
pub const RECALL_DEFAULT_LIMIT: usize = 5;
/// Cap the HTTP surface puts on `limit` (#903, DoS guard).
pub const RECALL_HTTP_MAX_LIMIT: usize = 100;
/// Character budget of a `pack` recall when the caller gives none.
pub const RECALL_DEFAULT_BUDGET_CHARS: usize = 4000;
/// Similarity floor when neither the request nor `[recall]` sets one.
pub const RECALL_DEFAULT_MIN_SCORE: f32 = 0.0;
/// Similarity floor of `strict` when `[recall] min_score_strict` is unset.
pub const RECALL_STRICT_MIN_SCORE: f32 = 0.5;

/// Where today's surfaces disagree, per surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecallPolicy {
    /// Upper bound applied to `limit`; `None` leaves it unbounded.
    pub max_limit: Option<usize>,
    /// Reject an empty or whitespace-only query (#907).
    pub reject_blank_query: bool,
    /// `tags: []` means "no tag filter" (otherwise it is a filter that
    /// matches nothing, because the core filter is any-of).
    pub empty_tags_mean_no_filter: bool,
}

impl RecallPolicy {
    /// `POST /recall`.
    pub const HTTP: Self = Self {
        max_limit: Some(RECALL_HTTP_MAX_LIMIT),
        reject_blank_query: true,
        empty_tags_mean_no_filter: true,
    };
    /// MCP `uteke_recall`.
    pub const MCP: Self = Self {
        max_limit: None,
        reject_blank_query: false,
        empty_tags_mean_no_filter: false,
    };
}

/// Raw, undecoded recall arguments as a surface read them.
#[derive(Debug, Clone, Default)]
pub struct RecallInput {
    pub query: String,
    pub limit: Option<usize>,
    pub tags: Option<Vec<String>>,
    pub namespace: Option<String>,
    pub entity: Option<String>,
    pub category: Option<String>,
    pub min_score: Option<f32>,
    pub strict: bool,
    /// RFC3339 point-in-time.
    pub at: Option<String>,
    /// RFC3339 lower bound on creation time.
    pub after: Option<String>,
    /// RFC3339 upper bound on creation time.
    pub before: Option<String>,
    /// `all` | `memory` | `doc`.
    pub search_type: Option<String>,
    pub enrich: bool,
    pub pack: bool,
    pub budget_chars: Option<usize>,
    pub exclude_ids: Vec<String>,
    /// `vector` | `fts5` | `hybrid` | `graph` | `fusion`.
    pub strategy: Option<String>,
    pub explain: bool,
}

impl RecallInput {
    /// The `POST /recall` body, for clients that call the server (CLI via
    /// server). Nothing is validated or rewritten here: the server owns that.
    /// `limit`, `tags` and `namespace` are always present; the other fields
    /// only when set, and `strict`, `enrich` and `pack` only when true. A
    /// `pack` body always carries `budget_chars`. `after`, `before`, `strategy`
    /// and `explain` are not part of this encoding (the CLI falls back to the
    /// local store when those flags are used).
    pub fn to_http_body(&self) -> serde_json::Value {
        let mut body = serde_json::json!({
            "query": self.query,
            "limit": self.limit.unwrap_or(RECALL_DEFAULT_LIMIT),
            "tags": self.tags.clone().unwrap_or_default(),
            "namespace": self.namespace,
        });
        if let Some(e) = &self.entity {
            body["entity"] = serde_json::json!(e);
        }
        if let Some(c) = &self.category {
            body["category"] = serde_json::json!(c);
        }
        if let Some(m) = self.min_score {
            body["min_score"] = serde_json::json!(m);
        }
        if self.strict {
            body["strict"] = serde_json::json!(true);
        }
        if let Some(a) = &self.at {
            body["at"] = serde_json::json!(a);
        }
        if let Some(t) = &self.search_type {
            body["search_type"] = serde_json::json!(t);
        }
        if self.enrich {
            body["enrich"] = serde_json::json!(true);
        }
        if self.pack {
            body["pack"] = serde_json::json!(true);
            body["budget_chars"] =
                serde_json::json!(self.budget_chars.unwrap_or(RECALL_DEFAULT_BUDGET_CHARS));
            if !self.exclude_ids.is_empty() {
                body["exclude_ids"] = serde_json::json!(self.exclude_ids);
            }
        }
        body
    }
}

/// A validation failure. The text is the exact message the surfaces return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecallRequestError(String);

impl RecallRequestError {
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RecallRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RecallRequestError {}

/// A decoded, defaulted and clamped recall request.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallRequest {
    pub query: String,
    /// Already defaulted and clamped by the policy.
    pub limit: usize,
    /// `None` = no tag filter.
    pub tags: Option<Vec<String>>,
    /// `None` = every namespace.
    pub namespace: Option<String>,
    pub entity: Option<String>,
    pub category: Option<String>,
    pub min_score: Option<f32>,
    pub strict: bool,
    pub at: Option<DateTime<Utc>>,
    pub after: Option<DateTime<Utc>>,
    pub before: Option<DateTime<Utc>>,
    pub enrich: bool,
    pub pack: bool,
    pub budget_chars: Option<usize>,
    pub exclude_ids: Vec<String>,
    pub explain: bool,
    // Kept raw: each is validated at the point the surface first needs it,
    // which is later than the fields above and not at all on some paths
    // (an invalid `search_type` is ignored next to `at`).
    search_type: Option<String>,
    strategy: Option<String>,
}

fn parse_ts(
    field: &str,
    value: Option<String>,
    example: &str,
) -> Result<Option<DateTime<Utc>>, RecallRequestError> {
    match value {
        None => Ok(None),
        Some(ts) => DateTime::parse_from_rfc3339(&ts)
            .map(|dt| Some(dt.with_timezone(&Utc)))
            .map_err(|_| {
                RecallRequestError(format!(
                    "Invalid '{field}' timestamp: {ts}. Use RFC3339 format (e.g. {example})"
                ))
            }),
    }
}

impl RecallRequest {
    /// Decode `input` under `policy`. Errors surface in the order the HTTP
    /// handler always reported them: query, `at`, `after`, `before`.
    pub fn decode(input: RecallInput, policy: RecallPolicy) -> Result<Self, RecallRequestError> {
        if policy.reject_blank_query && input.query.trim().is_empty() {
            return Err(RecallRequestError(
                "Query must not be empty or whitespace-only".to_string(),
            ));
        }
        let limit = input.limit.unwrap_or(RECALL_DEFAULT_LIMIT);
        let limit = policy.max_limit.map_or(limit, |max| limit.min(max));

        let tags = match input.tags {
            Some(t) if t.is_empty() && policy.empty_tags_mean_no_filter => None,
            other => other,
        };
        let at = parse_ts("at", input.at, "2026-06-01T12:00:00Z")?;
        let after = parse_ts("after", input.after, "2026-01-01T00:00:00Z")?;
        let before = parse_ts("before", input.before, "2026-01-01T00:00:00Z")?;

        Ok(Self {
            query: input.query,
            limit,
            tags,
            namespace: input.namespace,
            entity: input.entity,
            category: input.category,
            min_score: input.min_score,
            strict: input.strict,
            at,
            after,
            before,
            enrich: input.enrich,
            pack: input.pack,
            budget_chars: input.budget_chars,
            exclude_ids: input.exclude_ids,
            explain: input.explain,
            search_type: input.search_type,
            strategy: input.strategy,
        })
    }

    /// The raw `search_type` the caller gave, if any.
    pub fn search_type_raw(&self) -> Option<&str> {
        self.search_type.as_deref()
    }

    /// Resolve `search_type`; absent means [`SearchType::All`].
    pub fn search_type(&self) -> Result<SearchType, RecallRequestError> {
        match self.search_type.as_deref() {
            Some("memory") => Ok(SearchType::Memory),
            Some("doc") => Ok(SearchType::Document),
            Some("all") | None => Ok(SearchType::All),
            Some(other) => Err(RecallRequestError(format!(
                "Invalid search_type: '{other}'. Use 'all', 'memory', or 'doc'."
            ))),
        }
    }

    /// Resolve the strategy: the request's, else `config_default`
    /// (`[recall] default_strategy`), else fusion (#1034, #1123).
    pub fn strategy(
        &self,
        config_default: Option<&str>,
    ) -> Result<RecallStrategy, RecallRequestError> {
        match self.strategy.as_deref().or(config_default) {
            Some(name) => RecallStrategy::from_str_opt(name).ok_or_else(|| {
                RecallRequestError(format!(
                    "Invalid strategy: '{name}'. Use 'vector', 'fts5', 'hybrid', 'graph', or 'fusion'."
                ))
            }),
            None => Ok(RecallStrategy::Fusion),
        }
    }

    /// Similarity floor: `min_score`, else the `[recall]` value for the mode
    /// (`strict` or not), else the built-in default.
    pub fn resolve_min_score(
        &self,
        config_min_score: Option<f64>,
        config_min_score_strict: Option<f64>,
    ) -> f32 {
        let (configured, builtin) = if self.strict {
            (config_min_score_strict, RECALL_STRICT_MIN_SCORE)
        } else {
            (config_min_score, RECALL_DEFAULT_MIN_SCORE)
        };
        self.min_score
            .unwrap_or_else(|| configured.unwrap_or(builtin as f64) as f32)
    }

    /// Character budget for `pack`.
    pub fn budget_chars_or_default(&self) -> usize {
        self.budget_chars.unwrap_or(RECALL_DEFAULT_BUDGET_CHARS)
    }

    /// Tag filter as the core recall functions take it.
    pub fn tag_filter(&self) -> Option<Vec<&str>> {
        self.tags
            .as_ref()
            .map(|t| t.iter().map(String::as_str).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(query: &str) -> RecallInput {
        RecallInput {
            query: query.to_string(),
            ..RecallInput::default()
        }
    }

    #[test]
    fn default_limit_is_5_on_every_policy() {
        for p in [RecallPolicy::HTTP, RecallPolicy::MCP] {
            assert_eq!(RecallRequest::decode(input("q"), p).unwrap().limit, 5);
        }
    }

    #[test]
    fn limit_cap_is_http_only() {
        let mut i = input("q");
        i.limit = Some(100_000);
        let http = RecallRequest::decode(i.clone(), RecallPolicy::HTTP).unwrap();
        let mcp = RecallRequest::decode(i, RecallPolicy::MCP).unwrap();
        assert_eq!(http.limit, 100);
        assert_eq!(mcp.limit, 100_000);
    }

    #[test]
    fn limit_at_and_below_cap_is_untouched() {
        for l in [0, 1, 99, 100] {
            let mut i = input("q");
            i.limit = Some(l);
            assert_eq!(
                RecallRequest::decode(i, RecallPolicy::HTTP).unwrap().limit,
                l
            );
        }
        let mut i = input("q");
        i.limit = Some(101);
        assert_eq!(
            RecallRequest::decode(i, RecallPolicy::HTTP).unwrap().limit,
            100
        );
    }

    #[test]
    fn blank_query_is_rejected_by_http_only() {
        let err = RecallRequest::decode(input("  \t"), RecallPolicy::HTTP).unwrap_err();
        assert_eq!(err.message(), "Query must not be empty or whitespace-only");
        assert!(RecallRequest::decode(input("  \t"), RecallPolicy::MCP).is_ok());
        assert!(RecallRequest::decode(input(""), RecallPolicy::MCP).is_ok());
    }

    #[test]
    fn empty_tags_policy() {
        let mut i = input("q");
        i.tags = Some(vec![]);
        let http = RecallRequest::decode(i.clone(), RecallPolicy::HTTP).unwrap();
        let mcp = RecallRequest::decode(i, RecallPolicy::MCP).unwrap();
        assert_eq!(http.tag_filter(), None);
        assert_eq!(mcp.tag_filter(), Some(vec![]));

        let mut i = input("q");
        i.tags = Some(vec!["a".into(), "b".into()]);
        let r = RecallRequest::decode(i, RecallPolicy::HTTP).unwrap();
        assert_eq!(r.tag_filter(), Some(vec!["a", "b"]));
        assert_eq!(
            RecallRequest::decode(input("q"), RecallPolicy::MCP)
                .unwrap()
                .tag_filter(),
            None
        );
    }

    #[test]
    fn namespace_is_passed_through_and_absent_means_all() {
        let r = RecallRequest::decode(input("q"), RecallPolicy::HTTP).unwrap();
        assert_eq!(r.namespace, None);
        let mut i = input("q");
        i.namespace = Some("work".into());
        let r = RecallRequest::decode(i, RecallPolicy::MCP).unwrap();
        assert_eq!(r.namespace.as_deref(), Some("work"));
    }

    #[test]
    fn timestamp_errors_keep_their_exact_text_and_order() {
        let mut i = input("q");
        i.at = Some("nope".into());
        i.after = Some("also-bad".into());
        let err = RecallRequest::decode(i, RecallPolicy::HTTP).unwrap_err();
        assert_eq!(
            err.message(),
            "Invalid 'at' timestamp: nope. Use RFC3339 format (e.g. 2026-06-01T12:00:00Z)"
        );

        let mut i = input("q");
        i.after = Some("x".into());
        assert_eq!(
            RecallRequest::decode(i, RecallPolicy::HTTP)
                .unwrap_err()
                .message(),
            "Invalid 'after' timestamp: x. Use RFC3339 format (e.g. 2026-01-01T00:00:00Z)"
        );
        let mut i = input("q");
        i.before = Some("y".into());
        assert_eq!(
            RecallRequest::decode(i, RecallPolicy::HTTP)
                .unwrap_err()
                .message(),
            "Invalid 'before' timestamp: y. Use RFC3339 format (e.g. 2026-01-01T00:00:00Z)"
        );
    }

    #[test]
    fn timestamps_are_normalised_to_utc() {
        let mut i = input("q");
        i.at = Some("2026-06-01T14:00:00+02:00".into());
        let r = RecallRequest::decode(i, RecallPolicy::HTTP).unwrap();
        assert_eq!(
            r.at.unwrap().to_rfc3339(),
            "2026-06-01T12:00:00+00:00".to_string()
        );
    }

    #[test]
    fn invalid_search_type_is_lazy() {
        let mut i = input("q");
        i.search_type = Some("bogus".into());
        let r = RecallRequest::decode(i, RecallPolicy::HTTP).unwrap();
        assert_eq!(
            r.search_type().unwrap_err().message(),
            "Invalid search_type: 'bogus'. Use 'all', 'memory', or 'doc'."
        );
        assert_eq!(r.search_type_raw(), Some("bogus"));
    }

    #[test]
    fn search_type_mapping() {
        for (raw, want) in [
            (None, SearchType::All),
            (Some("all"), SearchType::All),
            (Some("memory"), SearchType::Memory),
            (Some("doc"), SearchType::Document),
        ] {
            let mut i = input("q");
            i.search_type = raw.map(String::from);
            let r = RecallRequest::decode(i, RecallPolicy::MCP).unwrap();
            assert_eq!(r.search_type().unwrap(), want);
        }
    }

    #[test]
    fn strategy_precedence_request_then_config_then_fusion() {
        let r = RecallRequest::decode(input("q"), RecallPolicy::HTTP).unwrap();
        assert_eq!(r.strategy(None).unwrap(), RecallStrategy::Fusion);
        assert_eq!(r.strategy(Some("vector")).unwrap(), RecallStrategy::Vector);

        let mut i = input("q");
        i.strategy = Some("fts5".into());
        let r = RecallRequest::decode(i, RecallPolicy::HTTP).unwrap();
        assert_eq!(r.strategy(Some("vector")).unwrap(), RecallStrategy::Fts5);

        let mut i = input("q");
        i.strategy = Some("nope".into());
        let r = RecallRequest::decode(i, RecallPolicy::HTTP).unwrap();
        assert_eq!(
            r.strategy(None).unwrap_err().message(),
            "Invalid strategy: 'nope'. Use 'vector', 'fts5', 'hybrid', 'graph', or 'fusion'."
        );
        // A bad config default is reported the same way.
        let r = RecallRequest::decode(input("q"), RecallPolicy::HTTP).unwrap();
        assert!(r.strategy(Some("zzz")).is_err());
    }

    #[test]
    fn min_score_resolution() {
        let plain = RecallRequest::decode(input("q"), RecallPolicy::HTTP).unwrap();
        assert_eq!(plain.resolve_min_score(None, None), 0.0);
        assert_eq!(plain.resolve_min_score(Some(0.25), Some(0.8)), 0.25);

        let mut i = input("q");
        i.strict = true;
        let strict = RecallRequest::decode(i, RecallPolicy::HTTP).unwrap();
        assert_eq!(strict.resolve_min_score(None, None), 0.5);
        assert_eq!(strict.resolve_min_score(Some(0.25), Some(0.8)), 0.8);

        let mut i = input("q");
        i.strict = true;
        i.min_score = Some(0.3);
        let explicit = RecallRequest::decode(i, RecallPolicy::HTTP).unwrap();
        assert_eq!(explicit.resolve_min_score(Some(0.9), Some(0.9)), 0.3);
    }

    #[test]
    fn budget_default() {
        let r = RecallRequest::decode(input("q"), RecallPolicy::MCP).unwrap();
        assert_eq!(r.budget_chars_or_default(), 4000);
        let mut i = input("q");
        i.budget_chars = Some(10);
        let r = RecallRequest::decode(i, RecallPolicy::MCP).unwrap();
        assert_eq!(r.budget_chars_or_default(), 10);
    }

    #[test]
    fn http_body_minimal_matches_the_old_cli_body() {
        let mut i = input("hello");
        i.limit = Some(5);
        i.namespace = Some("default".into());
        assert_eq!(
            i.to_http_body(),
            serde_json::json!({
                "query": "hello",
                "limit": 5,
                "tags": [],
                "namespace": "default",
            })
        );
    }

    #[test]
    fn http_body_full_matches_the_old_cli_body() {
        let i = RecallInput {
            query: "q".into(),
            limit: Some(7),
            tags: Some(vec!["a".into()]),
            namespace: Some("ns".into()),
            entity: Some("e".into()),
            category: Some("c".into()),
            min_score: Some(0.25),
            strict: true,
            at: Some("2026-06-01T12:00:00Z".into()),
            search_type: Some("memory".into()),
            enrich: true,
            pack: true,
            budget_chars: Some(900),
            exclude_ids: vec!["x".into(), "y".into()],
            ..RecallInput::default()
        };
        assert_eq!(
            i.to_http_body(),
            serde_json::json!({
                "query": "q",
                "limit": 7,
                "tags": ["a"],
                "namespace": "ns",
                "entity": "e",
                "category": "c",
                "min_score": 0.25,
                "strict": true,
                "at": "2026-06-01T12:00:00Z",
                "search_type": "memory",
                "enrich": true,
                "pack": true,
                "budget_chars": 900,
                "exclude_ids": ["x", "y"],
            })
        );
    }

    #[test]
    fn http_body_pack_without_excludes_omits_them() {
        let mut i = input("q");
        i.pack = true;
        i.budget_chars = Some(4000);
        let body = i.to_http_body();
        assert_eq!(body["pack"], serde_json::json!(true));
        assert_eq!(body["budget_chars"], serde_json::json!(4000));
        assert!(body.get("exclude_ids").is_none());
        assert!(body.get("strict").is_none());
        assert!(body.get("enrich").is_none());
    }
}
