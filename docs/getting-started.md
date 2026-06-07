---
title: Getting Started
---

# Getting Started

## Install

One-liner install (macOS, Linux, Windows):

```bash
curl -sSL https://raw.githubusercontent.com/ajianaz/uteke/main/install.sh | sh
```

Or install from source (requires [Rust](https://rustup.rs)):

```bash
cargo install --git https://github.com/ajianaz/uteke
```

Pre-built binaries and Docker image also available from [GitHub Releases](https://github.com/ajianaz/uteke/releases) and [GHCR](https://github.com/ajianaz/uteke/pkgs/container/uteke). First run downloads the embedding model (~188MB) — no API keys needed.

## Your First Memory

```bash
# Store a memory
uteke remember --tags project "My app uses SvelteKit 5 with Tailwind"

# Recall by meaning (semantic search)
uteke recall "What frontend framework do I use?"

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
├── uteke.db                    # SQLite (memories + metadata)
├── uteke_index.usearch         # Persistent vector index
├── uteke_index.keys            # Index key mapping
├── models/embeddinggemma/      # Local ONNX embedding model
└── logs/
    ├── uteke.log               # Current log
    └── uteke.log.YYYY-MM-DD    # Rotated logs
```

Copy the entire folder to back up or transfer to another machine.
