//! Auth, CORS, and request context for the uteke server.

use std::io::Cursor;

use sha2::{Digest, Sha256};
use tiny_http::{Header, Request, Response, StatusCode};

use crate::types::{ErrorResponse, RecallFileSection, json_header};

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
    /// No valid token presented. Only the unauthenticated-friendly surface
    /// (health check) is served — with a minimal payload (#1252).
    Anonymous,
}

#[derive(Clone)]
pub struct ReqCtx {
    /// Hashed admin auth token for constant-time comparison.
    /// If None, auth is disabled.
    pub auth_token_hash: Option<[u8; 32]>,
    /// Hashed read-only token (#409). Read-only requests can use this.
    pub read_only_token_hash: Option<[u8; 32]>,
    /// Allowed CORS origins from config. Empty = no CORS headers; `"*"` = wildcard.
    pub cors_origins: Vec<String>,
    /// Recall threshold config from [recall] section in uteke.toml.
    pub recall_config: Option<RecallFileSection>,
    /// Extraction config from [extraction] section in uteke.toml.
    pub extraction_config: Option<uteke_core::extraction::ExtractionConfig>,
    /// `Host` header allowlist against DNS rebinding (#1326).
    pub host_guard: HostGuard,
}

/// `Host` header policy against DNS rebinding (#1326).
///
/// A rebinding page makes the browser resolve an attacker-controlled name to
/// 127.0.0.1, so the request reaches the local server (even with CORS off) but
/// still carries `Host: <attacker name>`. When the server is bound to a
/// loopback address only loopback `Host` values (plus `allowed_hosts`) are
/// accepted. A non-loopback bind (e.g. Docker, where the service name is the
/// `Host`) keeps accepting everything unless `allowed_hosts` is set.
#[derive(Clone, Debug, Default)]
pub struct HostGuard {
    enforce: bool,
    allowed: Vec<String>,
}

impl HostGuard {
    /// Build the policy for a bind address and configured extra hosts.
    pub fn new(bind_host: &str, allowed_hosts: &[String]) -> Self {
        let allowed: Vec<String> = allowed_hosts
            .iter()
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect();
        Self {
            enforce: is_loopback_host(bind_host) || !allowed.is_empty(),
            allowed,
        }
    }

    /// Whether a request's `Host` header value is acceptable. A missing header
    /// (HTTP/1.0 clients) is allowed: rebinding browsers always send one.
    pub fn allows(&self, host_header: Option<&str>) -> bool {
        if !self.enforce {
            return true;
        }
        let Some(raw) = host_header else {
            return true;
        };
        let raw = raw.trim().to_ascii_lowercase();
        let name = host_without_port(&raw);
        is_loopback_host(name) || self.allowed.iter().any(|a| *a == raw || a == name)
    }
}

/// Strip an optional `:port` (and IPv6 brackets) from a lowercase host value.
fn host_without_port(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    match host.rsplit_once(':') {
        // More than one ':' without brackets is a bare IPv6 literal, not host:port.
        Some((name, _)) if !name.contains(':') => name,
        _ => host,
    }
}

/// `localhost`, `*.localhost`, `127.0.0.0/8` and `::1` (port and brackets ignored).
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    let name = host_without_port(&host);
    name == "localhost"
        || name.ends_with(".localhost")
        || name
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

impl ReqCtx {
    /// Resolve the allowed origin for a specific request by matching
    /// its `Origin` header against the configured origins list.
    ///
    /// Secure by default: with no origins configured NO CORS headers are sent,
    /// so browsers block cross-origin reads/writes (a hostile web page must not
    /// be able to talk to a local, possibly unauthenticated, server). Include
    /// `"*"` in `cors_origins` to explicitly opt in to wildcard CORS.
    /// Returns an empty string when CORS headers must be omitted.
    pub fn resolve_origin_for(&self, req: &Request) -> String {
        if self.cors_origins.iter().any(|o| o == "*") {
            return "*".to_string();
        }
        if let Some(origin_header) = req.headers().iter().find(|h| h.field.equiv("Origin")) {
            let origin = origin_header.value.as_str();
            if self.cors_origins.iter().any(|o| o == origin) {
                return origin.to_string();
            }
        }
        // No matching origin — omit CORS headers. Non-browser clients
        // (curl, agents) are unaffected by CORS.
        String::new()
    }

    /// True when wildcard CORS is combined with any configured token: the
    /// `Authorization` header must then not be allowed cross-origin.
    fn wildcard_cors_with_auth(&self) -> bool {
        (self.auth_token_hash.is_some() || self.read_only_token_hash.is_some())
            && self.cors_origins.iter().any(|o| o == "*")
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
        let allowed_headers = if self.wildcard_cors_with_auth() {
            "Content-Type"
        } else {
            "Content-Type, Authorization"
        };
        vec![
            Header::from_bytes("Access-Control-Allow-Origin", origin).unwrap(),
            Header::from_bytes(
                "Access-Control-Allow-Methods",
                "GET, POST, PUT, DELETE, OPTIONS",
            )
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
        let allowed_headers_set: &[&str] = if self.wildcard_cors_with_auth() {
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
            Header::from_bytes(
                "Access-Control-Allow-Methods",
                "GET, POST, PUT, DELETE, OPTIONS",
            )
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

#[cfg(test)]
mod cors_tests {
    use super::*;
    use tiny_http::{Method, TestRequest};

    fn ctx(origins: &[&str], auth: bool, read_only: bool) -> ReqCtx {
        ReqCtx {
            auth_token_hash: auth.then(|| Sha256::digest("t").into()),
            read_only_token_hash: read_only.then(|| Sha256::digest("r").into()),
            cors_origins: origins.iter().map(|s| s.to_string()).collect(),
            recall_config: None,
            extraction_config: None,
            host_guard: HostGuard::default(),
        }
    }

    fn request(origin: &str) -> Request {
        TestRequest::new()
            .with_method(Method::Get)
            .with_path("/health")
            .with_header(Header::from_bytes("Origin", origin).unwrap())
            .into()
    }

    #[test]
    fn no_origins_configured_sends_no_cors_headers() {
        let c = ctx(&[], false, false);
        assert!(
            c.cors_headers_for(&request("https://evil.example"))
                .is_empty()
        );
    }

    #[test]
    fn explicit_origin_is_allowed_others_are_not() {
        let c = ctx(&["https://app.example"], false, false);
        assert!(
            !c.cors_headers_for(&request("https://app.example"))
                .is_empty()
        );
        assert!(
            c.cors_headers_for(&request("https://evil.example"))
                .is_empty()
        );
    }

    #[test]
    fn wildcard_requires_explicit_opt_in_and_hides_authorization_with_any_token() {
        let c = ctx(&["*"], false, true); // only a read-only token configured
        let headers = c.cors_headers_for(&request("https://x.example"));
        let allow = headers
            .iter()
            .find(|h| h.field.equiv("Access-Control-Allow-Headers"))
            .unwrap();
        assert_eq!(allow.value.as_str(), "Content-Type");
    }
}

#[cfg(test)]
mod host_guard_tests {
    use super::*;

    fn loopback() -> HostGuard {
        HostGuard::new("127.0.0.1", &[])
    }

    #[test]
    fn loopback_bind_accepts_only_loopback_hosts() {
        let g = loopback();
        for ok in [
            "localhost",
            "localhost:8767",
            "127.0.0.1:8767",
            "127.0.0.1",
            "[::1]:8767",
            "::1",
            "app.localhost:3000",
            "LOCALHOST:8767",
        ] {
            assert!(g.allows(Some(ok)), "{ok} must be allowed");
        }
        for bad in [
            "evil.example",
            "evil.example:8767",
            "127.0.0.1.evil.example",
            "192.168.1.5:8767",
            "localhost.evil.example",
        ] {
            assert!(!g.allows(Some(bad)), "{bad} must be rejected");
        }
    }

    #[test]
    fn missing_host_header_is_allowed() {
        assert!(loopback().allows(None));
    }

    #[test]
    fn allowed_hosts_extend_a_loopback_bind() {
        let g = HostGuard::new("127.0.0.1", &["Uteke.Internal".to_string()]);
        assert!(g.allows(Some("uteke.internal:8767")));
        assert!(g.allows(Some("localhost")));
        assert!(!g.allows(Some("evil.example")));
    }

    #[test]
    fn docker_style_bind_without_allowlist_accepts_any_host() {
        let g = HostGuard::new("0.0.0.0", &[]);
        assert!(g.allows(Some("uteke:8767")));
        assert!(g.allows(Some("evil.example")));
    }

    #[test]
    fn docker_style_bind_with_allowlist_enforces_it() {
        let g = HostGuard::new("0.0.0.0", &["uteke".to_string()]);
        assert!(g.allows(Some("uteke:8767")));
        assert!(!g.allows(Some("evil.example")));
    }
}
