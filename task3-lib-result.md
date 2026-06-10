# Task 3 — Core lib: Wire recall threshold to Uteke struct

## File modified
`crates/uteke-core/src/lib.rs`

## Changes made

### 1. Added `RecallConfig` struct (after `TierConfig`)
- `pub struct RecallConfig` with `min_score: f32` field
- `Debug, Clone, Copy` derives
- `Default` impl with `min_score: 0.0`
- Doc comment: "Configuration for recall threshold"

### 2. Added `recall_config: RecallConfig` field to `Uteke` struct
- Fourth field after `tier_config`

### 3. Updated `finish_open()` signature
- Added `recall_config: RecallConfig` parameter
- Included `recall_config` in the `Ok(Self { ... })` return

### 4. Updated all existing constructors to pass `RecallConfig::default()`
- `open()` → `RecallConfig::default()`
- `open_with_embedder()` → `RecallConfig::default()`
- `open_with_tier()` → `RecallConfig::default()`

### 5. Added `open_with_recall()` constructor
- Accepts `path` and `recall_config: RecallConfig`
- Passes `TierConfig::default()` and `recall_config` to `finish_open`

### 6. No getter added for `recall_config` (as specified)

### 7. Added test `test_recall_config_default`
- Asserts `min_score` defaults to `0.0` within `f32::EPSILON`

## Notes
- `RecallConfig` is defined directly in `lib.rs` (like `TierConfig`), so it's publicly accessible as `uteke_core::RecallConfig` without additional `pub use` entries.
- No other files reference `finish_open`, so no cross-file breakage.
- `recall_config` field is stored but not yet used in operations methods — that wiring happens in a later task.
