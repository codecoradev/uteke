//! CLI argument definitions (clap structs and enums).
//!
//! All clap-derived types live here so main.rs stays focused on
//! orchestration (logging, config, dispatch).

use clap::{Parser, Subcommand};
use clap_complete::Shell;

/// Uteke — persistent memory engine for AI agents.
#[derive(Parser)]
#[command(
    name = "uteke",
    about = "The Brain for Your AI — persistent memory engine",
    version
)]
pub struct Cli {
    /// Store path override (default: ~/.uteke)
    #[arg(long, global = true)]
    pub store: Option<String>,

    /// Namespace for multi-agent isolation (default: "default")
    #[arg(long, global = true)]
    pub namespace: Option<String>,

    /// Output as JSON
    #[arg(long, global = true)]
    pub json: bool,

    /// Enable verbose logging
    #[arg(long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

/// Supported shell types for completions and hooks.
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum SupportedShell {
    Bash,
    Zsh,
    Fish,
}

/// All top-level CLI subcommands.
#[derive(Subcommand)]
pub enum Commands {
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
        /// Room ID to link this memory to (collaborative context)
        #[arg(long)]
        room: Option<String>,
        /// Author attribution when storing in a room
        #[arg(long)]
        author: Option<String>,
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
    /// Room management: list, stats, recall
    Room {
        #[command(subcommand)]
        command: RoomCommands,
    },
}

/// Subcommands for tag management.
#[derive(Subcommand)]
pub enum TagCommands {
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

/// Subcommands for namespace management.
#[derive(Subcommand)]
pub enum NamespaceCommands {
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

/// Subcommands for room management.
#[derive(Subcommand)]
pub enum RoomCommands {
    /// List all rooms
    List {
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
    },
    /// Show room statistics and participants
    Stats {
        /// Room ID
        room_id: String,
    },
    /// Recall all memories in a room (cross-namespace)
    Recall {
        /// Room ID
        room_id: String,
        /// Filter by author
        #[arg(long)]
        author: Option<String>,
        /// Maximum results to return
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Delete a room (memories are NOT deleted, only room links)
    Delete {
        /// Room ID
        room_id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        confirm: bool,
    },
}

/// Subcommands for memory aging operations.
#[derive(Subcommand)]
pub enum AgingCommands {
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
