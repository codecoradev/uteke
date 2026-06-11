//! Uteke CLI — persistent memory for AI agents.

mod commands;
mod config;
mod init;
mod output;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use config::Config;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};
use uteke_core::Uteke;

/// Global flag set by SIGINT handler.
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

// ── CLI definition ──────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "uteke",
    about = "The Brain for Your AI — persistent memory engine",
    version
)]
struct Cli {
    /// Store path override (default: ~/.uteke)
    #[arg(long, global = true)]
    store: Option<String>,

    /// Namespace for multi-agent isolation (default: "default")
    #[arg(long, global = true)]
    namespace: Option<String>,

    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,

    /// Enable verbose logging
    #[arg(long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum SupportedShell {
    Bash,
    Zsh,
    Fish,
}

#[derive(Subcommand)]
enum Commands {
    /// Store a new memory
    Remember {
        /// The content to remember
        content: String,
        /// Tags for categorization (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        /// Memory type: fact, procedure, preference, decision, context
        #[arg(long, default_value = "fact")]
        r#type: String,
        /// Enable contradiction detection (auto-deprecate conflicting memories)
        #[arg(long)]
        detect_contradiction: bool,
        /// Entity identifier for structured metadata
        #[arg(long)]
        entity: Option<String>,
        /// Category classification
        #[arg(long)]
        category: Option<String>,
        /// Arbitrary key:value metadata pairs (comma-separated)
        #[arg(long, value_delimiter = ',')]
        meta: Vec<String>,
    },
    /// Recall memories relevant to a query (semantic search)
    Recall {
        /// The search query
        query: String,
        /// Maximum results to return
        #[arg(long, default_value = "5")]
        limit: usize,
        /// Filter by tags (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        /// Filter by entity name
        #[arg(long)]
        entity: Option<String>,
        /// Filter by category
        #[arg(long)]
        category: Option<String>,
        /// Minimum similarity score (0.0-1.0). Results below are filtered.
        #[arg(long)]
        min: Option<f32>,
        /// Use strict threshold from config (min_score_strict)
        #[arg(long)]
        strict: bool,
    },
    /// Search memories by content keywords (text search)
    Search {
        /// Keywords to search for
        query: String,
        /// Maximum results to return
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Filter by tags (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
    },
    /// List memories, optionally filtered by tag
    List {
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
        /// Maximum results to return
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Offset for pagination
        #[arg(long, default_value = "0")]
        offset: usize,
        /// Filter by entity name
        #[arg(long)]
        entity: Option<String>,
        /// Filter by category
        #[arg(long)]
        category: Option<String>,
    },
    /// Get a single memory by ID
    Get {
        /// Memory ID (UUID)
        id: String,
    },
    /// Delete a memory by ID
    Forget {
        /// Memory ID (UUID)
        id: Option<String>,
        /// Delete all memories with this tag
        #[arg(long)]
        tag: Option<String>,
        /// Delete all cold (not accessed in 30+ days) memories
        #[arg(long)]
        cold: bool,
        /// Delete ALL memories in namespace (requires --confirm)
        #[arg(long)]
        all: bool,
        /// Confirm destructive operations
        #[arg(long)]
        confirm: bool,
    },
    /// Show memory store statistics
    Stats,
    /// Check system health (DB, index, model, consistency)
    Doctor,
    /// Verify DB and index consistency
    Verify,
    /// Verify binary integrity against SHA256 checksums
    VerifyChecksums {
        /// Path to CHECKSUMS.txt file
        #[arg(long, default_value = "CHECKSUMS.txt")]
        checksums_file: String,
        /// Path to the binary to verify
        #[arg(long)]
        binary: String,
    },
    /// Repair index by rebuilding from SQLite
    Repair,
    /// Export all memories to JSONL file (no embeddings — portable)
    Export {
        /// Output file path (use - for stdout)
        #[arg(default_value = "-")]
        output: String,
    },
    /// Import memories from JSONL file (re-embeds content)
    Import {
        /// Input file path (use - for stdin)
        #[arg(default_value = "-")]
        input: String,
    },
    /// Generate shell completions
    Completions {
        /// Shell type
        shell: Shell,
    },
    /// Initialize uteke integration for an AI agent
    Init {
        /// Agent type: pi, claude, cursor, copilot, codex
        #[arg(long, default_value = "pi")]
        agent: String,
    },
    /// Memory aging: status, preview cleanup, cleanup
    Aging {
        #[command(subcommand)]
        command: AgingCommands,
    },
    /// Output shell hook script for auto-context loading
    Hook {
        /// Shell type: bash, zsh, fish
        shell: SupportedShell,
    },
    /// Namespace management: list, stats
    Namespace {
        #[command(subcommand)]
        command: NamespaceCommands,
    },
    /// Manage tags: list, rename, delete
    Tags {
        #[command(subcommand)]
        command: TagCommands,
    },
    /// Prune deprecated memories (auto-forget with TTL)
    Prune {
        /// TTL in days — deprecate memories older than this
        #[arg(long, default_value = "30")]
        ttl: u32,
        /// Dry run — show what would be pruned without deleting
        #[arg(long)]
        dry_run: bool,
    },
    /// Consolidate near-duplicate memories
    Consolidate {
        /// Similarity threshold (0.0-1.0) for detecting duplicates
        #[arg(long, default_value = "0.90")]
        threshold: f32,
        /// Dry run — show duplicates without merging
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum TagCommands {
    /// List all tags with usage counts
    List {
        /// Sort by count (descending) instead of alphabetical
        #[arg(long)]
        by_count: bool,
    },
    /// Rename a tag across all memories
    Rename {
        /// Current tag name
        old: String,
        /// New tag name
        new: String,
    },
    /// Delete a tag from all memories
    Delete {
        /// Tag name to delete
        tag: String,
        /// Skip confirmation prompt
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Subcommand)]
enum NamespaceCommands {
    /// List all namespaces with memory counts
    List,
    /// Show stats for a specific namespace
    Stats {
        /// Namespace name
        name: String,
    },
    /// Set default namespace in config
    Switch {
        /// Namespace name to set as default
        name: String,
    },
}

#[derive(Subcommand)]
enum AgingCommands {
    /// Show aging status: hot, warm, cold, never-accessed counts
    Status,
    /// Preview memories eligible for cleanup (dry-run)
    Preview {
        /// Minimum age in days for a memory to be considered aged
        #[arg(long, default_value = "180")]
        older_than_days: u32,
        /// Maximum access count threshold
        #[arg(long, default_value = "1")]
        max_access_count: u32,
    },
    /// Delete aged memories (use --yes to skip confirmation)
    Cleanup {
        /// Minimum age in days for a memory to be considered aged
        #[arg(long, default_value = "180")]
        older_than_days: u32,
        /// Maximum access count threshold
        #[arg(long, default_value = "1")]
        max_access_count: u32,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    // ── Logging: console (existing behavior) + file (always DEBUG, daily rotation) ──
    let log_dir = dirs::home_dir()
        .map(|h| h.join(".uteke"))
        .unwrap_or_default();
    let _guard = {
        let file_appender = tracing_appender::rolling::daily(&log_dir, "uteke.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        let file_layer = tracing_subscriber::fmt::layer()
            .with_writer(non_blocking)
            .with_filter(EnvFilter::from_default_env().add_directive(Level::DEBUG.into()));

        let console_level = if cli.verbose {
            Level::DEBUG
        } else {
            Level::WARN
        };
        let console_layer = tracing_subscriber::fmt::layer()
            .with_filter(EnvFilter::from_default_env().add_directive(console_level.into()));

        tracing_subscriber::registry()
            .with(console_layer)
            .with(file_layer)
            .init();

        guard
    };
    // _guard must stay alive — dropping it flushes and disables the non-blocking file writer.

    // Handle completions and init early — don't need store
    match &cli.command {
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            generate(*shell, &mut cmd, &name, &mut io::stdout());
            std::process::exit(0);
        }
        Commands::Init { .. } => {
            // Handled in run_command, but we skip store opening
            let result = init::run_init_command(&cli);
            if let Err(e) = result {
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

    // Check if uteke server is running — if so, route via HTTP for <50ms latency
    let server_url = format!("http://{}:{}", config.server.host, config.server.port);
    let server_available = config.server.enabled && commands::is_server_running(&server_url);

    if server_available {
        tracing::info!("Server detected at {server_url}, routing via HTTP");
        match commands::run_via_server(&cli, &server_url) {
            Ok(()) => return,
            Err(e) if e == "unsupported" => {
                tracing::info!("Command not supported via server, using local store");
                // Fall through to local store
            }
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    }

    // Fallback: open local store (cold start ~1s)
    tracing::debug!("No server detected, using local store");

    // Determine store path: CLI > config > default
    let store_path = cli
        .store
        .as_deref()
        .map(|s| s.to_string())
        .unwrap_or_else(|| Config::expand_tilde(&config.store.path));

    tracing::debug!("Opening store at: {store_path}");

    // Install SIGINT handler for graceful shutdown
    let uteke = match Uteke::open_with_tier(
        &store_path,
        uteke_core::TierConfig {
            hot_days: config.tier.hot_days as i64,
            warm_days: config.tier.warm_days as i64,
            hot_boost: config.tier.hot_boost,
        },
    ) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Error: Failed to open memory store: {e}");
            std::process::exit(1);
        }
    };

    ctrlc::set_handler(|| {
        SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
        eprintln!("\nInterrupt received, shutting down gracefully...");
    })
    .expect("Failed to set SIGINT handler");

    let result = commands::run_command(&cli, &uteke, &config);

    // Graceful shutdown: save dirty index if needed
    if let Err(e) = uteke.shutdown() {
        tracing::warn!("Shutdown flush failed: {e}");
    }

    if SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
        eprintln!("Shutdown complete.");
        std::process::exit(130); // 128 + SIGINT(2)
    }

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

/// Resolve namespace: CLI flag > UTEKE_NAMESPACE env > config > "default"
fn resolve_namespace(cli: &Cli, config: &Config) -> String {
    // 1. CLI --namespace flag wins
    if let Some(ns) = &cli.namespace {
        return ns.clone();
    }
    // 2. Environment variable
    if let Ok(env_ns) = std::env::var("UTEKE_NAMESPACE") {
        if !env_ns.is_empty() {
            return env_ns;
        }
    }
    // 3. Config file
    if config.store.namespace != "default" {
        return config.store.namespace.clone();
    }
    // 4. Fallback
    "default".to_string()
}
