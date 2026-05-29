# 06 — Risk Assessment v2 (Reduced Scope)

**Uteke v2 · Library-First Approach**


---

## Risk Matrix: v1 vs v2

| Risk | v1 Severity | v2 Severity | Change | Why |
|------|-------------|-------------|--------|-----|
| **Scope creep** | 🔴 CRITICAL | 🟡 MEDIUM   | ↓↓↓    | v2 has hard scope boundaries; no product features |
| **Low adoption** | 🟠 HIGH     | 🟢 LOW      | ↓↓     | Personal use validates; OSS is free marketing |
| **Incumbent competition** | 🟠 HIGH     | 🟢 LOW      | ↓↓     | No direct competitor does "local Rust library" |
| **Solo burnout** | 🟠 HIGH     | 🟢 LOW      | ↓↓     | 100 hours total, not 1000+ |
| **Monetization failure** | 🟠 HIGH     | ⚪ NONE      | ↓↓↓    | No revenue in v2; not a product |
| **Technical complexity** | 🟡 MEDIUM   | 🟡 MEDIUM   | →      | Embeddings + HNSW still non-trivial |
| **Model quality** | 🟡 MEDIUM   | 🟡 MEDIUM   | →      | MiniLM-L6 may not be good enough |
| **Maintenance burden** | 🟠 HIGH     | 🟢 LOW      | ↓↓     | Library has minimal surface area |
| **Funding dependency** | 🔴 CRITICAL | ⚪ NONE      | ↓↓↓    | Self-funded, no runway pressure |

**Average v1 risk score: \~7/10 → v2 risk score: \~3/10**


---

## Detailed Risk Analysis

### 1. Scope Creep (CRITICAL → MEDIUM)

**v1:** Building a full product means every feature feels necessary. 9 subsystems, each with scope expansion potential. Classic "second system effect."

**v2:** The library does exactly 4 things: remember, recall, search, list. No API server, no billing, no auth. If someone requests a feature, the answer is: "Great, noted for v3. v2 is done."

**Mitigation:** Maintain a "Not in v2" list (see [03 - Development Plan](./uteke_v2_03)). If it's not on the Week 1-6 plan, it doesn't ship.

### 2. Low Adoption (HIGH → LOW)

**v1:** A full product with zero users is a failed business. Revenue projections don't materialize. Demoralizing.

**v2:** A library with 10 users is a success story. 50 stars means 50 people found it interesting. 0 stars means... I still use it daily. The personal utility floor means v2 cannot "fail."

**Mitigation:** None needed. The worst case is "only I use it," which was always the primary goal.

### 3. Incumbent Competition (HIGH → LOW)

**v1:** Competing with MemGPT, LangChain memory, Zep, etc. on their turf (cloud APIs, managed services). They have teams and funding.

**v2:** No incumbent does "local-first Rust library with embedded HNSW and offline embeddings." The competitors are:

* **MemGPT:** Cloud-based, Python, complex setup. We're local, Rust, CLI.
* **ChromaDB:** Python-first, needs a server. We're zero-dep single binary.
* **Zep:** Cloud service with pricing. We're free and offline.
* **Obsidian:** Note-taking, not semantic search for agents.

**Mitigation:** Clear differentiation in README. We're not competing; we're solving a different problem.

### 4. Solo Burnout (HIGH → LOW)

**v1:** 300+ hours across 6+ months. Maintaining API servers, handling user support, fixing production bugs. Classic indie hacker burnout pattern.

**v2:** \~100 hours over 4-6 weeks. After shipping, maintenance is minimal: a few GitHub issues per month. No server to keep running. No customers to support. If I stop working on it, it still works forever.

**Mitigation:** Ship and step back. OSS community can handle issues. Or they can't, and it's fine.

### 5. Monetization (HIGH → NONE)

**v1:** If no one pays, the business model collapses. SaaS requires critical mass.

**v2:** There is no monetization. This is not a product. It's a tool I built for myself. Money may come in v4+ or never.

**Mitigation:** Not applicable. No revenue goal = no revenue risk.


---

## Kill Criteria v2

Simple. Three gates. If any fails, stop and reassess.

### Kill Gate 1: Week 1 — Embedding Feasibility

**Question:** Can candle-transformers run MiniLM-L6-v2 on my machine with acceptable speed (<10ms per embed)?

**If NO:** Switch to ONNX runtime. If that also fails, evaluate whether local embedding is viable at all. If fundamentally blocked, the project pivots or stops.

### Kill Gate 2: Week 2 — Core Engine Works

**Question:** Can I `remember()` something and `recall()` it with relevant results?

**If NO:** The core value proposition fails. Debug for 3 days max. If still broken, pause and reassess the embedding model or search algorithm.

### Kill Gate 3: Week 6 — Personal Utility

**Question:** Do I genuinely use Uteke daily with Hermes after 2 weeks of real usage?

**If NO:** Ship it anyway (it's open source), but don't invest in v3+. The tool didn't solve the problem as expected. Learn from it.


---

## What We Gave Up (from v1)

| Component | Estimated Hours Saved | Trade-off |
|-----------|-----------------------|-----------|
| REST API server | \~60h                 | Users must use CLI or SDK instead |
| SDK bindings (Python + JS) | \~40h                 | subprocess pattern is simpler |
| VS Code extension | \~50h                 | Terminal-first is fine for power users |
| Sync engine + CRDTs | \~40h                 | Single-machine only |
| Authentication system | \~20h                 | No multi-user, no cloud |
| Billing (Stripe integration) | \~15h                 | No revenue model |
| Cloud storage infra | \~25h                 | SQLite is the database |
| Knowledge graph | \~30h                 | HNSW vectors are sufficient |
| Compression (entropy, delta) | \~15h                 | Storage is cheap locally |
| Admin dashboard | \~20h                 | CLI is the dashboard |
| Onboarding flow / UX | \~20h                 | README + CLI help is enough |
| **Total saved** | **\~335h**            | **Reduced from \~435h to \~100h** |

That's **8 months** of work avoided. For a solo developer with a day job, this is the difference between "someday maybe" and "shipping next month."


---

## Remaining Risks (v2-Specific)

| Risk | Mitigation |
|------|------------|
| candle model download fails on some systems | Graceful fallback message; document requirements |
| HNSW index corrupted by crash | WAL mode +定期 backups; rebuild index from DB |
| MiniLM quality insufficient for domain (Indonesian text, code) | Test early; switch model in v2.1 if needed |
| CLI UX is confusing for non-technical users | Not the target audience for v2; focus on developers |
| GitHub repo gets no traction | Fine; personal utility is the success metric |


---

## Risk Summary

**v2 is low-risk by design.** We reduced scope to the point where:

* The worst outcome is "I built a useful tool for myself"
* The best outcome is "a community forms around it and we grow organically"
* The cost of failure is \~100 hours of learning
* The cost of success is... shipping a useful tool

This is how software should be built.

## DECISION UPDATES (29 May 2026)

### Risk R2 REMOVED: candle-transformers (was MEDIUM)

**Status: RESOLVED** - Using EmbeddingGemma ONNX instead of candle-transformers.

EmbeddingGemma is already proven in production. No integration risk. The candle-transformers concern from Round 2 is no longer applicable.

### Updated Risk Matrix (post-decision)

| Risk | Probability | Impact | Severity |
|------|-------------|--------|----------|
| Scope creep | Medium      | Medium | MEDIUM   |
| onnxruntime FFI issues | Low         | Low    | LOW      |
| Low adoption | Irrelevant  | Low    | LOW      |
| Incumbent threat | Low         | Low    | LOW      |
| HNSW memory too high | Low         | Low    | LOW      |
| Burnout | Low         | Medium | LOW      |

**Average severity: LOW** (was MEDIUM before embedding decision). All major unknowns resolved.