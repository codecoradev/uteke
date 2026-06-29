---
title: Roadmap
---

# Roadmap

Demand-gated — we build what people actually use. Track progress on [GitHub Issues](https://github.com/codecoradev/uteke/issues).

## v0.5.0 — LLM Extraction & Hermes Integration `✓ Released 2026-06-27`

- [#46 LLM fact extraction on import](https://github.com/codecoradev/uteke/issues/46) `✓ Done`
  - `uteke import --extract` distills noisy text into atomic facts
- Hermes memory-provider plugin `✓ Done`
  - Automatic recall + extraction, no daemon required
- Configurable embedding endpoint path `✓ Done`
- Default max_seq_length 256 → 2048 `✓ Done`
- Public `store()` accessor for downstream crates `✓ Done`
- rusqlite 0.31 → 0.40 upgrade `✓ Done`

## v0.6.0 — Batch Import & Embed Fallback `:soon:`

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
