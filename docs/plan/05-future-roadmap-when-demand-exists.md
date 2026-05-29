# 05 — Future Roadmap (When Demand Exists)

**Uteke v2 · Library-First Approach**


---

## Philosophy

> **"Don't build it until someone asks for it twice."**

Every feature beyond v2 must pass a demand test:


1. **Someone requests it** — GitHub issue, discussion, or real conversation
2. **Someone else independently requests it** — confirms it's not a niche edge case
3. **Building it doesn't compromise the library's simplicity** — no scope creep

If the signal doesn't exist, we don't build it. Full stop.


---

## Phase A: Library Maturity

**Trigger: 100+ GitHub stars OR 3 months of personal daily use**

### v2.1 — Quality & Embeddings

* Replace candle MiniLM-L6 with **MiniLM-L12-v2** for better quality (still local, \~120MB)
* Streaming embeddings for large documents (chunk + embed + store pipeline)
* Memory deduplication: detect near-duplicates and merge
* Import/export: `uteke export --format json` and `uteke import`
* Shell completions for bash, zsh, fish

**Effort:** \~2 weeks

### v2.2 — Remote Embedding Integration

* Opt-in remote embedding via OpenAI / Voyage / Jina API
* Config: `embedding.provider = "openai"` + API key
* Fall back to local if API unreachable
* Model comparison benchmarks

**Effort:** \~1 week

### v2.3 — Context Builder

* `uteke context --query "..." --max-tokens 4096` — assemble optimized context for LLM prompts
* Automatic relevance scoring + token budgeting
* Output: formatted prompt-ready text

**Effort:** \~1 week


---

## Phase B: Ecosystem Expansion

**Trigger: 500+ GitHub stars OR clear community demand**

### v3.0 — SDK + Extensions

* **Python SDK** via PyO3 bindings: `pip install uteke`
* **Node.js SDK** via napi-rs: `npm install uteke`
* Extension trait system: users can build custom memory processors
* Plugin manifest format

**Effort:** \~4-6 weeks

### v3.1 — Sync Protocol

* File-based sync: Dropbox/iCloud/Google Drive folder sync
* CRDT-based conflict resolution (no server needed)
* Multi-device support: work laptop + home desktop share memory

**Effort:** \~4-6 weeks

### v3.2 — Editor Integration

* LSP (Language Server Protocol) for code-aware memories
* VS Code extension (community-built or official)
* Neovim plugin via Lua FFI

**Effort:** \~2-4 weeks (VS Code); community for others


---

## Phase C: Product

**Trigger: 1000+ GitHub stars OR external funding**

### v4.0 — Team & Infrastructure

* Form a small team (2-3 people)
* Cloud sync server (optional, opt-in)
* Team workspaces: shared memory stores
* Web dashboard for memory visualization

**Effort:** \~3-6 months

### v4.1 — Monetization

* **Freemium model:**
  * Free: Local-only, unlimited memories, full features
  * Pro ($5/mo): Cloud sync, remote embeddings, priority support
  * Team ($15/user/mo): Shared workspaces, admin, audit log
* No paywalling core library features — ever

### v4.2 — Intelligence

* Automatic summarization of stored memories
* Knowledge graph: extract entities and relationships
* Time-based decay: older memories fade unless reinforced
* "Uteke, what should I remember from today?" — proactive memory extraction

**Effort:** \~2-3 months


---

## Phase D: Platform (Optional / Speculative)

**Trigger: Massive organic growth (10k+ stars) OR strategic partner interest**

* Uteke as infrastructure: other tools build on top of it
* Marketplace for community memory processors
* Enterprise features: SSO, compliance, on-prem deployment
* Vertical solutions: legal, medical, academic memory tools

**Status:** Pure speculation. Not planning for this.


---

## Decision Gates

| Gate | Metric | Decision | Go/No-Go |
|------|--------|----------|----------|
| G1: Ship v2 | Personal daily use | Release on GitHub | ✅ I use it |
| G2: Quality pass | 100+ stars or 3 months | Build v2.1-v2.3 | ⏸️ Assess |
| G3: Ecosystem | 500+ stars | Build v3.0 SDK | ⏸️ Assess |
| G4: Product | 1000+ stars or funding | Build v4.0 | ⏸️ Assess |
| G5: Platform | 10k+ stars | Consider Phase D | ⏸️ Unlikely |

At each gate: **pause, assess honestly, and decide.** No auto-progression.


---

## What Gets Cut If No Signal

| Feature | Cut If... | Alternative |
|---------|-----------|-------------|
| Remote embeddings | No one asks by Month 6 | Keep local-only, it works fine |
| SDK bindings | No PRs or requests by Month 6 | CLI subprocess is always sufficient |
| Sync    | No multi-device requests | Single-machine is v2's design intent |
| VS Code | No editor requests | Terminal is power user territory anyway |
| Knowledge graph | No one asks (ever) | HNSW semantic search is enough |
| Team features | No funding | Stay solo, stay simple |
| Monetization | No product demand | MIT forever, no revenue needed |

**v2 is the final version if no one cares.** And that's fine. It solves my problem either way.