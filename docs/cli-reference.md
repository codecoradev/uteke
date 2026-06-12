---
title: CLI Reference
---

# CLI Reference

Complete reference for all uteke commands. Version **0.0.13**.

## Global Flags

| Flag | Description | Default |
|------|-------------|---------|
| `--store <path>` | Override store location | `~/.uteke` |
| `--namespace <name>` | Namespace for multi-agent isolation | `default` |
| `--config <path>` | Override config file path | auto-resolved |
| `--json` | Output as JSON | off |
| `--verbose` | Enable debug logging | off |

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

# With memory type and temporal bounds
uteke remember "API rate limit is 1000/min" --type fact --valid-from 2026-01-01 --valid-until 2026-12-31

# In a specific namespace
uteke remember "User prefers dark mode" --tags pref --namespace my-agent
```

| Flag | Description |
|------|-------------|
| `--tags <tags>` | Comma-separated tags |
| `--entity <name>` | Associate memory with an entity (e.g. "staging-server") |
| `--category <cat>` | Categorize the memory (e.g. "infrastructure") |
| `--meta <pairs>` | Key:value pairs, comma-separated. Auto-detects type (string/number/bool) |
| `--metadata <json>` | Arbitrary JSON metadata |
| `--detect-contradiction` | Detect conflicting memories (default threshold: 0.65) |
| `--type <type>` | Memory type: fact, procedure, preference, decision, context |
| `--valid-from <datetime>` | Start of validity period (ISO 8601) |
| `--valid-until <datetime>` | End of validity period (ISO 8601) |
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
| `--limit <n>` | Max results (default: 20) |
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

## uteke aging

Memory aging management with auto-cleanup.

```bash
# Show hot/warm/cold breakdown
uteke aging status

# Preview memories older than 90 days
uteke aging preview --days 90

# Delete memories older than 180 days
uteke aging cleanup --days 180 --confirm
```

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
| `uteke import <file>` | Import memories from JSONL |
| `uteke doctor` | Health check (DB, index, model, consistency) |
| `uteke verify` | Verify DB and index consistency |
| `uteke repair` | Rebuild index from SQLite |
| `uteke namespace list` | List all namespaces with memory counts |
| `uteke namespace stats <name>` | Show stats for a namespace |
| `uteke namespace switch <name>` | Set default namespace in config |
| `uteke hook install <shell>` | Install shell hook (bash/zsh/fish) |
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
