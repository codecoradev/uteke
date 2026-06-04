<h1 align="center">Uteke</h1>
<p align="center"><strong>Give your AI a memory that never leaves your machine.</strong></p>
<p align="center">
  <em>Offline-first semantic memory engine — single binary, zero config, 30ms recall.</em>
</p>
<p align="center">
  <a href="https://github.com/ajianaz/uteke/actions/workflows/ci.yml?branch=develop"><img src="https://github.com/ajianaz/uteke/actions/workflows/ci.yml/badge.svg?branch=develop" alt="CI" /></a>
  <a href="https://opensource.org/licenses/Apache-2.0"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg" alt="License: Apache 2.0" /></a>
  <img src="https://img.shields.io/badge/Rust-1.75+-orange.svg" alt="Rust 1.75+" />
  <img src="https://img.shields.io/badge/status-v0.0.8-green.svg" alt="v0.0.8" />
</p>

---

## Quick Start

```bash
# Install (macOS, Linux, Windows)
curl -sSL https://raw.githubusercontent.com/ajianaz/uteke/main/install.sh | sh

# Store a memory
uteke remember "Deploy v2.1 to staging on Friday" --tags deploy,staging

# Semantic search
uteke recall "when do we deploy?"

# Stats
uteke stats
```

**That's it.** No API keys. No Docker. No Python. First run downloads the embedding model (~188MB) and you're good to go.

> 📖 More install options: [INSTALL.md](INSTALL.md) · [Pre-built binaries](https://github.com/ajianaz/uteke/releases) · [Docker](https://github.com/ajianaz/uteke/pkgs/container/uteke)

---

## Why Uteke?

AI agents forget everything between sessions. Uteke gives them persistent, searchable memory — entirely offline, in one binary.

|| | **Uteke** | **Mem0** | **Letta** | **Zep** |
||---|---|---|---|---|
|| **Setup** | Single binary | pip + Docker + Qdrant | pip + Docker + Postgres | pip + Docker + Neo4j |
|| **API keys needed** | ❌ None | ✅ OpenAI/LLM key | ✅ LLM key | ✅ LLM key |
|| **Offline** | ✅ Fully | ❌ Cloud embedding | ❌ Needs LLM server | ❌ Needs LLM + vector DB |
|| **Semantic search** | ✅ Local ONNX | ✅ Cloud embedding | ⚠️ Keyword + archival | ✅ GraphRAG |
|| **Zero config** | ✅ Works instantly | ❌ Docker + env vars | ❌ Docker + env vars | ❌ Docker + env vars |
|| **Embedding model** | Built-in (ONNX) | External (cloud) | External | External |
|| **Recall speed** | ~30ms (library) | Network round-trip | Network round-trip | Network round-trip |
|| **Privacy** | ✅ Data never leaves machine | ⚠️ Data sent to LLM | ⚠️ Data sent to LLM | ⚠️ Data sent to LLM |
|| **Language** | Rust | Python | Python | Go + Python |
|| **License** | Apache 2.0 | Apache 2.0 | Apache 2.0 | Apache 2.0 |

---

## Design Philosophy

### What Uteke Is
- **Local-first memory store** — portable, offline, single binary
- **Smart but focused** — semantic search, contradiction detection, temporal facts, consolidation
- **Per-agent isolation** — namespaces, not shared graphs

### What Uteke Is NOT
- **NOT a knowledge graph** — no entity relationships, no cross-agent shared context
  - For knowledge graphs, use dedicated tools (Qdrant, Neo4j, or fleet-level systems)
- **NOT a vector database** — Uteke is a memory tool, not a DB engine
  - For large-scale vector search, use Qdrant, Weaviate, or Pinecone
- **NOT a cloud service** — no sync, no accounts, no subscriptions
  - By design: your memory stays on your machine

---

## Use Cases

- **🤖 AI Agents** — Give your agents persistent memory across sessions. Recall context from yesterday's conversation.
- **🔬 Research Notes** — Store findings with semantic search. Find that insight you read 3 months ago.
- **📝 Personal Knowledge** — A local, searchable second brain. No cloud, no subscriptions, no lock-in.
- **🛠️ Developer Context** — Remember architecture decisions, debug notes, and project-specific knowledge.

---

## Who is Uteke for?

| You are | You want | Uteke? |
|---------|----------|--------|
| AI agent builder | Persistent memory, no infra | ✅ Perfect fit |
| CLI power user | Searchable personal knowledge base | ✅ Perfect fit |
| Privacy-conscious dev | Memory tool that works offline | ✅ Perfect fit |
| Team needing shared memory | Multi-user sync + collaboration | ❌ Not yet (Phase B) |
| Enterprise needing graph RAG | Entity relationships, cross-agent knowledge | ❌ Use Mem0/Zep instead |

> **Not sure?** Try it — `curl -sSL https://raw.githubusercontent.com/ajianaz/uteke/main/install.sh | sh` — uninstall is just `rm -rf ~/.uteke`.

---

## Commands

| Command | Description | Example |
|---------|-------------|---------|
| `remember` | Store a new memory with optional tags | `uteke remember "text" --tags a,b` |
| `recall` | Semantic search using vector similarity | `uteke recall "query" --limit 5` |
| `search` | Keyword text search (supports `--tags` filter) | `uteke search "monorepo" --tags rust,cli` |
| `list` | List memories with pagination and tag filter | `uteke list --tag project --limit 20 --offset 10` |
| `get` | Get a single memory by ID | `uteke get <uuid>` |
| `forget` | Delete a memory by ID, tag, tier, or all | `uteke forget <uuid>`, `uteke forget --tag stale` |
| `consolidate` | Find and merge duplicate memories | `uteke consolidate --threshold 0.60 --dry-run` |
| `prune` | Remove deprecated/expired temporal memories | `uteke prune --ttl 30 --dry-run` |
| `namespace list` | List all namespaces with counts | `uteke namespace list` |
| `namespace switch` | Set default namespace in config | `uteke namespace switch my-agent` |
| `tags list` | List all tags with usage counts | `uteke tags list --by-count` |
| `tags rename` | Rename a tag across all memories | `uteke tags rename old-name new-name` |
| `tags delete` | Delete a tag from all memories | `uteke tags delete unused-tag` |
| `aging status` | Show hot/warm/cold/never-accessed breakdown | `uteke aging status` |
| `aging preview` | Preview memories older than N days | `uteke aging preview --days 90` |
| `aging cleanup` | Delete stale memories older than N days | `uteke aging cleanup --days 180 --confirm` |
| `stats` | Show memory store statistics (with tier breakdown) | `uteke stats` |
| `doctor` | Check system health (DB, index, model, consistency) | `uteke doctor` |
| `verify` | Verify DB and index consistency | `uteke verify` |
| `repair` | Rebuild index from SQLite | `uteke repair` |
| `hook install` | Install shell hook for auto-context loading | `uteke hook install bash` |
| `completions` | Generate shell completion scripts | `uteke completions bash` |

### Global Flags

| Flag | Description |
|------|-------------|
| `--store <path>` | Override store location (default: `~/.uteke`) |
| `--namespace <name>` | Namespace for multi-agent isolation (default: `"default"`) |
| `--json` | Output as JSON (all commands) |
| `--verbose` | Enable debug logging |

### JSON Output

Every command supports `--json` for machine-readable output:

```bash
uteke remember "hello" --json
# {"id":"a1b2c3d4-..."}

uteke recall "hello" --json
# [{"memory":{"id":"...","content":"hello",...},"score":0.95}]

uteke stats --json
# {"total_memories":42,"unique_tags":5,"db_size_bytes":102400}
```

### Server Mode

Start a persistent HTTP server to eliminate cold-start overhead. The ONNX embedding model loads once at startup — all subsequent requests skip the ~3s model load:

```bash
# Start server
uteke-serve --port 8767

# API is now warm — all requests are fast
uteke recall "what was that context?"   # ~42ms
uteke remember "New finding" --tags research
```

> **Why server mode?** The CLI loads the ONNX model from disk on every invocation (~3s). The server keeps the model in memory, making recall ~75x faster.

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Status and memory count |
| `/remember` | POST | Store a memory `{content, tags?, namespace?}` |
| `/recall` | POST | Semantic search `{query, limit?, namespace?}` |
| `/search` | POST | Keyword search `{query, limit?, namespace?}` |
| `/list` | POST | List with filters `{tag?, limit?, offset?, namespace?}` |
| `/forget` | DELETE | Delete by id, tag, or namespace |
| `/memory` | GET | Get single memory by id |
| `/stats` | GET | Store statistics |
| `/namespaces` | GET | List all namespaces |

### Multi-Agent Namespaces

Isolate memories per agent using `--namespace`:

```bash
# Each agent gets its own memory space
uteke --namespace hermes remember "Prod deploy config" --tags deploy
uteke --namespace pi remember "User prefers dark mode" --tags pref

# Search is scoped to the namespace
uteke --namespace hermes recall "deployment"  # Only finds hermes memories
uteke --namespace pi recall "preferences"    # Only finds pi memories

# Without --namespace, uses "default" namespace
uteke remember "General knowledge" --tags misc
```

Existing databases are auto-migrated — the `namespace` column is added on first run with zero data loss.

---

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                    CLI (clap)                        │
│  uteke-cli crate — auto-routes to server if running │
├─────────────────────────────────────────────────────┤
│              HTTP API (uteke-serve)                  │
│  /health /remember /recall /search /list /forget    │
│  /stats /namespaces — CORS enabled, ~42ms recall    │
├─────────────────────────────────────────────────────┤
│                    Uteke API                         │
│          uteke-core crate (lib)                      │
├──────────┬──────────────────┬────────────────────────┤
│   ONNX   │     usearch      │       SQLite           │
│ Embedding│  Vector Index    │    Metadata Store      │
│ (256d)   │ (Persistent HNSW)│    (rusqlite)          │
├──────────┴──────────────────┴────────────────────────┤
│              ~/.uteke/ (local storage)               │
│ uteke.db │ uteke_index.usearch │ models/embeddinggemma/ │
└─────────────────────────────────────────────────────┘
```

| Component | Technology | Detail |
|-----------|-----------|--------|
| Language | Rust (no unsafe) | Memory-safe, fast, single binary |
| Vector Index | usearch | Persistent HNSW with incremental updates |
| Storage | SQLite (rusqlite) | Embedded, zero-config, battle-tested |
| Embedding | EmbeddingGemma Q4 ONNX | 256d vectors, multilingual, downloaded on first run |
| Namespaces | SQLite column | Multi-agent isolation, zero overhead |
| Tiered Memory | Access tracking | Hot/Warm/Cold scoring boost |
| CLI | clap | Standard Rust CLI framework |

**How it works:**
1. `remember` → text is embedded into a 256d vector via ONNX → stored in SQLite + indexed in usearch
2. `recall` → query is embedded → usearch finds nearest neighbors → hot memories get +0.1 score boost → returns ranked results
3. `search` → SQLite LIKE-based keyword search (fast, deterministic, scoped to namespace)
4. `forget` → incremental delete from usearch + SQLite (no rebuild)
5. Everything lives in `~/.uteke/` — fully local, fully yours

---

## Performance

> **TL;DR:** Library recall in ~30ms. Server recall in ~42ms. CLI in ~3s (cold start). Zero external dependencies. All on CPU.

### The One Number That Matters

| Mode | Recall | Setup |
|------|--------|-------|
| **Library (Rust)** | **~30ms** | In-process, no startup |
| **Server (HTTP)** | **~42ms** | One-time ~2s init |
| **CLI (binary)** | **~3s** | Per-invocation (model load) |

For real-time agent use, run `uteke-serve` — model stays in memory, 75x faster than CLI.

Benchmarked on **Oracle Cloud ARM (Ampere Altra), CPU-only, no GPU** — single `uteke-serve` instance.

### CLI (cold process) vs Server (warm daemon)

The ONNX embedding model is the bottleneck — it must load from disk (~3s on ARM) and run inference (~60ms per text). The server eliminates the per-request model loading.

| Metric | CLI (cold) | Server (warm) | Speedup |
|--------|-----------|---------------|---------|
| **Insert 100** | ~316s (0.3/s) | 7.7s (13/s) | **41x** |
| **Insert 1,000** | ~53 min (0.3/s) | 84s (12/s) | **38x** |
| **Recall (avg/query)** | 3,158ms | 42ms | **75x** |
| **Search (avg/query)** | 3,158ms | 9ms | **367x** |
| **Model load** | Every invocation (~3s) | Once at startup (~2s) | — |
| **Daemon init** | N/A | ~2s (one-time) | — |

### Scaling (warm server)

| Data Size | Recall (avg) | Search (avg) | Notes |
|-----------|-------------|-------------|-------|
| 100 memories | 42ms | 9ms | Baseline |
| 1,000 memories | 49ms | 13ms | +7ms recall, +4ms search |
| 10,000 memories* | ~55ms (est.) | ~20ms (est.) | HNSW scales logarithmically |

*\*10K estimated — verified at 1K, HNSW vector search is O(log n)*

### What's Fast

- **Recall/search**: ~42ms end-to-end (embed + HNSW search + SQLite) — fast enough for real-time AI agent use
- **HNSW indexing**: Incremental, no rebuild needed on insert/delete
- **SQLite metadata**: Sub-millisecond tag filtering, pagination, namespace isolation

### What's Not Fast (and Why)

- **CLI cold start**: ~3s per invocation — ONNX model loading from disk (188MB)
  - **Solution:** Run `uteke-serve` as a daemon — model stays in memory
- **Bulk insert**: ~13/s — each memory needs ONNX inference (~60ms)
  - **This is expected** for CPU-only embedding. GPU or batch embed would improve this.
- **Consolidate at 1K**: ~11s — pairwise cosine similarity across all memories
  - **This is expected** — O(n²) comparison. Use `--namespace` to scope to smaller sets.

> 📊 Raw data and benchmark tool: `cargo run --release --bin memory-bench`
> Environment: Oracle Cloud ARM Ampere Altra, 4 vCPU, 24GB RAM, Ubuntu, no GPU.

---

## Python Integration

Uteke comes with a zero-dependency Python wrapper (stdlib only, Python 3.8+):

```python
from python_hermes import UtekeMemory

mem = UtekeMemory()

# Store
mid = mem.remember("Deploy v2.1 to staging", tags=["deploy", "staging"])

# Semantic search
results = mem.recall("deployment steps")
for r in results:
    print(f"[{r['score']:.2f}] {r['memory']['content']}")

# Forget
mem.forget(mid)
```

The wrapper calls the `uteke` binary via subprocess with `--json` — no FFI, no bindings, works everywhere.

See [`examples/python_hermes.py`](examples/python_hermes.py) for the full implementation.

---

## Shell Completions

```bash
uteke completions bash  > ~/.local/share/bash-completion/completions/uteke
uteke completions zsh   > ~/.zfunc/_uteke
uteke completions fish  > ~/.config/fish/completions/uteke.fish
```

---

## Configuration

Uteke supports `uteke.toml` configuration with layered resolution:

1. `.uteke/uteke.toml` (project-level, in cwd)
2. `~/.uteke/uteke.toml` (global user-level)
3. Built-in defaults

```toml
[store]
path = "~/.uteke"
namespace = "default"

[embedding]
model = "embeddinggemma-q4"
max_seq_length = 256

[tier]
hot_days = 7
warm_days = 30
hot_boost = 0.1

[logging]
level = "warn"

[server]
enabled = false
host = "127.0.0.1"
port = 8767
```

---

## Development

```bash
# Build
cargo build --workspace

# Test
cargo test --workspace

# Lint
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check

# Run locally
cargo run --bin uteke -- remember "test" --tags dev
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full contribution guide.

---

## Roadmap

Demand-gated — we build what people actually use.

**✅ v0.0.8 (current):** Multi-agent namespaces, server mode, memory aging, Docker, shell hooks, input validation, benchmarks
**🔮 Phase A (100+ stars):** Better embeddings, import/export, Python SDK (PyO3), editor integrations (VS Code)
**🔮 Phase B (500+ stars):** Cloud sync (opt-in), team collaboration, API gateway integration
**🔮 Phase C (1000+ stars):** Plugin ecosystem, advanced consolidation, community extensions

---

## License

[Apache License 2.0](LICENSE) — use it, fork it, ship it.

---

<p align="center">
  <strong>Offline. Zero config. Your memory, your machine.</strong>
</p>
