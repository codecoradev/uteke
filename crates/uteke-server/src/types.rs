//! Request/response structs, helpers, and constants for the uteke server.

use std::io::Read as IoRead;

use serde::{Deserialize, Serialize};
use tiny_http::Header;

// ── Document Types ──────────────────────────────────────────────────────

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct DocCreateRequest {
    pub slug: String,
    pub title: Option<String>,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub parent: Option<String>,
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct DocGetRequest {
    pub id: Option<String>,
    pub slug: Option<String>,
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct DocListParams {
    #[serde(default = "default_doc_limit")]
    pub limit: usize,
    #[serde(default)]
    pub roots_only: bool,
    #[serde(default)]
    pub parent: Option<String>,
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct DocSearchRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default = "default_search_mode")]
    pub mode: String,
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocMoveRequest {
    pub id: Option<String>,
    pub slug: Option<String>,
    #[serde(default)]
    pub new_parent: Option<String>,
    /// Optional sort order for the moved document (#sort-order).
    #[serde(default)]
    pub new_sort_order: Option<i64>,
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct DocUpdateRequest {
    pub id: Option<String>,
    pub slug: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

pub fn default_search_mode() -> String {
    "hybrid".to_string()
}

// ── Document endpoint helpers ─────────────────────────────────────────

pub fn resolve_doc_id(req: &DocGetRequest) -> Result<&str, &'static str> {
    match (&req.id, &req.slug) {
        (Some(id), _) => Ok(id),
        (_, Some(slug)) => Ok(slug),
        _ => Err("provide either 'id' or 'slug'"),
    }
}

pub fn resolve_doc_id_move(req: &DocMoveRequest) -> Result<&str, &'static str> {
    match (&req.id, &req.slug) {
        (Some(id), _) => Ok(id),
        (_, Some(slug)) => Ok(slug),
        _ => Err("provide either 'id' or 'slug'"),
    }
}

pub fn resolve_doc_id_update(req: &DocUpdateRequest) -> Result<&str, &'static str> {
    match (&req.id, &req.slug) {
        (Some(id), _) => Ok(id),
        (_, Some(slug)) => Ok(slug),
        _ => Err("provide either 'id' or 'slug'"),
    }
}

// ── API Versioning (#737) ────────────────────────────────────────────────────

/// Supported API versions. Routes prefixed with `/api/vN/` are dispatched
/// according to this version. Unversioned routes alias to `Latest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiVersion {
    /// v0.7.x compatible format (flat recall results).
    V1,
    /// Current format (v0.8.x+, wrapped UnifiedSearchResult).
    V2,
}

impl ApiVersion {
    /// Parse `/api/vN/` prefix from a path. Returns `(version, stripped_path)` on match,
    /// or `None` if the path is not versioned.
    pub fn from_path(path: &str) -> Option<(Self, &str)> {
        // The original strip_prefix("/api/v1/") removed the trailing slash,
        // producing "recall" (no leading /) which never matched route patterns
        // like "/recall". Fix: strip "/api/v1" without trailing slash, keeping
        // the "/" in the remainder: "/api/v1/recall" → rest = "/recall" ✅.
        if let Some(rest) = path.strip_prefix("/api/v1") {
            Some((Self::V1, rest))
        } else if let Some(rest) = path.strip_prefix("/api/v2") {
            Some((Self::V2, rest))
        } else {
            None
        }
    }
}

/// Convert a `UnifiedSearchResult` (v2) to the v1 flat format.
/// v1 consumers expect: `[{id, content, score, namespace, tags, ...}]`
/// instead of the v2 wrapped structure.
pub fn to_v1_flat(result: &uteke_core::memory::types::UnifiedSearchResult) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "content".into(),
        serde_json::Value::String(result.content.clone()),
    );
    map.insert("score".into(), serde_json::json!(result.score));
    if let Some(id) = &result.memory_id {
        map.insert("id".into(), serde_json::Value::String(id.clone()));
    }
    if let Some(ns) = &result.namespace {
        map.insert("namespace".into(), serde_json::Value::String(ns.clone()));
    }
    if let Some(source) = &result.source {
        map.insert("source".into(), serde_json::Value::String(source.clone()));
    }
    if !result.tags.is_empty() {
        map.insert("tags".into(), serde_json::json!(result.tags));
    }
    if let Some(meta) = &result.metadata {
        map.insert("metadata".into(), meta.clone());
    }
    if let Some(mt) = &result.memory_type {
        map.insert("type".into(), serde_json::Value::String(mt.clone()));
    }
    if let Some(imp) = result.importance {
        map.insert("importance".into(), serde_json::json!(imp));
    }
    if let Some(pin) = result.pinned {
        map.insert("pinned".into(), serde_json::json!(pin));
    }
    serde_json::Value::Object(map)
}

// ── Types ───────────────────────────────────────────────────────────────────

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct RememberRequest {
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub valid_from: Option<String>,
    #[serde(default)]
    pub valid_until: Option<String>,
    #[serde(default)]
    pub detect_contradiction: bool,
    /// Entity name — stored as metadata key "entity".
    #[serde(default)]
    pub entity: Option<String>,
    /// Category — stored as metadata key "category".
    #[serde(default)]
    pub category: Option<String>,
    /// Extra metadata key=value pairs, merged into the metadata map.
    /// Accepts an object (e.g. {"project": "uteke"}).
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// Source provenance — set via set_source() after storage.
    #[serde(default)]
    pub source: Option<String>,
    /// Source type (defaults to "user").
    #[serde(default)]
    pub source_type: Option<String>,
    /// Author type: "human" | "agent" (#1083). Defaults to "agent" when omitted.
    #[serde(default)]
    pub author_type: Option<String>,
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct RecallRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    /// Filter by entity metadata.
    #[serde(default)]
    pub entity: Option<String>,
    /// Filter by category metadata.
    #[serde(default)]
    pub category: Option<String>,
    /// Minimum similarity score (0.0-1.0). Results below are filtered.
    /// Default: 0.0 (no filtering). Use `strict=true` for 0.5 default (#995).
    #[serde(default)]
    pub min_score: Option<f32>,
    /// Use strict threshold (defaults to 0.5 if min_score not set).
    #[serde(default)]
    pub strict: bool,
    /// Time-travel: query memories that existed at this RFC3339 timestamp.
    #[serde(default)]
    pub at: Option<String>,
    /// Search type filter: "all" (default, unified), "memory", or "doc" (#531).
    #[serde(default)]
    pub search_type: Option<String>,
    /// Enrich results with cross-entity links (doc↔memory) (#689).
    /// When true, populates `linked_doc_slugs` on memory results and
    /// `linked_memory_ids` on document results.
    #[serde(default)]
    pub enrich: bool,
    /// Recall strategy: "hybrid" (default), "vector", "fts5", or "graph" (#900, #1034).
    /// When absent, the server falls back to `[recall] default_strategy` from
    /// uteke.toml, then to "hybrid" — matching the CLI default.
    /// Invalid values return HTTP 400.
    #[serde(default)]
    pub strategy: Option<String>,
    /// Temporal range filter: only return memories created at or after this
    /// RFC3339 timestamp (#902).
    #[serde(default)]
    pub after: Option<String>,
    /// Temporal range filter: only return memories created at or before this
    /// RFC3339 timestamp (#902).
    #[serde(default)]
    pub before: Option<String>,
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "default_limit_search")]
    pub limit: usize,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub namespace: Option<String>,
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct ListParams {
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub namespace: Option<String>,
    /// Time-travel: list memories that existed at this RFC3339 timestamp.
    #[serde(default)]
    pub at: Option<String>,
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    /// Server version (uteke-server crate version), so HTTP clients can gate
    /// features on the actual server capability rather than a local CLI probe.
    pub version: &'static str,
    pub memories: usize,
    pub namespaces: usize,
    /// Supported API versions (#737).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_versions: Option<Vec<&'static str>>,
    /// Latest API version (#737).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_latest: Option<&'static str>,
    /// Latest version available on GitHub, if newer than current.
    /// Populated from cache (24h TTL) — may be `None` if cache is stale.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_available: Option<String>,
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub fn default_limit() -> usize {
    5
}

/// Document listings are not paginated like memories — callers (e.g. the Corin
/// doc tree) need the full set to build the hierarchy client-side. Default high
/// so omitting `limit` does not silently cap the tree at 5 docs.
pub fn default_doc_limit() -> usize {
    1000
}
#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct RoomRecallRequest {
    pub room_id: String,
    /// Semantic search query. When `None` or empty, falls back to
    /// chronological recall (equivalent to `GET /room/memories`) (#785).
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub min_score: Option<f32>,
}

pub fn default_limit_search() -> usize {
    10
}

// ── Helpers ─────────────────────────────────────────────────────────────────

pub fn json_header() -> Header {
    Header::from_bytes("Content-Type", "application/json").unwrap()
}

pub fn read_body<T: serde::de::DeserializeOwned>(reader: &mut dyn IoRead) -> Result<T, String> {
    // Enforce payload size limit at the reader level — works regardless of
    // Content-Length header presence (handles chunked transfer, missing header, etc.)
    let mut limited = reader.take(uteke_core::MAX_PAYLOAD_SIZE as u64 + 1);
    let mut body = String::new();
    limited
        .read_to_string(&mut body)
        .map_err(|e| format!("Failed to read body: {e}"))?;
    if body.len() > uteke_core::MAX_PAYLOAD_SIZE {
        return Err(format!(
            "Payload too large: {} bytes (max {})",
            body.len(),
            uteke_core::MAX_PAYLOAD_SIZE
        ));
    }
    serde_json::from_str(&body).map_err(|e| format!("Invalid JSON: {e}"))
}

/// Decode percent-encoded URL query values (e.g. `%20` → space, `+` → space).
/// Handles multi-byte UTF-8 sequences correctly.
pub fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                decoded.push(b' ');
            }
            b'%' if i + 3 <= bytes.len() => {
                // Parse hex pair directly from raw bytes to avoid UTF-8 issues.
                let h1 = (bytes[i + 1] as char).to_digit(16);
                let h2 = (bytes[i + 2] as char).to_digit(16);
                if let (Some(h1), Some(h2)) = (h1, h2) {
                    decoded.push((h1 * 16 + h2) as u8);
                    i += 2;
                } else {
                    decoded.push(b'%');
                }
            }
            c => decoded.push(c),
        }
        i += 1;
    }
    String::from_utf8(decoded).unwrap_or_else(|_| s.to_string())
}

/// Parse a query parameter value from a query string like `"namespace=foo&bar=1"`.
pub fn parse_query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let mut kv = pair.splitn(2, '=');
        if kv.next()? == key {
            Some(url_decode(kv.next()?))
        } else {
            None
        }
    })
}

/// Extract `?namespace=` from a full path like `"/room/list?namespace=foo"`.
pub fn parse_query_namespace(path: &str) -> Option<String> {
    let query = path.split('?').nth(1)?;
    parse_query_param(query, "namespace")
}

pub fn default_namespace() -> String {
    "default".to_string()
}

pub fn ns(ns: &Option<String>) -> Option<&str> {
    ns.as_deref()
}

// ── Config Types (shared across modules) ─────────────────────────────────────

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(serde::Deserialize, Default, Clone)]
pub struct RecallFileSection {
    /// Minimum cosine similarity score for recall results.
    pub min_score: Option<f64>,
    /// Strict mode threshold (higher, for critical queries).
    pub min_score_strict: Option<f64>,
    /// Default recall strategy when a request omits `strategy` (#1034).
    /// One of: vector | fts5 | hybrid | graph. Server-side default: hybrid.
    pub default_strategy: Option<String>,
}

// ── Constants ────────────────────────────────────────────────────────────────

/// Hard cap on recall/list `limit` to prevent DoS via unbounded queries (#903).
pub const MAX_LIMIT: usize = 100;
/// Default strict mode threshold for server recall.
/// Used as fallback when [recall] min_score_strict is not configured.
pub const STRICT_THRESHOLD: f32 = 0.5;
/// Default minimum score for server recall.
/// Used as fallback when [recall] min_score is not configured.
pub const DEFAULT_MIN_SCORE: f32 = 0.0;

// ── Tag Management Types ─────────────────────────────────────────────────

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct TagRenameRequest {
    pub old: String,
    #[serde(default)]
    pub namespace: Option<String>,
    pub new: String,
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct TagDeleteRequest {
    pub tag: String,
    #[serde(default)]
    pub namespace: Option<String>,
}

// ── Memory Update Types (#659) ─────────────────────────────────────────────

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct MemoryUpdateRequest {
    /// UUID of the memory to update (required).
    pub id: String,
    /// New content. Triggers embedding regeneration.
    #[serde(default)]
    pub content: Option<String>,
    /// Replace tags entirely with this list.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Replace metadata entirely with this object.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// Set importance score (0.0–1.0).
    #[serde(default)]
    pub importance: Option<f64>,
    /// Set pinned state.
    #[serde(default)]
    pub pinned: Option<bool>,
    /// Set memory type (fact, procedure, preference, decision, context, note, insight, reference, event).
    #[serde(default)]
    pub memory_type: Option<String>,
}

// ── Pin Types ─────────────────────────────────────────────────────────────

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct PinRequest {
    pub id: String,
}

// ── Memory Mutation Types (#660) ───────────────────────────────────────────

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct MemoryPinRequest {
    pub id: String,
    pub pinned: bool,
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct MemoryImportanceRequest {
    pub id: String,
    pub importance: f64,
}

/// Request for memory feedback / trust scoring (#718).
#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct MemoryFeedbackRequest {
    pub id: String,
    /// "helpful" or "unhelpful"
    pub feedback: String,
}

// ── Graph Types ────────────────────────────────────────────────────────────

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct GraphEdgeRequest {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub edge_type: Option<String>,
    #[serde(default)]
    pub weight: Option<f64>,
}

// ── Extract Types ──────────────────────────────────────────────────────────

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct ExtractRequest {
    pub content: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub r#type: Option<String>,
    /// Override extraction model (else config default).
    #[serde(default)]
    pub model: Option<String>,
    /// Override max facts per document.
    #[serde(default)]
    pub max_facts: Option<usize>,
}

// ── Import Types ──────────────────────────────────────────────────────────

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct ImportRequest {
    /// JSONL content to import.
    pub content: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    #[allow(dead_code)] // not yet merged into JSONL entries on import
    pub tags: Vec<String>,
}

// ── Room Remember Types (#762) ────────────────────────────────────────

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct RoomRememberRequest {
    pub room_id: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// Author — stored as participant role in room link.
    #[serde(default)]
    pub author: Option<String>,
}

// ── Maintenance Types (#607) ──────────────────────────────────────────────

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct PruneRequest {
    #[serde(default = "default_prune_ttl")]
    pub ttl_days: u32,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub namespace: Option<String>,
}

fn default_prune_ttl() -> u32 {
    30
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct ConsolidateRequest {
    #[serde(
        default = "default_consolidate_threshold",
        deserialize_with = "flex_f32"
    )]
    pub threshold: f32,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub namespace: Option<String>,
}

fn default_consolidate_threshold() -> f32 {
    0.9
}

/// Deserialize an `f32` from either a number or a JSON string.
/// Some MCP layers (e.g. Hermes) serialize all parameters as strings,
/// so accepting `"0.95"` in addition to `0.95` avoids 400 errors
/// from downstream integrations.
fn flex_f32<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    use serde::de::{self, Visitor};
    use std::fmt;

    struct FlexF32;

    impl Visitor<'_> for FlexF32 {
        type Value = f32;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an f32 or a string representation of an f32")
        }

        fn visit_f32<E: de::Error>(self, v: f32) -> Result<f32, E> {
            Ok(v)
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<f32, E> {
            Ok(v as f32)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<f32, E> {
            v.parse::<f32>()
                .map_err(|_| de::Error::custom(format!("cannot parse \"{v}\" as f32")))
        }
    }

    d.deserialize_any(FlexF32)
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct AgingRequest {
    #[serde(default = "default_aging_action")]
    pub action: String,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub namespace: Option<String>,
    /// Days threshold for preview/cleanup (default: warm_days from config, fallback 90).
    #[serde(default)]
    pub older_than_days: Option<u32>,
    /// Max access count threshold for preview/cleanup (default: 1).
    #[serde(default)]
    pub max_access_count: Option<u32>,
}

fn default_aging_action() -> String {
    "status".to_string()
}

// ── Monitoring Types (#608) ──────────────────────────────────────────────

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct ImportanceRequest {
    #[serde(default)]
    #[allow(dead_code)] // recompute_importance is global (no namespace filter)
    pub namespace: Option<String>,
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct OrphansRequest {
    #[serde(default = "default_orphan_threshold")]
    pub threshold: f64,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub namespace: Option<String>,
}

fn default_orphan_threshold() -> f64 {
    0.3
}

#[cfg_attr(feature = "docgen", derive(schemars::JsonSchema))]
#[derive(Deserialize)]
pub struct RebuildBacklinksRequest {
    #[serde(default)]
    #[allow(dead_code)] // reserved for future verbose mode
    pub quiet: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_decode_basic() {
        assert_eq!(url_decode("hello"), "hello");
        assert_eq!(url_decode("hello%20world"), "hello world");
        assert_eq!(url_decode("hello+world"), "hello world");
        assert_eq!(url_decode("%41%42%43"), "ABC");
    }

    #[test]
    fn test_url_decode_trailing_percent() {
        // Truncated percent-encoding at end of string — should not panic.
        assert_eq!(url_decode("hello%4"), "hello%4");
        assert_eq!(url_decode("hello%"), "hello%");
        // Valid %XX at exact end of string.
        assert_eq!(url_decode("hello%21"), "hello!");
    }

    #[test]
    fn test_url_decode_utf8() {
        // %C3%A9 = é (2-byte UTF-8).
        assert_eq!(url_decode("caf%C3%A9"), "café");
    }
}
