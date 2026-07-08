---
title: CLI Reference
---

# CLI Reference

Complete reference for all uteke commands. Version **0.7.0**.

## Global Flags

| Flag | Description | Default |
|------|-------------|---------|
| `--store <path>` | Override store location | `~/.uteke` |
| `--namespace <name>` | Namespace for multi-agent isolation | `default` |
| `--json` | Output as JSON | off |
| `--verbose` | Enable debug logging | off |

Config file path is auto-resolved (`~/.uteke/uteke.toml` + `.uteke/uteke.toml`); there is no `--config` flag.

## uteke upgrade

Check for updates and upgrade to the latest Uteke release.

```bash
uteke upgrade
uteke upgrade --yes
```

| Flag | Description |
|------|-------------|
| `--yes` | Skip confirmation prompt |

> **Note:** Renamed from `uteke update` in v0.7.0 to avoid conflict with `uteke doc update`. Verifies SHA-256 checksums by default. Set `UTEKE_UPGRADE_SKIP_CHECKSUM=1` to skip verification (not recommended).

## uteke remember

Store a new memory with optional tags, metadata, and contradiction detection.

```bash
# Basic
uteke remember "Deploy v2.1 to staging Friday" --tags deploy,staging

# With metadata enrichment
uteke remember "Deploy staging to AWS us-east-1" \
  --tags deploy,aws \
  --entity staging-server \
  --category infrastructure \
  --meta "source:meeting-note,confidence:0.9"

# With contradiction detection
uteke remember "Server runs on port 8080" --tags config --detect-contradiction

# With memory type
uteke remember "API rate limit is 1000/min" --type fact

# With source attribution (#348)
uteke remember "Deploy at 3pm Friday" --tags deploy --source "https://slack.com/msg/123" --source-type url

# In a specific namespace
uteke remember "User prefers dark mode" --tags pref --namespace my-agent
```

| Flag | Description |
|------|-------------|
| `--tags <tags>` | Comma-separated tags |
| `--entity <name>` | Associate memory with an entity (e.g. "staging-server") |
| `--category <cat>` | Categorize the memory (e.g. "infrastructure") |
| `--meta <pairs>` | Key:value pairs, comma-separated. Auto-detects type (string/number/bool) |
| `--type <type>` | Memory type: fact, procedure, preference, decision, context, note, insight, reference, event |
| `--source <url-or-path>` | Source attribution (URL, file path, or description) |
| `--source-type <type>` | Source type: user, url, file, import, derived, system (default: user) |
| `--detect-contradiction` | Detect conflicting memories (default threshold: 0.65) |
| `--room <room_id>` | Link memory to a room (collaborative context) |
| `--author <name>` | Author attribution when storing in a room |
| `--json` | Output stored memory as JSON |

## uteke recall

Unified search combining vector similarity with FTS5 full-text search, ranked by Reciprocal Rank Fusion (RRF). Searches **both memories and documents** by default (unified mode). Hot memories (accessed within 7 days) get a score boost.

```bash
uteke recall "What framework does the API use?"
uteke recall "deployment" --limit 10
uteke recall "database config" --namespace hermes --json
uteke recall "server" --entity staging-server --json
uteke recall "config" --category infrastructure --limit 5
# Search documents only
uteke recall "API architecture" --type doc
# Search memories only (backward compatible)
uteke recall "deployment" --type memory
```

| Flag | Description |
|------|-------------|
| `--type <type>` | Search scope: `all` (default — memories + documents, unified via RRF), `memory` (memories only), `doc` (documents only) |
| `--limit <n>` | Max results (default: 5) |
| `--entity <name>` | Filter results to a specific entity |
| `--category <cat>` | Filter results to a specific category |
| `--content-format <fmt>` | Content display: `auto` (detect), `text`, `json` (pretty-print JSON memories) |
| `--where <key=value>` | Filter by JSON field on structured memories (e.g. `--where role=CTO`) |
| `--json` | Output as JSON array |

## uteke search

Keyword text search with tag filtering.

```bash
uteke search "monorepo"
uteke search "deploy" --tags staging,prod --limit 20
uteke search "api" --namespace backend --json
```

| Flag | Description |
|------|-------------|
| `--tags <tags>` | Filter by comma-separated tags |
| `--limit <n>` | Max results (default: 10) |
| `--json` | Output as JSON |

## uteke list

List memories with optional tag, entity, category filter and pagination.

```bash
uteke list --limit 20
uteke list --tag deploy --offset 10 --json
uteke list --entity staging-server --json
uteke list --category infrastructure --limit 10
uteke list --namespace hermes
```

| Flag | Description |
|------|-------------|
| `--tag <tag>` | Filter by single tag |
| `--entity <name>` | Filter by entity name |
| `--category <cat>` | Filter by category |
| `--limit <n>` | Max results (default: 20) |
| `--offset <n>` | Skip first N results |
| `--json` | Output as JSON |

## uteke forget

Delete memories by ID, tag, tier, or all. Supports bulk operations.

```bash
# Delete single memory
uteke forget <uuid> --confirm

# Delete all with a tag
uteke forget --tag deploy --confirm

# Delete all cold-tier memories (>30 days old)
uteke forget --cold --confirm

# Delete everything in namespace
uteke forget --all --confirm

# Preview without deleting
uteke forget --tag stale --dry-run
```

| Flag | Description |
|------|-------------|
| `--tag <tag>` | Delete all memories with this tag |
| `--cold` | Delete all cold-tier memories (>30 days) |
| `--all` | Delete all memories in namespace |
| `--confirm` | Skip confirmation prompt |
| `--dry-run` | Preview what would be deleted |

## uteke consolidate

Find and merge duplicate memories using cosine similarity.

```bash
# Preview duplicates (dry run)
uteke consolidate --threshold 0.60 --dry-run

# Merge duplicates
uteke consolidate --threshold 0.60

# Higher threshold = more conservative (default: 0.90)
uteke consolidate --dry-run
```

Recommended threshold: **0.60–0.70** for small embedding models (embeddinggemma-q4). Keeps newer memory, removes older.

## uteke prune

Remove deprecated and expired temporal memories.

```bash
# Preview what would be pruned
uteke prune --ttl 30 --dry-run

# Prune deprecated/expired memories
uteke prune --ttl 30
```

## uteke namespace

Manage namespaces — list, inspect, and switch defaults.

```bash
# List all namespaces with counts
uteke namespace list

# Show stats for a namespace
uteke namespace stats my-agent

# Switch default namespace (saved to config)
uteke namespace switch my-agent
```

Namespace resolution order: `--namespace flag` → `UTEKE_NAMESPACE` env → `uteke.toml` → `"default"`

## uteke tags

Manage tags across all memories.

```bash
# List all tags with counts
uteke tags list --by-count

# Rename a tag
uteke tags rename old-tag new-tag

# Delete a tag from all memories
uteke tags delete unused-tag
```

## uteke recall (enhanced)

Semantic search with new flags:

```bash
# Minimum similarity score filter
uteke recall "database config" --min 0.7
uteke recall "database config" --strict    # uses min_score_strict (default 0.5)

# Time-travel: query memories at specific point in time
uteke recall "deployment process" --at 2026-06-01T12:00:00Z

# Relationship graph traversal
uteke recall "auth" --related --depth 2

# Recall strategy (vector | fts5 | hybrid | graph)
uteke recall "auth" --strategy vector   # default — vector similarity only
uteke recall "auth" --strategy fts5     # full-text search only
uteke recall "auth" --strategy hybrid   # vector + FTS5 (RRF fusion)
uteke recall "auth" --strategy graph    # hybrid + graph-signal reranking (#378)
```

### `--strategy graph` (Graph-augmented RAG)

The `graph` strategy runs the hybrid (RRF) pipeline, then fuses graph
signals from the `memory_edges` table into each result's score. A memory
that is well-connected in the graph (referenced by many others, high edge
density) drifts upward; isolated memories are untouched.

The boost is **additive and log-scaled**, so it saturates quickly and never
lets a single hub dominate. Configure the weights under `[recall]` in
`uteke.toml` (see [Configuration](./configuration#recall)):

```bash
# Subtle boost (default): well-connected memories nudge up slightly
uteke recall "architecture" --strategy graph

# Stronger authority boost via env var
UTEKE_GRAPH_AUTHORITY_WEIGHT=0.3 uteke recall "architecture" --strategy graph
```

Cold start (no edges yet): `graph` behaves identically to `hybrid` — the
boost is zero when there are no signals.

# AI-context formatted output
uteke recall "api design" --context
```

| Flag | Description |
|------|-------------|
| `--min <score>` | Minimum similarity score (0.0-1.0) |
| `--strict` | Use strict threshold (`min_score_strict`, default 0.5) |
| `--at <timestamp>` | Query memories at point in time (RFC3339) |
| `--related` | Follow relationship edges |
| `--depth <n>` | Traversal depth for --related |
| `--context` | AI-prompt formatted output |
| `--salience` | Enable salience boost (higher score for decision/insight types) |
| `--recency` | Enable recency boost (higher score for recently created memories) |

## uteke list (enhanced)

```bash
# Time-travel list
uteke list --at 2026-06-01T12:00:00Z
```

## uteke room

Room-based memory management:

```bash
# Create a room
uteke room create "project-kickoff" --title "Project Kickoff"

# List rooms (cross-namespace by default, #392)
uteke room list
uteke room list --namespace my-agent  # scoped

# Add memory to room
uteke room add "project-kickoff" <memory-id> --author cto

# Recall within a room (semantic)
uteke room recall "project-kickoff" --query "database decision"

# Generate structured document from room
uteke room document "project-kickoff"

# Room summary (LLM-free, tag clustering)
uteke room summary "project-kickoff"

# Remove memory from room
uteke room remove "project-kickoff" <memory-id>

# Delete room
uteke room delete "project-kickoff"
```

## uteke bench

Run performance benchmarks with synthetic data:

```bash
# Default: 100, 1000, 10000 memories
uteke bench

# Custom counts
uteke bench --counts 500,5000

# JSON output
uteke bench --json
```

Measures insert throughput, recall latency (avg + p95), and storage footprint.

## uteke pin / unpin

Pin memories so they never decay:

```bash
uteke pin <id>
uteke unpin <id>
```

## uteke importance

Recalculate importance scores:

```bash
uteke importance
```

## uteke aging

Memory aging management with auto-cleanup.

```bash
# Show hot/warm/cold breakdown
uteke aging status

# Preview memories older than 90 days
uteke aging preview --older-than-days 90

# Delete memories older than 180 days
uteke aging cleanup --older-than-days 180 --yes
```

| Subcommand | Flags | Description |
|------------|-------|-------------|
| `status` | — | Show hot/warm/cold/never-accessed counts |
| `preview` | `--older-than-days N` (default 180), `--max-access-count N` (default 1) | Dry-run preview of cleanup candidates |
| `cleanup` | `--older-than-days N` (default 180), `--max-access-count N` (default 1), `--yes` | Delete aged memories (`--yes` skips confirmation) |

## uteke import

Import memories from a file (or stdin with `-`). Format is auto-detected from
the extension and content; override with `--format`.

```bash
# Import JSONL exported by `uteke export`
uteke import memories.jsonl

# Import markdown/plain text (split into chunks)
uteke import notes.md --tags imported,notes
```

### LLM fact extraction (`--extract`)

Raw source material (chat transcripts, long notes, exported dumps) is noisy:
greetings, filler, boilerplate. Importing it verbatim pollutes recall. With
`--extract`, uteke first sends the text to an OpenAI-compatible chat-completions
endpoint, asks the model to distill it into atomic facts, and stores one memory
per fact.

This is opt-in. Without `--extract`, import makes no network calls and stays
fully offline.

```bash
# Distill a transcript into atomic facts before storing
uteke import session.txt --extract \
  --extract-model gpt-4o-mini \
  --extract-base-url https://api.openai.com/v1 \
  --extract-api-key sk-...

# Use a local Ollama model
uteke import notes.md --extract \
  --extract-model llama3.1 \
  --extract-base-url http://localhost:11434/v1
```

Settings resolve in this order: CLI flag > `UTEKE_EXTRACTION_*` env var >
`[extraction]` config section > built-in default. The API key falls back to the
embedding / `OPENAI_API_KEY` credential, so an existing OpenAI-compatible setup
needs no duplicate key.

| Flag | Description |
|------|-------------|
| `--extract` | Enable LLM extraction (off by default) |
| `--extract-model <M>` | Override the chat model |
| `--extract-base-url <U>` | Override the endpoint base URL |
| `--extract-api-key <K>` | Override the API key |
| `--extract-max-facts <N>` | Cap facts kept per document (0 = default) |

See [configuration](#extraction) for the `[extraction]` config block and
`UTEKE_EXTRACTION_*` environment variables.

### Batch import (`--batch-dir`)

Import all files from a directory in one command. Each file is processed
according to its type:

| Extension | Strategy | Description |
|-----------|----------|-------------|
| `.md` / `.markdown` | Document (default) | Full content → auto-chunk → embed. No LLM. |
| `.md` + `--extract` | Memory extract | LLM fact extraction → atomic facts → embed. |
| `.txt` / `.jsonl` | Memory extract | LLM fact extraction (requires `--extract`). |
| `.txt` / `.jsonl` + `--as-doc` | Document override | Import as document, skip LLM. |

```bash
# Dry run — preview what would be imported
uteke import --batch-dir ./knowledge --dry-run

# Import all markdown as documents (no LLM, fully offline)
uteke import --batch-dir ./docs --recursive --tags docs

# Import with LLM fact extraction
uteke import --batch-dir ./notes --extract \
  --extract-model gpt-4o-mini \
  --extract-base-url https://api.openai.com/v1 \
  --tags imported

# Force all files as documents (skip extraction)
uteke import --batch-dir ./raw --as-doc --recursive

# Limit file size (default: 1MB)
uteke import --batch-dir ./large-docs --max-size 500000
```

| Flag | Description |
|------|-------------|
| `--batch-dir <PATH>` | Directory to import (enables batch mode) |
| `--recursive` | Recurse into subdirectories |
| `--as-doc` | Force all files as documents (skip LLM extraction) |
| `--as-memory` | Force all files through LLM extraction |
| `--max-size <BYTES>` | Skip files larger than N bytes (default: 1,000,000) |
| `--dry-run` | Preview files and strategies without importing |

## uteke graph

Knowledge graph operations (v0.2.0). Nodes and edges stored in SQLite (`graph_nodes`, `graph_edges` tables, schema v7).

```bash
# List all nodes (optionally filter by entity type)
uteke graph nodes
uteke graph nodes --entity-type person

# List all edges (optionally filter by relation)
uteke graph edges --relation owns

# Find neighbors of a node via BFS
uteke graph neighbors alice --depth 2

# Shortest path between two nodes
uteke graph path alice "project-x" --max-depth 5

# Query edges by relation type
uteke graph query part_of

# Show graph statistics
uteke graph stats
```

| Subcommand | Flags | Description |
|------------|-------|-------------|
| `nodes` | `--entity-type <t>` | List graph nodes |
| `edges` | `--relation <r>` | List graph edges |
| `neighbors <label>` | `--depth N` (default 1) | BFS neighbors of a node |
| `path <source> <target>` | `--max-depth N` (default 5) | BFS shortest path |
| `query <relation>` | — | Query edges by relation type |
| `stats` | — | Show graph statistics |

## uteke edges

List auto-wired edges for a memory (v0.2.1, #346). Edges are auto-extracted from content on every `remember()` call using pure pattern matching — no LLM.

### Supported patterns

| Pattern | Edge type | Resolved via |
|---------|-----------|--------------|
| `[[slug]]` | `references` | `memories.slug` lookup |
| `@tag` | `tagged_as` | most recent memory with that tag |
| `^<uuid>` | `supersedes` | direct memory UUID |
| `><uuid>` | `replies_to` | direct memory UUID |
| `rel:<type>:<uuid>` in `--meta` | `<type>` | direct memory UUID (legacy compat) |

### Usage

```bash
# List direct edges (both directions)
uteke edges <memory-id>

# Multi-hop BFS across the edge table
uteke edges <memory-id> --deep 2

# Only show incoming edges (backlinks)
uteke edges <memory-id> --direction incoming

# JSON output
uteke edges <memory-id> --json
```

With `--deep N`, returns memory ids reachable within N hops (cycles detected, start excluded).

`--direction` accepts `incoming`, `outgoing`, or `both` (default `both`).

## uteke rebuild-backlinks

Rebuild `referenced_by` backlinks from existing forward edges (v0.2.1, #350).

Every forward edge (`references`, `tagged_as`, `supersedes`, `replies_to`)
automatically gets an inverse `referenced_by` edge on `remember()`. This
command repairs stores that pre-date #350, or were written to via the
low-level edge API. Idempotent.

```bash
# Rebuild and print a summary
uteke rebuild-backlinks

# Print only the count of new backlinks (script-friendly)
uteke rebuild-backlinks --quiet

# JSON output
uteke rebuild-backlinks --json
```

## uteke dream

Run the full maintenance pipeline in one command (v0.2.1, #353).

Executes phases in dependency order: lint → backlinks → dedup → orphans → compact → verify.
Each phase records its status. Errors in individual phases are recorded but do not abort the pipeline.

```bash
# Run all phases
uteke dream

# Run specific phases only
uteke dream --phases lint,orphans

# Dry-run (preview what would change)
uteke dream --dry-run

# Scoped to a namespace
uteke dream --namespace my-agent
```

| Flag | Description |
|------|-------------|
| `--phases <list>` | Comma-separated subset: lint, backlinks, dedup, orphans, compact, verify |
| `--dry-run` | Preview without making changes |
| `--namespace <ns>` | Run scoped to a specific namespace (backlinks and verify are global) |
| `--json` | JSON output |

### Phases

| Phase | Description |
|-------|-------------|
| `lint` | Check for invalid memory types, missing slugs, stale deprecated flags |
| `backlinks` | Rebuild `referenced_by` edges (same as `rebuild-backlinks`) |
| `dedup` | Find and merge near-duplicate memories (cosine ≥ 0.90) |
| `orphans` | Find disconnected, low-importance memories |
| `compact` | Apply auto-prune to cold-tier and deprecated memories |
| `verify` | Verify DB and index consistency |

## uteke orphans

Find orphan memories — disconnected nodes with low importance and few accesses (v0.2.1, #351).

```bash
# Find orphans with default thresholds
uteke orphans

# Custom thresholds
uteke orphans --min-age-days 14 --max-access-count 3

# JSON output for scripting
uteke orphans --json
```

| Flag | Description |
|------|-------------|
| `--min-age-days <n>` | Minimum age in days (default: 7) |
| `--max-access-count <n>` | Maximum access count (default: 2) |
| `--limit <n>` | Max results (default: 50) |
| `--namespace <ns>` | Scope to namespace |
| `--json` | JSON output |

## uteke timeline

View chronological event log for a memory (v0.2.1, #347).

Every memory has an audit trail: creation, updates, type changes, supersession,
edge additions, and pin/unpin events.

```bash
# Show timeline for a memory
uteke timeline <memory-id>

# Limit to last N events
uteke timeline <memory-id> --limit 10

# JSON output
uteke timeline <memory-id> --json
```

| Flag | Description |
|------|-------------|
| `--limit <n>` | Max events (default: 50) |
| `--json` | JSON output |

## Other Commands

| Command | Description |
|---------|-------------|
| `uteke get <id>` | Retrieve a single memory by UUID |
| `uteke forget <id>` | Delete a memory by UUID |
| `uteke forget --tag <tag>` | Delete all memories with a tag |
| `uteke forget --cold` | Delete all cold-tier memories |
| `uteke forget --all` | Delete all memories in namespace |
| `uteke consolidate` | Find and merge duplicate memories |
| `uteke prune` | Remove deprecated/expired memories |
| `uteke stats` | Show store statistics with tier breakdown |
| `uteke export` | Export memories to JSONL (no embeddings) |
| `uteke import <file>` | Import memories from JSONL/Markdown/text (`--extract` distills with an LLM) |
| `uteke doctor` | Health check (DB, index, model, consistency) |
| `uteke verify` | Verify DB and index consistency |
| `uteke verify-checksums --binary <path>` | Verify binary integrity against SHA256 checksums |
| `uteke repair` | Rebuild index from SQLite |
| `uteke namespace list` | List all namespaces with memory counts |
| `uteke namespace stats <name>` | Show stats for a namespace |
| `uteke namespace switch <name>` | Set default namespace in config |
| `uteke hook <shell>` | Print shell hook script (bash/zsh/fish) |
| `uteke init --agent <type>` | Initialize integration (opencode, pi, claude, cursor, hermes) |
| `uteke init --agent hermes --memory-provider` | Install uteke as Hermes's default memory provider (auto recall + extraction). See [Hermes integration, Mode B](integrations/hermes.md). **Note:** Hermes Mode B deprecated — see [integrations/hermes.md](integrations/hermes.md) for migration. |
| `uteke init --agent pi --memory-provider` | Install uteke as pi's default memory provider (auto recall + extraction, #575/#577) |
| `uteke init --agent claude --memory-provider` | Install uteke as Claude Code's default memory provider (auto recall + extraction, #575/#577) |
| `uteke init --agent cursor --memory-provider` | Install uteke as Cursor's default memory provider (auto recall + extraction, #575/#577) |
| `uteke init --agent opencode` | Generate AGENTS.md with uteke instructions for OpenCode (#612) |
| `uteke completions <shell>` | Generate shell completions |

## uteke-serve (Server Mode)

Start a persistent HTTP server for fast AI agent access. Eliminates cold start (~980ms → ~21ms per operation).

```bash
# Start server on default port (8767)
uteke-serve

# Custom port
uteke-serve --port 9000

# With logging
RUST_LOG=info uteke-serve --port 8767
```

When `[server] enabled = true` is set in config, the CLI auto-routes commands through the server. Falls back to local store if server is not running.

### HTTP Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Health check |
| POST | `/remember` | Store a memory |
| POST | `/recall` | Semantic search |
| POST | `/search` | Keyword search |
| POST | `/list` | List memories |
| DELETE | `/forget` | Delete memory |
| GET | `/stats` | Store statistics (supports `?namespace=`) |
| POST | `/stats` | Store statistics (body params) |
| GET | `/namespaces` | List all namespaces |
| GET | `/memory?id=` | Get single memory |
| POST | `/room/create` | Create a room |
| GET | `/room/list` | List rooms (supports `?namespace=`) |
| POST | `/room/recall` | Recall from a room |
| POST | `/room/summary` | Room summary |
| POST | `/room/document` | Generate document from room |
| POST | `/room/stats` | Room statistics |
| DELETE | `/room/delete` | Delete a room |
| POST | `/doc/create` | Create/upsert document (#438) |
| POST | `/doc/get` | Get document by id or slug |
| POST | `/doc/list` | List documents (roots_only, parent filter) |
| POST | `/doc/search` | Hybrid/semantic/FTS document search (#440) |
| POST | `/doc/move` | Move document to new parent (#438) |
| DELETE | `/doc/delete?id=` | Delete document with cascade |
| POST | `/doc/update` | Partial document update with chunk rebuild (#589) |
| GET | `/recent` | Recent memories (supports `?namespace=`, `?limit=`, `?offset=`) (#528) |
| GET | `/tags` | List all tags with counts (#566) |
| POST | `/tags/rename` | Rename a tag across all memories (#566) |
| POST | `/tags/delete` | Delete a tag from all memories (#566) |
| POST | `/pin` | Pin a memory (prevent decay) (#566) |
| POST | `/unpin` | Unpin a memory (#566) |
| GET | `/timeline` | Timeline events for a memory (#566) |
| GET | `/edges` | List edges for a memory (#566) |
| GET | `/room/memories` | List memories in a room (#569) |
| GET | `/graph` | Graph visualization — nodes, edges, stats (#408) |
| POST | `/graph/edge` | Create a typed edge between two memories (#542) |
| DELETE | `/graph/edge` | Remove an edge by ID (#542) |
| POST | `/mcp` | MCP JSON-RPC endpoint (#381) |

### MCP Server

Two MCP transport modes are available (v0.2.1, #381):

```bash
# stdio transport (for Claude Desktop, Cursor, etc.)
uteke-mcp

# HTTP transport (POST /mcp on uteke-serve)
curl -X POST http://127.0.0.1:8767/mcp \
  -H "Content-Type: application/json" \
  -H "MCP-Protocol-Version: 2025-06-18" \
  -d '{"jsonrpc":"2.0","method":"initialize","params":{},"id":1}'
```

Protocol version: `2025-06-18` (Streamable HTTP spec).

## Document Commands (#406, #411, #438, #440)

Wiki/knowledge base engine — hierarchical documents with auto-chunking, tree operations, and hybrid search.

### `uteke doc create`

Create or update a document from file, content, or stdin.

```bash
# From file
uteke doc create architecture --file guide.md --tags wiki,tech

# From inline content
uteke doc create notes --content "# Notes\n\nQuick notes" --title "My Notes"

# From stdin
cat README.md | uteke doc create readme --file -

# As child of another document
uteke doc create chapter1 --parent book --file ch1.md
```

| Flag | Description |
|------|-------------|
| `--title <title>` | Document title (auto-derived from first `#` heading if omitted) |
| `--file <path>` | Read content from file (`-` for stdin) |
| `--content <text>` | Inline content (alternative to `--file`) |
| `--tags <tags>` | Comma-separated tags |
| `--parent <slug>` | Parent document slug (creates as child, #438) |

### `uteke doc get`

Get a document by slug or ID.

```bash
uteke doc get architecture
uteke doc get architecture --json
```

### `uteke doc list`

List documents in the current namespace.

```bash
uteke doc list --limit 20
uteke doc list --tree          # Indented tree view
uteke doc list --roots-only   # Root documents only
```

| Flag | Description |
|------|-------------|
| `--limit <n>` | Max results (default: 20) |
| `--tree` | Show as indented tree (recursive children) |
| `--roots-only` | Show only root documents (no parent) |

### `uteke doc search`

Search documents using hybrid, semantic, or FTS5 search (#440).

```bash
uteke doc search "authentication flow"
uteke doc search "database" --mode semantic
uteke doc search "API reference" --mode fts --limit 5
```

| Flag | Description |
|------|-------------|
| `--mode <mode>` | `hybrid` (default), `semantic`, or `fts` |
| `--limit <n>` | Max results (default: 5) |

### `uteke doc update`

Partially update a document — only provided fields are changed (#589). Content changes trigger chunk rebuild; metadata-only or title-only updates skip it.

```bash
# Update title only (no chunk rebuild)
uteke doc update architecture --title "New Architecture Guide"

# Update content from file (triggers chunk rebuild)
uteke doc update architecture --file updated-guide.md

# Update content from stdin
echo "# New content" | uteke doc update architecture --file -

# Replace tags
uteke doc update architecture --tags wiki,tech,v2

# Replace metadata
uteke doc update architecture --metadata '{"draft": false, "reviewer": "CTO"}'

# Multiple fields at once
uteke doc update architecture --title "Final Guide" --tags wiki,published
```

| Flag | Description |
|------|-------------|
| `--title <title>` | New title (no chunk rebuild) |
| `--content <text>` | New content (triggers chunk rebuild) |
| `--file <path>` | Read new content from file (`-` for stdin; triggers chunk rebuild) |
| `--tags <tags>` | Replace tags (comma-separated) |
| `--metadata <json>` | Replace metadata (JSON string) |

At least one field is required. Omitted fields are left unchanged.

### `uteke doc move`

Move a document to a new parent or root (#438).

```bash
uteke doc move chapter1 --parent new-book
uteke doc move orphan-doc               # Move to root
```

### `uteke doc delete`

Delete a document by ID or slug (cascades to children and chunks).

```bash
uteke doc delete architecture
uteke doc delete <id>
```

### `uteke doc export`

Export all documents as JSON or markdown.

```bash
uteke doc export           # markdown to stdout
uteke doc export --json    # JSON to stdout
```

## Graph API (#408, #542)

### Visualization

```bash
# All nodes + edges + stats
curl http://127.0.0.1:8767/graph

# Response: { nodes: [...], edges: [...], stats: {...} }
```

### Mutation

```bash
# Create a typed edge
curl -X POST http://127.0.0.1:8767/graph/edge \
  -H "Content-Type: application/json" \
  -d '{"from": "<uuid>", "to": "<uuid>", "edge_type": "references"}'

# Delete an edge by ID
curl -X DELETE "http://127.0.0.1:8767/graph/edge?id=<edge-uuid>"
```

## Server Authentication (#409)

Dual-role API token model:

| Flag / Env | Role | Access |
|------------|------|--------|
| `--auth-token` / `UTEKE_AUTH_TOKEN` | Admin | All endpoints (GET, POST, DELETE) |
| `--read-only-token` / `UTEKE_READ_ONLY_TOKEN` | ReadOnly | All read endpoints (GET + POST search/list/graph). 403 on writes. |

```bash
# Start with dual tokens
uteke-serve --auth-token admin-secret --read-only-token viewer-key

# Read-only client (GET only)
curl -H "Authorization: Bearer viewer-key" http://127.0.0.1:8767/stats

# Write attempt fails
curl -X POST -H "Authorization: Bearer viewer-key" ...  # 403 Forbidden
```

## Configurable Limits (#404)

All limits can be overridden via env vars or `[limits]` config section:

| Env Var | Config Key | Default | Description |
|---------|-----------|---------|-------------|
| `UTEKE_MAX_CONTENT_LENGTH` | `max_content_length` | 100000 | Max memory content (chars, 0=disable) |
| `UTEKE_MAX_TAGS_COUNT` | `max_tags_count` | 20 | Max tags per memory |
| `UTEKE_MAX_TAG_LENGTH` | `max_tag_length` | 50 | Max single tag length (chars) |
| `UTEKE_MAX_PAYLOAD_SIZE` | `max_payload_size` | 10485760 | Max server payload (bytes) |
| `UTEKE_DEFAULT_RECALL_LIMIT` | `default_recall_limit` | 5 | Default recall limit |

## Changelog

### v0.6.5
- Room fixes (#545/#546/#547/#549/#550): Fixed room creation race conditions, room memory list pagination, room summary edge cases, room document generation for empty rooms, and room delete cascade consistency.

### v0.6.6
- Room summary Unicode fix (#565): Summary truncation now uses char boundaries instead of byte offsets, preventing mid-character cuts in multi-byte content.
