<p align="center">
  <img src="https://img.shields.io/badge/Uteke-🧠-blue" alt="Uteke" />
</p>

<h1 align="center">Uteke</h1>
<p align="center"><strong>Local-first memory for AI agents — written in Rust</strong></p>
<p align="center">
  <a href="https://github.com/ajianaz/uteke/actions/workflows/ci.yml?branch=develop"><img src="https://github.com/ajianaz/uteke/actions/workflows/ci.yml/badge.svg?branch=develop" alt="CI" /></a>
  <img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT" />
  <img src="https://img.shields.io/badge/Rust-1.75+-orange.svg" alt="Rust 1.75+" />
  <img src="https://img.shields.io/badge/status-v0.1.0-green.svg" alt="v0.1.0" />
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

# Semantic search
uteke recall "what deployment is coming up?"

# Get stats
uteke stats
```

**That's it.** No API keys. No server. No config. First run downloads the embedding model (~90MB) and you're good to go.

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
| **License** | MIT | Apache 2.0 | Apache 2.0 | MIT |

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
| `search` | Keyword text search | `uteke search "monorepo" --limit 10` |
| `list` | List memories with optional tag filter | `uteke list --tag project --limit 20` |
| `get` | Get a single memory by ID | `uteke get <uuid>` |
| `forget` | Delete a memory by ID | `uteke forget <uuid>` |
| `stats` | Show memory store statistics | `uteke stats` |
| `completions` | Generate shell completion scripts | `uteke completions bash` |

### Global Flags

| Flag | Description |
|------|-------------|
| `--store <path>` | Override store location (default: `~/.uteke`) |
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

---

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                    CLI (clap)                        │
│                  uteke-cli crate                     │
├─────────────────────────────────────────────────────┤
│                    Uteke API                         │
│                  uteke-core crate                    │
├──────────┬──────────────────┬────────────────────────┤
│   ONNX   │      HNSW        │       SQLite           │
│ Embedding│  Vector Index    │    Metadata Store      │
│ (768d)   │  (Fast ANN)      │    (rusqlite)          │
├──────────┴──────────────────┴────────────────────────┤
│              ~/.uteke/ (local storage)               │
│   uteke.db  │  config.toml  │ models/embeddinggemma/ │
└─────────────────────────────────────────────────────┘
```

| Component | Technology | Detail |
|-----------|-----------|--------|
| Language | Rust (no unsafe) | Memory-safe, fast, single binary |
| Vector Index | HNSW | Fast approximate nearest neighbor search |
| Storage | SQLite (rusqlite) | Embedded, zero-config, battle-tested |
| Embedding | EmbeddingGemma Q4 ONNX | 768d vectors, multilingual, downloaded on first run |
| CLI | clap | Standard Rust CLI framework |

**How it works:**
1. `remember` → text is embedded into a 768d vector via ONNX → stored in SQLite + indexed in HNSW
2. `recall` → query is embedded → HNSW finds nearest neighbors → returns ranked results
3. `search` → SQLite LIKE-based keyword search (fast, deterministic)
4. Everything lives in `~/.uteke/` — fully local, fully yours

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

On first run, Uteke creates `~/.uteke/config.toml`:

```toml
[store]
# path = "~/.uteke"  # Default store location

[embedding]
# model = "embeddinggemma-q4"
# max_seq_length = 256
```

Override store path per-command with `--store`:

```bash
uteke --store /path/to/project/.uteke remember "project-specific note"
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

**Now (v0.1.0):** Core engine — store, recall, search, CLI, Python wrapper
**Phase A (100+ stars):** Better embeddings, dedup, import/export, remote embedding opt-in
**Phase B (500+ stars):** Python SDK (PyO3), Node.js SDK, editor integrations
**Phase C (1000+ stars):** Team features, cloud sync (opt-in), knowledge graph

See the [full blueprint](BLUEPRINT_V2.md) for details.

---

## License

[MIT](LICENSE) — use it, fork it, ship it.

---

<p align="center">
  <strong>Local-first. Zero config. Your memory, your machine.</strong>
</p>
