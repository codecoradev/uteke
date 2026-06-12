<h1 align="center">Uteke</h1>
<p align="center"><strong>Give your AI a memory that never leaves your machine.</strong></p>
<p align="center">
  <em>Offline-first semantic memory engine — single binary, zero config, 30ms recall.</em>
</p>
<p align="center">
  <a href="https://github.com/codecoradev/uteke/actions/workflows/ci.yml?branch=develop"><img src="https://github.com/codecoradev/uteke/actions/workflows/ci.yml/badge.svg?branch=develop" alt="CI" /></a>
  <a href="https://opensource.org/licenses/Apache-2.0"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg" alt="License: Apache 2.0" /></a>
  <img src="https://img.shields.io/badge/Rust-1.75+-orange.svg" alt="Rust 1.75+" />
  <img src="https://img.shields.io/badge/status-v0.0.14-green.svg" alt="v0.0.14" />
</p>

<p align="center">
  <strong>🇬🇧 English</strong> · <a href="README.id.md">🇮🇩 Bahasa Indonesia</a>
</p>

---

## Quick Start

```bash
# Install (macOS, Linux, Windows)
curl -sSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh

# Store a memory with metadata
uteke remember "Deploy v2.1 to staging" --tags deploy,staging \
  --entity staging-server --category infrastructure

# Hybrid search (vector + FTS5, ranked by RRF)
uteke recall "when do we deploy?"

# Stats
uteke stats
```

**That's it.** No API keys. No Docker. No Python. First run downloads the embedding model (~188MB) and you're good to go.

> 📖 [Install options](INSTALL.md) · [Pre-built binaries](https://github.com/codecoradev/uteke/releases) · [Docker](https://github.com/codecoradev/uteke/pkgs/container/uteke) · [Full docs](https://github.com/codecoradev/uteke/tree/develop/docs)

---

## Why Uteke?

AI agents forget everything between sessions. Uteke gives them persistent, searchable memory — entirely offline, in one binary.

| | **Uteke** | **Mem0** | **Letta** | **Zep** |
|---|---|---|---|---|
| **Setup** | Single binary | pip + Docker + Qdrant | pip + Docker + Postgres | pip + Docker + Neo4j |
| **API keys needed** | ❌ None | ✅ OpenAI/LLM key | ✅ LLM key | ✅ LLM key |
| **Offline** | ✅ Fully | ❌ Cloud embedding | ❌ Needs LLM server | ❌ Needs LLM + vector DB |
| **Semantic search** | ✅ Local ONNX + FTS5 hybrid | ✅ Cloud embedding | ⚠️ Keyword + archival | ✅ GraphRAG |
| **Full-text search** | ✅ FTS5 built-in | ❌ | ⚠️ Keyword only | ❌ |
| **Recall speed** | ~30ms (library) | Network round-trip | Network round-trip | Network round-trip |
| **Privacy** | ✅ Data never leaves machine | ⚠️ Data sent to LLM | ⚠️ Data sent to LLM | ⚠️ Data sent to LLM |
| **License** | Apache 2.0 | Apache 2.0 | Apache 2.0 | Apache 2.0 |

---

## Key Features

- 🧠 **Hybrid Search** — Vector similarity + FTS5 full-text search, merged by Reciprocal Rank Fusion (RRF)
- 🏷️ **Metadata Enrichment** — Tag, entity, category, and key:value metadata on every memory
- 👥 **Multi-Agent Namespaces** — Fully isolated memory per agent, zero overhead
- 🖥️ **Server Mode** — Persistent daemon with ~42ms warm recall (75x faster than CLI)
- 🔥 **Tiered Memory** — Hot/Warm/Cold tracking with auto-cleanup of stale memories
- 🔒 **Fully Offline** — Local ONNX embeddings (768d), no telemetry, no cloud, no API calls
- 📦 **Single Binary** — Zero dependencies. No Docker, no database server, no Python, no API keys
- 📥 **Import/Export** — JSONL-based backup and restore

📖 [Full documentation](docs/getting-started.md) · [CLI reference](docs/cli-reference.md) · [Configuration](docs/configuration.md)

---

## Development

```bash
cargo build --workspace        # Build
cargo test --workspace         # Test (108 unit tests)
cargo clippy -- -D warnings    # Lint
cargo fmt                      # Format
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full contribution guide.

---

## License

[Apache License 2.0](LICENSE) — use it, fork it, ship it.

---

## Star History

<a href="https://www.star-history.com/?repos=codecoradev%2Fcora-cli%2Ccodecoradev%2Futeke&type=date&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=codecoradev/cora-cli%2Ccodecoradev/uteke&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=codecoradev/cora-cli%2Ccodecoradev/uteke&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=codecoradev/cora-cli%2Ccodecoradev/uteke&type=date&legend=top-left" />
 </picture>
</a>
