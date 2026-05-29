//! Uteke CLI — persistent memory for AI agents.

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use std::io;
use std::path::PathBuf;
use tracing::Level;
use tracing_subscriber::EnvFilter;
use uteke_core::Uteke;

// ── Config ──────────────────────────────────────────────────────────────────

/// Runtime config loaded from ~/.uteke/config.toml (or defaults).
#[derive(serde::Deserialize, Default)]
struct Config {
    store_path: Option<String>,
}

impl Config {
    fn load() -> Self {
        let config_path = dirs::home_dir()
            .map(|h| h.join(".uteke").join("config.toml"))
            .unwrap_or_default();

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path).unwrap_or_default();
            toml::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    fn ensure_dirs() -> PathBuf {
        let base = dirs::home_dir()
            .expect("Cannot determine home directory")
            .join(".uteke");
        std::fs::create_dir_all(&base).ok();
        std::fs::create_dir_all(base.join("models")).ok();
        base
    }
}

fn write_default_config() {
    let base = Config::ensure_dirs();
    let config_path = base.join("config.toml");
    if !config_path.exists() {
        let default = r#"# Uteke configuration
[store]
# path = "~/.uteke"  # Default store location

[embedding]
# model = "embeddinggemma-q4"
# max_seq_length = 256
"#;
        std::fs::write(&config_path, default).ok();
    }
}

// ── JSON output helpers ─────────────────────────────────────────────────────

fn print_json<T: serde::Serialize>(value: &T) {
    println!("{}", serde_json::to_string(value).unwrap());
}

// ── Human-readable output helpers ───────────────────────────────────────────

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
    /// Generate shell completions
    Completions {
        /// Shell type
        shell: Shell,
    },
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    // Initialize tracing
    if cli.verbose {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env().add_directive(Level::DEBUG.into()))
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env().add_directive(Level::WARN.into()))
            .init();
    }

    // Handle completions early — doesn't need store
    if let Commands::Completions { shell } = cli.command {
        let mut cmd = Cli::command();
        let name = cmd.get_name().to_string();
        generate(shell, &mut cmd, &name, &mut io::stdout());
        std::process::exit(0);
    }

    // Ensure config directory exists
    write_default_config();
    let config = Config::load();

    // Determine store path
    let store_path = cli
        .store
        .as_deref()
        .or(config.store_path.as_deref())
        .unwrap_or("~/.uteke");

    // Expand tilde
    let store_path = if store_path.starts_with("~/") {
        dirs::home_dir()
            .map(|h| {
                let rest = &store_path[2..];
                h.join(rest).to_string_lossy().to_string()
            })
            .unwrap_or_else(|| store_path.to_string())
    } else {
        store_path.to_string()
    };

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
    match &cli.command {
        Commands::Remember { content, tags } => {
            tracing::info!("Remembering: {content}");
            let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
            let id = uteke
                .remember(content, &tag_refs, None)
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
                .recall(query, *limit, tags_filter)
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
                .search(query, *limit)
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
                .list(tag.as_deref(), *limit, *offset)
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
                .stats()
                .map_err(|e| format!("Failed to get stats: {e}"))?;
            if cli.json {
                print_json(&stats);
            } else {
                print_stats_human(&stats);
            }
            Ok(())
        }
        Commands::Completions { .. } => {
            // Already handled in main()
            Ok(())
        }
    }
}
