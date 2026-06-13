# Task #292 — Time-travel queries: COMPLETE ✅

## Summary
Implemented temporal queries for memories: `recall_at_time()` and `list_at_time()` filter memories by `created_at`, `valid_from`, `valid_until`, and `deprecated` columns to reconstruct the memory state at a specific point in time.

## Files modified

### Core (uteke-core)
1. **`crates/uteke-core/src/memory/crud.rs`** — New `Store::list_at_time()` method
   - SQL query with temporal conditions: `created_at <= pit AND (valid_from IS NULL OR valid_from <= pit) AND (valid_until IS NULL OR valid_until > pit) AND deprecated = 0`
   - Supports optional tag filter via `memory_tags` junction table (schema v5)
   - Uses `row_to_memory()` mapper consistent with other list methods

2. **`crates/uteke-core/src/operations.rs`** — Two new `Uteke` methods
   - `recall_at_time()` — over-fetches candidates from `recall()`, post-filters by temporal validity, truncates to `limit`
   - `list_at_time()` — thin wrapper around store-level temporal query

3. **`crates/uteke-core/src/memory/store.rs`** — Added 4 unit tests:
   - `test_list_at_time_basic` — future memories excluded
   - `test_list_at_time_excludes_expired` — `valid_until <= pit` excluded
   - `test_list_at_time_excludes_deprecated` — deprecated memories excluded
   - `test_list_at_time_excludes_future_valid_from` — `valid_from > pit` excluded

### CLI (uteke-cli)
4. **`crates/uteke-cli/src/cli.rs`** — Added `--at` flag to `Recall` and `List` commands
5. **`crates/uteke-cli/src/commands/recall.rs`** — `run_recall()` accepts `at: Option<&str>`, parses RFC3339, calls `recall_at_time()`. Validates `--at` and `--related` are mutually exclusive.
6. **`crates/uteke-cli/src/commands/list.rs`** — `run_list()` accepts `at: Option<&str>`, parses RFC3339, calls `list_at_time()`
7. **`crates/uteke-cli/src/commands/mod.rs`** — Dispatch updated to pass `at` field
8. **`crates/uteke-cli/src/commands/server.rs`** — Server proxy passes `at` in JSON body to `/recall` and `/list`
9. **`crates/uteke-cli/Cargo.toml`** — Added `chrono` dependency

### Server (uteke-server)
10. **`crates/uteke-server/src/main.rs`** — 
    - `RecallRequest` and `ListParams` structs: added `at: Option<String>` field
    - `/recall` endpoint: parses `at`, calls `recall_at_time()` when present
    - `/list` endpoint: parses `at`, calls `list_at_time()` when present
    - Both return 400 on invalid RFC3339 timestamps
11. **`crates/uteke-server/Cargo.toml`** — Added `chrono` dependency

## Temporal filter logic
A memory existed at `point_in_time` iff:
- `created_at <= point_in_time` (memory was created)
- `valid_from IS NULL OR valid_from <= point_in_time` (memory was effective)
- `valid_until IS NULL OR valid_until > point_in_time` (memory not yet invalidated)
- `deprecated = false` (memory not deprecated)

## Verification
- `cargo fmt --all` ✅
- `cargo clippy --workspace -- -D warnings` ✅ clean
- `cargo test --workspace` ✅ — 115 passed (was 111, +4 new tests), 4 ignored (ONNX-dependent)
- `cargo build --workspace` ✅
- CLI help shows `--at` flag on both `recall` and `list` ✅

## Usage examples
```bash
# List memories that existed on June 1st
uteke list --at 2026-06-01T12:00:00Z

# Semantic recall filtered to memories valid at a point in time
uteke recall "deployment process" --at 2026-06-01T12:00:00Z --limit 5

# Server API
curl -X POST http://localhost:7777/list \
  -H "Content-Type: application/json" \
  -d '{"at": "2026-06-01T12:00:00Z", "limit": 20}'
```
