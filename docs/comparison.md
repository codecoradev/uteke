---
title: Comparison
---

# Choosing an AI Memory Layer in 2026: A Practical Comparison Framework

*First published August 2026. **Last reviewed 2026-11-16.** Uteke's own claims on this page were checked against the repository at that date; claims about other tools were read from their own repositories on the same date, and each one is marked with its source. Tools evolve fast: verify current specs before making decisions.*

> **A capability-first guide to evaluating memory layers for AI agents, without the marketing fog.**

---

## The Problem Every AI Builder Hits

You're building an AI agent. It's smart, fast, and helpful. But every time the session ends, it forgets everything. The user's preferences. Last week's decisions. The architecture you spent hours explaining. Gone.

So you search for "AI memory layer" and find yourself in a jungle of options, each claiming to be the best. Cloud-hosted platforms. Self-hosted libraries. Research prototypes. Single-binary tools. How do you choose?

This guide gives you a **practical evaluation framework**, not a brand shoot-out. We'll cover:

1. The **decision dimensions** that actually matter
2. A **capability matrix** you can fill in for any tool
3. How **Uteke** (our contribution) fits the landscape

---

## Uteke at a Glance

| Capability | Uteke | What it means |
|---|---|---|
| **Architecture** | Single Rust binary | `curl \| sh && done`. No Docker, Python, or database server. |
| **Data sovereignty** | Fully offline | Local ONNX embeddings (768d). No telemetry. The only network call is a daily release check, off with `UTEKE_NO_UPDATE_CHECK=1`. |
| **Search** | Hybrid (Vector + FTS5 + RRF) | Finds by meaning AND exact keywords. Reciprocal Rank Fusion (k=60) merges both. |
| **Recall speed** | ~45ms warm recall (31ms average at 10K memories) | Local execution, no network round-trip. See [Benchmarks](/benchmarks) for the run and hardware. |
| **Multi-agent** | Rooms | Shared memory spaces with author attribution and cross-agent recall. |
| **Time-travel** | Native point-in-time | `uteke recall "deploy" --at 2025-01-15T00:00:00Z` — queries historical state. |
| **Extraction** | Offline (default) + LLM (optional) | Rule-based extraction works zero-config. Upgrade to LLM if desired. |
| **Integration** | MCP + CLI + HTTP API | JSON-RPC (stdio + HTTP), full CLI, REST API with view-only API keys. |
| **Dependencies** | Zero runtime deps | No Docker, Postgres, Neo4j, Python, or cloud account needed. |
| **License** | Apache 2.0 | Truly open source. |

---

## The 7 Decision Dimensions

After analyzing the AI memory layer landscape, we identified seven dimensions that separate one tool from another. Every meaningful difference maps to one of these:

### 1. Architecture: Binary vs. Runtime Stack

| Question | Why it matters |
|---|---|
| Do I install one binary or a stack of services? | A single binary means `curl \| sh && done`. A runtime stack means Docker, Python, Postgres, Neo4j, and someone has to maintain it. |
| What's my failure surface? | Each dependency is a potential outage. Single binary = one moving part. Microservices = many. |

**Categories:**
- **Single binary**: Install in 10 seconds, zero runtime deps. Works on any OS.
- **Python library**: Needs Python runtime, venv management, package conflicts.
- **Docker stack**: Needs Docker, plus one or more databases (Postgres, Neo4j, Qdrant).
- **Cloud platform**: Needs network, account, API key. Data leaves your machine.

### 2. Data Sovereignty: Where Does Your Data Live?

| Question | Why it matters |
|---|---|
| Does my data ever leave my machine? | Healthcare, finance, legal, and enterprise users care deeply. "Cloud-optional" is not the same as "never leaves." |
| Are embeddings computed locally or sent to an API? | Even if you self-host, some tools send raw text to OpenAI for embedding generation. That's data leaving your machine. |

**Categories:**
- **Fully offline**: Local embeddings (ONNX, etc.), no network calls, no telemetry.
- **Self-hosted but cloud-dependent**: You run it, but it calls external LLM APIs for extraction/embedding.
- **Cloud-native**: Your data lives on someone else's server.

### 3. Search Quality: Keyword vs. Semantic vs. Hybrid

| Question | Why it matters |
|---|---|
| Can I find a memory by meaning, not just exact keywords? | Searching "deployment schedule" should find "we're rolling out Friday at 3pm." That requires semantic search. |
| Does it also support exact keyword matching? | Sometimes you need to find the exact term "Postgres-16.2", not a semantic approximation. |
| Are both combined? | Hybrid search (vector + FTS + fusion) gives the best of both worlds. Pure keyword search misses synonyms. Pure vector search misses exact terms. |

**Categories:**
- **FTS-only**: Keyword matching only. Fast but can't find meaning.
- **Vector-only**: Semantic matching only. Good for meaning, bad for exact terms.
- **Hybrid (Vector + FTS + RRF)**: The gold standard. Finds by meaning AND exact keywords, merged by Reciprocal Rank Fusion.

### 4. Multi-Agent Support: Solo vs. Collaborative

| Question | Why it matters |
|---|---|
| Can multiple agents share a memory space? | In production, you often have multiple agents working on the same project. If they can't share context, they duplicate work and miss each other's findings. |
| Is there author attribution? | When any agent can write to shared memory, you need to know who said what. |
| Can agents recall across each other's memories? | Some tools technically support multiple agents but isolate their namespaces. True multi-agent means cross-agent recall. |

**Categories:**
- **Single-player**: One agent, one namespace. No sharing.
- **Shared API**: Multiple agents access the same backend, but memories aren't attributed or grouped.
- **Rooms**: Shared memory spaces with author attribution, cross-agent recall, and contextual grouping.

### 5. Time-Travel: Can You Query the Past?

| Question | Why it matters |
|---|---|
| Can I recall what the memory store knew at a specific point in time? | "What did we know before the incident?" "When was this decision made?" Without time-travel, you can only see the current state. |
| Is it native or bolted on? | Native time-travel stores full history efficiently. Bolted-on solutions use changelogs or temporal extensions: more overhead, less reliable. |

### 6. Integration: How Does It Connect to Your Agent?

| Question | Why it matters |
|---|---|
| Does it speak MCP (Model Context Protocol)? | MCP is becoming the standard for AI agent tool integrations. Without it, you're writing custom glue code. |
| Is there an HTTP API? | For server-based or remote agents, you need HTTP access. |
| Is there a CLI? | For debugging, scripting, and ad-hoc queries, a CLI is essential. |

### 7. Total Cost of Ownership

| Question | Why it matters |
|---|---|
| What's the setup cost? | "Free" tools that require Docker + Postgres + Neo4j have hidden operational costs. |
| What's the per-query cost? | Cloud LLM-based tools charge per extraction call. At scale, this adds up fast. |
| What's the maintenance cost? | Each dependency needs patching, updating, and monitoring. |

---

## The Capability Matrix

Fill in this matrix for any memory tool you evaluate. Here's how Uteke scores:

| Dimension | Uteke | What to look for in alternatives |
|---|---|---|
| **Architecture** | Single Rust binary (one `curl \| sh`) | Does it need Python? Docker? Postgres? Neo4j? Cloudflare Workers? |
| **Data sovereignty** | Fully offline. Local ONNX embeddings. No telemetry (one optional daily release check). | Does it call external APIs for embeddings/extraction? Does data leave your machine? |
| **Search** | Hybrid (Vector HNSW + FTS5 + RRF fusion) | Is it vector-only? FTS-only? Or hybrid? |
| **Recall speed** | ~45ms warm recall (31ms average at 10K memories) | What's the latency? Is it network round-trip or local? |
| **Multi-agent** | Rooms: shared memory, author attribution, cross-agent recall | Can multiple agents share context? Is there author attribution? |
| **Time-travel** | Native point-in-time queries (`--at 2025-01-15T00:00:00Z`) | Can you query past states? Is it native or bolted on? |
| **Extraction** | Default: offline rule-based (zero API). Optional: LLM extraction. | Does it require an LLM API key to extract facts? Is there an offline option? |
| **Integration** | MCP server (stdio + HTTP), CLI, HTTP API | Does it speak MCP? Is there a CLI for debugging? |
| **Dependencies** | Zero runtime deps. No Docker, no Python, no database server. | Count the dependencies. Each one is operational overhead. |
| **License** | Apache 2.0 | Is it truly open source or source-available? |

---

## Compared With Named Tools

Four open-source memory layers that people most often put next to Uteke, checked against their own repositories on **2026-11-16** (see [Sources checked](#sources-checked) for exactly what was read). Every cell comes from the project's README at that date. If a cell says "not stated", the README did not say, which is not the same as "no". These projects ship weekly, so re-check before you decide.

### Setup and architecture

| | **Uteke** | **[MemPalace](https://github.com/MemPalace/mempalace)** | **[agentmemory](https://github.com/rohitg00/agentmemory)** | **[Mem0](https://github.com/mem0ai/mem0)** | **[Mnemosyne](https://github.com/AxDSan/mnemosyne)** |
|---|---|---|---|---|---|
| **Language / runtime** | Rust, single binary | Python 3.10+ | Node.js 20+ (`npx`) plus a pinned iii-engine runtime | Python (`pip`) and a JS SDK (`npm`) | Python 3.10+ |
| **Install** | `curl \| sh`, `brew`, `cargo install uteke-cli` | `uv tool install mempalace` (also `pipx`, `pip`, Docker) | `npx -y @agentmemory/agentmemory@latest` | `pip install mem0ai` | `pip install mnemosyne-memory` |
| **Default storage** | SQLite + usearch (or vecq) | ChromaDB (other backends opt-in) | File store (Redis optional) | Not stated | SQLite (sqlite-vec + FTS5) |
| **API key for default mode** | None | None for the core path | None (keyless mode) | LLM required, `gpt-5-mini` from OpenAI is the default; default embedder is OpenAI `text-embedding-3-small` | None for storage and recall |
| **Embeddings by default** | Local ONNX (EmbeddingGemma Q4) | Local, downloaded on first run | Off in keyless mode (BM25 only); local MiniLM optional | OpenAI by default | Core profile has none; `[embeddings]` extra adds local `fastembed` |
| **Search** | Vector + FTS5, fused with RRF | Semantic; hybrid mode adds keyword, temporal and preference boosts | BM25 (+ vector and graph when enabled), fused with RRF | Not stated | Vector + FTS5 + importance, weighted 50/30/20 |
| **MCP** | stdio + HTTP | Yes, 47 tools | Yes, 54 tools (7 when the shim cannot reach a server) | Not stated in the README | Yes, built in (stdio, SSE, streamable HTTP) |
| **License** | Apache-2.0 | MIT | Apache-2.0 | Apache-2.0 | MIT |

Where Uteke differs, going by this table: one native binary with local embeddings on by default, hybrid search on by default, and no runtime to install. Where the others are strong: Mem0 has the largest user base and a managed platform, MemPalace and agentmemory publish detailed retrieval numbers, and Mnemosyne is the closest design to Uteke (local SQLite, vector plus FTS5, zero cloud). Not compared because we have not verified them for the other tools: time-travel recall, rooms and multi-agent attribution, document engine.

### Published benchmark figures (cross-source, not Uteke measurements)

These are the vendors' own numbers. They come from different harnesses, splits, metrics and judges, so **do not rank them against each other or against Uteke's figures**. See [Benchmark Positioning](/benchmark-positioning) for the policy.

| Tool | Figure | What it measures | Source | Self-reported |
|---|---|---|---|---|
| MemPalace | 96.6% R@5 | LongMemEval-S, 500 questions, raw semantic search, no LLM | [README](https://github.com/MemPalace/mempalace) | Yes |
| MemPalace | 98.4% R@5 | LongMemEval-S, **held-out 450 questions** (50 used for tuning), hybrid v4, no LLM | same | Yes |
| MemPalace | ≥99% R@5 | LongMemEval-S, 500 questions, **needs an LLM rerank** | same | Yes |
| MemPalace | 85.7% strict recall_all@5 (403/470), 96.6% any@5 (454/470) | LongMemEval-S, 470 answerable questions, raw mode. **Our recount** of the ranking file MemPalace committed, against the official gold labels; read 2026-11-27 | [benchmarks/longmemeval/COMPARISON.md](https://github.com/codecoradev/uteke/blob/develop/benchmarks/longmemeval/COMPARISON.md) | No, recounted by us |
| MemPalace | 90.0% strict recall_all@5 (423/470), 99.4% any@5 (467/470) | Same recount for the hybrid v4 mode with an LLM rerank of the top 20 | same | No, recounted by us |
| agentmemory | 95.2% R@5, 98.6% R@10, MRR 88.2% | LongMemEval-S, 500 questions, BM25 + vector, MiniLM-384. The page labels it a historical any-hit run on an older pipeline and not a strict-recall result | [benchmark/LONGMEMEVAL.md](https://github.com/rohitg00/agentmemory/blob/main/benchmark/LONGMEMEVAL.md) | Yes |
| agentmemory | 96.2% any-hit@5, 84.7% strict recall-all@5 | LongMemEval-S, keyless BM25 component only, 470 answerable questions scored, measured 2026-10-28 | [README](https://github.com/rohitg00/agentmemory) | Yes |
| gbrain | 95.96% strict recall_all@5 (451/470), 470/470 any@5 | LongMemEval-S, 470 answerable questions, `balanced` preset with OpenAI `text-embedding-3-large` and a Voyage reranker (paid APIs), gbrain `109b992`. It returns 5 chunks, not 5 sessions, and its own README says the configuration was chosen on these 470 questions | [gbrain-evals](https://github.com/garrytan/gbrain-evals) (README and `docs/benchmarks/2026-10-04-longmemeval-opaque-followups.md`) | Yes |
| ai-memory | 96.4% R@5 (keyword mode, binary-faithful run, v0.7.0); 97.0% R@5 (README headline, another harness) | LongMemEval-S, 500 questions. The recall variant is not stated and no strict figure is published; its results page says numbers are only comparable within one harness | [ai-memory-mcp](https://github.com/alphaonedev/ai-memory-mcp) (`benchmarks/longmemeval/results.md`) | Yes |
| Mem0 | 94.4 LongMemEval, 92.5 LoCoMo | "New Memory Algorithm (April 2026)". The README says these reflect the managed platform, with proprietary optimizations not in the open-source SDK. Metric and judge are not stated there | [README](https://github.com/mem0ai/mem0) | Yes |
| Mnemosyne | 65.2% BEAM end-to-end QA at 100K | v3.0.0. The README says its judge differs from the other systems listed, so the scores are "not directly comparable". It lists its own LongMemEval run as pending | [README](https://github.com/AxDSan/mnemosyne) | Yes |

On the 470 answerable questions, the basis of the official retrieval scorer, [benchmarks/longmemeval/COMPARISON.md](https://github.com/codecoradev/uteke/blob/develop/benchmarks/longmemeval/COMPARISON.md) puts Uteke, MemPalace and agentmemory side by side on both metrics, and shows the rows that need an LLM or a paid API, which are higher.

Uteke's own figures, from our harness with committed raw outputs, are on the [Benchmarks](/benchmarks) page and in the [README](https://github.com/codecoradev/uteke#benchmarks): 98.2% recall_any@5 and 87.6% strict recall_all@5 on the full 500 questions, with zero LLM calls in retrieval. Strict and any-hit are different metrics, and the denominators above differ, so the strict figures in this section are not a head-to-head.

**How to read the numbers.** Retrieval recall (does the right session land in the top 5?) is judge-free and reproducible. End-to-end QA numbers such as Mem0's and Mnemosyne's depend on the answering model and the judge, and the same answers have been scored tens of points apart under different judges. Treat any vendor number without a harness you can run as a claim.

### Sources checked

Each page was fetched and read on **2026-11-16**. We record the date, not a version, because READMEs change without a release. A figure here is only as current as that date.

| Tool | Page read | Date checked | Version shown on the page |
|---|---|---|---|
| Uteke | This repository at `develop` after the #1789 refresh (README, `docs/benchmarks.md`, `docs/configuration.md`) | 2026-11-26 | v0.24.0 benchmark run (2026-11-17); latency from the v0.17.0 run (2026-09-09) |
| MemPalace | [github.com/MemPalace/mempalace](https://github.com/MemPalace/mempalace) README | 2026-11-16 | 3.12.0 (badge) |
| agentmemory | [github.com/rohitg00/agentmemory](https://github.com/rohitg00/agentmemory) README and [benchmark/LONGMEMEVAL.md](https://github.com/rohitg00/agentmemory/blob/main/benchmark/LONGMEMEVAL.md) | 2026-11-16 | 0.9.33 |
| gbrain | [gbrain-evals](https://github.com/garrytan/gbrain-evals) README, `docs/comparison-systems.md` and the 2026-10-04 report | 2026-11-27 | gbrain `109b992` (v0.60.37.0) for the figure |
| ai-memory | [ai-memory-mcp](https://github.com/alphaonedev/ai-memory-mcp) README, `Cargo.toml`, `benchmarks/longmemeval/results.md` and `methodology.md` | 2026-11-27 | 0.10.0 |
| MemPalace (recount) | The two committed ranking files named in [COMPARISON.md](https://github.com/codecoradev/uteke/blob/develop/benchmarks/longmemeval/COMPARISON.md) and `benchmarks/longmemeval_bench.py` | 2026-11-27 | Files dated 2026-04-14 |
| Mem0 | [github.com/mem0ai/mem0](https://github.com/mem0ai/mem0) README. The docs site could not be reached from our environment, so nothing here comes from `docs.mem0.ai` | 2026-11-16 | Not stated on the page |
| Mnemosyne | [github.com/AxDSan/mnemosyne](https://github.com/AxDSan/mnemosyne) README | 2026-11-16 | Not stated (v3.0.0 appears only as the benchmark version) |

Counts that differ between pages of the same project (for example agentmemory's MCP tool count) use the main README figure. We did not run or reproduce any other tool's benchmark.

**Corrections welcome.** If a cell here is wrong or out of date, open an issue or a pull request with the source link and the date you checked.

---

## The Four Archetypes

Most memory layers fall into one of four archetypes. Understanding which one you need narrows your choice quickly:

### Archetype 1: Cloud-Native Platform

- **Promise:** "We handle everything. Just send us your data."
- **Trade-off:** Data leaves your machine. Per-query costs. Network dependency.
- **Best for:** Teams with cloud-native stacks who don't have data sovereignty concerns.
- **Watch out for:** API costs at scale, vendor lock-in, data residency requirements.

### Archetype 2: Self-Hosted Infrastructure

- **Promise:** "Run it yourself with Docker."
- **Trade-off:** You're now a DevOps engineer. Postgres, Neo4j, Qdrant. Pick your poison.
- **Best for:** Teams with dedicated infra teams who want full control.
- **Watch out for:** Operational overhead. Each service is a maintenance burden and failure point.

### Archetype 3: Python Library

- **Promise:** "`pip install` and you're done."
- **Trade-off:** Python runtime management. Virtual environments. Package conflicts. Not suitable for production agents that need to be always-on.
- **Best for:** Prototyping, Jupyter notebooks, research.
- **Watch out for:** Deploying Python to production. GIL limitations. Dependency hell.

### Archetype 4: Single Binary

- **Promise:** "One binary. Zero deps. Works everywhere."
- **Trade-off:** Less ecosystem breadth. You're trusting one binary to do everything.
- **Best for:** Edge computing, privacy-sensitive domains, developers who value simplicity, agents that run on any machine without setup.
- **Watch out for:** Fewer pre-built integrations compared to ecosystem-heavy tools.

**Uteke is Archetype 4.** We made that choice deliberately. Every trade-off in Uteke's design (local ONNX embeddings, SQLite instead of Postgres, rule-based extraction as default) serves the goal of **zero operational overhead**.

---

## The Extraction Question

One of the most overlooked dimensions is **how memory tools extract facts from raw text**. Two approaches:

### LLM-Based Extraction (Common)

Most tools send your raw text to an LLM API (OpenAI, Anthropic) to extract structured facts. This works well but:

- **Costs money** per call
- **Sends your data** to a third party
- **Requires an API key** before you can start
- **Adds latency** (network round-trip per extraction)

### Rule-Based Extraction (Uteke Default)

Uteke's default extraction mode uses a **zero-dependency offline extractor**: pattern matching, sentence boundary detection, keyword salience scoring, and deduplication. No API calls, no data leaving your machine, no configuration needed.

- **Works out of the box**: zero config, zero API keys
- **Completely private**: your text never leaves the machine
- **Multilingual**: handles Indonesian, emoji, UTF-8 properly
- **Optional LLM upgrade**: if you want richer extraction, switch to `mode = "llm"` and configure an API key

| Aspect | LLM-based (common) | Rule-based (Uteke default) |
|---|---|---|
| **API key required** | ✅ Yes | ❌ No |
| **Data leaves machine** | ✅ Yes (sent to LLM) | ❌ Never |
| **Cost per call** | ✅ Yes | ❌ Free |
| **Setup** | Configure API key, model, endpoint | Works immediately |
| **Quality** | Richer, context-aware | Pattern-based, good for common fact types |
| **Multilingual** | Depends on LLM | ✅ UTF-8 safe (tested with Indonesian, emoji) |
| **Upgrade path** | — | `mode = "llm"` in config to enable LLM extraction |

This means you can start using Uteke immediately. Run `curl | sh`, store a memory, get facts extracted, all without signing up for anything.

---

## The Rooms Advantage

Most memory layers are single-player. They store facts under a flat user ID and every recall is scoped to one agent's view. This works for personal chatbots but breaks down when:

- **Multiple agents work on the same project**: Agent A can't see Agent B's memories
- **A team shares an AI assistant**: everyone's memories are siloed
- **You want knowledge transfer between agents**: there's no shared space

**Uteke Rooms** create a shared memory layer that sits above individual agent namespaces:

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

Every memory in a room carries **author attribution**: you always know who said what. And any agent with access to the room can recall across all entries.

---

## Decision Tree: Which Archetype Do You Need?

```
Do you have data sovereignty requirements?
├── Yes (healthcare, finance, legal, enterprise)
│   └── Can you run Docker + database services?
│       ├── Yes → Self-Hosted Infrastructure (Archetype 2)
│       └── No  → Single Binary (Archetype 4) → Uteke
│
└── No
    └── Are you building for production or prototyping?
        ├── Prototyping → Python Library (Archetype 3)
        └── Production
            └── Do you want to manage infrastructure?
                ├── Yes → Self-Hosted Infrastructure (Archetype 2)
                └── No
                    └── Is network latency acceptable?
                        ├── Yes → Cloud-Native Platform (Archetype 1)
                        └── No  → Single Binary (Archetype 4) → Uteke
```

---

## Benchmark: What "Fast" Means

The ~45ms headline is a conservative upper bound. Measured numbers, from the v0.17.0 release binary on Oracle Cloud ARM (Ampere A1, 4 vCPU), run 2026-09-09 (full table and the older v0.12.0-era run on the [Benchmarks](/benchmarks) page):

| Metric | Result | Context |
|---|---|---|
| **Recall latency (10K memories)** | 31ms average, 38ms P95 | 31ms at 100, 42ms at 1,000. The floor is ONNX embedding inference, not the vector search |
| **Insert throughput** | 5.0–17.5 ops/s | CPU-bound (ONNX embedding inference); slower at 10K |
| **Storage** | 80.7MB for 10,000 memories | SQLite + index, about 8KB per memory |
| **LongMemEval-S recall_any@5** | 98.2% | Full 500 questions, zero LLM calls in retrieval; strict recall_all@5 is 87.6%. Receipts in [`benchmarks/longmemeval/`](https://github.com/codecoradev/uteke/tree/develop/benchmarks/longmemeval) |

These are Uteke's own numbers on one machine. Latency for other tools is not measured here, so this page makes no claim about how they compare. A tool that calls a remote embedding or LLM service adds a network round-trip to each query, but how much depends on the service and your connection.

**Run your own benchmarks:**

```bash
uteke bench --counts 100,1000,10000 --json
```

---

## The Bottom Line

There's no "best" memory layer. There's the **right one for your constraints**. The framework above helps you identify those constraints quickly:

| If your priority is... | Choose archetype... |
|---|---|
| **Simplicity** (install in 10 seconds) | Single Binary |
| **Privacy** (data never leaves machine) | Single Binary |
| **Maximum integrations** | Cloud-Native or Self-Hosted |
| **Team infrastructure control** | Self-Hosted |
| **Rapid prototyping** | Python Library |
| **Multi-agent collaboration** | Anything with Rooms |

**Uteke chose simplicity, privacy, and multi-agent collaboration.** If those are your priorities too, give it a try:

```bash
curl -sSL codecora.dev/uteke/install | sh
uteke onboard
```

No API keys. No Docker. No Python. Just a memory that works.

---

*Uteke is open-source (Apache 2.0), built in Rust, and runs fully offline. [Star us on GitHub](https://github.com/codecoradev/uteke) if this guide was helpful.*

---

## Read More

- 🏠 [Rooms](/rooms) — Multi-agent shared memory
- 🚀 [Quick Start](/getting-started) — Try Uteke in 30 seconds
- 📊 [Benchmarks](/benchmarks) — Detailed performance numbers
