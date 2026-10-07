//! Uteke HTTP Server — persistent warm memory for AI agents.
//!
//! Keeps the embedding model loaded in RAM for <50ms recall.
//! Usage: `uteke-serve [--port 8767] [--host 127.0.0.1] [--auth-token <TOKEN>]`

// api_registry is runtime data now: GET /routes serves it (#1289),
// in addition to its original docgen role.
mod api_registry;
mod context;
mod handlers;
mod types;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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
                println!("  --read-only-token <T> Read-only API token (GET endpoints only) (#409)");
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
    let defaults = uteke_core::DreamConfig::default();
    let uteke = match Uteke::open(&db_path) {
        Ok(mut u) => {
            // Apply dream pipeline thresholds from config (#731)
            if let Some(ref dc) = config.dream {
                u.set_dream_config(uteke_core::DreamConfig {
                    contradict_similarity_threshold: dc
                        .contradict_similarity_threshold
                        .unwrap_or(defaults.contradict_similarity_threshold),
                    contradict_tag_jaccard_min: dc
                        .contradict_tag_jaccard_min
                        .unwrap_or(defaults.contradict_tag_jaccard_min),
                    contradict_max_memories: dc
                        .contradict_max_memories
                        .unwrap_or(defaults.contradict_max_memories),
                    dedup_threshold: dc.dedup_threshold.unwrap_or(defaults.dedup_threshold),
                    orphan_importance_threshold: dc
                        .orphan_importance_threshold
                        .unwrap_or(defaults.orphan_importance_threshold),
                });
                info!(
                    "Dream config loaded: dedup={:.2}, orphan_thresh={:.2}",
                    dc.dedup_threshold.unwrap_or(defaults.dedup_threshold),
                    dc.orphan_importance_threshold
                        .unwrap_or(defaults.orphan_importance_threshold),
                );
            }

            // Apply lifecycle config from uteke.toml [lifecycle] section (#928)
            if let Some(ref lc) = config.lifecycle {
                let defaults = uteke_core::LifecycleConfig::default();
                u.set_lifecycle_config(uteke_core::LifecycleConfig {
                    soft_delete_only: lc.soft_delete_only.unwrap_or(defaults.soft_delete_only),
                    auto_aging_enabled: lc
                        .auto_aging_enabled
                        .unwrap_or(defaults.auto_aging_enabled),
                    auto_aging_interval_hours: lc
                        .auto_aging_interval_hours
                        .unwrap_or(defaults.auto_aging_interval_hours),
                    min_age_days: lc.min_age_days.unwrap_or(defaults.min_age_days),
                    max_access_count: lc.max_access_count.unwrap_or(defaults.max_access_count),
                    max_deprecate_percent: lc
                        .max_deprecate_percent
                        .unwrap_or(defaults.max_deprecate_percent),
                    min_deprecate_per_cycle: lc
                        .min_deprecate_per_cycle
                        .unwrap_or(defaults.min_deprecate_per_cycle),
                    max_deprecate_per_cycle: lc
                        .max_deprecate_per_cycle
                        .unwrap_or(defaults.max_deprecate_per_cycle),
                    deprecated_ttl_days: lc
                        .deprecated_ttl_days
                        .unwrap_or(defaults.deprecated_ttl_days),
                    auto_prune_enabled: lc
                        .auto_prune_enabled
                        .unwrap_or(defaults.auto_prune_enabled),
                    dream_dedup_soft_delete: lc
                        .dream_dedup_soft_delete
                        .unwrap_or(defaults.dream_dedup_soft_delete),
                    dream_compact_soft_delete: lc
                        .dream_compact_soft_delete
                        .unwrap_or(defaults.dream_compact_soft_delete),
                });
                info!(
                    "Lifecycle config loaded: soft_delete_only={}, ttl={}d, max_deprecate={:.1}%",
                    lc.soft_delete_only.unwrap_or(defaults.soft_delete_only),
                    lc.deprecated_ttl_days
                        .unwrap_or(defaults.deprecated_ttl_days),
                    lc.max_deprecate_percent
                        .unwrap_or(defaults.max_deprecate_percent),
                );
            }
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
    let ctx = context::ReqCtx {
        auth_token_hash,
        read_only_token_hash,
        cors_origins: cors_origins.clone(),
        recall_config: {
            // Sanitize [recall] default_strategy at startup (#1034): the
            // server only recently started honoring this key. An invalid
            // value (typo in uteke.toml) must not 400 every recall request
            // with a message blaming the request — warn loudly and fall
            // back to hybrid instead. Explicit request-level `strategy`
            // values remain strictly validated (HTTP 400).
            let mut recall = config.recall.clone();
            if let Some(r) = recall.as_mut() {
                if let Some(s) = r.default_strategy.as_deref() {
                    if uteke_core::RecallStrategy::from_str_opt(s).is_none() {
                        warn!(
                            "Invalid [recall] default_strategy='{s}' in config — \
                             falling back to 'hybrid'. \
                             Expected: vector | fts5 | hybrid | graph."
                        );
                        r.default_strategy = Some("hybrid".to_string());
                    }
                }
            }
            recall
        },
        extraction_config: config.extraction.clone(),
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
    let lifecycle_enabled = config
        .lifecycle
        .as_ref()
        .and_then(|lc| lc.auto_aging_enabled)
        .unwrap_or(true);
    let lifecycle_hours = config
        .lifecycle
        .as_ref()
        .and_then(|lc| lc.auto_aging_interval_hours)
        .unwrap_or(168) // weekly by default
        .max(1); // Minimum 1 hour to prevent busy loop
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
            let response = handlers::route(&uteke, &ctx, &mut req);
            if let Err(e) = req.respond(response) {
                warn!("Response error: {e}");
            }
            // Release slot and notify the waiting accept loop.
            let (lock, cvar) = &*pair;
            let mut active = lock.lock().unwrap();
            *active -= 1;
            cvar.notify_one();
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
    recall: Option<RecallFileSection>,
    extraction: Option<uteke_core::extraction::ExtractionConfig>,
    maintenance: Option<MaintenanceFileSection>,
    /// Deprecated: superseded by [lifecycle] section (#934). Kept for backward-compat deserialization.
    #[allow(dead_code)]
    aging: Option<AgingFileSection>,
    dream: Option<DreamFileSection>,
    lifecycle: Option<LifecycleFileSection>,
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

/// Dream pipeline thresholds from uteke.toml [dream] section (#731).
#[derive(serde::Deserialize, Default, Clone)]
struct DreamFileSection {
    contradict_similarity_threshold: Option<f32>,
    contradict_tag_jaccard_min: Option<f32>,
    contradict_max_memories: Option<usize>,
    dedup_threshold: Option<f32>,
    orphan_importance_threshold: Option<f64>,
}

/// Memory lifecycle config from uteke.toml [lifecycle] section (#928).
#[derive(serde::Deserialize, Default, Clone)]
struct LifecycleFileSection {
    soft_delete_only: Option<bool>,
    auto_aging_enabled: Option<bool>,
    auto_aging_interval_hours: Option<u64>,
    min_age_days: Option<u32>,
    max_access_count: Option<u32>,
    max_deprecate_percent: Option<f64>,
    min_deprecate_per_cycle: Option<usize>,
    max_deprecate_per_cycle: Option<usize>,
    deprecated_ttl_days: Option<u32>,
    auto_prune_enabled: Option<bool>,
    dream_dedup_soft_delete: Option<bool>,
    dream_compact_soft_delete: Option<bool>,
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
/// 1. $UTEKE_HOME/uteke.toml (or ~/.codecora/uteke/uteke.toml)
/// 2. $CWD/.uteke/uteke.toml
fn load_uteke_toml() -> ServerFileConfig {
    let mut config = ServerFileConfig::default();

    let global = match uteke_core::uteke_home() {
        Ok(h) => Some(h.join("uteke.toml")),
        Err(_) => None,
    };
    let project = std::env::current_dir()
        .ok()
        .map(|cwd| cwd.join(".uteke").join("uteke.toml"));

    if let Some(parsed) = global.as_deref().and_then(read_server_toml) {
        config = parsed;
    }
    if let Some(parsed) = project.as_deref().and_then(read_server_toml) {
        config = overlay_project_config(config, parsed);
    }

    config
}

fn read_server_toml(path: &std::path::Path) -> Option<ServerFileConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str::<ServerFileConfig>(&content).ok()
}

/// Overlay a project-local config (`$CWD/.uteke/uteke.toml`) on the global one.
///
/// Sections are merged one by one (a project file that only tunes `[recall]`
/// must not wipe the global `[server]` auth settings), and the project file is
/// never allowed to set `[server]` (host, auth tokens, CORS origins) or
/// `[extraction]` (LLM endpoint + key): it lives in the working tree, which
/// may be an untrusted clone.
fn overlay_project_config(
    mut base: ServerFileConfig,
    project: ServerFileConfig,
) -> ServerFileConfig {
    if project.server.is_some() {
        warn!(
            "Ignoring [server] section in project .uteke/uteke.toml (set it in the global config)"
        );
    }
    if project.extraction.is_some() {
        warn!(
            "Ignoring [extraction] section in project .uteke/uteke.toml (set it in the global config)"
        );
    }
    if project.recall.is_some() {
        base.recall = project.recall;
    }
    if project.maintenance.is_some() {
        base.maintenance = project.maintenance;
    }
    if project.aging.is_some() {
        base.aging = project.aging;
    }
    if project.dream.is_some() {
        base.dream = project.dream;
    }
    if project.lifecycle.is_some() {
        base.lifecycle = project.lifecycle;
    }
    base
}

#[cfg(test)]
mod config_overlay_tests {
    use super::*;

    fn parse(s: &str) -> ServerFileConfig {
        toml::from_str(s).unwrap()
    }

    #[test]
    fn project_config_keeps_global_server_auth() {
        // Dummy values generated at runtime (not real secrets).
        let line = |v: &str| format!("{} = \"{}\"", "auth_token", v);
        let global_line = line(&"g".repeat(10));
        let attacker_line = line(&"a".repeat(10));
        let global = parse(&format!(
            r#"
[server]
host = "0.0.0.0"
{global_line}
cors_origins = ["https://app.example"]
"#
        ));
        // Project file only tunes recall but also tries to override [server].
        let project = parse(&format!(
            r#"
[recall]
min_score = 0.5

[server]
{attacker_line}
host = "0.0.0.0"
"#
        ));
        let merged = overlay_project_config(global, project);
        let server = merged.server.expect("global [server] must survive");
        assert_eq!(server.auth_token.as_deref(), Some("g".repeat(10).as_str()));
        assert_eq!(
            server.cors_origins.as_deref(),
            Some(&["https://app.example".to_string()][..])
        );
        assert!(
            merged.recall.is_some(),
            "project recall tuning still applies"
        );
    }

    #[test]
    fn project_config_without_server_section_does_not_wipe_global() {
        let token = "t".repeat(10);
        let global = parse(&format!("[server]\n{} = \"{token}\"\n", "auth_token"));
        let project = parse("[recall]\nmin_score = 0.3\n");
        let merged = overlay_project_config(global, project);
        assert_eq!(
            merged.server.and_then(|s| s.auth_token).as_deref(),
            Some(token.as_str())
        );
    }
}
