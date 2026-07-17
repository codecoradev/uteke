---
title: Getting Started
---

# Getting Started

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh
```

See the [Installation guide](/install) for all methods (Cargo, binary, Docker).

> 💡 First run downloads the embedding model (~188MB). No API keys needed.

## Your First Memory

```bash
# Store a memory with metadata enrichment
uteke remember --tags project "My app uses SvelteKit 5 with Tailwind" \
  --entity my-app --category frontend

# Hybrid search (vector + FTS5, ranked by RRF)
uteke recall "What frontend framework do I use?"

# Filter by entity or category
uteke recall "frontend" --entity my-app
uteke list --category frontend

# Text search with tag filter
uteke search "SvelteKit" --tags project

# List all memories
uteke list

# Check system health
uteke doctor
```

## Tag Management

```bash
# List all tags with usage counts
uteke tags list --by-count

# Rename a tag across all memories
uteke tags rename old-name new-name

# Delete a tag from all memories
uteke tags delete unused-tag
```

## Multi-Agent Isolation

Each agent gets its own namespace. Memories never leak between agents:

```bash
# Agent "architect" stores its context
uteke --namespace architect remember "We chose PostgreSQL for ACID compliance"

# Agent "dev" has its own separate memory
uteke --namespace dev remember "Database connection string: postgres://localhost:5432/app"

# Each only sees its own memories
uteke --namespace architect recall "database"
uteke --namespace dev recall "database"
```

## Recall Cache

The recall cache eliminates redundant embedding for repeated queries (~50ms savings). It's automatic — no configuration needed. Use `--context` for AI-prompt formatted output:

```bash
# AI-optimized context output
uteke recall "api design" --context
```

## Export & Import

Port your memories anywhere:

```bash
# Export to JSONL (no embeddings — small, portable)
uteke export > memories.jsonl

# Import on another machine
uteke import memories.jsonl

# Import with LLM fact extraction (distills raw text into atomic facts)
uteke import notes.txt --extract
```

## MCP Integration

Add uteke as an MCP server to your AI coding agent in seconds:

**Claude Code** — add to `.mcp.json`:

```json
{ "mcpServers": { "uteke": { "command": "uteke-mcp" } } }
```

**With HTTP** (requires `uteke-serve`):

```json
{ "mcpServers": { "uteke": { "url": "http://127.0.0.1:8767/mcp" } } }
```

See [MCP Server](/mcp) for all supported clients and tools.

> 💡 **Hermes users?** Three integration modes available:
> - **Mode C (shell hook):** Lightest — automatic recall via `pre_llm_call` hook, no plugin/daemon needed. See [Hermes integration](/integrations/hermes).
> - **Mode B (memory-provider):** Full auto — `uteke init --agent hermes --memory-provider`. Automatic recall + LLM fact extraction.
> - **Mode A (uteke-tool):** Manual — `uteke init --agent hermes`. Explicit `uteke(action="...")` calls with multi-agent room support.
>
> The install script installs all three binaries (`uteke`, `uteke-serve`, `uteke-mcp`) so MCP integration is available immediately.

## Troubleshooting

If something goes wrong, uteke has built-in self-healing:

```bash
# Check system health (DB, index, model, consistency)
uteke doctor

# Verify DB and index consistency
uteke verify

# Repair index by rebuilding from SQLite
uteke repair
```

## Where is Data Stored?

All data lives in `~/.uteke/`:

```
~/.uteke/
├── uteke.db                    # SQLite (memories + metadata + FTS5)
├── uteke_index.usearch         # Persistent HNSW vector index
├── uteke_index.keys            # Index key mapping
├── embeddinggemma-q4/          # Local ONNX embedding model (~188MB)
│   └── onnx/                   # model_q4.onnx + model_q4.onnx_data
└── logs/
    ├── uteke.log               # Current log
    └── uteke.log.YYYY-MM-DD    # Rotated logs
```

Copy the entire folder to back up or transfer to another machine.

## Next Steps

- [Installation](/install) — all install methods
- [Rooms](/rooms) — group memories by context
- [Time-Travel Queries](/time-travel) — recall memories at any point in time
- [Smart Decay](/smart-decay) — pinning, importance scoring, aging
- [Relationship Graph](/relationship-graph) — link and traverse memories
- [Benchmarks](/benchmarks) — performance numbers
- [Shell Hooks](/shell-hooks) — auto-load project context
- [MCP Server](/mcp) — AI agent integration
- [CLI Reference](/cli-reference) — complete command reference
- [Configuration](/configuration) — config file and options
