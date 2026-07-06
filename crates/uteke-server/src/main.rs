//! Uteke HTTP Server — persistent warm memory for AI agents.
//!
//! Keeps the embedding model loaded in RAM for <50ms recall.
//! Usage: `uteke-serve [--port 8767] [--host 127.0.0.1] [--auth-token <TOKEN>]`

mod context;
mod handlers;
mod types;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use std::path::PathBuf;

use sha2::{Digest, Sha256};
use tiny_http::Server;
use tracing::{error, info, warn};
use uteke_core::Uteke;

use types::RecallFileSection;

// ── Main ────────────────────────────────────────────────────────────────────

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

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
                println!("  (except GET /health) require Authorization: Bearer ***");
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
                println!("  GET  /room/memories       → ?room_id=<id>[&author=&limit=] → chronological memories");
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
    let ctx = context::ReqCtx {
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
    // Runs aging cleanup periodically to remove cold, low-importance memories.
    let aging_enabled = config
        .maintenance
        .as_ref()
        .and_then(|m| m.auto_aging_enabled)
        .unwrap_or(true);
    let aging_hours = config
        .maintenance
        .as_ref()
        .and_then(|m| m.auto_aging_interval_hours)
        .unwrap_or(6)
        .max(1); // Minimum 1 hour to prevent busy loop
    let aging_uteke = Arc::clone(&uteke);
    let aging_config = config.aging.clone();
    if aging_enabled {
        info!("Auto-aging: enabled (every {aging_hours}h)");
        std::thread::spawn(move || {
            let interval = std::time::Duration::from_secs(aging_hours * 60 * 60);
            loop {
                std::thread::sleep(interval);
                if SHUTDOWN.load(Ordering::SeqCst) {
                    break;
                }
                match aging_uteke.lock() {
                    Ok(u) => {
                        let age_days = aging_config
                            .as_ref()
                            .and_then(|a| a.max_age_days)
                            .unwrap_or(365);
                        let max_access = aging_config
                            .as_ref()
                            .and_then(|a| a.max_access_count)
                            .unwrap_or(10);
                        match u.aging_cleanup(age_days, max_access, None) {
                            Ok(result) => {
                                if result.deleted > 0 {
                                    info!("Auto-aging: cleaned up {} stale memories (age>{age_days}d, access<{max_access})", result.deleted);
                                }
                            }
                            Err(e) => {
                                warn!("Auto-aging failed: {e}");
                            }
                        }
                    }
                    Err(_) => {
                        tracing::debug!("Auto-aging: lock busy, skipping cycle");
                    }
                }
            }
        });
    } else {
        info!("Auto-aging: disabled");
    }

    // Auto-dream background thread (#442 enhancement).
    // Runs dream cycle periodically to maintain graph health.
    let dream_enabled = config
        .maintenance
        .as_ref()
        .and_then(|m| m.auto_dream_enabled)
        .unwrap_or(true);
    let dream_days = config
        .maintenance
        .as_ref()
        .and_then(|m| m.auto_dream_interval_days)
        .unwrap_or(3)
        .max(1); // Minimum 1 day to prevent busy loop
    let dream_uteke = Arc::clone(&uteke);
    if dream_enabled {
        info!("Auto-dream: enabled (every {dream_days}d)");
        std::thread::spawn(move || {
            let interval = std::time::Duration::from_secs(dream_days * 24 * 60 * 60);
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
    } else {
        info!("Auto-dream: disabled");
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
            let response = handlers::route(&uteke, &ctx, &mut req);
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
    maintenance: Option<MaintenanceFileSection>,
    aging: Option<AgingFileSection>,
}

#[derive(serde::Deserialize, Default, Clone)]
struct AgingFileSection {
    max_age_days: Option<u32>,
    max_access_count: Option<u32>,
}

#[derive(serde::Deserialize, Default, Clone)]
struct MaintenanceFileSection {
    auto_aging_enabled: Option<bool>,
    auto_aging_interval_hours: Option<u64>,
    auto_dream_enabled: Option<bool>,
    auto_dream_interval_days: Option<u64>,
}

#[derive(serde::Deserialize, Default)]
struct ServerFileSection {
    host: Option<String>,
    port: Option<u16>,
    /// Bearer token for API authentication.
    /// If set, all endpoints except GET /health require Authorization: Bearer ***
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
