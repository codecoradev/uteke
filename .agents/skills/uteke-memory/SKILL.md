---
description: "Persistent memory engine for AI agents via the uteke CLI — remember, recall, search, forget with semantic + FTS5 hybrid search, metadata enrichment, and multi-agent namespaces."
---

# Uteke Memory Skill

Persistent memory engine for AI agents via the `uteke` CLI.
Version: **0.0.13** — FTS5 hybrid search with RRF, metadata enrichment, concurrent reads.

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
| `uteke remember <TEXT>` | Store a new memory with metadata | `--tags <tags>`, `--entity <name>`, `--category <cat>`, `--meta <k:v,...>` |
| `uteke recall <QUERY>` | Hybrid search (vector + FTS5, ranked by RRF) | `--limit <n>` (default 5), `--entity <name>`, `--category <cat>` |
| `uteke search <QUERY>` | Keyword text search | `--limit <n>` (default 10), `--tags <tags>` |
| `uteke list` | List memories with filters | `--tag <tag>`, `--entity <name>`, `--category <cat>`, `--limit <n>`, `--offset <n>` |
| `uteke get <ID>` | Get single memory by UUID | |
| `uteke forget <ID>` | Delete memory by ID, tag, tier, or all | `--tag <tag>`, `--cold`, `--all`, `--confirm` |

### Memory Intelligence

| Command | Description |
|---------|-------------|
| `uteke consolidate` | Find and merge duplicate memories (`--threshold 0.60 --dry-run`) |
| `uteke prune` | Remove deprecated/expired temporal memories (`--ttl 30 --dry-run`) |

### Tags & Namespaces

| Command | Description |
|---------|-------------|
| `uteke tags list` | List all tags with counts (`--by-count`) |
| `uteke tags rename <old> <new>` | Rename a tag across all memories |
| `uteke tags delete <tag>` | Delete a tag from all memories |
| `uteke namespace list` | List all namespaces with counts |
| `uteke namespace switch <name>` | Set default namespace in config |

### Memory Aging

| Command | Description |
|---------|-------------|
| `uteke aging status` | Show hot/warm/cold/never-accessed breakdown |
| `uteke aging preview --days N` | Preview memories older than N days |
| `uteke aging cleanup --days N --confirm` | Delete stale memories |

### Health & Maintenance

| Command | Description |
|---------|-------------|
| `uteke stats` | Memory store statistics + tier breakdown (🔥 Hot / 🟡 Warm / ❄️ Cold) |
| `uteke doctor` | Full health check: DB, index, embedding model, consistency |
| `uteke verify` | Compare DB count vs vector index count |
| `uteke repair` | Rebuild vector index from SQLite |

### Data Portability

| Command | Description |
|---------|-------------|
| `uteke export [FILE]` | Export to JSONL (no embeddings — portable). Default: stdout |
| `uteke import [FILE]` | Import from JSONL (re-embeds content). Default: stdin |

### Setup & Shell

| Command | Description |
|---------|-------------|
| `uteke hook install <shell>` | Install shell hook for auto-context loading (bash/zsh/fish) |
| `uteke completions <SHELL>` | Generate shell completions |

## Architecture

- **Storage:** SQLite (WAL mode, bundled) with `namespace` column + FTS5 virtual table
- **Vector index:** usearch persistent HNSW (768d, cosine similarity) with `RwLock` for concurrent reads
- **Full-text search:** SQLite FTS5 with phrase + token-OR fallback
- **Hybrid search:** Reciprocal Rank Fusion (RRF, k=60) merges vector + FTS5 results
- **Embedding:** ONNX EmbeddingGemma Q4 (768d), auto-downloaded
- **Tiered memory:** Hot (<7d, +0.1 boost), Warm (<30d), Cold (>30d)
- **Metadata:** Entity, category, and key:value pairs stored as JSON blob
- **Schema versioning:** Integer counter, auto-migration on upgrade (currently v2)
- **Index files:** `uteke_index.usearch` + `uteke_index.keys` (atomic save)
- **Zero unsafe code** (`unsafe_code = "forbid"`)

## Usage Patterns

### Session lifecycle
```bash
# Before work — load context
uteke recall "project architecture decisions" --namespace pi-agent

# After decisions — persist with metadata
uteke remember "Use WAL mode for concurrent reads" \
  --tags architecture,db \
  --entity db-layer \
  --category decision \
  --namespace pi-agent

# End of session — summarize
uteke remember "Refactored auth middleware, token expiry fix" \
  --tags session,auth --namespace pi-agent

# Session handoff
uteke recall "last session state" --namespace pi-agent
```

### Metadata-enriched storage
```bash
uteke remember "Deploy staging to AWS us-east-1" \
  --tags deploy,aws \
  --entity staging-server \
  --category infrastructure \
  --meta "source:meeting-note,confidence:0.9"
```

### Filter by entity or category
```bash
uteke recall "server" --entity staging-server
uteke list --category infrastructure --limit 10
```

### Project-scoped memory
```bash
uteke --store .uteke remember "Uses React Server Components" --tags frontend
```

### Multi-agent isolation
```bash
uteke remember "Agent-specific context" --namespace code-reviewer
uteke recall "recent decisions" --namespace code-reviewer
```

### Health check
```bash
uteke stats          # 🔥 Hot: 12 | 🟡 Warm: 45 | ❄️ Cold: 200
uteke doctor         # Full diagnostics
uteke verify         # Quick consistency check
uteke repair         # Fix broken index
```

### Data portability
```bash
uteke export backup.jsonl           # Export all memories
uteke export --namespace agent-x > agent-x.jsonl
uteke import backup.jsonl           # Import (re-embeds automatically)
```

### Programmatic (JSON output)
```bash
uteke recall "auth flow" --json --limit 3
uteke stats --json
uteke list --tag architecture --json --limit 50
uteke remember "test" --json        # {"id":"a1b2c3d4-..."}
```

## When to Use

| Trigger | Action |
|---------|--------|
| Before starting work | `uteke recall "<project context>"` |
| After making decisions | `uteke remember "<decision>" --tags <tags> --entity <name>` |
| Before closing session | `uteke remember "<session summary>" --tags session` |
| Important findings | `uteke remember "<finding>" --tags finding,<topic>` |
| Multi-agent context | Use `--namespace <agent-name>` |
| Index feels slow/corrupt | `uteke doctor` → `uteke repair` if needed |

## Server Mode

For real-time agent use, start a persistent daemon:

```bash
uteke-serve --port 8767
# Now CLI auto-routes to server: ~42ms recall vs ~3s cold start
```
