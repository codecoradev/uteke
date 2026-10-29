# Generational reset and resumable sliced maintenance (#1469)

Status: **design for agreement. The issue says not to start code before the design is agreed.**

## 1. Where we are

- The maintenance passes (`DreamPhase`: lint, backlinks, dedup, contradict, orphans, compact, verify, in `crates/uteke-core/src/dream.rs`) run against the live store in one call: `uteke dream`, or the server's `auto_dream` thread every few days. A run is all-or-nothing from the caller's point of view: there is no persisted progress, so a long run that dies starts over, and nothing records which slice did what.
- Derived data lives in two places: SQLite tables (`memory_edges`, the graph tables, timeline) and the vector index file (`uteke_index.usearch` / `.vecq`). SQLite is the source of truth; the index is rebuilt from it when missing or invalid (`finish_open_full`).
- The index file is protected across processes by an exclusive `fs2` file lock held for the life of the `VectorIndex`; threads inside one process share it behind an `RwLock`.
- The vecq engine already compacts tombstones (`compact`), which is a rebuild of one derived structure.

## 2. Two separable problems

1. **Resumable sliced runs.** Do a pass in bounded slices with persisted progress, so it survives a restart and can be paused.
2. **Generations of derived structures.** Start a fresh generation without a reader ever seeing old nodes joined to new edges.

They can ship separately, and (1) is useful on its own. This note proposes doing (1) first.

## 3. Proposal

### 3.1 Sliced runs (first)

```
maintenance_runs(id, pass, scope, state, position, total, started_at, updated_at, error)
maintenance_slices(run_id, seq, started_at, finished_at, changes, warnings)   -- one audit row per slice
```

A pass is written as `fn step(&self, run: &mut RunState) -> Result<SliceOutcome>` that does at most a bounded amount of work (N memories, or T milliseconds) and returns the new position. A driver loops `step` until done, persisting `position` and one `maintenance_slices` row after each slice, in the same transaction as the slice's changes. State machine: `pending -> running -> done | failed`, plus `paused`. `uteke dream --resume` and the server thread pick up a `running` row after a restart. Status is `uteke dream status` / `GET /dream/status` (LAB): pending, running, done, failed, with scope and position.

The final step of a full run refreshes the cached snapshot; a run that skipped it (paused, failed, or an operator stopping early) marks the snapshot stale, so a reader never trusts a half-refreshed one.

### 3.2 Generations (second, only after the first proves out)

- A `generation` integer on the derived SQLite tables and a `current_generation` row in a meta table. A reset creates generation N+1 empty, marks N read-only (kept for rollback), and new derivation writes into N+1.
- **Reads pin one generation per request**: read `current_generation` once at the start of the request and filter every derived read by it. Inside one SQLite read transaction (WAL gives a stable snapshot) the pin cannot change mid-request, which is exactly the "no old nodes with new edges" guarantee, without a lock.
- **The vector index** is derived from `memories`: a reset builds the new index into `uteke_index.<gen>.tmp` and swaps it in under the existing file lock, then flips `current_generation`. The old file stays until pruned.

### 3.3 Retention and pruning

Keep the previous generation (one) until `uteke dream verify` passes on the new one, then delete it after a configurable number of days. Disk overhead is therefore about one extra copy of the derived tables and one index file for a short window.

## 4. Open questions (the issue's own, with a proposed answer)

| Question | Proposed answer | Needs |
|---|---|---|
| Locking vs the `fs2` index lock | keep it: the swap happens while holding it; readers in other processes see either the old or the new file, never a partial one | check on Windows (the repo has had lock-violation bugs there) |
| Disk overhead and pruning | one previous generation, pruned after verify plus N days | owner: N |
| Interaction with usearch rebuild | rebuild goes to a temp file, then atomic rename; no in-place rebuild | benchmark the rebuild time on a large store |
| Do we need generations at all for the SQLite tables? | maybe not: a pass that rewrites edges inside one transaction is already atomic. Generations pay off only for multi-slice runs that must not be visible half-done | decide after the sliced runs exist |

## 5. Acceptance (from the issue) and the plan to meet it

- **Design note agreed** before code: this document.
- **Reset + sliced run + resume on a large store without read errors:** a test store of at least 100k memories built by the Modal harness; a script that runs a sliced dedup while hammering `recall`, kills the process mid-run, resumes, and asserts zero read errors and final equality with an uninterrupted run.
- **Status visible:** the `status` command and endpoint above.

## 6. Sequencing

1. Sliced runs for one pass (`dedup`, the slowest) behind `uteke dream --sliced`, with the status command.
2. The remaining passes.
3. Re-evaluate generations with real numbers; write a second design note if still needed.
