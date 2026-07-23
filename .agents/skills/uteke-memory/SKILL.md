---
description: "Persistent memory engine for AI agents via the uteke CLI — remember, recall, search, forget with semantic + FTS5 hybrid search, documents, knowledge graph, rooms, tiered memory, and multi-agent namespaces."
---

# Uteke Memory Skill

Persistent memory engine for AI agents via the `uteke` CLI.
Version: **0.10.0** — SQLite + usearch HNSW + FTS5 hybrid search (RRF k=60). Zero unsafe code.

> **Hermes integration:** Install the `uteke-memory` plugin for automatic recall
> on every turn via the `pre_llm_call` hook. No shell hook or daemon needed.
> See `extensions/hermes-memory-provider/` for the plugin source.
> Manual tool calls via `uteke-tool` plugin remain available for explicit
> remember/forget/room operations.

## Global Flags (all commands)

| Flag | Description |
|------|-------------|
| `--store <PATH>` | Override store path (default: `~/.uteke`) |
| `--namespace <NS>` | Multi-agent isolation (default: `"default"`) |
| `--json` | Machine-readable JSON output |
| `--verbose` | Debug logging |

## Commands

### Core Memory Operations

| Command | Description | Key Options |
|---------|-------------|-------------|
| `uteke remember <TEXT>` | Store a new memory | `--tags`, `--type`, `--entity`, `--category`, `--meta`, `--room`, `--author`, `--source`, `--source-type`, `--detect-contradiction` |
| `uteke recall <QUERY>` | Hybrid search (vector + FTS5, RRF) | `--limit`, `--tags`, `--entity`, `--category`, `--min`, `--strategy` (vector/fts5/hybrid/graph), `--salience`, `--recency`, `--related`, `--depth`, `--context`, `--at` (time-travel), `--type` (all/memory/doc), `--where` (JSON field filter) |
| `uteke search <QUERY>` | Keyword text search | `--limit`, `--tags` |
| `uteke list` | List memories with filters | `--tag`, `--entity`, `--category`, `--limit`, `--offset`, `--at` |
| `uteke get <ID>` | Get single memory by UUID | |
| `uteke forget <ID>` | Delete memory by ID, tag, tier, or all | `--tag`, `--cold`, `--all`, `--confirm` |

### Documents (Wiki / Knowledge Base)

| Command | Description | Key Options |
|---------|-------------|-------------|
| `uteke doc create <SLUG>` | Create document from file/stdin | `--title`, `--file`, `--content`, `--tags`, `--parent` |
| `uteke doc get <ID_OR_SLUG>` | Get a document | |
| `uteke doc update <ID_OR_SLUG>` | Partial update — only provided fields change | `--title`, `--content`, `--file`, `--tags`, `--metadata` |
| `uteke doc delete <ID>` | Delete document (cascades to children) | |
| `uteke doc list` | List documents | `--limit`, `--tree` (hierarchy) |
| `uteke doc search <QUERY>` | Search documents (semantic + FTS5) | `--limit`, `--mode` (semantic/fts/hybrid) |
| `uteke doc children <PARENT>` | List child documents | `--limit` |
| `uteke doc move <ID_OR_SLUG>` | Move to new parent or root | `--parent` |
| `uteke doc breadcrumbs <ID_OR_SLUG>` | Show path from root | |
| `uteke doc descendants <ID_OR_SLUG>` | List all descendants | `--max-depth`, `--limit` |
| `uteke doc export` | Export all documents as JSON | `--output` |

### Knowledge Graph

| Command | Description | Key Options |
|---------|-------------|-------------|
| `uteke graph nodes` | List all graph nodes | `--entity-type` |
| `uteke graph edges` | List all graph edges | `--relation` |
| `uteke graph neighbors <LABEL>` | Find neighbors (BFS) | `--depth` |
| `uteke graph path <SRC> <TGT>` | Shortest path (BFS) | `--max-depth` |
| `uteke graph query <RELATION>` | Query edges by relation type | |
| `uteke graph stats` | Graph statistics | |

### Memory Relationships

| Command | Description | Key Options |
|---------|-------------|-------------|
| `uteke edges <ID>` | List edges for a memory (auto-wired backlinks/forward links) | `--deep` (BFS depth), `--direction` (incoming/outgoing/both) |
| `uteke rebuild-backlinks` | Rebuild `referenced_by` from forward edges | `--quiet` |

### Rooms (Collaborative Memory)

| Command | Description | Key Options |
|---------|-------------|-------------|
| `uteke room create <ID>` | Create a room | `--title` |
| `uteke room list` | List all rooms | `--namespace` |
| `uteke room stats <ID>` | Room statistics and participants | |
| `uteke room recall <ID>` | Recall room memories | `--query`, `--author`, `--limit`, `--min` |
| `uteke room summary <ID>` | Topic clustering summary | |
| `uteke room document <ID>` | Generate structured document from room | |
| `uteke room delete <ID>` | Delete room (memories preserved) | `--confirm` |

### Pinning & Importance

| Command | Description |
|---------|-------------|
| `uteke pin <ID>` | Pin a memory (never decays) |
| `uteke unpin <ID>` | Unpin a memory |
| `uteke importance` | Recalculate importance scores for all memories |
| `uteke orphans` | Find disconnected memories with low importance | `--threshold` (default 0.3), `--limit` (default 50) |

### Maintenance

| Command | Description | Key Options |
|---------|-------------|-------------|
| `uteke dream` | Full maintenance pipeline: lint → backlinks → dedup → orphans → compact → verify | `--phases`, `--skip`, `--dry-run`, `--quiet` |
| `uteke consolidate` | Merge near-duplicate memories | `--threshold` (default 0.90), `--dry-run` |
| `uteke prune` | Remove deprecated/expired memories | `--ttl` (default 30), `--dry-run` |
| `uteke timeline <ID>` | Show audit log events for a memory | `--limit` (default 20) |

### Tags & Namespaces

| Command | Description |
|---------|-------------|
| `uteke tags list` | List all tags with counts (`--by-count`) |
| `uteke tags rename <old> <new>` | Rename a tag across all memories |
| `uteke tags delete <tag>` | Delete a tag from all memories (`--confirm`) |
| `uteke namespace list` | List all namespaces with counts |
| `uteke namespace stats <name>` | Stats for a specific namespace |
| `uteke namespace switch <name>` | Set default namespace in config |

### Memory Aging

| Command | Description | Key Options |
|---------|-------------|-------------|
| `uteke aging status` | Hot/warm/cold/never-accessed breakdown | |
| `uteke aging preview` | Preview stale memories | `--older-than-days` (default 180), `--max-access-count` (default 1) |
| `uteke aging cleanup` | Delete aged memories | `--older-than-days`, `--max-access-count`, `--yes` |

### Health, Stats & Data

| Command | Description |
|---------|-------------|
| `uteke stats` | Memory store statistics + tier breakdown |
| `uteke doctor` | Full health check: DB, index, embedding model, consistency |
| `uteke verify` | Compare DB count vs vector index count |
| `uteke verify-checksums` | Verify binary integrity against SHA256 checksums |
| `uteke repair` | Rebuild vector index from SQLite |
| `uteke export [FILE]` | Export to JSONL (no embeddings). Default: stdout |
| `uteke import [FILE]` | Import from JSONL/Markdown/text. Default: stdin |
| `uteke bench` | Performance benchmarks | `--counts`, `--json` |

### Setup & Upgrade

| Command | Description |
|---------|-------------|
| `uteke init --agent <TYPE>` | Initialize integration (pi/claude/cursor/opencode/hermes) |
| `uteke hook install <SHELL>` | Install shell hook (bash/zsh/fish) |
| `uteke completions <SHELL>` | Generate shell completions (bash/zsh/fish/powershell) |
| `uteke upgrade` | Check for updates and upgrade | `-y` (skip confirmation) |

## Architecture

- **Storage:** SQLite (WAL mode) with namespace column + FTS5 virtual table
- **Vector index:** usearch persistent HNSW (768d, cosine similarity)
- **Hybrid search:** RRF (k=60) merges vector + FTS5 results; graph strategy adds graph-signal reranking
- **Embedding:** ONNX EmbeddingGemma Q4 (768d), auto-downloaded
- **Tiered memory:** Hot (<7d, +0.1 boost), Warm (<30d), Cold (>30d)
- **Schema versioning:** Integer counter, auto-migration on upgrade
- **Zero unsafe code** (`unsafe_code = "forbid"`)
- **Project-scoped stores:** `uteke --store .uteke remember "..."`

## Usage Patterns

### Session lifecycle
```bash
uteke recall "project architecture decisions" --namespace pi-agent
uteke remember "Use WAL mode for concurrent reads" --tags architecture,db --entity db-layer
uteke recall "last session state" --namespace pi-agent
```

### Metadata-enriched storage
```bash
uteke remember "Deploy staging to AWS us-east-1" \
  --tags deploy,aws --entity staging-server --category infrastructure \
  --source "meeting-notes.md" --source-type file
```

### Time-travel queries
```bash
uteke recall "auth flow" --at 2026-06-01T12:00:00Z
uteke list --at 2026-06-01T12:00:00Z
```

### Graph-enhanced recall
```bash
uteke recall "database design" --strategy graph --related --depth 2
uteke graph neighbors "auth-service" --depth 3
uteke graph path "api-gateway" "database"
```

### Document wiki
```bash
uteke doc create architecture/overview --title "Architecture Overview" --file overview.md --parent docs
uteke doc search "authentication" --mode hybrid
uteke doc list --tree
```

### Room collaboration
```bash
uteke remember "Decision: use PostgreSQL" --room design-review --author alice
uteke room recall design-review --query "database"
uteke room summary design-review
```

### Programmatic (JSON)
```bash
uteke recall "auth flow" --json --limit 3
uteke stats --json
uteke list --tag architecture --json --limit 50
```

### Maintenance pipeline
```bash
uteke dream                           # Full pipeline
uteke dream --phases lint,verify --dry-run  # Selective, safe
uteke importance                      # Recompute scores
uteke orphans --threshold 0.2         # Find disconnected
```

## Project-Aware Memory (MANDATORY)

**Always tag memories with `project:<name>` when working in a project.** This prevents noise when recalling — you only see memories relevant to the current project.

### How to detect the project name

1. From `workdir` / `cwd` — extract the repo folder name:
   - `/home/user/repos/bond/` → `project:bond`
   - `/home/user/repos/uteke/` → `project:uteke`
   - `/home/user/repos/my-saas-app/` → `project:my-saas-app`
2. From file paths mentioned in the conversation
3. From the project name the user mentions

### Rules

| Rule | Details |
|------|---------|
| **REMEMBER** | Always include `project:<name>` in `--tags` when the memory is project-specific |
| **RECALL** | Always include `--tags project:<name>` to scope recall to the current project |
| **NO PROJECT** | If the conversation is not about any specific project (e.g., general chat), do NOT add a `project:` tag |
| **TAG FORMAT** | Always lowercase: `project:bond`, `project:uteke` (not `project:Bond`) |

### Examples

```bash
# Working on the "bond" project
uteke recall "auth flow" --tags project:bond
uteke remember "JWT rotation implemented with refresh tokens" --tags project:bond,decision,auth
uteke remember "DB schema uses UUID v7 for primary keys" --tags project:bond,architecture,db

# Working on the "uteke" project
uteke recall "vector index corruption" --tags project:uteke
uteke remember "Fixed usearch rebuild race condition" --tags project:uteke,bugfix,index

# General conversation — no project tag
uteke remember "User prefers concise responses" --tags preference

# Cross-project recall (explicitly requested)
uteke recall "shared auth pattern" --tags project:bond,project:corin
```

### Why this matters

Without project tags, memories from all projects mix together. When you recall "fix auth bug" while working on project Bond, you might get results from project Corin's auth system — causing confusion and wasted context. Project tags ensure recall is scoped and noise-free.

## When to Use

| Trigger | Action |
|---------|--------|
| Before starting work | `uteke recall "<project context>" --tags project:<name>` |
| After making decisions | `uteke remember "<decision>" --tags project:<name>,<other tags>` |
| Important documents | `uteke doc create <slug> --file <path> --parent <slug>` |
| Collaborative decisions | `uteke remember "..." --room <room> --author <name>` |
| Memory feels stale | `uteke dream` or `uteke importance` |
| Index feels corrupt | `uteke doctor` → `uteke repair` |

## Hermes Auto-Recall Plugin

The `uteke-memory` plugin registers a `pre_llm_call` hook that automatically
recalls relevant memories on every turn and injects them into the user message.
No shell hook, no daemon required (subprocess transport). Supports HTTP transport
to `uteke-serve` for container deployments.

```bash
# Generate and install the plugin
uteke init --agent hermes
# → writes to ~/.hermes/plugins/uteke-memory/

# Enable in config.yaml
# plugins:
#   enabled:
#     - uteke-memory
```

Config via `~/.hermes/uteke.json` or env vars:
`UTEKE_BIN`, `UTEKE_NAMESPACE`, `UTEKE_SERVER_URL`, `UTEKE_TOKEN`,
`UTEKE_RECALL_LIMIT` (default 5), `UTEKE_RECALL_MIN_SCORE` (default 0.40).

## Server Mode (uteke-serve)

HTTP daemon with REST API for all operations + monitoring/maintenance endpoints.

```bash
uteke-serve --port 8767
# CLI auto-routes to server when running: ~42ms recall vs ~3s cold start
```

Endpoints mirror CLI commands (e.g., `POST /remember`, `POST /recall`). Supports read-only API tokens for restricted access.
