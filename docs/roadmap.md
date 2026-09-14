---
title: Roadmap
---

# Roadmap

Demand-gated — we build what people actually use. Track progress on [GitHub Issues](https://github.com/codecoradev/uteke/issues).

## v0.18.0 - Agent-Operable Memory Plumbing `Released 2026-09-13`

- **Ingest date anchors (#1232/#1238)** - `remember --timestamp` + import timestamps; time-travel and audit no longer depend on ingest order
- **Room lifecycle management (#1202/#1203)** - rename/update/move-memory across HTTP, CLI, and MCP (schema v19)
- **Recall payload conformance tests (#1233/#1239)** - full-payload contract pinned across all surfaces
- Fixes: dedup live-check (#1211), default min_score 0.0 (#1228), install.sh ORT libs (#1221), LoCoMo label mapping (#1222)

## v0.17.0 — Inspectable, Trustworthy Memory `✓ Released 2026-09-06`

- **Explain recall (#1160)** — see why each memory ranked where it did, on every recall surface (CLI `--explain`, HTTP `explain`, MCP flag)
- **Contradiction resolution as an auditable ledger (#1172)** — supersede/list/undo with provenance; contradiction benchmark segment: fusion winner@1 0.850 → 1.000 after supersede, stale@5 1.000 → 0.000
- **Provenance data model (#1172, schema v18)** — `source_hash` tamper evidence, actor/evidence timeline events, `provenance(id)` report
- **Dual-engine vector layer** — runtime-switchable vector engine (#1169/#1171)
- **Namespace management API (#1181)** — rename/merge/delete with explicit memory strategy; `/list` pagination metadata (#1188)

## v0.16.0 — Fusion Default `✓ Released 2026-08-29`

- **`fusion` is now the default recall strategy (#1123)** — weighted RRF of the vector and hybrid rankings; LongMemEval 500Q R@5 0.946 vs 0.854 hybrid (+9.2 pts), zero config needed
- Public benchmark page: dual-metric view (recall_any + strict recall_all) with comparison chart (#1141)
- Full-release validation: 500Q pure-default run, binary built from the exact release SHA
- Post-release fixes: `repair --reembed` handles NULL-embedding rows (#1146), CLA check bot allowlist (#1132), repair report reflects final index state (#1149)

## v0.15.0 — Trust Across Surfaces, Portable Store `✓ Released 2026-08-19`

- **Supersession workflow (#1069)** — mark a stale decision as superseded by a newer one; recall flags superseded entries
- **Structural export/import (#1068)** — full-store round-trip (rooms, graph, edges, documents, timeline) without loss
- UUIDv7 IDs for new memories — time-ordered at the front
- MCP `uteke_get` + `uteke_update` (#1067), richer MCP outputs (#1066)
- `uteke_dream` runs dry-run first — destructive passes need explicit confirmation (#1065)
- `/export` keeps namespace attribution (#1064); recall cache score parity (#1063)

## v0.14.0–v0.14.3 — Hybrid Default, Scene Extraction, Surface Parity `✓ Released 2026-08-14–15`

- **Hybrid (RRF) is now the default recall strategy (#1005)** — recall@5 85.4% → 98.0%; revert via `strategy = "vector"` in `[recall]`
- Scene-segmented LLM extraction with priority scoring (#1009) — `scene:<topic>` tags, semantic types, importance values
- `GET /lifecycle/deprecated` + `GET /guide` endpoints (#1007, #1010)
- Source provenance for extracted memories (#1012, #1013)
- Chunker fixes via mutation testing: heading duplication + multibyte infinite loop (#1024) — mutation score 50% → 97%
- `uteke-cli` crates.io publish restored + release verify step (#1021, #1030)
- HTTP & MCP recall `strategy` parity — default hybrid, loud 400/JSON-RPC errors on invalid values (#1034, #1035, #1038)
- Cross-compilation for Android/iOS targets (#1014), batch-mode update check skip (#1006), embedding pre-truncation (#1002)
- Release notes heredoc hardening — template file, no shell expansion (#1041)

## v0.13.0–v0.13.2 — Safe Lifecycle, Offline Extraction, Security Hardening `✓ Released 2026-08-08–11`

- **Safe memory lifecycle (#928–#937)** — `ACTIVE → DEPRECATED (30-day TTL, restorable) → PRUNED`; soft-delete default, `uteke lifecycle` commands + HTTP endpoints
- Offline rule-based extractor as default (#890) — no LLM/API key needed for basic extraction
- Persistent embedding cache + lazy ONNX load (#896), `repair --reembed/--rebuild` (#919, #901)
- Update check on all surfaces (#990), NULL-embedding startup crash fix (#992, #993)
- Security batch (#966–#987): command injection fix, checksum abort, pinned-memory data-loss guard, UTF-8 panics, zero unsafe blocks, N+1 elimination, case-sensitive routes
- Branding refresh (#991) — "One memory. Every agent. Zero cloud."

## v0.12.0 — Full-text Doc Search, Junction Tools, Windows Installer `✓ Released 2026-07-22`

- FTS5 indexes full document content body — every word searchable, not just titles (#783)
- Room↔Document junction in CLI & MCP — `uteke room doc add/remove/list`, 4 new MCP tools (#789)
- Renamed MCP tool: `uteke_room_document` → `uteke_room_summary_document` (#735)
- Windows PowerShell installer (`install.ps1`) + `.zip` binary release (#781)
- `POST /memory/feedback` trust scoring endpoint (#718)
- Bug fixes: deprecated memory leak in vector search, doc search scope

## v0.11.0 — Hardening & Governance `✓ Released 2026-07-10`

- `POST /doc/move` `#[serde(deny_unknown_fields)]` — silent data loss fix (#833)
- Recall score reports cosine similarity instead of RRF rank (#831)
- MCP/CLI store mismatch — hardcoded `~/.uteke` paths fixed (#830)
- `POST /consolidate` accepts string threshold via `flex_f32` (#826)
- API route drift guard test — 46 handler paths verified (#829)
- Open-source governance docs — Code of Conduct, PR template, YAML issue forms (#834)

## v0.10.0 — Rust Edition 2024 `✓ Released 2026-07-05`

- Rust edition 2021 → 2024 — minimum Rust 1.85 (#755)
- `POST /room/remember` endpoint — store + link to room in one call (#762)
- `DELETE /forget` returns 404 for non-existent IDs (#762)
- Clippy lints after edition upgrade

## v0.9.0 — Onboarding Wizard & API Versioning `✓ Released 2026-06-30`

- `uteke onboard` — interactive onboarding wizard (#743)
- API URL versioning `/api/v1` and `/api/v2` (#741)
- Configurable dream pipeline thresholds (#742)
- `SECURITY.md` and PR template (#746)
- Deprecated memory leak in vector search fix (#748)
- Windows ERROR_LOCK_VIOLATION fix (#747)
- Embedding model streaming download (#740)
- Contributors: @webhop123, @gnoviawan

## v0.8.0 — Trust Scoring & Cross-Entity Linking `✓ Released 2026-06-29`

- `PUT /memory` — partial memory updates with embedding regen (#676)
- Room↔Document junction table — schema v15 (#689)
- Memory↔Document linking via `[[doc-slug]]` wikilinks (#691)
- Trust scoring — `uteke feedback helpful/unhelpful` (#718)
- Jaccard token similarity reranking (#719)
- Auto-contradiction scan as Dream pipeline Phase 4 (#720)
- Cross-entity enrichment in recall (`--enrich`) (#703)
- FTS5 indexes `memory_type` — schema v14 (#662)

## v0.7.0 — HTTP API Expansion & Global Documents `✓ Released 2026-06-28`

- Project-aware memory tagging (#616)
- OpenCode init support (#612)
- Maintenance HTTP endpoints — `/prune`, `/consolidate`, `/aging` (#607)
- Monitoring HTTP endpoints — `/importance`, `/orphans`, `/rebuild-backlinks` (#608)
- Extract/Import/Export HTTP endpoints (#604–#606)
- Document partial update CLI + HTTP (#589)
- MCP: pin/unpin, 6 room tools, tag management, doc update+move tools
- `uteke upgrade` command (renamed from `uteke update`) (#603)
- Documents now global — no namespace isolation, schema v13 (#614)

## v0.5.0 — LLM Extraction & Hermes Integration `✓ Released 2026-06-27`

- [#46 LLM fact extraction on import](https://github.com/codecoradev/uteke/issues/46) `✓ Done`
  - `uteke import --extract` distills noisy text into atomic facts
- Hermes memory-provider plugin `✓ Done`
  - Automatic recall + extraction, no daemon required
- Configurable embedding endpoint path `✓ Done`
- Default max_seq_length 256 → 2048 `✓ Done`
- Public `store()` accessor for downstream crates `✓ Done`
- rusqlite 0.31 → 0.40 upgrade `✓ Done`

## v0.6.0 — Batch Import & Embed Fallback `✓ Released 2026-06-30`

- Batch directory import (`--batch-dir`) `✓ Done`
  - Auto-detection: `.md` → document, `.txt`/`.jsonl` → memory extraction
  - `--as-doc`/`--as-memory` override, `--recursive`, `--dry-run`
- Cloud embedding fallback `✓ Done`
  - Optional `[embed_fallback]` config for cloud API failover
  - Dimension validation at startup
- Mode C (shell hook) docs `✓ Done`
- Migration upgrade regression test `✓ Done`
- Schema migration fix (#492) `✓ Done`

## v0.4.x — Polish & Stability `✓ Released 2026-06-22–24`

- Hierarchical documents — depth-10 tree engine `✓ Done`
- Hybrid document search (semantic + FTS5 + RRF) `✓ Done`
- MCP tools: `uteke_context`, `uteke_dream` `✓ Done`
- Auto-dream (3-day cycle) + configurable maintenance daemon `✓ Done`
- Dedup on insert (cosine ≥ 0.95) + pinned memory protection `✓ Done`
- Binary version mismatch fix, release workflow fixes `✓ Done`

## v0.3.0 — Graph RAG `✓ Released 2026-06-21`

- [#401 Cosine auto-linking + dedup](https://github.com/codecoradev/uteke/issues/401) `✓ Done`
  - `similar_to` (≥0.80) and `possible_duplicate` (≥0.92) edges
- [#404 Configurable limits](https://github.com/codecoradev/uteke/issues/404) `✓ Done`
  - Env vars + `[limits]` config section
- [#405 Markdown/prose chunker](https://github.com/codecoradev/uteke/issues/405) `✓ Done`
  - Heading-aware splitting, code block protection
- [#406 Document engine](https://github.com/codecoradev/uteke/issues/406) `✓ Done`
  - Wiki/knowledge base, schema v11, documents + document_chunks
- [#407 Embed-aware chunking](https://github.com/codecoradev/uteke/issues/407) `✓ Done`
  - Chunk size from `embedder.max_seq_len()`
- [#408 /graph API endpoint](https://github.com/codecoradev/uteke/issues/408) `✓ Done`
  - Nodes + edges JSON for visualization
- [#409 View-only API key](https://github.com/codecoradev/uteke/issues/409) `✓ Done`
  - Dual-role tokens (admin + read-only)
- [#410 Hermes plugin room_remember](https://github.com/codecoradev/uteke/issues/410) `✓ Done`
- [#411 Document CLI commands](https://github.com/codecoradev/uteke/issues/411) `✓ Done`
  - `uteke doc create/get/list/delete/export`

## v0.2.1 — DX & Ecosystem `✓ Released 2026-06-21`

- [#337 OpenAI + Ollama embedding backends](https://github.com/codecoradev/uteke/issues/337) `✓ Done`
  - reqwest-based HTTP backends, ONNX stays default
- [#346 Typed auto-edges — self-wiring knowledge graph](https://github.com/codecoradev/uteke/issues/346) `✓ Done`
  - Auto-wired memory edges on every `remember()`
- [#347 Timeline event tracking](https://github.com/codecoradev/uteke/issues/347) `✓ Done`
  - Per-memory chronological audit log
- [#348 Citation & source attribution](https://github.com/codecoradev/uteke/issues/348) `✓ Done`
  - `source`, `source_type` columns (schema v10)
- [#349 Memory type formalization](https://github.com/codecoradev/uteke/issues/349) `✓ Done`
  - Typed categories with auto-inference
- [#350 Backlink auto-generation](https://github.com/codecoradev/uteke/issues/350) `✓ Done`
  - Bidirectional memory edges
- [#351 Orphan detection](https://github.com/codecoradev/uteke/issues/351) `✓ Done`
  - Find disconnected memories in the graph
- [#352 Salience + recency dual-axis recall](https://github.com/codecoradev/uteke/issues/352) `✓ Done`
  - Boost by memory type and age
- [#353 Dream cycle](https://github.com/codecoradev/uteke/issues/353) `✓ Done`
  - Coordinated maintenance pipeline (lint → backlinks → dedup → orphans)
- [#381 MCP Streamable HTTP transport](https://github.com/codecoradev/uteke/issues/381) `✓ Done`
  - Protocol version `2025-06-18`, `POST /mcp` endpoint
- [#385 Hermes plugin auto-install](https://github.com/codecoradev/uteke/issues/385) `✓ Done`
  - Direct install to `~/.hermes/plugins/uteke-tool/`
- [#393 `uteke room create` command](https://github.com/codecoradev/uteke/issues/393) `✓ Done`
- [#395 Room operations in Hermes plugin](https://github.com/codecoradev/uteke/issues/395) `✓ Done`
- [#402 Fix: plugin missing `__init__.py`](https://github.com/codecoradev/uteke/issues/402) `✓ Done`
- [#403 Fix: contradictory server log + DB size label](https://github.com/codecoradev/uteke/issues/403) `✓ Done`

## v0.1.0 — Rooms, Intelligence & Pluggability `✓ Done`

- [#292 Time-travel queries](https://github.com/codecoradev/uteke/issues/292) `✓ Done`
  - Recall/list at specific point in time (`--at` flag)
  - Temporal validity filter: created_at, valid_from/valid_until, deprecated
- [#249 Pluggable embedding models](https://github.com/codecoradev/uteke/issues/249) `✓ Done`
  - `Embedder` trait abstraction for multiple backends
  - ONNX backend (default), future: OpenAI, Ollama
- [#306 Room document view](https://github.com/codecoradev/uteke/issues/306) `✓ Done`
  - Structured document output grouped by memory_type
- [#305 Room summary](https://github.com/codecoradev/uteke/issues/305) `✓ Done`
  - LLM-free room summary via tag clustering
- [#304 Semantic room recall](https://github.com/codecoradev/uteke/issues/304) `✓ Done`
  - `recall_room_semantic()` with `--query` flag
- [#184 Normalize tags junction table](https://github.com/codecoradev/uteke/issues/184) `✓ Done`
  - Schema v5, memory_tags table, O(log n) lookups
- [#252 Configurable recall threshold](https://github.com/codecoradev/uteke/issues/252) `✓ Done`
  - `--min`, `--strict`, `[recall] min_score` config
- [#286 Room-based memory](https://github.com/codecoradev/uteke/issues/286) `✓ Done`
  - Full room management with author attribution
- [#181 Recall cache optimization](https://github.com/codecoradev/uteke/issues/181) `✓ Done`
  - LRU cache with TTL, `--context` output format
- [#246 Relationship graph](https://github.com/codecoradev/uteke/issues/246) `✓ Done`
  - `--related --depth N` BFS traversal
- [#247 Smart memory decay](https://github.com/codecoradev/uteke/issues/247) `✓ Done`
  - Composite importance scoring, pin/unpin
- [#49 Benchmark suite](https://github.com/codecoradev/uteke/issues/49) `✓ Done`
  - `uteke bench` command + LongMemEval retrieval harness
- [#316 LongMemEval harness](https://github.com/codecoradev/uteke/issues/316) `✓ Done`
  - Retrieval accuracy evaluation (Recall@k, NDCG@k)

## v0.0.15 — CLI Performance `✓ Done`

- [#185 Lazy ONNX model loading](https://github.com/codecoradev/uteke/issues/185)
  - CLI cold start: ~3s → ~20ms for non-embedding commands
  - Model loaded on first use (`remember`, `recall`, `search`)
- [#131 Modular CLI refactor](https://github.com/codecoradev/uteke/issues/131)
  - CLI args extracted to `cli.rs`, logging to `logging.rs`
- Release workflow decoupled: parallel builds + crates.io publish

## v0.0.14 — Security & Polish `✓ Done`

- [#134 Binary checksums & file permissions](https://github.com/codecoradev/uteke/issues/134)
  - SHA256 checksum verification for ONNX model files
  - Owner-only permissions (0700/0600) on database and model dirs
- [#277 Indonesian README translation](https://github.com/codecoradev/uteke/issues/277)
- [#100 TLS & Reverse Proxy docs](https://github.com/codecoradev/uteke/issues/100)
- Crates.io metadata in all Cargo.toml files

## v0.0.13 — Search & Concurrency `✓ Done`

- [#250 FTS5 hybrid search with RRF](https://github.com/codecoradev/uteke/issues/250)
  - FTS5 full-text search as parallel retrieval channel
  - Reciprocal Rank Fusion (k=60) merges vector + FTS5 results
  - Schema migration v1→v2 (auto, zero data loss)
- [#251 Metadata enrichment via CLI flags](https://github.com/codecoradev/uteke/issues/251)
  - `--entity`, `--category`, `--meta key:value,...` on remember
- [#209 Concurrent reads via RwLock](https://github.com/codecoradev/uteke/issues/209)
  - `Mutex<VectorIndex>` → `RwLock<VectorIndex>` for read-heavy workload
- [#139 Vector index consistency](https://github.com/codecoradev/uteke/issues/139)
  - Atomic save for `.keys` sidecar file (temp + rename)

## v0.0.10 — Codebase Quality `✓ Done`

- [#187 Split commands.rs into per-command modules](https://github.com/codecoradev/uteke/issues/187)
- [#186 Split store.rs into focused modules](https://github.com/codecoradev/uteke/issues/186)
- [#178 Remove all Hermes branding](https://github.com/codecoradev/uteke/issues/178)
- [#196 Address all Cora code review findings](https://github.com/codecoradev/uteke/issues/196)

## v0.0.9 — Website Migration `✓ Done`

- [#194 Website migrated to VitePress](https://github.com/codecoradev/uteke/issues/194)

## v0.0.8 — Stability & Architecture `✓ Done`

- [#130 Architecture: module split](https://github.com/codecoradev/uteke/issues/130), [#132 Input validation](https://github.com/codecoradev/uteke/issues/132), [#134 Binary checksums](https://github.com/codecoradev/uteke/issues/134)
- [#138 Schema versioning](https://github.com/codecoradev/uteke/issues/138), [#144 Error handling rewrite](https://github.com/codecoradev/uteke/issues/144)
- Memory consolidation, import/export (JSONL)

## v0.0.7 — Core Stability `✓ Done`

- [#120 Tag queries → json_each()](https://github.com/codecoradev/uteke/issues/120), [#127 Configurable tier thresholds](https://github.com/codecoradev/uteke/issues/127)

## v0.0.5–v0.0.6 — Docker & Hardening `✓ Done`

- [#95 UTEKE_HOME](https://github.com/codecoradev/uteke/issues/95), [#97 Dockerfile](https://github.com/codecoradev/uteke/issues/97), [#99 GHCR](https://github.com/codecoradev/uteke/issues/99)

## v0.0.4 — Server Mode & Intelligence `✓ Done`

- [#54 Daemon/server mode](https://github.com/codecoradev/uteke/issues/54), [#51 Temporal facts](https://github.com/codecoradev/uteke/issues/51), [#52 Consolidation](https://github.com/codecoradev/uteke/issues/52)

## v0.0.2–v0.0.3 — Foundation `✓ Done`

- [#40 usearch persistent index](https://github.com/codecoradev/uteke/issues/40), [#39 Multi-agent namespaces](https://github.com/codecoradev/uteke/issues/39)
- [#38 Tiered memory](https://github.com/codecoradev/uteke/issues/38), [#42 Tag management](https://github.com/codecoradev/uteke/issues/42)

---

## v0.2.0 — Knowledge Graph & Scale `✓ Done`

- [#317 SQLite graph storage](https://github.com/codecoradev/uteke/issues/317) — optional knowledge graph mode
- [#245 Code-aware embedding with AST chunking](https://github.com/codecoradev/uteke/issues/245) — entity extraction from code
- [#293 Structured memory](https://github.com/codecoradev/uteke/issues/293) — nested JSON content
- [#46 Import from external knowledge sources](https://github.com/codecoradev/uteke/issues/46)
- [#55 Hermes plugin](https://github.com/codecoradev/uteke/issues/55) — uteke integration
