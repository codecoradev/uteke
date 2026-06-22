//! Uteke HTTP Server — persistent warm memory for AI agents.
//!
//! Keeps the embedding model loaded in RAM for <50ms recall.
//! Usage: `uteke-serve [--port 8767] [--host 127.0.0.1] [--auth-token <TOKEN>]`

use std::io::{Cursor, Read as IoRead};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use tracing::{error, info, warn};
use uteke_core::Uteke;

// ── Document Types ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct DocCreateRequest {
    slug: String,
    title: Option<String>,
    content: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    parent: Option<String>,
}

#[derive(Deserialize)]
struct DocGetRequest {
    id: Option<String>,
    slug: Option<String>,
    #[serde(default)]
    namespace: Option<String>,
}

#[derive(Deserialize)]
struct DocListParams {
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    roots_only: bool,
    #[serde(default)]
    parent: Option<String>,
}

#[derive(Deserialize)]
struct DocSearchRequest {
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default = "default_search_mode")]
    mode: String,
}

#[derive(Deserialize)]
struct DocMoveRequest {
    id: Option<String>,
    slug: Option<String>,
    #[serde(default)]
    new_parent: Option<String>,
    #[serde(default)]
    namespace: Option<String>,
}

fn default_search_mode() -> String {
    "hybrid".to_string()
}

// ── Document endpoint helpers ─────────────────────────────────────────

fn resolve_doc_id(req: &DocGetRequest) -> Result<&str, &'static str> {
    match (&req.id, &req.slug) {
        (Some(id), _) => Ok(id),
        (_, Some(slug)) => Ok(slug),
        _ => Err("provide either 'id' or 'slug'"),
    }
}

fn resolve_doc_id_move(req: &DocMoveRequest) -> Result<&str, &'static str> {
    match (&req.id, &req.slug) {
        (Some(id), _) => Ok(id),
        (_, Some(slug)) => Ok(slug),
        _ => Err("provide either 'id' or 'slug'"),
    }
}

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RememberRequest {
    content: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    valid_from: Option<String>,
    #[serde(default)]
    valid_until: Option<String>,
    #[serde(default)]
    detect_contradiction: bool,
}

#[derive(Deserialize)]
struct RecallRequest {
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    namespace: Option<String>,
    /// Filter by entity metadata.
    #[serde(default)]
    entity: Option<String>,
    /// Filter by category metadata.
    #[serde(default)]
    category: Option<String>,
    /// Minimum similarity score. Results below are filtered.
    #[serde(default)]
    min_score: Option<f32>,
    /// Use strict threshold (defaults to 0.5 if min_score not set).
    #[serde(default)]
    strict: bool,
    /// Time-travel: query memories that existed at this RFC3339 timestamp.
    #[serde(default)]
    at: Option<String>,
}

#[derive(Deserialize)]
struct SearchRequest {
    query: String,
    #[serde(default = "default_limit_search")]
    limit: usize,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    namespace: Option<String>,
}

#[derive(Deserialize)]
struct ListParams {
    #[serde(default)]
    tag: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    namespace: Option<String>,
    /// Time-travel: list memories that existed at this RFC3339 timestamp.
    #[serde(default)]
    at: Option<String>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    memories: usize,
    namespaces: usize,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

fn default_limit() -> usize {
    5
}
#[derive(Deserialize)]
struct RoomRecallRequest {
    room_id: String,
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    min_score: Option<f32>,
}

fn default_limit_search() -> usize {
    10
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn json_header() -> Header {
    Header::from_bytes("Content-Type", "application/json").unwrap()
}

/// Build a 401 Unauthorized response with CORS and WWW-Authenticate headers.
fn unauthorized_response(
    ctx: &ReqCtx,
    req: &Request,
    error_msg: &str,
) -> Response<Cursor<Vec<u8>>> {
    let mut hdrs = ctx.cors_headers_for(req);
    hdrs.push(Header::from_bytes("WWW-Authenticate", "Bearer realm=\"uteke\"").unwrap());
    hdrs.push(json_header());
    let body = ErrorResponse {
        error: error_msg.to_string(),
    };
    let data = serde_json::to_string(&body).unwrap();
    Response::new(
        StatusCode::from(401),
        hdrs,
        Cursor::new(data.into_bytes()),
        None,
        None,
    )
}

/// Check bearer token auth on a request (#409: dual-role tokens).
/// Returns the role the request is authenticated as.
fn check_auth(req: &Request, ctx: &ReqCtx) -> Result<AuthResult, Response<Cursor<Vec<u8>>>> {
    // No tokens configured — auth disabled.
    if ctx.auth_token_hash.is_none() && ctx.read_only_token_hash.is_none() {
        return Ok(AuthResult::Disabled);
    }

    // Look for Authorization: Bearer <token>
    let auth_header = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Authorization"));

    let token = match auth_header {
        Some(h) => {
            let val = h.value.as_str().trim();
            let parts: Vec<&str> = val.split_whitespace().collect();
            if parts.len() != 2 {
                return Err(unauthorized_response(
                    ctx,
                    req,
                    "Invalid auth header format. Use: Authorization: Bearer <token>",
                ));
            }
            if !parts[0].eq_ignore_ascii_case("Bearer") {
                return Err(unauthorized_response(
                    ctx,
                    req,
                    "Invalid auth scheme. Use: Authorization: Bearer <token>",
                ));
            }
            parts[1]
        }
        None => {
            return Err(unauthorized_response(
                ctx,
                req,
                "Authentication required. Provide Authorization: Bearer <token>",
            ));
        }
    };

    // Check against admin token first, then read-only token.
    let provided_hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();

    if let Some(admin_hash) = &ctx.auth_token_hash {
        if constant_time_eq_digest(&provided_hash, admin_hash) {
            return Ok(AuthResult::Authenticated(ApiRole::Admin));
        }
    }

    if let Some(ro_hash) = &ctx.read_only_token_hash {
        if constant_time_eq_digest(&provided_hash, ro_hash) {
            return Ok(AuthResult::Authenticated(ApiRole::ReadOnly));
        }
    }

    Err(unauthorized_response(ctx, req, "Invalid or expired token"))
}

/// Constant-time comparison of two fixed-length SHA-256 digests.
/// The configured token's digest is precomputed at startup,
/// so only the incoming token is hashed per-request.
fn constant_time_eq_digest(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut result: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

/// API key role (#409).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ApiRole {
    /// Full access — all endpoints (default token).
    Admin,
    /// Read-only access — GET endpoints only (recall, search, list, stats, graph).
    ReadOnly,
}

/// Result of authentication: which role this request is authorized as.
enum AuthResult {
    /// Auth disabled — full access.
    Disabled,
    /// Authenticated with a specific role.
    Authenticated(ApiRole),
}

#[derive(Clone)]
struct ReqCtx {
    /// Hashed admin auth token for constant-time comparison.
    /// If None, auth is disabled.
    auth_token_hash: Option<[u8; 32]>,
    /// Hashed read-only token (#409). Read-only requests can use this.
    read_only_token_hash: Option<[u8; 32]>,
    /// Allowed CORS origins from config. Empty = wildcard.
    cors_origins: Vec<String>,
    /// Recall threshold config from [recall] section in uteke.toml.
    recall_config: Option<RecallFileSection>,
}

impl ReqCtx {
    /// Resolve the allowed origin for a specific request by matching
    /// its `Origin` header against the configured origins list.
    /// Returns "*" if no origins configured (backward compatible).
    /// Returns the matching origin if found, or "*" as fallback.
    fn resolve_origin_for(&self, req: &Request) -> String {
        if self.cors_origins.is_empty() {
            return "*".to_string();
        }
        // Check if request has an Origin header
        if let Some(origin_header) = req.headers().iter().find(|h| h.field.equiv("Origin")) {
            let origin = origin_header.value.as_str();
            if self.cors_origins.iter().any(|o| o == origin) {
                return origin.to_string();
            }
        }
        // No matching origin — return empty string so CORS headers are omitted.
        // Browser will block cross-origin requests from untrusted origins.
        // Non-browser clients (API users) are unaffected by CORS.
        String::new()
    }

    fn cors_headers_for(&self, req: &Request) -> Vec<Header> {
        let origin = self.resolve_origin_for(req);
        if origin.is_empty() {
            // Origin not in allowlist — omit CORS headers entirely.
            // Browser will block cross-origin reads.
            return vec![];
        }
        // When auth is enabled but CORS is wildcard, omit Authorization
        // from allowed headers to prevent browser-origin auth abuse.
        let allowed_headers = if self.auth_token_hash.is_some() && self.cors_origins.is_empty() {
            "Content-Type"
        } else {
            "Content-Type, Authorization"
        };
        vec![
            Header::from_bytes("Access-Control-Allow-Origin", origin).unwrap(),
            Header::from_bytes("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS")
                .unwrap(),
            Header::from_bytes("Access-Control-Allow-Headers", allowed_headers).unwrap(),
        ]
    }

    /// Build CORS headers for preflight, validating requested headers
    /// against an explicit allowlist.
    fn preflight_headers(&self, req: &Request) -> Vec<Header> {
        let origin = self.resolve_origin_for(req);
        if origin.is_empty() {
            // Origin not in allowlist — return minimal headers so browser blocks.
            return vec![];
        }
        // Fixed allowlist of headers we accept in cross-origin requests
        // When auth is enabled but CORS is wildcard, restrict to prevent browser abuse
        let allowed_headers_set: &[&str] =
            if self.auth_token_hash.is_some() && self.cors_origins.is_empty() {
                &["Content-Type", "Accept", "X-Requested-With"]
            } else {
                &[
                    "Content-Type",
                    "Authorization",
                    "Accept",
                    "X-Requested-With",
                ]
            };
        let allow_headers = req
            .headers()
            .iter()
            .find(|h| h.field.equiv("Access-Control-Request-Headers"))
            .map(|h| {
                // Only echo back headers that are in our allowlist
                h.value
                    .as_str()
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| {
                        allowed_headers_set
                            .iter()
                            .any(|a| a.eq_ignore_ascii_case(s))
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(String::new);
        // If no requested headers matched the allowlist, browser gets no
        // Access-Control-Allow-Headers — it will block the request as intended.
        vec![
            Header::from_bytes("Access-Control-Allow-Origin", origin).unwrap(),
            Header::from_bytes("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS")
                .unwrap(),
            Header::from_bytes("Access-Control-Allow-Headers", allow_headers).unwrap(),
        ]
    }

    /// Build an error response with CORS headers specific to a request.
    fn error_response_for(
        &self,
        req: &Request,
        status: u16,
        msg: impl Into<String>,
    ) -> Response<Cursor<Vec<u8>>> {
        let body = ErrorResponse { error: msg.into() };
        let data = serde_json::to_string(&body).unwrap();
        let mut headers = self.cors_headers_for(req);
        headers.push(json_header());
        Response::new(
            StatusCode::from(status),
            headers,
            Cursor::new(data.into_bytes()),
            None,
            None,
        )
    }

    /// Build an OK response with CORS headers specific to a request.
    fn ok_response_for<T: Serialize>(&self, req: &Request, body: &T) -> Response<Cursor<Vec<u8>>> {
        let data = serde_json::to_string(body)
            .unwrap_or_else(|e| serde_json::json!({"error": e.to_string()}).to_string());
        let mut headers = self.cors_headers_for(req);
        headers.push(json_header());
        Response::new(
            StatusCode::from(200),
            headers,
            Cursor::new(data.into_bytes()),
            None,
            None,
        )
    }
}

fn read_body<T: serde::de::DeserializeOwned>(reader: &mut dyn IoRead) -> Result<T, String> {
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
fn url_decode(s: &str) -> String {
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
fn parse_query_param(query: &str, key: &str) -> Option<String> {
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
fn parse_query_namespace(path: &str) -> Option<String> {
    let query = path.split('?').nth(1)?;
    parse_query_param(query, "namespace")
}

fn default_namespace() -> String {
    "default".to_string()
}

fn ns(ns: &Option<String>) -> Option<&str> {
    ns.as_deref()
}

// ── Router ──────────────────────────────────────────────────────────────────

fn route(uteke: &Mutex<Uteke>, ctx: &ReqCtx, req: &mut Request) -> Response<Cursor<Vec<u8>>> {
    let method = req.method().clone();
    let path = req.url().to_string();

    // CORS preflight — no auth required
    if method == Method::Options {
        return Response::new(
            StatusCode::from(204),
            ctx.preflight_headers(req),
            Cursor::new(Vec::new()),
            None,
            None,
        );
    }

    // Health endpoint — no auth required (useful for load balancers)
    let is_health = matches!((&method, path.as_str()), (Method::Get, "/health"));

    // Authenticate all non-health requests
    let auth_role = if !is_health {
        match check_auth(req, ctx) {
            Ok(role) => role,
            Err(resp) => return resp,
        }
    } else {
        AuthResult::Disabled
    };

    // Enforce read-only restriction (#409): ReadOnly tokens can only use GET.
    if let AuthResult::Authenticated(ApiRole::ReadOnly) = auth_role {
        if method != Method::Get {
            return ctx.error_response_for(
                req,
                403,
                "Read-only token cannot perform write operations",
            );
        }
    }

    // Lock the Uteke instance for the duration of this request.
    // This serializes requests but prevents data races on the SQLite connection.
    // Future: use rwlock for read-heavy workloads.
    let uteke = match uteke.lock() {
        Ok(u) => u,
        Err(e) => {
            return ctx.error_response_for(req, 500, format!("Internal error: {e}").as_str());
        }
    };

    match (method, path.as_str()) {
        // ── Health ──────────────────────────────────────────────────────
        (Method::Get, "/health") => {
            let total = uteke.count(None).unwrap_or(0);
            let namespaces = uteke.list_namespaces().unwrap_or_default().len();
            ctx.ok_response_for(
                req,
                &HealthResponse {
                    status: "ok",
                    memories: total,
                    namespaces,
                },
            )
        }

        // ── Remember ───────────────────────────────────────────────────
        (Method::Post, "/remember") => match read_body::<RememberRequest>(req.as_reader()) {
            Ok(req_data) => {
                // Validate input
                if let Err(e) = uteke_core::validate_input(&req_data.content, &req_data.tags) {
                    return ctx.error_response_for(req, 400, e.to_string());
                }

                let tag_refs: Vec<&str> = req_data.tags.iter().map(|s| s.as_str()).collect();

                // Build metadata from optional fields
                let mut meta = serde_json::Map::new();
                if let Some(t) = &req_data.r#type {
                    meta.insert("type".into(), serde_json::Value::String(t.clone()));
                }
                if let Some(vf) = &req_data.valid_from {
                    meta.insert("valid_from".into(), serde_json::Value::String(vf.clone()));
                }
                if let Some(vu) = &req_data.valid_until {
                    meta.insert("valid_until".into(), serde_json::Value::String(vu.clone()));
                }
                let metadata = if meta.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::Object(meta))
                };

                let result = if req_data.detect_contradiction {
                    uteke
                        .remember_with_contradiction(
                            &req_data.content,
                            &tag_refs,
                            ns(&req_data.namespace),
                            req_data.r#type.as_deref(),
                            true,
                            0.65,
                        )
                        .map(|(id, _)| id)
                } else {
                    uteke.remember(
                        &req_data.content,
                        &tag_refs,
                        metadata,
                        ns(&req_data.namespace),
                    )
                };

                match result {
                    Ok(id) => ctx.ok_response_for(req, &serde_json::json!({"id": id})),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Recall (semantic search) ────────────────────────────────────
        (Method::Post, "/recall") => match read_body::<RecallRequest>(req.as_reader()) {
            Ok(req_data) => {
                let tag_refs: Vec<&str> = req_data.tags.iter().map(|s| s.as_str()).collect();
                let tags_filter = if tag_refs.is_empty() {
                    None
                } else {
                    Some(tag_refs.as_slice())
                };
                // Resolve threshold: min_score > strict (→ from config or default 0.5) > 0.0
                // Server reads [recall] section from uteke.toml, matching CLI behavior.
                let min_score = if req_data.strict {
                    req_data.min_score.unwrap_or(
                        ctx.recall_config
                            .as_ref()
                            .and_then(|r| r.min_score_strict)
                            .unwrap_or(STRICT_THRESHOLD as f64) as f32,
                    )
                } else {
                    req_data.min_score.unwrap_or(
                        ctx.recall_config
                            .as_ref()
                            .and_then(|r| r.min_score)
                            .unwrap_or(DEFAULT_MIN_SCORE as f64) as f32,
                    )
                };
                // Strategy: when entity/category filters are present,
                // recall WITHOUT min_score to avoid discarding valid matches
                // that might satisfy metadata but be ranked lower. Apply
                // min_score after metadata filtering.
                let has_meta_filter = req_data.entity.is_some() || req_data.category.is_some();
                let fetch_min_score = if has_meta_filter { 0.0 } else { min_score };
                let fetch_limit = if has_meta_filter {
                    // Cap at 200 to prevent unbounded amplification.
                    // May return fewer than requested when matches are sparse.
                    (req_data.limit * 10).min(200)
                } else {
                    req_data.limit
                };

                // Time-travel mode: parse --at and use recall_at_time
                let point_in_time = match req_data.at.as_deref() {
                    Some(at_str) => match chrono::DateTime::parse_from_rfc3339(at_str) {
                        Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
                        Err(_) => {
                            return ctx.error_response_for(
                                    req,
                                    400,
                                    format!(
                                        "Invalid 'at' timestamp: {at_str}. Use RFC3339 format (e.g. 2026-06-01T12:00:00Z)"
                                    ),
                                );
                        }
                    },
                    None => None,
                };

                let recall_result = if let Some(pit) = point_in_time {
                    uteke.recall_at_time(
                        &req_data.query,
                        fetch_limit,
                        tags_filter,
                        ns(&req_data.namespace),
                        pit,
                        fetch_min_score,
                    )
                } else {
                    uteke.recall(
                        &req_data.query,
                        fetch_limit,
                        tags_filter,
                        ns(&req_data.namespace),
                        fetch_min_score,
                    )
                };

                match recall_result {
                    Ok(raw_results) => {
                        // Post-filter by entity/category metadata
                        let mut results: Vec<_> = raw_results
                            .into_iter()
                            .filter(|sr| {
                                if let Some(ent) = &req_data.entity {
                                    let matches = sr
                                        .memory
                                        .metadata
                                        .get("entity")
                                        .and_then(|v| v.as_str())
                                        .is_some_and(|e| e == ent);
                                    if !matches {
                                        return false;
                                    }
                                }
                                if let Some(cat) = &req_data.category {
                                    let matches = sr
                                        .memory
                                        .metadata
                                        .get("category")
                                        .and_then(|v| v.as_str())
                                        .is_some_and(|c| c == cat);
                                    if !matches {
                                        return false;
                                    }
                                }
                                true
                            })
                            .collect::<Vec<_>>();
                        // Apply min_score filter after metadata filtering
                        // (deferred from recall call to avoid losing valid matches)
                        if min_score > 0.0 {
                            results.retain(|sr| sr.score >= min_score);
                        }
                        // Trim to requested limit after filtering
                        results.truncate(req_data.limit);

                        if results.is_empty() && min_score > 0.0 {
                            ctx.ok_response_for(
                                req,
                                &serde_json::json!({
                                    "results": [],
                                    "total": 0,
                                    "threshold": min_score,
                                    "message": "No memories above similarity threshold"
                                }),
                            )
                        } else {
                            ctx.ok_response_for(req, &results)
                        }
                    }
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Search (keyword) ────────────────────────────────────────────
        (Method::Post, "/search") => match read_body::<SearchRequest>(req.as_reader()) {
            Ok(req_data) => {
                let tag_refs: Vec<&str> = req_data.tags.iter().map(|s| s.as_str()).collect();
                let tags_filter = if tag_refs.is_empty() {
                    None
                } else {
                    Some(tag_refs.as_slice())
                };
                match uteke.search(
                    &req_data.query,
                    req_data.limit,
                    tags_filter,
                    ns(&req_data.namespace),
                ) {
                    Ok(results) => ctx.ok_response_for(req, &results),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── List ────────────────────────────────────────────────────────
        (Method::Post, "/list") => match read_body::<ListParams>(req.as_reader()) {
            Ok(req_data) => {
                // Time-travel mode: parse --at and use list_at_time
                let list_result = match req_data.at.as_deref() {
                    Some(at_str) => match chrono::DateTime::parse_from_rfc3339(at_str) {
                        Ok(dt) => {
                            let pit = dt.with_timezone(&chrono::Utc);
                            uteke.list_at_time(
                                req_data.tag.as_deref(),
                                req_data.limit,
                                req_data.offset,
                                ns(&req_data.namespace),
                                pit,
                            )
                        }
                        Err(_) => {
                            return ctx.error_response_for(
                                    req,
                                    400,
                                    format!(
                                        "Invalid 'at' timestamp: {at_str}. Use RFC3339 format (e.g. 2026-06-01T12:00:00Z)"
                                    ),
                                );
                        }
                    },
                    None => uteke.list(
                        req_data.tag.as_deref(),
                        req_data.limit,
                        req_data.offset,
                        ns(&req_data.namespace),
                    ),
                };
                match list_result {
                    Ok(memories) => ctx.ok_response_for(req, &memories),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Forget by ID or tag (DELETE /forget?id=xxx or ?tag=xxx) ────
        (Method::Delete, p) if p == "/forget" || p.starts_with("/forget?") => {
            let query = p.split('?').nth(1).unwrap_or("");
            let params: std::collections::HashMap<String, String> = query
                .split('&')
                .filter_map(|pair| {
                    let mut kv = pair.splitn(2, '=');
                    Some((kv.next()?.to_string(), kv.next()?.to_string()))
                })
                .collect();

            if let Some(id) = params.get("id") {
                // Validate UUID format
                if uuid::Uuid::parse_str(id).is_err() {
                    return ctx.error_response_for(req, 400, format!("Invalid UUID format: {id}"));
                }
                match uteke.forget(id) {
                    Ok(()) => ctx.ok_response_for(req, &serde_json::json!({"forgotten": id})),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            } else if let Some(tag) = params.get("tag") {
                let namespace = params.get("namespace").map(|s| s.as_str());
                match uteke.bulk_forget_by_tag(tag, namespace) {
                    Ok(result) => ctx.ok_response_for(req, &result),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            } else {
                ctx.error_response_for(req, 400, "Provide ?id= or ?tag= parameter")
            }
        }

        // ── Stats (GET = all or ?namespace=<name>) ───────────────────
        (Method::Get, "/stats") => {
            // Parse ?namespace= query parameter for scoped stats (#382).
            let query = req.url().split('?').nth(1).unwrap_or("");
            let params: std::collections::HashMap<String, String> = query
                .split('&')
                .filter_map(|pair| {
                    let mut kv = pair.splitn(2, '=');
                    Some((kv.next()?.to_string(), kv.next()?.to_string()))
                })
                .collect();
            let ns_param = params.get("namespace").map(|s| s.as_str());
            match uteke.stats(ns_param) {
                Ok(stats) => ctx.ok_response_for(req, &stats),
                Err(e) => {
                    error!("Internal error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        (Method::Post, "/stats") => {
            #[derive(Deserialize)]
            struct StatsReq {
                namespace: Option<String>,
            }
            match read_body::<StatsReq>(req.as_reader()) {
                Ok(req_data) => match uteke.stats(ns(&req_data.namespace)) {
                    Ok(stats) => ctx.ok_response_for(req, &stats),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                },
                Err(e) => ctx.error_response_for(req, 400, e),
            }
        }

        // ── Namespaces ──────────────────────────────────────────────────
        (Method::Get, "/namespaces") => match uteke.list_namespaces() {
            Ok(namespaces) => ctx.ok_response_for(req, &namespaces),
            Err(e) => {
                error!("Internal error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },

        // ── Graph Visualization (#408) ───────────────────────────────────
        (Method::Get, p) if p == "/graph" || p.starts_with("/graph?") => {
            let ns = parse_query_namespace(&path);
            match uteke.graph_data(ns.as_deref()) {
                Ok(data) => ctx.ok_response_for(req, &data),
                Err(e) => {
                    error!("Graph data error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }

        // ── Room Summary ────────────────────────────────────────────────
        (Method::Post, "/room/summary") => {
            #[derive(Deserialize)]
            struct RoomSummaryRequest {
                room_id: String,
            }
            match read_body::<RoomSummaryRequest>(req.as_reader()) {
                Ok(req_data) => match uteke.room_summary(&req_data.room_id) {
                    Ok(Some(summary)) => ctx.ok_response_for(req, &summary),
                    Ok(None) => ctx.error_response_for(
                        req,
                        404,
                        format!("Room not found: {}", req_data.room_id),
                    ),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                },
                Err(e) => ctx.error_response_for(req, 400, e),
            }
        }

        // ── Room Document ────────────────────────────────────────────────
        (Method::Post, "/room/document") => {
            #[derive(Deserialize)]
            struct RoomDocumentRequest {
                room_id: String,
            }
            match read_body::<RoomDocumentRequest>(req.as_reader()) {
                Ok(req_data) => match uteke.room_document(&req_data.room_id) {
                    Ok(Some(doc)) => ctx.ok_response_for(req, &doc),
                    Ok(None) => ctx.error_response_for(
                        req,
                        404,
                        format!("Room not found: {}", req_data.room_id),
                    ),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                },
                Err(e) => ctx.error_response_for(req, 400, e),
            }
        }

        // ── Get memory by ID ──────────────────────────────────────────
        (Method::Get, p) if p.starts_with("/memory?id=") => {
            let id = p.trim_start_matches("/memory?id=");
            // Validate UUID format
            if uuid::Uuid::parse_str(id).is_err() {
                return ctx.error_response_for(req, 400, format!("Invalid UUID format: {id}"));
            }
            match uteke.get_by_id(id) {
                Ok(Some(memory)) => ctx.ok_response_for(req, &memory),
                Ok(None) => ctx.error_response_for(req, 404, format!("Memory not found: {id}")),
                Err(e) => {
                    error!("Internal error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }

        // ── Room Recall (semantic) ────────────────────────────────────────
        (Method::Post, "/room/recall") => match read_body::<RoomRecallRequest>(req.as_reader()) {
            Ok(req_data) => {
                let min_score = req_data.min_score.unwrap_or(
                    ctx.recall_config
                        .as_ref()
                        .and_then(|r| r.min_score)
                        .unwrap_or(DEFAULT_MIN_SCORE as f64) as f32,
                );
                match uteke.recall_room_semantic(
                    &req_data.room_id,
                    &req_data.query,
                    req_data.limit,
                    req_data.author.as_deref(),
                    min_score,
                ) {
                    Ok(results) => ctx.ok_response_for(req, &results),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Room management endpoints (#395) ────────────────────────────
        (Method::Post, "/room/create") => {
            #[derive(Deserialize)]
            struct RoomCreateRequest {
                room_id: String,
                #[serde(default)]
                title: Option<String>,
                #[serde(default = "default_namespace")]
                namespace: String,
            }
            match read_body::<RoomCreateRequest>(req.as_reader()) {
                Ok(req_data) => {
                    match uteke.create_room(
                        &req_data.room_id,
                        req_data.title.as_deref(),
                        &req_data.namespace,
                    ) {
                        Ok(()) => ctx.ok_response_for(
                            req,
                            &serde_json::json!({
                                "created": req_data.room_id,
                                "namespace": req_data.namespace
                            }),
                        ),
                        Err(e) => {
                            let msg = format!("Failed to create room: {e}");
                            ctx.error_response_for(req, 400, &msg)
                        }
                    }
                }
                Err(e) => ctx.error_response_for(req, 400, e),
            }
        }

        (Method::Get, "/room/list") => {
            let ns_param = parse_query_namespace(&path);
            match uteke.list_rooms(ns_param.as_deref()) {
                Ok(rooms) => ctx.ok_response_for(req, &rooms),
                Err(e) => {
                    error!("Internal error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }

        (Method::Post, "/room/stats") => {
            #[derive(Deserialize)]
            struct RoomStatsRequest {
                room_id: String,
            }
            match read_body::<RoomStatsRequest>(req.as_reader()) {
                Ok(req_data) => match uteke.room_stats(&req_data.room_id) {
                    Ok(Some(stats)) => ctx.ok_response_for(req, &stats),
                    Ok(None) => ctx.error_response_for(
                        req,
                        404,
                        format!("Room not found: {}", req_data.room_id),
                    ),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                },
                Err(e) => ctx.error_response_for(req, 400, e),
            }
        }

        (Method::Delete, p) if p == "/room/delete" || p.starts_with("/room/delete?") => {
            let room_id = if let Some(q) = p.strip_prefix("/room/delete?") {
                parse_query_param(q, "room_id")
            } else {
                // Try reading from query params in headers or body
                None
            };
            let room_id = match room_id {
                Some(id) => id,
                None => {
                    // Try body as JSON
                    #[derive(Deserialize)]
                    struct RoomDeleteRequest {
                        room_id: String,
                    }
                    match read_body::<RoomDeleteRequest>(req.as_reader()) {
                        Ok(data) => data.room_id,
                        Err(_) => {
                            return ctx.error_response_for(req, 400, "Missing 'room_id' parameter")
                        }
                    }
                }
            };
            match uteke.delete_room(&room_id) {
                Ok(()) => ctx.ok_response_for(req, &serde_json::json!({"deleted": room_id})),
                Err(e) => {
                    let msg = format!("{e}");
                    if msg.contains("not found") {
                        ctx.error_response_for(req, 404, &msg)
                    } else {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
        }
        // ── Context Summary (#442) ───────────────────────────────────────
        (Method::Post, "/context") => match read_body::<serde_json::Value>(req.as_reader()) {
            Ok(body) => {
                let ns = body.get("namespace").and_then(|v| v.as_str());
                match uteke.build_context(ns) {
                    Ok(context) => {
                        let resp = serde_json::json!({ "context": context });
                        ctx.ok_response_for(req, &resp)
                    }
                    Err(e) => {
                        error!("Context error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(_) => ctx.error_response_for(req, 400, "Invalid JSON body"),
        },

        // ── Dream Cycle (#442) ─────────────────────────────────────────────
        (Method::Post, "/dream") => match read_body::<serde_json::Value>(req.as_reader()) {
            Ok(body) => {
                let ns = body.get("namespace").and_then(|v| v.as_str());
                let dry_run = body
                    .get("dry_run")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                match uteke.dream(ns, dry_run, &[]) {
                    Ok(report) => ctx.ok_response_for(req, &report),
                    Err(e) => {
                        error!("Dream error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(_) => ctx.error_response_for(req, 400, "Invalid JSON body"),
        },

        (Method::Post, "/mcp") => {
            // Enforce a body size limit to prevent memory exhaustion
            // (CodeCora #397). 1 MiB is generous for JSON-RPC.
            const MAX_MCP_BODY: u64 = 1024 * 1024;
            // Check Content-Length and reject oversized requests.
            let content_length = req
                .headers()
                .iter()
                .find(|h| h.field.as_str() == "content-length")
                .and_then(|h| h.value.as_str().parse::<u64>().ok())
                .unwrap_or(0);
            if content_length > MAX_MCP_BODY {
                return ctx.error_response_for(req, 413, "Payload too large");
            }
            let mut body = String::new();
            if let Err(e) = req.as_reader().take(MAX_MCP_BODY).read_to_string(&mut body) {
                return ctx.error_response_for(req, 400, format!("Failed to read body: {e}"));
            }
            let response = uteke_mcp::handle_jsonrpc(&uteke, &body);
            tiny_http::Response::from_string(response)
                .with_header(
                    tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                        .unwrap(),
                )
                .with_header(
                    tiny_http::Header::from_bytes(&b"MCP-Protocol-Version"[..], &b"2025-06-18"[..])
                        .unwrap(),
                )
        }

        // ── Document: Create / Upsert ────────────────────────────────────
        (Method::Post, "/doc/create") => match read_body::<DocCreateRequest>(req.as_reader()) {
            Ok(req_data) => {
                let tag_refs: Vec<&str> = req_data.tags.iter().map(|s| s.as_str()).collect();
                let parent = req_data.parent.as_deref();
                match uteke.doc_upsert_with_parent(
                    &req_data.slug,
                    req_data.title.as_deref().unwrap_or(""),
                    &req_data.content,
                    &tag_refs,
                    ns(&req_data.namespace),
                    parent,
                ) {
                    Ok(id) => ctx.ok_response_for(
                        req,
                        &serde_json::json!({"id": id, "slug": req_data.slug}),
                    ),
                    Err(e) => {
                        if e.to_string().contains("already exists") {
                            ctx.error_response_for(
                                req,
                                409,
                                format!("document slug '{}' already exists", req_data.slug),
                            )
                        } else if e.to_string().contains("maximum")
                            || e.to_string().contains("parent")
                        {
                            ctx.error_response_for(req, 400, e.to_string())
                        } else {
                            error!("doc create error: {e}");
                            ctx.error_response_for(req, 500, "Internal server error")
                        }
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Document: Get ───────────────────────────────────────────────
        (Method::Post, "/doc/get") => match read_body::<DocGetRequest>(req.as_reader()) {
            Ok(req_data) => match resolve_doc_id(&req_data) {
                Ok(id_or_slug) => match uteke.doc_get(id_or_slug, ns(&req_data.namespace)) {
                    Ok(Some(doc)) => ctx.ok_response_for(req, &doc),
                    Ok(None) => ctx.error_response_for(
                        req,
                        404,
                        format!("document not found: {id_or_slug}"),
                    ),
                    Err(e) => {
                        error!("doc get error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                },
                Err(e) => ctx.error_response_for(req, 400, e),
            },
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Document: List ─────────────────────────────────────────────
        (Method::Post, "/doc/list") => match read_body::<DocListParams>(req.as_reader()) {
            Ok(params) => {
                let result = if params.roots_only {
                    uteke.doc_list_roots(ns(&params.namespace), params.limit)
                } else if let Some(ref parent) = params.parent {
                    uteke.doc_list_children(parent, ns(&params.namespace), params.limit)
                } else {
                    uteke.doc_list(ns(&params.namespace), params.limit)
                };
                match result {
                    Ok(docs) => ctx.ok_response_for(req, &docs),
                    Err(e) => {
                        error!("doc list error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Document: Search ────────────────────────────────────────────
        (Method::Post, "/doc/search") => match read_body::<DocSearchRequest>(req.as_reader()) {
            Ok(req_data) => {
                match uteke.doc_search(
                    &req_data.query,
                    ns(&req_data.namespace),
                    req_data.limit,
                    &req_data.mode,
                ) {
                    Ok(results) => ctx.ok_response_for(req, &results),
                    Err(e) => {
                        if e.to_string().contains("embed") {
                            ctx.error_response_for(
                                req,
                                503,
                                "embedding model not available for semantic search",
                            )
                        } else {
                            error!("doc search error: {e}");
                            ctx.error_response_for(req, 500, "Internal server error")
                        }
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Document: Move ───────────────────────────────────────────────
        (Method::Post, "/doc/move") => match read_body::<DocMoveRequest>(req.as_reader()) {
            Ok(req_data) => match resolve_doc_id_move(&req_data) {
                Ok(id_or_slug) => {
                    let new_parent = req_data.new_parent.as_deref();
                    match uteke.doc_move(id_or_slug, new_parent, ns(&req_data.namespace)) {
                        Ok(moved) => ctx.ok_response_for(req, &serde_json::json!({"moved": moved})),
                        Err(e) => {
                            if e.to_string().contains("not found") {
                                ctx.error_response_for(req, 404, e.to_string())
                            } else {
                                error!("doc move error: {e}");
                                ctx.error_response_for(req, 500, "Internal server error")
                            }
                        }
                    }
                }
                Err(e) => ctx.error_response_for(req, 400, e),
            },
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Document: Delete ─────────────────────────────────────────────
        (Method::Delete, p) if p == "/doc/delete" || p.starts_with("/doc/delete?") => {
            let url = req.url().to_string();
            let ns_param = parse_query_param(&url, "namespace");
            let id = parse_query_param(&url, "id");
            let slug = parse_query_param(&url, "slug");

            let id_or_slug = match (&id, &slug) {
                (Some(id), _) => id.as_str(),
                (_, Some(slug)) => slug.as_str(),
                _ => {
                    return ctx.error_response_for(
                        req,
                        400,
                        "provide either 'id' or 'slug' query parameter",
                    );
                }
            };

            match uteke.doc_delete(id_or_slug, ns_param.as_deref()) {
                Ok((deleted, subtree)) => ctx.ok_response_for(
                    req,
                    &serde_json::json!({"deleted": deleted, "subtree_size": subtree}),
                ),
                Err(e) => {
                    error!("doc delete error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }

        // ── 404 ─────────────────────────────────────────────────────────
        _ => ctx.error_response_for(req, 404, "Not found"),
    }
}

// ── Main ────────────────────────────────────────────────────────────────────

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Default strict mode threshold for server recall.
/// Used as fallback when [recall] min_score_strict is not configured.
const STRICT_THRESHOLD: f32 = 0.5;
/// Default minimum score for server recall.
/// Used as fallback when [recall] min_score is not configured.
const DEFAULT_MIN_SCORE: f32 = 0.0;

fn main() {
    // Parse CLI args — these override config
    let args: Vec<String> = std::env::args().collect();
    let mut cli_host: Option<String> = None;
    let mut cli_port: Option<u16> = None;
    let mut cli_auth_token: Option<String> = None;
    let mut cli_read_only_token: Option<String> = None;
    let mut cli_cors_origins: Vec<String> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--host" => {
                i += 1;
                if i < args.len() {
                    cli_host = Some(args[i].clone());
                } else {
                    eprintln!("Error: --host requires a value");
                    std::process::exit(1);
                }
            }
            "--port" => {
                i += 1;
                if i < args.len() {
                    cli_port = Some(args[i].parse().unwrap_or_else(|e| {
                        eprintln!("Invalid port: {e}");
                        std::process::exit(1);
                    }));
                } else {
                    eprintln!("Error: --port requires a value");
                    std::process::exit(1);
                }
            }
            "--auth-token" => {
                i += 1;
                if i < args.len() {
                    cli_auth_token = Some(args[i].clone());
                } else {
                    eprintln!("Error: --auth-token requires a value");
                    std::process::exit(1);
                }
            }
            "--read-only-token" => {
                i += 1;
                if i < args.len() {
                    cli_read_only_token = Some(args[i].clone());
                } else {
                    eprintln!("Error: --read-only-token requires a value");
                    std::process::exit(1);
                }
            }
            "--cors-origin" => {
                i += 1;
                if i < args.len() {
                    cli_cors_origins.push(args[i].clone());
                } else {
                    eprintln!("Error: --cors-origin requires a value");
                    std::process::exit(1);
                }
            }
            "--help" | "-h" => {
                println!("uteke-serve — persistent warm memory server");
                println!();
                println!("Usage: uteke-serve [OPTIONS]");
                println!();
                println!("Options:");
                println!("  --host <HOST>        Bind address (default: 127.0.0.1)");
                println!("  --port <PORT>        Port number (default: 8767)");
                println!("  --auth-token <TOKEN> Bearer token for API auth");
                println!("  --cors-origin <URL>  Allowed CORS origin (repeatable)");
                println!("  --read-only-token <T> Read-only API token (GET endpoints only) (#409)");
                println!("  -h, --help           Show this help");
                println!();
                println!("Config: reads [server] section from uteke.toml");
                println!("  CLI args override config values.");
                println!();
                println!("Environment:");
                println!("  UTEKE_HOME          Data directory (default: ~/.uteke)");
                println!("  UTEKE_AUTH_TOKEN     Bearer token (alternative to --auth-token)");
                println!(
                    "  UTEKE_READ_ONLY_TOKEN  Read-only token (alternative to --read-only-token)"
                );
                println!();
                println!("Security:");
                println!("  If --auth-token or UTEKE_AUTH_TOKEN is set, all endpoints");
                println!("  (except GET /health) require Authorization: Bearer <TOKEN>.");
                println!("  --read-only-token grants GET-only access (recall, search, list, stats, graph).");
                println!("  Configure CORS origins in uteke.toml [server].cors_origins.");
                println!();
                println!("API:");
                println!("  GET  /health              → {{ status, memories }}");
                println!("  POST /remember            → {{ content, tags? }} → {{ id }}");
                println!("  POST /recall              → {{ query, limit? }} → {{ results }}");
                println!("  POST /search              → {{ query, limit? }} → {{ results }}");
                println!(
                    "  POST /list                → {{ tag?, limit?, offset? }} → {{ memories }}"
                );
                println!("  DELETE /forget?id=UUID     → {{ forgotten }}");
                println!("  DELETE /forget?tag=TAG     → {{ deleted }}");
                println!("  GET  /memory?id=UUID       → {{ memory }}");
                println!("  GET  /stats               → {{ stats }}");
                println!("  GET  /namespaces           → {{ namespaces }}");
                println!("  POST /room/create          → {{ room_id, title, namespace }} → {{ created }}");
                println!("  GET  /room/list            → [?namespace=] → [rooms]");
                println!("  POST /room/recall          → {{ room_id, query }} → ranked memories");
                println!("  POST /room/summary         → {{ room_id }} → {{ summary }}");
                println!("  POST /room/document        → {{ room_id }} → {{ document }}");
                println!("  POST /room/stats           → {{ room_id }} → room stats");
                println!("  DEL  /room/delete          → {{ room_id }} → {{ deleted }}");
                println!();
                println!("  Document endpoints:");
                println!("  POST /doc/create          → {{ slug, content, title?, tags?, parent? }} → {{ id, slug }}");
                println!("  POST /doc/get              → {{ id | slug }} → {{ document }}");
                println!("  POST /doc/list             → {{ namespace?, limit?, roots_only?, parent? }} → [documents]");
                println!("  POST /doc/search            → {{ query, mode?, namespace?, limit? }} → [results]");
                println!(
                    "  POST /doc/move              → {{ id | slug, new_parent? }} → {{ moved }}"
                );
                println!("  DEL  /doc/delete?id=UUID    → {{ deleted, subtree_size }}");
                std::process::exit(0);
            }
            _ => {
                eprintln!("Unknown argument: {}. Use --help.", args[i]);
                std::process::exit(1);
            }
        }
        i += 1;
    }

    // Load config: defaults → uteke.toml → CLI args (env vars fill gaps where CLI is absent)
    let config = load_uteke_toml();
    let config_host = config
        .server
        .as_ref()
        .and_then(|s| s.host.clone())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let config_port = config.server.as_ref().and_then(|s| s.port).unwrap_or(8767);
    let config_auth_token = config.server.as_ref().and_then(|s| s.auth_token.clone());
    let config_cors_origins = config
        .server
        .as_ref()
        .and_then(|s| s.cors_origins.clone())
        .unwrap_or_default();

    // Merge CORS origins: CLI flags override config
    let cors_origins = if !cli_cors_origins.is_empty() {
        cli_cors_origins
    } else {
        config_cors_origins
    };

    let host = cli_host.unwrap_or(config_host);
    let port = cli_port.unwrap_or(config_port);

    // Auth token precedence: CLI flag > environment variable > config file
    let auth_token = cli_auth_token
        .or_else(|| std::env::var("UTEKE_AUTH_TOKEN").ok())
        .or(config_auth_token);

    // Logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    // Open store
    let home = match uteke_core::uteke_home() {
        Ok(h) => h,
        Err(e) => {
            error!("Failed to determine home directory: {e}");
            std::process::exit(1);
        }
    };
    let db_path = home.join("uteke.db").to_string_lossy().to_string();

    info!("Opening store at: {db_path}");
    let uteke = match Uteke::open(&db_path) {
        Ok(u) => Arc::new(Mutex::new(u)),
        Err(e) => {
            error!("Failed to open store: {e}");
            std::process::exit(1);
        }
    };

    // Precompute auth token hash at startup so only incoming tokens
    // need hashing per-request (avoids double-hash on every auth check).
    let auth_token_hash = auth_token.as_deref().map(|t| Sha256::digest(t).into());

    // Read-only token (#409): CLI arg or env var.
    let read_only_token =
        cli_read_only_token.or_else(|| std::env::var("UTEKE_READ_ONLY_TOKEN").ok());
    let read_only_token_hash = read_only_token.as_deref().map(|t| Sha256::digest(t).into());

    // Build request context
    // Warn if auth is configured but CORS origins are not — this is safe for
    // non-browser clients (curl, SDKs, agents) but risky if browser access is needed.
    if auth_token_hash.is_some() && cors_origins.is_empty() {
        warn!("Security: auth token is set but cors_origins is not configured.");
        warn!("  For browser access, set cors_origins in uteke.toml or --cors-origin.");
        warn!("  Non-browser clients (curl, agents) are unaffected by CORS.");
    }
    let ctx = ReqCtx {
        auth_token_hash,
        read_only_token_hash,
        cors_origins: cors_origins.clone(),
        recall_config: config.recall.clone(),
    };

    // Start server
    let addr = format!("{host}:{port}");
    let server = Server::http(&addr).unwrap_or_else(|e| {
        error!("Failed to bind {addr}: {e}");
        std::process::exit(1);
    });
    info!("Uteke server listening on http://{addr}");
    info!("Embedding model warm. Ready for <50ms recall.");

    // Security info
    if auth_token.is_some() {
        info!("Authentication: enabled (Bearer token)");
    } else {
        warn!("Authentication: disabled — set --auth-token or UTEKE_AUTH_TOKEN for production");
    }
    if read_only_token.is_some() {
        info!("Read-only token: enabled (GET-only access, #409)");
    }
    if cors_origins.is_empty() {
        warn!("CORS: wildcard (*) — restrict cors_origins in uteke.toml for production");
    } else {
        info!("CORS: allowing origins: {:?}", cors_origins);
    }

    // Auto-aging background thread (#442 enhancement).
    // Runs aging cleanup every 6 hours to remove cold, low-importance memories.
    let aging_uteke = Arc::clone(&uteke);
    std::thread::spawn(move || {
        let interval = std::time::Duration::from_secs(6 * 60 * 60); // 6 hours
        loop {
            std::thread::sleep(interval);
            if SHUTDOWN.load(Ordering::SeqCst) {
                break;
            }
            match aging_uteke.lock() {
                Ok(u) => match u.aging_cleanup(180, 10000, None) {
                    Ok(result) => {
                        if result.deleted > 0 {
                            info!("Auto-aging: cleaned up {} stale memories", result.deleted);
                        }
                    }
                    Err(e) => {
                        warn!("Auto-aging failed: {e}");
                    }
                },
                Err(_) => {
                    // Lock contention — skip this cycle.
                    tracing::debug!("Auto-aging: lock busy, skipping cycle");
                }
            }
        }
    });

    // Auto-dream background thread (#442 enhancement).
    // Runs dream cycle every 3 days to maintain graph health.
    let dream_uteke = Arc::clone(&uteke);
    std::thread::spawn(move || {
        let interval = std::time::Duration::from_secs(3 * 24 * 60 * 60); // 3 days
        loop {
            std::thread::sleep(interval);
            if SHUTDOWN.load(Ordering::SeqCst) {
                break;
            }
            match dream_uteke.lock() {
                Ok(u) => match u.dream(None, false, &[]) {
                    Ok(report) => {
                        if report.total_changes > 0 {
                            info!(
                                "Auto-dream: {} changes, {} warnings ({}ms)",
                                report.total_changes, report.total_warnings, report.duration_ms
                            );
                        }
                    }
                    Err(e) => {
                        warn!("Auto-dream failed: {e}");
                    }
                },
                Err(_) => {
                    tracing::debug!("Auto-dream: lock busy, skipping cycle");
                }
            }
        }
    });

    // SIGINT handler
    ctrlc::set_handler(|| {
        if SHUTDOWN.load(Ordering::SeqCst) {
            eprintln!("\nForce exit.");
            std::process::exit(130);
        }
        SHUTDOWN.store(true, Ordering::SeqCst);
        eprintln!("\nShutting down gracefully... (Ctrl+C again to force)");
    })
    .expect("Failed to set SIGINT handler");

    // Request loop — spawn each request in a thread for concurrent handling.
    // Arc<Mutex<Uteke>> allows safe shared access across threads.
    for mut req in server.incoming_requests() {
        if SHUTDOWN.load(Ordering::SeqCst) {
            info!("Shutdown requested, stopping.");
            break;
        }

        let method = req.method().clone();
        let url = req.url().to_string();
        info!("{method} {url}");

        let uteke = Arc::clone(&uteke);
        let ctx = ctx.clone();

        std::thread::spawn(move || {
            let response = route(&uteke, &ctx, &mut req);
            if let Err(e) = req.respond(response) {
                warn!("Response error: {e}");
            }
        });
    }

    // Graceful shutdown
    info!("Saving index and closing DB...");
    if let Err(e) = uteke.lock().expect("shutdown lock").shutdown() {
        error!("Shutdown error: {e}");
    }

    info!("Goodbye.");
}

// ── Config Loading ────────────────────────────────────────────────────────

/// Minimal [server] config section for parsing uteke.toml.
#[derive(serde::Deserialize, Default)]
struct ServerFileConfig {
    server: Option<ServerFileSection>,
    recall: Option<RecallFileSection>,
}

#[derive(serde::Deserialize, Default, Clone)]
struct RecallFileSection {
    /// Minimum cosine similarity score for recall results.
    min_score: Option<f64>,
    /// Strict mode threshold (higher, for critical queries).
    min_score_strict: Option<f64>,
}

#[derive(serde::Deserialize, Default)]
struct ServerFileSection {
    host: Option<String>,
    port: Option<u16>,
    /// Bearer token for API authentication.
    /// If set, all endpoints except GET /health require Authorization: Bearer <token>.
    auth_token: Option<String>,
    /// Allowed CORS origins. Defaults to empty (wildcard `*`).
    /// Set to specific origins like ["http://localhost:3000"] for production.
    /// Each request's `Origin` header is matched against this list.
    cors_origins: Option<Vec<String>>,
}

/// Find and parse the nearest uteke.toml, looking at:
/// 1. $UTEKE_HOME/uteke.toml (or ~/.uteke/uteke.toml)
/// 2. $CWD/.uteke/uteke.toml
fn load_uteke_toml() -> ServerFileConfig {
    let mut config = ServerFileConfig::default();

    let mut paths: Vec<PathBuf> = vec![match uteke_core::uteke_home() {
        Ok(h) => h.join("uteke.toml"),
        Err(_) => PathBuf::new(),
    }];
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join(".uteke").join("uteke.toml"));
    }

    for path in paths {
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(parsed) = toml::from_str::<ServerFileConfig>(&content) {
                    config = parsed;
                }
            }
        }
    }

    config
}
