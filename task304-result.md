# Task #304 — Semantic Room Recall: Implementation Complete

## Summary

Implemented semantic room recall — `uteke room recall <room_id> --query "..."` ranks memories by relevance instead of chronological dump. Backward compatible: without `--query`, original chronological behavior preserved.

## Files Modified

### 1. `crates/uteke-core/src/memory/rooms.rs` — `get_room_memory_ids()`
- New method returns `Vec<String>` of memory IDs linked to a room
- Optional author filter via `WHERE author = ?`
- Cheap query — avoids loading full memory objects

### 2. `crates/uteke-core/src/rooms.rs` — `recall_room_semantic()`
- New method on `Uteke`: `recall_room_semantic(room_id, query, limit, author, min_score) -> Result<Vec<SearchResult>>`
- Algorithm:
  1. Get room memory IDs via `store.get_room_memory_ids()`
  2. Over-fetch via `recall_hybrid()` with `RecallStrategy::Hybrid`, no namespace filter (rooms are cross-namespace)
  3. Post-filter to only results whose `memory.id` is in the room ID set
  4. Apply `min_score` filter
  5. Sort by score descending, truncate to limit

### 3. `crates/uteke-cli/src/cli.rs` — Extended `RoomCommands::Recall`
- Added `--query <TEXT>` — semantic query for relevance ranking
- Added `--min <FLOAT>` — minimum similarity score (0.0-1.0), only used with `--query`
- Backward compatible: `room recall <id>` without flags unchanged

### 4. `crates/uteke-cli/src/commands/room.rs` — Handler updated
- `run()` now takes `config: &Config` for threshold resolution
- If `query` is `Some`: calls `recall_room_semantic()`, shows scores via `print_room_semantic_human()`
- If `query` is `None`: calls `recall_room()` as before (chronological)
- Resolves `min_score` from `--min` flag or `config.recall.min_score` default
- Updated dispatch in `commands/mod.rs` to pass config

### 5. `crates/uteke-cli/src/output.rs` — `print_room_semantic_human()`
- Shows results with room context: room ID, author, scores
- Format: `1. (score: 0.82) Deploy v2 to staging Friday [deploy]`

### 6. `crates/uteke-server/src/main.rs` — `POST /room/recall` route
- New request type `RoomRecallRequest` with `room_id`, `query`, `limit`, `author`, `min_score`
- Calls `uteke.recall_room_semantic()`, returns `Vec<SearchResult>` as JSON
- Resolves threshold from request or config

## Verification

- `cargo check --workspace` passes (0 errors)
- `cargo fmt --all` applied
- Only pre-existing warning: unused mut in rooms.rs (unrelated)

## Usage Examples

```bash
# Semantic recall — rank by relevance
uteke room recall discord:123 --query "deployment strategy"

# With author filter and minimum score
uteke room recall discord:123 --query "deploy" --author kai --min 0.5

# Original chronological recall (unchanged)
uteke room recall discord:123
uteke room recall discord:123 --author kai --limit 50

# Server API
curl -X POST http://localhost:8767/room/recall \
  -H "Content-Type: application/json" \
  -d '{"room_id":"discord:123","query":"deploy","limit":10,"min_score":0.3}'
```
