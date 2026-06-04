# Uteke Go-to-Market: README Overhaul + Landing Page Refresh

> **Status:** Phase 1 ✅ Completed · Phase 2 ✅ Completed · Phase 3 🔜 Ready to execute
>
> **Merged:** PR #174 (2026-06-04) — squash merged to `develop`

**Goal:** Transform Uteke from "developer tool" positioning to "must-have AI memory" — starting with README overhaul and landing page refresh to prepare for public launch (Hacker News, Reddit, X/Threads).

**Architecture:** README is the #1 conversion point (GitHub is where devs land). Landing page (uteke.ajianaz.dev) is secondary but must be polished for HN/Reddit shares. Both must convey: offline-first, zero-dep, single binary, 30ms recall, privacy.

**Tech Stack:** Markdown (README), SvelteKit 5 + Tailwind 4 (website, already built)

**Current State (post-PR #174):**
- README: ✅ Overhauled — benefit-first hero, updated comparison (Mem0/Letta/Zep), install.sh quick start, audience fit table, performance highlights
- Website: ✅ Refreshed — version badge v0.0.8, install.sh CTA, social proof section, updated comparison table, OG meta tags
- Install script: sudah ada (`install.sh`)

---

## Phase 1: README Overhaul ✅ COMPLETED

> **Merged in PR #174.** All 6 tasks executed in a single squash commit.

README adalah halaman depan. Developer lihat README dulu sebelum star/download. Harus membuat orang langsung "I need this".

### Task 1.1: Refresh README Hero + Tagline ✅

**Objective:** Hook pembaca dalam 5 detik pertama

**Files:**
- Modify: `/opt/data/repos/uteke/README.md:1-18` (hero section)

**What to change:**

Replace current centered badge section:

```markdown
<p align="center">
  <img src="https://img.shields.io/badge/Uteke-🧠-blue" alt="Uteke" />
</p>

<h1 align="center">Uteke</h1>
<p align="center"><strong>Local-first memory for AI agents — written in Rust</strong></p>
```

With:

```markdown
<h1 align="center">Uteke</h1>
<p align="center"><strong>Give your AI a memory that never leaves your machine.</strong></p>
<p align="center">
  <em>Offline-first semantic memory engine — single binary, zero config, 30ms recall.</em>
</p>
```

**Key changes:**
- Tagline: benefit-first ("Give your AI a memory") bukan descriptive ("memory for AI agents")
- Sub: 3 core selling points compressed — offline, zero config, fast
- Remove emoji badge, keep CI + license + Rust version badges
- Update version badge: `v0.0.8`

**Verify:** Read top 20 lines. If it doesn't make you curious in 5 seconds, iterate.

---

### Task 1.2: Fix Quick Start — Use install.sh ✅

**Objective:** First install experience harus one-liner, bukan `git clone + cargo install`

**Files:**
- Modify: `/opt/data/repos/uteke/README.md:20-44` (Quick Start section)

**What to change:**

```markdown
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
```

**Key changes:**
- Primary CTA: `curl | sh` (one-liner, instant gratification)
- `git clone + cargo install` dipindah ke INSTALL.md sebagai alternative
- Tambah link ke GitHub Releases + Docker (GHCR)
- Contoh `recall` pakai natural question ("when do we deploy?") — bukan keyword

---

### Task 1.3: Update Comparison Table ✅

**Objective:** Comparison table harus akurat vs kompetitor 2026 yang sebenarnya

**Files:**
- Modify: `/opt/data/repos/uteke/README.md:47-62` (comparison table)

**What to change:**

Replace old comparison (MemGPT, ChromaDB, Zep) with current competitors (Mem0, Letta, Zep, Cognee):

```markdown
## Why Uteke?

AI agents forget everything between sessions. Uteke gives them persistent, searchable memory — entirely offline, in one binary.

| | **Uteke** | **Mem0** | **Letta** | **Zep** |
|---|---|---|---|---|
| **Setup** | Single binary | pip + Docker + Qdrant | pip + Docker + Postgres | pip + Docker + Neo4j |
| **API keys needed** | ❌ None | ✅ OpenAI/LLM key | ✅ LLM key | ✅ LLM key |
| **Offline** | ✅ Fully | ❌ Cloud embedding | ❌ Needs LLM server | ❌ Needs LLM + vector DB |
| **Semantic search** | ✅ Local ONNX | ✅ Cloud embedding | ⚠️ Keyword + archival | ✅ GraphRAG |
| **Zero config** | ✅ Works instantly | ❌ Docker + env vars | ❌ Docker + env vars | ❌ Docker + env vars |
| **Embedding model** | Built-in (ONNX) | External (cloud) | External | External |
| **Recall speed** | ~30ms (library) | Network round-trip | Network round-trip | Network round-trip |
| **Privacy** | ✅ Data never leaves machine | ⚠️ Data sent to LLM | ⚠️ Data sent to LLM | ⚠️ Data sent to LLM |
| **Language** | Rust | Python | Python | Go + Python |
| **License** | Apache 2.0 | Apache 2.0 | Apache 2.0 | Apache 2.0 |
```

**Key changes:**
- Kompetitor yang relevan 2026: Mem0 (57K ⭐), Letta, Zep
- Tambah row: "API keys needed" dan "Privacy" — ini Uteke's killer differentiator
- Tambah row: "Recall speed" — angka konkret (30ms vs "network")
- Remove ChromaDB (bukan memory tool, vector DB)

---

### Task 1.4: Add "Who is this for" Section ✅

**Objective:** Orang harus langsung tahu apakah Uteke buat mereka

**Files:**
- Modify: `/opt/data/repos/uteke/README.md` (add after "Use Cases" section, ~line 87)

**What to add (after Use Cases):**

```markdown
## Who is Uteke for?

| You are | You want | Uteke? |
|---------|----------|--------|
| AI agent builder | Persistent memory, no infra | ✅ Perfect fit |
| CLI power user | Searchable personal knowledge base | ✅ Perfect fit |
| Privacy-conscious dev | Memory tool that works offline | ✅ Perfect fit |
| Team needing shared memory | Multi-user sync + collaboration | ❌ Not yet (Phase B) |
| Enterprise needing graph RAG | Entity relationships, cross-agent knowledge | ❌ Use Mem0/Zep instead |

> **Not sure?** Try it — `curl -sSL https://raw.githubusercontent.com/ajianaz/uteke/main/install.sh | sh` — uninstall is just `rm -rf ~/.uteke`.
```

---

### Task 1.5: Refresh Performance Section ✅

**Objective:** Performance section harus punya "wow" factor dan konteks

**Files:**
- Modify: `/opt/data/repos/uteke/README.md:230-276` (Performance section)

**What to change:**

Add a highlight box BEFORE the detailed table:

```markdown
## Performance

> **TL;DR:** Library recall in ~30ms. Server recall in ~42ms. CLI in ~3s (cold start). Zero external dependencies. All on CPU.

### The One Number That Matters

| Mode | Recall | Setup |
|------|--------|-------|
| **Library (Rust)** | **~30ms** | In-process, no startup |
| **Server (HTTP)** | **~42ms** | One-time ~2s init |
| **CLI (binary)** | **~3s** | Per-invocation (model load) |

For real-time agent use, run `uteke-serve` — model stays in memory, 75x faster than CLI.

[... rest of existing performance section stays ...]
```

---

### Task 1.6: Refresh Roadmap + Footer ✅

**Objective:** Roadmap harus menunjukkan momentum dan credibility

**Files:**
- Modify: `/opt/data/repos/uteke/README.md:370-392` (Roadmap + Footer)

**What to change:**

```markdown
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
```

---

## Phase 2: Landing Page Refresh ✅ COMPLETED

> **Merged in PR #174.** All 5 tasks executed alongside Phase 1 in the same commit.

Website sudah ada dan bagus, tapi ada beberapa hal yang harus di-fix sebelum go-public.

### Task 2.1: Fix Version Badge + Install CTA ✅

**Objective:** Version badge dan install command harus current

**Files:**
- Modify: `/opt/data/repos/uteke/website/src/routes/+page.svelte` (hero section)

**What to change:**

1. **Version badge** (line ~108): Change `v0.0.3 released` → `v0.0.8 released` and update release link
2. **Install command** (line ~122-130): Change `cargo install --git https://github.com/ajianaz/uteke` → `curl -sSL https://raw.githubusercontent.com/ajianaz/uteke/main/install.sh | sh`
3. **Copy button** onclick should copy the new install command
4. **Get Started section** (step 1): Also change to `curl | sh`

---

### Task 2.2: Add "Trusted By" / Social Proof Section ✅

**Objective:** Tambah credibility signal untuk visitor baru

**Files:**
- Modify: `/opt/data/repos/uteke/website/src/routes/+page.svelte`

**What to add** (after terminal demo, before "Problem → Solution"):

```svelte
<!-- Social Proof -->
<section class="max-w-6xl mx-auto px-6 py-10 text-center">
    <div class="flex flex-wrap items-center justify-center gap-8 text-[var(--color-text-dim)]">
        <div class="flex items-center gap-2">
            <span class="text-xl">🦀</span>
            <span class="text-sm">Built with Rust</span>
        </div>
        <div class="flex items-center gap-2">
            <span class="text-xl">🔒</span>
            <span class="text-sm">100% Offline</span>
        </div>
        <div class="flex items-center gap-2">
            <span class="text-xl">⚡</span>
            <span class="text-sm">30ms Recall</span>
        </div>
        <div class="flex items-center gap-2">
            <span class="text-xl">📦</span>
            <span class="text-sm">Single Binary</span>
        </div>
        <div class="flex items-center gap-2">
            <span class="text-xl">🌍</span>
            <span class="text-sm">Linux · macOS · Windows</span>
        </div>
        <div class="flex items-center gap-2">
            <span class="text-xl">🐳</span>
            <span class="text-sm">Docker Ready</span>
        </div>
    </div>
</section>
```

---

### Task 2.3: Update Comparison Table Data ✅

**Objective:** Sync landing page comparison with README comparison (Task 1.3)

**Files:**
- Modify: `/opt/data/repos/uteke/website/src/routes/+page.svelte` (comparisons array)

**What to change:**

Update the `comparisons` array to match README:

```typescript
const comparisons: Comparison[] = [
    { feature: 'Install', uteke: '1 binary', mem0: 'pip + Docker + Qdrant', letta: 'pip + Docker + Postgres', cognee: 'pip + Docker + Neo4j' },
    { feature: 'API Keys', uteke: '✅ None needed', mem0: '❌ OpenAI/LLM key', letta: '❌ LLM key', cognee: '❌ LLM + vector DB' },
    { feature: 'Offline', uteke: '✅ Fully', mem0: '❌ Cloud embedding', letta: '❌ Needs server', cognee: '❌ Needs LLM + DB' },
    { feature: 'Semantic Search', uteke: '✅ Local ONNX', mem0: '✅ Cloud embedding', letta: '⚠️ Keyword + archival', cognee: '✅ GraphRAG' },
    { feature: 'Privacy', uteke: '✅ Data stays local', mem0: '⚠️ Sent to LLM', letta: '⚠️ Sent to LLM', cognee: '⚠️ Sent to LLM' },
    { feature: 'Recall Speed', uteke: '~30ms', mem0: 'Network RTT', letta: 'Network RTT', cognee: 'Network RTT' },
    { feature: 'Tag Management', uteke: '✅ list/rename/delete', mem0: '⚠️ Basic', letta: '❌', cognee: '⚠️ Basic' },
    { feature: 'Memory Aging', uteke: '✅ Auto-cleanup', mem0: '✅', letta: '✅ Core memory', cognee: '✅ TTL-based' },
    { feature: 'Shell Hooks', uteke: '✅ bash/zsh/fish', mem0: '❌', letta: '❌', cognee: '❌' },
    { feature: 'License', uteke: 'Apache-2.0', mem0: 'Apache-2.0', letta: 'Apache-2.0', cognee: 'Apache-2.0' }
];
```

Added rows: "API Keys", "Privacy", "Recall Speed" — these are Uteke's strongest differentiators.

---

### Task 2.4: Add OG Image + Meta Tags ✅

**Objective:** Saat di-share ke Twitter/LinkedIn/Discord, preview card harus menarik

**Files:**
- Modify: `/opt/data/repos/uteke/website/src/routes/+page.svelte` (svelte:head)
- Create: `/opt/data/repos/uteke/website/static/og-image.png` (1200x630)

**What to change in svelte:head:**

```svelte
<svelte:head>
    <title>uteke — Give Your AI a Memory (Offline)</title>
    <meta name="description" content="Local-first semantic memory engine. Single Rust binary, zero infrastructure, 30ms recall, fully offline. No API keys needed." />
    <meta property="og:title" content="uteke — Give Your AI a Memory" />
    <meta property="og:description" content="Offline-first semantic memory. Single binary. Zero config. 30ms recall." />
    <meta property="og:type" content="website" />
    <meta property="og:image" content="/og-image.png" />
    <meta name="twitter:card" content="summary_large_image" />
    <meta name="twitter:title" content="uteke — Give Your AI a Memory" />
    <meta name="twitter:description" content="Offline-first semantic memory. Single binary. Zero config. 30ms recall." />
    <meta name="twitter:image" content="/og-image.png" />
</svelte:head>
```

**OG Image:** Generate dengan `image_generate` tool — dark theme, text: "uteke" + tagline + "Offline · Zero Config · 30ms Recall". Size 1200x630.

---

### Task 2.5: Fix Hero Headline for Clarity ✅

**Objective:** Current headline "Your AI forgets everything. Fix that." bagus tapi kurang jelas WHAT Uteke is

**Files:**
- Modify: `/opt/data/repos/uteke/website/src/routes/+page.svelte` (hero h1 + subheadline)

**What to change:**

```svelte
<!-- Headline -->
<h1 class="animate-fade-in-delay-1 text-4xl sm:text-5xl md:text-7xl font-bold tracking-tight mb-6 leading-[1.1]">
    Your AI forgets<br />
    everything<span class="text-[var(--color-text-dim)]">.</span>
    <br />
    <span class="hero-gradient glow-text">Give it a memory.</span>
</h1>

<!-- Subheadline -->
<p class="animate-fade-in-delay-2 text-base md:text-xl text-[var(--color-text-muted)] max-w-2xl mx-auto mb-10 leading-relaxed">
    uteke gives AI agents persistent, searchable memory —<br class="hidden md:block" />
    <strong class="text-[var(--color-text)]">fully offline, single binary, 30ms recall.</strong>
</p>
```

**Key change:** "Fix that" → "Give it a memory" (more descriptive). Sub bold-kan angka-angka key.

---

## Phase 3: Distribution Prep 🔜 READY

> **Prerequisites met.** Phase 1 + 2 merged. Launch docs ready for review/update.

### Task 3.1: Prepare Hacker News "Show HN" Post 🔜

**File to create:** `/opt/data/repos/uteke/docs/plans/show-hn-draft.md`

**Draft:**

```
Show HN: Uteke – Offline-first semantic memory for AI agents, single Rust binary

I built Uteke because every AI agent I run forgets everything between sessions. Existing solutions (Mem0, Letta, Zep) all require cloud APIs, Docker, or LLM keys.

Uteke is different:
- Single binary, no dependencies (curl | sh to install)
- Built-in ONNX embedding model (EmbeddingGemma) — works fully offline
- ~30ms recall via library, ~42ms via HTTP server
- SQLite + usearch HNSW for storage + vector search
- Multi-agent namespaces, memory aging, shell hooks

Tech: Rust, ONNX Runtime, SQLite (rusqlite), usearch HNSW, EmbeddingGemma Q4

https://github.com/ajianaz/uteke
```

### Task 3.2: Prepare Reddit Posts 🔜

**Subreddits:** r/rust, r/CLI, r/selfhosted, r/LocalLLaMA, r/MachineLearning

**Angle per subreddit:**
- r/rust: "Built a local semantic memory engine in Rust — ONNX + SQLite + HNSW, single binary"
- r/CLI: "Made a CLI tool that gives your terminal persistent, searchable memory"
- r/selfhosted: "Self-hosted AI memory that works 100% offline — no Docker needed"
- r/LocalLLaMA: "Local-first semantic memory for AI agents — no cloud, built-in embeddings"

### Task 3.3: Record Terminal Demo (asciinema/vhs) 🔜

**Objective:** Record demo yang bisa di-embed di README dan landing page

**Steps:**
1. Install: `curl -sSL ... | sh`
2. `uteke remember "I prefer dark mode" --tags pref`
3. `uteke remember "Deploy v2 to staging Friday" --tags deploy`
4. `uteke recall "what are my preferences?"` → shows dark mode result
5. `uteke stats`
6. `uteke aging status`

**Format:** asciinema cast file atau vhs tape → GIF untuk README

---

## Summary

| Phase | Tasks | Status | Result |
|-------|-------|--------|--------|
| **Phase 1: README** | 1.1–1.6 | ✅ Completed | PR #174 merged |
| **Phase 2: Landing Page** | 2.1–2.5 | ✅ Completed | PR #174 merged |
| **Phase 3: Distribution** | 3.1–3.3 | 🔜 Ready | Awaits execution |

**Next steps:**
1. Review & update `docs/launch/hn-post.md` and `docs/launch/twitter-thread.md` to match current README/landing page content
2. Execute distribution when ready
