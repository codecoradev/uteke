# Uteke 🧠

**The Brain for Your AI** — Persistent memory and smart context management for AI agents.

> From Javanese: *uteke* (otak) = brain

## What

Uteke is a Rust library + CLI for persistent memory and smart context retrieval for AI agents. Think of it as the memory layer your AI tools are missing.

```
$ uteke remember "BOND uses Go + SvelteKit monorepo architecture" --tags bond,architecture
✓ Memory stored
  ID: a1b2c3d4-...

$ uteke recall "what architecture does BOND use?"
Found 1 result(s):

  1. (score: 0.940) BOND uses Go + SvelteKit monorepo architecture [bond, architecture]
     ID: a1b2c3d4-...
     Created: 2025-01-15T10:30:00Z
```

## Why

- **Memory loss** — AI agents forget everything between sessions
- **Context window** — Can't fit entire codebase in context
- **Cross-tool** — Different tools, different memory formats, no unification

## Features

- 🦀 **Rust** — Single binary, fast, minimal footprint
- 🏠 **Local-first** — All data stays on your machine
- ⚡ **Fast** — Sub-millisecond recall for 10K memories
- 🧠 **Smart** — Semantic search with ONNX embeddings (all-MiniLM-L6-v2)
- 📦 **Zero config** — Works out of the box, no API keys needed
- 🔄 **JSON output** — Machine-readable `--json` flag on all commands

## Install

### From source

```bash
git clone https://github.com/ajianaz/uteke.git
cd uteke
cargo install --path crates/uteke-cli
```

### Prerequisites

- Rust 1.75+ (for ONNX runtime compatibility)

## Quick Start

```bash
# Store a memory
uteke remember "your important context here" --tags project,architecture

# Semantic search (recall)
uteke recall "what was I working on?"

# Text search (keyword match)
uteke search "monorepo decisions" --limit 5

# List all memories
uteke list
uteke list --tag project --limit 20

# Get a specific memory
uteke get <memory-id>

# Delete a memory
uteke forget <memory-id>

# Show statistics
uteke stats
```

## JSON Output

All commands support `--json` for machine-readable output:

```bash
uteke remember "hello" --json
# {"id":"a1b2c3d4-..."}

uteke recall "hello" --json
# [{"memory":{"id":"...","content":"hello",...},"score":0.95}]

uteke stats --json
# {"total_memories":42,"unique_tags":5,"db_size_bytes":102400}
```

## Shell Completions

```bash
uteke completions bash  > ~/.local/share/bash-completion/completions/uteke
uteke completions zsh   > ~/.zfunc/_uteke
uteke completions fish  > ~/.config/fish/completions/uteke.fish
```

## Configuration

On first run, Uteke creates `~/.uteke/config.toml`:

```toml
[store]
# path = "~/.uteke"  # Default store location

[embedding]
# model = "all-MiniLM-L6-v2"
# max_seq_length = 256
```

Override the store path with `--store` on any command:

```bash
uteke --store /path/to/project/.uteke remember "project-specific note"
```

## How It Works

```
uteke remember "context" --tags tag
    ↓
ONNX Embedding (all-MiniLM-L6-v2, 384d vector)
    ↓
HNSW index (fast approximate nearest neighbor)
    ↓
SQLite (metadata + structured data)
    ↓
~/.uteke/ (everything local)
```

## Architecture

| Component | Technology |
|-----------|-----------|
| Language | Rust (no unsafe code) |
| Vector Index | HNSW |
| Structured Storage | SQLite (rusqlite) |
| Embedding | all-MiniLM-L6-v2 ONNX (384d) |
| CLI | clap |

## Commands Reference

| Command | Description |
|---------|-------------|
| `remember` | Store a new memory with optional tags |
| `recall` | Semantic search using vector similarity |
| `search` | Keyword text search |
| `list` | List memories with optional tag filter |
| `get` | Get a single memory by ID |
| `forget` | Delete a memory by ID |
| `stats` | Show memory store statistics |
| `completions` | Generate shell completion scripts |

## Global Flags

| Flag | Description |
|------|-------------|
| `--store <path>` | Override store location |
| `--json` | Output as JSON |
| `--verbose` | Enable debug logging |

## Development

```bash
# Build
cargo build --workspace

# Test
cargo test --workspace

# Lint
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

## License

MIT
