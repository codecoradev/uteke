---
description: "Persistent memory engine for AI agents via the uteke CLI — remember, recall, search, forget with hybrid + fusion semantic search (fusion default since 0.16.0), documents, knowledge graph, rooms, tiered memory, and multi-agent namespaces."
---

# Uteke Memory Skill

Persistent memory engine for AI agents via the `uteke` CLI.
Version: **0.20.0** — SQLite + HNSW vector index (usearch default, or vecq) + FTS5 hybrid search (RRF k=60); `fusion` (weighted RRF of the vector and hybrid rankings) is the default recall strategy since 0.16.0. `unsafe_code = "forbid"` workspace-wide.

> This skill ships with each release — its version tracks the CLI version
> (a CI gate fails when they drift, see `skill-version-parity` test).
> If this file is older than your `uteke --version`, run `uteke init` again
> or re-read the docs for the current release.

> **Hermes integration:** Install the `uteke-memory` plugin for automatic recall
> on every turn via the `pre_llm_call` hook. No shell hook or daemon needed.
> See `extensions/hermes-uteke-memory/` for the plugin source.
> Manual tool calls via `uteke-tool` plugin remain available for explicit
> remember/forget/room operations.

## Global Flags (all commands)

| Flag | Description |
|------|-------------|
| `--store <PATH>` | Override store path (default: `~/.codecora/uteke`) |
| `--namespace <NS>` | Multi-agent isolation (default: `"default"`) |
| `--json` | Machine-readable JSON output |
| `--verbose` | Debug logging |

## Commands

### Core Memory Operations

| Command | Description | Key Options |
|---------|-------------|-------------|
| `uteke remember <TEXT>` | Store a new memory | `--tags`, `--type`, `--entity`, `--category`, `--meta`, `--room`, `--author`, `--source`, `--source-type`, `--detect-contradiction` |
| `uteke recall <QUERY>` | Fusion search (default since 0.16.0 — weighted RRF of vector + hybrid rankings) | `--limit`, `--tags`, `--entity`, `--category`, `--min`, `--strategy` (fusion/vector/fts5/hybrid/graph), `--salience`, `--recency`, `--related`, `--depth`, `--context`, `--at` (time-travel), `--type` (all/memory/doc), `--where` (JSON field filter), `--explain` (real vector similarity), `--budget`, `--pack`, `--strict`, `--enrich`, `--exclude-ids`, `--content-format`, `--no-salience`, `--no-recency` |
| `uteke search <QUERY>` | Keyword text search | `--limit`, `--tags` |
| `uteke list` | List memories with filters | `--tag`, `--entity`, `--category`, `--limit`, `--offset`, `--at` |
| `uteke get <ID>` | Get single memory by UUID | |
| `uteke update <ID>` | Update a memory in place | `--content`, `--tags` (replaces set), `--importance`, `--pinned`, `--type` |
| `uteke context` | Project context summary (counts, top tags, recent activity) | `--namespace` |
| `uteke guide` | Print the agent-facing memory tools guide for system-prompt injection | |
| `uteke feedback <ID> <helpful\|unhelpful>` | Record usefulness (importance +0.05 / −0.10) | |
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
| `uteke room update <ID>` | Update title/description | `--title`, `--description` |
| `uteke room rename <OLD> <NEW>` | Rename a room (members and document links move too) | |
| `uteke room move-memory <MEMORY_ID>` | Move a memory between rooms, keeping link provenance | `--from`, `--to` |
| `uteke room add-document <ROOM> <SLUG>` | Link a document to a room | |
| `uteke room remove-document <ROOM> <SLUG>` | Unlink a document | |
| `uteke room list-documents <ROOM>` | List documents linked to a room | |
| `uteke room list-rooms <SLUG>` | List rooms that reference a document | |
| `uteke room consolidate <ID>` | Merge a room's memories into denser records (dry-run unless `--apply`; uses LLM calls) | `--apply`, `--max-calls` |
| `uteke room list` | List all rooms | `--namespace` |
| `uteke room stats <ID>` | Room statistics and participants | |
| `uteke room recall <ID>` | Recall room memories | `--query`, `--author`, `--limit`, `--min` |
| `uteke room summary <ID>` | Topic clustering summary | |
| `uteke room document <ID>` | Generate structured document from room | |
| `uteke room delete <ID>` | Delete room (memories preserved) | `--confirm` |

### Conflict Resolution

| Command | Description |
|---------|-------------|
| `uteke supersede <OLD> <NEW>` | Mark `OLD` superseded by `NEW` (resolves a contradiction) |
| `uteke contradictions list` | Resolution ledger: superseded-but-not-restored memories |
| `uteke contradictions undo <ID>` | Restore a superseded memory (undoes the pair) |
| `uteke provenance <ID>` | Full provenance report for a memory |

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
| `uteke dream` | Full maintenance pipeline: lint → backlinks → dedup → orphans → compact → verify (dry-run first by default) | `--phases`, `--skip`, `--dry-run`, `--quiet` |
| `uteke lifecycle <cycle\|promote\|status>` | Safe lifecycle: soft-deprecate aged memories, promote back, status | `--namespace` |
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
| `uteke namespace move <ID> <NS>` | Move a memory to another namespace |
| `uteke namespace rename <FROM> <TO>` | Rename a namespace (merges if the target exists) |
| `uteke namespace delete <NAME>` | Delete with `--strategy refuse\|merge\|deprecate` (`--target` for merge, `--confirm` required) |

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
| `uteke repair` | Rebuild vector index from SQLite (`--rebuild` deletes corrupt index files first, `--reembed` regenerates missing embeddings) |

> **Stale-index symptom:** a memory is recallable via
> `--strategy fts5` but missing under the default fusion/hybrid — that is a
> vector-index desync, not data loss. Run `uteke verify`, then `uteke repair`.
> On uteke-serve, use `POST /verify` / `POST /repair` (HTTP) or the
> `uteke_verify` / `uteke_repair` MCP tools — no restart needed.
> A held index lock (another `uteke`/`uteke-serve` process) never deletes the
> index in builds after 0.19.1: the CLI falls back to an in-memory index instead.
> Run `verify` after every serve upgrade (#1245 class: CLI upgraded, serve left
> behind → index written by the old binary reads as mismatched).
>
> **Search strategy guide — filters, not silos:**
> - `namespace` = agent/workspace identity — pass it explicitly on recall.
> - `room` = collaborative discussion context (`uteke room recall`).
> - `tags` (e.g. `project:<repo>`) = the primary project filter.
> - Strategies: `fusion` (default), `hybrid` (RRF vector+FTS5), `fts5`
>   (keyword — best for short 1–3 word queries and exact terms), `vector`
>   (pure semantic), `graph`. Scores are rank-based (RRF), not cosine —
>   don't threshold them like similarity; use `recall --explain` for the
>   real vector similarity.
> - Prefer 2–3 core keywords: FTS5 AND-matches all tokens, long natural
>   sentences can zero out the keyword arm.

### Import / Export / Bench

| Command | Description |
|---------|-------------|
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
| `uteke onboard` | Interactive setup wizard: detect install, pick agent, toggle features, write config | `--yes --agent <TYPE>` |

## Architecture

- **Storage:** SQLite (WAL mode) with namespace column + FTS5 virtual table
- **Vector index:** usearch persistent HNSW (default) or vecq (quantized, no C++ dependency), 768d cosine. Select with `UTEKE_VECTOR_BACKEND=usearch|vecq` or `[vector] backend` in `uteke.toml`; switching engines on an existing store rebuilds the index from SQLite
- **Hybrid search:** RRF (k=60) merges vector + FTS5 results; graph strategy adds graph-signal reranking
- **Fusion (default since 0.16.0):** weighted RRF of the vector and hybrid rankings — LongMemEval 500Q R@5 0.946
- **Embedding:** ONNX EmbeddingGemma Q4 (768d), auto-downloaded
- **Tiered memory:** Hot (<7d, +0.1 boost), Warm (<30d), Cold (>30d)
- **Schema versioning:** Integer counter, auto-migration on upgrade
- **`unsafe_code = "forbid"`** at the workspace level
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
4. If the project has a registry (a doc such as `uteke-project-registry`), use the
   canonical name from it, resolving aliases (e.g. a folder `api-v2` that the
   registry lists as `recruitment-api`).

**Never derive a tag from a generic folder name** (`development`, `projects`,
`documents`, `desktop`, `downloads`, `tmp`, `src`, `code`, `work`, `workspace`,
`repos`, or your home folder). Those are not projects; when `cwd` is one of them
and the conversation names no project, add no `project:` tag.

### Rules

| Rule | Details |
|------|---------|
| **REMEMBER** | Always include `project:<name>` in `--tags` when the memory is project-specific |
| **RECALL** | Always include `--tags project:<name>` to scope recall to the current project |
| **NO PROJECT** | If the conversation is not about any specific project (e.g., general chat), do NOT add a `project:` tag |
| **TAG FORMAT** | Lowercase with hyphens only — no uppercase, underscores, or spaces: `project:bond`, `project:my-saas-app` |
| **HIERARCHY** | A component of a larger project carries two tags, the component and its parent (`project:samson-mobile` + `project:samson`): the component matches the folder name, the parent recalls the whole project |
| **NO GENERIC TAGS** | No tags from generic folder names, no tags that act as ids, no tags that mimic a field (use `--type`, not `qtype:`) |

## Namespace, Room and Tag Conventions

Keep the three axes distinct so recall stays filterable:

| Axis | Means | Rule |
|------|-------|------|
| **Namespace** | Workspace / whose area | Few and coarse (e.g. one per agent family or tool). Not one per project: project identity lives in the room and tags. Avoid `repo-*` / `project:*` namespaces. Do not keep writing to `default` once you have chosen real ones |
| **Room** | One project or discussion | One room per parent project named `<name>` (lowercase, hyphens; no `project:` / `repo-` prefix, no `-dev` suffix). Discussions `disc:<topic>`, research `riset:<topic>`. Always set `--title` |
| **Tag** | Search aid | `project:<name>` (registry names only), `tool:<name>` for items about a specific tool or agent (`tool:claude-code`), `agent:<role>` only for the origin of a multi-profile agent |

- **A room's namespace cannot be changed after creation** (#1352) and merging
  namespaces does not relabel rooms: create the room in the right namespace
  from the start.
- `uteke update --tags` **replaces** the whole tag set — read the memory first
  and write back the merged list; there is no bulk add-tag operation.
- `uteke namespace rename` changes only the memories' namespace, not the `rooms` table.

### What to store

Store only what is worth recalling again: decisions with their reason, causes of
non-obvious bugs, gotchas, procedures — one fact per memory. Never store tokens,
passwords, or other credentials. Do not auto-save every prompt; automatic writers
produce noise and junk tags.

### Destructive cleanup needs the owner's go-ahead

Writing bulk changes (tag deletes, namespace merges/deletes, room deletes, memory
moves) is not reversible in general. Take a backup first (`uteke export`, plus a
map of ids before/after), do a dry run and a small pilot, and ask for explicit
confirmation per step. `uteke room delete` keeps the memories; `uteke forget` does not.

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

Security defaults (builds after 0.19.1):

- **Auth:** `--auth-token` / `UTEKE_AUTH_TOKEN` (admin) and `UTEKE_READ_ONLY_TOKEN`. When the CLI routes through the server (`[server] enabled = true`) it forwards `UTEKE_AUTH_TOKEN` as a bearer token, to loopback servers only.
- **CORS is off unless configured:** set `cors_origins` in `uteke.toml` / `--cors-origin`. `"*"` is an explicit opt-in — avoid it without auth.
- **Project config is not trusted for endpoints:** a `.uteke/uteke.toml` in the working directory cannot set `[server]`, `[extraction]`, embedding/extraction `base_url`, `endpoint_path`, `api_key`, `embedding.backend`, or `server.host/port` (set them in the global config). `UTEKE_TRUST_PROJECT_CONFIG=1` overrides this.
- List-style endpoints cap `limit` at 1000; `/extract` caps `max_facts` at 100.
