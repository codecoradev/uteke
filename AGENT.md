# Uteke — Agent Context

> This file is permanent context for AI agents working in this repository. Read it fully before you start coding.

## Project Overview

**Uteke** is a local-first semantic memory engine for AI agents. Single Rust binary, fully offline, ~30ms recall. No API key, Docker, or cloud service needed.

- **Repo:** `codecoradev/uteke` (remote GitHub), local at `/Users/mis-puragroup/development/riset-ai/uteke`
- **Version:** 0.0.14
- **License:** Apache 2.0
- **Main branches:** `develop` (all PRs go here), `main` (release)

## Architecture

### Workspace Crates (3 crates)

| Crate | Path | Purpose |
|-------|------|---------|
| `uteke-core` | `crates/uteke-core/` | Library — storage, embedding, vector search, FTS5, operations |
| `uteke-cli` | `crates/uteke-cli/` | CLI binary — clap commands, JSON output, server proxy |
| `uteke-server` | `crates/uteke-server/` | HTTP server — persistent daemon for fast agent access |

### Module Structure

```
crates/uteke-core/src/
├── lib.rs              # Uteke struct — main public API
├── operations.rs       # High-level ops (remember, recall, search, forget, etc.)
├── error.rs            # Error enum, sanitized messages
├── consolidate.rs      # Memory consolidation (cosine dedup)
├── maintenance.rs      # Doctor, verify, repair
├── import_export.rs    # JSONL import/export
├── embed/
│   ├── mod.rs          # Embedder struct — ONNX inference
│   └── engine.rs       # Engine trait + ONNX implementation
└── memory/
    ├── mod.rs          # Module re-exports
    ├── store.rs        # Store struct — SQLite operations
    ├── vector.rs       # VectorIndex — usearch HNSW + RwLock
    ├── fts5.rs         # FTS5 full-text search + RRF fusion
    ├── schema.rs       # Schema versioning + migrations
    ├── crud.rs         # CRUD operations (insert, get, update, delete)
    ├── types.rs        # Type definitions (Memory, RecallResult, RecallStrategy, etc.)
    ├── tags.rs         # Tag operations (json_each queries)
    ├── aging.rs        # Aging tier operations
    ├── bulk.rs         # Bulk delete operations
    └── vector.rs       # Vector index management

crates/uteke-cli/src/
├── main.rs             # Entry point, clap app definition
└── commands/
    ├── mod.rs
    ├── remember.rs      # --entity, --category, --meta flags
    ├── recall.rs        # --entity, --category filter
    ├── list.rs          # --entity, --category filter
    ├── server.rs        # HTTP proxy to uteke-serve
    └── ...              # Other per-command modules

crates/uteke-server/src/
└── main.rs             # Actix-web server
```

### Key Components

| Component | Technology | Details |
|-----------|------------|---------|
| Vector Index | usearch v2.25.3 | Persistent HNSW, `RwLock` for concurrent reads |
| Full-Text Search | SQLite FTS5 | Virtual table, phrase + token-OR fallback |
| Hybrid Search | RRF (k=60) | Reciprocal Rank Fusion merges vector + FTS5 results |
| Storage | SQLite (rusqlite) | WAL mode, schema v2 |
| Embedding | EmbeddingGemma Q4 ONNX | 768d vectors, `Mutex` (ONNX tokenizer needs `&mut self`) |
| CLI | clap v4 | Standard Rust CLI |
| Server | actix-web | CORS enabled, ~42ms warm recall |

### Schema Versioning

- `schema_version` table with integer counter
- Current: **v2** (FTS5 migration)
- Auto-migration on upgrade, zero data loss

---

## Critical Rules — MUST FOLLOW

### 1. Always `cargo fmt` Before Commit

CI runs `cargo fmt --check` and **will fail** if there are formatting issues. A single space or newline mistake is enough to fail the PR.

```bash
# ALWAYS run before commit
cargo fmt
```

### 2. Run Cora Review Locally Before Push

Cora CLI has found **real bugs** in this project (BM25 score always 0, RRF normalization wrong, metadata missing in server mode). Don't wait for CI.

```bash
cora review --base origin/develop --format text
```

If Cora finds error-level issues, **fix first** before pushing.

### 3. Clippy = Error

CI runs `cargo clippy -- -D warnings`. All warnings are treated as errors.

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

### 4. Branch Protection Rules

- **Develop branch:** all changes must go through PR
- **10 CI checks** must pass: Build, Check, Clippy, Format, Test, Cora Review, CodeRabbit, Cargo Audit, Trivy FS Scan, GitGuardian
- **Cannot** push directly to develop

### 5. Never `.unwrap()` in Production Code

Use `.expect("clear message")` or `if let` / `match` patterns. `.unwrap()` without context makes debugging impossible if it panics.

```rust
// ❌ Don't
memories.remove(0).unwrap()

// ✅ Correct
memories.remove(0).expect("guaranteed by prior count check")

// ✅ Better
if let Some(memory) = memories.into_iter().next() { ... }
```

### 6. Atomic File Writes

For all file I/O that writes important data (index, config, model), use this pattern:

```rust
// Write to .tmp first, then rename (atomic on POSIX)
let tmp_path = path.with_extension("tmp");
fs::write(&tmp_path, &data)?;
fs::rename(&tmp_path, &path)?;
```

### 7. SQLite-First Dual-Write Pattern

- `remember()`: Write to SQLite **first**, then vector index
- `forget()`: Acquire index lock **first**, then SQLite delete
- This pattern prevents inconsistency between DB and index

### 8. Always Update CHANGELOG and Docs Before Commit

Every commit that adds/changes a feature or fix **must** include updates to:

1. **CHANGELOG.md** — Add entry under `[Unreleased]` (Added/Fixed/Changed)
2. **docs/cli-reference.md** — If there's a new CLI flag or behavior change
3. **docs/roadmap.md** — If an issue is completed
4. **README.md** — If there are significant changes to features or quick start
5. **AGENT.md** — If there are architecture changes, new limitations, or lessons learned
6. **Version badge** — If releasing, update badge in README

Outdated documentation is more dangerous than no documentation.

### 9. VitePress Docs Auto-Deploy on Release

The `deploy-website.yml` workflow automatically deploys to Cloudflare Pages when:
- Push to `main` (after release workflow syncs from develop)
- Push tag `v*` (new release)
- Manual trigger via GitHub UI

Make sure docs are up-to-date **before** pushing a release tag. Docs deploy from the `main` branch, not `develop`.

### 10. All CI Checks Must Pass — No Exceptions

Never ignore a failing CI check with excuses like "it's an external app" or "not a required check." Every red CI check must be investigated.

**Real experience:** PR #274 had a `CodeCora` failure that was ignored. It turned out Cora Review (a separate check) found 2 critical bugs:
1. RRF scores ≠ cosine similarity — `min_score` filter was targeting the wrong thing
2. Server ignores `[recall]` config — CLI vs server behavior differed

If a CI check fails:
1. **Read the error/log** — don't immediately say "it's safe"
2. **Check if the finding is valid** — trace to the related code
3. **If valid** → fix first, don't merge
4. **If false positive** → document why, don't stay silent
5. **Don't merge while there's red** — unless 100% sure it's noise

Principle: **Red CI = there's a problem. Investigate first, don't assume.**

---

## Lessons Learned — From Real Experience

### FTS5 + Vector Hybrid Search Is Non-Trivial

Combining two ranking systems is tricky:
- **Vector cosine similarity:** range 0..1
- **BM25 (FTS5):** negative unbounded (can be -5, -10, etc.)
- **Don't clamp BM25 to 0..1 directly** — it destroys ranking
- Use **RRF (Reciprocal Rank Fusion)** — rank-based, doesn't care about original scale
- If you need to normalize to 0..1, use sigmoid: `1.0 / (1.0 + (-score).exp())`

### RwLock vs Mutex — Choose Based on Workload

- `RwLock` for **read-heavy** workloads (vector index: recall/search far more frequent than remember/forget)
- `Mutex` when the operation needs `&mut self` (ONNX tokenizer)
- **Don't blindly swap Mutex → RwLock.** Profile first.

### Score Normalization Must Be Precise

The smallest bug in score calculation can break a feature entirely:
- `.min(1.0)` vs `.clamp(0.0, 1.0)` — huge difference when negative values exist
- Always write **unit tests** that verify score ranges

### Server Mode = Hidden Surface Area

When adding new parameters to the CLI, **don't forget to update server mode too.** Bug #264: `--entity`, `--category`, `--meta` were added to CLI but forgotten in the server endpoint.

**Checklist when adding a new CLI flag:**
1. Command module (`commands/remember.rs`)
2. Server endpoint (`commands/server.rs` — proxy body)
3. Server handler (`uteke-server/src/main.rs`)
4. API docs
5. CLI reference docs

### Metadata in JSON Blob — Post-Filter, Not SQL Filter

Entity, category, and meta are stored as JSON in the `metadata` column. This means:
- **Filtering is done in Rust**, not SQL WHERE clause
- No index on individual fields inside JSON
- For large datasets (>10K), consider separate columns

### Unit Tests Are Not Enough — Manual Stress Testing Is Required

Unit tests (108) don't cover:
- Bulk insert of 100+ memories (performance regression?)
- Concurrent access via server mode
- Unicode / special characters in content
- Schema migration from old DB version
- Crash recovery (kill during write)

Run manual stress tests after significant changes.

### Documentation Gets Outdated Quickly

CONTRIBUTING.md once said "2 crates" when it had been 3 since v0.0.4. Version badge was behind. **Always update docs before commit** — see Critical Rule #8.

---

## Proven Workflow

### Per-Issue Workflow

```
 1. git checkout develop && git pull
 2. git checkout -b <type>/<short-description>
 3. Implementation (read related modules first)
 4. cargo fmt && cargo clippy && cargo test
 5. cora review --base origin/develop --format text  (local review)
 6. Fix all Cora findings
 7. Update CHANGELOG.md (add under [Unreleased])
 8. Update docs/ if there are new features/flags (see Critical Rule #8)
 9. git add -A && git commit -m "type: description"
10. git push origin <branch>
11. gh pr create --base develop
12. Monitor CI (gh pr checks <number>)
13. Review PR comments (Cora, CodeRabbit)
14. Fix if there are new findings
15. gh pr merge <number> --squash --delete-branch
16. Pick next issue
```

### Branch Naming Convention

```
feat/<new-feature>
fix/<bug-being-fixed>
docs/<what-was-updated>
refactor/<what-was-refactored>
```

### Commit Message Convention

```
type: description (#issue-number)

type: feat, fix, docs, refactor, test, chore
```

Examples:
```
feat: add FTS5 hybrid search with RRF (#250)
fix: BM25 score always returning 0.0
docs: update CLI reference for metadata flags
```

---

## Known Limitations

| Limitation | Status | Details |
|------------|--------|---------|
| usearch `ef` parameter cannot be set | External | usearch v2.25.3 Rust bindings don't expose `ef` in `search()` |
| Embedder requires `Mutex` | Architectural | ONNX tokenizer internally uses `&mut self` |
| Metadata filtering is post-filter | Design | Entity/category/meta in JSON blob, not SQL column |
| Consolidate is O(n²) | Algorithm | Pairwise cosine, slow at >1000 memories |
| FTS5-only mode score placeholder | Design | BM25 can't normalize to 0..1, actual ranking via RRF |

---

## Quick Reference

```bash
# Build
cargo build --workspace

# Test (108 unit tests)
cargo test --workspace

# Format + Lint
cargo fmt && cargo clippy --workspace --all-targets -- -D warnings

# Local Cora review
cora review --base origin/develop --format text

# Create PR
gh pr create --base develop --title "type: description" --body 'summary'

# Check CI
gh pr checks <number>

# Merge
gh pr merge <number> --squash --delete-branch
```
