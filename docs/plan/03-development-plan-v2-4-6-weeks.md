# 03 — Development Plan v2 (4-6 Weeks)

**Uteke v2 · Library-First Approach**


---

## Week 1-2: Core Engine

| Day | Task | Deliverable |
|-----|------|-------------|
| 1-2 | Project setup: Cargo workspace, CI skeleton, test framework | Builds on `cargo test` |
| 3-4 | `Memory` struct, SQLite store (CRUD operations) | Can write/read memories |
| 5-6 | HNSW integration: index build, search, persistence | Can search by vector |
| 7-8 | Embedding engine: candle MiniLM-L6-v2, caching | Can embed text locally |
| 9-10 | Wire it all: `remember()` embeds + stores + indexes, `recall()` queries | End-to-end works |
| 11-14 | Unit tests, edge cases, basic bench suite | >80% coverage, perf baselines |

**Exit criteria:** `Uteke::open()` → `remember()` → `recall()` works end-to-end with tests passing.


---

## Week 3-4: CLI + Polish

| Day | Task | Deliverable |
|-----|------|-------------|
| 15-16 | clap CLI: `remember`, `recall`, `search`, `list`, `forget`, `stats` | All commands functional |
| 17-18 | JSON output mode, error formatting, help text | Scriptable output |
| 19-20 | Tags system, config file (`~/.uteke/config.toml`), model download UX | First-run experience |
| 21-22 | Error handling polish, logging, edge cases (empty DB, corrupt index) | Robust CLI  |
| 23-26 | Integration tests, README, man pages, shell completions | Release-ready |

**Exit criteria:** Someone can `cargo install uteke` and use all commands without reading source.


---

## Week 5-6: Open Source Release + Hermes Integration

| Day | Task | Deliverable |
|-----|------|-------------|
| 27-28 | GitHub repo setup: CI/CD (GitHub Actions), issue templates | Green builds |
| 29-30 | Documentation: README with demo, API docs, CONTRIBUTING | Complete docs |
| 31-32 | Hermes integration: Python helper class, test with real agent | Hermes can use Uteke |
| 33-34 | Launch prep: Hacker News post, social content, announcement | Ready to announce |
| 35-36 | Buffer for bugs discovered during integration | Ship with confidence |

**Exit criteria:** Public repo, passing CI, README with GIF demo, Hermes fleet using it.


---

## What's NOT in v2 (Deferred)

| Feature | v1 Plan | v2 Status | Why Deferred |
|---------|---------|-----------|--------------|
| SDK bindings (Python/JS) | v1.0    | ❌         | CLI subprocess is sufficient |
| VS Code extension | v1.1    | ❌         | Premature — no users yet |
| Sync engine | v1.2    | ❌         | Complex, not needed for solo use |
| REST API server | v1.0    | ❌         | Local-only is v2's strength |
| Authentication | v1.1    | ❌         | No network, no auth needed |
| Cloud storage | v1.2    | ❌         | SQLite is the cloud (local) |
| Compression | v1.3    | ❌         | Optimize when size matters |
| Knowledge graph | v2.0 (old) | ❌         | Interesting but not essential |
| Billing / Stripe | v1.1    | ❌         | No revenue in v2 |
| Multi-model embeddings | v1.0    | Partial   | candle only, remote as v2.2 opt-in |
| FFI (C bindings) | —       | ❌         | Python subprocess works fine |


---

## Effort Breakdown

| Phase | Days | Hours (est.) | Key Risk |
|-------|------|--------------|----------|
| Core engine | 14   | \~45h        | candle integration issues |
| CLI + polish | 12   | \~35h        | UX decisions take time |
| OSS release + Hermes | 10   | \~20h        | Launch timing, integration bugs |
| **Total** | **36** | **\~100h**   | \~5h/day for 4 weeks or 3h/day for 6 weeks |

Realistically: **5-6 weeks** at part-time pace (2-3h/day alongside other work).


---

## Risk Mitigation

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| candle crate breaks or lacks MiniLM support | MEDIUM      | HIGH   | Fallback: use ONNX runtime or rust-bert; kill criterion if Week 1 |
| HNSW index too slow for 100k+ memories | LOW         | MEDIUM | hnsw crate is well-tested; benchmark early |
| SQLite concurrent access issues | LOW         | LOW    | Single-process in v2; WAL mode helps |
| CLI UX is confusing | MEDIUM      | MEDIUM | Test with real workflows; iterate before launch |
| Model download is too large for users | LOW         | LOW    | 90MB is acceptable; show progress bar; v2.2 remote opt-in |
| Hermes integration is fragile | MEDIUM      | LOW    | Subprocess pattern is dead simple; retry logic |


---

## Success Checkpoint

After Week 6, answer honestly:


1. **Do I use Uteke daily with Hermes?** → If yes, ship it.
2. **Is retrieval actually useful?** → If no, the embeddings or search need work.
3. **Would I recommend it to a friend?** → If yes, it's ready for OSS.

No external metrics matter for v2.

## CONFIRMED DECISIONS (29 May 2026)

**Decision 1: Rust langsung** (no Python prototype spike)

* Sibung confirmed: Rust is the end goal, no need for Python spike
* Rationale: EmbeddingGemma already works via Python subprocess FFI. No need to validate embedding in Python first.

**Decision 2: EmbeddingGemma ONNX Q4 (768d)**

* Already deployed at /opt/data/models/embeddinggemma/onnx/model_q4.onnx
* Integration: Python subprocess FFI (v2), onnxruntime-rs (v2.1)
* No evaluation needed - model is proven in production

These two decisions eliminate the biggest unknowns from Round 2 review.