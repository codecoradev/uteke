# Uteke 🧠

**The Brain for Your AI** — Persistent memory and smart context management for AI agents.

> From Javanese: *uteke* (otak) = brain

## What

Uteke is a Rust library + CLI for persistent memory and smart context retrieval for AI agents. Think of it as the memory layer your AI tools are missing.

```
$ cargo install uteke
$ uteke remember "BOND uses Go + SvelteKit monorepo architecture" --tags bond,architecture
$ uteke recall "what architecture does BOND use?"
→ Memory: BOND uses Go + SvelteKit monorepo architecture (score: 0.94)
```

## Why

- **Memory loss** — AI agents forget everything between sessions
- **Context window** — Can't fit entire codebase in context
- **Cross-tool** — Different tools, different memory formats, no unification

## Features

- 🦀 **Rust** — Single binary, 2MB, zero dependencies
- 🏠 **Local-first** — All data stays on your machine
- ⚡ **Fast** — <5ms recall for 10K memories
- 🧠 **Smart** — Semantic search with EmbeddingGemma (768d)
- 📦 **Zero config** — Works out of the box, no API keys needed

## Quick Start

```bash
# Install
cargo install uteke

# Remember
uteke remember "your important context here" --tags project,architecture

# Recall
uteke recall "what was I working on?"

# Search
uteke search "monorepo decisions" --limit 5

# List
uteke list --tag project
uteke stats
```

## How It Works

```
uteke remember "context" --tags tag
    ↓
EmbeddingGemma ONNX (768d vector)
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
| Language | Rust |
| Vector Index | HNSW |
| Structured Storage | SQLite (rusqlite) |
| Embedding | EmbeddingGemma ONNX Q4 (768d) |
| CLI | clap |

## License

MIT
