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
    // Embedding backend + vector engine from uteke.toml / env (#1399). Fail
    // fast on an unsupported backend instead of opening the store with a
    // different embedder than the operator asked for.
    let embedding = match resolve_embedding(&config, |name| std::env::var(name).ok()) {
        Ok(e) => e,
        Err(msg) => {
            error!("{msg}");
            std::process::exit(1);
        }
    };
    info!("Embedding backend: {}", embedding.backend);
    // The engine sections: the file, then the UTEKE_RECALL_* / UTEKE_GRAPH_*
    // overrides (as in the CLI), an unknown default strategy replaced by
    // hybrid (#1034: a typo must not 400 every recall request), and the
    // embedding fallback only when it is complete.
    let mut engine = config.engine.clone();
    engine.recall = engine.recall.with_env_overrides().sanitized();
    let fallback = resolve_embed_fallback(&config, |k| std::env::var(k).ok());
    if let Some(f) = &fallback {
        info!("Embedding fallback: enabled ({})", f.base_url);
    }
    engine.embed_fallback = fallback.unwrap_or_default();
    let uteke = match Uteke::open_with_embedding_and_graph(
        &db_path,
        &embedding.backend,
        embedding.settings,
        engine.tier,
        uteke_core::RecallConfig::default(),
        engine.recall.graph_rerank(),
        embedding.vector_backend.as_deref(),
    ) {
        Ok(mut u) => {
            // [privacy] (#1464): secret filter on the write path.
            let privacy = uteke_core::privacy::load_config();
            info!("Privacy filter: {:?}", privacy.mode);
            u.set_privacy_config(privacy);
            // Tier, graph rerank, Jaccard weight, recall defaults, dream
            // thresholds, lifecycle and the embedding fallback, applied the same
            // way as in the CLI and MCP.
            u.apply_settings(&engine);
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
    let ctx = context::ReqCtx {
        auth_token_hash,
        read_only_token_hash,
        cors_origins: cors_origins.clone(),
        recall_config: Some(engine.recall.clone()),
        extraction_config: config.extraction.clone(),
        host_guard: context::HostGuard::new(&host, &allowed_hosts),
        store_id: uteke_core::store_identity(&home),
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
    /// `[embedding]`: which embedder the server uses (#1399).
    #[serde(default)]
    embedding: uteke_core::embedding_config::EmbeddingSection,
    /// `[vector]`: which vector engine runs when both are compiled in (#1168).
    vector: Option<VectorFileSection>,
    /// `[tier]`, `[recall]`, `[dream]`, `[lifecycle]` and `[embed_fallback]`: the
    /// engine sections, one schema shared with the CLI (#1515).
    #[serde(flatten)]
    engine: uteke_core::EngineSettings,
    server: Option<ServerFileSection>,
    extraction: Option<uteke_core::extraction::ExtractionConfig>,
    maintenance: Option<MaintenanceFileSection>,
    /// Deprecated: superseded by [lifecycle] section (#934). Kept for backward-compat deserialization.
    #[allow(dead_code)]
    aging: Option<AgingFileSection>,
}

/// `[vector]` in uteke.toml.
#[derive(serde::Deserialize, Default, Clone)]
struct VectorFileSection {
    backend: Option<String>,
}

/// The fallback embedder to apply, or `None` when it is not fully configured
/// (api_key, base_url AND model). Env overrides come from `lookup`.
fn resolve_embed_fallback(
    config: &ServerFileConfig,
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<uteke_core::FallbackSettings> {
    let settings = config
        .engine
        .embed_fallback
        .clone()
        .with_overrides_from(lookup);
    settings.is_configured().then_some(settings)
}

/// What the server opens the store with.
struct EmbeddingResolution {
    backend: String,
    settings: uteke_core::EmbeddingSettings,
    vector_backend: Option<String>,
}

/// Resolve the embedder with the core's one resolution (backend list,
/// validation, `UTEKE_EMBEDDING_*` precedence; #1399, #1537) and add the
/// `[vector]` preference.
fn resolve_embedding(
    config: &ServerFileConfig,
    get: impl Fn(&str) -> Option<String>,
) -> Result<EmbeddingResolution, String> {
    let resolved = config.embedding.resolve_from(get)?;
    Ok(EmbeddingResolution {
        backend: resolved.backend,
        settings: resolved.settings,
        // UTEKE_VECTOR_BACKEND (higher precedence) is read inside the core.
        vector_backend: config.vector.as_ref().and_then(|v| v.backend.clone()),
    })
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
    let global = uteke_core::uteke_home().ok().map(|h| h.join("uteke.toml"));
    let project = std::env::current_dir()
        .ok()
        .map(|cwd| cwd.join(".uteke").join("uteke.toml"));
    resolve_server_config(&uteke_core::config_layers::Layers {
        global: global.as_deref(),
        project: project.as_deref(),
        trust_project: matches!(
            std::env::var("UTEKE_TRUST_PROJECT_CONFIG").as_deref(),
            Ok("1") | Ok("true")
        ),
    })
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
        assert!(
            merged.engine.recall.min_score == 0.5,
            "project recall tuning still applies"
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

    #[test]
    fn project_recall_keys_merge_per_key_not_per_section() {
        // The old server overlay replaced whole sections: a project file setting
        // one [recall] key wiped the other global [recall] keys.
        let merged = resolve(
            "[recall]\nmin_score = 0.3\ndefault_strategy = \"vector\"\n",
            "[recall]\nmin_score = 0.6\n",
            false,
        );
        let recall = merged.engine.recall;
        assert_eq!(recall.min_score, 0.6);
        assert_eq!(recall.default_strategy, "vector");
    }
}

#[cfg(test)]
mod embedding_config_tests {
    //! #1399 — the server honours `[embedding]` / `[vector]` / env.
    use super::*;

    fn parse(toml_text: &str) -> ServerFileConfig {
        toml::from_str(toml_text).expect("valid toml")
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn default_is_onnx_with_empty_settings() {
        let r = resolve_embedding(&ServerFileConfig::default(), no_env).unwrap();
        assert_eq!(r.backend, "onnx");
        assert!(r.settings.base_url.is_empty() && r.settings.model.is_empty());
        assert_eq!(r.settings.dims, 0);
        assert_eq!(r.vector_backend, None);
    }

    #[test]
    fn toml_selects_an_external_backend_and_passes_its_settings() {
        let cfg = parse(
            "[embedding]\nbackend = \"openai\"\nbase_url = \"http://embor:8355/v1\"\n\
             model = \"embeddinggemma-q4\"\ndims = 768\nendpoint_path = \"/embed\"\n\
             [vector]\nbackend = \"vecq\"\n",
        );
        let r = resolve_embedding(&cfg, no_env).unwrap();
        assert_eq!(r.backend, "openai");
        assert_eq!(r.settings.base_url, "http://embor:8355/v1");
        assert_eq!(r.settings.model, "embeddinggemma-q4");
        assert_eq!(r.settings.endpoint_path, "/embed");
        assert_eq!(r.settings.dims, 768);
        assert_eq!(r.vector_backend.as_deref(), Some("vecq"));
    }

    #[test]
    fn the_env_backend_is_resolved_by_the_core_rules() {
        let cfg = parse("[embedding]\nbackend = \"ollama\"\n");
        let env = |v: &'static str| {
            move |n: &str| (n == "UTEKE_EMBEDDING_BACKEND").then(|| v.to_string())
        };
        assert_eq!(
            resolve_embedding(&cfg, env("openai")).unwrap().backend,
            "openai"
        );
        assert_eq!(
            resolve_embedding(&cfg, env("  ")).unwrap().backend,
            "ollama"
        );
        let err = resolve_embedding(&ServerFileConfig::default(), env("cohere"))
            .err()
            .expect("must fail");
        assert!(
            err.contains("cohere") && err.contains("UTEKE_EMBEDDING_BACKEND"),
            "{err}"
        );
    }

    #[test]
    fn untrusted_project_file_cannot_pick_the_embedder() {
        // The shared layered resolver strips embedding.backend / base_url from
        // the project-local file unless the project config is trusted.
        let dir = std::env::temp_dir().join(format!("uteke_emb_cfg_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let global = dir.join("global.toml");
        let project = dir.join("project.toml");
        std::fs::write(&global, "[embedding]\nbackend = \"onnx\"\n").unwrap();
        std::fs::write(
            &project,
            "[embedding]\nbackend = \"openai\"\nbase_url = \"https://evil.example\"\n",
        )
        .unwrap();
        let cfg = resolve_server_config(&uteke_core::config_layers::Layers {
            global: Some(&global),
            project: Some(&project),
            trust_project: false,
        });
        let r = resolve_embedding(&cfg, no_env).unwrap();
        assert_eq!(r.backend, "onnx");
        assert!(r.settings.base_url.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod embed_fallback_config_tests {
    //! #1355 step A — the server reads `[embed_fallback]`.
    use super::*;

    fn parse(t: &str) -> ServerFileConfig {
        toml::from_str(t).expect("valid toml")
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn absent_or_partial_section_means_no_fallback() {
        assert!(resolve_embed_fallback(&ServerFileConfig::default(), no_env).is_none());
        let partial = parse("[embed_fallback]\napi_key = \"k\"\nbase_url = \"https://x\"\n");
        assert!(
            resolve_embed_fallback(&partial, no_env).is_none(),
            "model missing"
        );
    }

    #[test]
    fn a_complete_section_is_applied_with_its_endpoint_path() {
        let cfg = parse(
            "[embed_fallback]\napi_key = \"k\"\nbase_url = \"https://x\"\nmodel = \"m\"\nendpoint_path = \"/embed\"\n",
        );
        let f = resolve_embed_fallback(&cfg, no_env).expect("configured");
        assert_eq!(
            (f.api_key.as_str(), f.base_url.as_str()),
            ("k", "https://x")
        );
        assert_eq!(
            (f.model.as_str(), f.endpoint_path.as_str()),
            ("m", "/embed")
        );
    }

    #[test]
    fn env_overrides_the_file_and_can_complete_it() {
        let cfg = parse("[embed_fallback]\nbase_url = \"https://file\"\nmodel = \"m\"\n");
        let f = resolve_embed_fallback(&cfg, |k| match k {
            "UTEKE_EMBED_FALLBACK_API_KEY" => Some("env-key".to_string()),
            "UTEKE_EMBED_FALLBACK_BASE_URL" => Some("https://env".to_string()),
            _ => None,
        })
        .expect("env completes the file");
        assert_eq!(
            (f.api_key.as_str(), f.base_url.as_str()),
            ("env-key", "https://env")
        );
    }

    #[test]
    fn untrusted_project_file_cannot_set_the_fallback_endpoint() {
        let dir = std::env::temp_dir().join(format!("uteke_fb_cfg_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let project = dir.join("project.toml");
        std::fs::write(
            &project,
            "[embed_fallback]\napi_key = \"stolen\"\nbase_url = \"https://evil.example\"\nmodel = \"m\"\n",
        )
        .unwrap();
        let cfg = resolve_server_config(&uteke_core::config_layers::Layers {
            global: None,
            project: Some(&project),
            trust_project: false,
        });
        assert!(resolve_embed_fallback(&cfg, no_env).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod recall_tuning_tests {
    //! #1355 step B — the server reads the graph and jaccard keys of `[recall]`.
    use super::*;

    fn parse(t: &str) -> ServerFileConfig {
        toml::from_str(t).expect("valid toml")
    }

    fn recall_of(c: ServerFileConfig) -> uteke_core::RecallSettings {
        c.engine.recall
    }

    #[test]
    fn defaults_match_the_core_and_the_cli() {
        let t = recall_of(ServerFileConfig::default());
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
        let r = cfg.engine.recall;
        assert_eq!(r.min_score, 0.4);
        assert_eq!(r.default_strategy, "vector");
        assert_eq!(r.jaccard_weight, 0.1);
    }
}

#[cfg(test)]
mod tier_config_tests {
    //! #1355 step C — the server reads `[tier]`.
    use super::*;

    fn parse(t: &str) -> ServerFileConfig {
        toml::from_str(t).expect("valid toml")
    }

    fn tier_of(c: ServerFileConfig) -> uteke_core::TierConfig {
        c.engine.tier
    }

    #[test]
    fn defaults_equal_the_core_defaults() {
        let t = tier_of(ServerFileConfig::default());
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
