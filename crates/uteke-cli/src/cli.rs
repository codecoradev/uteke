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
    /// Store path override (default: ~/.codecora/uteke)
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
        /// Who authored the memory: human or agent (#1084). Default: agent
        /// (matches the schema default, #1083).
        #[arg(long)]
        author_type: Option<String>,
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
        /// Timestamp anchor: prepend "[Session date/time: ...]" to the content
        /// so recall can answer temporal questions (#1232). Accepts RFC 3339,
        /// "YYYY-MM-DD HH:MM:SS", or "YYYY-MM-DD".
        #[arg(long)]
        timestamp: Option<String>,
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
        /// Recall strategy: vector, fts5, hybrid, graph, or fusion (fusion =
        /// weighted RRF of vector×1.7 + hybrid×1, #1123; graph = hybrid +
        /// graph-signal reranking, #378). Defaults to config's
        /// `[recall].default_strategy` (fusion).
        #[arg(long)]
        strategy: Option<String>,
        /// Enable salience boost (how much each result matters) (#352).
        /// Uses the configured `[recall].salience_weight` (default 0.15).
        /// When absent, salience uses the default weight (0.1). Use --no-salience to disable (#721).
        #[arg(long)]
        salience: Option<bool>,
        /// Enable recency boost (how fresh each result is) (#352).
        /// Uses the configured `[recall].recency_weight` (default 0.15).
        /// When absent, recency uses the default weight (0.1). Use --no-recency to disable (#721).
        #[arg(long)]
        recency: Option<bool>,
        /// Explain each result: show the ranking signals behind it (#1160)
        #[arg(long)]
        explain: bool,
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
        /// Search type filter: 'all' (default, unified), 'memory', or 'doc'.
        /// 'all' searches both memories and documents merged via RRF.
        /// 'memory' returns memories only (backward compatible).
        /// 'doc' returns documents only.
        #[arg(long)]
        r#type: Option<String>,
        /// Enrich results with cross-entity links (doc↔memory references).
        /// Populates linked_doc_slugs on memory results and linked_memory_ids
        /// on document results (#689).
        #[arg(long)]
        enrich: bool,
    },
    /// Show project context summary (memory counts, top tags, recent activity)
    Context {
        /// Namespace to summarize (default: resolved namespace)
        #[arg(long)]
        namespace: Option<String>,
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
    /// Update a memory in place (content/tags/importance/pinned/type) (#1202)
    Update {
        /// Memory ID (UUID)
        id: String,
        /// New content (omit to keep the current one)
        #[arg(long)]
        content: Option<String>,
        /// New tags, comma-separated (replaces the current set)
        #[arg(long)]
        tags: Option<String>,
        /// Importance override (0.0–1.0)
        #[arg(long)]
        importance: Option<f64>,
        /// Pinned state (never decays, boosted in recall)
        #[arg(long)]
        pinned: Option<bool>,
        /// Memory type: fact, procedure, preference, decision, context, note, insight, reference, event
        #[arg(long = "type")]
        r#type: Option<String>,
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
        /// Confirm destructive operations (the non-interactive equivalent of
        /// the y/N prompt; scripts and cron jobs must pass this)
        #[arg(long)]
        confirm: bool,
    },
    /// Show memory store statistics
    Stats,
    /// Print agent-facing memory tools guide for system prompt injection (#1010)
    Guide,
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
    /// Repair index by rebuilding from SQLite.
    /// Use --rebuild to delete corrupt index files first.
    /// Use --reembed to regenerate embeddings for memories missing them.
    Repair {
        /// Delete existing index files before rebuilding (ignores corrupt index).
        /// Use when the index is corrupted and normal repair fails.
        #[arg(long)]
        rebuild: bool,

        /// Re-generate embeddings for memories with missing/empty vectors.
        /// Makes previously invisible memories searchable via semantic recall.
        #[arg(long)]
        reembed: bool,
    },
    /// Export all memories to JSONL file (no embeddings — portable)
    Export {
        /// Output file path (use - for stdout)
        #[arg(default_value = "-")]
        output: String,
        /// Full structural export: rooms, graph, edges, documents, chunks,
        /// timeline (manifest + tagged NDJSON) — round-trips the whole store (#1057)
        #[arg(long)]
        full: bool,
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
        /// Distill the input into atomic facts before storing.
        ///
        /// Extraction mode depends on config:
        /// - offline (default): rule-based, zero API calls, no key needed.
        /// - llm: requires an OpenAI-compatible endpoint + API key.
        #[arg(long)]
        extract: bool,
        /// Override the extraction model (only used in LLM mode;
        /// else [extraction] model / UTEKE_EXTRACTION_MODEL)
        #[arg(long)]
        extract_model: Option<String>,
        /// Override the extraction API key (only used in LLM mode;
        /// else config / UTEKE_EXTRACTION_API_KEY / OPENAI_API_KEY)
        #[arg(long)]
        extract_api_key: Option<String>,
        /// Override the extraction base URL for LLM mode
        /// (e.g. http://localhost:11434/v1 for Ollama)
        #[arg(long)]
        extract_base_url: Option<String>,
        /// Max facts to keep per document (0 = default)
        #[arg(long)]
        extract_max_facts: Option<usize>,
        /// Batch import: directory path (imports all .md/.txt/.jsonl files)
        #[arg(long, conflicts_with = "input")]
        batch_dir: Option<String>,
        /// Force document strategy (no LLM extraction) for all files
        #[arg(long, requires = "batch_dir", conflicts_with = "as_memory")]
        as_doc: bool,
        /// Force memory extraction strategy (LLM) for all files
        #[arg(long, requires = "batch_dir", conflicts_with = "as_doc")]
        as_memory: bool,
        /// Dry run: show what would be imported without actually importing
        #[arg(long)]
        dry_run: bool,
        /// Max file size in bytes (default: 1MB)
        #[arg(long, default_value = "1048576")]
        max_size: usize,
        /// Recurse into subdirectories
        #[arg(long, default_value_t = false)]
        recursive: bool,
        /// Timestamp anchor: prepend "[Session date/time: ...]" to the imported
        /// content (text format only) so recall can answer temporal questions
        /// (#1232). Accepts RFC 3339, "YYYY-MM-DD HH:MM:SS", or "YYYY-MM-DD".
        #[arg(long)]
        timestamp: Option<String>,
    },
    /// Generate shell completions
    Completions {
        /// Shell type
        shell: Shell,
    },
    /// Initialize uteke integration for an AI agent
    Init {
        /// Agent type: pi, claude, cursor, opencode, hermes
        #[arg(long, default_value = "pi")]
        agent: String,
        /// For --agent hermes: install the memory-provider plugin (automatic
        /// recall + extraction as Hermes's default memory) instead of the
        /// uteke-tool plugin (manual HTTP-backed actions).
        #[arg(long, default_value_t = false)]
        memory_provider: bool,
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
    /// Memory lifecycle management (#935): cycle, promote, status
    Lifecycle {
        #[command(subcommand)]
        command: LifecycleCommands,
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
    /// Record feedback on a memory's usefulness (#718)
    ///
    /// Boosts (helpful) or reduces (unhelpful) the memory's importance score.
    /// Helpful: +0.05, Unhelpful: -0.10 (asymmetric — penalty > reward).
    Feedback {
        /// Memory ID (UUID)
        id: String,
        #[command(subcommand)]
        action: FeedbackAction,
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
    /// Show the full provenance report for a memory (#1172)
    Provenance {
        /// Memory ID (UUID)
        id: String,
    },
    /// Resolve a conflict: mark old_id superseded by new_id (#1053)
    Supersede {
        /// Full UUID or unambiguous prefix of the STALE memory
        old: String,
        /// Full UUID or unambiguous prefix of the CURRENT memory
        new: String,
        /// Why it was superseded (stored on the deprecation)
        #[arg(long)]
        reason: Option<String>,
    },
    /// Inspect the contradiction resolution ledger (#1172)
    Contradictions {
        #[command(subcommand)]
        command: ContradictionCommands,
    },
    /// Document operations — wiki/knowledge base (#406, #411)
    Doc {
        #[command(subcommand)]
        command: DocCommands,
    },
    /// Check for updates and upgrade to the latest version
    Upgrade {
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },
    /// Interactive onboarding — detect install, pick agent, toggle features, showcase
    Onboard {
        /// Skip interactive prompts and use defaults (non-TTY mode)
        #[arg(long)]
        yes: bool,
        /// Agent to configure (skip agent selection prompt)
        #[arg(long)]
        agent: Option<String>,
        /// Namespace to use (skip namespace prompt)
        #[arg(long)]
        namespace: Option<String>,
    },
}

/// Document subcommands (#411, #438).
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
        /// Parent document slug (for hierarchical documents, #438)
        #[arg(long)]
        parent: Option<String>,
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
        /// Show as tree hierarchy (#438)
        #[arg(long)]
        tree: bool,
    },
    /// List children of a document (#438)
    Children {
        /// Parent document slug or ID
        parent: String,
        /// Maximum results
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Move a document to a new parent or root (#438)
    Move {
        /// Document slug or ID to move
        id_or_slug: String,
        /// New parent document slug (omit to move to root)
        #[arg(long)]
        parent: Option<String>,
    },
    /// Show breadcrumb path from root to a document (#438)
    Breadcrumbs {
        /// Document slug or ID
        id_or_slug: String,
    },
    /// List all descendants of a document (#438)
    Descendants {
        /// Document slug or ID
        id_or_slug: String,
        /// Maximum depth (0 = unlimited)
        #[arg(long, default_value = "0")]
        max_depth: u32,
        /// Maximum results
        #[arg(long, default_value = "50")]
        limit: usize,
    },
    /// Search documents by query (semantic + FTS5)
    Search {
        /// Search query
        query: String,
        /// Maximum results
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Search mode: semantic, fts, or hybrid (default)
        #[arg(long, default_value = "hybrid")]
        mode: String,
    },
    /// Partially update a document — only provided fields are changed (#589)
    Update {
        /// Document slug or ID
        id_or_slug: String,
        /// New title (optional — no chunk rebuild)
        #[arg(long)]
        title: Option<String>,
        /// New content (triggers chunk rebuild)
        #[arg(long)]
        content: Option<String>,
        /// Read new content from file (use - for stdin; triggers chunk rebuild)
        #[arg(long)]
        file: Option<String>,
        /// Replace tags (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        /// Replace metadata (JSON string)
        #[arg(long)]
        metadata: Option<String>,
    },
    /// Delete a document by ID (cascades to children, #438)
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

/// Subcommands for the contradiction resolution ledger (#1172).
#[derive(Subcommand)]
pub enum ContradictionCommands {
    /// List superseded-but-not-restored memories (the resolution ledger)
    List {
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Maximum entries to show
        #[arg(long, default_value = "50")]
        limit: usize,
    },
    /// Restore a superseded memory — undoes the supersession pair
    Undo {
        /// Memory ID (UUID) of the retired memory to restore
        id: String,
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
    /// Move a memory to another namespace (#1181)
    Move {
        /// Memory ID
        id: String,
        /// Target namespace
        namespace: String,
    },
    /// Rename a namespace — merges into the target when it already exists (#1181)
    Rename {
        /// Current namespace name
        from: String,
        /// New namespace name
        to: String,
    },
    /// Delete a namespace with an explicit strategy for its memories (#1181)
    Delete {
        /// Namespace to delete
        name: String,
        /// What happens to its memories: refuse (default), merge, deprecate
        #[arg(long, default_value = "refuse")]
        strategy: String,
        /// Target namespace when strategy = merge
        #[arg(long)]
        target: Option<String>,
        /// Confirm the deletion (required)
        #[arg(long)]
        confirm: bool,
    },
}

/// Feedback actions for trust scoring (#718).
#[derive(Subcommand, Clone)]
pub enum FeedbackAction {
    /// Mark memory as helpful: importance += 0.05
    Helpful,
    /// Mark memory as unhelpful: importance -= 0.10
    Unhelpful,
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
    /// Rename a room — registry row and all member/document links move together (#1202)
    Rename {
        /// Current room ID
        old_id: String,
        /// New room ID
        new_id: String,
    },
    /// Update a room's title and/or description (#1202)
    Update {
        /// Room ID
        room_id: String,
        /// New title (omit to keep the current one)
        #[arg(long)]
        title: Option<String>,
        /// New description (omit to keep the current one)
        #[arg(long)]
        description: Option<String>,
    },
    /// Move a memory from one room to another, preserving link provenance (#1202)
    MoveMemory {
        /// Memory ID
        memory_id: String,
        /// Source room ID (the memory must be a member)
        #[arg(long = "from")]
        from_room: String,
        /// Target room ID (must exist)
        #[arg(long = "to")]
        to_room: String,
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
    ///
    /// Examples:
    ///   uteke room recall engineering                    # all memories
    ///   uteke room recall engineering "caching strategy" # semantic search
    ///   uteke room recall engineering --author alice     # filter by author
    Recall {
        /// Room ID
        room_id: String,
        /// Semantic query (optional positional) — rank memories by relevance.
        /// Equivalent to --query. Use when you want natural `room recall <id> "<query>"` syntax.
        query: Option<String>,
        /// Semantic query (flag form) — same as positional query.
        /// If both are given, positional takes precedence.
        #[arg(long = "query", visible_alias = "query-flag")]
        query_flag: Option<String>,
        /// Filter by author
        #[arg(long)]
        author: Option<String>,
        /// Maximum results to return (0 = all, default for bounded room contexts)
        #[arg(long, default_value = "0")]
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
    /// Generate a structured document from room memories (API: POST /room/summary)
    Document {
        /// Room ID
        room_id: String,
    },
    /// Link a document to a room
    AddDocument {
        /// Room ID
        room_id: String,
        /// Document slug
        doc_slug: String,
    },
    /// Unlink a document from a room
    RemoveDocument {
        /// Room ID
        room_id: String,
        /// Document slug
        doc_slug: String,
    },
    /// List documents linked to a room
    ListDocuments {
        /// Room ID
        room_id: String,
    },
    /// List rooms that reference a document
    ListRooms {
        /// Document slug
        doc_slug: String,
    },
    /// Consolidate a room's memories into fewer, denser records (#1088).
    ///
    /// Dry-run by default: shows the batching plan and estimated LLM calls
    /// without contacting any API. Pass --apply to execute (requires
    /// extraction LLM config in uteke.toml or UTEKE_* env vars).
    Consolidate {
        /// Room ID
        room_id: String,
        /// Execute the plan (LLM calls + store writes). Omit for dry-run.
        #[arg(long)]
        apply: bool,
        /// Max LLM requests for this run (default 10, failed calls count too)
        #[arg(long, default_value_t = 10)]
        max_calls: usize,
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
        /// Preview what would be deleted without actually deleting
        #[arg(long)]
        dry_run: bool,
    },
}

/// Subcommands for memory lifecycle management (#935).
#[derive(Subcommand)]
pub enum LifecycleCommands {
    /// Run one lifecycle cycle: soft-deprecate aged memories (cap-limited) + auto-prune
    Cycle {
        /// Namespace to target (default: all)
        #[arg(long)]
        namespace: Option<String>,
    },
    /// Promote a deprecated memory back to active
    Promote {
        /// Memory ID to promote
        id: String,
    },
    /// Show lifecycle status: active vs deprecated counts, config summary
    Status {
        /// Namespace to check (default: all)
        #[arg(long)]
        namespace: Option<String>,
    },
}
