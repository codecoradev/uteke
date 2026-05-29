//! Uteke CLI — persistent memory for AI agents.

mod config;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use config::Config;
use std::io::{self, Read};
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};
use uteke_core::Uteke;

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

#[derive(Subcommand)]
enum Commands {
    /// Store a new memory
    Remember {
        /// The content to remember
        content: String,
        /// Tags for categorization (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
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
        id: String,
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
    /// Manage tags: list, rename, delete
    Tags {
        #[command(subcommand)]
        command: TagCommands,
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

    // Determine store path: CLI > config > default
    let store_path = cli
        .store
        .as_deref()
        .map(|s| s.to_string())
        .unwrap_or_else(|| Config::expand_tilde(&config.store.path));

    tracing::debug!("Opening store at: {store_path}");

    let uteke = match Uteke::open(&store_path) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Error: Failed to open memory store: {e}");
            std::process::exit(1);
        }
    };

    let result = run_command(&cli, &uteke);
    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn run_command(cli: &Cli, uteke: &Uteke) -> Result<(), String> {
    let ns = cli.namespace.as_deref();

    match &cli.command {
        Commands::Remember { content, tags } => {
            tracing::info!("Remembering: {content}");
            let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
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
        Commands::Search { query, limit } => {
            tracing::info!("Searching: {query} (limit: {limit})");
            let results = uteke
                .search(query, *limit, ns)
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
        Commands::Forget { id } => {
            tracing::info!("Forgetting memory: {id}");
            uteke
                .forget(id)
                .map_err(|e| format!("Failed to delete memory: {e}"))?;
            if cli.json {
                let obj = serde_json::json!({"forgotten": id});
                println!("{}", obj);
            } else {
                println!("✓ Memory forgotten: {id}");
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
    }
}
