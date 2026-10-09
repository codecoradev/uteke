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
    let defaults = uteke_core::DreamConfig::default();
    // Embedding backend + vector engine from uteke.toml / env (#1399). Fail
    // fast on an unsupported backend instead of opening the store with a
    // different embedder than the operator asked for.
    let embedding = match resolve_embedding(
        &config,
        std::env::var("UTEKE_EMBEDDING_BACKEND").ok().as_deref(),
    ) {
        Ok(e) => e,
        Err(msg) => {
            error!("{msg}");
            std::process::exit(1);
        }
    };
    info!("Embedding backend: {}", embedding.backend);
    let uteke = match Uteke::open_with_embedding_and_graph(
        &db_path,
        &embedding.backend,
        embedding.settings,
        uteke_core::TierConfig::default(),
        uteke_core::RecallConfig::default(),
        uteke_core::GraphRerankConfig::default(),
        embedding.vector_backend.as_deref(),
    ) {
        Ok(mut u) => {
            // [embed_fallback] + UTEKE_EMBED_FALLBACK_* (#1355): only when complete.
            if let Some(fallback) = resolve_embed_fallback(&config, |k| std::env::var(k).ok()) {
                info!("Embedding fallback: enabled ({})", fallback.base_url);
                u.set_fallback_settings(fallback);
            }
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
        host_guard: context::HostGuard::new(&host, &allowed_hosts),
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
    embedding: Option<EmbeddingFileSection>,
    /// `[vector]`: which vector engine runs when both are compiled in (#1168).
    vector: Option<VectorFileSection>,
    /// `[embed_fallback]`: second embedding endpoint used when the primary fails (#1355).
    embed_fallback: Option<EmbedFallbackFileSection>,
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

/// `[embedding]` in uteke.toml, same keys as the CLI. Empty / missing values
/// fall back to the backend defaults; `UTEKE_EMBEDDING_*` env vars win over
/// these at resolve time inside the core.
#[derive(serde::Deserialize, Default, Clone)]
struct EmbeddingFileSection {
    backend: Option<String>,
    model: Option<String>,
    api_key: Option<String>,
    base_url: Option<String>,
    endpoint_path: Option<String>,
    dims: Option<usize>,
}

/// `[vector]` in uteke.toml.
#[derive(serde::Deserialize, Default, Clone)]
struct VectorFileSection {
    backend: Option<String>,
}

/// `[embed_fallback]` in uteke.toml, same keys as the CLI. `UTEKE_EMBED_FALLBACK_*`
/// env vars win over these (resolved in the core).
#[derive(serde::Deserialize, Default, Clone)]
struct EmbedFallbackFileSection {
    api_key: Option<String>,
    base_url: Option<String>,
    endpoint_path: Option<String>,
    model: Option<String>,
}

/// The fallback embedder to apply, or `None` when it is not fully configured
/// (api_key, base_url AND model). Env overrides come from `lookup`.
fn resolve_embed_fallback(
    config: &ServerFileConfig,
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<uteke_core::FallbackSettings> {
    let file = config.embed_fallback.clone().unwrap_or_default();
    let settings = uteke_core::FallbackSettings {
        api_key: file.api_key.unwrap_or_default(),
        base_url: file.base_url.unwrap_or_default(),
        endpoint_path: file.endpoint_path.unwrap_or_default(),
        model: file.model.unwrap_or_default(),
    }
    .with_overrides_from(lookup);
    settings.is_configured().then_some(settings)
}

/// Backends `Uteke` can initialize lazily (same list as the CLI validates).
const SUPPORTED_EMBEDDING_BACKENDS: &[&str] = &["onnx", "openai", "ollama"];

/// What the server opens the store with.
struct EmbeddingResolution {
    backend: String,
    settings: uteke_core::EmbeddingSettings,
    vector_backend: Option<String>,
}

/// Resolve the embedder: `UTEKE_EMBEDDING_BACKEND` (non-empty) wins over
/// `[embedding] backend`, which wins over the `onnx` default. An unsupported
/// backend is an error with an actionable message (#1399).
fn resolve_embedding(
    config: &ServerFileConfig,
    env_backend: Option<&str>,
) -> Result<EmbeddingResolution, String> {
    let file = config.embedding.clone().unwrap_or_default();
    let backend = env_backend
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(str::to_string)
        .or_else(|| file.backend.clone().filter(|b| !b.trim().is_empty()))
        .unwrap_or_else(|| "onnx".to_string());
    if !SUPPORTED_EMBEDDING_BACKENDS.contains(&backend.as_str()) {
        return Err(format!(
            "Unsupported embedding backend '{backend}'. Supported: {}. \
             Set UTEKE_EMBEDDING_BACKEND or [embedding] backend in uteke.toml.",
            SUPPORTED_EMBEDDING_BACKENDS.join(", ")
        ));
    }
    Ok(EmbeddingResolution {
        backend,
        settings: uteke_core::EmbeddingSettings {
            api_key: file.api_key.unwrap_or_default(),
            base_url: file.base_url.unwrap_or_default(),
            endpoint_path: file.endpoint_path.unwrap_or_default(),
            model: file.model.unwrap_or_default(),
            dims: file.dims.unwrap_or(0),
        },
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
            merged.recall.is_some(),
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
        let recall = merged.recall.expect("recall section");
        assert_eq!(recall.min_score, Some(0.6));
        assert_eq!(recall.default_strategy.as_deref(), Some("vector"));
    }
}

#[cfg(test)]
mod embedding_config_tests {
    //! #1399 — the server honours `[embedding]` / `[vector]` / env.
    use super::*;

    fn parse(toml_text: &str) -> ServerFileConfig {
        toml::from_str(toml_text).expect("valid toml")
    }

    #[test]
    fn default_is_onnx_with_empty_settings() {
        let r = resolve_embedding(&ServerFileConfig::default(), None).unwrap();
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
        let r = resolve_embedding(&cfg, None).unwrap();
        assert_eq!(r.backend, "openai");
        assert_eq!(r.settings.base_url, "http://embor:8355/v1");
        assert_eq!(r.settings.model, "embeddinggemma-q4");
        assert_eq!(r.settings.endpoint_path, "/embed");
        assert_eq!(r.settings.dims, 768);
        assert_eq!(r.vector_backend.as_deref(), Some("vecq"));
    }

    #[test]
    fn env_backend_wins_over_toml_and_empty_env_is_ignored() {
        let cfg = parse("[embedding]\nbackend = \"ollama\"\n");
        assert_eq!(
            resolve_embedding(&cfg, Some("openai")).unwrap().backend,
            "openai"
        );
        assert_eq!(resolve_embedding(&cfg, Some("")).unwrap().backend, "ollama");
        assert_eq!(
            resolve_embedding(&cfg, Some("  ")).unwrap().backend,
            "ollama"
        );
        // env alone, no [embedding] section (the labs setup)
        let r = resolve_embedding(&ServerFileConfig::default(), Some("openai")).unwrap();
        assert_eq!(r.backend, "openai");
    }

    #[test]
    fn unsupported_backend_is_an_actionable_error() {
        let err = resolve_embedding(&ServerFileConfig::default(), Some("cohere"))
            .err()
            .expect("must fail");
        assert!(
            err.contains("cohere") && err.contains("onnx, openai, ollama"),
            "{err}"
        );
        assert!(err.contains("UTEKE_EMBEDDING_BACKEND"), "{err}");
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
        let r = resolve_embedding(&cfg, None).unwrap();
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
