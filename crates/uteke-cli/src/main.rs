//! Uteke CLI — persistent memory for AI agents.

mod cli;
mod commands;
mod config;
mod extract;
mod init;
mod logging;
mod onboard;
mod output;

use clap::{CommandFactory, Parser};
use cli::{Cli, Commands};
use config::Config;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use uteke_core::Uteke;

/// Global flag set by SIGINT handler.
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Store resolution order (#1105): `--store` flag > `UTEKE_HOME` env >
/// `uteke.toml [store].path` > built-in default. `UTEKE_HOME` previously only
/// affected models/config/logs while the DB silently stayed on the default
/// (often production) path — an operator trap that could read/write the wrong
/// database from an "isolated-looking" invocation.
pub(crate) fn resolve_store_path(cli: &Cli, config: &Config) -> String {
    match cli.store.as_deref() {
        Some(s) => s.to_string(),
        None => default_store_path(config),
    }
}

/// The store used when no `--store` flag is given: `UTEKE_HOME` env, then
/// `uteke.toml [store].path`. Shared by `resolve_store_path` and the server
/// routing decision so the two cannot drift.
fn default_store_path(config: &Config) -> String {
    if let Ok(home) = std::env::var("UTEKE_HOME") {
        if !home.is_empty() {
            return home;
        }
    }
    Config::expand_tilde(&config.store.path)
}

/// Whether two store paths name the same store: canonical paths when both
/// exist, otherwise tilde-expanded strings with trailing slashes trimmed.
fn same_path(a: &str, b: &str) -> bool {
    if let (Ok(ca), Ok(cb)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        return ca == cb;
    }
    let norm = |p: &str| {
        let e = Config::expand_tilde(p);
        let t = e.trim_end_matches('/');
        if t.is_empty() { e } else { t.to_string() }
    };
    norm(a) == norm(b)
}

/// Whether to route this invocation through a running server (#1439 B4).
/// An explicit `--store` naming a different database than the default one
/// runs locally; the server only serves its own store. `--store` pointing at
/// the default store still routes (the server holds that store's index lock).
/// `default_store` is `default_store_path(config)`, injected for testability.
fn should_route_via_server(cli: &Cli, default_store: &str, server_enabled: bool) -> bool {
    if !server_enabled {
        return false;
    }
    match cli.store.as_deref() {
        None => true,
        Some(explicit) => {
            let same = same_path(explicit, default_store);
            if !same {
                tracing::debug!(
                    "--store {explicit} is not the default store, skipping server routing"
                );
            }
            same
        }
    }
}

fn main() {
    let cli = Cli::parse();

    // Initialize logging (console + file). Guard must stay alive.
    let _log_guard = logging::init(cli.verbose);

    // Background update notification (non-blocking, cached 24h).
    // Skipped for early-exit commands (completions, init, onboard, bench).
    let early_exit = matches!(
        &cli.command,
        Commands::Completions { .. }
            | Commands::Init { .. }
            | Commands::Onboard { .. }
            | Commands::Bench { .. }
            | Commands::Upgrade { .. }
    );
    // Skip update check in subprocess/batch mode (e.g. benchmarks calling
    // `uteke recall` 1000× via subprocess). Env var set by benchmark scripts
    // and any automated pipeline (#1006).
    let skip_update_check = std::env::var("UTEKE_NO_UPDATE_CHECK").is_ok();

    let update_handle = if !early_exit && !skip_update_check {
        commands::update_check::spawn()
    } else {
        None
    };

    match &cli.command {
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            clap_complete::generate(*shell, &mut cmd, &name, &mut io::stdout());
            std::process::exit(0);
        }
        Commands::Init { .. } => {
            if let Err(e) = init::run_init_command(&cli) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        Commands::Onboard { .. } => {
            if let Err(e) = onboard::run(&cli) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        Commands::Bench { counts, json } => {
            // Bench creates its own temp stores — skip opening the user store.
            if let Err(e) = commands::bench::run_bench(*json, counts.clone()) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        Commands::Upgrade { yes } => {
            // Upgrade only needs network + filesystem — skip opening the store (#772).
            if let Err(e) = commands::upgrade::run(*yes) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        _ => {}
    }

    // Ensure config directory exists and load layered config
    Config::write_default_config();
    let config = Config::load();

    // Validate embedding backend early (fail-fast before store open)
    if let Err(e) = config.embedding.validate_backend() {
        eprintln!("Config error: {e}");
        std::process::exit(1);
    }

    // Namespace deprecation signal (#1427): one line on stderr for writes to a
    // non-default namespace; stdout and --json output are untouched.
    if let Some(notice) = commands::namespace_write_notice(&cli, &config) {
        eprintln!("{notice}");
    }

    // Check if uteke server is running — if so, route via HTTP for <50ms latency
    let server_url = format!("http://{}:{}", config.server.host, config.server.port);
    // Cheap check first: no network probe when --store names another store.
    // Then the server must serve the store this invocation resolved (#1445):
    // `UTEKE_HOME` pointing elsewhere is not sent to a server for another store.
    let server_available =
        should_route_via_server(&cli, &default_store_path(&config), config.server.enabled)
            && commands::is_server_running(&server_url)
            && commands::server_serves_store(&server_url, &resolve_store_path(&cli, &config));

    if server_available {
        tracing::info!("Server detected at {server_url}, routing via HTTP");
        match commands::run_via_server(&cli, &server_url) {
            Ok(()) => {
                if let Some(handle) = update_handle {
                    let _ = handle.join();
                }
                return;
            }
            Err(e) if e == "unsupported" => {
                tracing::info!("Command not supported via server, using local store");
            }
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    }

    // Fallback: open local store
    // This branch is reached when:
    //   1. No server detected (server_available == false)
    //   2. Server detected but command not supported via HTTP (aging, doctor, etc.)
    // Log appropriately to avoid contradictory messages (#403).
    if server_available {
        tracing::debug!("Opening local store for server-unsupported command");
    } else {
        tracing::debug!("No server detected, using local store");
    }

    let store_path = resolve_store_path(&cli, &config);

    tracing::debug!("Opening store at: {store_path}");

    let mut uteke = match Uteke::open_with_embedding_and_graph(
        &store_path,
        &config.embedding.backend,
        uteke_core::EmbeddingSettings {
            api_key: config.embedding.api_key.clone(),
            base_url: config.embedding.base_url.clone(),
            endpoint_path: config.embedding.endpoint_path.clone(),
            model: config.embedding.model.clone(),
            dims: config.embedding.dims,
        },
        uteke_core::TierConfig {
            hot_days: config.tier.hot_days as i64,
            warm_days: config.tier.warm_days as i64,
            hot_boost: config.tier.hot_boost,
        },
        uteke_core::RecallConfig {
            min_score: config.recall.min_score as f32,
        },
        uteke_core::GraphRerankConfig {
            density_weight: config.recall.graph_density_weight,
            authority_weight: config.recall.graph_authority_weight,
            enabled: config.recall.graph_rerank_enabled,
        },
        // Runtime vector-engine preference from uteke.toml [vector] (#1168).
        // UTEKE_VECTOR_BACKEND env (higher precedence) is read inside core.
        Some(config.vector.backend.as_str()),
    ) {
        Ok(mut u) => {
            // #719: apply Jaccard weight from config
            u.set_jaccard_weight(config.recall.jaccard_weight);
            // #731: apply dream pipeline thresholds from config
            u.set_dream_config(uteke_core::DreamConfig {
                contradict_similarity_threshold: config.dream.contradict_similarity_threshold,
                contradict_tag_jaccard_min: config.dream.contradict_tag_jaccard_min,
                contradict_max_memories: config.dream.contradict_max_memories,
                dedup_threshold: config.dream.dedup_threshold,
                orphan_importance_threshold: config.dream.orphan_importance_threshold,
            });
            // #928: apply lifecycle configuration from config
            u.set_lifecycle_config(config.lifecycle.to_core());
            // #1464: secret filter on the write path ([privacy] in uteke.toml)
            u.set_privacy_config(uteke_core::privacy::load_config());
            u
        }
        Err(e) => {
            eprintln!("Error: Failed to open memory store: {e}");
            std::process::exit(1);
        }
    };

    // Configure embed fallback if enabled (zero-config: only when fields are set)
    if config.embed_fallback.is_configured() {
        uteke.set_fallback_settings(uteke_core::FallbackSettings {
            api_key: config.embed_fallback.api_key.clone(),
            base_url: config.embed_fallback.base_url.clone(),
            endpoint_path: config.embed_fallback.endpoint_path.clone(),
            model: config.embed_fallback.model.clone(),
        });
    }

    ctrlc::set_handler(|| {
        SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
        eprintln!("\nInterrupt received, shutting down gracefully...");
    })
    .expect("Failed to set SIGINT handler");

    let result = commands::run_command(&cli, &mut uteke, &config);

    // Wait for update check thread to finish before shutdown.
    if let Some(handle) = update_handle {
        let _ = handle.join();
    }

    if let Err(e) = uteke.shutdown() {
        tracing::warn!("Shutdown flush failed: {e}");
    }

    if SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
        eprintln!("Shutdown complete.");
        std::process::exit(130);
    }

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

/// Resolve namespace: CLI flag > UTEKE_NAMESPACE env > config > "default"
pub(crate) fn resolve_namespace(cli: &Cli, config: &Config) -> String {
    if let Some(ns) = &cli.namespace {
        return ns.clone();
    }
    if let Ok(env_ns) = std::env::var("UTEKE_NAMESPACE") {
        if !env_ns.is_empty() {
            return env_ns;
        }
    }
    if config.store.namespace != "default" {
        return config.store.namespace.clone();
    }
    "default".to_string()
}

#[allow(unsafe_code)]
#[cfg(test)]
mod resolve_store_tests {
    use super::*;

    /// All four precedence cases mutate the process-global UTEKE_HOME, so
    /// they run sequentially inside ONE #[test] — parallel #[test]s racing
    /// on the same env var flake nondeterministically (same lesson as the
    /// ort_init tests).
    #[test]
    fn store_resolution_precedence() {
        let cfg = Config::default();
        let expanded_default = Config::expand_tilde(&cfg.store.path);

        // 1) --store flag beats everything
        // SAFETY: single-threaded test section; no other test touches this var.
        unsafe { std::env::set_var("UTEKE_HOME", "/tmp/env-home") };
        let mut cli = Cli::try_parse_from(["uteke", "stats"]).unwrap();
        cli.store = Some("/tmp/flag-store".to_string());
        assert_eq!(resolve_store_path(&cli, &cfg), "/tmp/flag-store");

        // 2) UTEKE_HOME beats config when flag absent
        cli.store = None;
        assert_eq!(resolve_store_path(&cli, &cfg), "/tmp/env-home");

        // 3) empty env falls back to config
        unsafe { std::env::set_var("UTEKE_HOME", "") };
        assert_eq!(resolve_store_path(&cli, &cfg), expanded_default);

        // 4) config default when no flag, no env
        unsafe { std::env::remove_var("UTEKE_HOME") };
        assert_eq!(resolve_store_path(&cli, &cfg), expanded_default);
    }

    /// Namespace precedence (#1078 P0 batch 2). All cases mutate the
    /// process-global UTEKE_NAMESPACE, so they run sequentially inside ONE
    /// #[test] — same rationale as store_resolution_precedence above.
    #[test]
    fn namespace_resolution_precedence() {
        let mut cfg = Config::default();

        // Baseline: nothing set → "default"
        unsafe { std::env::remove_var("UTEKE_NAMESPACE") };
        let mut cli = Cli::try_parse_from(["uteke", "stats"]).unwrap();
        assert_eq!(resolve_namespace(&cli, &cfg), "default");

        // 1) --namespace flag beats env AND config
        cfg.store.namespace = "from-config".to_string();
        unsafe { std::env::set_var("UTEKE_NAMESPACE", "from-env") };
        cli.namespace = Some("from-flag".to_string());
        assert_eq!(resolve_namespace(&cli, &cfg), "from-flag");

        // 2) UTEKE_NAMESPACE env beats config when flag absent
        cli.namespace = None;
        assert_eq!(resolve_namespace(&cli, &cfg), "from-env");

        // 3) empty env is ignored, falls back to config
        unsafe { std::env::set_var("UTEKE_NAMESPACE", "") };
        assert_eq!(resolve_namespace(&cli, &cfg), "from-config");

        // 4) config namespace used when no flag, no env
        unsafe { std::env::remove_var("UTEKE_NAMESPACE") };
        assert_eq!(resolve_namespace(&cli, &cfg), "from-config");

        // 5) config "default" literal falls through to "default"
        cfg.store.namespace = "default".to_string();
        assert_eq!(resolve_namespace(&cli, &cfg), "default");
    }
}

#[cfg(test)]
mod routing_tests {
    use super::*;

    fn cli_with_store(store: Option<&str>) -> Cli {
        let mut cli = Cli::try_parse_from(["uteke", "stats"]).unwrap();
        cli.store = store.map(str::to_string);
        cli
    }

    /// #1439 B4: an explicit `--store` naming another database must never be
    /// routed through a running server, which only serves its own store.
    #[test]
    fn other_store_never_routes_via_server() {
        let default_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let default = default_dir.path().to_str().unwrap();
        let cli = cli_with_store(Some(other_dir.path().to_str().unwrap()));
        assert!(!should_route_via_server(&cli, default, true));
        assert!(!should_route_via_server(&cli, default, false));
        // Non-existent paths fall back to string comparison.
        let cli = cli_with_store(Some("/nonexistent/uteke-other"));
        assert!(!should_route_via_server(
            &cli,
            "/nonexistent/uteke-default",
            true
        ));
    }

    /// `--store` pointing at the default store keeps routing: the server
    /// holds that store's index lock, so a local open would block.
    #[test]
    fn store_equal_to_default_still_routes() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.store.path = dir.path().to_str().unwrap().to_string();
        // Injected default store (no dependence on the UTEKE_HOME env).
        let default = Config::expand_tilde(&cfg.store.path);
        let cli = cli_with_store(Some(&cfg.store.path));
        assert!(should_route_via_server(&cli, &default, true));
        assert!(!should_route_via_server(&cli, &default, false));
        // Trailing slash on an existing path: canonicalize matches.
        let cli = cli_with_store(Some(&format!("{}/", cfg.store.path)));
        assert!(should_route_via_server(&cli, &default, true));
        // Trailing slash on a non-existent path: string fallback matches.
        let cli = cli_with_store(Some("/nonexistent/uteke-store/"));
        assert!(should_route_via_server(
            &cli,
            "/nonexistent/uteke-store",
            true
        ));
    }

    #[test]
    fn no_store_routes_only_when_server_enabled() {
        let cli = cli_with_store(None);
        assert!(should_route_via_server(&cli, "/any/default", true));
        assert!(!should_route_via_server(&cli, "/any/default", false));
    }
}
