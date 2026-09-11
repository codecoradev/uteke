# Changelog

## [Unreleased]

### Added

- **Benchmarks directory restructure** — `benchmarks/` is now the single source of truth: `benchmarks/README.md` (index, policies, reproduction), `internal/` (uteke bench, re-verified on v0.17.0), `longmemeval/` (harness + committed canonical raw artifacts under `results/` + corrected baselines), `locomo/` (planned external benchmark #2). The old `benchmarks/RESULTS.md` stub (stale pre-embedding figures) is removed — real numbers live in `internal/RESULTS.md` and `docs/benchmarks.md`.

### Fixed

- **Default recall `min_score` lowered 0.3 → 0.0 (#1223)** — since the fusion strategy (weighted RRF) became the recall default in 0.16.0, returned scores are rank-based (RRF contribution plus salience/recency boosts, typically ~0.0-0.2) rather than cosine similarity, so the CLI default threshold of 0.3 (a cosine-era value; HTTP/MCP already default 0.0) silently filtered out almost every result — measured on embeddinggemma-q4: all 20 paraphrase probes scored 0.169-0.186 rank-1 while a legacy-distribution 0.3 default expects ~0.5+, making default `uteke recall` return empty. Thresholds remain opt-in via `[recall] min_score`, `--min`, `--strict` (0.5), or HTTP `min_score`; docs now state the score scale per strategy.
- **LongMemEval baseline mislabel** — RESULTS.md previously attributed the 0.854/0.885 (2026-08-13) full-500 baseline to "v0.15.0 hybrid"; those figures are the **vector-only** run (hybrid was validated at 50 questions only). Corrected, with both delta bases stated (+8.9pp full-500, +9.2pp 470-non-abstention).
- **Full-dataset aggregates** — RESULTS.md now labels all three bases explicitly: Overall (470 non-abstention), Abstention (30), and Overall (full 500 = 0.982 recall_any@5 / 0.880 strict recall_all@5); no questions silently dropped from the aggregate rows.
- Removed stale run logs and an outdated metrics snapshot from git tracking (values superseded by recomputation from committed raw artifacts).

- **Room rename, room update, and memory room-move (#1202)** — rooms now have sanctioned lifecycle ops matching the namespace family: `Uteke::rename_room(old, new)` rewrites the registry row and every `room_memories`/`room_documents` reference in ONE transaction (FK-safe insert→repoint→delete order; no `ON UPDATE CASCADE` needed), preserving title/description/namespace; `Uteke::update_room(id, title?, description?)` edits room metadata (schema v19 adds the additive `rooms.description` column; structural export/import carry it and stay backward-compatible with 5-column exports); `Uteke::move_memory_to_room(id, from, to)` moves a memory between rooms preserving the link's `author`/`role`/`joined_at` provenance (`Ok(0)` when not a member of `from`). Surfaces: HTTP `POST /room/rename|/room/update|/room/memory/move` (404 on missing room/link), CLI `uteke room rename|update|move-memory`, MCP `uteke_room_rename|uteke_room_update|uteke_room_memory_move`.
- **`uteke update <id>` CLI** — in-place memory edit (content/tags/importance/pinned/type), closing the CLI surface gap: HTTP `PUT /memory` and MCP `uteke_update` already existed (#1202 item 5).

## [0.17.0] — 2026-09-06

Minor release. Theme: **inspectable, trustworthy memory** — explain recall on
every surface, auditable conflict resolution with a measurable payoff, honest
graphs, pagination metadata, and a dual-engine vector layer.

### Added

- **Contradiction benchmark segment (#1172, phase 3)** — `benchmarks/longmemeval/contradiction_segment.py`: 40-topic active-store segment measuring conflict-resolution quality end-to-end. Baseline (both facts active) vs resolved (superseded): fusion winner@1 0.850 → 1.000, stale@5 1.000 → 0.000. Published in `benchmarks/longmemeval/RESULTS.md`. Also adds `uteke supersede <old> <new> [--reason]` — CLI surface parity for supersession (previously MCP/HTTP only).

- **`/list` pagination metadata (#1188)** — `POST /list` accepts `"include_meta": true` to respond with an envelope `{memories, total, has_more, next_offset}` (`next_offset` is `null` on the last page) so clients no longer blind-paginate with 100-row guesses. The default response is unchanged (bare array) — existing clients are untouched; `include_meta` is ignored in `at` (point-in-time) mode, which stays a bare array.

- **Explain recall (#1160)** — `explain` mode on every recall surface shows WHY each memory ranked where it did: vector similarity and rank, FTS rank, RRF score with per-channel fusion contributions, and jaccard/salience/recency/graph boost deltas. Surfaces: `uteke recall "…" --explain` (human-readable, combine with `--json` for machine output), `POST /recall` with `"explain": true` (memory-only — combined with `search_type`/`at`/`before`/`after` returns 400), and the `explain` flag on the MCP `uteke_recall` tool. The explanation path replays the active strategy's exact pipeline (same channel depths, RRF constants, and boost order) while bypassing the recall cache, so the explanation always matches the returned results; fts5 explanation works without an embedder, other strategies embed the query once (~50 ms, same as a cold recall).

- **Contradiction resolution ledger + undo (#1172, phase 2)** — supersessions are now a first-class, auditable ledger instead of a side effect: `Uteke::contradiction_resolutions(namespace, limit)` lists superseded-but-not-restored memories (winner, reason, timestamp via the deprecation row), `Uteke::undo_supersession(id)` restores a retired memory, removes the supersession edge pair, and records a `supersession_undone` event on both sides (only memories carrying a live `superseded_by` edge can be undone — the undo is itself auditable). Ledger membership is edge-driven (deprecated row + `superseded_by` edge), the same predicate undo resolves against, and re-superseding an already-deprecated memory refreshes the stored reason/timestamp so the ledger always names the current winner. Surfaces: `GET /contradictions?namespace=&limit=`, `POST /contradictions/undo` (`{id}`; 404 when nothing to undo), `uteke contradictions list|undo`, and MCP `uteke_contradictions` / `uteke_contradictions_undo`. Fixed in the process: the no-namespace ledger query bound its limit parameter to a nonexistent placeholder (`?2`) and failed at runtime — caught by the new MCP roundtrip test.

- **Provenance data model (#1172, phase 1)** — schema v18 (additive): `memories.source_hash` records the SHA-256 of content at write time (tamper evidence — audits recompute it against live content), and `timeline_events.actor`/`evidence_json` record who performed an event and what evidence supports it. New `Uteke::provenance(id)` returns the full report (provenance fields, trust tier, hash comparison, event chain) — exposed as `GET /provenance?id=`, `uteke provenance <id>`, and the `uteke_provenance` MCP tool.

### Fixed

- **Graph data returned stale nodes (#1189)** — `GET /graph` without a namespace returned every `graph_nodes` row raw, including nodes whose parent memory had been forgotten or deprecated; with soft-delete the store accumulated stale nodes on every conflict resolution. Memory-linked nodes are now filtered by liveness (memory exists and `deprecated = 0`) in every `graph_data` path, edges touching removed nodes are dropped, and `stats` counts the filtered graph.
- **Memory graph nodes labeled with raw UUIDs (#1187)** — `ensure_node_for_memory` now labels new memory nodes with a readable content preview (first 60 chars of the memory) instead of the raw memory UUID, and upgrades legacy UUID-labeled rows in place on next access. Entity nodes are unaffected.

- **Namespace management API (#1181)** — namespaces are a derived view, now with sanctioned ops: `PUT /memory` accepts `namespace` (move a memory — plain column update, no re-embed), `POST /namespaces/rename` (`{from, to}`; existing target = merge, returns `{from, to, moved, target_existed}`), and `POST /namespaces/delete` with an explicit strategy for its memories: `refuse` (default — 409 while any memory references the name), `merge` (move all memories to `target`, the name vanishes), or `deprecate` (soft-delete — restorable via promote, never hard-deleted). `GET /namespaces?with_counts=true` now adds `active`/`deprecated` breakdown fields (`count` stays the total). CLI parity: `uteke namespace move|rename|delete` (delete requires `--confirm`). MCP parity: `uteke_namespace_rename`, `uteke_namespace_delete`, and `namespace` field on `uteke_update`.

### Fixed

- **`POST /graph/edge` always returned 500 for valid memory IDs (#1180)** — the handler validated `source`/`target` as memory IDs but inserted them directly into `graph_edges`, whose foreign keys point at `graph_nodes(id)`. Memory IDs are now resolved to their linked graph node (or a node is ensured automatically) before insertion. `DELETE /graph/edge` accepts memory IDs or graph node IDs the same way, and its documented query params are corrected to `?source=...&target=...`. `POST /graph/edge` now responds with `{ok, source_node, target_node}` so clients can track the created nodes.

## [0.16.0] — 2026-08-28

Minor release. One theme: retrieval quality that ships by default.

The new `fusion` strategy — weighted Reciprocal Rank Fusion of the vector and hybrid rankings — is now the default everywhere: CLI, HTTP API, and MCP. Vector and hybrid fail on different questions; fusing both captures each side's wins. Zero config needed.

### Added

- **`fusion` recall strategy (#1123)** — runs vector and hybrid rankings and RRF-fuses them (k=60, weights tuned on LongMemEval fast50 actual x86 rankings). LongMemEval fast50: R@5 0.98 vs 0.9267 hybrid, R@10 1.0. Available on every surface: `--strategy fusion`, HTTP `strategy: "fusion"`, MCP `strategy: "fusion"`.

### Changed

- **Default recall strategy: `hybrid` → `fusion` (#1123)** — applies ONLY when no strategy is specified (CLI flag, HTTP field, MCP param, or `default_strategy` config). Existing configs with an explicit `default_strategy` are untouched.

### Validated

- **Full-release validation: pure-default 500Q LongMemEval run (2026-08-29)** — zero-config `--strategy default` on the complete validation set: **R@5 0.946 / R@10 0.977** on 470 non-abstention questions (**+9.2 pts** R@5 vs 0.15.0 hybrid baseline 0.854). Binary built from the exact release SHA; aggregate results in `benchmarks/longmemeval/RESULTS.md`, raw per-question results on the benchmark Modal volume for independent verification.
- **Public benchmark page + comparison chart (#1141)** — `docs/benchmarks.md` now publishes the dual-metric view from the same run: recall_any@5 **98.2%** (the metric competitor benchmarks publish) alongside the stricter recall_all family (recall_all@10 95.4%, strict recall_all@5 88.0% with mathematical ceiling 99.4%, coverage@5 94.3%). No other system in the comparison publicly reports the strict family.

## [0.15.0] — 2026-08-19

Minor release. The theme: memory you can trust across surfaces, and a store you can move.

New memories now get UUIDv7 IDs — time-ordered at the front, so indexes and timelines sort naturally instead of jumping around. Existing v4 IDs keep working untouched.

### Added

- **Supersession workflow (#1069)** — mark a stale decision as superseded by a newer one. Recall flags superseded entries so agents stop acting on outdated decisions.
- **Structural export (#1068)** — full-store round-trip: rooms, graph, edges, documents, and timeline export to a single file and import back without loss.
- **MCP `uteke_get` + `uteke_update` (#1067)** — read and edit a single memory by ID, with short-ID resolution on every ID-taking tool.
- **Richer MCP outputs (#1066)** — `uteke_stats` includes tier and namespace breakdowns, `uteke_doc_search` returns scores, room results carry room IDs.

### Fixed

- **`uteke_dream` no longer destructive by default (#1065)** — runs as dry-run first; destructive passes require explicit scope or confirmation.
- **`/export` keeps namespace attribution (#1064)** — exported rows no longer lose which namespace they came from, and the deprecated-row delta is documented.
- **Recall cache score parity (#1063)** — a cache hit used to skip salience and recency boosts, so the same query scored differently cold vs warm. Both paths now score identically.
- **`search_content(None)` no longer collapses to the default namespace (#1062)** — keyword search without an explicit namespace now searches across namespaces as expected.
- **Soft-forgotten memories stay hidden (#1061)** — they were leaking into list, search, and doctor counts.

### Changed

- **UUIDv4 → UUIDv7 for new IDs (#1060)** — time-sortable IDs. Existing v4 IDs are fully compatible; nothing migrates, nothing breaks.

## [0.14.3] — 2026-08-15

Patch release: recall `strategy` now behaves the same everywhere. The CLI got the fix in #900; the HTTP API and MCP server never got the wiring, so the same request quietly returned different results depending on which surface you asked.

### Fixed

- **HTTP recall strategy: default is hybrid, not legacy vector (#1034)** — A request without `strategy` silently fell back to the legacy vector path, and `strategy: "bogus"` returned 200 OK with vector results behind it. Strategy is now resolved once per request (request > `[recall] default_strategy` from uteke.toml > hybrid) and invalid values return HTTP 400 on every path — bare recall, unified search, and v1. The eager legacy recall that ran before strategy resolution is gone, which also removes a wasted query on every memory recall.
- **MCP `uteke_recall` exposes `strategy` (#1035)** — The tool schema didn't list a `strategy` parameter, silently hardcoded vector, and ignored invalid values. The schema now documents `vector | fts5 | hybrid | graph`, the default resolves to hybrid, and invalid values return a loud JSON-RPC error instead of a quiet vector search.
- **Server honors `[recall] default_strategy`** — The HTTP server now reads the config key the CLI has documented all along. A typo'd value is caught at startup with a warning and falls back to hybrid, so one bad config line can't 400 every request with a message blaming the request.
- **Memory recall with entity/category filters** now routes through the hybrid engine with the same 3× over-fetch post-filter the unified path uses, instead of a vector-only path that could miss FTS5 matches.

### Verified

Empirically, against a scratch store with a release build: HTTP matrix 10/10 (default hybrid, all four strategies, 400 on invalid across all three paths, config override, startup sanitization), MCP harness 8/8 (schema, defaults, loud errors, engine parity default == hybrid on warm cache). Workspace suite 538/538.

Known issues filed during this work: #1036 (export drops namespace), #1037 (recall cache-hit skips salience/recency boosts).

## [0.14.2] — 2026-08-14

Patch release to restore the uteke-cli crates.io publish. The CLI crate embedded asset files from outside its package root, which `cargo publish` cannot bundle, so every publish attempt since June failed silently while the release workflow reported success. The crate was stuck at 0.4.3 on crates.io while the repo shipped 0.14.x.

### Fixed

- `uteke-cli` publishes again: the memory-skill and provider templates now live inside `crates/uteke-cli/assets/` and ship inside the `.crate` archive
- The release workflow now verifies every crate reports the tagged version on crates.io and fails the job if a publish step was silently skipped

### Note for `cargo install` users

If you installed `uteke-cli` before this release, you have a build from June. Run `cargo install uteke-cli --version 0.14.2` to get the current CLI.

## [0.14.1] — 2026-08-14

Patch release: two chunker bug fixes found by mutation testing, plus test-suite hardening. No API or behavior changes beyond the fixes.

### Fixed

- **Markdown heading duplicated in oversized sections** — When a markdown section exceeded `max_chunk_chars`, its heading appeared twice in the first sub-chunk: once from `split_by_headings`, then again from the sub-chunking loop. Every oversized markdown document was quietly producing corrupted chunk content, which flows directly into embedding input and degrades retrieval quality. The dead re-prepend path is removed entirely (#1024).

- **Infinite loop on multi-byte text with small chunk sizes** — The zero-progress guard in `split_long_text` advanced by raw byte offsets that could land inside multi-byte UTF-8 characters (CJK, emoji), flooring back to the start position forever. `chunk_markdown("日本語", 2)` would hang. The guard now advances to the next valid character boundary (#1024).

### Changed

- **cargo-mutants configuration moved to `.cargo/mutants.toml`** — The previous config at `crates/uteke-core/mutants.toml` used the key `exclude_files`, which is not a valid field in cargo-mutants v27 and was silently never applied. Migration to `.cargo/mutants.toml` (the location cargo-mutants actually reads) with the correct v27 schema (`exclude_globs`, `exclude_re` as arrays) and workspace-root-relative glob paths. This release is the first where mutant exclusions actually work.

- **Chunker test suite grew from 20 to 60 tests** — Mutation score improved from 50% to 97% (149/164 mutants caught, 0 missed non-equivalent). 3 proven-equivalent mutants are excluded with documented reasoning.

## [0.14.0] — 2026-08-14

Minor release: hybrid recall as default strategy, scene-segmented extraction, memory tools guide, lifecycle introspection, and source provenance. One behavior change (recall default).

### ⚠️ Behavior Change

- **Hybrid (RRF) is now the default recall strategy (#1005)** — Previously defaulted to vector-only. Hybrid combines vector similarity with FTS5 keyword matching via Reciprocal Rank Fusion, improving recall@5 from 85.4% to 98.0% on the internal benchmark. No action needed — existing setups automatically benefit. To revert to vector-only, set `strategy = "vector"` in `[recall]`.

### Added

- **Scene-segmented LLM extraction with priority scoring (#1009)** — When using `--extract` with an LLM, facts are now grouped by topic (scene) and assigned a priority score (0.0–1.0). Each fact gets a `scene:<topic>` tag for filtering, a semantic type (`decision`, `fact`, `preference`), and an importance value reflecting its priority. Offline extraction is unaffected. Backward compatible — flat string arrays from older models still parse correctly.

- **List deprecated memories endpoint (#1007)** — `GET /lifecycle/deprecated` returns deprecated memories with TTL metadata (deprecated date, expiry date, remaining days). Useful for auditing what's slated for pruning before it's gone.

- **Memory tools guide endpoint (#1010)** — `GET /guide` returns an agent-facing reference document covering all available memory operations. Designed for system-prompt injection — agents can discover capabilities without hardcoded instructions.

- **Source provenance for extracted memories (#1012, #1013)** — Memories created via `--extract` now automatically record their source file path and extraction timestamp. CLI and server paths both covered.

### Fixed

- **Cross-compilation fails for Android/iOS targets (#1014)** — `ORT_LIB_NAME` environment variable was not cfg'd for mobile targets, breaking `cargo build --target aarch64-linux-android` and `aarch64-apple-ios`. Fixed with conditional compilation flags.

- **Update check wastes API calls in batch mode (#1006)** — The startup update check ran even during `--batch-dir` imports, adding latency to batch operations. Now skipped when `--batch-dir` is active.

- **Embedding text not pre-truncated before API call (#1002)** — Long content strings were sent to the embedding API untruncated, causing rejections from providers with token limits. Text is now pre-truncated to the configured `max_chunk_tokens`.

### Contributors

- [@ajianaz](https://github.com/ajianaz)

---

## [0.13.2] — 2026-08-11

Patch release with update notifications, a startup crash fix, and benchmark accuracy improvements. No breaking changes.

### Added

- **Update check across all surfaces (#990)** — Uteke now checks GitHub for newer releases on startup (CLI, MCP, HTTP). Uses a 302 redirect as primary source (no API rate limit), REST API as fallback, with a 24-hour cache (`~/.config/uteke/update-cache.json`). MCP server runs the check in a detached background thread to avoid blocking JSON-RPC.

### Fixed

- **NULL embedding rows crash vector index build on startup (#992, #993)** — `load_all()` returned rows with NULL embedding blobs, which `row_to_memory()` silently converted to `vec![]`. When these reached `index.build()`, the dimension validator rejected them (0 ≠ 768), crashing the server on startup and blocking `repair_index`. Fixed with a SQL filter (`WHERE embedding IS NOT NULL`) plus defense-in-depth guards at call sites.

- **Update check redirect parser fails on absolute URLs (#994)** — GitHub 302 redirects return absolute URLs (`https://github.com/.../v0.13.1`), not relative paths. The previous `strip_prefix('/codecoradev/...')` always failed on absolute URLs, falling through to a fallback that required a `v` prefix. Fixed with `find()` to locate the tag marker anywhere in the URL. Also fixed a UTF-8 safety issue: byte-indexed slicing replaced with `split_once()`.

- **LongMemEval benchmark false negatives from CLI score threshold (#995, #996)** — The benchmark harness called `uteke recall` without `--min`, inheriting the CLI default threshold of 0.3. Relevant evidence sessions scoring 0.30–0.40 were silently filtered, causing 8/500 false negatives in the Oracle subset. Fixed by passing `--min 0.0` to evaluate raw ranking quality. Also documented the threshold chain inconsistency: CLI defaults to 0.3 (UX), HTTP API and MCP default to 0.0.

### Changed

- **Branding refresh (#991)** — Tagline updated to "One memory. Every agent. Zero cloud." Install URL changed to `codecora.dev/uteke/install`. MCP compatibility (Claude, Cursor, Copilot) now mentioned in subtitle.

### Contributors

- [@ajianaz](https://github.com/ajianaz)

---

## [0.13.1] — 2026-08-10

Patch release focused on security hardening, data integrity, and query performance. No breaking changes. 13 commits across 32 files.

### Fixed

#### Security
- **Command injection via execSync in Hermes plugin (#973, #969)** — URL parameter was passed unsanitized to a shell command. Now goes through proper argument escaping.
- **install.ps1 checksum mismatch does not stop installation (#980)** — Corrupted or tampered binaries were installed silently. The installer now aborts on checksum failure.

#### Data Integrity
- **aging_cleanup missing pinned filter → data loss (#974)** — Pinned memories could be hard-deleted during aging. Added `WHERE pinned = 0` guard.
- **FTS5 search reads rank from wrong column index (#975)** — `rank` was read from the column position of `importance`, producing wrong relevance ordering.
- **Pinned memory importance always 0.0 in recompute formula (#976)** — The salience multiplier for pinned memories was zeroed out. Recompute now applies the correct weight.
- **update() and update_fields() lack transaction for memory + tags dual-write (#977)** — A crash between the memory write and the tag write left inconsistent state. Both are now in a single transaction.

#### Crash Prevention
- **UTF-8 panic: string slicing at fixed byte offsets in 8+ locations (#978, #969)** — `&str[..N]` panicked on multi-byte characters. Replaced with `safe_truncate()` which checks `is_char_boundary()`. Zero unsafe blocks remain in the codebase.
- **VectorIndex::build() infers dimensions from first item without validation (#979)** — A single malformed vector silently corrupted the index. Now validates dimension count against the configured value.
- **Chunker infinite loop on truncated UTF-8 at boundary (#969)** — Chunk boundaries landing inside a multi-byte character caused the loop to never advance.
- **CLI JSON construction via format! in 6 locations (#981, #984)** — Manual string concatenation risked injection and encoding errors. Replaced with `serde_json::to_string()`.
- **Windows vector.rs cross-process race condition (#982, #984)** — `exists()` + `write()` had a TOCTOU window. Now uses `OpenOptions::new().create(true).write(true).truncate(false)`.
- **Vector dimension growth overflow on large stores (#970)** — Dimension arithmetic could overflow `usize` on very large indices. Switched to `saturating_add`.

#### Concurrency & Stability
- **consolidate.rs holds write lock for entire loop (#986)** — `index.write()` was inside the loop body. Hoisted outside; `index.save()` runs once after.
- **Server backpressure via busy-spin AtomicUsize (#986)** — Replaced spin-loop thread limiter with a Condvar-based semaphore for proper wakeup/sleep semantics.
- **Vector persist path corruption on Windows (#985)** — Path join produced invalid separators on some Windows locales.

### Changed
- **Route matching now case-sensitive (#970)** — Previously `/Rooms/x` and `/rooms/x` were treated as the same route, leaking data across namespaces.
- **Namespace filter bypass in recall (#970)** — Recall could return memories from other namespaces when the filter was set to `None`. Now defaults to the current namespace.
- **Thread cap for batch operations (#985)** — Edge insertion and re-embed loops now cap concurrent operations to avoid exhausting the connection pool.

### Performance
- **N+1 query elimination across recall, graph BFS, rooms, and edge insertion (#987)** — Three new batch methods: `get_by_ids()` (chunked in batches of 900 to stay under SQLite's 999 host parameter limit), `touch_access_batch()`, and prepared-statement reuse in `add_memory_edges_batch()`. Recall now does 1 query instead of N, hybrid search does 1 query per pass instead of N, graph BFS batches per level, and edge insertion prepares once per transaction.
- **Dead code cleanup (#971)** — Four unused `pub fn` marked `#[deprecated]`.

### Infrastructure
- **CLA check workflow (#966, #967)** — Automated CLA verification on PRs, with skip for already-signed contributors.
- **README image cleanup (#965, #968)** — Removed dead PNG and HTML assets unreferenced after the docs restructure.

### Contributors
- [@ajianaz](https://github.com/ajianaz) — all fixes, performance work, and infrastructure

## [0.13.0] — 2026-08-08

Major release: safe memory lifecycle, offline extraction, persistent embedding cache, repair tooling, and 20+ bug fixes across recall, import, and HTTP API.

### Added

#### Safe Memory Lifecycle
- **Safe memory lifecycle (#928–#935, #937)** — Memories transition through a reviewable soft-delete state before any hard deletion. The flow: `ACTIVE → DEPRECATED (hidden, restorable, 30-day TTL) → PRUNED`. Hard delete is confined to exactly two controlled paths: `prune()` (expired TTL) and `forget()` (only when `soft_delete_only = false`). Everything else redirects to soft-delete by default.
- **`LifecycleConfig` (#928)** — 11 configurable fields governing deprecation age thresholds, per-cycle percentage cap (default 1%), TTL, and auto-cycle scheduling. Conservative defaults out of the box.
- **CLI lifecycle commands (#935)** — `uteke lifecycle status`, `uteke lifecycle cycle`, `uteke lifecycle promote <id>`, `uteke lifecycle restore <id>`. All support `--json` output.
- **HTTP API lifecycle endpoints (#935)** — `GET /lifecycle/status`, `POST /lifecycle/cycle`, `POST /lifecycle/promote`.
- **Memory Lifecycle docs page (#937)** — New `/memory-lifecycle` page with best practices, migration notes, and configuration reference.

#### Extraction & Onboarding
- **Offline rule-based extractor as default (#890)** — New extraction mode that works without an LLM. Handles version splits, header-only facts, and parenthetical qualifiers. No API key required for basic knowledge capture.
- **Onboarding wizard improvements (#889)** — Extraction config step, memory test step, and rooms intro added to `uteke init` flow.

#### Performance & Repair
- **Persistent embedding cache (#896)** — Embedding results cached to disk. Repeated `remember()` calls with identical content skip ONNX inference entirely. Lazy ONNX load: model only loads on first embedding, not on startup.
- **`repair --reembed` (#919)** — Regenerates missing or corrupted embeddings. Scans for memories with null/empty vectors and re-embeds them in batches.
- **`repair --rebuild` (#901)** — Rebuilds the usearch index from scratch when corruption is detected. Graceful fallback: corrupt index no longer crashes the server.

#### CLI & UX
- **CLI `context` subcommand (#908)** — Inspect what Uteke knows about a given query without running a full recall. Includes strict strategy validation.
- **Startup update notification (#917)** — Checks for new releases once per 24h (cached). Non-intrusive: prints a one-line notice if an update is available.

### Changed
- **All delete paths now respect `soft_delete_only` (#930–#932)** — `aging_cleanup()`, `consolidate()`, `delete()`, `bulk_delete()`, `forget()`, and `bulk_forget_*()` all redirect to `deprecate()` when `soft_delete_only = true` (the default). No automated process can hard-delete a memory.
- **Server auto-aging thread → auto-lifecycle thread (#934)** — Background maintenance thread now runs the full 2-phase lifecycle cycle (deprecate + prune) instead of just aging cleanup.
- **Dynamic per-cycle cap (#933)** — Each cycle deprecates at most `max_deprecate_percent` (default 1%) of active memories, clamped to `[1, 50]`. Prevents sudden data loss on large stores.
- **Linux release builds pinned to ubuntu-22.04** — Ensures GLIBC backward compatibility for older Linux distributions.

### Fixed

#### Recall & Search
- **`recall_semantic` returns zero results for small rooms (#894)** — Room search in large stores with small rooms returned empty results. Now falls back to exhaustive search when bounded search yields insufficient coverage.
- **Recall strategy flag ignored in unified search (#900)** — `--strategy` flag was silently overridden. Now properly routes through the correct search path.
- **HTTP temporal filters, limit cap, empty query guard (#902, #903, #907)** — Date-range filters now work correctly. Result limit capped to prevent memory exhaustion. Empty query strings no longer cause 500 errors.
- **Aging count discrepancy across namespaces (#905)** — `count_active()` counted deprecated memories in some namespaces. Fixed with `--dry-run` flag to preview changes.

#### Import & Embedding
- **Import JSON array/object support (#904)** — Previously only JSONL was accepted. Now handles JSON arrays, single objects, and validates minimum required fields.
- **Import JSONL fallback strategy (#940)** — Robust parsing with line-by-line recovery. Malformed lines are skipped with warnings instead of aborting the entire import.
- **Embedder lock scope (#920)** — Lock was held too long during re-embed loops, blocking concurrent reads. Moved outside the loop.
- **ONNX auto-detect Python site-packages** — Resolves ORT shared library from Python virtualenvs automatically. No manual `ORT_LIB_PATH` needed for pip installs.

#### Lifecycle & Safety
- **Bounded lock timeout + forget error propagation (#922, #926, #927)** — Lock contention no longer hangs indefinitely. `forget()` on non-existent IDs now returns an error instead of silently succeeding.
- **Migration: `deprecate_reason` column** — Added via `column_exists()` pattern. No schema version bump. Existing databases upgrade transparently.
- **Offline extraction edge cases (#895)** — Version splits, header-only facts, and parenthetical qualifiers now parsed correctly.

#### Comparison Docs
- **Competitor comparison validated against source repos (#924)** — All 8 competitors verified against their actual README/BENCHMARKS/Cargo.toml. Corrected mismatched claims and stale versions.

### Contributors
- [@ajianaz](https://github.com/ajianaz) — lifecycle system, offline extractor, embedding cache, repair tooling, recall/import fixes, comparison validation

## [0.12.0] — 2026-08-05

### Fixed
- **`merge_from_file` skips 5 config sections (#856)** — Config merge now includes `[embed_fallback]`, `[extraction]`, `[recall]` weights (`salience_weight`, `recency_weight`), `[server]` advanced fields, and `[limits]` advanced fields. Previously these sections were silently ignored when loading from `uteke.toml`.
- **`doc_upsert` double-deletes chunks — usearch index orphaned entries (#857)** — Chunk capture now happens *before* `upsert_document` via new `get_chunk_ids_for_documents()` method. Previously the flow deleted chunks after upsert (which re-creates them), causing orphaned entries in the usearch vector index — leading to index bloat and stale results.
- **`delete_document` leaves dangling `room_documents` references (#858)** — Document deletion now cleans up the `room_documents` junction table via `DELETE FROM room_documents WHERE doc_slug = ?1`. Previously junction rows were orphaned since there's no foreign key constraint on the junction table.
- **Default config template only includes 5 of 12 sections (#862)** — `write_default_config()` now emits `[embed_fallback]`, `[extraction]`, `salience_weight`/`recency_weight` in `[recall]`, and other previously missing sections. Users running `uteke init` now get a complete reference template.
- **Misleading MCP tool name `uteke_room_document` (#861)** — Renamed to `uteke_room_summary_document` for clarity (it generates a summary document, not a CRUD operation). Old name kept as backward-compatible alias — no breaking change for existing MCP clients.

### Added
- **MCP + CLI room-document junction tools (#859)** — The HTTP API had 4 junction endpoints (`/room/add-document`, `/room/remove-document`, `/room/list-documents`, `/doc/list-rooms`) but these were not exposed via MCP or CLI. Added 4 new MCP tools (`uteke_room_add_document`, `uteke_room_remove_document`, `uteke_room_list_documents`, `uteke_doc_list_rooms`) and 4 CLI subcommands (`room add-document`, `room remove-document`, `room list-documents`, `room list-rooms`). MCP and CLI clients can now manage room↔document links without HTTP.
- **Windows PowerShell installer (#779)** — Added `install.ps1` for native Windows installation via PowerShell. Mirrors the bash installer (`install.sh`) — auto-detects latest release, downloads binary, adds to PATH.

### Changed
- **FTS5 documents index now includes content body (#860)** — The `documents_fts` virtual table previously indexed only `title` and `slug` columns. Now also indexes `content`, enabling full-text search across document bodies. Schema migration auto-detects old FTS tables (missing `content` column via `pragma_table_info`) and recreates with full indexing + backfill. Fresh databases get 3-column FTS5 from initial schema.

### Contributors
- [@ajianaz](https://github.com/ajianaz) — all fixes, features, and FTS5 migration
- [@KazamiHazaki](https://github.com/KazamiHazaki) — Windows PowerShell installer (#779)
- [@dependabot](https://github.com/apps/dependabot) — dependency updates (toml, clap, clap_complete, serde)

## [0.11.0] — 2026-08-03

### Fixed
- **`POST /doc/move` silent data loss on unknown fields (#833)** — Added `#[serde(deny_unknown_fields)]` to `DocMoveRequest`. Consumers sending wrong field names (e.g. `parent_slug` instead of `new_parent`) now get HTTP 400 instead of doc being moved to root with no error.
- **`doc_move()` UUID fallback for parent resolution (#833)** — `doc_move()` now tries slug lookup first, then UUID lookup for `new_parent`. Consistent with source doc resolution and `doc_delete()` behavior.
- **DB error propagation in doc_move (#833)** — Replaced `.unwrap_or(None)` with proper `?` + `match` pattern. Transient DB errors now propagate correctly instead of being silently swallowed.
- **Recall score reports RRF rank instead of similarity (#831)** — `recall_unified_all` now reports cosine similarity scores instead of rank-derived RRF values when results appear in only one store.
- **MCP/CLI store mismatch — hardcoded `~/.uteke` paths (#830)** — Four call sites hardcoded the old data directory. `uteke-mcp` opened a separate store from the CLI/server. All paths now resolve through `uteke_home()`.
- **`POST /consolidate` rejects string threshold (#826)** — Added `flex_f32` deserializer accepting both numeric and string JSON values for `threshold` parameter. Fixes MCP layers that stringify all parameters.

### Added
- **API route drift guard test (#829)** — Source-backed test extracts handler paths from `handlers.rs` and asserts every handler route is registered in the API registry. 46 handler paths, 57 registered paths, 0 missing.
- **Missing doc endpoints in API reference (#827)** — Registered `/doc/list`, `/doc/search`, `/doc/update` in the docgen registry. Auto-generated `api-reference.md` now covers all document endpoints.

### Changed
- **Open-source governance docs (#834)** — Added `CODE_OF_CONDUCT.md`, rewrote `CONTRIBUTING.md` (solo maintainer context, branch naming convention, test requirements), upgraded PR template (What/Why/How/Testing), replaced `.md` issue templates with YAML forms, added `pr-checks.yml` CI workflow.
- **API reference updated** — 3 new document endpoints documented.

### Contributors
- [@ajianaz](https://github.com/ajianaz) — doc_move fixes, recall score fix, API docs, governance docs
- [@themiguelamador](https://github.com/themiguelamador) — store path resolution via `uteke_home()`, recall similarity reporting (#830, #831)
- [@tomatotomata](https://github.com/tomatotomata) — API route drift guard test (#829)

## [0.10.3] — 2026-08-01

### Fixed
- **ORT init AVX2 early-return skips system fallback (#820)** — `resolve_ort_lib()` no longer returns `Err` when AVX2 library is found but fails to load. Falls through to standard/SSE4.2 system paths with a warning.
- **Misleading "CPU may lack SIMD" error message (#821)** — Error now accurately states "Set ORT_LIB_PATH" or "use the 'legacy' feature" instead of blaming CPU SIMD support.
- **ONNX init retried on every request after failure (#822)** — Embedder init errors are now cached: ONNX failures cached permanently, network backend failures (OpenAI/Ollama) auto-retry after 60s TTL.

### Dependencies
- Bump `thiserror` 2.0.18 → 2.0.19
- Bump `serde_json` 1.0.150 → 1.0.151
- Bump `clap` 4.6.1 → 4.6.4
- Bump `which` 7.0.3 → 8.0.5
- Bump `uuid` 1.23 → 1.24
- Bump `actions/cache` 4 → 6

## [0.10.2] — 2026-07-28

### Fixed
- **SIGILL on CPUs without AVX2 (#709)** — Runtime CPU feature detection now dynamically selects between AVX2 and SSE4.2 ONNX Runtime shared libraries. CI builds a legacy SSE4.2-only ORT sidecar for both Linux and Windows. Use the `*-legacy*` release bundles on older CPUs (e.g., Intel Celeron J4125/N4020).
- **`POST /room/remember` returned 500 for invalid memory types (#789)** — Validation errors now correctly return HTTP 400 instead of 500.
- **Room operations counted deprecated memories (#784)** — `room_stats()`, `recall_room()`, and `get_room_memory_ids()` now filter `deprecated = 0`. Previously caused 76% stat inflation (620 vs 148 active).
- **`POST /room/recall` required query parameter (#785)** — `query` is now `Option<String>` with `#[serde(default)]`. Empty/missing query falls back to chronological recall instead of returning 400.
- **`DELETE /forget` rejected short ID prefixes (#794)** — `list` and `room_recall` display 8-char ID prefixes, but `forget` required full UUIDs. Now resolves short prefixes via SQL `LIKE` with ambiguous-match detection (400 if >1 match).

### Changed
- **ORT loading switched from `download-binaries` to `load-dynamic`** — ORT shared library is now loaded at runtime via `dlopen`/`LoadLibrary` instead of being statically linked. Release bundles now include the ORT `.so`/`.dylib`/`.dll` as sidecar files.

### Added
- **Auto-generated API reference docs (#786)** — New `crates/docgen` binary reads route registry + `schemars::JsonSchema` derives to generate `docs/api-reference.md`. CI `docs-check` job fails if docs are stale. Feature-gated behind `docgen` flag — zero runtime overhead.

### Security
- **Bump quinn-proto 0.11.14 → 0.11.15 (GHSA-4w2j-m93h-cj5j)** — Dependency patch for HTTP/3 stream injection vulnerability.

### Docs
- **Hermes integration docs updated** — Fixed incorrect `room_remember` example (was using `remember` with `room_id` which is not supported by `/remember` endpoint). Added `room_document` action. Added "Valid Memory Types" section. Added "HTTP API Notes" section documenting POST-with-body pattern and known issues (#784, #785, #786).

## [0.10.1] — 2026-07-24

### Changed
- **Default data directory migrated from `~/.uteke` to `~/.codecora/uteke` (#773, #778)** — First launch auto-migrates existing data via atomic `rename` (cross-device fallback: recursive copy). Set `UTEKE_HOME` to override. All docs, CLI help, and config defaults updated.

### Fixed
- **`uteke upgrade` command not found (#777)** — `commands::upgrade` module was private, now `pub(crate)`.
- **`DELETE /doc/delete` ignored query string on CI runners (#777)** — `req.url().query()` returned `None` on some environments; replaced with `req.uri().query()` split pattern.
- **Windows uninstall paths in INSTALL.md (#778)** — All four `~/.uteke` references now point to `~/.codecora/uteke`.
- **Clippy: redundant `&format!` borrow (#778)** — `Error::generic(&format!(…))` → `Error::generic(format!(…))`.
- **Test env var pollution (#778)** — `uteke_home` tests now save/restore `HOME` and `UTEKE_HOME` to prevent parallel test race conditions.

### Contributors
- [@ajianaz](https://github.com/ajianaz) — data dir migration with auto-migrate (#778)
- [@ajianaz](https://github.com/ajianaz) — fix upgrade visibility and doc delete query (#777)

## [0.10.0] — 2026-07-22

### Changed
- **Rust edition 2021 → 2024 (#755)** — Project-wide edition upgrade. Enables `gen` blocks, `let` chains, `unsafe` `extern` blocks, improved `if let`/`while let` chains, and `ref` pattern simplification. Minimum Rust version raised from 1.75 to 1.85.

### Added
- **`POST /room/remember` HTTP endpoint (#762)** — Store a memory and link it to a room in a single API call. Accepts `room_id`, `content`, `tags`, `namespace`, `type`, `metadata`, and `author`. Previously `remember_in_room()` existed in uteke-core but was never exposed via HTTP.

### Fixed
- **`DELETE /forget` returned 200 for non-existent memory IDs (#762)** — Now checks memory existence before calling `forget()` and returns proper 404 when the ID doesn't exist. Previously silently returned success.
- **Clippy lints after edition 2024 upgrade (#755)** — Fixed `repeat("?").take(n)` → `repeat_n("?", n)`, `.map_or(true, |_| true)` → `.is_none_or(|_| true)`, and `let_binding_from_block` in `slug_from_path`.

## [0.9.1] — 2026-07-21

### Fixed
- **API versioning routes return 404** — `ApiVersion::from_path()` stripped `/api/v1/` prefix removing the trailing slash, producing `"recall"` instead of `"/recall"`. All versioned routes (`/api/v1/*`, `/api/v2/*`) returned 404. Fix: strip `/api/v1` without trailing slash, preserving leading `/` in remainder. (#754)

### Contributors
- [@gnoviawan](https://github.com/gnoviawan) — uteke onboard wizard implementation (#743)

## [0.9.0] — 2026-07-20

### Added
- **`uteke onboard` — interactive onboarding wizard (#743)** — guides new users from zero to productive in one command. Detects install, picks AI agent (Hermes/Claude/Cursor/Pi/OpenCode), integration mode (tool vs memory-provider), feature toggles (Aging, Auto-maintenance, Graph rerank, Salience/Recency boost, Server mode), writes `uteke.toml`, runs `uteke init`, feature showcase. Non-interactive: `uteke onboard --yes --agent hermes`.
- **API URL versioning /api/v1 and /api/v2 (#741)** — prefix-based API versioning on uteke-server for backward compatibility.
- **Configurable dream pipeline thresholds (#742)** — dedup threshold, contradiction similarity, tag jaccard, max memories, orphan importance now configurable via config (previously hardcoded).
- **SECURITY.md and PR template (#746)** — security policy with private vulnerability reporting, SLA by severity. GitHub PR template with type checklist and CI verification steps.

### Changed
- **POST /room/document renamed to POST /room/summary-document (#735, #739)** — avoids collision with /room/summary. Old endpoint logs deprecation warning.

### Fixed
- **Deprecated memories appearing in vector search results (#748, #749)** — `recall()` was missing deprecated filter, causing 61.7% deprecated memories to leak into all vector-based search paths (Vector, Hybrid RRF, Graph). FTS5 was already filtered at SQL level.
- **Windows ERROR_LOCK_VIOLATION (OS error 33) (#747)** — usearch index now reads from the locked file handle directly instead of opening a second handle, preventing exclusive lock conflicts on Windows.
- **Embedding model download robustness (#740, #744)** — streaming download (no more buffering 187MB in RAM), retry with 3 attempts, connect/read timeouts, progress indicator, integrity verification.

### Contributors
- [@webhop123](https://github.com/webhop123) — Windows OS error 33 fix (#747)
- [@gnoviawan](https://github.com/gnoviawan) — uteke onboard wizard implementation (#743)

## [0.8.0] — 2026-07-17

### Added
- **`uteke onboard` — interactive onboarding wizard** — guides new users from zero to productive in one command. Detects install, asks which AI agent they use (Hermes/Claude/Cursor/Pi/OpenCode), picks integration mode (tool vs memory-provider), toggles features on/off (Aging, Auto-maintenance, Graph rerank, Salience/Recency boost, Server mode), writes `uteke.toml`, runs `uteke init`, and prints a full feature showcase. Non-interactive mode: `uteke onboard --yes --agent hermes --namespace default`.
- **PUT /memory — partial memory updates (#676)** — Update any combination of content, tags, metadata, importance, pinned state, or memory_type on an existing memory. Content changes trigger embedding regeneration. Replaces the old pattern of forget+remember.
- **POST /memory/pin and POST /memory/importance endpoints (#660)** — Dedicated endpoints for pin/unpin toggle (accepts `pinned` boolean) and importance score setting (0.0–1.0). Both return the updated memory on success.
- **Room ↔ Document junction table — schema v15 (#689, #692)** — New `room_documents` table links rooms to documents bidirectionally. Endpoints: `POST /room/document/list`, `PUT /room/document/add`, `DELETE /room/document/remove`, `POST /doc/room/list`.
- **memory↔document cross-entity linking via [[doc-slug]] wikilinks (#691)** — Memories containing `[[doc-slug]]` patterns are auto-wired to document references. Query endpoints: `POST /memory/doc-refs` (doc slugs for a memory) and `POST /doc/mem-refs` (memory IDs referencing a doc).
- **Schema v14: memory_type added to FTS5 index (#662, #664)** — FTS5 full-text search now indexes `memory_type`, enabling keyword search by type. Migration rebuilds the FTS5 index from existing memories.
- **Trust scoring with feedback API (#718, #725)** — `uteke feedback helpful <id>` (+0.05 importance) and `uteke feedback unhelpful <id>` (-0.10 importance). HTTP endpoint: `POST /memory/feedback` with `{ id, feedback: 'helpful'|'unhelpful' }`. Importance clamped to [0.0, 1.0].
- **Jaccard token similarity as post-RRF reranking signal (#719, #723)** — Token-level Jaccard similarity applied after RRF score normalization in `recall_rrf`. Configurable via `jaccard_weight` in config (default 0.0, opt-in). Module: `jaccard.rs`.
- **Auto-contradiction scan as Dream pipeline phase (#720, #726)** — New Phase 4 (Contradict) in Dream maintenance pipeline. Scans top-200 recently updated memories for pairs with high tag overlap (Jaccard ≥ 0.3) + low embedding cosine similarity (≤ 0.6). Creates `contradicts` graph edges (older → newer). Pipeline order: lint → backlinks → dedup → contradict → orphans → compact → verify.
- **Cross-entity enrichment in recall (#703, #704, #705)** — `--enrich` flag and `enrich` parameter on recall endpoints add cross-entity links to results. Room summaries include `referenced_documents`. `POST /memory/doc-refs` and `POST /doc/mem-refs` for cross-entity queries.
- **Codecora theme adoption (#702)** — VitePress docs now use `@codecora/theme` (Catppuccin Mocha, new base `/uteke/docs/`).
- **Room operations test suite (#701)** — Comprehensive tests for room CRUD and room-document junction operations.
- **Cross-entity integration tests (#706)** — Integration tests covering memory↔document linking, room-document junction, and wikilink resolution.

### Changed
- **Entity/category filter pushed into core recall (#667)** — Entity and category metadata filters now run inside the core recall candidate loop instead of post-fetch amplification. Eliminates the 10x fetch overhead for filtered queries.
- **Full memory detail fields in UnifiedSearchResult (#688)** — Unified search results now include complete memory metadata (tags, importance, pinned, namespace, memory_type, source info) directly in the response, eliminating secondary lookups.
- **Salience/recency boosts enabled by default (#721, #722)** — Default weights changed from 0.0 to 0.1 for both salience and recency. CLI flags now tri-state: `--salience`/`--recency` (explicit), `--no-salience`/`--no-recency` (disable), omit (default 0.1). No longer opt-in.
- **Dream pipeline expanded to 7 phases** — Added Contradict phase between Dedup and Orphans. `all_in_order()` returns 7 phases. CLI `--phases` filter accepts `contradict`.
- **Docs restructured (#716)** — Split getting-started into separate install/comparison/feature pages. Improved navigation and content organization.

### Deprecated
- **hermes-memory-provider extension (#724)** — The Hermes memory provider plugin is deprecated in favor of direct HTTP transport. Use `uteke serve` + HTTP API instead of the plugin wrapper.

### Fixed
- **Search access count tracking (#687)** — Search operations now correctly increment the access count on recalled memories, improving tier scoring accuracy.
- **Windows usearch buffer overflow (#684, #685)** — usearch save/load now serializes via in-memory buffer first, avoiding C++ `fopen` failures on Windows due to MAX_PATH limits and file lock conflicts.
- **Metadata fields propagation (#682, #683)** — Metadata fields set via `POST /remember` are now consistently propagated through all downstream operations (recall, search, export, contradiction detection).
- **Cross-entity linking bugs (#690)** — Fixed edge cases in `[[doc-slug]]` resolution where slugs with special characters or missing documents produced incorrect edges. Fixed O(n) reverse scan in `get_related()`.
- **Room-document validation (#698, #700)** — Room-document junction endpoints now validate that both room_id and doc_slug exist before linking/unlinking.
- **Importance endpoint error dispatch (#697, #699)** — Fixed `POST /memory/importance` to match `Error::Validation` instead of string matching.

### Dependencies
- Bumped `usearch` 2.25.3 → 2.26.0 (#679)
- Bumped `uuid` 1.23.4 → 1.23.5 (#680)
- Bumped `actions/setup-node` 6 → 7 (#678)

## [0.7.3] — 2026-07-13

### Fixed
- **Windows vector index 0 bytes after save (#647)** — usearch's C++ `fopen("wb")` silently fails on Windows due to MAX_PATH limits, file lock conflicts with `fs2` exclusive lock (#543), and Windows Defender interference. Fix: `save()` now serializes to an in-memory buffer via `save_to_buffer()`, then writes to disk using Rust's `std::fs::write()` with atomic temp+rename. Load still uses usearch native `Index::restore()` — the on-disk format is identical. New round-trip test proves compatibility.
- **`recall --json` missing metadata field (#646)** — `UnifiedSearchResult` lacked a `metadata` field, so consumers (Hermes agent, MCP server, benchmarks) had to round-trip lookup by `memory_id` to access metadata stored via `--meta`. Fix: added `metadata` field to `UnifiedSearchResult` and populated it from the recall query.

### Changed
- **README fact-check and optimization (#642, #643)** — Verified all claims against source code and live data. Improved competitive positioning, added infographic, updated benchmarks.

## [0.7.2] — 2026-07-09

### Added
- **Version field in `/health` response (#636)** — `GET /health` now reports the server's crate version (`CARGO_PKG_VERSION`). HTTP clients (e.g. Corin) can gate features against the remote server version instead of guessing from the local CLI. Backward compatible — `version` is an added JSON field.

### Fixed
- **Defensive datetime parsing — tolerate missing timezone in RFC3339 fields (#635)** — A single corrupted row with timezone-less `updated_at` (ISO 8601 but not RFC3339) crashed `load_all()`, making the entire memory database inaccessible. Fix: new `parse_datetime_flexible()` falls back to assuming UTC (`+00:00`) when strict RFC3339 parse fails; new idempotent `repair_datetime_timezones()` scans `memories` + `documents` on every DB open and repairs bad rows in-place.
- **`POST /doc/list` default limit 5 → 1000 (#634)** — Document listing reused the memory pagination default (`5`), silently truncating client-side document trees. Documents are not paginated like memories — added dedicated `default_doc_limit() = 1000`. Memory and room-recall defaults unchanged.

## [0.7.1] — 2026-07-09

### Fixed
- **Vector index silently desyncs from SQLite (#621)** — Memories exist in SQLite but have no vector embedding, invisible to `uteke recall`. Root cause: `index.save()` failure in `remember_precomputed` and `forget` silently returned `Ok(())`. Fix: explicit error propagation + `uteke verify` / `uteke repair` commands.
- **`uteke remember` ignores stdin pipe content (#620)** — Piping content via `cat file | uteke remember -` stored literal `"-"` instead of reading stdin. Fix: added stdin detection when content argument is `"-"`, with `Box::leak` for lifetime extension.
- **`uteke room recall` default limit 20 silently truncates (#623)** — Rooms with >20 memories had results silently cut. Fix: increased default limit to 100.
- **Author metadata not exposed in room recall JSON (#624)** — `room_memories.author` was not selected in the recall SQL query. Fix: added `rm.author` to SELECT with fallback to `"unknown"`.

### Changed
- **Documentation audit (#618)** — Added 9 missing HTTP endpoints to docs, fixed CHANGELOG link, updated sidebar anchors, corrected `doc list` namespace description.

### Dependencies
- `clap_complete` 4.6.6 → 4.6.7

## [0.7.0] — 2026-07-08

### Added
- **Project-aware memory tagging for noise-free recall (#616)** — Tag-based project scoping using `project:<name>` convention. SKILL.md includes mandatory project-aware memory section. pi-memory-provider extension auto-detects project from CWD and auto-tags recall. Hermes hooks detect project from `/repos/<project>/` paths. No binary changes — works with existing `--tags` flag.
- **OpenCode init support (#612)** — `uteke init --agent opencode` generates AGENTS.md with uteke instructions. Bundled SKILL.md updated to v0.6.7.
- **Maintenance HTTP endpoints (#607)** — `POST /prune` (TTL-based deprecated memory cleanup), `POST /consolidate` (near-duplicate merging), `POST /aging` (memory lifecycle: status/preview/cleanup). Write token required.
- **Monitoring HTTP endpoints (#608)** — `POST /importance` (recalculate importance scores), `POST /orphans` (find disconnected low-importance memories, read-only), `POST /rebuild-backlinks` (rebuild referenced_by edges). Orphans accepts read-only token.
- **Extract/Import/Export HTTP endpoints (#604–#606)** — `POST /extract` (LLM fact extraction + auto-store, 1MB limit), `POST /import` (JSONL import with re-embedding, 5MB limit), `GET /export` (JSONL export with optional namespace filter). Extractor moved from uteke-cli to uteke-core (shared module).
- **Document partial update CLI + HTTP (#589, #583)** — `uteke doc update <slug>` for partial document updates (title, content, tags, metadata) with automatic chunk rebuild. `POST /doc/update` endpoint.
- **MCP: pin/unpin tools (#588)** — `uteke_pin` and `uteke_unpin` MCP tools for memory persistence control.
- **MCP: 6 room tools (#586)** — `uteke_room_create`, `uteke_room_delete`, `uteke_room_stats`, `uteke_room_summary`, `uteke_room_document`, `uteke_room_memories` MCP tools for full room management.
- **MCP: tag management tools (#566)** — `uteke_tags_list`, `uteke_tags_rename`, `uteke_tags_delete` MCP tools.
- **MCP: document update + move tools (#589, #438)** — `uteke_doc_update` (partial document update with chunk rebuild), `uteke_doc_move` (move document to new parent).
- **`uteke upgrade` command (#603)** — Renamed from `uteke update` (which conflicted with `uteke doc update`). Self-update mechanism for installing latest Uteke release.

### Changed
- **Documents are now global — no namespace isolation (#614, #615)** — Documents use unique slugs across all namespaces. Schema migration v12→v13 adds `author` column, deprecates namespace on documents, migrates duplicate slugs. All document CRUD (CLI, server, MCP) no longer accepts namespace parameter.
- **Extractor moved to uteke-core** — `Extractor` struct moved from `uteke-cli` to `uteke-core` shared module. CLI extract command delegates to core. Net -213 lines.
- **Schema version v13** — Migration v12→v13: documents namespace deprecated, `author` column added, duplicate slug cleanup, global unique slug index.

### Fixed
- **Security: fail-hard on checksum verification failure (#609)** — `uteke upgrade` now fails with error when checksums download fails or archive checksum is missing, preventing MITM tampering. Found by Cora code review (GLM-5.2).
- **Room tables in SCHEMA constant (#596)** — Fresh databases now include room tables in the base SCHEMA, preventing issues when schema_version check doesn't run migrations.
- **CI: graceful sync workflow (#598)** — Fixed sync workflow + added missing cargo audit ignores for known advisories.
- **Deps: crossbeam-epoch 0.9.18 → 0.9.20 (#597)** — Security update (RUSTSEC-2026-0204).

## [0.6.7] — 2026-07-06

### Added
- **Automatic memory-provider for pi, claude, cursor (#575, #577)** — `uteke init --agent <agent> --memory-provider` now mirrors the Hermes auto-recall experience for all agents. Pi gets a TypeScript extension hooking `before_agent_start` with slash commands. Claude and Cursor get enhanced rules with auto-recall instructions and MCP server config snippets.
- **GET /room/memories endpoint + uteke_room_memories MCP tool (#569)** — Chronological room memory listing with optional author filter.

### Fixed
- **uteke-mcp JSON-RPC 2.0 spec compliance (#573, #576)** — Success responses no longer include `"error": null` (tagged enum replaces flat struct with Option fields). Notifications (requests with no `id`) no longer receive a response. Fixes Claude Code `✗ Failed to connect`.

### Changed
- **Slim Docker image** — Removed bundled embedding model (~208MB) from Docker build. Image size reduced from ~218MB to ~10MB. Model downloads lazily on first container start and persists in the named volume.

## [0.6.6] — 2026-07-05

### Added
- **Tags, pin, timeline, and edges HTTP endpoints (#566)** — 7 new REST endpoints: `GET /tags`, `POST /tags/rename`, `POST /tags/delete`, `POST /pin`, `POST /unpin`, `GET /timeline`, `GET /edges`. Enables full tag management, memory pinning, audit timeline, and graph edge queries via HTTP.

### Fixed
- **Room summary panic on multi-byte Unicode (#565)** — `room_summary()` panicked on Unicode chars (≤, ≥, etc.) because of byte-index slicing. Replaced with char-based truncation (`chars().take(N).collect()`).

## [0.6.5] — 2026-07-05

### Added
- **HTTP graph mutation endpoints (#542)** — `POST /graph/edge` creates a new typed edge between two memories; `DELETE /graph/edge` removes an edge by its ID. Enables programmatic graph editing without the CLI.

### Fixed
- **Cross-process file lock (#543)** — usearch index is now protected by a file lock to prevent race conditions when multiple uteke processes access the same database concurrently.
- **FTS5 initialization on every startup (#544)** — FTS5 virtual table and triggers are now re-created on every startup, not just during schema migration. Fixes missing FTS5 after partial upgrades.
- **Room list `--namespace` filter returns empty array (#545)** — namespace filtering now correctly matches room namespaces instead of returning no results.
- **Room recall `--query` returns empty results (#546)** — semantic search within rooms now returns correct results; the query embedding was being silently discarded.
- **Room document missing sections for note/insight/reference/event types (#547)** — document sections for non-core memory types (note, insight, reference, event) are now generated correctly in room documents.
- **documents_fts migration repair (#549)** — FTS5 virtual table is rebuilt if missing or corrupted during migration, preventing `no such table: documents_fts` errors.
- **Document delete by slug (#550)** — `uteke doc delete` now correctly resolves documents by slug (not just UUID), matching the behavior of `get` and `list`.

### Docs
- **HTTP API documentation for `/recent` and graph mutation endpoints** — `GET /recent` (with query params), `GET /graph`, `POST /graph/edge`, `DELETE /graph/edge` added to HTTP Endpoints table. Graph API section expanded with mutation curl examples.
- **VitePress sidebar entries** — added Document Commands and Graph API links to docs sidebar.

## [0.6.4] — 2026-07-03

### Added
- **Gate ONNX behind `onnx` feature (#533, #534)** — `ort` and transitive `numkong` are now optional, gated behind `default-features = false`. Consumers that don't need ONNX/embedding can opt out entirely, resolving CI build failures on Ubuntu 22.04 (AVX-512) and macOS Intel (no prebuilt).
- **Unified search across memories and documents (#531)** — single search endpoint returns results from both memories and documents.
- **`/namespaces?with_counts=true` + `/recent` endpoints (#527, #528)** — namespace listing with per-namespace memory counts; recent memories across all namespaces.
- **Cross-namespace `/list` and `/list_at_time` (#526)** — calling `/list` without namespace returns results from all namespaces.

### Changed
- **Split uteke-server monolith into modules (#514)** — refactored single-file server into modular structure for maintainability.

### Fixed
- **Read-only token on POST read endpoints (#524)** — read-only tokens now work on POST-based read endpoints (search, list, graph, etc).
- **SQLite count column type** — use `u32` instead of `usize` (no `FromSql` impl for `usize`).
- **`Option<String>` to `Option<&str>`** — fixed `list()` call parameter type.
- **CodeCora alerts** — min_score in document path + improved error logging.

## [0.6.3] — 2026-07-01

### Fixed
- **Docker ARM64 libmvec.so.1 missing** — ARM64 binaries built on `ubuntu-24.04-arm` (glibc 2.39) require `libmvec.so.1` (ARM SVE math library), which doesn't exist in Debian Bookworm (glibc 2.36). Upgraded Dockerfile base image to `debian:trixie-slim` (glibc 2.41) and added `libstdc++6` runtime dependency. Also removes incorrect `libgomp1` from v0.6.2.

## [0.6.2] — 2026-07-01

### Fixed
- **Docker image missing libgomp1** — `uteke-serve` failed with `libmvec.so.1` on Debian runtime. Added `libgomp1` to Dockerfile runtime dependencies. (Incorrect fix — see v0.6.3)

### Changed
- **Docker Hub multi-push** — Release workflow now publishes to Docker Hub (`codecoradev/uteke`) in addition to GHCR, conditional on `DOCKERHUB_USERNAME` / `DOCKERHUB_TOKEN` org secrets. Falls back to GHCR-only when secrets are absent.

## [0.6.1] — 2026-06-30

### Fixed
- **#500: Schema migration v11→v12 missing has_children column** — Partially-migrated databases (schema_version=12 but missing `has_children`) now get the column repaired on next open. `uteke repair` also fixes schema inconsistencies.

### Changed
- **#501: install.sh now installs uteke-mcp** — All three binaries (uteke, uteke-serve, uteke-mcp) are now installed via the quick-install script.

### Docs
- Updated version strings to 0.6.0 in cli-reference.md, roadmap.md, AGENT.md.

## [0.6.0] — 2026-06-30

### Added
- **Batch import (`--batch-dir`)** — import all `.md`/`.markdown`/`.txt`/`.jsonl` files from a
  directory in one command. Two strategies: **Document** (`.md` → auto-chunk →
  embed, no LLM) and **MemoryExtract** (`.txt`/`.jsonl` → LLM fact extraction).
  Use `--as-doc` or `--as-memory` to override auto-detection (mutually exclusive).
  `--recursive` for nested directories, `--dry-run` to preview, `--max-size` to
  skip large files. Path is canonicalized before traversal to prevent symlink
  escapes (#492 review).
- **Embed fallback** — optional cloud embedding API fallback when local ONNX fails.
  Configured via `[embed_fallback]` in `uteke.toml` or env vars
  `UTEKE_EMBED_FALLBACK_*`. Validates dimension compatibility at startup.
  Requires all three fields (`api_key`, `base_url`, `model`) — partial config
  produces a warning instead of a runtime crash.
- **Mode C documentation** — `docs/integrations/hermes.md` now documents all three
  Hermes integration modes (A: uteke-tool, B: memory-provider, C: shell hook).
  Getting-started callout updated to reference all modes.
- **Migration upgrade regression test** — simulates v0.4.x DB (schema_version=11)
  and verifies migration v11→v12 completes with all hierarchy columns and indexes.

### Changed
- `ensure_embedder()` now wraps `OnnxEmbedder` in `FallbackEmbedder` when
  fallback settings are present and dims match.

### Fixed
- **#492: Schema migration crash on upgrade from v0.4.x** — duplicate document
  hierarchy indexes (`idx_documents_path/parent/depth/sort`) in the `SCHEMA`
  constant caused `execute_batch()` to fail before versioned migrations ran.
  Columns didn't exist yet in existing DBs. Moved indexes to
  `migrate_v11_to_v12()` where they belong.
- **Batch import path canonicalization** — `--batch-dir` now resolves symlinks
  and `..` components before traversal.
- **Batch import silent errors** — `read_dir` failures on entries now logged
  instead of silently swallowed.
- **Fallback embedder partial config** — `is_configured()` now requires all
  three fields (`api_key`, `base_url`, `model`). Partial config produces a
  clear warning instead of a confusing runtime crash on first embed call.

## [0.5.0] — 2026-06-27

### Added
- **Hermes memory-provider plugin** — `uteke init --agent hermes --memory-provider`
  installs a `MemoryProvider` plugin to `~/.hermes/plugins/uteke/` that makes
  uteke Hermes's default long-term memory: automatic recall injected into the
  prompt each turn, plus opt-in LLM fact extraction on session end / pre-compress.
  Talks to the `uteke` binary directly (no `uteke-serve` daemon) and has a
  circuit breaker so a bad endpoint never blocks the agent. Templates live in
  `extensions/hermes-memory-provider/` and are embedded via `include_str!`.
  Docs: `docs/integrations/hermes.md` (Mode B). Complements the existing
  `uteke-tool` plugin (Mode A) rather than replacing it.
- **#46: LLM-backed fact extraction on import** — `uteke import --extract`
  distills noisy source text (chat transcripts, long notes, exported dumps)
  into atomic facts via an OpenAI-compatible chat-completions endpoint, storing
  one memory per fact instead of importing raw text verbatim. Opt-in and
  offline-first: without `--extract` the importer makes no network calls and
  behaves exactly as before. Configurable via the `[extraction]` config section
  or `UTEKE_EXTRACTION_*` env vars (`MODEL`, `API_KEY`, `BASE_URL`,
  `ENDPOINT_PATH`, `MAX_FACTS`), plus per-run flags `--extract-model`,
  `--extract-api-key`, `--extract-base-url`, and `--extract-max-facts`. The
  extraction API key falls back to the embedding/`OPENAI_API_KEY` credential so
  an existing OpenAI-compatible setup needs no duplication.
- **#473: Configurable embedding endpoint path** — `endpoint_path` in `[embedding]`
  config or `UTEKE_EMBEDDING_ENDPOINT_PATH` env var. Auto-normalizes leading
  slash for non-standard API paths (e.g. Azure, custom proxies).
- **#472: Default max_seq_length increased to 2048** — ONNX embedding backend
  now defaults to 2048 tokens (was 256). Configurable via `max_seq_length` in
  config or `UTEKE_MAX_SEQ_LENGTH` env var.
- **#466: Public `store()` accessor** — `Uteke::store()` exposes the internal
  `Store` handle for downstream crates that need direct room/tag operations.

### Changed
- **rusqlite 0.31 → 0.40** — upgraded to match CorIn's dependency. All
  `COUNT(*)` results and `LIMIT`/`OFFSET` bindings migrated from `usize` to `i64`
  (rusqlite ≥0.32 breaking change).

## [0.4.3] — 2026-06-22

### Fixed
- **#463: Binary version mismatch** — Cargo.toml on main was 0.3.2
  despite develop being 0.4.2. Merge conflicts resolved to wrong side.
  Now all crates report correct version.
- **#458: Release workflow** — filename order, double v, CHANGELOG dupes.
- **#467: Bump actions/checkout v6 to v7.**

## [0.4.2] — 2026-06-22

### Fixed
- **#458: Release workflow** — filename mismatch, double v in UTEKE_VERSION,
  duplicate CHANGELOG entries. Binary download table now matches actual
  filenames. Quick start command no longer produces \.

## [0.4.1] — 2026-06-22

### Added
- **uteke_context MCP tool** — smart project summary for agent prompts
- **uteke_dream MCP tool** — trigger dream cycle from any agent
- **POST /context** and **POST /dream** API endpoints
- **Auto-dream** — server background thread runs dream every 3 days
- **Configurable maintenance daemon** — [maintenance] config section
- **Safe aging defaults** — pinned protection, max 100/cycle, configurable thresholds
- **Dedup on insert** — cosine >= 0.95 returns existing memory ID

### Fixed
- **#442: server deadlock** — RwLock deadlock in remember_precomputed
- **#448: recall without namespace** — now searches ALL namespaces (was: only "default")
- **Pinned memories could be deleted** by aging cleanup — added pinned=0 guard
- **Schema migration v7→v11** — indexes now created after migrations complete

## [0.4.0] — 2026-06-22

### Added

- **Hierarchical documents — depth-10 tree engine** (#438 #440)
  - Schema v12: `parent_id`, `path` (materialized), `depth`, `sort_order`,
    `has_children` columns on documents table
  - Tree operations: create with parent, children, descendants, move,
    breadcrumbs
  - `MAX_DEPTH = 10` enforced in `move_document()`
  - `uteke doc create --parent <slug>`, `uteke doc children`, `uteke doc
    descendants`, `uteke doc move`, `uteke doc breadcrumbs`
  - `uteke doc list --tree` for indented tree view

- **Hybrid document search** (#440)
  - `uteke doc search <query>` with 3 modes:
    - `semantic`: usearch chunk embeddings with `chunk:` prefix
    - `fts`: FTS5 keyword search on title and slug
    - `hybrid` (default): Reciprocal Rank Fusion of semantic + FTS (k=60)
  - FTS5 virtual table `documents_fts(title, slug, content)` with
    INSERT/UPDATE/DELETE sync triggers
  - Chunk embeddings auto-inserted into usearch on document upsert
  - Old chunks cleaned from usearch on update and delete

### Fixed

- **move_document descendant path predicate** used UUID instead of full
  materialized path. Descendants of nested documents were not updated after
  move operations.
- **delete_document cascade** hardcoded `/{uuid}/%` prefix that never
  matched actual materialized paths. Now fetches document path first.
- **doc_delete namespace** hardcoded `DEFAULT_NAMESPACE`. Now accepts
  `namespace: Option<&str>` parameter.
- **move_document** now recomputes `has_children` on old parent after
  moving last child away.


## [0.3.2] — 2026-06-22

### Fixed

- **Harden schema migrations for partially-migrated databases** (#435)
  - `migrate_v9_to_v10` now uses `column_exists()` guard before `ALTER TABLE
    ADD COLUMN` for `source` and `source_type`. Prevents "duplicate column
    name" crash on databases that were partially migrated by a buggy binary
    (e.g. v0.3.0 which ran SCHEMA_INDEXES before migrations).
  - `migrate_v7_to_v8` now creates `idx_memories_slug` index as a separate
    statement instead of inside `execute_batch`, producing clearer error
    messages on failure.

### Docs

- Added detailed documentation checklist to AGENT.md release process (#434).

## [0.3.1] — 2026-06-21

### Fixed

- **Critical: schema migration failure for existing databases**
  - DBs created with v0.2.x or earlier (schema v7) failed to open after
    upgrading to v0.3.0. `CREATE INDEX ... ON memories(slug)` in the SCHEMA
    constant was executed BEFORE migrations added the `slug` column.
  - Fix: Split schema init into two phases. `SCHEMA` contains only
    `CREATE TABLE` + safe indexes. `SCHEMA_INDEXES` (indexes on
    migration-added columns) runs AFTER `ensure_schema_version()` completes.
    Index creation is best-effort (errors logged, not fatal).
  - The problematic indexes (`idx_memories_namespace`, `idx_memories_deprecated`,
    `idx_memories_slug`) are now created post-migration.

## [0.3.0] — 2026-06-21

### Added

- **Document engine — wiki/knowledge base** (#406)
  - Schema v11: `documents` + `document_chunks` tables
  - Full markdown content with slug, title, tags, version
  - Auto-chunking via markdown chunker (#405) + per-chunk embeddings
  - Foundation for Obsidian/Outline-style wiki

- **Document CLI commands** (#411)
  - `uteke doc create/get/list/delete/export`
  - Accept content from --file, --content, or stdin
  - Auto-derive title from first heading

- **Markdown/prose chunker** (#405)
  - Split by headings (levels 1-6), respect code block fences
  - Paragraph-level fallback for oversized sections
  - `TextChunk { heading, content, level, char_start, char_end }`

- **Embed-aware chunking** (#407)
  - `chunk_markdown_embed_aware()` derives chunk size from `embedder.max_seq_len()`
  - ~4 chars per token heuristic

- **Cosine-based auto-linking + dedup** (#401)
  - `auto_link_cosine()` runs after every `remember()`
  - `similar_to` edge when cosine >= 0.80
  - `possible_duplicate` edge when cosine >= 0.92
  - Namespace-scoped (no cross-namespace links)

- **Configurable limits** (#404)
  - `LimitsConfig` struct with env var overrides
  - MAX_CONTENT_LENGTH: 10K → 100K
  - `[limits]` section in uteke.toml

- **`/graph` API endpoint** (#408)
  - `GET /graph` returns all nodes + edges + stats as JSON
  - For visualization clients

- **View-only API key** (#409)
  - `--read-only-token` for GET-only access
  - Dual-role: Admin (full) + ReadOnly (GET only)
  - Env vars: `UTEKE_AUTH_TOKEN`, `UTEKE_READ_ONLY_TOKEN`

- **Hermes plugin room_remember** (#410)
  - New `room_remember` action in plugin template

### Changed

- Schema version bumped from v10 to v11
- Internal dependency versions widened from `0.2.0` to `0.3`
- `serialize_embedding` visibility changed to `pub(crate)`

## [0.2.1] — 2026-06-21

### Added

- **Hermes plugin auto-install** (#385)
  - `uteke init --agent hermes` now installs directly to
    `~/.hermes/plugins/uteke-tool/` instead of generating to CWD.
  - Plugin uses Python stdlib only (no `requests` dependency needed).
  - MCP server discovery: README documents `hermes mcp add uteke --command
    uteke-mcp` as an alternative integration path.

- **Room operations in Hermes plugin** (#395)
  - Plugin now exposes `room_create`, `room_recall`, `room_list`,
    `room_summary`, `room_stats`, `room_delete` actions.
  - Added server endpoints: `POST /room/create`, `GET /room/list`,
    `POST /room/stats`, `DELETE /room/delete`.

- **`uteke room create` command** (#393)
  - Explicit room creation: `uteke room create <id> [--title "Name"]`

### Fixed

- **Room list cross-namespace visibility** (#392)
  - `uteke room list` now shows ALL rooms by default, not just those in
    the current `--namespace`. Rooms are collaboration spaces spanning
    namespaces.

- **Schema mismatch error message** (#394)
  - Error now includes binary name and version: "please upgrade uteke
    (current binary: uteke-core v0.2.0, schema v10)".

- **Hermes plugin missing `__init__.py`** (#402)
  - `uteke init --agent hermes` now generates `__init__.py` in the plugin
    directory. Without it, Hermes logs a warning and the plugin never
    loads.

- **Contradictory server detection log** (#403)
  - When server was detected but command unsupported via HTTP (aging,
    doctor, etc.), the fallback path logged "No server detected" —
    contradicting the earlier detection message. Now logs accurately
    based on actual server state.

- **Misleading `db_size_bytes` in stats** (#403)
  - Stats output now labels database size as "(global, shared)" to
    clarify it reflects the entire shared SQLite file, not just the
    queried namespace.

### Added (previous)

- **MCP Streamable HTTP transport** (#381)
  - `uteke-mcp` protocol version bumped from `2024-11-05` to `2025-06-18`
    (current MCP spec).
  - New `POST /mcp` endpoint on `uteke-server` exposing the full MCP
    JSON-RPC API over HTTP. Returns `Content-Type: application/json` with
    `MCP-Protocol-Version: 2025-06-18` header.
  - Shared handler extracted into `uteke_mcp` library crate — used by both
    the stdio binary (`uteke-mcp`) and the HTTP endpoint (`uteke-serve`).
  - Enables remote MCP clients (Claude Desktop, web agents) to connect
    without spawning a subprocess.

- **Citation & source attribution** (#348)
  - Schema v10 migration: adds `source` and `source_type` columns to
    `memories` table. Existing rows get `source_type = 'unknown'`.
  - Source types: `user`, `url`, `file`, `import`, `derived`, `system`,
    `unknown`.
  - `Memory` struct gains `source: Option<String>` and `source_type: String`
    fields. Defaults: `source=None`, `source_type="user"`.
  - `Uteke::set_source(id, source, source_type)` for post-insert provenance
    updates.
  - `ExportEntry` gains optional `source` field for round-trip preservation.
  - CLI: `--source <URL/path>` and `--source-type <type>` flags on
    `uteke remember`.
  - Import sets `source_type = 'import'` with `source = 'import:<filename>'`.

- **Dream cycle** (#353)
  - New `crates/uteke-core/src/dream.rs` module: coordinated maintenance
    pipeline that runs all 6 phases in dependency order, all local, zero
    LLM. Inspired by GBrain's overnight dream cycle.
  - Phases:
    1. **Lint** — type validation + broken-ref count
    2. **Backlinks** — rebuild `referenced_by` edges (#350)
    3. **Dedup** — find & merge near-duplicates (existing `consolidate`)
    4. **Orphans** — detect disconnected memories (#351-compatible inline
       SQL when #351 is not yet merged)
    5. **Compact** — aging cleanup + prune cold memories (existing)
    6. **Verify** — schema + index integrity check (existing `verify`)
  - `DreamPhase` enum, `DreamReport`, `PhaseResult`, `PhaseStatus` types.
  - `Uteke::dream(namespace, dry_run, phases)` orchestrator.
  - CLI: new `uteke dream [--phases] [--skip] [--dry-run] [--quiet]`
    command. Exits non-zero on errors (cron-friendly).

- **Orphan detection** (#351)
  - New `crates/uteke-core/src/orphans.rs` module: detect memories with no
    graph edges, no recall access, not pinned, and below an importance
    threshold.
  - Detection is a single SQL pass (LEFT JOIN on `memory_edges` twice) —
    no O(n²) scan.
  - `OrphanMemory` struct with `orphan_score` (0.0..=1.0):
    `(1 - edge_density) × 0.4 + (1 - access_freq) × 0.3 + (1 - importance) × 0.3`.
  - `Uteke::find_orphans(namespace, threshold, limit)` with namespace
    scoping and `DEFAULT_ORPHAN_THRESHOLD = 0.3`.
  - CLI: new `uteke orphans [--threshold 0.3] [--limit 50]` command.
  - Leverages #350 backlinks: any memory referenced by another is
    automatically excluded (it has an incoming `referenced_by` edge).

- **Timeline event tracking** (#347)
  - New `crates/uteke-core/src/timeline.rs` module: append-only audit log
    per memory in the `timeline_events` table (schema v9).
  - Event types: `created`, `updated`, `recalled`, `consolidated`, `tagged`,
    `forgot`. Each event has optional JSON `event_data`.
  - Store methods: `add_timeline_event`, `list_timeline_events`,
    `count_timeline_events`.
  - Uteke methods: `timeline(memory_id, limit)`,
    `count_timeline_events(memory_id)`, `try_timeline_event()` (best-effort
    append that never fails the primary operation).
  - Auto-emission: `remember_precomputed` now emits a `created` event.
  - CLI: new `uteke timeline <id> [--limit N]` command (default 20 events).
  - Schema migration v8 → v9 (idempotent, no data backfill — timeline
    tracking starts from this version forward).

- **Memory type formalization** (#349)
  - `MemoryType` enum expanded from 5 to 9 variants: original (Fact,
    Procedure, Preference, Decision, Context) + new (Note, Insight,
    Reference, Event).
  - Pattern-based auto-inference (`MemoryType::infer_from_content`):
    URL prefix → `Reference`; decided/chose/will use → `Decision`;
    realized/learned/discovered → `Insight`; how-to/numbered list →
    `Procedure`; always/never/prefer/hate → `Preference`; ISO date +
    time word → `Event`; fallback → `Note`. Zero LLM.
  - When callers pass the default `"fact"`, `remember_typed` now runs
    inference and overrides with a more specific type when one is
    detected (falls back to `Fact` for ambiguous content, preserving
    backward compatibility).
  - `MemoryType::recall_boost()` — small additive score boost per type
    (Decision/Preference +0.05, Insight +0.03, Event +0.02, Note -0.02).
    To be wired into recall scoring by #352.
  - CLI: `--type` help text documents the new types and auto-inference.

- **Salience + recency dual-axis recall ranking** (#352)
  - New `crates/uteke-core/src/salience_recency.rs` module with two
    orthogonal, additive boost functions:
    - `salience_score(memory)` — 0..=1 blend of `access_count`, `importance`,
      and `pinned` (importance × 0.5 + access_freq × 0.3 + pinned 0.2).
    - `recency_score(memory, now)` — per-type exponential decay
      (`exp(-age_days / τ)`). Time constants: Decision/Preference 365d,
      Fact/Reference 180d, Insight 240d, Event 30d, default 90d.
      (τ is the age at which recency drops to ~0.37 = 1/e.)
  - `SalienceRecencyConfig { salience_weight, recency_weight }` defaults
    to zero (opt-in per query). `sanitized()` clamps weights to [0, 1].
  - `Uteke::set_salience_recency_config()` for per-query override.
  - Boosts applied AFTER recall cache lookup (cache stays time-independent).
  - CLI: `--salience` / `--recency` flags on `recall`. Default weights
    (0.15 each) configurable via `[recall]` in `uteke.toml`.
  - Public exports: `salience_score`, `recency_score`, `type_half_life_days`,
    `apply_boosts`, `SalienceRecencyConfig`.

- **Backlink auto-generation** (#350)
  - Bidirectional links: whenever memory A creates a forward edge to B
    (`references`, `tagged_as`, `supersedes`, `replies_to`), an inverse
    `referenced_by` edge from B → A is automatically inserted. Makes the
    graph navigable in both directions without an O(n) scan.
  - New `Store::ensure_backlink()` (idempotent),
    `Store::add_memory_edge_with_backlink()`, and
    `Store::rebuild_backlinks()` (scan + repair pass).
  - `add_memory_edges_batch()` (used by `wire_edges` on every `remember`)
    now also ensures the backlink for each forward edge.
  - New `Uteke::link_memories()` public API for explicit edges with
    automatic backlink.
  - New `Uteke::rebuild_backlinks()` for one-time migration of pre-#350
    stores.
  - New CLI command `uteke rebuild-backlinks [--quiet]` rebuilds all
    `referenced_by` edges from existing forward edges.
  - `uteke edges <id>` gains `--direction <incoming|outgoing|both>`
    (default `both`); `incoming` shows backlinks.
  - New public exports: `backlink_type_for`, `EdgeList`, `MemoryEdge`,
    `EDGE_REFERENCES`, `EDGE_REFERENCED_BY`, `EDGE_REPLIES_TO`,
    `EDGE_SUPERSEDES`, `EDGE_TAGGED_AS`.

- **Graph-augmented RAG reranking** (#378)
  - New recall strategy `graph`: runs the hybrid (RRF) pipeline, then fuses
    graph signals from the `memory_edges` table into each result's score.
    Well-connected memories drift upward; isolated memories are untouched.
  - New `crates/uteke-core/src/graph_rerank.rs` module:
    - `compute_graph_signals()` — single batched SQL query over
      `memory_edges` computes per-memory `edge_count`, `neighbor_count`,
      `edge_type_diversity`, `incoming_count`, `outgoing_count`.
    - `rerank_with_graph()` — additive, log-scaled density + authority
      boosts (`ln(1+x) * weight`), capped at 1.0. Disabled or empty-signal
      inputs are a no-op (cold-start safe).
    - `GraphRerankConfig { density_weight, authority_weight, enabled }`
      with `sanitized()` clamping.
  - `RecallStrategy::Graph` variant added (`memory/types.rs`); wired into
    `recall_hybrid` so the boost runs *before* caching (cache is
    strategy-keyed → `graph` has its own entries, no collision with
    `hybrid`/`vector`/`fts5`).
  - New `Uteke` field + `open_with_embedding_and_graph` constructor so the
    CLI passes the merged `[recall]` weights.
  - CLI: new `--strategy <vector|fts5|hybrid|graph>` flag on `recall`
    (defaults to `[recall].default_strategy`, itself defaulting to
    `vector` — preserves original behavior). Unknown strategies warn and
    fall back to `vector`.
  - Config: `[recall]` gains `default_strategy`, `graph_density_weight`,
    `graph_authority_weight`, `graph_rerank_enabled`, plus env overrides
    `UTEKE_RECALL_STRATEGY`, `UTEKE_GRAPH_DENSITY_WEIGHT`,
    `UTEKE_GRAPH_AUTHORITY_WEIGHT`, `UTEKE_GRAPH_RERANK_ENABLED`.
  - 10 new unit tests covering signal counting, hub boost, log-saturation,
    score capping, cold-start no-op, disabled-flag no-op, and a <10ms
    latency guard over 5000 edges.
  - Backward compatible: existing strategies are unchanged; `graph` is opt-in.

### Changed

- **Bump sha2 0.10 → 0.11** (supersedes Dependabot PR #364)
  - sha2 0.11 dropped the `LowerHex` impl on the digest output, breaking
    `format!("{:x}", hasher.finalize())` in `engine.rs`.
  - Fix: iterate the digest bytes and format each as `{:02x}`.
  - Unifies the crate on a single sha2 version (uteke-server was already
    on 0.11).

### Added

- **OpenAI + Ollama embedding backends** (#337)
  - New `OpenAiEmbedder` (`crates/uteke-core/src/embed/openai.rs`) — HTTP
    call to `{base_url}/embeddings`, default model `text-embedding-3-small`
    (1536d). Azure OpenAI compatible via `base_url`.
  - New `OllamaEmbedder` (`crates/uteke-core/src/embed/ollama.rs`) — HTTP
    call to `{base_url}/api/embed`, default model `nomic-embed-text` (768d).
  - `[embedding]` config section extended with `api_key`, `base_url`, `dims`.
  - New env vars: `UTEKE_EMBEDDING_BACKEND`, `UTEKE_EMBEDDING_MODEL`,
    `UTEKE_EMBEDDING_API_KEY` (fallback: `OPENAI_API_KEY`),
    `UTEKE_EMBEDDING_BASE_URL`, `UTEKE_EMBEDDING_DIMS`.
  - Dim mismatch detection: opening an existing store with a backend that
    produces a different dims now returns a clear error pointing the user
    at `uteke repair` instead of silently corrupting the index.
  - `reqwest` `json` feature added (always included — no feature flag).
  - 16 new unit tests (backend construction, endpoint normalization, default
    constants, response parsing, config merge + env var precedence).
  - ONNX remains the default — fully backward compatible.

- **Auto-wired memory edges** (#346)
  - New `memory_edges` SQLite table (schema v8) for typed edges between
    memories.
  - Optional `slug` column on memories for `[[slug]]` Wikilink-style
    references.
  - Pattern-based entity extraction on every `remember()` call — zero LLM,
    pure string parsing:
    - `[[slug]]` → `references` edge
    - `@tag` → `tagged_as` edge (most recent memory with that tag)
    - `^<uuid>` → `supersedes` edge
    - `><uuid>` → `replies_to` edge
    - `rel:<type>:<uuid>` (legacy `--meta` form) → `<type>` edge
  - New `uteke edges <id> [--deep N]` CLI subcommand: lists direct edges or
    runs BFS across the edge table.
  - Rewrote `get_related()` to prefer the edge table (indexed SQL) over the
    old O(n) JSON metadata scan. Legacy path retained as fallback.
  - Migration v7→v8 backfills existing `metadata.relationships` JSON entries
    into `memory_edges` rows.
  - 20 new unit tests (extraction patterns, edge roundtrip, BFS cycle safety,
    slug/tag resolution).

### Changed

- Schema version v7 → v8.
- `Memory` struct gains optional `slug: Option<String>` field.
- Clippy: cleaned up pre-existing `else { if .. }` collapse warnings in
  `commands/graph.rs` (3 sites) so `cargo clippy --workspace -- -D warnings`
  now passes cleanly.

## [0.2.0] — 2026-06-14

### Added

- **SQLite knowledge graph storage** (#317)
  - `graph_nodes` + `graph_edges` tables (schema v7)
  - `uteke graph nodes/edges/neighbors/path/query/stats`
  - BFS pathfinding with parent tracking
  - `GraphStore` API: upsert_node, add_edge, find_path, query_relation
- **Structured memory — JSON content** (#293)
  - Auto-detect JSON content, sets `content_type='json'`
  - Schema v6: `content_type TEXT NOT NULL DEFAULT 'text'`
  - Flatten JSON for embedding: `{"name":"Alice"}` → `"name: Alice"`
  - CLI: `--where key=value` filters JSON memories
  - CLI: `--content-format json` pretty-prints JSON output
- **External knowledge import** (#46)
  - `uteke import <file> --tags a,b --format markdown`
  - Auto-detect: `.md`, `.jsonl`, `.txt` from extension or content
  - Markdown: split by headings, each section becomes a memory
  - Text: split by double newline (paragraphs)
  - Stdin: `echo 'text' | uteke import - --tags note`
- **AST-aware code chunking** (#245)
  - Regex-based chunker (zero tree-sitter dependency)
  - Languages: Rust, Go, Python, TypeScript/JS, Dart
  - `detect_language()`, `chunk_code()`, `extract_imports()`
  - Fallback: whole file for unknown languages
- **Hermes plugin integration guide** (#55)
  - `docs/integrations/hermes.md` — complete setup guide
- **Docker quickstart** (#336)
  - `docker-compose.yml` with healthcheck + volume persistence
  - `docs/docker.md` — full Docker guide
  - README Docker section with localhost-only binding
- **Environment variable coverage** (#338)
  - `UTEKE_LOG_LEVEL`, `UTEKE_SERVER_HOST/PORT`
  - `UTEKE_RECALL_MIN_SCORE/STRICT`
  - Resolution: CLI flag > env var > config file > default
  - Invalid values logged as warning

### Changed

- **Schema v7**: graph_nodes + graph_edges tables
- **Schema v6**: content_type column (text vs json)
- `PRAGMA foreign_keys=ON` enabled for cascade deletes

### Fixed

- **Recall `--json` output consistency** — empty results always output `[]`
  instead of `{"results":[]}` when min_score threshold is active

## [0.1.0] — 2026-06-13

### Added

- **Performance benchmark command** (#49)
  - `uteke bench` — generates synthetic memories and benchmarks insert/recall at scale
  - `--counts 100,1000,10000` — configurable memory counts
  - `--json` — machine-readable output
  - Measures: insert ops/sec, recall avg/p95 latency, DB/index size
- **LongMemEval retrieval harness** (#316)
  - `benchmarks/longmemeval/` — evaluates retrieval accuracy vs LongMemEval benchmark
  - Session-level Recall@5/10/50, NDCG@5/10/50
  - Per-question-type breakdown
  - Comparison table vs Mem0/Hindsight
- **Time-travel queries** (#292)
  - `uteke recall --at 2026-06-01T12:00:00Z` — recall memories as they existed at a specific point in time
  - `uteke list --at 2026-06-01T12:00:00Z` — list memories that existed at timestamp
  - Filters by created_at, valid_from/valid_until, deprecated status
  - Server: `/recall` and `/list` accept `"at"` field
- **Pluggable embedding models** (#249)
  - New `Embedder` trait — enables different embedding backends (ONNX, OpenAI, Ollama)
  - `EmbeddingEngine` renamed to `OnnxEmbedder`, implements `Embedder`
  - Config: `[embedding] backend = "onnx"` (default)
  - Lazy backend selection via `embedder_backend` field
- **Room document view** (#306)
  - `uteke room document <room>` — generate structured document from room memories
  - Sections grouped by memory_type: 📋 Decisions, 🔍 Facts, ⚙️ Procedures, 🎨 Preferences, 💬 Context
  - Pinned memories get 📌 section first
  - Server: `POST /room/document`
- **Semantic room recall** (#304)
  - `uteke room recall <room> --query "topic"` — semantic recall within a room
  - `/room/recall` endpoint
- **Room summary** (#305)
  - `uteke room summary <room>` — LLM-free room summary via tag clustering
  - `/room/summary` endpoint
- **Configurable recall threshold** (#252)
  - `uteke recall --min 0.7` — set minimum similarity score
  - `uteke recall --strict` — use 0.7 default threshold
  - `[recall] min_score = 0.7` in config
- **Room-based memory** (#286)
  - `uteke room create/list/add/recall/remove/delete` — full room management
  - Memories can belong to rooms with author attribution
  - `/room/*` endpoints
- **Agent recall optimization** (#181)
  - Recall cache: LRU with TTL (5min, 256 entries) — avoids redundant embedding (~50ms) for repeated queries
  - `uteke recall --context` — formatted output for AI prompt injection
  - Cache metrics in `uteke stats` (hits, misses, hit rate)
  - Auto-invalidation on remember/forget mutations
  - `recall_context()` library API for direct prompt injection
- **Relationship graph layer between memories** (#246)
  - `uteke recall --related --depth N` — follow relationship edges via BFS traversal
  - `uteke remember --meta "rel:supersedes:ID"` — link memories with typed relationships
  - Relationship types: supersedes, contradicts, part_of, references
  - Score decay per depth level (0.8x) to rank direct matches higher
  - No new tables — relationships stored in metadata JSON

### Fixed

- **Recall `--json` output consistency** — empty results now always output `[]`
  instead of `{"results":[]}` when min_score threshold is active. Ensures
  machine consumers (cora-cli, scripts, MCP) can parse output reliably.

### Changed
  - New `memory_tags` junction table for O(log n) tag lookups
  - Schema v5: creates table, populates from existing JSON tags
  - Dual-write: insert/update writes to both JSON column and junction table
  - All tag queries use junction table instead of json_each()
  - Backward compat: JSON `tags` column preserved
- **Smart memory decay and importance scoring** (#247)
  - Composite importance score: 0.3*access + 0.3*recency + 0.2*connectivity + 0.2*pinning
  - `uteke pin <id>` / `uteke unpin <id>` — pin memories so they never decay
  - `uteke importance` — recalculate importance scores for all memories
  - Schema v4: `importance REAL` and `pinned INTEGER` columns
  - Exponential recency decay (half-life: 30 days)
  - Connectivity score from relationship graph (#246)


## [0.0.15] — 2026-06-12

### Changed

- **CLI cold start: ~3s → ~20ms for non-embedding commands** (#185)
  ONNX embedding model is now loaded lazily on first use. Commands like
  `list`, `get`, `stats`, `tags`, `forget`, `namespace`, `aging`, `export`,
  `doctor`, and `verify` start instantly without waiting for model load.
  Commands that need embedding (`remember`, `recall`, `search`) still take
  ~3s on first use per process invocation.
- **Refactor CLI into modular structure** (#131)
  CLI argument definitions extracted to `cli.rs`, logging setup to `logging.rs`.
  main.rs reduced from 449 to ~100 lines for easier maintenance.
- Release workflow now decoupled: crates.io publish runs in parallel with
  builds, GitHub Release only waits for builds. Single platform failure
  no longer blocks release.
- Shell hook scripts inlined into uteke-cli crate for crates.io compatibility.
- Added `.cora.yaml` config and pre-commit hook (Cora v0.5.0).

## [0.0.14] — 2026-06-12


### Security

- Set owner-only file permissions (0700/0600) on database and model directories (#134)
- Add SHA256 checksum verification for downloaded ONNX model files (#134)
- Pin expected model checksums to detect corrupted/tampered downloads

### Added

- Indonesian README translation (README.id.md) with language switcher (#277)
- TLS & Reverse Proxy documentation page (Caddy, Nginx, Cloudflare Tunnel) (#100)
- Crates.io metadata in all Cargo.toml files (#136)

### Changed

- **Server now handles requests concurrently** via thread-per-request (#233)
  Uses `Arc<Mutex<Uteke>>` for safe shared access across threads.
- Contradiction threshold is now a parameter instead of hardcoded 0.65 (#253)
- Rename `euclidean_to_cosine` to `cosine_distance_to_similarity` (#232)
- 9 code quality improvements from Cora scan (#232)

## [0.0.13] — 2026-06-10

### Added

- **FTS5 hybrid search with RRF** — Full-text search (FTS5) as parallel retrieval channel merged with vector search via Reciprocal Rank Fusion (RRF, k=60). New `RecallStrategy` enum: `hybrid` (default), `vector`, `fts5`. FTS5 virtual table auto-created; existing DBs get schema migration v1→v2. Phrase search + token-OR fallback. Deprecated memories excluded from FTS5. 6 new tests (#250, PR #261)
- **Metadata enrichment via CLI flags** — `--entity`, `--category`, `--meta key:value,...` on `remember`. Post-filter on `recall` and `list` by `--entity` and `--category`. `parse_meta_pairs()` with auto type detection (string/number/bool). JSON output includes metadata when present (#251, PR #262)
- **Concurrent reads via RwLock** — `Mutex<VectorIndex>` → `RwLock<VectorIndex>` for read-heavy workload. Multiple concurrent recalls share read lock. Embedder remains `Mutex` (ONNX tokenizer requires `&mut self`) (#209, PR #260)

### Fixed

- **Vector index consistency** — Atomic save for `.keys` sidecar file (temp + rename). `insert()` and `build()` now return `Result` for error propagation (#139, PR #263)
- **FTS5 BM25 score conversion** — Negative unbounded BM25 values were always clamped to 0.0. Fixed to proper sigmoid-based normalization (PR #264)
- **RRF normalization** — `.min(1.0)` → `.clamp(0.0, 1.0)` with clearer math (PR #264)
- **`memories.remove().unwrap()`** — Replaced with `.expect()` for meaningful panic message (PR #264)
- **Server-mode metadata support** — `remember` via HTTP API now includes entity, category, and meta in request body (PR #264)
- **Clippy `collapsible_else_if`** — 2 pre-existing warnings fixed (PR #260)

### Changed

- **Repository transferred** — `ajianaz/uteke` → `codecoradev/uteke`. All references updated across 16 files
- **Cora Review CI** — switched from local Infisical OIDC action to `codecoradev/cora-review-action@v1` with GitHub Secrets. Removed `.cora.yaml` project config
- **README simplified** — 400 → 97 lines. Detailed content moved to VitePress docs (`docs/architecture.md`, `docs/cli-reference.md`, etc.)
- **Roadmap cleaned** — consolidated old versions, removed speculative Phase B/C phases
- **CONTRIBUTING.md** — added Cora CLI integration docs, CI checks table, architecture updated to 3 crates, Key Design Decisions section
- **AGENT.md** — new file with persistent AI agent context: critical rules, architecture, lessons learned, proven workflow
- **docs/architecture.md** — new VitePress page with system overview, data flow diagrams, performance benchmarks, design decisions
- **docs/roadmap.md** — v0.0.12 section added, old versions consolidated, "What's Next" list
- **Star History chart** added to README (cora-cli + uteke)

## [0.0.12] — 2026-06-07

### Fixed

- **TOCTOU race in tag operations** — `rename_tag` and `delete_tag` now start transaction before SELECT, preventing lost updates from concurrent writers (#235)
- **TOCTOU race in aging/prune** — `aging_cleanup` and `prune` now delete by specific IDs instead of re-querying by criteria, preventing vector index orphans (#235)
- **bulk_forget_* lock order** — All 3 bulk delete methods now acquire index lock before SQLite delete, matching the pattern from `forget()` (#236)
- **Server 500 leaks internals** — 500 responses now return generic "Internal server error" to client; full error logged server-side (#237)
- **Server JSON fallback** — `json_response` fallback now uses `serde_json::json!` instead of `format!`, preventing broken JSON (#237)
- **Atomic write tmp naming** — Temp files now named `filename.tmp` instead of fragile extension swapping (#238)

### Added

- **`Store::delete_by_ids()`** — New method for atomic batch deletion by specific IDs

## [0.0.11] — 2026-06-07

### Fixed

- **[CRITICAL] Timestamp format mismatch** — Aging/pruning queries never matched because SQLite `datetime('now')` format differs from stored RFC3339 timestamps. Now computes cutoffs in Rust using `chrono` (#221)
- **Namespace=None inconsistency** — Tag operations (`tags_with_counts`, `rename_tag`, `delete_tag`, `count_by_tag`) treated `None` as "default" namespace instead of "all namespaces". Now consistent with `unique_tags` behavior (#222)
- **Non-atomic model file write** — Model downloads now use atomic write (`.tmp` + rename) to prevent corrupt files on crash. Cleans up leftover `.tmp` files on startup (#225)
- **`uteke_home()` panic** — Replaced `.expect()` with `Result` return type to prevent crashes in minimal Docker/CI environments (#226)
- **Server path matching** — `DELETE /forget` now uses exact path match, preventing false matches on `/forgetful` etc. (#228)
- **Query param parsing** — Use `splitn(2, '=')` to preserve values containing `=` (#228)
- **Missing CLI arg value** — `--host`/`--port` without value now prints error instead of silently ignoring (#228)
- **404 path reflection** — Generic "Not found" message instead of echoing request path (#228)
- **SQLite/index inconsistency** — `forget()` now acquires index lock before SQLite delete to narrow the inconsistency window (#231)
- **Memory type validation** — `remember_typed()` now validates `memory_type` against known variants (#229)

### Added

- **Security scanning workflow** — New `security.yml` CI workflow with `cargo audit` + Trivy filesystem scan. Runs on push, PRs, and daily schedule (#177, #220)
- **quinn-proto update** — Updated to v0.11.14 fixing CVE-2026-31812 (DoS via crafted QUIC packet)
- **`Error::Generic` variant** — New error type for general-purpose errors

### Changed

- **`uteke_home()` returns `Result`** — All callers updated to handle potential failure

## [0.0.10] — 2026-06-07

### Fixed

- **Safe slice for deprecated IDs** — `dep_id.get(..8).unwrap_or(dep_id)` prevents panic on short IDs (#192)
- **Index lock before SQLite write** — Acquire vector index lock before any SQLite writes so lock failures are detected early, preventing false errors (#191)
- **HTTP status checking** — Server proxy now validates response status codes, returning proper error messages instead of silently accepting failures (#193)
- **Aging cleanup filter** — `cleanup_aged` now includes `deprecated = 0` filter matching `find_aged` criteria (#189)
- **Schema migration transactions** — Each migration step + version stamp wrapped in SQLite transaction (#188)
- **Batch bulk deletes** — Replace N individual DELETE statements with single batched query for better performance (#190)

### Changed

- **Store module split** — `store.rs` (2,065 LOC) split into 8 focused modules: schema, crud, tags, aging, bulk, vector, types, store (#179)
- **Commands module split** — `commands.rs` (820 LOC) split into 9 per-command modules (#180)
- **SQLite-first dual-write** — `remember()` now writes to SQLite before vector index, matching `forget()` pattern (#182)
- **Embedding docs corrected** — All docs now correctly state 768d (not 256d) for EmbeddingGemma (#183)
- **Shell hook guards** — Bash `PROMPT_COMMAND` and Zsh `chpwd_functions` now have idempotency guards (#143)
- **Hermes branding removed** — All product-specific branding replaced with generic names; only `--namespace` examples remain (#178)

## [0.0.9] — 2026-06-07

### Changed

- **Website migrated to VitePress** — SvelteKit (3,750 LOC, 10 deps) → VitePress (1,300 LOC markdown, 2 deps) (#194)
  - Built-in full-text search (previously missing)
  - Build time: ~15s → ~6s
  - Content now editable via markdown
  - Brand theme (amber/dark) preserved

## [0.0.8] — 2026-06-04

### Added

- **Architecture: module split** — `lib.rs` (1471→352) and `main.rs` (1538→422) broken into focused modules: `operations`, `maintenance`, `consolidate`, `error`, `types`, `import_export`, `commands`, `init`, `output`, `bench`
- **Input validation** — Max content 10K chars, max 20 tags, max server payload 1MB (#132)
- **Binary checksums** — SHA256 checksums in release artifacts + `verify-checksums` subcommand (#134)
- **Schema versioning** — `schema_version` table + migration framework for future DB upgrades (#138)
- **Error handling rewrite** — `Error` enum with sanitized user-friendly messages, ~90 call sites migrated from raw rusqlite/usearch/ONNX errors (#144)
- **Python wrapper expansion** — 7→21 methods covering all CLI commands, namespace support, type hints, Google-style docstrings (#137)
- **Memory benchmark** — `memory-bench` binary for library-level timing across dataset sizes (#49)
- **Memory consolidation** — `consolidate` command to find and merge near-duplicate memories
- **Import/Export** — JSONL-based memory backup and restore via `import` / `export` commands

### Changed

- **Contradiction detection** — Now read-only during check; deprecation happens after new memory is safely persisted (prevents data loss on insert failure) (#139)
- **README** — v0.0.8 badge, Design Philosophy section, Performance benchmarks

### Fixed

- **Deadlock in `check_contradiction`** — Mutex re-acquire pattern fixed by separating read-only check from mutation (#139)

### Security

- **Error sanitization** — Internal error details (file paths, SQL, model names, ONNX internals) no longer exposed to users (#144)

## [0.0.7] — 2026-06-02

### Added

- **Tag storage: `json_each()` queries** — All 8 tag query methods refactored from `LIKE '%\"tag\"%'` to `json_each()` for exact matching and performance (#120)
- **Config wiring: tier thresholds** — `TierConfig` struct with configurable `hot_days`, `warm_days`, `hot_boost`; `Uteke::open_with_tier()` accepts custom config (#127)
- **Test coverage: 34 → 94 tests** — Comprehensive tests for store, lib, and config modules (#129)
- **Config tests** — 7 new tests for `merge_from_file`, `expand_tilde`, `set_namespace_in_toml` (#129)

### Changed

- **`MemoryTier::from_last_accessed()`** — Now accepts `hot_days` and `warm_days` parameters (was hardcoded 7/30)
- **`tags_with_counts()`** — N+1 query pattern replaced with single `GROUP BY` via `json_each()`
- **`unique_tags()`** — SQL returns individual tag values directly (no in-Rust JSON parsing)
- **`tier_counts()` and `bulk_delete_cold()`** — Now accept configurable threshold parameters

### Fixed

- **Tag substring false positives** — Tag `"rust"` no longer matches memory tagged `"rustacean"`
- **README configuration docs** — Fixed config search paths, removed non-existent `--config` flag, corrected TOML format (#128)

## [0.0.6] — 2026-06-02

### Fixed

- **JSON output omits embedding vector** — `Memory.embedding` now uses `#[serde(skip_serializing, default)]`
  - Reduces JSON response size by ~3KB per memory
  - Embeddings are populated programmatically via ONNX, not from JSON
- **`import()` now persists vector index** — previously imported memories were lost on restart because the index was never saved
- **CI: Node.js 24 enforcement** — added `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24` to all workflows
- **Docker: non-root container** — added `USER uteke` directive (uid/gid 1000) with owned `/data` directory
- **CI: removed unused `musl-tools`** install — targets are glibc only

### Added

- **Dependabot** — automated dependency updates for cargo, GitHub Actions, and Docker

## [0.0.5] — 2026-06-01

### Added

- **UTEKE_HOME environment variable** — single env var to override all `dirs::home_dir()` paths
  - Affects: database path (`uteke.db`), vector index (`uteke_index.usearch`), model cache (`models/`)
  - Default: `$HOME/.uteke` when not set
  - Essential for Docker volume mounts and custom data directories
- **Server reads uteke.toml config** — `uteke-serve` now respects configuration file
  - Reads `[server]` section: `host`, `port`
  - Default host changed to `0.0.0.0` (was `127.0.0.1`) for Docker/network compatibility
  - Config loaded at startup, printed to logs
- **Smart server fallback** — CLI auto-falls back to local mode for server-unsupported commands
  - Commands not yet available via HTTP API gracefully fall back to local execution
  - No more error when `server.enabled = true` and command lacks server endpoint
- **API parity — expanded remember endpoint** — `POST /remember` now accepts all CLI fields
  - `memory_type`, `detect_contradiction`, `valid_from`, `valid_until` parameters
  - Returns contradiction detection result when enabled
- **GET /memory endpoint** — retrieve single memory by ID via `GET /memory?id=<id>`
- **DELETE /forget bulk operations** — `DELETE /forget?all=true&cold=true` for mass deletion
- **Multi-stage Dockerfile** — production-ready Docker image for `uteke-serve`
  - Base: `debian:bookworm-slim` (glibc/ONNX compatible)
  - Model baked into image at build time (~208MB total)
  - Non-root user, health check endpoint, configurable via env vars
- **Docker image CI** — automatic build and push to GHCR on release
  - Multi-platform: `linux/amd64` + `linux/arm64`
  - Buildx with cache, tags: `latest` + version tag
- **Release notes from CHANGELOG.md** — dynamic extraction via `awk` (no hardcoded notes)

### Changed

- Server default host: `127.0.0.1` → `0.0.0.0` (Docker/network accessible)
- Cora review action: hardcoded version → `latest` (auto-updates)

### Fixed

- Pre-existing format issue: `.to_string_lossy().to_string()` chain cleaned up

## [0.0.4] — 2026-05-31

### Added

- **Daemon/server mode** — `uteke-serve` for persistent HTTP API (new `uteke-server` crate)
  - Endpoints: `/health`, `/remember`, `/recall`, `/search`, `/list`, `/forget`, `/stats`, `/namespaces`
  - CORS enabled for browser/extension access
  - Graceful shutdown (SIGINT)
  - Warm recall: **~21ms** vs CLI cold start ~980ms (45x faster)
  - Configuration via `[server]` section in `uteke.toml`
- **CLI auto-routes to server** — CLI detects running server and routes commands via HTTP
  - Transparent fallback to local store if server is not running
  - Config: `[server] enabled = true` in `uteke.toml`
  - Latency: recall 21ms, stats 34ms, remember 32ms (via server)
- **Namespace switching & defaults** — `uteke namespace list/stats/switch`
  - Layered resolution: CLI flag > env `UTEKE_NAMESPACE` > config > default
  - Config persistence in `uteke.toml` under `[store]`
  - `uteke namespace switch <name>` sets default namespace
- **Auto-forget & temporal facts** — contradiction detection and time-bounded memories
  - `--detect-contradiction` flag on `remember` — detects conflicting memories (threshold 0.65)
  - `--type` flag: fact, procedure, preference, decision, context
  - `--valid-from` / `--valid-until` for temporal facts
  - `uteke prune --ttl N --dry-run` — remove deprecated/expired memories
  - DB migration: `deprecated`, `valid_from`, `valid_until`, `memory_type` columns
- **Consolidation & deduplication** — `uteke consolidate --threshold 0.90 --dry-run`
  - O(n²) cosine similarity pairwise comparison
  - Merges duplicates: keeps newer memory, removes older
  - `SimilarPair` and `ConsolidationResult` types
- **Bulk operations** — mass delete by tag, cold tier, or all
  - `forget --tag <tag>`, `forget --cold`, `forget --all`
  - Confirmation flags: `--confirm` or `--dry-run`
- **CI: Cora AI code review** — automated PR review via composite action

### Changed

- Version bumped from 0.0.3 → 0.0.4
- Embedding model confirmed: embeddinggemma-q4 (256 dim)
- Contradiction threshold calibrated at 0.65 for small embedding models
- Consolidate default threshold 0.90 (recommend 0.60-0.70 for small models)

### Stress Test Results

| Test Suite | Operations | Result |
|---|---|---|
| CLI cold start (92 ops) | 92/92 | ✅ (avg ~950ms/op) |
| Server warm (112 ops) | 112/112 | ✅ (avg ~35ms/op) |
| Full functional retest | 15 phases | ✅ All pass |

## [0.0.3] — 2026-05-30

### Added

- **Graceful shutdown** — SIGINT (Ctrl+C) handler via `ctrlc` crate
  - Saves usearch index to disk before exit
  - Prevents index corruption on interrupt
- **File logging with daily rotation** — via `tracing-appender`
  - Logs written to `~/.uteke/logs/uteke.log`
  - Automatic daily rotation (`uteke.log.YYYY-MM-DD`)
  - Non-blocking async writer
- **Configuration file** — `uteke.toml` with layered resolution
  - Search order: `./uteke.toml` → parent dirs → `~/.config/uteke/uteke.toml` → defaults
  - Configurable: `store_path`, `log_level`, `log_dir`, `default_namespace`
  - New `--config` flag to override config file path
- **Tag management commands** — `tags list`, `tags rename`, `tags delete`
  - `tags list [--by-count]` — list all tags with usage counts
  - `tags rename <old> <new>` — rename tag across all memories
  - `tags delete <tag>` — remove tag from all memories
- **`--tags` filter for search** — filter search results by tags
  - `uteke search "query" --tags "rust,cli"`
- **Memory aging with auto-cleanup** — `aging status`, `aging preview`, `aging cleanup`
  - `aging status` — show hot/warm/cold/never-accessed breakdown
  - `aging preview --days N` — preview memories older than N days
  - `aging cleanup --days N [--confirm]` — delete stale memories
- **Shell hook for auto-context loading** — `hook install`
  - Supports bash, zsh, fish
  - Walks up from cwd to find `.uteke/uteke.db`
  - Auto-loads project-scoped context on shell init
  - Shell scripts loaded via `include_str!` from canonical files
  - `SupportedShell` enum for parse-time shell validation
- **Node.js 24** — CI upgraded from Node.js 20 → 24

### Changed

- Version bumped from 0.0.2 → 0.0.3

### Stress Test Results (50 memories)

| Phase | Result | Time |
|---|---|---|
| WRITE (50 memories) | 50/50 ✅ | 49.6s (~1.0/s) |
| RECALL (5 queries) | 5/5 ✅ | 4.8s |
| SEARCH (5 queries) | 5/5 ✅ | 4.7s |
| EXPORT/IMPORT | 51/51 ✅ | - |
| TAGS (list/rename/delete) | ✅ | - |
| AGING | ✅ | - |
| VERIFY + DOCTOR | ✅ All pass | - |
| CLEANUP (50 delete) | 50/50 ✅ | 50.2s |

## [0.0.2] — 2026-05-29

### Added

- **Website** — https://github.com/codecoradev/uteke (SvelteKit 5 + Tailwind)
  - Landing page, docs, roadmap
  - Auto-deploy via CF Pages + Infisical OIDC
- **Release matrix** — 4 platforms: Linux x64, Linux ARM64, macOS ARM64, Windows x64
- **Persistent vector index** — replaced in-memory HNSW with usearch (persistent HNSW)
  - Cold start: loads from disk (~5ms) instead of rebuilding from SQLite (~5s at 10K memories)
  - Incremental delete: `remove()` in ~0.1ms instead of full index rebuild
  - Index persisted as `uteke_index.usearch` + `uteke_index.keys` sidecar
  - Auto-migration: builds usearch index from SQLite on first load
- **Multi-agent namespaces** — isolated memory spaces per agent
  - `--namespace` global flag on all commands
  - SQLite `namespace` column with index
  - Auto-migration of existing databases (zero data loss)
  - Each namespace is fully isolated: recall, search, list, stats scoped
  - Default namespace: `"default"` (backward compatible)
- **Tiered memory** — access-based scoring with Hot/Warm/Cold tiers
  - `access_count` and `last_accessed` tracked per memory
  - Hot memories (accessed within 7 days) get +0.1 score boost in recall
  - Warm (30 days) and Cold (>30 days) tiers for visibility
  - `uteke stats` shows tier breakdown: 🔥 Hot / 🟡 Warm / ❄️ Cold
  - Auto-migration: columns added to existing databases
- **Health check commands** — `doctor`, `verify`, `repair`
  - `uteke doctor` — checks SQLite DB, usearch index, embedding model, consistency
  - `uteke verify` — compares DB count vs index count
  - `uteke repair` — rebuilds usearch index from SQLite
  - All support `--json` output

### Changed

- **License:** MIT → Apache 2.0
- **Vector index:** HNSW (in-memory) → usearch (persistent, incremental)
- **Delete:** rebuild-based → incremental `remove()` + save
- **Startup:** rebuild from SQLite → `restore()` from disk
- **Binary size:** 26MB (v0.0.1) → 28MB (v0.0.2, +usearch)
- **CI:** only runs on PR to develop and push to main (eliminates duplicate runs)
- **Release:** versioned artifact filenames (`uteke-{version}-{target}.tar.gz`)
- **CI secrets:** Infisical OIDC for CF Pages deploy (website workflow)

### Removed

- Old deps: `hnsw`, `rand_pcg`, `space` (replaced by `usearch`)
- macOS Intel (`x86_64-apple-darwin`) from release matrix
- Windows ARM64 (`aarch64-pc-windows-msvc`) from release matrix (numkong incompatibility)

### Docs

- **INSTALL.md:** Windows setup guide (pre-built + build from source)
- **CONTRIBUTING.md:** HNSW → usearch references updated
- **README:** architecture table, tiered memory, health check commands

## [0.0.1] — 2026-05-29

### Added

- **Core memory engine** — store, recall, search, forget, list, get operations
- **Semantic search** — vector similarity using HNSW index with cosine scoring
- **ONNX embedding** — EmbeddingGemma Q4 model (768d), auto-downloaded on first run
- **SQLite storage** — embedded database with indexed tags and metadata
- **CLI** — full command-line interface with clap
  - `remember` — store memories with optional tags
  - `recall` — semantic search with `--limit` and tag filter
  - `search` — keyword text search
  - `list` — paginated listing with `--tag` filter
  - `get` — retrieve single memory by ID
  - `forget` — delete memory by ID
  - `stats` — show store statistics
  - `completions` — generate shell completions (bash, zsh, fish)
- **JSON output** — `--json` flag on all commands for machine-readable output
- **Python wrapper** — zero-dependency `UtekeMemory` class (stdlib only, Python 3.8+)
- **Custom store path** — `--store` flag to override default `~/.uteke` location
- **Verbose logging** — `--verbose` flag for debug output
- **CI pipeline** — GitHub Actions with check, fmt, clippy, test, build jobs
- **Workspace structure** — `uteke-core` library + `uteke-cli` binary crates
- **No unsafe code** — `unsafe_code = "forbid"` in workspace lints

### Technical Details

- **Embedding model:** onnx-community/embeddinggemma-300m-ONNX (Q4 quantized, 768 dimensions)
- **Vector index:** HNSW with configurable ef and k parameters
- **Storage:** SQLite via rusqlite (bundled) with WAL mode
- **Tokenization:** HuggingFace tokenizers crate
- **Binary name:** `uteke`
- **Minimum Rust version:** 1.75+

[Unreleased]: https://github.com/codecoradev/uteke/compare/v0.8.0...HEAD
[0.8.0]: https://github.com/codecoradev/uteke/compare/v0.7.3...v0.8.0
[0.7.3]: https://github.com/codecoradev/uteke/compare/v0.7.2...v0.7.3
[0.7.2]: https://github.com/codecoradev/uteke/compare/v0.7.1...v0.7.2
[0.7.1]: https://github.com/codecoradev/uteke/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/codecoradev/uteke/compare/v0.6.7...v0.7.0
[0.6.7]: https://github.com/codecoradev/uteke/releases/tag/v0.6.7
[0.6.6]: https://github.com/codecoradev/uteke/releases/tag/v0.6.6
[0.6.5]: https://github.com/codecoradev/uteke/releases/tag/v0.6.5
[0.6.4]: https://github.com/codecoradev/uteke/releases/tag/v0.6.4
[0.6.3]: https://github.com/codecoradev/uteke/releases/tag/v0.6.3
[0.6.2]: https://github.com/codecoradev/uteke/releases/tag/v0.6.2
[0.6.1]: https://github.com/codecoradev/uteke/releases/tag/v0.6.1
[0.6.0]: https://github.com/codecoradev/uteke/releases/tag/v0.6.0
[0.5.0]: https://github.com/codecoradev/uteke/releases/tag/v0.5.0
[0.4.3]: https://github.com/codecoradev/uteke/releases/tag/v0.4.3
[0.4.2]: https://github.com/codecoradev/uteke/releases/tag/v0.4.2
[0.4.1]: https://github.com/codecoradev/uteke/releases/tag/v0.4.1
[0.4.0]: https://github.com/codecoradev/uteke/releases/tag/v0.4.0
[0.3.2]: https://github.com/codecoradev/uteke/releases/tag/v0.3.2
[0.3.1]: https://github.com/codecoradev/uteke/releases/tag/v0.3.1
[0.3.0]: https://github.com/codecoradev/uteke/releases/tag/v0.3.0
[0.2.1]: https://github.com/codecoradev/uteke/releases/tag/v0.2.1
[0.2.0]: https://github.com/codecoradev/uteke/releases/tag/v0.2.0
[0.1.0]: https://github.com/codecoradev/uteke/releases/tag/v0.1.0
[0.0.15]: https://github.com/codecoradev/uteke/releases/tag/v0.0.15
[0.0.14]: https://github.com/codecoradev/uteke/releases/tag/v0.0.14
[0.0.13]: https://github.com/codecoradev/uteke/releases/tag/v0.0.13
[0.0.12]: https://github.com/codecoradev/uteke/releases/tag/v0.0.12
[0.0.11]: https://github.com/codecoradev/uteke/releases/tag/v0.0.11
[0.0.10]: https://github.com/codecoradev/uteke/releases/tag/v0.0.10
[0.0.9]: https://github.com/codecoradev/uteke/releases/tag/v0.0.9
[0.0.8]: https://github.com/codecoradev/uteke/releases/tag/v0.0.8
[0.0.7]: https://github.com/codecoradev/uteke/releases/tag/v0.0.7
[0.0.6]: https://github.com/codecoradev/uteke/releases/tag/v0.0.6
[0.0.5]: https://github.com/codecoradev/uteke/releases/tag/v0.0.5
[0.0.4]: https://github.com/codecoradev/uteke/releases/tag/v0.0.4
[0.0.3]: https://github.com/codecoradev/uteke/releases/tag/v0.0.3
[0.0.2]: https://github.com/codecoradev/uteke/releases/tag/v0.0.2
[0.0.1]: https://github.com/codecoradev/uteke/releases/tag/v0.0.1
