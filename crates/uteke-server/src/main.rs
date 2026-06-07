//! Uteke HTTP Server — persistent warm memory for AI agents.
//!
//! Keeps the embedding model loaded in RAM for <50ms recall.
//! Usage: `uteke-serve [--port 8767] [--host 127.0.0.1]`

use std::io::{Cursor, Read as IoRead};
use std::sync::atomic::{AtomicBool, Ordering};

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Response, Server, StatusCode};
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

fn cors_headers() -> Vec<Header> {
    vec![
        Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap(),
        Header::from_bytes("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS").unwrap(),
        Header::from_bytes("Access-Control-Allow-Headers", "Content-Type").unwrap(),
    ]
}

fn json_response<T: Serialize>(status: u16, body: &T) -> Response<Cursor<Vec<u8>>> {
    let data = serde_json::to_string(body).unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#));
    let mut headers = cors_headers();
    headers.push(json_header());
    Response::new(
        StatusCode::from(status),
        headers,
        Cursor::new(data.into_bytes()),
        None,
        None,
    )
}

fn error_response(status: u16, msg: impl Into<String>) -> Response<Cursor<Vec<u8>>> {
    let body = ErrorResponse { error: msg.into() };
    json_response(status, &body)
}

fn ok_response<T: Serialize>(body: &T) -> Response<Cursor<Vec<u8>>> {
    json_response(200, body)
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

fn route(
    uteke: &Uteke,
    method: &Method,
    path: &str,
    body: &mut dyn IoRead,
) -> Response<Cursor<Vec<u8>>> {
    // CORS preflight
    if method == &Method::Options {
        return Response::new(
            StatusCode::from(204),
            cors_headers(),
            Cursor::new(Vec::new()),
            None,
            None,
        );
    }

    match (method, path) {
        // ── Health ──────────────────────────────────────────────────────
        (&Method::Get, "/health") => {
            let total = uteke.count(None).unwrap_or(0);
            let namespaces = uteke.list_namespaces().unwrap_or_default().len();
            ok_response(&HealthResponse {
                status: "ok",
                memories: total,
                namespaces,
            })
        }

        // ── Remember ───────────────────────────────────────────────────
        (&Method::Post, "/remember") => match read_body::<RememberRequest>(body) {
            Ok(req) => {
                let tag_refs: Vec<&str> = req.tags.iter().map(|s| s.as_str()).collect();

                // Build metadata from optional fields
                let mut meta = serde_json::Map::new();
                if let Some(t) = &req.r#type {
                    meta.insert("type".into(), serde_json::Value::String(t.clone()));
                }
                if let Some(vf) = &req.valid_from {
                    meta.insert("valid_from".into(), serde_json::Value::String(vf.clone()));
                }
                if let Some(vu) = &req.valid_until {
                    meta.insert("valid_until".into(), serde_json::Value::String(vu.clone()));
                }
                let metadata = if meta.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::Object(meta))
                };

                let result = if req.detect_contradiction {
                    uteke
                        .remember_with_contradiction(
                            &req.content,
                            &tag_refs,
                            ns(&req.namespace),
                            req.r#type.as_deref(),
                            true,
                        )
                        .map(|(id, _)| id)
                } else {
                    uteke.remember(&req.content, &tag_refs, metadata, ns(&req.namespace))
                };

                match result {
                    Ok(id) => ok_response(&serde_json::json!({"id": id})),
                    Err(e) => error_response(500, format!("Failed: {e}")),
                }
            }
            Err(e) => error_response(400, e),
        },

        // ── Recall (semantic search) ────────────────────────────────────
        (&Method::Post, "/recall") => match read_body::<RecallRequest>(body) {
            Ok(req) => {
                let tag_refs: Vec<&str> = req.tags.iter().map(|s| s.as_str()).collect();
                let tags_filter = if tag_refs.is_empty() {
                    None
                } else {
                    Some(tag_refs.as_slice())
                };
                match uteke.recall(&req.query, req.limit, tags_filter, ns(&req.namespace)) {
                    Ok(results) => ok_response(&results),
                    Err(e) => error_response(500, format!("Failed: {e}")),
                }
            }
            Err(e) => error_response(400, e),
        },

        // ── Search (keyword) ────────────────────────────────────────────
        (&Method::Post, "/search") => match read_body::<SearchRequest>(body) {
            Ok(req) => {
                let tag_refs: Vec<&str> = req.tags.iter().map(|s| s.as_str()).collect();
                let tags_filter = if tag_refs.is_empty() {
                    None
                } else {
                    Some(tag_refs.as_slice())
                };
                match uteke.search(&req.query, req.limit, tags_filter, ns(&req.namespace)) {
                    Ok(results) => ok_response(&results),
                    Err(e) => error_response(500, format!("Failed: {e}")),
                }
            }
            Err(e) => error_response(400, e),
        },

        // ── List ────────────────────────────────────────────────────────
        (&Method::Post, "/list") => match read_body::<ListParams>(body) {
            Ok(req) => {
                match uteke.list(
                    req.tag.as_deref(),
                    req.limit,
                    req.offset,
                    ns(&req.namespace),
                ) {
                    Ok(memories) => ok_response(&memories),
                    Err(e) => error_response(500, format!("Failed: {e}")),
                }
            }
            Err(e) => error_response(400, e),
        },

        // ── Forget by ID or tag (DELETE /forget?id=xxx or ?tag=xxx) ────
        (&Method::Delete, path) if path == "/forget" || path.starts_with("/forget?") => {
            let query = path.split('?').nth(1).unwrap_or("");
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
                    return error_response(400, format!("Invalid UUID format: {id}"));
                }
                match uteke.forget(id) {
                    Ok(()) => ok_response(&serde_json::json!({"forgotten": id})),
                    Err(e) => error_response(500, format!("Failed: {e}")),
                }
            } else if let Some(tag) = params.get("tag") {
                let namespace = params.get("namespace").map(|s| s.as_str());
                match uteke.bulk_forget_by_tag(tag, namespace) {
                    Ok(result) => ok_response(&result),
                    Err(e) => error_response(500, format!("Failed: {e}")),
                }
            } else {
                error_response(400, "Provide ?id= or ?tag= parameter")
            }
        }

        // ── Stats (GET = all, POST = by namespace) ─────────────────────
        (&Method::Get, "/stats") => match uteke.stats(None) {
            Ok(stats) => ok_response(&stats),
            Err(e) => error_response(500, format!("Failed: {e}")),
        },
        (&Method::Post, "/stats") => {
            #[derive(Deserialize)]
            struct StatsReq {
                namespace: Option<String>,
            }
            match read_body::<StatsReq>(body) {
                Ok(req) => match uteke.stats(ns(&req.namespace)) {
                    Ok(stats) => ok_response(&stats),
                    Err(e) => error_response(500, format!("Failed: {e}")),
                },
                Err(e) => error_response(400, e),
            }
        }

        // ── Namespaces ──────────────────────────────────────────────────
        (&Method::Get, "/namespaces") => match uteke.list_namespaces() {
            Ok(namespaces) => ok_response(&namespaces),
            Err(e) => error_response(500, format!("Failed: {e}")),
        },

        // ── Get memory by ID ──────────────────────────────────────────
        (&Method::Get, path) if path.starts_with("/memory?id=") => {
            let id = path.trim_start_matches("/memory?id=");
            // Validate UUID format
            if uuid::Uuid::parse_str(id).is_err() {
                return error_response(400, format!("Invalid UUID format: {id}"));
            }
            match uteke.get_by_id(id) {
                Ok(Some(memory)) => ok_response(&memory),
                Ok(None) => error_response(404, format!("Memory not found: {id}")),
                Err(e) => error_response(500, format!("Failed: {e}")),
            }
        }

        // ── 404 ─────────────────────────────────────────────────────────
        _ => error_response(404, "Not found"),
    }
}

// ── Main ────────────────────────────────────────────────────────────────────

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

fn main() {
    // Parse CLI args — these override config
    let args: Vec<String> = std::env::args().collect();
    let mut cli_host: Option<String> = None;
    let mut cli_port: Option<u16> = None;

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
            "--help" | "-h" => {
                println!("uteke-serve — persistent warm memory server");
                println!();
                println!("Usage: uteke-serve [OPTIONS]");
                println!();
                println!("Options:");
                println!("  --host <HOST>  Bind address (default: 0.0.0.0)");
                println!("  --port <PORT>  Port number (default: 8767)");
                println!("  -h, --help     Show this help");
                println!();
                println!("Config: reads [server] section from uteke.toml");
                println!("  CLI args override config values.");
                println!();
                println!("Environment:");
                println!("  UTEKE_HOME    Data directory (default: ~/.uteke)");
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

    // Load config: defaults → uteke.toml → CLI args
    let config = load_uteke_toml();
    let config_host = config
        .server
        .as_ref()
        .and_then(|s| s.host.clone())
        .unwrap_or_else(|| "0.0.0.0".to_string());
    let config_port = config.server.as_ref().and_then(|s| s.port).unwrap_or(8767);

    let host = cli_host.unwrap_or(config_host);
    let port = cli_port.unwrap_or(config_port);

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

    // Start server
    let addr = format!("{host}:{port}");
    let server = Server::http(&addr).unwrap_or_else(|e| {
        error!("Failed to bind {addr}: {e}");
        std::process::exit(1);
    });
    info!("Uteke server listening on http://{addr}");
    info!("Embedding model warm. Ready for <50ms recall.");

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

        let response = route(&uteke, &method, &url, &mut req.as_reader());
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
