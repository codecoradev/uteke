//! Request/response structs, helpers, and constants for the uteke server.

use std::io::Read as IoRead;

use serde::{Deserialize, Serialize};
use tiny_http::Header;

// ── Document Types ──────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DocCreateRequest {
    pub slug: String,
    pub title: Option<String>,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
}

#[derive(Deserialize)]
pub struct DocGetRequest {
    pub id: Option<String>,
    pub slug: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Deserialize)]
pub struct DocListParams {
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub roots_only: bool,
    #[serde(default)]
    pub parent: Option<String>,
}

#[derive(Deserialize)]
pub struct DocSearchRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default = "default_search_mode")]
    pub mode: String,
}

#[derive(Deserialize)]
pub struct DocMoveRequest {
    pub id: Option<String>,
    pub slug: Option<String>,
    #[serde(default)]
    pub new_parent: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
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

// ── Types ───────────────────────────────────────────────────────────────────

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
}

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
    /// Minimum similarity score. Results below are filtered.
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
}

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

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub memories: usize,
    pub namespaces: usize,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub fn default_limit() -> usize {
    5
}
#[derive(Deserialize)]
pub struct RoomRecallRequest {
    pub room_id: String,
    pub query: String,
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
            b'%' if i + 2 < bytes.len() => {
                let hex = &s[i + 1..i + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    decoded.push(byte);
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

#[derive(serde::Deserialize, Default, Clone)]
pub struct RecallFileSection {
    /// Minimum cosine similarity score for recall results.
    pub min_score: Option<f64>,
    /// Strict mode threshold (higher, for critical queries).
    pub min_score_strict: Option<f64>,
}

// ── Constants ────────────────────────────────────────────────────────────────

/// Default strict mode threshold for server recall.
/// Used as fallback when [recall] min_score_strict is not configured.
pub const STRICT_THRESHOLD: f32 = 0.5;
/// Default minimum score for server recall.
/// Used as fallback when [recall] min_score is not configured.
pub const DEFAULT_MIN_SCORE: f32 = 0.0;
