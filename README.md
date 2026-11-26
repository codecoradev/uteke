<p align="center">
  <img src="docs/assets/uteke-icon.png" alt="Uteke owl logo" width="88" />
</p>

<h1 align="center">Uteke</h1>

<p align="center">
  <strong>Long-term memory for any AI agent.</strong><br />
  One Rust binary, runs offline, no API key. Self-host it free, or let us host it on <a href="https://uteke.app/?utm_source=github&utm_medium=readme&utm_campaign=prove-the-recall">Uteke Cloud</a>.
</p>

<p align="center">
  <a href="https://github.com/codecoradev/uteke/actions/workflows/ci.yml?branch=develop"><img src="https://github.com/codecoradev/uteke/actions/workflows/ci.yml/badge.svg?branch=develop" alt="CI" /></a>
  <a href="https://github.com/codecoradev/uteke/releases"><img src="https://img.shields.io/github/v/release/codecoradev/uteke?style=flat-square&color=green" alt="Latest Release" /></a>
  <a href="https://opensource.org/licenses/Apache-2.0"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg?style=flat-square" alt="License: Apache 2.0" /></a>
  <a href="https://github.com/codecoradev/uteke/stargazers"><img src="https://img.shields.io/github/stars/codecoradev/uteke?style=flat-square&color=yellow" alt="GitHub Stars" /></a>
  <a href="https://hub.docker.com/r/codecoradev/uteke"><img src="https://img.shields.io/docker/pulls/codecoradev/uteke?style=flat-square&logo=docker&logoColor=white&label=Docker%20pulls&color=2496ED" alt="Docker Hub pulls" /></a>
  <a href="https://github.com/codecoradev/uteke/releases"><img src="https://img.shields.io/github/downloads/codecoradev/uteke/total?style=flat-square&logo=github&label=release%20downloads&color=blueviolet" alt="GitHub release downloads" /></a>
  <a href="#benchmarks"><img src="https://img.shields.io/badge/LongMemEval--S_recall@5-98.2%25-1F908C.svg?style=flat-square" alt="LongMemEval-S recall@5: 98.2%" /></a>
</p>

<p align="center">
  <a href="#quick-start">Quick start</a> ·
  <a href="#benchmarks">Benchmarks</a> ·
  <a href="docs/mcp.md">Connect your agent</a> ·
  <a href="#documentation">Docs</a> ·
  <a href="https://uteke.app/?utm_source=github&utm_medium=readme&utm_campaign=prove-the-recall">Uteke Cloud</a> ·
  <a href="README.id.md">Bahasa Indonesia</a>
</p>

---

## Quick start

```bash
# Install (macOS Apple Silicon, Linux; Windows: see below)
curl -sSL codecora.dev/uteke/install | sh

# Store a memory
uteke remember "Deploy v2.1 to staging at 3pm"

# Search it back: by meaning AND by keyword
uteke recall "when do we deploy?"

# Look at what is stored, in a read-only browser page
uteke view
```

<p align="center">
  <img src="docs/assets/uteke-demo.gif" width="760" alt="Terminal demo: uteke remember stores two notes, then uteke recall 'which port is the database on?' returns the Postgres port note and 'what is our release process?' returns the deploy note, with no shared keywords." />
</p>

No API keys, no Python, no cloud account. The first run downloads the embedding model (~200 MB, one time).

Give your agent (Claude Code, Cursor, Hermes) the same memory with one line:

```jsonc
// .mcp.json
{ "mcpServers": { "uteke": { "command": "uteke-mcp" } } }
```

New to Uteke, or you *are* an AI agent? Run `uteke onboard`. It detects your setup, asks which agent you use, and wires it up. 📖 [Onboarding](docs/getting-started.md#interactive-onboarding)

Rather not run anything locally? **[Uteke Cloud](https://uteke.app/?utm_source=github&utm_medium=readme&utm_campaign=prove-the-recall)** runs the same engine for you: add one URL to Claude, sign in, done. Free during the open beta.

<details>
<summary>📦 More install options & Docker</summary>

| Method | Command |
|--------|---------|
| **Homebrew** | `brew install codecoradev/tap/uteke` (macOS Apple Silicon & Linux) |
| **Cargo** | `cargo install uteke-cli` |
| **Docker** | `docker run -d -p 127.0.0.1:8767:8767 -v uteke-data:/data codecoradev/uteke:latest --host 0.0.0.0` |
| **Binary** | [GitHub Releases](https://github.com/codecoradev/uteke/releases) (macOS, Linux, Windows) |
| **Windows (PowerShell)** | `powershell -ExecutionPolicy Bypass -Command "irm https://raw.githubusercontent.com/codecoradev/uteke/main/install.ps1 \| iex"` |

📖 [Full install guide](INSTALL.md) · [Docker docs](docs/docker.md)
</details>

---

## Benchmarks

[LongMemEval-S](https://arxiv.org/abs/2410.10813) (ICLR 2025) hides the facts an agent needs across ~115 chat sessions per question and checks whether retrieval finds them. 500 hand-curated questions, five memory abilities. Uteke runs the full suite with **zero LLM calls in the retrieval path**: local embeddings, one CPU, deterministic.

| Metric | Uteke **v0.24.0** | MemPalace¹ | agentmemory¹ |
|---|---|---|---|
| **recall_any@5** (evidence in top 5) | **98.2%** | 96.6%⁴ | 95.2%³ |
| recall_any@10 | 98.8% | — | 98.6%³ |
| **strict recall_all@5** (all evidence in top 5) | <b>87.6%</b>² | not published | 84.7%³ |
| LLM calls in retrieval | **0** | 0 (raw mode)⁴ | 0 |

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/longmemeval-comparison-dark.png" />
    <source media="(prefers-color-scheme: light)" srcset="docs/assets/longmemeval-comparison-light.png" />
    <img src="docs/assets/longmemeval-comparison-light.png" width="760" alt="LongMemEval-S recall_any@5, 500 questions: Uteke 98.2%, MemPalace 96.6%, agentmemory 95.2%. Axis starts at 0." />
  </picture>
</p>

<sub>¹ Their published numbers on LongMemEval-S, each on its own recall_any@5 basis ([head-to-head notes](docs/benchmarks.md#head-to-head-vs-published-systems)); evaluation setups differ. Read from their READMEs on 2026-11-16. ² Strict = *every* gold session in the top 5; 65% of questions have more than one. Max possible 99.4%. v0.24.0 scores 87.6% (438 of 500) where v0.16.0 to v0.20.0 scored 88.0% (440): two questions moved one of their two gold sessions across the rank-5 boundary while their top-50 sets stayed identical and recall_any@5 did not change, the adjacent-rank jitter that [RESULTS.md](benchmarks/longmemeval/RESULTS.md#v0240-validation--done-2026-11-17) documents. ³ agentmemory: 95.2% / 98.6% is its earlier hybrid run, which its own page labels historical (older pipeline, 500 questions). Its current README, measured 2026-10-28, reports 84.7% strict recall_all@5 and 96.2% any-hit@5 for the keyless BM25 component alone, scoring 470 answerable questions. Different denominator and pipeline, so not a head-to-head with ours. ⁴ MemPalace: 96.6% is its raw mode (semantic search, no LLM, 500 questions). It also reports 98.4% for a hybrid mode on a held-out 450-question subset (50 used for tuning) and ≥99% with an LLM rerank; neither is the same split or setup, so neither is in the table.</sub>

<details>
<summary>Stability across releases and results per question type</summary>

**Stable across releases** (full 500 questions, scored from committed raw, [RESULTS.md](benchmarks/longmemeval/RESULTS.md#release-stability-full-500-question-basis-scored-from-committed-raw)):

| Release | v0.16.0 | v0.17.0 | v0.19.0 | v0.20.0 | fix 32a5cf3b | v0.24.0 |
|---|---|---|---|---|---|---|
| recall_any@5 | 98.2% | 98.4% | 98.2% | 98.2% | 98.2% | 98.2% |
| strict recall_all@5 | 88.0% | 88.0% | 88.0% | 88.0% | 88.0% | 87.6% |

<sub>Re-validated 2026-11-08 after the #1690 import fix: strict@5 identical (88.0%), strict@10 +1 question (95.4%), zero misses@50 — four questions shifted partial gold ordering, none lost all evidence. The v0.24.0 column is the official release binary (2026-11-17): recall_any@5 and recall_any@10 are unchanged (98.2%, 98.8%), strict@10 is 95.2%, zero misses@50, and strict@5 differs by the two boundary questions in note ².</sub>

**By question type** (recall_any@5, v0.24.0, the 470 non-abstention questions; identical to v0.20.0):

| knowledge-update | single-session | temporal | multi-session |
|:---:|:---:|:---:|:---:|
| **100%** | 96.7–98.2% | 98.4% | 98.3% |

Every question's evidence lands in the top 50 (zero misses). What's left is ordering when a question needs several sessions at once: strict recall_all@5 is 87.6% against a 99.4% ceiling, and most misses lose exactly one session.

</details>

> **🎯 Don't trust our benchmark. Run it yourself.** Public dataset, committed raw outputs for every validated release, scoring you can recompute in ~20 lines of Python. Verifying the committed results takes a minute and needs no embedder; re-running all 500 questions costs about $10.
> **👉 [benchmarks/longmemeval/REPRODUCING.md](benchmarks/longmemeval/REPRODUCING.md)** · 🧾 Receipts: [v0.24.0 run](benchmarks/longmemeval/receipts/v0.24.0-default-500q/) · [v0.19.0 run](benchmarks/longmemeval/receipts/v0.19.0-default-500q/)

**LoCoMo.** LoCoMo end-to-end QA measures answer accuracy, and the result depends on the LLM answerer and the LLM judge used. In an internal comparison, identical answers were scored up to 8 points apart when only the judge model was swapped. For that reason we do not publish a single LoCoMo number in this README. The full table, the per-judge figures and the receipts are in [benchmarks/locomo/RESULTS.md](benchmarks/locomo/RESULTS.md). Retrieval recall, which needs no judge, remains the metric we rely on.

`uteke bench --counts 100,1000,10000` measures latency and throughput on your own machine. 📖 [Benchmark docs](docs/benchmarks.md)

---

<p align="center">
  <a href="benchmarks/longmemeval/RESULTS.md">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="docs/assets/longmemeval-breakdown-dark.png" />
      <source media="(prefers-color-scheme: light)" srcset="docs/assets/longmemeval-breakdown-light.png" />
      <img src="docs/assets/longmemeval-breakdown-light.png" width="880" alt="LongMemEval-S, 500 questions, Uteke v0.20.0: recall_any@5 98.2%, strict recall_all@10 95.2%, coverage@5 94.4%, strict recall_all@5 88.0% (max possible 99.4%). Keyword search alone 91.4% vs Uteke's fused default 98.2%." />
    </picture>
  </a>
</p>

<p align="center">
  <i>The v0.20.0 run, all 500 LongMemEval-S questions, no LLM call in retrieval (v0.24.0 is in the table above: strict recall_all@5 87.6%, coverage@5 94.2%). Raw outputs are committed: <a href="benchmarks/longmemeval/REPRODUCING.md">check them in a minute, or re-run everything for about $10</a>.</i>
</p>

---

## Why Uteke

On Monday you tell your agent how you deploy. By Thursday it has no idea. Every session starts from zero, and every agent keeps its own half-memory.

Uteke gives all of them one long-term memory that runs on your own machine:

- **Verifiable recall.** [98.2% recall@5 on LongMemEval-S](#benchmarks), with the stricter numbers shown next to it and the raw outputs in this repo.
- **Fast and cheap to query.** ~45 ms warm recall, 0 LLM tokens per query, CPU only.
- **One binary.** No Python, no Docker, no vector database, no API key. The embedding model (~200 MB) downloads once.
- **One memory, every agent.** Claude Code, Claude Desktop, Cursor, Codex, Hermes and any MCP client share the same store. [Rooms](docs/rooms.md) record which agent or person wrote what.
- **Finds by meaning and by keyword.** Local vectors and SQLite full-text search, fused into one ranking ([how it works](#how-it-works)).
- **Remembers over time.** Point-in-time recall (`--at`), a relationship graph, a document engine, and decay for stale memories.
- **Private by default.** No telemetry. Secrets such as API keys are masked before a memory is stored.
- **Open source.** Apache-2.0, cosign-signed releases.

---

## Self-host or Cloud

Same engine, two ways to run it.

| | **Self-host** (this repo) | **[Uteke Cloud](https://uteke.app/?utm_source=github&utm_medium=readme&utm_campaign=prove-the-recall)** |
|---|---|---|
| Engine and recall | Full, nothing paywalled | Same engine |
| Runs on | Your laptop or your server | Hosted for you |
| Connect an agent | `uteke-mcp` in `.mcp.json` | Add `https://uteke.app/mcp` to Claude, sign in |
| Your data | Never leaves your machine | Stored by us; export any time |
| Price | Free, Apache-2.0 | Free during the open beta ([limits](https://uteke.app/pricing?utm_source=github&utm_medium=readme&utm_campaign=prove-the-recall)) |

**Choose self-host** if your data must stay on your machine, or you want to run Uteke inside your own infrastructure.
**Choose Cloud** if you want one memory across several devices and agents without running a server.

---

## Connect your agent

| Client | How |
|---|---|
| **Claude Code** | `.mcp.json`: `{ "mcpServers": { "uteke": { "command": "uteke-mcp" } } }` · [details](docs/mcp.md#claude-code) |
| **Claude Desktop** | [config snippet](docs/mcp.md#claude-desktop) |
| **Cursor** | same `.mcp.json` as Claude Code · [details](docs/mcp.md#cursor) |
| **Codex** | `uteke connect codex` installs recall and capture hooks · [details](docs/integrations/codex.md) |
| **Hermes** | stdio or HTTP · [details](docs/mcp.md#hermes) |
| **Any MCP client** | stdio (`uteke-mcp`) or Streamable HTTP (`uteke-serve`) · [transports](docs/mcp.md#transport-comparison) |
| **Claude on the web, desktop or mobile** | add `https://uteke.app/mcp` as a connector, sign in ([Uteke Cloud](https://uteke.app/?utm_source=github&utm_medium=readme&utm_campaign=prove-the-recall)) |
| **No MCP?** | any agent that can run a shell command can call the `uteke` CLI |

<details>
<summary>🏠 Rooms: several agents, one shared memory (example)</summary>

```bash
# Create a shared room
uteke room create engineering --title "Team decisions"

# Alice's agent stores a decision
uteke remember "We chose Redis for caching over Memcached" \
  --room engineering --author alice

# Bob's agent adds context
uteke remember "Redis cluster: 3 nodes, 2 replicas each" \
  --room engineering --author bob

# Any agent can recall the shared history
uteke room recall engineering "caching decision"
```

Author attribution on every memory; cross-agent recall without manual sync. 📖 [Rooms](docs/rooms.md)
</details>

---

## Features

| Memory | Search | Integrations | Privacy & operations |
|---|---|---|---|
| Rooms with author attribution | Fusion of vector + full-text (default) | MCP over stdio and Streamable HTTP | Fully offline, no telemetry |
| Point-in-time recall (`--at`) | Relationship graph and auto-links | Server mode and Docker image | Secrets masked before storage |
| Tags, entities, typed memories | Salience and recency boosts | Document engine (`uteke doc`) | Signed upgrades (cosign) |
| Partial updates and citations | Dream cycle: dedup, backlinks, orphans | Batch import, full export/import | Two vector engines (`usearch`, `vecq`) |

<details>
<summary>All features with details</summary>

### Core memory

| Feature | What it does |
|---------|-------------|
| 🧠 **Hybrid + Fusion Search** | Vector similarity + FTS5 full-text search, merged by Reciprocal Rank Fusion (RRF). Since v0.16.0, `fusion` (a weighted RRF of the vector and hybrid rankings) is the default recall strategy. Finds by meaning AND exact keywords. |
| 🏠 **Rooms** | **Multi-agent shared memory.** Group memories by context (meetings, projects, clients). Multiple agents read/write to the same room with author attribution. Cross-agent recall without manual sync. |
| ⏳ **Time-travel** | Recall memories as they existed at any point in time. `uteke recall "deploy" --at 2025-01-15T00:00:00Z` |
| 🏷️ **Rich Metadata** | Tags, entities, categories, key:value pairs on every memory. |
| 🧩 **Memory Types** | Typed categories (fact, procedure, decision, etc.) with auto-inference. |
| ✏️ **Partial Updates** | `uteke update <id>` changes content, tags, importance, pin or type in place. |
| 📎 **Citations** | Source attribution on every memory (URL, file, user, import batch). |
| 👤 **Author** | Who wrote a memory, with or without a room (`--author`), plus `human` vs `agent` type. |
| 🚫 **Rejected Values** | `uteke forget <id> --reason wrong` records a tombstone: every write path (remember, update, import, supersede) refuses that value again, matched after normalization, until `uteke tombstone remove`. |

### Search & intelligence

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
| 📂 **Batch Import** | Import entire directories with auto-strategy routing (document vs. memory extraction). `--defer-embed` stores a JSONL file first and embeds later in portions (`repair --reembed --limit`). |
| 🪝 **Agent Hooks** | `uteke connect claude\|codex\|cursor` installs recall and capture hooks in the agent app (`pi`/`hermes`: the memory-provider plugin); `uteke disconnect` removes exactly them. See [Claude Code](docs/integrations/claude-code.md), [Codex](docs/integrations/codex.md), [Cursor](docs/integrations/cursor.md). |
| 🎙️ **Raw Capture** | Queue conversation turns with no LLM call (`uteke capture`, `POST /capture`, MCP `uteke_capture`); `extract-queue run` or the server's schedule turns them into memories later, on a budget. Opt-in. |
| 📝 **Document Engine** | Wiki/knowledge base with `uteke doc create/get/list` and auto-chunking. |
| 📥 **Import/Export** | JSONL backup and restore; `export --full` / `import --full` move the whole store: documents, rooms, the graph and each memory's author and provenance (vectors are rebuilt on the target). |
| 🔑 **View-Only API Keys** | Read-only tokens for safe GET-only access to the server. |

### Performance & privacy

| Feature | What it does |
|---------|-------------|
| 📦 **Single Binary** | Zero dependencies. No Python, no API keys. Local-first by default. |
| 🐳 **Docker Ready** | Official multi-arch image on [Docker Hub](https://hub.docker.com/r/codecoradev/uteke). Run as a shared service for teams or cloud deployments. |
| 🔒 **Fully Offline** | Local ONNX embeddings (EmbeddingGemma Q4, 768d). No telemetry, no cloud. The only network call is a daily release check, off with `UTEKE_NO_UPDATE_CHECK=1`. |
| 🧱 **Fenced MCP Output** | MCP read tools wrap stored text in `<uteke-stored-data>` so an agent treats recalled memories as data, not instructions (`UTEKE_MCP_FENCE=0` turns it off). |
| 🛡️ **Privacy Filter** | Secrets (API keys, tokens, `.env` values) are masked before a memory is embedded or stored; `[privacy] mode = "strict"` refuses the write instead. |
| ✍️ **Signed Upgrades** | `uteke upgrade` verifies the release's cosign (Sigstore keyless) signature before installing. |
| ⚡ **Recall Cache** | LRU cache eliminates redundant embedding for repeated queries. |
| 🔥 **Tiered Memory** | Hot/Warm/Cold tracking with auto-cleanup of stale memories. |
| 🧮 **Two Vector Engines** | `usearch` (HNSW, default) or `vecq` (4-bit quantized, pure Rust, ~3x smaller index), chosen at runtime. |
| 🔄 **Embed Fallback** | Gracefully degrades to no-op embedder if local model fails (never crashes). |
| 📊 **Benchmarks** | Built-in `uteke bench` for perf testing. [See results](docs/benchmarks.md). |

</details>

---

## How it works

```mermaid
graph LR
    Input[Query] --> Embed[Local embedder<br/>EmbeddingGemma Q4, 768d]
    Input --> FTS5[Full-text search<br/>SQLite FTS5]
    Embed --> Vec[Vector index<br/>usearch or vecq]
    Vec --> Hybrid[Hybrid<br/>vector + FTS5, RRF k=60]
    FTS5 --> Hybrid
    Vec --> Fusion[Fusion<br/>vector x1.7 + hybrid x1]
    Hybrid --> Fusion
    Fusion --> Boost[Salience + recency]
    Boost --> Results[Ranked results]

    style Input fill:#131A15,color:#F3F8F4
    style Fusion fill:#1F908C,color:#fff
    style Results fill:#0B6667,color:#fff
```

1. **Vector index** finds by meaning ("deploy" matches "rollout").
2. **FTS5** finds by exact terms ("deploy" matches "deploy").
3. **Hybrid** merges the two lists with Reciprocal Rank Fusion (k=60).
4. **Fusion** merges the vector ranking (weight 1.7) with the hybrid ranking (weight 1), then adds small salience and recency boosts.

Everything runs in-process: no network, no server required. `--strategy vector | fts5 | hybrid | graph` picks a single stage instead.

<details>
<summary>🐳 Deployment modes</summary>

**Local-first (default)**: single binary, zero infrastructure. Your data stays in `~/.codecora/uteke/`.

**Docker / server mode**: for teams or remote agents.

```bash
docker run -d -p 127.0.0.1:8767:8767 -v uteke-data:/data codecoradev/uteke:latest --host 0.0.0.0
# or, without Docker: uteke-serve --port 8767
```

📖 [Docker setup](docs/docker.md) · [Server mode](docs/configuration.md#server-mode) · [Remote access](docs/mcp.md#remote-access-vps--server--domain)
</details>

---

## Documentation

| | |
|---|---|
| **Getting started** | [Installation](INSTALL.md) · [Getting started](docs/getting-started.md) · [Onboarding](docs/getting-started.md#interactive-onboarding) |
| **Reference** | [CLI reference](docs/cli-reference.md) · [Configuration](docs/configuration.md) · [Docker](docs/docker.md) · [HTTP API](docs/api-reference.md) |
| **Concepts** | [Viewer](docs/viewer.md) · [Rooms](docs/rooms.md) · [Organizing memories](docs/organizing-memories.md) · [Architecture](docs/architecture.md) |
| **Integrations** | [MCP setup](docs/mcp.md) (Claude Code, Claude Desktop, Cursor, Hermes) · [Hermes](docs/integrations/hermes.md) · [Claude Code](docs/integrations/claude-code.md) / [Codex](docs/integrations/codex.md) / [Cursor](docs/integrations/cursor.md) hooks |
| **Security** | [SECURITY.md](SECURITY.md) (vulnerability reports, release signing) |
| **Benchmarks** | [docs/benchmarks.md](docs/benchmarks.md) · [RESULTS.md](benchmarks/longmemeval/RESULTS.md) · [Reproduce it yourself](benchmarks/longmemeval/REPRODUCING.md) |

---

## FAQ

<details>
<summary><strong>"Fully offline", but there's a Uteke Cloud?</strong></summary>

The engine in this repo is complete and runs fully offline; nothing in it calls the Cloud. Uteke Cloud is a separate, optional hosted service that runs the same engine for people who don't want to run anything locally, and you can export your data from it any time. The open-source engine stays full-capability, and none of the benchmark results above depend on the Cloud.
</details>

<details>
<summary><strong>Why not just keep notes in a Markdown file?</strong></summary>

A file works until you need to find something by meaning: "ship the release" won't match "deploy v2.1" with grep. Uteke searches by meaning and by keyword at once, keeps who wrote each memory, and works the same for every agent you use, instead of one file per tool.
</details>

<details>
<summary><strong>How is Uteke different from cloud-dependent memory tools?</strong></summary>

Many memory layers need cloud API keys (OpenAI or another LLM) and extra infrastructure (Docker, Postgres, Qdrant), and your data is sent to an LLM provider. Uteke is a single binary with no API keys; embeddings run locally via ONNX. [See the benchmarks](#benchmarks).
</details>

<details>
<summary><strong>How is Uteke different from other single-binary memory tools?</strong></summary>

Most of them are full-text only (keyword matching). Uteke fuses vector search with full-text search, so you can search by meaning as well as exact words, and adds rooms, time-travel, a relationship graph, decay, a document engine and batch import.
</details>

<details>
<summary><strong>Does it really work offline?</strong></summary>

Yes. The embedding model (EmbeddingGemma Q4, 768d) downloads once (~200 MB) on first run. After that, memory operations make no network calls and there is no telemetry. The binaries check GitHub for a new release once a day; set `UTEKE_NO_UPDATE_CHECK=1` to turn that off. If the local model fails, Uteke degrades to a no-op embedder; it never calls a cloud API.
</details>

<details>
<summary><strong>How fast is recall?</strong></summary>

~45 ms as a CLI (31 ms average at 10K memories on the published bench host; flat from 100 to 10K memories). No network round-trip, and an LRU cache skips re-embedding repeated queries.
</details>

<details>
<summary><strong>Is it production-ready?</strong></summary>

Uteke has 1,300+ tests, mutation testing before each release, CI on every pull request, cosign-signed releases and a benchmark harness. It's used in production by the CodeCora team and other early adopters. It is still 0.x (see the release badge), so expect rough edges, but the core is stable.
</details>

---

## Contributing

```bash
cargo build --workspace                                # Build
cargo test --workspace                                 # Test
cargo clippy --workspace --all-targets -- -D warnings  # Lint
cargo fmt --all                                        # Format
```

Contributions welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) for the full guide, or say hi in [Discussions](https://github.com/codecoradev/uteke/discussions).

## License

[Apache License 2.0](LICENSE). Use it, fork it, ship it.

---

<p align="center">
  <a href="https://www.star-history.com/?repos=codecoradev%2Futeke&type=timeline&legend=top-left">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=codecoradev/uteke&type=timeline&theme=dark&legend=top-left" />
      <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=codecoradev/uteke&type=timeline&theme=light&legend=top-left" />
      <img alt="Uteke star history" src="https://api.star-history.com/chart?repos=codecoradev/uteke&type=timeline&legend=top-left" width="600" />
    </picture>
  </a>
</p>

<p align="center">
  If Uteke helps your agents remember, a ⭐ helps other people find it.
</p>
