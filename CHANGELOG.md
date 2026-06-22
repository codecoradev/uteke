## [0.4.0] — 2026-06-22

### Added

- **Hierarchical documents — depth-10 tree engine** (#438 #440)
  - Schema v12: `parent_id`, `path` (materialized), `depth`, `sort_order`,
    `has_children` columns on documents table
  - Tree operations: create with parent, children, descendants, move,
    breadcrumbs
  - `MAX_DEPTH = 10` enforced in `move_document()`
  - `uteke doc create --parent <slug>`, `uteke doc children`, `uteke doc
    descendants`, `uteke doc move`, `uteke doc breadcrumbs`
  - `uteke doc list --tree` for indented tree view

- **Hybrid document search** (#440)
  - `uteke doc search <query>` with 3 modes:
    - `semantic`: usearch chunk embeddings with `chunk:` prefix
    - `fts`: FTS5 keyword search on title and slug
    - `hybrid` (default): Reciprocal Rank Fusion of semantic + FTS (k=60)
  - FTS5 virtual table `documents_fts(title, slug, content)` with
    INSERT/UPDATE/DELETE sync triggers
  - Chunk embeddings auto-inserted into usearch on document upsert
  - Old chunks cleaned from usearch on update and delete

### Fixed

- **move_document descendant path predicate** used UUID instead of full
  materialized path. Descendants of nested documents were not updated after
  move operations.
- **delete_document cascade** hardcoded `/{uuid}/%` prefix that never
  matched actual materialized paths. Now fetches document path first.
- **doc_delete namespace** hardcoded `DEFAULT_NAMESPACE`. Now accepts
  `namespace: Option<&str>` parameter.
- **move_document** now recomputes `has_children` on old parent after
  moving last child away.

## [0.3.3] — 2026-06-22

### Fixed

- **Critical: server deadlock on POST /remember** (#442)
  - RwLock deadlock in `remember_precomputed()`: the write lock on the
    vector index was held when `auto_link_cosine()` tried to acquire a
    read lock on the same RwLock. After 2-3 inserts, the server became
    completely unresponsive.
  - Fix: `drop(index)` before calling `auto_link_cosine()`.

- **Cosine auto-linking never created edges** (#442)
  - `auto_link_cosine()` deadlocked before reaching search/edge code.
  - With the deadlock fixed, `similar_to` and `possible_duplicate`
    edges are now created correctly on every `remember()`.

## [0.3.2] — 2026-06-22
 — 2026-06-22

### Fixed

- **Harden schema migrations for partially-migrated databases** (#435)
  - `migrate_v9_to_v10` now uses `column_exists()` guard before `ALTER TABLE
    ADD COLUMN` for `source` and `source_type`. Prevents "duplicate column
    name" crash on databases that were partially migrated by a buggy binary
    (e.g. v0.3.0 which ran SCHEMA_INDEXES before migrations).
  - `migrate_v7_to_v8` now creates `idx_memories_slug` index as a separate
    statement instead of inside `execute_batch`, producing clearer error
    messages on failure.

### Docs

- Added detailed documentation checklist to AGENT.md release process (#434).

## [0.3.1] — 2026-06-21

### Fixed

- **Critical: schema migration failure for existing databases**
  - DBs created with v0.2.x or earlier (schema v7) failed to open after
    upgrading to v0.3.0. `CREATE INDEX ... ON memories(slug)` in the SCHEMA
    constant was executed BEFORE migrations added the `slug` column.
  - Fix: Split schema init into two phases. `SCHEMA` contains only
    `CREATE TABLE` + safe indexes. `SCHEMA_INDEXES` (indexes on
    migration-added columns) runs AFTER `ensure_schema_version()` completes.
    Index creation is best-effort (errors logged, not fatal).
  - The problematic indexes (`idx_memories_namespace`, `idx_memories_deprecated`,
    `idx_memories_slug`) are now created post-migration.

## [0.3.0] — 2026-06-21

### Added

- **Document engine — wiki/knowledge base** (#406)
  - Schema v11: `documents` + `document_chunks` tables
  - Full markdown content with slug, title, tags, version
  - Auto-chunking via markdown chunker (#405) + per-chunk embeddings
  - Foundation for Obsidian/Outline-style wiki

- **Document CLI commands** (#411)
  - `uteke doc create/get/list/delete/export`
  - Accept content from --file, --content, or stdin
  - Auto-derive title from first heading

- **Markdown/prose chunker** (#405)
  - Split by headings (levels 1-6), respect code block fences
  - Paragraph-level fallback for oversized sections
  - `TextChunk { heading, content, level, char_start, char_end }`

- **Embed-aware chunking** (#407)
  - `chunk_markdown_embed_aware()` derives chunk size from `embedder.max_seq_len()`
  - ~4 chars per token heuristic

- **Cosine-based auto-linking + dedup** (#401)
  - `auto_link_cosine()` runs after every `remember()`
  - `similar_to` edge when cosine >= 0.80
  - `possible_duplicate` edge when cosine >= 0.92
  - Namespace-scoped (no cross-namespace links)

- **Configurable limits** (#404)
  - `LimitsConfig` struct with env var overrides
  - MAX_CONTENT_LENGTH: 10K → 100K
  - `[limits]` section in uteke.toml

- **`/graph` API endpoint** (#408)
  - `GET /graph` returns all nodes + edges + stats as JSON
  - For visualization clients

- **View-only API key** (#409)
  - `--read-only-token` for GET-only access
  - Dual-role: Admin (full) + ReadOnly (GET only)
  - Env vars: `UTEKE_AUTH_TOKEN`, `UTEKE_READ_ONLY_TOKEN`

- **Hermes plugin room_remember** (#410)
  - New `room_remember` action in plugin template

### Changed

- Schema version bumped from v10 to v11
- Internal dependency versions widened from `0.2.0` to `0.3`
- `serialize_embedding` visibility changed to `pub(crate)`

## [0.2.1] — 2026-06-21

### Added

- **Hermes plugin auto-install** (#385)
  - `uteke init --agent hermes` now installs directly to
    `~/.hermes/plugins/uteke-tool/` instead of generating to CWD.
  - Plugin uses Python stdlib only (no `requests` dependency needed).
  - MCP server discovery: README documents `hermes mcp add uteke --command
    uteke-mcp` as an alternative integration path.

- **Room operations in Hermes plugin** (#395)
  - Plugin now exposes `room_create`, `room_recall`, `room_list`,
    `room_summary`, `room_stats`, `room_delete` actions.
  - Added server endpoints: `POST /room/create`, `GET /room/list`,
    `POST /room/stats`, `DELETE /room/delete`.

- **`uteke room create` command** (#393)
  - Explicit room creation: `uteke room create <id> [--title "Name"]`

### Fixed

- **Room list cross-namespace visibility** (#392)
  - `uteke room list` now shows ALL rooms by default, not just those in
    the current `--namespace`. Rooms are collaboration spaces spanning
    namespaces.

- **Schema mismatch error message** (#394)
  - Error now includes binary name and version: "please upgrade uteke
    (current binary: uteke-core v0.2.0, schema v10)".

- **Hermes plugin missing `__init__.py`** (#402)
  - `uteke init --agent hermes` now generates `__init__.py` in the plugin
    directory. Without it, Hermes logs a warning and the plugin never
    loads.

- **Contradictory server detection log** (#403)
  - When server was detected but command unsupported via HTTP (aging,
    doctor, etc.), the fallback path logged "No server detected" —
    contradicting the earlier detection message. Now logs accurately
    based on actual server state.

- **Misleading `db_size_bytes` in stats** (#403)
  - Stats output now labels database size as "(global, shared)" to
    clarify it reflects the entire shared SQLite file, not just the
    queried namespace.

### Added (previous)

- **MCP Streamable HTTP transport** (#381)
  - `uteke-mcp` protocol version bumped from `2024-11-05` to `2025-06-18`
    (current MCP spec).
  - New `POST /mcp` endpoint on `uteke-server` exposing the full MCP
    JSON-RPC API over HTTP. Returns `Content-Type: application/json` with
    `MCP-Protocol-Version: 2025-06-18` header.
  - Shared handler extracted into `uteke_mcp` library crate — used by both
    the stdio binary (`uteke-mcp`) and the HTTP endpoint (`uteke-serve`).
  - Enables remote MCP clients (Claude Desktop, web agents) to connect
    without spawning a subprocess.

- **Citation & source attribution** (#348)
  - Schema v10 migration: adds `source` and `source_type` columns to
    `memories` table. Existing rows get `source_type = 'unknown'`.
  - Source types: `user`, `url`, `file`, `import`, `derived`, `system`,
    `unknown`.
  - `Memory` struct gains `source: Option<String>` and `source_type: String`
    fields. Defaults: `source=None`, `source_type="user"`.
  - `Uteke::set_source(id, source, source_type)` for post-insert provenance
    updates.
  - `ExportEntry` gains optional `source` field for round-trip preservation.
  - CLI: `--source <URL/path>` and `--source-type <type>` flags on
    `uteke remember`.
  - Import sets `source_type = 'import'` with `source = 'import:<filename>'`.

- **Dream cycle** (#353)
  - New `crates/uteke-core/src/dream.rs` module: coordinated maintenance
    pipeline that runs all 6 phases in dependency order, all local, zero
    LLM. Inspired by GBrain's overnight dream cycle.
  - Phases:
    1. **Lint** — type validation + broken-ref count
    2. **Backlinks** — rebuild `referenced_by` edges (#350)
    3. **Dedup** — find & merge near-duplicates (existing `consolidate`)
    4. **Orphans** — detect disconnected memories (#351-compatible inline
       SQL when #351 is not yet merged)
    5. **Compact** — aging cleanup + prune cold memories (existing)
    6. **Verify** — schema + index integrity check (existing `verify`)
  - `DreamPhase` enum, `DreamReport`, `PhaseResult`, `PhaseStatus` types.
  - `Uteke::dream(namespace, dry_run, phases)` orchestrator.
  - CLI: new `uteke dream [--phases] [--skip] [--dry-run] [--quiet]`
    command. Exits non-zero on errors (cron-friendly).

- **Orphan detection** (#351)
  - New `crates/uteke-core/src/orphans.rs` module: detect memories with no
    graph edges, no recall access, not pinned, and below an importance
    threshold.
  - Detection is a single SQL pass (LEFT JOIN on `memory_edges` twice) —
    no O(n²) scan.
  - `OrphanMemory` struct with `orphan_score` (0.0..=1.0):
    `(1 - edge_density) × 0.4 + (1 - access_freq) × 0.3 + (1 - importance) × 0.3`.
  - `Uteke::find_orphans(namespace, threshold, limit)` with namespace
    scoping and `DEFAULT_ORPHAN_THRESHOLD = 0.3`.
  - CLI: new `uteke orphans [--threshold 0.3] [--limit 50]` command.
  - Leverages #350 backlinks: any memory referenced by another is
    automatically excluded (it has an incoming `referenced_by` edge).

- **Timeline event tracking** (#347)
  - New `crates/uteke-core/src/timeline.rs` module: append-only audit log
    per memory in the `timeline_events` table (schema v9).
  - Event types: `created`, `updated`, `recalled`, `consolidated`, `tagged`,
    `forgot`. Each event has optional JSON `event_data`.
  - Store methods: `add_timeline_event`, `list_timeline_events`,
    `count_timeline_events`.
  - Uteke methods: `timeline(memory_id, limit)`,
    `count_timeline_events(memory_id)`, `try_timeline_event()` (best-effort
    append that never fails the primary operation).
  - Auto-emission: `remember_precomputed` now emits a `created` event.
  - CLI: new `uteke timeline <id> [--limit N]` command (default 20 events).
  - Schema migration v8 → v9 (idempotent, no data backfill — timeline
    tracking starts from this version forward).

- **Memory type formalization** (#349)
  - `MemoryType` enum expanded from 5 to 9 variants: original (Fact,
    Procedure, Preference, Decision, Context) + new (Note, Insight,
    Reference, Event).
  - Pattern-based auto-inference (`MemoryType::infer_from_content`):
    URL prefix → `Reference`; decided/chose/will use → `Decision`;
    realized/learned/discovered → `Insight`; how-to/numbered list →
    `Procedure`; always/never/prefer/hate → `Preference`; ISO date +
    time word → `Event`; fallback → `Note`. Zero LLM.
  - When callers pass the default `"fact"`, `remember_typed` now runs
    inference and overrides with a more specific type when one is
    detected (falls back to `Fact` for ambiguous content, preserving
    backward compatibility).
  - `MemoryType::recall_boost()` — small additive score boost per type
    (Decision/Preference +0.05, Insight +0.03, Event +0.02, Note -0.02).
    To be wired into recall scoring by #352.
  - CLI: `--type` help text documents the new types and auto-inference.

- **Salience + recency dual-axis recall ranking** (#352)
  - New `crates/uteke-core/src/salience_recency.rs` module with two
    orthogonal, additive boost functions:
    - `salience_score(memory)` — 0..=1 blend of `access_count`, `importance`,
      and `pinned` (importance × 0.5 + access_freq × 0.3 + pinned 0.2).
    - `recency_score(memory, now)` — per-type exponential decay
      (`exp(-age_days / τ)`). Time constants: Decision/Preference 365d,
      Fact/Reference 180d, Insight 240d, Event 30d, default 90d.
      (τ is the age at which recency drops to ~0.37 = 1/e.)
  - `SalienceRecencyConfig { salience_weight, recency_weight }` defaults
    to zero (opt-in per query). `sanitized()` clamps weights to [0, 1].
  - `Uteke::set_salience_recency_config()` for per-query override.
  - Boosts applied AFTER recall cache lookup (cache stays time-independent).
  - CLI: `--salience` / `--recency` flags on `recall`. Default weights
    (0.15 each) configurable via `[recall]` in `uteke.toml`.
  - Public exports: `salience_score`, `recency_score`, `type_half_life_days`,
    `apply_boosts`, `SalienceRecencyConfig`.

- **Backlink auto-generation** (#350)
  - Bidirectional links: whenever memory A creates a forward edge to B
    (`references`, `tagged_as`, `supersedes`, `replies_to`), an inverse
    `referenced_by` edge from B → A is automatically inserted. Makes the
    graph navigable in both directions without an O(n) scan.
  - New `Store::ensure_backlink()` (idempotent),
    `Store::add_memory_edge_with_backlink()`, and
    `Store::rebuild_backlinks()` (scan + repair pass).
  - `add_memory_edges_batch()` (used by `wire_edges` on every `remember`)
    now also ensures the backlink for each forward edge.
  - New `Uteke::link_memories()` public API for explicit edges with
    automatic backlink.
  - New `Uteke::rebuild_backlinks()` for one-time migration of pre-#350
    stores.
  - New CLI command `uteke rebuild-backlinks [--quiet]` rebuilds all
    `referenced_by` edges from existing forward edges.
  - `uteke edges <id>` gains `--direction <incoming|outgoing|both>`
    (default `both`); `incoming` shows backlinks.
  - New public exports: `backlink_type_for`, `EdgeList`, `MemoryEdge`,
    `EDGE_REFERENCES`, `EDGE_REFERENCED_BY`, `EDGE_REPLIES_TO`,
    `EDGE_SUPERSEDES`, `EDGE_TAGGED_AS`.

- **Graph-augmented RAG reranking** (#378)
  - New recall strategy `graph`: runs the hybrid (RRF) pipeline, then fuses
    graph signals from the `memory_edges` table into each result's score.
    Well-connected memories drift upward; isolated memories are untouched.
  - New `crates/uteke-core/src/graph_rerank.rs` module:
    - `compute_graph_signals()` — single batched SQL query over
      `memory_edges` computes per-memory `edge_count`, `neighbor_count`,
      `edge_type_diversity`, `incoming_count`, `outgoing_count`.
    - `rerank_with_graph()` — additive, log-scaled density + authority
      boosts (`ln(1+x) * weight`), capped at 1.0. Disabled or empty-signal
      inputs are a no-op (cold-start safe).
    - `GraphRerankConfig { density_weight, authority_weight, enabled }`
      with `sanitized()` clamping.
  - `RecallStrategy::Graph` variant added (`memory/types.rs`); wired into
    `recall_hybrid` so the boost runs *before* caching (cache is
    strategy-keyed → `graph` has its own entries, no collision with
    `hybrid`/`vector`/`fts5`).
  - New `Uteke` field + `open_with_embedding_and_graph` constructor so the
    CLI passes the merged `[recall]` weights.
  - CLI: new `--strategy <vector|fts5|hybrid|graph>` flag on `recall`
    (defaults to `[recall].default_strategy`, itself defaulting to
    `vector` — preserves original behavior). Unknown strategies warn and
    fall back to `vector`.
  - Config: `[recall]` gains `default_strategy`, `graph_density_weight`,
    `graph_authority_weight`, `graph_rerank_enabled`, plus env overrides
    `UTEKE_RECALL_STRATEGY`, `UTEKE_GRAPH_DENSITY_WEIGHT`,
    `UTEKE_GRAPH_AUTHORITY_WEIGHT`, `UTEKE_GRAPH_RERANK_ENABLED`.
  - 10 new unit tests covering signal counting, hub boost, log-saturation,
    score capping, cold-start no-op, disabled-flag no-op, and a <10ms
    latency guard over 5000 edges.
  - Backward compatible: existing strategies are unchanged; `graph` is opt-in.

### Changed

- **Bump sha2 0.10 → 0.11** (supersedes Dependabot PR #364)
  - sha2 0.11 dropped the `LowerHex` impl on the digest output, breaking
    `format!("{:x}", hasher.finalize())` in `engine.rs`.
  - Fix: iterate the digest bytes and format each as `{:02x}`.
  - Unifies the crate on a single sha2 version (uteke-server was already
    on 0.11).

### Added

- **OpenAI + Ollama embedding backends** (#337)
  - New `OpenAiEmbedder` (`crates/uteke-core/src/embed/openai.rs`) — HTTP
    call to `{base_url}/embeddings`, default model `text-embedding-3-small`
    (1536d). Azure OpenAI compatible via `base_url`.
  - New `OllamaEmbedder` (`crates/uteke-core/src/embed/ollama.rs`) — HTTP
    call to `{base_url}/api/embed`, default model `nomic-embed-text` (768d).
  - `[embedding]` config section extended with `api_key`, `base_url`, `dims`.
  - New env vars: `UTEKE_EMBEDDING_BACKEND`, `UTEKE_EMBEDDING_MODEL`,
    `UTEKE_EMBEDDING_API_KEY` (fallback: `OPENAI_API_KEY`),
    `UTEKE_EMBEDDING_BASE_URL`, `UTEKE_EMBEDDING_DIMS`.
  - Dim mismatch detection: opening an existing store with a backend that
    produces a different dims now returns a clear error pointing the user
    at `uteke repair` instead of silently corrupting the index.
  - `reqwest` `json` feature added (always included — no feature flag).
  - 16 new unit tests (backend construction, endpoint normalization, default
    constants, response parsing, config merge + env var precedence).
  - ONNX remains the default — fully backward compatible.

- **Auto-wired memory edges** (#346)
  - New `memory_edges` SQLite table (schema v8) for typed edges between
    memories.
  - Optional `slug` column on memories for `[[slug]]` Wikilink-style
    references.
  - Pattern-based entity extraction on every `remember()` call — zero LLM,
    pure string parsing:
    - `[[slug]]` → `references` edge
    - `@tag` → `tagged_as` edge (most recent memory with that tag)
    - `^<uuid>` → `supersedes` edge
    - `><uuid>` → `replies_to` edge
    - `rel:<type>:<uuid>` (legacy `--meta` form) → `<type>` edge
  - New `uteke edges <id> [--deep N]` CLI subcommand: lists direct edges or
    runs BFS across the edge table.
  - Rewrote `get_related()` to prefer the edge table (indexed SQL) over the
    old O(n) JSON metadata scan. Legacy path retained as fallback.
  - Migration v7→v8 backfills existing `metadata.relationships` JSON entries
    into `memory_edges` rows.
  - 20 new unit tests (extraction patterns, edge roundtrip, BFS cycle safety,
    slug/tag resolution).

### Changed

- Schema version v7 → v8.
- `Memory` struct gains optional `slug: Option<String>` field.
- Clippy: cleaned up pre-existing `else { if .. }` collapse warnings in
  `commands/graph.rs` (3 sites) so `cargo clippy --workspace -- -D warnings`
  now passes cleanly.

## [0.2.0] — 2026-06-14

### Added

- **SQLite knowledge graph storage** (#317)
  - `graph_nodes` + `graph_edges` tables (schema v7)
  - `uteke graph nodes/edges/neighbors/path/query/stats`
  - BFS pathfinding with parent tracking
  - `GraphStore` API: upsert_node, add_edge, find_path, query_relation
- **Structured memory — JSON content** (#293)
  - Auto-detect JSON content, sets `content_type='json'`
  - Schema v6: `content_type TEXT NOT NULL DEFAULT 'text'`
  - Flatten JSON for embedding: `{"name":"Alice"}` → `"name: Alice"`
  - CLI: `--where key=value` filters JSON memories
  - CLI: `--content-format json` pretty-prints JSON output
- **External knowledge import** (#46)
  - `uteke import <file> --tags a,b --format markdown`
  - Auto-detect: `.md`, `.jsonl`, `.txt` from extension or content
  - Markdown: split by headings, each section becomes a memory
  - Text: split by double newline (paragraphs)
  - Stdin: `echo 'text' | uteke import - --tags note`
- **AST-aware code chunking** (#245)
  - Regex-based chunker (zero tree-sitter dependency)
  - Languages: Rust, Go, Python, TypeScript/JS, Dart
  - `detect_language()`, `chunk_code()`, `extract_imports()`
  - Fallback: whole file for unknown languages
- **Hermes plugin integration guide** (#55)
  - `docs/integrations/hermes.md` — complete setup guide
- **Docker quickstart** (#336)
  - `docker-compose.yml` with healthcheck + volume persistence
  - `docs/docker.md` — full Docker guide
  - README Docker section with localhost-only binding
- **Environment variable coverage** (#338)
  - `UTEKE_LOG_LEVEL`, `UTEKE_SERVER_HOST/PORT`
  - `UTEKE_RECALL_MIN_SCORE/STRICT`
  - Resolution: CLI flag > env var > config file > default
  - Invalid values logged as warning

### Changed

- **Schema v7**: graph_nodes + graph_edges tables
- **Schema v6**: content_type column (text vs json)
- `PRAGMA foreign_keys=ON` enabled for cascade deletes

### Fixed

- **Recall `--json` output consistency** — empty results always output `[]`
  instead of `{"results":[]}` when min_score threshold is active

## [0.1.0] — 2026-06-13

### Added

- **Performance benchmark command** (#49)
  - `uteke bench` — generates synthetic memories and benchmarks insert/recall at scale
  - `--counts 100,1000,10000` — configurable memory counts
  - `--json` — machine-readable output
  - Measures: insert ops/sec, recall avg/p95 latency, DB/index size
- **LongMemEval retrieval harness** (#316)
  - `benchmarks/longmemeval/` — evaluates retrieval accuracy vs LongMemEval benchmark
  - Session-level Recall@5/10/50, NDCG@5/10/50
  - Per-question-type breakdown
  - Comparison table vs Mem0/Hindsight
- **Time-travel queries** (#292)
  - `uteke recall --at 2026-06-01T12:00:00Z` — recall memories as they existed at a specific point in time
  - `uteke list --at 2026-06-01T12:00:00Z` — list memories that existed at timestamp
  - Filters by created_at, valid_from/valid_until, deprecated status
  - Server: `/recall` and `/list` accept `"at"` field
- **Pluggable embedding models** (#249)
  - New `Embedder` trait — enables different embedding backends (ONNX, OpenAI, Ollama)
  - `EmbeddingEngine` renamed to `OnnxEmbedder`, implements `Embedder`
  - Config: `[embedding] backend = "onnx"` (default)
  - Lazy backend selection via `embedder_backend` field
- **Room document view** (#306)
  - `uteke room document <room>` — generate structured document from room memories
  - Sections grouped by memory_type: 📋 Decisions, 🔍 Facts, ⚙️ Procedures, 🎨 Preferences, 💬 Context
  - Pinned memories get 📌 section first
  - Server: `POST /room/document`
- **Semantic room recall** (#304)
  - `uteke room recall <room> --query "topic"` — semantic recall within a room
  - `/room/recall` endpoint
- **Room summary** (#305)
  - `uteke room summary <room>` — LLM-free room summary via tag clustering
  - `/room/summary` endpoint
- **Configurable recall threshold** (#252)
  - `uteke recall --min 0.7` — set minimum similarity score
  - `uteke recall --strict` — use 0.7 default threshold
  - `[recall] min_score = 0.7` in config
- **Room-based memory** (#286)
  - `uteke room create/list/add/recall/remove/delete` — full room management
  - Memories can belong to rooms with author attribution
  - `/room/*` endpoints
- **Agent recall optimization** (#181)
  - Recall cache: LRU with TTL (5min, 256 entries) — avoids redundant embedding (~50ms) for repeated queries
  - `uteke recall --context` — formatted output for AI prompt injection
  - Cache metrics in `uteke stats` (hits, misses, hit rate)
  - Auto-invalidation on remember/forget mutations
  - `recall_context()` library API for direct prompt injection
- **Relationship graph layer between memories** (#246)
  - `uteke recall --related --depth N` — follow relationship edges via BFS traversal
  - `uteke remember --meta "rel:supersedes:ID"` — link memories with typed relationships
  - Relationship types: supersedes, contradicts, part_of, references
  - Score decay per depth level (0.8x) to rank direct matches higher
  - No new tables — relationships stored in metadata JSON

### Fixed

- **Recall `--json` output consistency** — empty results now always output `[]`
  instead of `{"results":[]}` when min_score threshold is active. Ensures
  machine consumers (cora-cli, scripts, MCP) can parse output reliably.

### Changed
  - New `memory_tags` junction table for O(log n) tag lookups
  - Schema v5: creates table, populates from existing JSON tags
  - Dual-write: insert/update writes to both JSON column and junction table
  - All tag queries use junction table instead of json_each()
  - Backward compat: JSON `tags` column preserved
- **Smart memory decay and importance scoring** (#247)
  - Composite importance score: 0.3*access + 0.3*recency + 0.2*connectivity + 0.2*pinning
  - `uteke pin <id>` / `uteke unpin <id>` — pin memories so they never decay
  - `uteke importance` — recalculate importance scores for all memories
  - Schema v4: `importance REAL` and `pinned INTEGER` columns
  - Exponential recency decay (half-life: 30 days)
  - Connectivity score from relationship graph (#246)


## [0.0.15] — 2026-06-12

### Changed

- **CLI cold start: ~3s → ~20ms for non-embedding commands** (#185)
  ONNX embedding model is now loaded lazily on first use. Commands like
  `list`, `get`, `stats`, `tags`, `forget`, `namespace`, `aging`, `export`,
  `doctor`, and `verify` start instantly without waiting for model load.
  Commands that need embedding (`remember`, `recall`, `search`) still take
  ~3s on first use per process invocation.
- **Refactor CLI into modular structure** (#131)
  CLI argument definitions extracted to `cli.rs`, logging setup to `logging.rs`.
  main.rs reduced from 449 to ~100 lines for easier maintenance.
- Release workflow now decoupled: crates.io publish runs in parallel with
  builds, GitHub Release only waits for builds. Single platform failure
  no longer blocks release.
- Shell hook scripts inlined into uteke-cli crate for crates.io compatibility.
- Added `.cora.yaml` config and pre-commit hook (Cora v0.5.0).

## [0.0.14] — 2026-06-12


### Security

- Set owner-only file permissions (0700/0600) on database and model directories (#134)
- Add SHA256 checksum verification for downloaded ONNX model files (#134)
- Pin expected model checksums to detect corrupted/tampered downloads

### Added

- Indonesian README translation (README.id.md) with language switcher (#277)
- TLS & Reverse Proxy documentation page (Caddy, Nginx, Cloudflare Tunnel) (#100)
- Crates.io metadata in all Cargo.toml files (#136)

### Changed

- **Server now handles requests concurrently** via thread-per-request (#233)
  Uses `Arc<Mutex<Uteke>>` for safe shared access across threads.
- Contradiction threshold is now a parameter instead of hardcoded 0.65 (#253)
- Rename `euclidean_to_cosine` to `cosine_distance_to_similarity` (#232)
- 9 code quality improvements from Cora scan (#232)

## [0.0.13] — 2026-06-10

### Added

- **FTS5 hybrid search with RRF** — Full-text search (FTS5) as parallel retrieval channel merged with vector search via Reciprocal Rank Fusion (RRF, k=60). New `RecallStrategy` enum: `hybrid` (default), `vector`, `fts5`. FTS5 virtual table auto-created; existing DBs get schema migration v1→v2. Phrase search + token-OR fallback. Deprecated memories excluded from FTS5. 6 new tests (#250, PR #261)
- **Metadata enrichment via CLI flags** — `--entity`, `--category`, `--meta key:value,...` on `remember`. Post-filter on `recall` and `list` by `--entity` and `--category`. `parse_meta_pairs()` with auto type detection (string/number/bool). JSON output includes metadata when present (#251, PR #262)
- **Concurrent reads via RwLock** — `Mutex<VectorIndex>` → `RwLock<VectorIndex>` for read-heavy workload. Multiple concurrent recalls share read lock. Embedder remains `Mutex` (ONNX tokenizer requires `&mut self`) (#209, PR #260)

### Fixed

- **Vector index consistency** — Atomic save for `.keys` sidecar file (temp + rename). `insert()` and `build()` now return `Result` for error propagation (#139, PR #263)
- **FTS5 BM25 score conversion** — Negative unbounded BM25 values were always clamped to 0.0. Fixed to proper sigmoid-based normalization (PR #264)
- **RRF normalization** — `.min(1.0)` → `.clamp(0.0, 1.0)` with clearer math (PR #264)
- **`memories.remove().unwrap()`** — Replaced with `.expect()` for meaningful panic message (PR #264)
- **Server-mode metadata support** — `remember` via HTTP API now includes entity, category, and meta in request body (PR #264)
- **Clippy `collapsible_else_if`** — 2 pre-existing warnings fixed (PR #260)

### Changed

- **Repository transferred** — `ajianaz/uteke` → `codecoradev/uteke`. All references updated across 16 files
- **Cora Review CI** — switched from local Infisical OIDC action to `codecoradev/cora-review-action@v1` with GitHub Secrets. Removed `.cora.yaml` project config
- **README simplified** — 400 → 97 lines. Detailed content moved to VitePress docs (`docs/architecture.md`, `docs/cli-reference.md`, etc.)
- **Roadmap cleaned** — consolidated old versions, removed speculative Phase B/C phases
- **CONTRIBUTING.md** — added Cora CLI integration docs, CI checks table, architecture updated to 3 crates, Key Design Decisions section
- **AGENT.md** — new file with persistent AI agent context: critical rules, architecture, lessons learned, proven workflow
- **docs/architecture.md** — new VitePress page with system overview, data flow diagrams, performance benchmarks, design decisions
- **docs/roadmap.md** — v0.0.12 section added, old versions consolidated, "What's Next" list
- **Star History chart** added to README (cora-cli + uteke)

## [0.0.12] — 2026-06-07

### Fixed

- **TOCTOU race in tag operations** — `rename_tag` and `delete_tag` now start transaction before SELECT, preventing lost updates from concurrent writers (#235)
- **TOCTOU race in aging/prune** — `aging_cleanup` and `prune` now delete by specific IDs instead of re-querying by criteria, preventing vector index orphans (#235)
- **bulk_forget_* lock order** — All 3 bulk delete methods now acquire index lock before SQLite delete, matching the pattern from `forget()` (#236)
- **Server 500 leaks internals** — 500 responses now return generic "Internal server error" to client; full error logged server-side (#237)
- **Server JSON fallback** — `json_response` fallback now uses `serde_json::json!` instead of `format!`, preventing broken JSON (#237)
- **Atomic write tmp naming** — Temp files now named `filename.tmp` instead of fragile extension swapping (#238)

### Added

- **`Store::delete_by_ids()`** — New method for atomic batch deletion by specific IDs

## [0.0.11] — 2026-06-07

### Fixed

- **[CRITICAL] Timestamp format mismatch** — Aging/pruning queries never matched because SQLite `datetime('now')` format differs from stored RFC3339 timestamps. Now computes cutoffs in Rust using `chrono` (#221)
- **Namespace=None inconsistency** — Tag operations (`tags_with_counts`, `rename_tag`, `delete_tag`, `count_by_tag`) treated `None` as "default" namespace instead of "all namespaces". Now consistent with `unique_tags` behavior (#222)
- **Non-atomic model file write** — Model downloads now use atomic write (`.tmp` + rename) to prevent corrupt files on crash. Cleans up leftover `.tmp` files on startup (#225)
- **`uteke_home()` panic** — Replaced `.expect()` with `Result` return type to prevent crashes in minimal Docker/CI environments (#226)
- **Server path matching** — `DELETE /forget` now uses exact path match, preventing false matches on `/forgetful` etc. (#228)
- **Query param parsing** — Use `splitn(2, '=')` to preserve values containing `=` (#228)
- **Missing CLI arg value** — `--host`/`--port` without value now prints error instead of silently ignoring (#228)
- **404 path reflection** — Generic "Not found" message instead of echoing request path (#228)
- **SQLite/index inconsistency** — `forget()` now acquires index lock before SQLite delete to narrow the inconsistency window (#231)
- **Memory type validation** — `remember_typed()` now validates `memory_type` against known variants (#229)

### Added

- **Security scanning workflow** — New `security.yml` CI workflow with `cargo audit` + Trivy filesystem scan. Runs on push, PRs, and daily schedule (#177, #220)
- **quinn-proto update** — Updated to v0.11.14 fixing CVE-2026-31812 (DoS via crafted QUIC packet)
- **`Error::Generic` variant** — New error type for general-purpose errors

### Changed

- **`uteke_home()` returns `Result`** — All callers updated to handle potential failure

## [0.0.10] — 2026-06-07

### Fixed

- **Safe slice for deprecated IDs** — `dep_id.get(..8).unwrap_or(dep_id)` prevents panic on short IDs (#192)
- **Index lock before SQLite write** — Acquire vector index lock before any SQLite writes so lock failures are detected early, preventing false errors (#191)
- **HTTP status checking** — Server proxy now validates response status codes, returning proper error messages instead of silently accepting failures (#193)
- **Aging cleanup filter** — `cleanup_aged` now includes `deprecated = 0` filter matching `find_aged` criteria (#189)
- **Schema migration transactions** — Each migration step + version stamp wrapped in SQLite transaction (#188)
- **Batch bulk deletes** — Replace N individual DELETE statements with single batched query for better performance (#190)

### Changed

- **Store module split** — `store.rs` (2,065 LOC) split into 8 focused modules: schema, crud, tags, aging, bulk, vector, types, store (#179)
- **Commands module split** — `commands.rs` (820 LOC) split into 9 per-command modules (#180)
- **SQLite-first dual-write** — `remember()` now writes to SQLite before vector index, matching `forget()` pattern (#182)
- **Embedding docs corrected** — All docs now correctly state 768d (not 256d) for EmbeddingGemma (#183)
- **Shell hook guards** — Bash `PROMPT_COMMAND` and Zsh `chpwd_functions` now have idempotency guards (#143)
- **Hermes branding removed** — All product-specific branding replaced with generic names; only `--namespace` examples remain (#178)

## [0.0.9] — 2026-06-07

### Changed

- **Website migrated to VitePress** — SvelteKit (3,750 LOC, 10 deps) → VitePress (1,300 LOC markdown, 2 deps) (#194)
  - Built-in full-text search (previously missing)
  - Build time: ~15s → ~6s
  - Content now editable via markdown
  - Brand theme (amber/dark) preserved

## [0.0.8] — 2026-06-04

### Added

- **Architecture: module split** — `lib.rs` (1471→352) and `main.rs` (1538→422) broken into focused modules: `operations`, `maintenance`, `consolidate`, `error`, `types`, `import_export`, `commands`, `init`, `output`, `bench`
- **Input validation** — Max content 10K chars, max 20 tags, max server payload 1MB (#132)
- **Binary checksums** — SHA256 checksums in release artifacts + `verify-checksums` subcommand (#134)
- **Schema versioning** — `schema_version` table + migration framework for future DB upgrades (#138)
- **Error handling rewrite** — `Error` enum with sanitized user-friendly messages, ~90 call sites migrated from raw rusqlite/usearch/ONNX errors (#144)
- **Python wrapper expansion** — 7→21 methods covering all CLI commands, namespace support, type hints, Google-style docstrings (#137)
- **Memory benchmark** — `memory-bench` binary for library-level timing across dataset sizes (#49)
- **Memory consolidation** — `consolidate` command to find and merge near-duplicate memories
- **Import/Export** — JSONL-based memory backup and restore via `import` / `export` commands

### Changed

- **Contradiction detection** — Now read-only during check; deprecation happens after new memory is safely persisted (prevents data loss on insert failure) (#139)
- **README** — v0.0.8 badge, Design Philosophy section, Performance benchmarks

### Fixed

- **Deadlock in `check_contradiction`** — Mutex re-acquire pattern fixed by separating read-only check from mutation (#139)

### Security

- **Error sanitization** — Internal error details (file paths, SQL, model names, ONNX internals) no longer exposed to users (#144)

## [0.0.7] — 2026-06-02

### Added

- **Tag storage: `json_each()` queries** — All 8 tag query methods refactored from `LIKE '%\"tag\"%'` to `json_each()` for exact matching and performance (#120)
- **Config wiring: tier thresholds** — `TierConfig` struct with configurable `hot_days`, `warm_days`, `hot_boost`; `Uteke::open_with_tier()` accepts custom config (#127)
- **Test coverage: 34 → 94 tests** — Comprehensive tests for store, lib, and config modules (#129)
- **Config tests** — 7 new tests for `merge_from_file`, `expand_tilde`, `set_namespace_in_toml` (#129)

### Changed

- **`MemoryTier::from_last_accessed()`** — Now accepts `hot_days` and `warm_days` parameters (was hardcoded 7/30)
- **`tags_with_counts()`** — N+1 query pattern replaced with single `GROUP BY` via `json_each()`
- **`unique_tags()`** — SQL returns individual tag values directly (no in-Rust JSON parsing)
- **`tier_counts()` and `bulk_delete_cold()`** — Now accept configurable threshold parameters

### Fixed

- **Tag substring false positives** — Tag `"rust"` no longer matches memory tagged `"rustacean"`
- **README configuration docs** — Fixed config search paths, removed non-existent `--config` flag, corrected TOML format (#128)

## [0.0.6] — 2026-06-02

### Fixed

- **JSON output omits embedding vector** — `Memory.embedding` now uses `#[serde(skip_serializing, default)]`
  - Reduces JSON response size by ~3KB per memory
  - Embeddings are populated programmatically via ONNX, not from JSON
- **`import()` now persists vector index** — previously imported memories were lost on restart because the index was never saved
- **CI: Node.js 24 enforcement** — added `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24` to all workflows
- **Docker: non-root container** — added `USER uteke` directive (uid/gid 1000) with owned `/data` directory
- **CI: removed unused `musl-tools`** install — targets are glibc only

### Added

- **Dependabot** — automated dependency updates for cargo, GitHub Actions, and Docker

## [0.0.5] — 2026-06-01

### Added

- **UTEKE_HOME environment variable** — single env var to override all `dirs::home_dir()` paths
  - Affects: database path (`uteke.db`), vector index (`uteke_index.usearch`), model cache (`models/`)
  - Default: `$HOME/.uteke` when not set
  - Essential for Docker volume mounts and custom data directories
- **Server reads uteke.toml config** — `uteke-serve` now respects configuration file
  - Reads `[server]` section: `host`, `port`
  - Default host changed to `0.0.0.0` (was `127.0.0.1`) for Docker/network compatibility
  - Config loaded at startup, printed to logs
- **Smart server fallback** — CLI auto-falls back to local mode for server-unsupported commands
  - Commands not yet available via HTTP API gracefully fall back to local execution
  - No more error when `server.enabled = true` and command lacks server endpoint
- **API parity — expanded remember endpoint** — `POST /remember` now accepts all CLI fields
  - `memory_type`, `detect_contradiction`, `valid_from`, `valid_until` parameters
  - Returns contradiction detection result when enabled
- **GET /memory endpoint** — retrieve single memory by ID via `GET /memory?id=<id>`
- **DELETE /forget bulk operations** — `DELETE /forget?all=true&cold=true` for mass deletion
- **Multi-stage Dockerfile** — production-ready Docker image for `uteke-serve`
  - Base: `debian:bookworm-slim` (glibc/ONNX compatible)
  - Model baked into image at build time (~208MB total)
  - Non-root user, health check endpoint, configurable via env vars
- **Docker image CI** — automatic build and push to GHCR on release
  - Multi-platform: `linux/amd64` + `linux/arm64`
  - Buildx with cache, tags: `latest` + version tag
- **Release notes from CHANGELOG.md** — dynamic extraction via `awk` (no hardcoded notes)

### Changed

- Server default host: `127.0.0.1` → `0.0.0.0` (Docker/network accessible)
- Cora review action: hardcoded version → `latest` (auto-updates)

### Fixed

- Pre-existing format issue: `.to_string_lossy().to_string()` chain cleaned up

## [0.0.4] — 2026-05-31

### Added

- **Daemon/server mode** — `uteke-serve` for persistent HTTP API (new `uteke-server` crate)
  - Endpoints: `/health`, `/remember`, `/recall`, `/search`, `/list`, `/forget`, `/stats`, `/namespaces`
  - CORS enabled for browser/extension access
  - Graceful shutdown (SIGINT)
  - Warm recall: **~21ms** vs CLI cold start ~980ms (45x faster)
  - Configuration via `[server]` section in `uteke.toml`
- **CLI auto-routes to server** — CLI detects running server and routes commands via HTTP
  - Transparent fallback to local store if server is not running
  - Config: `[server] enabled = true` in `uteke.toml`
  - Latency: recall 21ms, stats 34ms, remember 32ms (via server)
- **Namespace switching & defaults** — `uteke namespace list/stats/switch`
  - Layered resolution: CLI flag > env `UTEKE_NAMESPACE` > config > default
  - Config persistence in `uteke.toml` under `[store]`
  - `uteke namespace switch <name>` sets default namespace
- **Auto-forget & temporal facts** — contradiction detection and time-bounded memories
  - `--detect-contradiction` flag on `remember` — detects conflicting memories (threshold 0.65)
  - `--type` flag: fact, procedure, preference, decision, context
  - `--valid-from` / `--valid-until` for temporal facts
  - `uteke prune --ttl N --dry-run` — remove deprecated/expired memories
  - DB migration: `deprecated`, `valid_from`, `valid_until`, `memory_type` columns
- **Consolidation & deduplication** — `uteke consolidate --threshold 0.90 --dry-run`
  - O(n²) cosine similarity pairwise comparison
  - Merges duplicates: keeps newer memory, removes older
  - `SimilarPair` and `ConsolidationResult` types
- **Bulk operations** — mass delete by tag, cold tier, or all
  - `forget --tag <tag>`, `forget --cold`, `forget --all`
  - Confirmation flags: `--confirm` or `--dry-run`
- **CI: Cora AI code review** — automated PR review via composite action

### Changed

- Version bumped from 0.0.3 → 0.0.4
- Embedding model confirmed: embeddinggemma-q4 (256 dim)
- Contradiction threshold calibrated at 0.65 for small embedding models
- Consolidate default threshold 0.90 (recommend 0.60-0.70 for small models)

### Stress Test Results

| Test Suite | Operations | Result |
|---|---|---|
| CLI cold start (92 ops) | 92/92 | ✅ (avg ~950ms/op) |
| Server warm (112 ops) | 112/112 | ✅ (avg ~35ms/op) |
| Full functional retest | 15 phases | ✅ All pass |

## [0.0.3] — 2026-05-30

### Added

- **Graceful shutdown** — SIGINT (Ctrl+C) handler via `ctrlc` crate
  - Saves usearch index to disk before exit
  - Prevents index corruption on interrupt
- **File logging with daily rotation** — via `tracing-appender`
  - Logs written to `~/.uteke/logs/uteke.log`
  - Automatic daily rotation (`uteke.log.YYYY-MM-DD`)
  - Non-blocking async writer
- **Configuration file** — `uteke.toml` with layered resolution
  - Search order: `./uteke.toml` → parent dirs → `~/.config/uteke/uteke.toml` → defaults
  - Configurable: `store_path`, `log_level`, `log_dir`, `default_namespace`
  - New `--config` flag to override config file path
- **Tag management commands** — `tags list`, `tags rename`, `tags delete`
  - `tags list [--by-count]` — list all tags with usage counts
  - `tags rename <old> <new>` — rename tag across all memories
  - `tags delete <tag>` — remove tag from all memories
- **`--tags` filter for search** — filter search results by tags
  - `uteke search "query" --tags "rust,cli"`
- **Memory aging with auto-cleanup** — `aging status`, `aging preview`, `aging cleanup`
  - `aging status` — show hot/warm/cold/never-accessed breakdown
  - `aging preview --days N` — preview memories older than N days
  - `aging cleanup --days N [--confirm]` — delete stale memories
- **Shell hook for auto-context loading** — `hook install`
  - Supports bash, zsh, fish
  - Walks up from cwd to find `.uteke/uteke.db`
  - Auto-loads project-scoped context on shell init
  - Shell scripts loaded via `include_str!` from canonical files
  - `SupportedShell` enum for parse-time shell validation
- **Node.js 24** — CI upgraded from Node.js 20 → 24

### Changed

- Version bumped from 0.0.2 → 0.0.3

### Stress Test Results (50 memories)

| Phase | Result | Time |
|---|---|---|
| WRITE (50 memories) | 50/50 ✅ | 49.6s (~1.0/s) |
| RECALL (5 queries) | 5/5 ✅ | 4.8s |
| SEARCH (5 queries) | 5/5 ✅ | 4.7s |
| EXPORT/IMPORT | 51/51 ✅ | - |
| TAGS (list/rename/delete) | ✅ | - |
| AGING | ✅ | - |
| VERIFY + DOCTOR | ✅ All pass | - |
| CLEANUP (50 delete) | 50/50 ✅ | 50.2s |

## [0.0.2] — 2026-05-29

### Added

- **Website** — https://github.com/codecoradev/uteke (SvelteKit 5 + Tailwind)
  - Landing page, docs, roadmap
  - Auto-deploy via CF Pages + Infisical OIDC
- **Release matrix** — 4 platforms: Linux x64, Linux ARM64, macOS ARM64, Windows x64
- **Persistent vector index** — replaced in-memory HNSW with usearch (persistent HNSW)
  - Cold start: loads from disk (~5ms) instead of rebuilding from SQLite (~5s at 10K memories)
  - Incremental delete: `remove()` in ~0.1ms instead of full index rebuild
  - Index persisted as `uteke_index.usearch` + `uteke_index.keys` sidecar
  - Auto-migration: builds usearch index from SQLite on first load
- **Multi-agent namespaces** — isolated memory spaces per agent
  - `--namespace` global flag on all commands
  - SQLite `namespace` column with index
  - Auto-migration of existing databases (zero data loss)
  - Each namespace is fully isolated: recall, search, list, stats scoped
  - Default namespace: `"default"` (backward compatible)
- **Tiered memory** — access-based scoring with Hot/Warm/Cold tiers
  - `access_count` and `last_accessed` tracked per memory
  - Hot memories (accessed within 7 days) get +0.1 score boost in recall
  - Warm (30 days) and Cold (>30 days) tiers for visibility
  - `uteke stats` shows tier breakdown: 🔥 Hot / 🟡 Warm / ❄️ Cold
  - Auto-migration: columns added to existing databases
- **Health check commands** — `doctor`, `verify`, `repair`
  - `uteke doctor` — checks SQLite DB, usearch index, embedding model, consistency
  - `uteke verify` — compares DB count vs index count
  - `uteke repair` — rebuilds usearch index from SQLite
  - All support `--json` output

### Changed

- **License:** MIT → Apache 2.0
- **Vector index:** HNSW (in-memory) → usearch (persistent, incremental)
- **Delete:** rebuild-based → incremental `remove()` + save
- **Startup:** rebuild from SQLite → `restore()` from disk
- **Binary size:** 26MB (v0.0.1) → 28MB (v0.0.2, +usearch)
- **CI:** only runs on PR to develop and push to main (eliminates duplicate runs)
- **Release:** versioned artifact filenames (`uteke-{version}-{target}.tar.gz`)
- **CI secrets:** Infisical OIDC for CF Pages deploy (website workflow)

### Removed

- Old deps: `hnsw`, `rand_pcg`, `space` (replaced by `usearch`)
- macOS Intel (`x86_64-apple-darwin`) from release matrix
- Windows ARM64 (`aarch64-pc-windows-msvc`) from release matrix (numkong incompatibility)

### Docs

- **INSTALL.md:** Windows setup guide (pre-built + build from source)
- **CONTRIBUTING.md:** HNSW → usearch references updated
- **README:** architecture table, tiered memory, health check commands

## [0.0.1] — 2026-05-29

### Added

- **Core memory engine** — store, recall, search, forget, list, get operations
- **Semantic search** — vector similarity using HNSW index with cosine scoring
- **ONNX embedding** — EmbeddingGemma Q4 model (768d), auto-downloaded on first run
- **SQLite storage** — embedded database with indexed tags and metadata
- **CLI** — full command-line interface with clap
  - `remember` — store memories with optional tags
  - `recall` — semantic search with `--limit` and tag filter
  - `search` — keyword text search
  - `list` — paginated listing with `--tag` filter
  - `get` — retrieve single memory by ID
  - `forget` — delete memory by ID
  - `stats` — show store statistics
  - `completions` — generate shell completions (bash, zsh, fish)
- **JSON output** — `--json` flag on all commands for machine-readable output
- **Python wrapper** — zero-dependency `UtekeMemory` class (stdlib only, Python 3.8+)
- **Custom store path** — `--store` flag to override default `~/.uteke` location
- **Verbose logging** — `--verbose` flag for debug output
- **CI pipeline** — GitHub Actions with check, fmt, clippy, test, build jobs
- **Workspace structure** — `uteke-core` library + `uteke-cli` binary crates
- **No unsafe code** — `unsafe_code = "forbid"` in workspace lints

### Technical Details

- **Embedding model:** onnx-community/embeddinggemma-300m-ONNX (Q4 quantized, 768 dimensions)
- **Vector index:** HNSW with configurable ef and k parameters
- **Storage:** SQLite via rusqlite (bundled) with WAL mode
- **Tokenization:** HuggingFace tokenizers crate
- **Binary name:** `uteke`
- **Minimum Rust version:** 1.75+

[0.0.13]: https://github.com/codecoradev/uteke/releases/tag/v0.0.13
[0.0.12]: https://github.com/codecoradev/uteke/releases/tag/v0.0.12
[0.0.10]: https://github.com/codecoradev/uteke/releases/tag/v0.0.10
[0.0.9]: https://github.com/codecoradev/uteke/releases/tag/v0.0.9
[0.0.8]: https://github.com/codecoradev/uteke/releases/tag/v0.0.8
[0.0.7]: https://github.com/codecoradev/uteke/releases/tag/v0.0.7
[0.0.6]: https://github.com/codecoradev/uteke/releases/tag/v0.0.6
[0.0.5]: https://github.com/codecoradev/uteke/releases/tag/v0.0.5
[0.0.4]: https://github.com/codecoradev/uteke/releases/tag/v0.0.4
[0.0.3]: https://github.com/codecoradev/uteke/releases/tag/v0.0.3
[0.0.2]: https://github.com/codecoradev/uteke/releases/tag/v0.0.2
[0.0.1]: https://github.com/codecoradev/uteke/releases/tag/v0.0.1
