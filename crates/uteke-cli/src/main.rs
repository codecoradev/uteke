//! Uteke CLI — persistent memory for AI agents.

mod config;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use config::Config;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};
use uteke_core::Uteke;

/// Global flag set by SIGINT handler.
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

// ── Config is in config.rs ─────────────────────────────────────────────────

// ── JSON output helpers ─────────────────────────────────────────────────────

fn print_json<T: serde::Serialize>(value: &T) {
    println!("{}", serde_json::to_string(value).unwrap());
}

// ── Human-readable output helpers ───────────────────────────────────────────

fn print_tags_human(tags: &[uteke_core::TagInfo], _by_count: bool) {
    if tags.is_empty() {
        println!("No tags found.");
        return;
    }
    println!("Tags ({} total):\n", tags.len());
    for t in tags {
        println!("  {} ({})", t.name, t.count);
    }
}

fn print_remember_human(id: &str) {
    println!("✓ Memory stored");
    println!("  ID: {id}");
}

fn print_recall_human(results: &[uteke_core::SearchResult]) {
    if results.is_empty() {
        println!("No matching memories found.");
        return;
    }
    println!("Found {} result(s):\n", results.len());
    for (i, r) in results.iter().enumerate() {
        let tags = if r.memory.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", r.memory.tags.join(", "))
        };
        println!(
            "  {}. (score: {:.3}) {}{}",
            i + 1,
            r.score,
            r.memory.content,
            tags
        );
        println!("     ID: {}", r.memory.id);
        println!("     Created: {}", r.memory.created_at.to_rfc3339());
    }
}

fn print_search_human(results: &[uteke_core::SearchResult]) {
    if results.is_empty() {
        println!("No matching memories found.");
        return;
    }
    println!("Found {} result(s):\n", results.len());
    for (i, r) in results.iter().enumerate() {
        let tags = if r.memory.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", r.memory.tags.join(", "))
        };
        println!("  {}. {}{}", i + 1, r.memory.content, tags);
        println!("     ID: {}", r.memory.id);
    }
}

fn print_list_human(memories: &[uteke_core::Memory]) {
    if memories.is_empty() {
        println!("No memories found.");
        return;
    }
    for m in memories {
        let tags = if m.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", m.tags.join(", "))
        };
        println!("  {}{}", m.content, tags);
        println!("    ID: {}", m.id);
        println!("    Created: {}", m.created_at.to_rfc3339());
    }
}

fn print_get_human(memory: &uteke_core::Memory) {
    println!("ID: {}", memory.id);
    println!("Content: {}", memory.content);
    if !memory.tags.is_empty() {
        println!("Tags: {}", memory.tags.join(", "));
    }
    if !memory.metadata.is_null() {
        println!(
            "Metadata: {}",
            serde_json::to_string_pretty(&memory.metadata).unwrap()
        );
    }
    println!("Created: {}", memory.created_at.to_rfc3339());
    println!("Updated: {}", memory.updated_at.to_rfc3339());
}

fn print_stats_human(stats: &uteke_core::StoreStats) {
    println!("Memory Store Statistics");
    println!("──────────────────────");
    println!("  Total memories: {}", stats.total_memories);
    println!("  🔥 Hot (7d):    {}", stats.hot);
    println!("  🟡 Warm (30d):  {}", stats.warm);
    println!("  ❄️  Cold (>30d):  {}", stats.cold);
    println!("  Unique tags:    {}", stats.unique_tags);
    let size_str = if stats.db_size_bytes < 1024 {
        format!("{} B", stats.db_size_bytes)
    } else if stats.db_size_bytes < 1024 * 1024 {
        format!("{:.1} KB", stats.db_size_bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", stats.db_size_bytes as f64 / (1024.0 * 1024.0))
    };
    println!("  Database size:  {}", size_str);
}

fn print_doctor_human(report: &uteke_core::DoctorReport) {
    println!("Uteke Health Check");
    println!("───────────────────");
    let all_ok = report
        .checks
        .iter()
        .all(|c| matches!(c.status, uteke_core::DoctorStatus::Ok));
    for check in &report.checks {
        let icon = match check.status {
            uteke_core::DoctorStatus::Ok => "✓",
            uteke_core::DoctorStatus::Warn => "⚠",
            uteke_core::DoctorStatus::Error => "✗",
        };
        println!("  {} {}: {}", icon, check.name, check.detail);
    }
    if all_ok {
        println!("\n  All checks passed.");
    } else {
        println!("\n  Some checks failed. Run `uteke repair` if index is inconsistent.");
    }
}

fn print_verify_human(report: &uteke_core::VerifyReport) {
    println!("Verify Report");
    println!("─────────────");
    println!("  SQLite DB:    {} memories", report.db_count);
    println!("  usearch index: {} vectors", report.index_count);
    if report.consistent {
        println!("  ✓ Consistent");
    } else {
        println!("  ✗ MISMATCH — run `uteke repair` to rebuild index");
    }
}

fn print_repair_human(report: &uteke_core::RepairReport) {
    println!("Repair Report");
    println!("─────────────");
    println!("  SQLite DB:     {} memories", report.db_count);
    println!("  Index before:  {} vectors", report.index_before);
    println!("  Index after:   {} vectors", report.index_after);
    if report.index_after == report.db_count {
        println!("  ✓ Index rebuilt successfully");
    } else {
        println!("  ⚠ Index count still differs from DB");
    }
}

fn print_aging_status_human(status: &uteke_core::AgingStatus) {
    println!("Memory Aging Status");
    println!("────────────────────");
    println!("  Total:          {}", status.total);
    println!("  🔥 Hot (7d):    {}", status.hot);
    println!("  🟡 Warm (30d):  {}", status.warm);
    println!("  ❄️  Cold (>30d):  {}", status.cold);
    println!("  🚫 Never accessed: {}", status.never_accessed);
}

fn print_aging_preview_human(memories: &[uteke_core::Memory]) {
    if memories.is_empty() {
        println!("No aged memories eligible for cleanup.");
        return;
    }
    println!("Aged Memories ({} eligible for cleanup):\n", memories.len());
    for (i, m) in memories.iter().enumerate() {
        let accessed = m
            .last_accessed
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "never".to_string());
        println!(
            "  {}. {}",
            i + 1,
            m.content.chars().take(80).collect::<String>()
        );
        println!("     ID: {}", m.id);
        println!("     Created: {}", m.created_at.to_rfc3339());
        println!("     Accessed: {} (count: {})", accessed, m.access_count);
    }
}

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

// ── Agent Init ──────────────────────────────────────────────────────────────

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

fn run_init_command(cli: &Cli) -> Result<(), String> {
    if let Commands::Init { agent } = &cli.command {
        return run_init(agent, cli.json);
    }
    Ok(())
}

fn run_init(agent: &str, json: bool) -> Result<(), String> {
    match agent {
        "pi" => init_pi(json),
        "claude" => init_claude(json),
        "cursor" => init_cursor(json),
        _ => Err(format!(
            "Unknown agent: {agent}. Supported: pi, claude, cursor"
        )),
    }
}

fn init_pi(json: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;
    let skill_dir = cwd.join(".agents").join("skills").join("uteke-memory");
    std::fs::create_dir_all(&skill_dir).map_err(|e| format!("Failed to create skill dir: {e}"))?;

    let skill_content = r#"# Uteke Memory Skill

Provides persistent memory for AI agents via the `uteke` CLI.

## Commands

- `uteke remember "<text>" --tags <tags>` — Store a memory
- `uteke recall "<query>" --limit <n>` — Semantic search
- `uteke search "<keywords>"` — Keyword search
- `uteke list --tag <tag>` — List memories
- `uteke get <id>` — Get a memory by ID
- `uteke forget <id>` — Delete a memory
- `uteke stats` — Show statistics
- `uteke export [file]` — Export memories to JSONL
- `uteke import [file]` — Import memories from JSONL

## Usage Patterns

### Store important context
```bash
uteke remember "Database uses WAL mode for concurrent reads" --tags architecture,db
```

### Recall relevant context
```bash
uteke recall "how does the database work?"
```

### Project-specific store
```bash
uteke --store .uteke remember "Uses React Server Components" --tags frontend
```

## When to Use
- Before starting work: `uteke recall "<project context>"`
- After making decisions: `uteke remember "<decision>" --tags <tags>`
- Before closing session: `uteke remember "<session state>" --tags session`
"#;

    std::fs::write(skill_dir.join("SKILL.md"), skill_content)
        .map_err(|e| format!("Failed to write skill: {e}"))?;

    if json {
        let obj = serde_json::json!({
            "agent": "pi",
            "skill": skill_dir.to_string_lossy(),
            "status": "installed"
        });
        println!("{obj}");
    } else {
        println!("✓ Pi skill installed: {}", skill_dir.display());
        println!("  Restart your agent to activate.");
    }
    Ok(())
}

fn init_claude(json: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;
    let md_path = cwd.join("UTEKE.md");

    let md_content = r#"# Uteke Memory Integration

## Commands
- `uteke remember "<text>" --tags <tags>` — Store a memory
- `uteke recall "<query>" --limit <n>` — Semantic search
- `uteke search "<keywords>"` — Keyword search
- `uteke list --tag <tag>` — List memories
- `uteke get <id>` — Get by ID
- `uteke forget <id>` — Delete
- `uteke stats` — Statistics
- `uteke export [file]` — Export to JSONL
- `uteke import [file]` — Import from JSONL

## Usage Guidelines
1. Before starting work: recall relevant context
2. After making decisions: store them with tags
3. Before closing session: store session state
4. Use project-specific stores with `--store .uteke`

## Example
```bash
uteke recall "how does auth work?"
uteke remember "Auth uses JWT with 24h expiry" --tags auth,security
```
"#;

    std::fs::write(&md_path, md_content).map_err(|e| format!("Failed to write UTEKE.md: {e}"))?;

    // Try to add reference to CLAUDE.md
    let claude_md = cwd.join("CLAUDE.md");
    if claude_md.exists() {
        let existing = std::fs::read_to_string(&claude_md)
            .map_err(|e| format!("Failed to read CLAUDE.md: {e}"))?;
        if !existing.contains("UTEKE.md") {
            let updated = format!("{existing}\n\n## Uteke Memory\n\nSee [UTEKE.md](UTEKE.md) for uteke memory commands.\n");
            std::fs::write(&claude_md, updated)
                .map_err(|e| format!("Failed to update CLAUDE.md: {e}"))?;
        }
    }

    if json {
        let obj = serde_json::json!({
            "agent": "claude",
            "file": md_path.to_string_lossy(),
            "status": "installed"
        });
        println!("{obj}");
    } else {
        println!("✓ Claude integration installed: {}", md_path.display());
        if claude_md.exists() {
            println!("  Reference added to CLAUDE.md");
        } else {
            println!("  Tip: Create CLAUDE.md and add a reference to UTEKE.md");
        }
    }
    Ok(())
}

fn init_cursor(json: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;
    let rules_dir = cwd.join(".cursor").join("rules");
    std::fs::create_dir_all(&rules_dir).map_err(|e| format!("Failed to create rules dir: {e}"))?;

    let rules_content = r#"# Uteke Memory Integration

## Commands
- `uteke remember "<text>" --tags <tags>` — Store a memory
- `uteke recall "<query>" --limit <n>` — Semantic search
- `uteke search "<keywords>"` — Keyword search
- `uteke list --tag <tag>` — List memories
- `uteke get <id>` — Get by ID
- `uteke forget <id>` — Delete
- `uteke stats` — Statistics

## Guidelines
1. Before starting work: recall relevant context
2. After making decisions: store them with tags
3. Before closing session: store session state
4. Use project-specific stores with `--store .uteke`
"#;

    let rules_path = rules_dir.join("uteke.mdc");
    std::fs::write(&rules_path, rules_content)
        .map_err(|e| format!("Failed to write rules: {e}"))?;

    if json {
        let obj = serde_json::json!({
            "agent": "cursor",
            "file": rules_path.to_string_lossy(),
            "status": "installed"
        });
        println!("{obj}");
    } else {
        println!("✓ Cursor rules installed: {}", rules_path.display());
    }
    Ok(())
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
            let result = run_init_command(&cli);
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
    let server_available = config.server.enabled && is_server_running(&server_url);

    if server_available {
        tracing::info!("Server detected at {server_url}, routing via HTTP");
        let result = run_via_server(&cli, &server_url);
        if let Err(e) = result {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
        return;
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
    let uteke = match Uteke::open(&store_path) {
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

    let result = run_command(&cli, &uteke, &config);

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
/// Check if uteke server is reachable.
fn is_server_running(url: &str) -> bool {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_millis(100))
        .build()
        .map(|c| c.get(format!("{url}/health")).send().is_ok())
        .unwrap_or(false)
}

/// Route CLI commands through the HTTP server for <50ms latency.
fn run_via_server(cli: &Cli, server_url: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::new();
    let ns = cli.namespace.as_deref().unwrap_or("default");

    match &cli.command {
        Commands::Remember {
            content,
            tags,
            r#type,
            detect_contradiction,
        } => {
            let mut body = serde_json::json!({
                "content": content,
                "tags": tags,
                "namespace": ns
            });
            if !r#type.is_empty() {
                body["type"] = serde_json::json!(r#type);
            }
            if *detect_contradiction {
                body["detect_contradiction"] = serde_json::json!(true);
            }
            let resp = client
                .post(format!("{server_url}/remember"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let data: serde_json::Value = resp.json().map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                println!("{data}");
            } else {
                println!("\u{2713} Memory stored\n  ID: {}", data["id"]);
            }
        }
        Commands::Recall {
            query, limit, tags, ..
        } => {
            let body = serde_json::json!({
                "query": query,
                "limit": limit,
                "tags": tags,
                "namespace": ns
            });
            let resp = client
                .post(format!("{server_url}/recall"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let results: Vec<uteke_core::SearchResult> =
                resp.json().map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                print_json(&results);
            } else {
                print_recall_human(&results);
            }
        }
        Commands::Search {
            query, limit, tags, ..
        } => {
            let body = serde_json::json!({
                "query": query,
                "limit": limit,
                "tags": tags,
                "namespace": ns
            });
            let resp = client
                .post(format!("{server_url}/search"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let results: Vec<uteke_core::SearchResult> =
                resp.json().map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                print_json(&results);
            } else {
                print_search_human(&results);
            }
        }
        Commands::List {
            tag, limit, offset, ..
        } => {
            let body = serde_json::json!({
                "tag": tag,
                "limit": limit,
                "offset": offset,
                "namespace": ns
            });
            let resp = client
                .post(format!("{server_url}/list"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let memories: Vec<uteke_core::Memory> =
                resp.json().map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                print_json(&memories);
            } else {
                print_list_human(&memories);
            }
        }
        Commands::Stats => {
            let body = serde_json::json!({ "namespace": ns });
            let resp = client
                .post(format!("{server_url}/stats"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let stats: uteke_core::StoreStats =
                resp.json().map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                print_json(&stats);
            } else {
                print_stats_human(&stats);
            }
        }
        Commands::Forget {
            id,
            tag,
            cold: _,
            all: _,
            confirm: _,
        } => {
            if let Some(id) = id {
                let resp = client
                    .delete(format!(
                        "{server_url}/forget?id={}",
                        urlencoding::encode(id)
                    ))
                    .send()
                    .map_err(|e| format!("Server error: {e}"))?;
                let data: serde_json::Value =
                    resp.json().map_err(|e| format!("Parse error: {e}"))?;
                if cli.json {
                    println!("{data}");
                } else {
                    println!("\u{2713} Memory forgotten: {id}");
                }
            } else if let Some(tag) = tag {
                let resp = client
                    .delete(format!(
                        "{server_url}/forget?tag={}&namespace={}",
                        urlencoding::encode(tag),
                        urlencoding::encode(ns)
                    ))
                    .send()
                    .map_err(|e| format!("Server error: {e}"))?;
                let data: serde_json::Value =
                    resp.json().map_err(|e| format!("Parse error: {e}"))?;
                if cli.json {
                    println!("{data}");
                } else {
                    println!(
                        "\u{2713} Deleted {} memories with tag '{}'",
                        data["deleted"], tag
                    );
                }
            } else {
                return Err("Provide an ID, --tag, --cold, or --all".into());
            }
        }
        // Commands not supported via server fall through to local
        _ => {
            return Err("This command requires local store. Disable server mode to use it.".into());
        }
    }
    Ok(())
}

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

fn run_command(cli: &Cli, uteke: &Uteke, config: &Config) -> Result<(), String> {
    // Resolve effective namespace once: CLI > env > config > "default"
    let resolved_ns = resolve_namespace(cli, config);
    let ns: Option<&str> = Some(resolved_ns.as_str());

    match &cli.command {
        Commands::Remember {
            content,
            tags,
            r#type,
            detect_contradiction,
        } => {
            tracing::info!(
                "Remembering: {content} (type: {type}, contradiction: {detect_contradiction})"
            );
            let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();

            if *detect_contradiction {
                let (id, contradiction) = uteke
                    .remember_with_contradiction(
                        content,
                        &tag_refs,
                        ns,
                        Some(r#type.as_str()),
                        true,
                    )
                    .map_err(|e| format!("Failed to store memory: {e}"))?;
                tracing::info!("Memory stored with ID: {id}");
                if cli.json {
                    let obj = serde_json::json!({
                        "id": id,
                        "contradiction": {
                            "detected": contradiction.contradicted,
                            "deprecated_id": contradiction.deprecated_id,
                            "similarity": contradiction.similarity
                        }
                    });
                    println!("{}", obj);
                } else {
                    print_remember_human(&id);
                    if contradiction.contradicted {
                        if let Some(dep_id) = &contradiction.deprecated_id {
                            println!(
                                "  \u{26a0} Contradiction detected (sim: {:.3}): deprecated {}",
                                contradiction.similarity,
                                &dep_id[..8]
                            );
                        }
                    }
                }
            } else {
                let id = uteke
                    .remember(content, &tag_refs, None, ns)
                    .map_err(|e| format!("Failed to store memory: {e}"))?;
                tracing::info!("Memory stored with ID: {id}");
                if cli.json {
                    let obj = serde_json::json!({"id": id});
                    println!("{}", obj);
                } else {
                    print_remember_human(&id);
                }
            }
            Ok(())
        }
        Commands::Recall { query, limit, tags } => {
            tracing::info!("Recalling: {query} (limit: {limit})");
            let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
            let tags_filter = if tag_refs.is_empty() {
                None
            } else {
                Some(tag_refs.as_slice())
            };
            let results = uteke
                .recall(query, *limit, tags_filter, ns)
                .map_err(|e| format!("Failed to recall: {e}"))?;
            if cli.json {
                print_json(&results);
            } else {
                print_recall_human(&results);
            }
            Ok(())
        }
        Commands::Search { query, limit, tags } => {
            tracing::info!("Searching: {query} (limit: {limit})");
            let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
            let tags_filter = if tag_refs.is_empty() {
                None
            } else {
                Some(tag_refs.as_slice())
            };
            let results = uteke
                .search(query, *limit, tags_filter, ns)
                .map_err(|e| format!("Failed to search: {e}"))?;
            if cli.json {
                print_json(&results);
            } else {
                print_search_human(&results);
            }
            Ok(())
        }
        Commands::List { tag, limit, offset } => {
            tracing::info!(
                "Listing memories (tag: {:?}, limit: {limit}, offset: {offset})",
                tag
            );
            let results = uteke
                .list(tag.as_deref(), *limit, *offset, ns)
                .map_err(|e| format!("Failed to list: {e}"))?;
            if cli.json {
                print_json(&results);
            } else {
                print_list_human(&results);
            }
            Ok(())
        }
        Commands::Get { id } => {
            tracing::info!("Getting memory: {id}");
            let memory = uteke
                .get(id)
                .map_err(|e| format!("Failed to get memory: {e}"))?;
            if cli.json {
                print_json(&memory);
            } else {
                print_get_human(&memory);
            }
            Ok(())
        }
        Commands::Forget {
            id,
            tag,
            cold,
            all,
            confirm,
        } => {
            if let Some(id) = id {
                // Single delete — confirm if no --confirm flag
                if !confirm {
                    println!("About to delete memory: {id}");
                    print!("Are you sure? [y/N] ");
                    io::Write::flush(&mut io::stdout()).ok();
                    let mut input = String::new();
                    io::stdin().read_line(&mut input).ok();
                    if !input.trim().eq_ignore_ascii_case("y") {
                        println!("Cancelled.");
                        return Ok(());
                    }
                }
                tracing::info!("Forgetting memory: {id}");
                uteke
                    .forget(id.as_str())
                    .map_err(|e| format!("Failed to delete memory: {e}"))?;
                if cli.json {
                    print_json(&serde_json::json!({"forgotten": id}));
                } else {
                    println!("✓ Memory forgotten: {id}");
                }
            } else if let Some(tag) = tag {
                // Bulk delete by tag
                tracing::info!("Bulk forgetting by tag: {tag}");
                let count = uteke.store().count_by_tag(tag.as_str(), ns).unwrap_or(0);
                if !confirm && count > 0 {
                    println!("Found {count} memories with tag '{tag}'. Use --confirm to delete.");
                    return Ok(());
                }
                let result = uteke
                    .bulk_forget_by_tag(tag.as_str(), ns)
                    .map_err(|e| format!("Failed: {e}"))?;
                if cli.json {
                    print_json(&serde_json::json!({"deleted": result.deleted, "ids": result.ids}));
                } else {
                    println!("✓ Deleted {} memories with tag '{}'", result.deleted, tag);
                }
            } else if *cold {
                // Bulk delete cold memories
                tracing::info!("Bulk forgetting cold memories");
                if !confirm {
                    println!("This will delete all cold memories (>30 days or never accessed).");
                    println!("Use --confirm to proceed.");
                    return Ok(());
                }
                let result = uteke
                    .bulk_forget_cold(ns)
                    .map_err(|e| format!("Failed: {e}"))?;
                if cli.json {
                    print_json(&serde_json::json!({"deleted": result.deleted, "ids": result.ids}));
                } else {
                    println!("✓ Deleted {} cold memories", result.deleted);
                }
            } else if *all {
                // Delete all in namespace
                tracing::info!("Bulk forgetting all memories");
                if !confirm {
                    println!("⚠️  This will delete ALL memories in the namespace.");
                    println!("Use --confirm to proceed.");
                    return Ok(());
                }
                let result = uteke
                    .bulk_forget_all(ns)
                    .map_err(|e| format!("Failed: {e}"))?;
                if cli.json {
                    print_json(&serde_json::json!({"deleted": result.deleted, "ids": result.ids}));
                } else {
                    println!("✓ Deleted {} memories", result.deleted);
                }
            } else {
                return Err("Provide an ID, --tag, --cold, or --all".into());
            }
            Ok(())
        }
        Commands::Stats => {
            tracing::info!("Getting stats");
            let stats = uteke
                .stats(ns)
                .map_err(|e| format!("Failed to get stats: {e}"))?;
            if cli.json {
                print_json(&stats);
            } else {
                print_stats_human(&stats);
            }
            Ok(())
        }
        Commands::Doctor => {
            tracing::info!("Running doctor");
            let report = uteke.doctor().map_err(|e| format!("Doctor failed: {e}"))?;
            if cli.json {
                print_json(&report);
            } else {
                print_doctor_human(&report);
            }
            Ok(())
        }
        Commands::Verify => {
            tracing::info!("Running verify");
            let report = uteke.verify().map_err(|e| format!("Verify failed: {e}"))?;
            if cli.json {
                print_json(&report);
            } else {
                print_verify_human(&report);
            }
            Ok(())
        }
        Commands::Repair => {
            tracing::info!("Running repair");
            let report = uteke.repair().map_err(|e| format!("Repair failed: {e}"))?;
            if cli.json {
                print_json(&report);
            } else {
                print_repair_human(&report);
            }
            Ok(())
        }
        Commands::Tags { command } => {
            match command {
                TagCommands::List { by_count } => {
                    tracing::info!("Listing tags (by_count: {by_count})");
                    let mut tags = uteke
                        .tags_with_counts(ns)
                        .map_err(|e| format!("Failed to list tags: {e}"))?;
                    if *by_count {
                        tags.sort_by_key(|b| std::cmp::Reverse(b.count));
                    } else {
                        tags.sort_by(|a, b| a.name.cmp(&b.name));
                    }
                    if cli.json {
                        print_json(&tags);
                    } else {
                        print_tags_human(&tags, *by_count);
                    }
                }
                TagCommands::Rename { old, new } => {
                    tracing::info!("Renaming tag: {old} -> {new}");
                    let count = uteke
                        .rename_tag(old, new, ns)
                        .map_err(|e| format!("Failed to rename tag: {e}"))?;
                    if cli.json {
                        print_json(
                            &serde_json::json!({"renamed": count, "tag": old, "new_tag": new}),
                        );
                    } else {
                        println!("✓ Tag '{old}' renamed to '{new}' ({count} memories updated)");
                    }
                }
                TagCommands::Delete { tag, confirm } => {
                    if !confirm {
                        // Check if tag exists and show count
                        let tags = uteke
                            .tags_with_counts(ns)
                            .map_err(|e| format!("Failed to list tags: {e}"))?;
                        let info = tags.iter().find(|t| t.name == *tag);
                        match info {
                            Some(info) => {
                                println!(
                                    "Tag '{}' is used by {} memory(ies).",
                                    info.name, info.count
                                );
                                println!("Use --confirm to proceed with deletion.");
                                return Err(
                                    "Tag deletion not confirmed. Use --confirm flag.".to_string()
                                );
                            }
                            None => {
                                return Err(format!("Tag '{}' not found.", tag));
                            }
                        }
                    }
                    tracing::info!("Deleting tag: {tag}");
                    let count = uteke
                        .delete_tag(tag, ns)
                        .map_err(|e| format!("Failed to delete tag: {e}"))?;
                    if cli.json {
                        print_json(&serde_json::json!({"deleted": count, "tag": tag}));
                    } else {
                        println!("✓ Tag '{tag}' deleted ({count} memories updated)");
                    }
                }
            }
            Ok(())
        }
        Commands::Prune { ttl, dry_run } => {
            tracing::info!("Pruning with TTL={ttl}d (dry_run={dry_run})");
            let result = uteke
                .prune(*ttl, ns, *dry_run)
                .map_err(|e| format!("Failed to prune: {e}"))?;
            if cli.json {
                print_json(&result);
            } else {
                if result.deprecated_ids.is_empty() && result.pruned == 0 {
                    println!("No deprecated memories to prune.");
                } else if *dry_run {
                    println!(
                        "Dry run — {} deprecated memories would be pruned (TTL: {ttl}d):",
                        result.deprecated
                    );
                    for id in &result.deprecated_ids {
                        println!("  {}", id);
                    }
                } else {
                    println!(
                        "\u{2713} Pruned {} deprecated memories (TTL: {ttl}d)",
                        result.pruned
                    );
                }
            }
            Ok(())
        }
        Commands::Consolidate { threshold, dry_run } => {
            tracing::info!("Consolidating (threshold: {threshold}, dry_run: {dry_run})");
            if *dry_run {
                let pairs = uteke
                    .find_duplicates(ns, *threshold)
                    .map_err(|e| format!("Failed to find duplicates: {e}"))?;
                if cli.json {
                    print_json(&pairs);
                } else if pairs.is_empty() {
                    println!("No duplicate pairs found (threshold: {threshold}).");
                } else {
                    println!("Found {} potential duplicate(s):\n", pairs.len());
                    for (i, p) in pairs.iter().enumerate() {
                        println!("  {}. sim={:.3}", i + 1, p.similarity);
                        println!("     A: {}", p.content_a);
                        println!("     B: {}", p.content_b);
                    }
                }
            } else {
                let result = uteke
                    .consolidate(ns, *threshold, false)
                    .map_err(|e| format!("Failed to consolidate: {e}"))?;
                if cli.json {
                    print_json(&result);
                } else {
                    println!("\u{2713} Consolidation complete:");
                    println!("  Duplicates found: {}", result.duplicates_found);
                    println!("  Merged: {}", result.merged);
                    if !result.removed_ids.is_empty() {
                        println!("  Removed:");
                        for id in &result.removed_ids {
                            println!("    {}", id);
                        }
                    }
                }
            }
            Ok(())
        }
        Commands::Export { output } => {
            tracing::info!("Exporting memories to {output}");
            let jsonl = uteke
                .export(ns)
                .map_err(|e| format!("Failed to export: {e}"))?;

            if output == "-" {
                println!("{jsonl}");
            } else {
                std::fs::write(output, &jsonl)
                    .map_err(|e| format!("Failed to write export file: {e}"))?;
                let count = jsonl.lines().filter(|l| !l.trim().is_empty()).count();
                if cli.json {
                    print_json(&serde_json::json!({"exported": count}));
                } else {
                    println!("✓ Exported {count} memories");
                }
            }
            Ok(())
        }
        Commands::Import { input } => {
            tracing::info!("Importing memories from {input}");
            let jsonl = if input == "-" {
                let mut buf = String::new();
                io::stdin()
                    .read_to_string(&mut buf)
                    .map_err(|e| format!("Failed to read stdin: {e}"))?;
                buf
            } else {
                std::fs::read_to_string(input)
                    .map_err(|e| format!("Failed to read import file: {e}"))?
            };

            let result = uteke
                .import(&jsonl, ns)
                .map_err(|e| format!("Failed to import: {e}"))?;

            if cli.json {
                print_json(&result);
            } else {
                println!(
                    "✓ Imported {} memories ({} skipped)",
                    result.imported, result.skipped
                );
            }
            Ok(())
        }
        Commands::Completions { .. } => {
            // Already handled in main()
            Ok(())
        }
        Commands::Init { agent } => run_init(agent, cli.json),
        Commands::Namespace { command } => match command {
            NamespaceCommands::List => {
                tracing::info!("Listing namespaces");
                let namespaces = uteke
                    .list_namespaces()
                    .map_err(|e| format!("Failed to list namespaces: {e}"))?;
                if cli.json {
                    print_json(&namespaces);
                } else {
                    if namespaces.is_empty() {
                        println!("No namespaces found.");
                    } else {
                        println!("Namespaces ({} total):\n", namespaces.len());
                        for ns_name in &namespaces {
                            let count = uteke.store().count(Some(ns_name.as_str())).unwrap_or(0);
                            println!("  {} ({} memories)", ns_name, count);
                        }
                    }
                }
                Ok(())
            }
            NamespaceCommands::Stats { name } => {
                tracing::info!("Namespace stats: {name}");
                let stats = uteke
                    .stats(Some(name.as_str()))
                    .map_err(|e| format!("Failed to get namespace stats: {e}"))?;
                if cli.json {
                    print_json(&stats);
                } else {
                    println!("Namespace: {name}");
                    print_stats_human(&stats);
                }
                Ok(())
            }
            NamespaceCommands::Switch { name } => {
                tracing::info!("Switching default namespace to: {name}");
                Config::set_default_namespace(name)
                    .map_err(|e| format!("Failed to switch namespace: {e}"))?;
                if cli.json {
                    print_json(&serde_json::json!({"default_namespace": name}));
                } else {
                    println!("✓ Default namespace set to '{name}'");
                }
                Ok(())
            }
        },
        Commands::Aging { command } => match command {
            AgingCommands::Status => {
                tracing::info!("Aging status");
                let status = uteke
                    .aging_status(ns)
                    .map_err(|e| format!("Failed to get aging status: {e}"))?;
                if cli.json {
                    print_json(&status);
                } else {
                    print_aging_status_human(&status);
                }
                Ok(())
            }
            AgingCommands::Preview {
                older_than_days,
                max_access_count,
            } => {
                tracing::info!(
                    "Aging preview (older_than: {}d, max_access: {})",
                    older_than_days,
                    max_access_count
                );
                let memories = uteke
                    .aging_preview(*older_than_days, *max_access_count, ns)
                    .map_err(|e| format!("Failed to preview aged memories: {e}"))?;
                if cli.json {
                    print_json(&memories);
                } else {
                    print_aging_preview_human(&memories);
                }
                Ok(())
            }
            AgingCommands::Cleanup {
                older_than_days,
                max_access_count,
                yes,
            } => {
                // Preview first
                let preview = uteke
                    .aging_preview(*older_than_days, *max_access_count, ns)
                    .map_err(|e| format!("Failed to preview aged memories: {e}"))?;

                if preview.is_empty() {
                    if cli.json {
                        print_json(&uteke_core::CleanupResult { deleted: 0 });
                    } else {
                        println!("No aged memories to clean up.");
                    }
                    return Ok(());
                }

                if !yes {
                    if !cli.json {
                        print_aging_preview_human(&preview);
                        println!();
                        println!(
                            "⚠ About to delete {} memory(ies). Use --yes to confirm.",
                            preview.len()
                        );
                    }
                    return Err("Cleanup not confirmed. Use --yes flag to proceed.".to_string());
                }

                let count = preview.len();
                let result = uteke
                    .aging_cleanup(*older_than_days, *max_access_count, ns)
                    .map_err(|e| format!("Failed to cleanup aged memories: {e}"))?;
                if cli.json {
                    print_json(&result);
                } else {
                    println!("✓ Cleaned up {} aged memory(ies)", result.deleted);
                }
                let _ = count;
                Ok(())
            }
        },
        Commands::Hook { shell } => {
            let script = match shell {
                SupportedShell::Bash => include_str!("../../../scripts/shell/uteke-hook.bash"),
                SupportedShell::Zsh => include_str!("../../../scripts/shell/uteke-hook.zsh"),
                SupportedShell::Fish => include_str!("../../../scripts/shell/uteke-hook.fish"),
            };
            print!("{script}");
            Ok(())
        }
    }
}
// test
