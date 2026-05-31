<p align="center">
  <img src="https://img.shields.io/badge/Uteke-🧠-blue" alt="Uteke" />
</p>

<h1 align="center">Uteke</h1>
<p align="center"><strong>Local-first memory for AI agents — written in Rust</strong></p>
<p align="center">
  <a href="https://github.com/ajianaz/uteke/actions/workflows/ci.yml?branch=develop"><img src="https://github.com/ajianaz/uteke/actions/workflows/ci.yml/badge.svg?branch=develop" alt="CI" /></a>
  <a href="https://opensource.org/licenses/Apache-2.0"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg" alt="License: Apache 2.0" /></a>
  <img src="https://img.shields.io/badge/Rust-1.75+-orange.svg" alt="Rust 1.75+" />
  <img src="https://img.shields.io/badge/status-v0.0.4-green.svg" alt="v0.0.4" />
</p>

<p align="center">
  <em>From Javanese: <strong>uteke</strong> (otak) = brain</em>
</p>

---

## Quick Start

```bash
# Install
git clone https://github.com/ajianaz/uteke.git
cd uteke
cargo install --path crates/uteke-cli

# Store a memory
uteke remember "Deploy v2.1 to staging on Friday" --tags deploy,staging

# Store in a specific namespace (multi-agent isolation)
uteke --namespace hermes remember "Prod server on AWS us-east-1" --tags deploy

# Semantic search (scoped to namespace)
uteke --namespace hermes recall "server deployment"

# Get stats
uteke stats
```

**That's it.** No API keys. No server. No config. First run downloads the embedding model (~188MB) and you're good to go.

> 📖 For more install options (pre-built binaries, quick install script), see [INSTALL.md](INSTALL.md).

---

## Why Uteke?

AI agents forget everything between sessions. Uteke gives them persistent, searchable memory — entirely local.

| | **Uteke** | MemGPT | ChromaDB | Zep |
|---|---|---|---|---|
| **Setup** | Single binary | Python + deps | Python + server | Cloud service |
| **Cloud required** | ❌ No | ❌ No | ✅ Optional | ✅ Yes |
| **Semantic search** | ✅ Built-in | ✅ | ✅ | ✅ |
| **Embedding model** | Built-in (ONNX) | External | External | External |
| **Zero config** | ✅ | ❌ | ❌ | ❌ |
| **Offline** | ✅ Fully | ⚠️ Partial | ⚠️ Partial | ❌ |
| **Language** | Rust | Python | Python | Go + Python |
| **Binary size** | Small | Large | Large | N/A |
| **License** | Apache 2.0 | Apache 2.0 | Apache 2.0 | MIT |

---

## Use Cases

- **🤖 AI Agents** — Give your agents persistent memory across sessions. Recall context from yesterday's conversation.
- **🔬 Research Notes** — Store findings with semantic search. Find that insight you read 3 months ago.
- **📝 Personal Knowledge** — A local, searchable second brain. No cloud, no subscriptions, no lock-in.
- **🛠️ Developer Context** — Remember architecture decisions, debug notes, and project-specific knowledge.

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
| `--config <path>` | Override config file path |
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

Start a persistent HTTP server for fast AI agent access (21ms vs 980ms cold start):

```bash
# Start server
uteke-serve --port 8767

# Enable auto-routing in config
# [server]
# enabled = true

# CLI commands now route via HTTP automatically
uteke recall "what was that context?"  # 21ms!
uteke remember "New finding" --tags research
```

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
│  /stats /namespaces — CORS enabled, ~21ms recall    │
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

1. `./uteke.toml` (current directory)
2. Parent directories up to root
3. `~/.config/uteke/uteke.toml` (user-level)
4. Built-in defaults

```toml
store_path = "~/.uteke"
log_level = "info"
log_dir = "~/.uteke/logs"
default_namespace = "default"
```

Override config file path with `--config`:

```bash
uteke --config ./my-config.toml remember "project-specific note"
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

Uteke follows a demand-gated roadmap — we build what people actually use.

**Now (v0.0.4):** Server mode (21ms warm recall), contradiction detection, consolidation, bulk operations, namespace switching, CLI auto-routing to server
**Phase A (100+ stars):** Better embeddings, import from external sources, Hermes plugin, benchmark
**Phase B (500+ stars):** Python SDK (PyO3), Node.js SDK, editor integrations
**Phase C (1000+ stars):** Team features, cloud sync (opt-in), knowledge graph

See the [full blueprint](BLUEPRINT_V2.md) for details.

---

## License

[Apache License 2.0](LICENSE) — use it, fork it, ship it.

---

<p align="center">
  <strong>Local-first. Zero config. Your memory, your machine.</strong>
</p>
