# Competitive Analysis

## Competitor Matrix

| Capability | mem0 | Obsidian | Khoj | Zep | Letta | Uteke |
|------------|------|----------|------|-----|-------|-------|
| Semantic Memory | YES  | NO       | YES  | YES | YES   | YES   |
| Structured Memory | PARTIAL | YES      | NO   | YES | YES   | YES   |
| Graph Memory | NO   | YES (manual) | NO   | NO  | NO    | YES   |
| Temporal Memory | NO   | YES (manual) | NO   | YES | YES   | YES   |
| Context Optimization | NO   | NO       | NO   | YES | YES   | YES   |
| Cross-Device Sync | NO   | YES      | YES  | NO  | NO    | YES   |
| Local-First | NO   | YES      | YES  | NO  | NO    | YES   |
| Multi-Framework | NO (Python) | NO       | NO   | NO (Python) | NO (Python) | YES (Rust/Py/Go/TS) |
| Open Source | YES (MIT) | YES      | YES  | NO  | YES (custom) | YES (MIT) |
| Self-Hosted | NO   | N/A      | YES  | NO  | YES   | YES   |
| Binary Size | 200MB+ | 200MB+   | 500MB+ | N/A | N/A   | \~2MB |
| RAM Idle   | 150MB+ | 200MB+   | 200MB+ | N/A | N/A   | \~15MB |

## Detailed Competitor Analysis

### mem0 (30K stars, $10M+ funding)

**Strengths:**

* Most popular AI memory solution
* Good semantic memory with deduplication
* Active development, strong community
* API-first, easy to integrate

**Weaknesses:**

* Python-only (heavy runtime dependency)
* Cloud-first (no local-first)
* No sync mechanism
* 200MB+ with dependencies
* No graph or temporal memory

**Why Uteke wins:** Local-first, Rust (2MB vs 200MB), cross-device sync, multi-framework.

### Obsidian (Revenue: $5M+/yr)

**Strengths:**

* Best local-first knowledge management
* Excellent sync (Obsidian Sync $4/mo)
* Huge plugin ecosystem
* Markdown-native

**Weaknesses:**

* NOT designed for AI agents (manual knowledge only)
* No semantic search or vector embedding
* No AI context management
* No code integration

**Why Uteke wins:** AI-native memory vs manual notes. Complementary, not competitive.

### Khoj (Growing fast)

**Strengths:**

* AI-native personal knowledge assistant
* Self-hosted, local-first
* Semantic search built-in
* Multi-modal (text, images, PDFs)

**Weaknesses:**

* Monolithic (Python, heavy)
* No IDE integration
* No cross-device sync (self-hosted only)
* No team features
* No code/agent-specific memory
* Slow startup, heavy resource usage

**Why Uteke wins:** Lightweight (15MB vs 500MB), IDE-native, sync, team features.

### Zep (Enterprise focus)

**Strengths:**

* Enterprise-grade AI memory
* Long-term memory with summarization
* Session management
* Good for customer support agents

**Weaknesses:**

* NOT open source
* Cloud-only (no local-first)
* Expensive (enterprise pricing)
* No IDE integration
* Python-only

**Why Uteke wins:** Open source, local-first, IDE integration, affordable.

### Letta (Formerly MemGPT)

**Strengths:**

* Advanced memory management (working memory + archival)
* Agent orchestration built-in
* Good research backing

**Weaknesses:**

* Python-only, heavy
* No local-first
* No sync mechanism
* Complex setup
* More of an agent framework than memory tool

**Why Uteke wins:** Focused on memory (not agent framework), lightweight, sync, local-first.

## Positioning Statement

"Uteke is the first AI memory tool that is:


1. **Local-first** (data on your device, not in someone's cloud)
2. **Cross-device** (git sync, zero config)
3. **Lightweight** (2MB binary, 15MB RAM, Rust)
4. **Multi-framework** (works with ANY AI tool, not just one)"

## Uteke vs The Field - Key Differentiators

| Differentiator | Why It Matters |
|----------------|----------------|
| **Rust core**  | 100x smaller, 10x less RAM than Python alternatives |
| **Git sync**   | Every dev already has git. Zero infrastructure. Version-controlled context. |
| **Embedded**   | No server, no Docker, no install. Works in VS Code extension natively. |
| **Memory types** | Only tool with Vector + Structured + Graph + Temporal in one package |
| **Multi-framework** | Not locked to one AI tool. Works everywhere. |