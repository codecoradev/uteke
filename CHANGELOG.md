# Changelog

All notable changes to Uteke will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

- **Website** — https://uteke.ajianaz.dev (SvelteKit 5 + Tailwind)
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

[0.0.10]: https://github.com/ajianaz/uteke/releases/tag/v0.0.10
[0.0.9]: https://github.com/ajianaz/uteke/releases/tag/v0.0.9
[0.0.8]: https://github.com/ajianaz/uteke/releases/tag/v0.0.8
[0.0.7]: https://github.com/ajianaz/uteke/releases/tag/v0.0.7
[0.0.6]: https://github.com/ajianaz/uteke/releases/tag/v0.0.6
[0.0.5]: https://github.com/ajianaz/uteke/releases/tag/v0.0.5
[0.0.4]: https://github.com/ajianaz/uteke/releases/tag/v0.0.4
[0.0.3]: https://github.com/ajianaz/uteke/releases/tag/v0.0.3
[0.0.2]: https://github.com/ajianaz/uteke/releases/tag/v0.0.2
[0.0.1]: https://github.com/ajianaz/uteke/releases/tag/v0.0.1
