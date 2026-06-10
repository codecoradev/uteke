# Task 2 Result — Core: Add recall threshold filtering to all recall paths

**File modified:** `crates/uteke-core/src/operations.rs`

## Changes Summary

Added `min_score: f32` parameter to all 4 recall methods:

### 1. `recall()` (line ~115)
- Added `min_score: f32` parameter
- After `results.sort_by()`, added filter block:
  ```rust
  if min_score > 0.0 {
      results.retain(|r| r.score >= min_score);
  }
  ```

### 2. `recall_hybrid()` (line ~230)
- Added `min_score: f32` parameter
- Passes `min_score` through to all 3 strategy dispatches (Vector→recall, Fts5→recall_fts5_only, Hybrid→recall_rrf)

### 3. `recall_fts5_only()` (line ~246)
- Added `min_score: f32` parameter
- After result collection, added filter block before touch access
- Fallback to `recall()` passes `0.0` (no double-filtering)

### 4. `recall_rrf()` (line ~314)
- Added `min_score: f32` parameter
- Internal vector search call passes `0.0` (RRF does its own filtering on normalized scores)
- Fallback to `recall_fts5_only` passes `min_score` through
- After final result collection, added filter block before touch access

## Design Decisions
- `min_score == 0.0` means no filtering (backward compatible)
- RRF passes `0.0` to internal `recall()` call because RRF normalizes scores differently — filtering happens once on the final normalized RRF scores
- FTS5-only fallback to vector also passes `0.0` since the FTS5 method will apply its own filtering

## Expected Compilation Impact
Callers in `commands/recall.rs` and `server/main.rs` will fail to compile until Task 4 updates them to pass the new `min_score` parameter.
