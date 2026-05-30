# Changelog

All notable changes to Uteke will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[0.0.3]: https://github.com/ajianaz/uteke/releases/tag/v0.0.3

## [Unreleased]

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

[0.0.2]: https://github.com/ajianaz/uteke/releases/tag/v0.0.2
[0.0.1]: https://github.com/ajianaz/uteke/releases/tag/v0.0.1
