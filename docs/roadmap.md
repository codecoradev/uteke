---
title: Roadmap
---

# Roadmap

Demand-gated — we build what people actually use. Track progress on [GitHub Issues](https://github.com/codecoradev/uteke/issues).

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

## v0.0.12 — Search & Concurrency `✓ Done`

- [#250 FTS5 hybrid search with RRF](https://github.com/codecoradev/uteke/issues/250) `✓ Done`
  - FTS5 full-text search as parallel retrieval channel
  - Reciprocal Rank Fusion (k=60) merges vector + FTS5 results
  - `RecallStrategy` enum: hybrid (default), vector, fts5
  - Schema migration v1→v2 (auto, zero data loss)
  - Phrase search + token-OR fallback
  - Deprecated memories excluded from FTS5
- [#251 Metadata enrichment via CLI flags](https://github.com/codecoradev/uteke/issues/251) `✓ Done`
  - `--entity`, `--category`, `--meta key:value,...` on remember
  - Post-filter on `recall` and `list` by entity/category
  - Auto type detection for meta values (string/number/bool)
- [#209 Concurrent reads via RwLock](https://github.com/codecoradev/uteke/issues/209) `✓ Done`
  - `Mutex<VectorIndex>` → `RwLock<VectorIndex>` for read-heavy workload
  - Multiple concurrent recalls share read lock
  - Embedder lock scope minimized
- [#139 Vector index consistency](https://github.com/codecoradev/uteke/issues/139) `✓ Done`
  - Atomic save for `.keys` sidecar file (temp + rename)
  - `insert()` and `build()` now return `Result` (error propagation)

## v0.0.10 — Codebase Quality `✓ Done`

- [#187 Split commands.rs into per-command modules](https://github.com/codecoradev/uteke/issues/187) `✓ Done`
- [#186 Split store.rs into focused modules](https://github.com/codecoradev/uteke/issues/186) `✓ Done`
- [#178 Remove all Hermes branding](https://github.com/codecoradev/uteke/issues/178) `✓ Done`
- [#196 Address all Cora code review findings](https://github.com/codecoradev/uteke/issues/196) `✓ Done`
- Safe slice, index lock ordering, HTTP status checking, aging filter fix
- Schema migration transactions, batch bulk deletes, SQLite-first dual-write
- Embedding docs corrected (768d), shell hook idempotency guards

## v0.0.9 — Website Migration `✓ Done`

- [#194 Website migrated to VitePress](https://github.com/codecoradev/uteke/issues/194) `✓ Done`
  - SvelteKit → VitePress, built-in full-text search

## v0.0.8 — Stability & Architecture `✓ Done`

- [#130 Architecture: module split](https://github.com/codecoradev/uteke/issues/130), [#132 Input validation](https://github.com/codecoradev/uteke/issues/132), [#134 Binary checksums](https://github.com/codecoradev/uteke/issues/134)
- [#138 Schema versioning](https://github.com/codecoradev/uteke/issues/138), [#144 Error handling rewrite](https://github.com/codecoradev/uteke/issues/144)
- [#137 Python wrapper: 21 methods](https://github.com/codecoradev/uteke/issues/137), [#49 Memory benchmark](https://github.com/codecoradev/uteke/issues/49)
- Memory consolidation, import/export (JSONL)

## v0.0.7 — Core Stability `✓ Done`

- [#120 Tag queries → json_each()](https://github.com/codecoradev/uteke/issues/120), [#127 Configurable tier thresholds](https://github.com/codecoradev/uteke/issues/127)
- [#129 Test coverage: 34 → 94](https://github.com/codecoradev/uteke/issues/129)

## v0.0.5–v0.0.6 — Docker & Hardening `✓ Done`

- [#95 UTEKE_HOME](https://github.com/codecoradev/uteke/issues/95), [#97 Dockerfile](https://github.com/codecoradev/uteke/issues/97), [#99 GHCR](https://github.com/codecoradev/uteke/issues/99)
- Non-root container, Dependabot, JSON output omits embeddings

## v0.0.4 — Server Mode & Intelligence `✓ Done`

- [#54 Daemon/server mode](https://github.com/codecoradev/uteke/issues/54), [#51 Temporal facts](https://github.com/codecoradev/uteke/issues/51), [#52 Consolidation](https://github.com/codecoradev/uteke/issues/52)
- [#43 Bulk operations](https://github.com/codecoradev/uteke/issues/43), [#45 Namespace switching](https://github.com/codecoradev/uteke/issues/45), [#86 Cora CI review](https://github.com/codecoradev/uteke/issues/86)

## v0.0.2–v0.0.3 — Foundation `✓ Done`

- [#40 usearch persistent index](https://github.com/codecoradev/uteke/issues/40), [#39 Multi-agent namespaces](https://github.com/codecoradev/uteke/issues/39)
- [#38 Tiered memory](https://github.com/codecoradev/uteke/issues/38), [#37 Health checks](https://github.com/codecoradev/uteke/issues/37)
- [#42 Tag management](https://github.com/codecoradev/uteke/issues/42), [#44 Memory aging](https://github.com/codecoradev/uteke/issues/44)
- [#48 Configuration file](https://github.com/codecoradev/uteke/issues/48), [#47 Shell hooks](https://github.com/codecoradev/uteke/issues/47)
- Graceful shutdown, file logging with rotation

## What's Next

Demand-gated — we build what people actually use.

- Better embeddings (larger model option)
- [#46 Import from external knowledge sources](https://github.com/codecoradev/uteke/issues/46)
- Python SDK (PyO3 bindings)
- Editor integrations (VS Code, JetBrains)
- Node.js SDK (NAPI)
- [#100 TLS support](https://github.com/codecoradev/uteke/issues/100)
- [#101 API key auth for uteke-serve](https://github.com/codecoradev/uteke/issues/101)
