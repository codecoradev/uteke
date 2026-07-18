//! Uteke CLI — persistent memory for AI agents.

mod cli;
mod commands;
mod config;
mod extract;
mod init;
mod logging;
mod output;

use clap::{CommandFactory, Parser};
use cli::{Cli, Commands};
use config::Config;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use uteke_core::Uteke;

/// Global flag set by SIGINT handler.
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

fn main() {
    let cli = Cli::parse();

    // Initialize logging (console + file). Guard must stay alive.
    let _log_guard = logging::init(cli.verbose);

    // Handle completions and init early — don't need store
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
        Commands::Bench { counts, json } => {
            // Bench creates its own temp stores — skip opening the user store.
            if let Err(e) = commands::bench::run_bench(*json, counts.clone()) {
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

    // Check if uteke server is running — if so, route via HTTP for <50ms latency
    let server_url = format!("http://{}:{}", config.server.host, config.server.port);
    let server_available = config.server.enabled && commands::is_server_running(&server_url);

    if server_available {
        tracing::info!("Server detected at {server_url}, routing via HTTP");
        match commands::run_via_server(&cli, &server_url) {
            Ok(()) => return,
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

    let store_path = cli
        .store
        .as_deref()
        .map(|s| s.to_string())
        .unwrap_or_else(|| Config::expand_tilde(&config.store.path));

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
