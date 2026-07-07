//! Auth, CORS, and request context for the uteke server.

use std::io::Cursor;

use sha2::{Digest, Sha256};
use tiny_http::{Header, Request, Response, StatusCode};

use crate::types::{json_header, ErrorResponse, RecallFileSection};

/// Build a 401 Unauthorized response with CORS and WWW-Authenticate headers.
pub fn unauthorized_response(
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
pub fn check_auth(req: &Request, ctx: &ReqCtx) -> Result<AuthResult, Response<Cursor<Vec<u8>>>> {
    // No tokens configured — auth disabled.
    if ctx.auth_token_hash.is_none() && ctx.read_only_token_hash.is_none() {
        return Ok(AuthResult::Disabled);
    }

    // Look for Authorization: Bearer ***
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
                    "Invalid auth header format. Use: Authorization: Bearer ***",
                ));
            }
            if !parts[0].eq_ignore_ascii_case("Bearer") {
                return Err(unauthorized_response(
                    ctx,
                    req,
                    "Invalid auth scheme. Use: Authorization: Bearer ***",
                ));
            }
            parts[1]
        }
        None => {
            return Err(unauthorized_response(
                ctx,
                req,
                "Authentication required. Provide Authorization: Bearer ***",
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
pub fn constant_time_eq_digest(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut result: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

/// API key role (#409).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ApiRole {
    /// Full access — all endpoints (default token).
    Admin,
    /// Read-only access — GET endpoints only (recall, search, list, stats, graph).
    ReadOnly,
}

/// Result of authentication: which role this request is authorized as.
pub enum AuthResult {
    /// Auth disabled — full access.
    Disabled,
    /// Authenticated with a specific role.
    Authenticated(ApiRole),
}

#[derive(Clone)]
pub struct ReqCtx {
    /// Hashed admin auth token for constant-time comparison.
    /// If None, auth is disabled.
    pub auth_token_hash: Option<[u8; 32]>,
    /// Hashed read-only token (#409). Read-only requests can use this.
    pub read_only_token_hash: Option<[u8; 32]>,
    /// Allowed CORS origins from config. Empty = wildcard.
    pub cors_origins: Vec<String>,
    /// Recall threshold config from [recall] section in uteke.toml.
    pub recall_config: Option<RecallFileSection>,
    /// Extraction config from [extraction] section in uteke.toml.
    pub extraction_config: Option<uteke_core::extraction::ExtractionConfig>,
}

impl ReqCtx {
    /// Resolve the allowed origin for a specific request by matching
    /// its `Origin` header against the configured origins list.
    /// Returns "*" if no origins configured (backward compatible).
    /// Returns the matching origin if found, or "*" as fallback.
    pub fn resolve_origin_for(&self, req: &Request) -> String {
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

    pub fn cors_headers_for(&self, req: &Request) -> Vec<Header> {
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
    pub fn preflight_headers(&self, req: &Request) -> Vec<Header> {
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
    pub fn error_response_for(
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
    pub fn ok_response_for<T: serde::Serialize>(
        &self,
        req: &Request,
        body: &T,
    ) -> Response<Cursor<Vec<u8>>> {
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
