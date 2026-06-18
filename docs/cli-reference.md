---
title: CLI Reference
---

# CLI Reference

Complete reference for all uteke commands. Version **0.2.0**.

## Global Flags

| Flag | Description | Default |
|------|-------------|---------|
| `--store <path>` | Override store location | `~/.uteke` |
| `--namespace <name>` | Namespace for multi-agent isolation | `default` |
| `--json` | Output as JSON | off |
| `--verbose` | Enable debug logging | off |

Config file path is auto-resolved (`~/.uteke/uteke.toml` + `.uteke/uteke.toml`); there is no `--config` flag.

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

# In a specific namespace
uteke remember "User prefers dark mode" --tags pref --namespace my-agent
```

| Flag | Description |
|------|-------------|
| `--tags <tags>` | Comma-separated tags |
| `--entity <name>` | Associate memory with an entity (e.g. "staging-server") |
| `--category <cat>` | Categorize the memory (e.g. "infrastructure") |
| `--meta <pairs>` | Key:value pairs, comma-separated. Auto-detects type (string/number/bool) |
| `--type <type>` | Memory type: fact, procedure, preference, decision, context |
| `--detect-contradiction` | Detect conflicting memories (default threshold: 0.65) |
| `--room <room_id>` | Link memory to a room (collaborative context) |
| `--author <name>` | Author attribution when storing in a room |
| `--json` | Output stored memory as JSON |

## uteke recall

Hybrid search combining vector similarity with FTS5 full-text search, ranked by Reciprocal Rank Fusion (RRF). Hot memories (accessed within 7 days) get a score boost.

```bash
uteke recall "What framework does the API use?"
uteke recall "deployment" --limit 10
uteke recall "database config" --namespace hermes --json
uteke recall "server" --entity staging-server --json
uteke recall "config" --category infrastructure --limit 5
```

| Flag | Description |
|------|-------------|
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

# List rooms
uteke room list

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

# JSON output
uteke edges <memory-id> --json
```

With `--deep N`, returns memory ids reachable within N hops (cycles detected, start excluded).

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
| `uteke import <file>` | Import memories from JSONL/Markdown/text |
| `uteke doctor` | Health check (DB, index, model, consistency) |
| `uteke verify` | Verify DB and index consistency |
| `uteke verify-checksums --binary <path>` | Verify binary integrity against SHA256 checksums |
| `uteke repair` | Rebuild index from SQLite |
| `uteke namespace list` | List all namespaces with memory counts |
| `uteke namespace stats <name>` | Show stats for a namespace |
| `uteke namespace switch <name>` | Set default namespace in config |
| `uteke hook <shell>` | Print shell hook script (bash/zsh/fish) |
| `uteke init --agent <type>` | Initialize integration (pi, claude, cursor, copilot, codex) |
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
