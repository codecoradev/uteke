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
        /// Memory type: fact, procedure, preference, decision, context,
        /// note, insight, reference, event. Default 'fact' triggers pattern-based
        /// auto-inference (#349) unless an explicit type is passed.
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
        /// Source provenance: URL, file path, or identifier (#348)
        #[arg(long)]
        source: Option<String>,
        /// Source type: user, url, file, import, derived, system, unknown (#348)
        #[arg(long)]
        source_type: Option<String>,
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
        /// Recall strategy: vector, fts5, hybrid, or graph (graph = hybrid +
        /// graph-signal reranking, #378). Defaults to config's
        /// `[recall].default_strategy` (vector).
        #[arg(long)]
        strategy: Option<String>,
        /// Enable salience boost (how much each result matters) (#352).
        /// Uses the configured `[recall].salience_weight` (default 0.15).
        #[arg(long)]
        salience: bool,
        /// Enable recency boost (how fresh each result is) (#352).
        /// Uses the configured `[recall].recency_weight` (default 0.15).
        #[arg(long)]
        recency: bool,
        /// Follow relationship edges in memory metadata
        #[arg(long)]
        related: bool,
        /// Depth of relationship traversal (default: 1, use with --related)
        #[arg(long, default_value = "1")]
        depth: usize,
        /// Output as formatted context for AI prompt injection
        #[arg(long)]
        context: bool,
        /// Query memories as they existed at this timestamp (RFC3339, e.g. 2026-06-01T12:00:00Z)
        #[arg(long)]
        at: Option<String>,
        /// Content display format: 'auto' (detect), 'text' (force text), 'json' (pretty-print JSON)
        #[arg(long, default_value = "auto")]
        content_format: String,
        /// Filter results by JSON field (format: key=value, e.g. --where role=CTO)
        #[arg(long)]
        r#where: Option<String>,
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
        /// List memories as they existed at this timestamp (RFC3339)
        #[arg(long)]
        at: Option<String>,
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
    /// Import memories from JSONL, Markdown, or text files (re-embeds content)
    Import {
        /// Input file path (use - for stdin)
        #[arg(default_value = "-")]
        input: String,
        /// Tags to apply to all imported memories (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        /// Import format: auto, jsonl, markdown, text (default: auto-detect)
        #[arg(long, default_value = "auto")]
        format: String,
    },
    /// Generate shell completions
    Completions {
        /// Shell type
        shell: Shell,
    },
    /// Initialize uteke integration for an AI agent
    Init {
        /// Agent type: pi, claude, cursor, hermes
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
    /// Pin a memory so it never decays
    Pin {
        /// Memory ID (UUID)
        id: String,
    },
    /// Unpin a memory
    Unpin {
        /// Memory ID (UUID)
        id: String,
    },
    /// Recalculate importance scores for all memories
    Importance,
    /// Room management: list, stats, recall
    Room {
        #[command(subcommand)]
        command: RoomCommands,
    },
    /// Run performance benchmarks with synthetic data
    Bench {
        /// Memory counts to benchmark (default: 100, 1000, 10000)
        #[arg(long, value_delimiter = ',', default_value = "100,1000,10000")]
        counts: Vec<usize>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Knowledge graph operations
    Graph {
        #[command(subcommand)]
        command: GraphCommands,
    },
    /// List auto-wired edges for a memory (v8, #346)
    Edges {
        /// Memory ID (UUID)
        id: String,
        /// Multi-hop traversal depth. 0 (default) = list direct edges only.
        /// N>0 performs BFS across the edge table and returns reachable memory ids.
        #[arg(long, default_value = "0")]
        deep: usize,
        /// Filter by edge direction: `incoming`, `outgoing`, or `both` (default).
        ///
        /// `incoming` is useful for viewing backlinks (#350).
        #[arg(long, default_value = "both")]
        direction: String,
    },
    /// Rebuild `referenced_by` backlinks from existing forward edges (#350)
    RebuildBacklinks {
        /// Show only the count, no per-row detail (default: false)
        #[arg(long)]
        quiet: bool,
    },
    /// Run the full maintenance pipeline: lint → backlinks → dedup → orphans → compact → verify (#353)
    Dream {
        /// Comma-separated list of phases to run (default: all)
        #[arg(long, value_delimiter = ',')]
        phases: Vec<String>,
        /// Phases to skip (comma-separated)
        #[arg(long, value_delimiter = ',')]
        skip: Vec<String>,
        /// Dry-run mode: report only, make no changes
        #[arg(long)]
        dry_run: bool,
        /// Quiet mode: warnings/errors only
        #[arg(long)]
        quiet: bool,
    },
    /// Find orphan memories — disconnected nodes with low importance (#351)
    Orphans {
        /// Importance threshold below which a memory is a candidate (default 0.3)
        #[arg(long)]
        threshold: Option<f64>,
        /// Maximum results (0 = all, default 50)
        #[arg(long, default_value = "50")]
        limit: usize,
    },
    /// Show timeline events for a memory (audit log, #347)
    Timeline {
        /// Memory ID (UUID)
        id: String,
        /// Maximum events to return (0 = all)
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Document operations — wiki/knowledge base (#406, #411)
    Doc {
        #[command(subcommand)]
        command: DocCommands,
    },
}

/// Document subcommands (#411).
#[derive(Subcommand)]
pub enum DocCommands {
    /// Create or update a document from a file or stdin
    Create {
        /// Document slug (URL-friendly identifier)
        slug: String,
        /// Document title
        #[arg(long)]
        title: Option<String>,
        /// Read content from file (use - for stdin)
        #[arg(long)]
        file: Option<String>,
        /// Inline content (alternative to --file)
        #[arg(long)]
        content: Option<String>,
        /// Tags (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
    },
    /// Get a document by slug or ID
    Get {
        /// Document slug or ID
        id_or_slug: String,
    },
    /// List documents
    List {
        /// Maximum results
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Delete a document by ID
    Delete {
        /// Document ID
        id: String,
    },
    /// Export all documents as JSON
    Export {
        /// Output file (default: stdout)
        #[arg(long)]
        output: Option<String>,
    },
}

/// Subcommands for knowledge graph operations.
#[derive(Subcommand)]
pub enum GraphCommands {
    /// List all graph nodes
    Nodes {
        /// Filter by entity type
        #[arg(long)]
        entity_type: Option<String>,
    },
    /// List all graph edges
    Edges {
        /// Filter by relation type
        #[arg(long)]
        relation: Option<String>,
    },
    /// Find neighbors of a node (outgoing edges via BFS)
    Neighbors {
        /// Node label
        label: String,
        /// Max traversal depth
        #[arg(long, default_value = "1")]
        depth: usize,
    },
    /// Find shortest path between two nodes (BFS)
    Path {
        /// Source node label
        source: String,
        /// Target node label
        target: String,
        /// Max search depth
        #[arg(long, default_value = "5")]
        max_depth: usize,
    },
    /// Query edges by relation type
    Query {
        /// Relation type (e.g., "owns", "part_of")
        relation: String,
    },
    /// Show graph statistics
    Stats,
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
    /// Create a new room explicitly (#393)
    Create {
        /// Room ID (unique identifier)
        room_id: String,
        /// Optional title for the room
        #[arg(long)]
        title: Option<String>,
    },
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
        /// Semantic query — rank memories by relevance instead of chronological
        #[arg(long)]
        query: Option<String>,
        /// Filter by author
        #[arg(long)]
        author: Option<String>,
        /// Maximum results to return
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Minimum similarity score (0.0-1.0). Only used with --query.
        #[arg(long)]
        min: Option<f32>,
    },
    /// Delete a room (memories are NOT deleted, only room links)
    Delete {
        /// Room ID
        room_id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        confirm: bool,
    },
    /// Generate a summary of room discussion (topic clustering, no LLM needed)
    Summary {
        /// Room ID
        room_id: String,
    },
    /// Generate a structured document from room memories
    Document {
        /// Room ID
        room_id: String,
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
