---
title: Getting Started
---

# Getting Started

## Install

One-liner install (macOS, Linux, Windows):

```bash
curl -sSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh
```

Or install from source (requires [Rust](https://rustup.rs)):

```bash
cargo install --git https://github.com/codecoradev/uteke
```

Pre-built binaries and Docker image also available from [GitHub Releases](https://github.com/codecoradev/uteke/releases) and [GHCR](https://github.com/codecoradev/uteke/pkgs/container/uteke). First run downloads the embedding model (~188MB) — no API keys needed.

> 💡 **Using Docker?** See the [Docker guide](docker.md) for `docker compose` setup, env vars, and persistence.

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

## Memory Aging

```bash
# Show hot/warm/cold/never-accessed breakdown
uteke aging status

# Preview memories older than 90 days
uteke aging preview --days 90

# Delete stale memories older than 180 days
uteke aging cleanup --days 180 --confirm
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

## Rooms

Group related memories by context (meetings, projects, discussions):

```bash
# Create a room
uteke room create "project-kickoff" --title "Project Kickoff"

# Add a memory to a room with author attribution
uteke room add "project-kickoff" <memory-id> --author alice

# Semantic recall within a room
uteke room recall "project-kickoff" --query "database decision"

# Generate a structured document from room memories
uteke room document "project-kickoff"

# Get a summary of room discussions
uteke room summary "project-kickoff"
```

## Time-Travel Queries

Query memories as they existed at a specific point in time:

```bash
# List memories that existed on a given date
uteke list --at 2026-06-01T12:00:00Z

# Semantic recall filtered to memories valid at a point in time
uteke recall "deployment process" --at 2026-06-01T12:00:00Z
```

Timestamps use [RFC 3339](https://www.rfc-editor.org/rfc/rfc3339) format. Replace the example date with your actual target.

## Memory Importance & Pinning

Pin critical memories so they never decay:

```bash
# Pin a memory
uteke pin <id>

# Unpin a memory
uteke unpin <id>

# Recalculate importance scores
uteke importance
```

## Relationship Graph

Link related memories and traverse the graph:

```bash
# Store a memory that supersedes an old one
uteke remember "API rate limit is now 2000/min" --meta "rel:supersedes:old-memory-id"

# Recall with relationship traversal
uteke recall "rate limit" --related --depth 2
```

## Recall Cache

The recall cache eliminates redundant embedding for repeated queries (~50ms savings). It's automatic — no configuration needed. Use `--context` for AI-prompt formatted output:

```bash
# AI-optimized context output
uteke recall "api design" --context
```

## Benchmarking

Test performance with synthetic data:

```bash
# Run full benchmark suite
uteke bench

# Custom counts + JSON output
uteke bench --counts 100,1000 --json
```

See also: [LongMemEval retrieval harness](https://github.com/codecoradev/uteke/tree/develop/benchmarks/longmemeval) for accuracy evaluation.

## Shell Hooks

Auto-load project-scoped memory when you cd into a project directory:

```bash
# Install hook for your shell
uteke hook install bash   # or zsh, fish

# Now when you cd into a project with .uteke/uteke.db,
# uteke auto-discovers it
```

## Export & Import

Port your memories anywhere:

```bash
# Export to JSONL (no embeddings — small, portable)
uteke export > memories.jsonl

# Import on another machine
uteke import memories.jsonl
```

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

## Where is data stored?

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
