//! Uteke HTTP Server — persistent warm memory for AI agents.
//!
//! Keeps the embedding model loaded in RAM for <50ms recall.
//! Usage: `uteke-serve [--port 8767] [--host 127.0.0.1] [--auth-token <TOKEN>]`

use std::io::{Cursor, Read as IoRead};
use std::sync::atomic::{AtomicBool, Ordering};

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use tracing::{error, info, warn};
use uteke_core::Uteke;

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
fn default_limit_search() -> usize {
    10
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn json_header() -> Header {
    Header::from_bytes("Content-Type", "application/json").unwrap()
}

/// Check bearer token auth on a request.
/// Returns Ok(()) if auth passes or is disabled.
/// Returns Err(response) with 401 if auth fails.
fn check_auth(req: &Request, ctx: &ReqCtx) -> Result<(), Response<Cursor<Vec<u8>>>> {
    let token = match &ctx.auth_token {
        // No token configured — auth disabled
        None => return Ok(()),
        Some(t) => t,
    };

    // Look for Authorization: Bearer <token>
    let auth_header = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Authorization"));

    match auth_header {
        Some(h) => {
            let val = h.value.as_str();
            if let Some(provided) = val.strip_prefix("Bearer ") {
                // Constant-time comparison to prevent timing attacks
                if constant_time_eq(provided.as_bytes(), token.as_bytes()) {
                    Ok(())
                } else {
                    let mut hdrs = ctx.cors_headers_for(req);
                    hdrs.push(
                        Header::from_bytes("WWW-Authenticate", "Bearer realm=\"uteke\"").unwrap(),
                    );
                    hdrs.push(json_header());
                    let body = ErrorResponse {
                        error: "Invalid or expired token".into(),
                    };
                    let data = serde_json::to_string(&body).unwrap();
                    Err(Response::new(
                        StatusCode::from(401),
                        hdrs,
                        Cursor::new(data.into_bytes()),
                        None,
                        None,
                    ))
                }
            } else {
                let mut hdrs = ctx.cors_headers_for(req);
                hdrs.push(
                    Header::from_bytes("WWW-Authenticate", "Bearer realm=\"uteke\"").unwrap(),
                );
                hdrs.push(json_header());
                let body = ErrorResponse {
                    error: "Invalid auth scheme. Use: Authorization: Bearer <token>".into(),
                };
                let data = serde_json::to_string(&body).unwrap();
                Err(Response::new(
                    StatusCode::from(401),
                    hdrs,
                    Cursor::new(data.into_bytes()),
                    None,
                    None,
                ))
            }
        }
        None => {
            let mut hdrs = ctx.cors_headers_for(req);
            hdrs.push(Header::from_bytes("WWW-Authenticate", "Bearer realm=\"uteke\"").unwrap());
            hdrs.push(json_header());
            let body = ErrorResponse {
                error: "Authentication required. Provide Authorization: Bearer <token>".into(),
            };
            let data = serde_json::to_string(&body).unwrap();
            Err(Response::new(
                StatusCode::from(401),
                hdrs,
                Cursor::new(data.into_bytes()),
                None,
                None,
            ))
        }
    }
}

/// Constant-time byte comparison to resist timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

struct ReqCtx {
    auth_token: Option<String>,
    /// Allowed CORS origins from config. Empty = wildcard.
    cors_origins: Vec<String>,
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
        let allowed_headers = if self.auth_token.is_some() && self.cors_origins.is_empty() {
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
            if self.auth_token.is_some() && self.cors_origins.is_empty() {
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
            .unwrap_or_else(|| "Content-Type".to_string());
        // Fallback if no requested headers matched
        let allow_headers = if allow_headers.is_empty() {
            allowed_headers_set.join(", ")
        } else {
            allow_headers
        };
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

fn ns(ns: &Option<String>) -> Option<&str> {
    ns.as_deref()
}

// ── Router ──────────────────────────────────────────────────────────────────

fn route(uteke: &Uteke, ctx: &ReqCtx, req: &mut Request) -> Response<Cursor<Vec<u8>>> {
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
    if !is_health {
        if let Err(resp) = check_auth(req, ctx) {
            return resp;
        }
    }

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
                match uteke.recall(
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
                match uteke.list(
                    req_data.tag.as_deref(),
                    req_data.limit,
                    req_data.offset,
                    ns(&req_data.namespace),
                ) {
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

        // ── Stats (GET = all, POST = by namespace) ─────────────────────
        (Method::Get, "/stats") => match uteke.stats(None) {
            Ok(stats) => ctx.ok_response_for(req, &stats),
            Err(e) => {
                error!("Internal error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
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

        // ── 404 ─────────────────────────────────────────────────────────
        _ => ctx.error_response_for(req, 404, "Not found"),
    }
}

// ── Main ────────────────────────────────────────────────────────────────────

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

fn main() {
    // Parse CLI args — these override config
    let args: Vec<String> = std::env::args().collect();
    let mut cli_host: Option<String> = None;
    let mut cli_port: Option<u16> = None;
    let mut cli_auth_token: Option<String> = None;
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
                println!("  -h, --help           Show this help");
                println!();
                println!("Config: reads [server] section from uteke.toml");
                println!("  CLI args override config values.");
                println!();
                println!("Environment:");
                println!("  UTEKE_HOME          Data directory (default: ~/.uteke)");
                println!("  UTEKE_AUTH_TOKEN     Bearer token (alternative to --auth-token)");
                println!();
                println!("Security:");
                println!("  If --auth-token or UTEKE_AUTH_TOKEN is set, all endpoints");
                println!("  (except GET /health) require Authorization: Bearer <TOKEN>.");
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
        Ok(u) => u,
        Err(e) => {
            error!("Failed to open store: {e}");
            std::process::exit(1);
        }
    };

    // Build request context
    // Warn if auth is configured but CORS origins are not — this is safe for
    // non-browser clients (curl, SDKs, agents) but risky if browser access is needed.
    if auth_token.is_some() && cors_origins.is_empty() {
        warn!("Security: auth token is set but cors_origins is not configured.");
        warn!("  For browser access, set cors_origins in uteke.toml or --cors-origin.");
        warn!("  Non-browser clients (curl, agents) are unaffected by CORS.");
    }
    let ctx = ReqCtx {
        auth_token: auth_token.clone(),
        cors_origins: cors_origins.clone(),
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
    if cors_origins.is_empty() {
        warn!("CORS: wildcard (*) — restrict cors_origins in uteke.toml for production");
    } else {
        info!("CORS: allowing origins: {:?}", cors_origins);
    }

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

    // Request loop
    for mut req in server.incoming_requests() {
        if SHUTDOWN.load(Ordering::SeqCst) {
            info!("Shutdown requested, stopping.");
            break;
        }

        let method = req.method().clone();
        let url = req.url().to_string();
        info!("{method} {url}");

        let response = route(&uteke, &ctx, &mut req);
        if let Err(e) = req.respond(response) {
            warn!("Response error: {e}");
        }
    }

    // Graceful shutdown
    info!("Saving index and closing DB...");
    if let Err(e) = uteke.shutdown() {
        error!("Shutdown error: {e}");
    }

    info!("Goodbye.");
}

// ── Config Loading ────────────────────────────────────────────────────────

/// Minimal [server] config section for parsing uteke.toml.
#[derive(serde::Deserialize, Default)]
struct ServerFileConfig {
    server: Option<ServerFileSection>,
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
