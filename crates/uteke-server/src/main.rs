//! Uteke HTTP Server — persistent warm memory for AI agents.
//!
//! Keeps the embedding model loaded in RAM for <50ms recall.
//! Usage: `uteke-serve [--port 8767] [--host 127.0.0.1] [--auth-token <TOKEN>]`

// api_registry is runtime data now: GET /routes serves it (#1289),
// in addition to its original docgen role.
mod api_registry;
mod capture_worker;
mod context;
mod handlers;
mod types;
mod viewer;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use tiny_http::Server;
use tracing::{error, info, warn};

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
    let mut cli_allowed_hosts: Vec<String> = Vec::new();
    let mut viewer_mode = false;

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
            "--allowed-host" => {
                i += 1;
                if i < args.len() {
                    cli_allowed_hosts.push(args[i].clone());
                } else {
                    eprintln!("Error: --allowed-host requires a value");
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
            "--viewer" => viewer_mode = true,
            "--version" | "-V" => {
                // Match the main CLI's format: `uteke v0.15.0` (see #1044)
                println!("uteke-serve {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
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
                println!(
                    "  --allowed-host <H>   Extra accepted Host header value (repeatable, #1326)"
                );
                println!("  --read-only-token <T> Read-only API token (GET endpoints only) (#409)");
                println!(
                    "  --viewer             Run the read-only browser viewer instead (`uteke view`, #1470)"
                );
                println!("  -V, --version        Show version");
                println!("  -h, --help           Show this help");
                println!();
                println!("Config: reads [server] section from uteke.toml");
                println!("  CLI args override config values.");
                println!();
                println!("Environment:");
                println!("  UTEKE_HOME          Data directory (default: ~/.codecora/uteke)");
                println!("  UTEKE_AUTH_TOKEN     Bearer token (alternative to --auth-token)");
                println!(
                    "  UTEKE_READ_ONLY_TOKEN  Read-only token (alternative to --read-only-token)"
                );
                println!();
                println!("Security:");
                println!("  If --auth-token or UTEKE_AUTH_TOKEN is set, all endpoints");
                println!("  (except GET /health) require Authorization: Bearer ***");
                println!(
                    "  --read-only-token grants GET-only access (recall, search, list, stats, graph)."
                );
                println!("  Configure CORS origins in uteke.toml [server].cors_origins.");
                println!();
                println!("API:");
                println!(
                    "  GET  /health              → {{ status, version, memories, namespaces }}"
                );
                println!(
                    "  GET  /routes              → machine-readable endpoint registry (method, path, tier, description) (#1289)"
                );
                println!("  POST /remember            → {{ content, tags? }} → {{ id }}");
                println!("  POST /recall              → {{ query, limit? }} → {{ results }}");
                println!("  POST /search              → {{ query, limit? }} → {{ results }}");
                println!(
                    "  POST /list                → {{ tag?, limit?, offset? }} → {{ memories }}"
                );
                println!("  DELETE /forget?id=UUID     → {{ forgotten }}");
                println!("  DELETE /forget?tag=TAG     → {{ deleted }}");
                println!("  GET  /memory?id=UUID       -> {{ memory }}");
                println!("  POST /memory/pin           -> {{ id, pinned }} -> {{ memory }}");
                println!("  POST /memory/importance    -> {{ id, importance }} -> {{ memory }}");
                println!(
                    "  POST /memory/feedback      -> {{ id, feedback }} -> {{ id, delta, importance }} (#718)"
                );
                println!("  GET  /stats               → {{ stats }}");
                println!("  GET  /namespaces           → {{ namespaces }}");
                println!(
                    "  POST /room/create          → {{ room_id, title, namespace }} → {{ created }}"
                );
                println!("  GET  /room/list            → [?namespace=] → [rooms]");
                println!(
                    "  GET  /room/memories       → ?room_id=<id>[&author=&namespace=&limit=] → chronological memories"
                );
                println!(
                    "  POST /room/recall          → {{ room_id, query? }} → ranked memories (query optional, falls back to chronological)"
                );
                println!("  POST /room/summary         → {{ room_id }} → {{ summary }}");
                println!("  POST /room/document        → {{ room_id }} → {{ document }}");
                println!("  POST /room/stats           → {{ room_id }} → room stats");
                println!(
                    "  POST /room/search          → {{ query, limit?, namespace? }} → rooms for a query"
                );
                println!("  DEL  /room/delete          → {{ room_id }} → {{ deleted }}");
                println!();
                println!("  Document endpoints:");
                println!(
                    "  POST /doc/create          → {{ slug, content, title?, tags?, parent? }} → {{ id, slug }}"
                );
                println!("  POST /doc/get              → {{ id | slug }} → {{ document }}");
                println!(
                    "  POST /doc/list             → {{ namespace?, limit?, roots_only?, parent? }} → [documents]"
                );
                println!(
                    "  POST /doc/search            → {{ query, mode?, namespace?, limit? }} → [results]"
                );
                println!(
                    "  POST /doc/similar           → {{ slug | text | report, threshold?, limit?, namespace?, subtree? }} → {{ hits | pairs }}"
                );
                println!(
                    "  POST /skill/search          → {{ query, mode?, limit?, approved_only? }} → {{ hits }}"
                );
                println!("  POST /skill/list            → {{ status?, limit? }} → {{ skills }}");
                println!("  POST /skill/show            → {{ slug }} → skill detail");
                println!(
                    "  POST /skill/similar         → {{ slug | text, threshold?, limit? }} → {{ hits }}"
                );
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

    // Merge allowed Host values: CLI flags override config (#1326)
    let allowed_hosts = if !cli_allowed_hosts.is_empty() {
        cli_allowed_hosts
    } else {
        config
            .server
            .as_ref()
            .and_then(|s| s.allowed_hosts.clone())
            .unwrap_or_default()
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

    // `uteke view` (#1470): the read-only viewer is its own small listener.
    if viewer_mode {
        std::process::exit(viewer::run(viewer::Options {
            host,
            port: cli_port.unwrap_or(8768),
            auth_token,
            allowed_hosts,
        }));
    }

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
    // The one open-and-configure sequence (#1754), shared with the CLI and MCP:
    // fail fast on an unsupported embedding backend instead of opening the store
    // with a different embedder than the operator asked for (#1399), then open
    // the store and apply the engine settings and the privacy filter.
    let open_config = match uteke_core::OpenConfig::load() {
        Ok(c) => c,
        Err(msg) => {
            error!("{msg}");
            std::process::exit(1);
        }
    };
    info!("Embedding backend: {}", open_config.embedding.backend);
    if open_config.engine.embed_fallback.is_configured() {
        info!(
            "Embedding fallback: enabled ({})",
            open_config.engine.embed_fallback.base_url
        );
    }
    let engine = &open_config.engine;
    let uteke = match open_config.open(&db_path) {
        Ok(u) => {
            info!("Privacy filter: {:?}", u.privacy_config().mode);
            info!(
                "Dream config loaded: dedup={:.2}, orphan_thresh={:.2}",
                engine.dream.dedup_threshold, engine.dream.orphan_importance_threshold,
            );
            info!(
                "Lifecycle config loaded: soft_delete_only={}, ttl={}d, max_deprecate={:.1}%",
                engine.lifecycle.soft_delete_only,
                engine.lifecycle.deprecated_ttl_days,
                engine.lifecycle.max_deprecate_percent,
            );
            Arc::new(Mutex::new(u))
        }
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
    // CORS is off unless origins are configured. Wildcard is an explicit opt-in
    // and is dangerous without auth: any web page could read/write memories.
    if cors_origins.iter().any(|o| o == "*")
        && auth_token_hash.is_none()
        && read_only_token_hash.is_none()
    {
        warn!("Security: cors_origins contains \"*\" and authentication is disabled —");
        warn!("  any website opened in a local browser can read and modify memories.");
        warn!("  Set an auth token or list explicit origins.");
    }
    // `[extraction]` with the `UTEKE_EXTRACTION_*` variables on top, the same
    // resolution as the CLI: `POST /extract` and the capture schedule use it.
    let extraction = config
        .extraction
        .clone()
        .unwrap_or_default()
        .with_env_overrides();
    let ctx = context::ReqCtx {
        auth_token_hash,
        read_only_token_hash,
        cors_origins: cors_origins.clone(),
        // The store's own `[recall]` defaults, the source MCP reads too (#1754).
        recall_config: Some(
            uteke
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .recall_settings()
                .clone(),
        ),
        extraction_config: Some(extraction.clone()),
        host_guard: context::HostGuard::new(&host, &allowed_hosts),
        store_id: uteke_core::store_identity(&home),
        embed_warmer: Default::default(),
        remote_embeds: Default::default(),
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

    // Auto-lifecycle background thread (#934 — replaces auto-aging #442).
    // Runs lifecycle_cycle periodically: soft-deprecate aged memories (cap-limited)
    // + auto-prune expired deprecated memories.
    let lifecycle_config = engine.lifecycle;
    let lifecycle_enabled = lifecycle_config.auto_aging_enabled;
    // Minimum 1 hour to prevent a busy loop.
    let lifecycle_hours = lifecycle_config.auto_aging_interval_hours.max(1);
    let lifecycle_uteke = Arc::clone(&uteke);
    if lifecycle_enabled {
        info!("Auto-lifecycle: enabled (every {lifecycle_hours}h)");
        std::thread::spawn(move || {
            let interval = std::time::Duration::from_secs(lifecycle_hours * 60 * 60);
            loop {
                std::thread::sleep(interval);
                if SHUTDOWN.load(Ordering::SeqCst) {
                    break;
                }
                match lifecycle_uteke.lock() {
                    Ok(u) => match u.lifecycle_cycle(None) {
                        Ok(result) => {
                            if result.deprecated > 0 || result.pruned > 0 {
                                info!(
                                    "Auto-lifecycle: deprecated {}/{} (cap={}, total_active={}), pruned {} expired",
                                    result.deprecated,
                                    result.candidates,
                                    result.cap,
                                    result.total_active,
                                    result.pruned
                                );
                            }
                        }
                        Err(e) => {
                            warn!("Auto-lifecycle failed: {e}");
                        }
                    },
                    Err(_) => {
                        tracing::debug!("Auto-lifecycle: lock busy, skipping cycle");
                    }
                }
            }
        });
    } else {
        info!("Auto-lifecycle: disabled");
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

    // Scheduled extraction of the capture queue (#1466). Opt-in: runs only
    // when `[capture] enabled` is on in the global uteke.toml.
    let capture = engine.capture;
    if capture.enabled {
        info!(
            "Capture extraction: enabled (every {}min, {} session(s) per run)",
            capture.interval_minutes.max(1),
            capture.budget_per_run
        );
        let capture_uteke = Arc::clone(&uteke);
        let worker_extraction = extraction.clone();
        std::thread::spawn(move || {
            capture_worker::run_forever(&capture_uteke, &worker_extraction, capture, &SHUTDOWN);
        });
    } else {
        info!("Capture extraction: disabled");
    }

    // Update check — startup notification + periodic 24h re-check.
    // Long-running server process won't see CLI's startup check, so we
    // run our own. Logs via `warn!` so it's visible in structured logs.
    std::thread::spawn(move || {
        loop {
            // Cache first — avoids hitting GitHub on every server restart
            // within the 24h cache window.
            let info = uteke_core::update_check::check_cached()
                .or_else(|| uteke_core::update_check::check_network().ok());
            if let Some(info) = info.filter(|i| i.is_update_available()) {
                warn!(
                    "Uteke {} is available (currently v{}). Run `uteke upgrade` to update.",
                    info.latest, info.current
                );
            }
            // Sleep 24h, checking shutdown flag hourly.
            for _ in 0..24 {
                std::thread::sleep(std::time::Duration::from_secs(3600));
                if SHUTDOWN.load(Ordering::SeqCst) {
                    return;
                }
            }
        }
    });

    // Vector-index flusher (#1322): per-operation index saves are batched, so
    // a single write followed by idle time would otherwise sit in memory until
    // the next write or a clean shutdown (SIGTERM does not run the handler).
    let flush_uteke = Arc::clone(&uteke);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if SHUTDOWN.load(Ordering::SeqCst) {
                return;
            }
            if let Ok(u) = flush_uteke.lock() {
                if let Err(e) = u.flush_index() {
                    warn!("Periodic index flush failed: {e}");
                }
                // vecq only: rebuild once dead rows pass 25% of the index (#1324).
                if let Err(e) = u.compact_index_if_needed(false) {
                    warn!("Index compaction failed: {e}");
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
    // Cap concurrent threads via Condvar-based semaphore: park instead of spin.
    let max_threads = std::thread::available_parallelism()
        .map(|n| n.get() * 2)
        .unwrap_or(8);
    let pair = Arc::new((
        std::sync::Mutex::new(0usize), // active count
        std::sync::Condvar::new(),
    ));

    for mut req in server.incoming_requests() {
        if SHUTDOWN.load(Ordering::SeqCst) {
            info!("Shutdown requested, stopping.");
            break;
        }

        // Backpressure: wait until a thread slot is available (parked, not spinning).
        {
            let (lock, cvar) = &*pair;
            let mut active = lock.lock().unwrap();
            while *active >= max_threads && !SHUTDOWN.load(Ordering::SeqCst) {
                active = cvar.wait(active).unwrap();
            }
        }
        if SHUTDOWN.load(Ordering::SeqCst) {
            break;
        }

        let method = req.method().clone();
        let url = req.url().to_string();
        // Log the path only: query strings can carry memory text / search
        // phrases (PII, secrets) that must not land in server logs.
        info!("{method} {}", url.split('?').next().unwrap_or(&url));

        let uteke = Arc::clone(&uteke);
        let ctx = ctx.clone();
        let pair = Arc::clone(&pair);
        let pair_err = Arc::clone(&pair);

        {
            let (lock, _) = &*pair;
            *lock.lock().unwrap() += 1;
        }

        let result = std::thread::Builder::new().spawn(move || {
            // RAII: the slot is released even if the handler panics, so a
            // single bad request can never leak capacity.
            let _slot = SlotGuard(pair);
            let routed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                handlers::route(&uteke, &ctx, &mut req)
            }));
            let response = match routed {
                Ok(r) => r,
                Err(_) => {
                    error!("Request handler panicked: {method} {url}");
                    tiny_http::Response::from_data(b"Internal server error".to_vec())
                        .with_status_code(500)
                }
            };
            if let Err(e) = req.respond(response) {
                warn!("Response error: {e}");
            }
        });

        if let Err(e) = result {
            // Spawn failed — release the slot we reserved.
            let (lock, cvar) = &*pair_err;
            let mut active = lock.lock().unwrap();
            *active -= 1;
            cvar.notify_one();
            warn!("Failed to spawn request thread: {e}");
        }
    }

    // Graceful shutdown — save dirty index to disk.
    // Handle poisoned mutex gracefully instead of panicking (#845).
    match uteke.lock() {
        Ok(u) => {
            if let Err(e) = u.shutdown() {
                error!("Shutdown error: {e}");
            }
        }
        Err(_) => {
            error!("Shutdown: mutex poisoned, forcing exit without index save");
        }
    }

    info!("Goodbye.");
}

// ── Config Loading ────────────────────────────────────────────────────────

/// Minimal [server] config section for parsing uteke.toml.
#[derive(serde::Deserialize, Default)]
struct ServerFileConfig {
    server: Option<ServerFileSection>,
    extraction: Option<uteke_core::extraction::ExtractionConfig>,
    maintenance: Option<MaintenanceFileSection>,
    /// Deprecated: superseded by [lifecycle] section (#934). Kept for backward-compat deserialization.
    #[allow(dead_code)]
    aging: Option<AgingFileSection>,
}

/// Deprecated: superseded by [lifecycle] section (#934). Kept for backward-compat deserialization.
#[derive(serde::Deserialize, Default, Clone)]
#[allow(dead_code)]
struct AgingFileSection {
    max_age_days: Option<u32>,
    max_access_count: Option<u32>,
}

#[derive(serde::Deserialize, Default, Clone)]
struct MaintenanceFileSection {
    /// Deprecated: moved to [lifecycle] section. Kept for backward-compat.
    #[allow(dead_code)]
    auto_aging_enabled: Option<bool>,
    /// Deprecated: moved to [lifecycle] section. Kept for backward-compat.
    #[allow(dead_code)]
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
    /// Extra accepted `Host` header values (#1326). On a loopback bind only
    /// loopback hosts are accepted by default (DNS-rebinding guard); list
    /// additional names here. On a non-loopback bind (Docker) every `Host`
    /// is accepted unless this list is set.
    allowed_hosts: Option<Vec<String>>,
}

/// Releases one concurrency slot (and wakes the accept loop) on drop.
struct SlotGuard(Arc<(std::sync::Mutex<usize>, std::sync::Condvar)>);

impl Drop for SlotGuard {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.0;
        let mut active = lock.lock().unwrap_or_else(|e| e.into_inner());
        *active = active.saturating_sub(1);
        cvar.notify_one();
    }
}

/// Resolve `uteke.toml` from:
/// 1. `$UTEKE_HOME/uteke.toml` (or `~/.codecora/uteke/uteke.toml`)
/// 2. `$CWD/.uteke/uteke.toml` (project-local, untrusted for `[server]`,
///    endpoints and credentials unless `UTEKE_TRUST_PROJECT_CONFIG=1`)
///
/// Merge precedence, the untrusted-key policy and per-layer validation are
/// shared with the CLI in `uteke_core::config_layers`.
fn load_uteke_toml() -> ServerFileConfig {
    uteke_core::config_layers::with_standard_layers(resolve_server_config)
}

fn resolve_server_config(layers: &uteke_core::config_layers::Layers<'_>) -> ServerFileConfig {
    uteke_core::config_layers::resolve::<ServerFileConfig>(layers).value
}

#[cfg(test)]
mod config_overlay_tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_file(tag: &str, body: &str) -> PathBuf {
        // Unique per call: tests run in parallel inside one process.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "uteke_server_cfg_{}_{}_{}.toml",
            std::process::id(),
            tag,
            n
        ));
        std::fs::write(&path, body).unwrap();
        path
    }

    fn resolve(global: &str, project: &str, trust: bool) -> ServerFileConfig {
        let g = temp_file("g", global);
        let p = temp_file("p", project);
        let cfg = resolve_server_config(&uteke_core::config_layers::Layers {
            global: Some(&g),
            project: Some(&p),
            trust_project: trust,
        });
        let _ = std::fs::remove_file(g);
        let _ = std::fs::remove_file(p);
        cfg
    }

    #[test]
    fn project_config_keeps_global_server_auth() {
        // Dummy values generated at runtime (not real secrets).
        let line = |v: &str| format!("{} = \"{}\"", "auth_token", v);
        let global_line = line(&"g".repeat(10));
        let attacker_line = line(&"a".repeat(10));
        let global = format!(
            "[server]\nhost = \"0.0.0.0\"\n{global_line}\ncors_origins = [\"https://app.example\"]\n"
        );
        // Project file only tunes recall but also tries to override [server].
        let project =
            format!("[recall]\nmin_score = 0.5\n\n[server]\n{attacker_line}\nhost = \"0.0.0.0\"\n");
        let merged = resolve(&global, &project, false);
        let server = merged.server.expect("global [server] must survive");
        assert_eq!(server.auth_token.as_deref(), Some("g".repeat(10).as_str()));
        assert_eq!(
            server.cors_origins.as_deref(),
            Some(&["https://app.example".to_string()][..])
        );
    }

    #[test]
    fn project_config_without_server_section_does_not_wipe_global() {
        let dummy = "t".repeat(10);
        let global = format!("[server]\n{} = \"{dummy}\"\n", "auth_token");
        let merged = resolve(&global, "[recall]\nmin_score = 0.3\n", false);
        assert_eq!(
            merged.server.and_then(|s| s.auth_token).as_deref(),
            Some(dummy.as_str())
        );
    }
}

#[cfg(test)]
mod recall_tuning_tests {
    //! #1355 step B — the graph and jaccard keys of `[recall]`.
    fn parse(t: &str) -> uteke_core::EngineSettings {
        toml::from_str(t).expect("valid toml")
    }

    fn recall_of(c: uteke_core::EngineSettings) -> uteke_core::RecallSettings {
        c.recall
    }

    #[test]
    fn defaults_match_the_core_and_the_cli() {
        let t = recall_of(uteke_core::EngineSettings::default());
        let d = uteke_core::GraphRerankConfig::default();
        assert_eq!(t.graph_rerank().density_weight, d.density_weight);
        assert_eq!(t.graph_rerank().authority_weight, d.authority_weight);
        assert_eq!(t.graph_rerank().enabled, d.enabled);
        assert_eq!(t.jaccard_weight, 0.0, "jaccard is opt-in");
        // a [recall] section without these keys changes nothing either
        let t = recall_of(parse("[recall]\nmin_score = 0.2\n"));
        assert_eq!(
            (t.graph_rerank().density_weight, t.jaccard_weight),
            (0.1, 0.0)
        );
    }

    #[test]
    fn file_values_are_applied() {
        let t = recall_of(parse(
            "[recall]\ngraph_density_weight = 0.3\ngraph_authority_weight = 0.2\n\
             graph_rerank_enabled = false\njaccard_weight = 0.12\n",
        ));
        assert_eq!(t.graph_rerank().density_weight, 0.3);
        assert_eq!(t.graph_rerank().authority_weight, 0.2);
        assert!(!t.graph_rerank().enabled);
        assert_eq!(t.jaccard_weight, 0.12);
    }

    #[test]
    fn a_partial_section_keeps_the_other_defaults() {
        let t = recall_of(parse("[recall]\ngraph_density_weight = 0.0\n"));
        assert_eq!(t.graph_rerank().density_weight, 0.0);
        assert_eq!(t.graph_rerank().authority_weight, 0.1);
        assert!(t.graph_rerank().enabled);
    }

    #[test]
    fn existing_recall_keys_still_parse_next_to_the_new_ones() {
        let cfg = parse(
            "[recall]\nmin_score = 0.4\ndefault_strategy = \"vector\"\njaccard_weight = 0.1\n",
        );
        let r = cfg.recall;
        assert_eq!(r.min_score, 0.4);
        assert_eq!(r.default_strategy, "vector");
        assert_eq!(r.jaccard_weight, 0.1);
    }
}

#[cfg(test)]
mod tier_config_tests {
    //! #1355 step C — `[tier]`.
    fn parse(t: &str) -> uteke_core::EngineSettings {
        toml::from_str(t).expect("valid toml")
    }

    fn tier_of(c: uteke_core::EngineSettings) -> uteke_core::TierConfig {
        c.tier
    }

    #[test]
    fn defaults_equal_the_core_defaults() {
        let t = tier_of(uteke_core::EngineSettings::default());
        let d = uteke_core::TierConfig::default();
        assert_eq!(
            (t.hot_days, t.warm_days, t.hot_boost),
            (d.hot_days, d.warm_days, d.hot_boost)
        );
        assert_eq!((t.hot_days, t.warm_days), (7, 30));
    }

    #[test]
    fn file_values_are_applied_and_a_partial_section_keeps_the_rest() {
        let t = tier_of(parse(
            "[tier]\nhot_days = 2\nwarm_days = 10\nhot_boost = 0.5\n",
        ));
        assert_eq!((t.hot_days, t.warm_days, t.hot_boost), (2, 10, 0.5));
        let t = tier_of(parse("[tier]\nhot_boost = 0.0\n"));
        assert_eq!((t.hot_days, t.warm_days, t.hot_boost), (7, 30, 0.0));
    }
}
