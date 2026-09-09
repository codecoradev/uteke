<p align="center">
  <img src="docs/assets/uteke-banner.png" alt="Uteke: One memory. Every agent. Zero cloud." width="640" />
</p>

<h1 align="center">Uteke</h1>
<p align="center"><strong>One memory. Every agent. Zero cloud.</strong></p>
<p align="center">
  Give your AI a memory that never leaves your machine. Works with Claude, Cursor, Copilot, and any MCP-compatible agent.
</p>

<p align="center">
  <strong>98.4% LongMemEval-S recall@5</strong> · ~45 ms warm query · <strong>0 LLM tokens</strong> per query · CPU-only · fully offline
</p>

<p align="center">
  <a href="https://github.com/codecoradev/uteke/actions/workflows/ci.yml?branch=develop"><img src="https://github.com/codecoradev/uteke/actions/workflows/ci.yml/badge.svg?branch=develop" alt="CI" /></a>
  <a href="https://github.com/codecoradev/uteke/releases"><img src="https://img.shields.io/github/v/release/codecoradev/uteke?style=flat-square&color=green" alt="Latest Release" /></a>
  <a href="https://github.com/codecoradev/uteke/stargazers"><img src="https://img.shields.io/github/stars/codecoradev/uteke?style=flat-square&color=yellow" alt="GitHub Stars" /></a>
  <a href="https://opensource.org/licenses/Apache-2.0"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg?style=flat-square" alt="License: Apache 2.0" /></a>
  <img src="https://img.shields.io/badge/Rust-1.85+-orange.svg?style=flat-square" alt="Rust 1.85+" />
  <a href="https://github.com/codecoradev/uteke/pkgs/container/uteke"><img src="https://img.shields.io/badge/Docker-ready-blue.svg?style=flat-square" alt="Docker" /></a>
  <img src="https://img.shields.io/badge/recall-~45ms-brightgreen.svg?style=flat-square" alt="Recall ~45ms" />
  <a href="#-benchmarks--984-recall-on-longmemeval-s"><img src="https://img.shields.io/badge/LongMemEval--S_recall@5-98.4%25-crimson.svg?style=flat-square" alt="LongMemEval-S recall@5: 98.4%" /></a>
</p>

<p align="center">
  <strong>🇬🇧 English</strong> · <a href="README.id.md">🇮🇩 Bahasa Indonesia</a>
</p>

---

## ⚡ 30-Second Quick Start

```bash
# Install (macOS, Linux, Windows)
curl -sSL codecora.dev/uteke/install | sh

# Store a memory
uteke remember "Deploy v2.1 to staging at 3pm"

# Search it back: by meaning AND by keyword
uteke recall "when do we deploy?"
```

**That's it.** No API keys, no Python, no cloud required. First run downloads the embedding model (~200MB, one-time) and you're running.

Want your agent (Claude Code, Cursor, Hermes) to use it? One line:

```jsonc
// .mcp.json
{ "mcpServers": { "uteke": { "command": "uteke-mcp" } } }
```

<details>
<summary>📦 More install options & Docker</summary>

| Method | Command |
|--------|---------|
| **Homebrew** | `brew install codecoradev/tap/uteke` |
| **Cargo** | `cargo install uteke-cli` |
| **Docker** | `docker run -d -p 127.0.0.1:8767:8767 -v uteke-data:/data ghcr.io/codecoradev/uteke:latest` |
| **Binary** | [GitHub Releases](https://github.com/codecoradev/uteke/releases) (macOS, Linux, Windows) |
| **Windows (PowerShell)** | `powershell -ExecutionPolicy Bypass -Command "irm https://raw.githubusercontent.com/codecoradev/uteke/main/install.ps1 | iex"` |

📖 [Full install guide](INSTALL.md) · [Docker docs](docs/docker.md)
</details>

New to uteke (or you *are* an AI agent)? Run `uteke onboard`. It detects your
setup, asks which agent you use, and wires everything up. 📖 [Onboarding docs](docs/getting-started.md#interactive-onboarding)

---

## 📊 Benchmarks: 98.4% recall on LongMemEval-S

[LongMemEval-S](https://arxiv.org/abs/2410.10813) (ICLR 2025) hides the facts an
agent needs across ~115 chat sessions per question and checks whether retrieval
finds the evidence. 500 hand-curated questions, five memory abilities. Uteke runs
the full suite with **zero LLM calls in the retrieval path**: local embeddings,
one CPU, deterministic.

| Metric | uteke **v0.17.0** | agentmemory¹ | BM25-only¹ |
|---|---|---|---|
| **recall_any@5** (evidence in top-5) | **98.4%** | 95.2% | 86.2% |
| recall_any@10 | 98.8% | 98.6% | 94.6% |
| **recall_all@5** (all evidence, strict) | **88.0%**² | 88.2% MRR³ | n/a |
| LLM tokens / query | **0** | 0 | 0 |

<sub>¹ agentmemory's published numbers, same benchmark, same 500-question split (their recall_any@5 basis; verified apples-to-apples in our [head-to-head](docs/benchmarks.md#head-to-head-vs-published-systems)). ² Strict = *every* gold session in top-5; 65% of questions have multiple gold sessions. Mathematical ceiling 99.4%. ³ MRR, not recall_all (not directly comparable; shown for completeness).</sub>

<p align="center">
  <img src="docs/assets/longmemeval-recall-v017.png" alt="LongMemEval-S recall@5: uteke 98.4% (revalidated v0.17.0) vs MemPalace 96.6% and agentmemory 95.2% (raw results committed in-repo)" width="880" />
</p>

**By question category** (recall_any@5: the category-level story most tools don't show):

| knowledge-update | single-session | temporal | multi-session |
|:---:|:---:|:---:|:---:|
| **100%** | 96.7–98.2% | **99.2%** | 98.3% |

The hard part isn't finding *a* needle; every question's evidence lands in the
top-50 (**zero misses**). The residual gap is *ordering* when a question needs
several sessions at once: strict recall_all@5 is 88.0% against a 99.4% ceiling.

> **🎯 Don't trust our benchmark. Run it yourself.** The full harness is in this
> repo: public dataset, committed raw outputs for both releases, deterministic
> scoring you can recompute in ~20 lines of Python. No embedder needed to verify,
> ~$10 to re-run the whole 500 questions yourself.
> **👉 [benchmarks/longmemeval/REPRODUCING.md](benchmarks/longmemeval/REPRODUCING.md)**

Also built in: `uteke bench --counts 100,1000,10000` for latency/throughput on
your own machine. 📖 [Full benchmark docs](docs/benchmarks.md) · [RESULTS.md](benchmarks/longmemeval/RESULTS.md)

---

## 💡 What Can You Do With Uteke?

**🤖 Building AI agents?** Give them persistent memory without cloud dependencies. Your agent remembers user preferences, past decisions, and context across sessions, fully offline.

**👥 Working in a team?** Use [Rooms](docs/rooms.md) to share knowledge. Meeting notes, project decisions, architecture choices: searchable by everyone, attributed by author.

**🔒 Building for privacy-sensitive domains?** Healthcare, finance, legal: data stays on your machine. No API calls, no telemetry, no cloud. Local embeddings (ONNX, 768d).

**⌨️ Power user who lives in the terminal?** Uteke is your personal knowledge graph. Remember anything, recall by meaning, link related thoughts. All from the command line.

---

## 🔥 Why Uteke?

You just spent 2 hours explaining your codebase to ChatGPT. Next session? Blank slate. Again.

Every AI tool forgets. Context windows fill up, sessions end, and your AI starts over every single time. Uteke gives it persistent memory and keeps it on your machine.

| | **Uteke** | **Tool A** | **Tool B** | **Tool C** | **Tool D** | **Tool E** | **Tool F** | **Tool G** |
|---|---|---|---|---|---|---|---|---|
| **Language** | Rust (single binary) | Python (pip) | Python | TypeScript | TypeScript | Python | TypeScript | Go (single binary) |
| **Setup** | One binary (`curl \| sh`) | pip install + venv | pip + Docker + Qdrant | npm + iii-engine | npm (Node.js) | pip + Docker + Neo4j | Cloud or local binary | One binary |
| **API keys** | ❌ None | ⚠️ For remote embeddings | ✅ OpenAI/LLM | ✅ LLM key | ✅ LLM key | ✅ LLM key | ⚠️ Cloud only | ❌ None |
| **Works offline** | ✅ Fully | ⚠️ Optional | ❌ Cloud embedding | ❌ Needs LLM | ❌ Needs LLM | ❌ Needs LLM + vector DB | ✅ Local binary + Ollama | ✅ Fully |
| **Search** | **Fusion** (weighted RRF of vector + hybrid; hybrid = HNSW + FTS5 RRF) | sqlite-vec + FTS5 | Vector + Graph | Vector + Graph | Vector | Hybrid (semantic + keyword + graph) | Vector + rerank | **FTS5 only** |
| **Recall speed** | ~45ms | ~50ms+ | Network round-trip | Network round-trip | Network round-trip | Network round-trip | Network round-trip | ~Fast (local) |
| **Multi-agent** | ✅ **Rooms** (shared memory, cross-agent recall, author attribution) | ✅ Multi-agent surface | ❌ | ✅ Shared server | ✅ Multi-agent groups | ❌ | ❌ | ⚠️ Shared via MCP |
| **Time-travel** | ✅ Native point-in-time | ⚠️ Temporal triples | ❌ | ❌ | ❌ | ✅ Temporal graphs | ❌ | ❌ |
| **MCP server** | ✅ JSON-RPC + HTTP | ✅ stdio + SSE | ❌ | ✅ 54 MCP tools | ❌ | ✅ Graphiti MCP | ✅ Open-source MCP | ✅ stdio MCP |
| **Your data** | ✅ Never leaves machine | ✅ Local-first | ⚠️ Sent to LLM cloud | ✅ Local (iii-engine) | ⚠️ Sent to LLM cloud | ⚠️ Sent to LLM cloud | ⚠️ Cloudflare-hosted | ✅ Local |
| **License** | Apache 2.0 | MIT | Apache 2.0 | Apache 2.0 | Apache 2.0 | Apache 2.0 | MIT | MIT |

> **Note:** Tool labels (A–G) represent common categories of AI memory layers available as of August 2026. Capabilities are assessed from public documentation and may change. This table is a starting point for your own evaluation, not a definitive ranking.

> **Uteke vs Tool A (Python local-first):** Both offer local-first with semantic + FTS5 search. Uteke's edge: **single binary** (no Python runtime), **native time-travel** (vs temporal triples), **rooms**, and **zero runtime dependencies**.

> **Uteke vs Tool G (Go single binary):** Both are single-binary, offline, no-API-key, and both ship MCP servers. Tool G is **FTS5-only** (keyword search). Uteke adds **vector semantic search + RRF fusion + rooms + time-travel + graph relationships + smart decay + document engine + batch import**. Same simplicity thesis, more capabilities.

> **Uteke vs Tool F (TypeScript local mode):** Tool F now offers a local binary mode with Ollama support, a solid step toward offline-first. But it's still TypeScript/Node.js under the hood. Uteke is Rust: smaller footprint, faster startup, zero runtime. And Uteke has **hybrid search with FTS5** (Tool F's local mode uses vector-only, no keyword fallback).

> **Uteke vs Tools B/D/E:** Those are powerful, but all require cloud LLM API keys and Docker infrastructure. Your data goes to external LLM providers. Uteke runs fully offline with local ONNX embeddings. No Docker, no Python, no API keys.
>
> **Uteke vs Tool C:** Tool C has 54 MCP tools and multi-agent shared memory via a local engine. Uteke's edge: **zero dependencies** (no npm, no extra engine), **hybrid search** (Tool C lacks FTS5), and **time-travel queries**.

---

## ✨ Features

### Core Memory

| Feature | What it does |
|---------|-------------|
| 🧠 **Hybrid + Fusion Search** | Vector similarity + FTS5 full-text search, merged by Reciprocal Rank Fusion (RRF). Since v0.16.0, `fusion` (a weighted RRF of the vector and hybrid rankings) is the default recall strategy. Finds by meaning AND exact keywords. |
| 🏠 **Rooms** | **Multi-agent shared memory.** Group memories by context (meetings, projects, clients). Multiple agents read/write to the same room with author attribution. Cross-agent recall without manual sync. |
| ⏳ **Time-travel** | Recall memories as they existed at any point in time. `uteke recall "deploy" --at 2025-01-15` |
| 🏷️ **Rich Metadata** | Tags, entities, categories, key:value pairs on every memory. |
| 🧩 **Memory Types** | Typed categories (fact, procedure, decision, etc.) with auto-inference. |
| ✏️ **Partial Updates** | Update content, tags, metadata, importance, or type without full rewrite. |
| 📎 **Citations** | Source attribution on every memory (URL, file, user, import batch). |

### Search & Intelligence

| Feature | What it does |
|---------|-------------|
| 🔗 **Relationship Graph** | Link memories with typed edges (supersedes, contradicts, references). Auto-backlinks. |
| 🔗 **Cross-Entity Linking** | Bidirectional memory↔document references via `[[doc-slug]]` wikilinks. |
| 🤖 **Cosine Auto-Linking** | Automatically creates `similar_to` edges between related memories. |
| 📉 **Smart Decay** | Composite importance scoring. Pin what matters, let stale memories fade. |
| 📈 **Salience + Recency** | Dual-axis recall boost by memory type and age. |
| 🔍 **Orphan Detection** | Find disconnected, low-importance memories for cleanup. |
| 🌙 **Dream Cycle** | One-command maintenance: lint → backlinks → dedup → orphans. |
| 🧬 **Consolidation** | Merge near-duplicate room memories into fewer, denser records: segment-level planner, provenance trust policy, per-pair control. |

### Integrations

| Feature | What it does |
|---------|-------------|
| 🔌 **MCP Server** | JSON-RPC over stdio + Streamable HTTP. Works with Claude Code, Cursor, Hermes. |
| 🖥️ **Server Mode** | Persistent daemon: eliminates cold-start embedding load on every call. |
| 📂 **Batch Import** | Import entire directories with auto-strategy routing (document vs. memory extraction). |
| 📝 **Document Engine** | Wiki/knowledge base with `uteke doc create/get/list` and auto-chunking. |
| 📥 **Import/Export** | JSONL-based backup and restore. |
| 🔑 **View-Only API Keys** | Read-only tokens for safe GET-only access to the server. |
| 👤 **Author Types** | `human` vs `agent` attribution on every memory, across CLI, HTTP, and MCP. |

### Performance & Privacy

| Feature | What it does |
|---------|-------------|
| 📦 **Single Binary** | Zero dependencies. No Python, no API keys. Local-first by default. |
| 🐳 **Docker Ready** | Official image on GHCR. Run as a shared service for teams or cloud deployments. |
| 🔒 **Fully Offline** | Local ONNX embeddings (EmbeddingGemma Q4, 768d). No telemetry, no cloud. |
| ⚡ **Recall Cache** | LRU cache eliminates redundant embedding for repeated queries. |
| 🔥 **Tiered Memory** | Hot/Warm/Cold tracking with auto-cleanup of stale memories. |
| 🔄 **Embed Fallback** | Gracefully degrades to no-op embedder if local model fails (never crashes). |
| 👥 **Multi-Agent Namespaces** | Fully isolated memory per agent, zero overhead. |
| 📊 **Benchmarks** | Built-in `uteke bench` for perf testing. [See results](docs/benchmarks.md). |

---

## 🚀 How It Works

```mermaid
graph LR
    Input[User Query] --> Embed[Local ONNX Embedder<br/>768d, EmbeddingGemma Q4]
    Embed --> HNSW[HNSW Vector Index<br/>usearch]
    Embed --> FTS5[FTS5 Full-Text<br/>SQLite]
    HNSW --> RRF[Reciprocal Rank Fusion<br/>k=60]
    FTS5 --> RRF
    RRF --> Results[Ranked Results]

    style Input fill:#4a9eff,color:#fff
    style Results fill:#4aff9e,color:#000
    style RRF fill:#ff9e4a,color:#fff
```

**How hybrid search works:**
1. **HNSW** (usearch): finds by meaning ("deploy" matches "rollout")
2. **FTS5** (SQLite): finds by exact terms ("deploy" matches "deploy")
3. **RRF** (k=60): merges both ranked lists → best of both worlds

Everything runs in-process. No network. No cloud. No server required (unless you want server mode).

<details>
<summary>🐳 Deployment modes (local-first by default, Docker/server when you need it)</summary>

**Local-first (default)**: single binary, zero infrastructure.

```bash
curl -sSL codecora.dev/uteke/install | sh
uteke remember "first memory"
```

Your data stays in `~/.codecora/uteke/`.

**Docker / server mode**: for teams or remote agents.

```bash
docker run -d -p 127.0.0.1:8767:8767 -v uteke-data:/data ghcr.io/codecoradev/uteke:latest
# or: uteke serve --host 0.0.0.0 --port 8767
```

📖 [Docker setup guide](docs/docker.md) · [Server mode docs](docs/configuration.md#server-mode)
</details>

<details>
<summary>🏠 Rooms: multi-agent shared memory (example)</summary>

```bash
# Create a shared room
uteke room create "engineering" --description "Team decisions"

# Alice's agent stores a decision
uteke remember "We chose Redis for caching over Memcached" \
  --room engineering --author alice

# Bob's agent adds context
uteke remember "Redis cluster: 3 nodes, 2 replicas each" \
  --room engineering --author bob

# Any agent can recall the shared history
uteke recall "caching decision" --room engineering
```

Author attribution on every memory; cross-agent recall without manual sync.
📖 [Full Rooms documentation →](docs/rooms.md)
</details>

<details>
<summary>🔌 MCP config: connect to Claude Code, Cursor, Hermes</summary>

```jsonc
// .mcp.json (Claude Code, Cursor)
{ "mcpServers": { "uteke": { "command": "uteke-mcp" } } }
```

For Claude Desktop, Hermes, and HTTP transport, see [MCP docs](docs/mcp.md).
</details>

---

## 📚 Documentation

| | |
|---|---|
| **Getting started** | [Installation](INSTALL.md) · [Getting started](docs/getting-started.md) · [Onboarding](docs/getting-started.md#interactive-onboarding) |
| **Reference** | [CLI reference](docs/cli-reference.md) · [Configuration](docs/configuration.md) · [Docker](docs/docker.md) |
| **Integrations** | [MCP setup](docs/mcp.md) · Claude Code · Cursor · Hermes |
| **Benchmarks** | [docs/benchmarks.md](docs/benchmarks.md) · [RESULTS.md](benchmarks/longmemeval/RESULTS.md) · [Reproduce it yourself](benchmarks/longmemeval/REPRODUCING.md) |

---

## ❓ FAQ

<details>
<summary><strong>How is Uteke different from cloud-dependent memory tools?</strong></summary>

Many memory layers (Python-based or TypeScript-based) require cloud API keys (OpenAI/LLM) and external infrastructure (Docker, Postgres, Qdrant). Your data gets sent to a cloud LLM provider. Uteke is a single binary with zero API keys. All embeddings run locally via ONNX. Your data never leaves your machine. [See comparison table](#-why-uteke-).
</details>

<details>
<summary><strong>How is Uteke different from multi-tool MCP platforms?</strong></summary>

Some platforms offer dozens of MCP tools and multi-agent shared memory via a local engine. They're packed with integrations, but require npm, a separate runtime, and LLM API keys for embeddings. Uteke is Rust, zero dependencies, and works fully offline with local ONNX embeddings. If you want maximum integrations → those platforms. If you want privacy, speed, zero setup, and hybrid search → Uteke.
</details>

<details>
<summary><strong>How is Uteke different from other single-binary memory tools?</strong></summary>

Some single-binary tools share our philosophy: one binary, zero deps, MCP server, local-first. The key difference is **search**: most are **FTS5-only** (keyword matching). Uteke uses **hybrid search** (HNSW vector similarity + FTS5 + Reciprocal Rank Fusion), meaning you can search by *meaning*, beyond exact words. Uteke also adds rooms, time-travel, graph relationships, smart decay, document engine, and batch import.
</details>

<details>
<summary><strong>Does it really work offline?</strong></summary>

Yes. The embedding model (EmbeddingGemma Q4, 768d) downloads once (~200MB) on first run. After that, zero network calls. No telemetry. If the local model fails, Uteke degrades gracefully to a no-op embedder. It never crashes and never calls a cloud API.
</details>

<details>
<summary><strong>How fast is recall?</strong></summary>

~45ms as a CLI (measurements: 31ms avg @10K on the published bench host, 100–10K memories, flat with store size). No network round-trip because everything is local. The LRU recall cache eliminates redundant embedding computation for repeated queries.
</details>

<details>
<summary><strong>Can I use Uteke with my existing AI tools?</strong></summary>

Yes. Uteke ships with an MCP server that works with Claude Code, Cursor, and Hermes. You can also use the HTTP API directly in any language. [See MCP setup →](docs/mcp.md)
</details>

<details>
<summary><strong>Is it production-ready?</strong></summary>

Uteke is at v0.17.0 with 200+ tests, CI/CD on every commit, and a benchmark harness. It's used in production by the CodeCora team and other early adopters. Still in 0.x, so expect rough edges, but the core is stable.
</details>

---

## 🗺️ Roadmap & Editions

| | **Uteke OSS** (this repo) | **Uteke Cloud** |
|---|---|---|
| Retrieval quality | ✅ Full (identical engine) | ✅ Identical (parity-benchmarked) |
| License | Apache-2.0, self-host | Managed service |
| LLM-powered answering (recency-aware fact resolution, abstention) | BYOK | Included |
| Multi-workspace, dashboard, backups | DIY | ✅ Included |
| Price | Free | *Coming soon* |

The open-source engine stays full-capability. Nothing about the benchmark
results above is paywalled. Cloud adds hosted convenience on top.

---

## 🤝 Contributing

```bash
cargo build --workspace        # Build
cargo test --workspace         # Test (200+ tests)
cargo clippy -- -D warnings    # Lint
cargo fmt                      # Format
```

Contributions welcome! Read [CONTRIBUTING.md](CONTRIBUTING.md) for the full guide.

---

## 📄 License

[Apache License 2.0](LICENSE). Use it, fork it, ship it.

---

## ⭐ Star History

<p align="center">
  <img src="https://s3.ajianaz.dev/hermes/codecoradev/uteke/star-history.png" alt="Uteke Star History" width="720" />
</p>

---

<p align="center">
  <strong>Found this useful?</strong> ⭐ Star this repo. It helps others discover Uteke.
</p>
<p align="center">
  <a href="https://github.com/codecoradev/uteke/stargazers">
    <img src="https://img.shields.io/github/stars/codecoradev/uteke?style=social" alt="Star this repo" />
  </a>
</p>
