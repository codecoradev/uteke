# Task 1 — `[recall]` Config Section

## Status: ✅ Complete

## Changes made to `crates/uteke-cli/src/config.rs`

### 1. Added `RecallConfig` struct
- Placed before `ServerConfig` (after `AgingConfig`)
- Fields: `min_score: f64` (default 0.3), `min_score_strict: f64` (default 0.5)
- Derives `serde::Deserialize, Clone` with `#[serde(default)]`

### 2. Added `recall` field to `Config` struct
- `pub recall: RecallConfig` between `aging` and `server` fields

### 3. Added merge logic in `merge_from_file()`
- Merges `[recall]` section keys `min_score` and `min_score_strict`
- Same pattern as other sections (check key presence, copy from overlay)

### 4. Added `[recall]` to default TOML template in `write_default_config()`
```toml
[recall]
# min_score = 0.3
# min_score_strict = 0.5
```

### 5. Added tests
- `default_recall_config` — verifies defaults (min_score=0.3, min_score_strict=0.5)
- `parse_recall_config` — parses TOML with `[recall]` section, verifies custom values (0.45, 0.7)
- `merge_recall_config` — merges file with recall overrides, verifies other config untouched
- Updated `merge_from_file_all_sections` — includes `[recall]` section with min_score=0.4, min_score_strict=0.65

## Test results
All 18 tests pass (15 existing + 3 new).
