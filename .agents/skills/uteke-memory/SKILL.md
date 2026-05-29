# Uteke Memory Skill

Persistent memory engine for AI agents via the `uteke` CLI.
Version: **0.0.2** — persistent vector index, multi-agent namespaces, tiered memory.

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
| `uteke remember <TEXT>` | Store a new memory | `--tags <tags>` (comma-separated) |
| `uteke recall <QUERY>` | Semantic search (vector similarity) | `--limit <n>` (default 5), `--tags <tags>` |
| `uteke search <QUERY>` | Keyword text search | `--limit <n>` (default 10) |
| `uteke list` | List memories (paginated) | `--tag <tag>`, `--limit <n>`, `--offset <n>` |
| `uteke get <ID>` | Get single memory by UUID | |
| `uteke forget <ID>` | Delete memory by UUID | |

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
| `uteke init` | Initialize uteke integration for an AI agent. `--agent <pi\|claude\|cursor\|copilot\|codex>` |
| `uteke completions <SHELL>` | Generate shell completions (bash, zsh, fish, elvish, powershell) |

## Architecture (v0.0.2)

- **Storage:** SQLite (WAL mode, bundled) with `namespace` column + index
- **Vector index:** usearch persistent HNSW (768d, cosine similarity)
- **Embedding:** ONNX EmbeddingGemma Q4 (768d), auto-downloaded
- **Tiered memory:** Hot (<7d, +0.1 boost), Warm (<30d), Cold (>30d)
- **Index files:** `uteke_index.usearch` + `uteke_index.keys`
- **Zero unsafe code** (`unsafe_code = "forbid"`)

## Usage Patterns

### Session lifecycle
```bash
# Before work — load context
uteke recall "project architecture decisions" --namespace pi-agent

# After decisions — persist
uteke remember "Use WAL mode for concurrent reads" --tags architecture,db --namespace pi-agent

# End of session — summarize
uteke remember "Refactored auth middleware, token expiry fix" --tags session,auth --namespace pi-agent

# Session handoff
uteke recall "last session state" --namespace pi-agent
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
```

## When to Use

| Trigger | Action |
|---------|--------|
| Before starting work | `uteke recall "<project context>"` |
| After making decisions | `uteke remember "<decision>" --tags <tags>` |
| Before closing session | `uteke remember "<session summary>" --tags session` |
| Important findings | `uteke remember "<finding>" --tags finding,<topic>` |
| Multi-agent context | Use `--namespace <agent-name>` |
| Index feels slow/corrupt | `uteke doctor` → `uteke repair` if needed |
