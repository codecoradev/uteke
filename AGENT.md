# Uteke — Agent Context

> File ini adalah konteks permanen untuk AI agent yang bekerja di repository ini. Baca sepenuhnya sebelum mulai coding.

## Project Overview

**Uteke** adalah local-first semantic memory engine untuk AI agents. Single Rust binary, fully offline, ~30ms recall. Tidak butuh API key, Docker, atau cloud service.

- **Repo:** `ajianaz/uteke` (remote GitHub), local di `/Users/mis-puragroup/development/riset-ai/uteke`
- **Versi:** 0.0.12
- **Lisensi:** Apache 2.0
- **Branch utama:** `develop` ( semua PR ke sini), `main` (release)

## Architecture

### Workspace Crates (3 crate)

| Crate | Path | Fungsi |
|-------|------|--------|
| `uteke-core` | `crates/uteke-core/` | Library — storage, embedding, vector search, FTS5, operations |
| `uteke-cli` | `crates/uteke-cli/` | CLI binary — clap commands, JSON output, server proxy |
| `uteke-server` | `crates/uteke-server/` | HTTP server — persistent daemon untuk fast agent access |

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

| Komponen | Teknologi | Detail |
|----------|-----------|--------|
| Vector Index | usearch v2.25.3 | Persistent HNSW, `RwLock` untuk concurrent reads |
| Full-Text Search | SQLite FTS5 | Virtual table, phrase + token-OR fallback |
| Hybrid Search | RRF (k=60) | Reciprocal Rank Fusion merges vector + FTS5 results |
| Storage | SQLite (rusqlite) | WAL mode, schema v2 |
| Embedding | EmbeddingGemma Q4 ONNX | 768d vectors, `Mutex` (ONNX tokenizer needs `&mut self`) |
| CLI | clap v4 | Standard Rust CLI |
| Server | actix-web | CORS enabled, ~42ms warm recall |

### Schema Versioning

- Tabel `schema_version` dengan integer counter
- Saat ini: **v2** (FTS5 migration)
- Auto-migration saat upgrade, zero data loss

---

## Critical Rules — WAJIB DIIKUTI

### 1. Selalu `cargo fmt` Sebelum Commit

CI menjalankan `cargo fmt --check` dan **akan gagal** kalau ada formatting issue. Satu spasi atau newline salah cukup untuk gagalkan PR.

```bash
# SELALU jalankan sebelum commit
cargo fmt
```

### 2. Jalankan Cora Review Lokal Sebelum Push

Cora CLI menemukan **bug real** di proyek ini (BM25 score selalu 0, RRF normalization salah, metadata hilang di server mode). Jangan tunggu CI.

```bash
cora review --base origin/develop --format text
```

Kalau Cora menemukan error-level issues, **fix dulu** baru push.

### 3. Clippy = Error

CI menjalankan `cargo clippy -- -D warnings`. Semua warning dianggap error.

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

### 4. Branch Protection Rules

- **Develop branch:** semua perubahan harus lewat PR
- **10 CI checks** harus pass: Build, Check, Clippy, Format, Test, Cora Review, CodeRabbit, Cargo Audit, Trivy FS Scan, GitGuardian
- **Tidak bisa** push langsung ke develop

### 5. Jangan Pernah `.unwrap()` di Production Code

Gunakan `.expect("pesan yang jelas")` atau pattern `if let` / `match`. `.unwrap()` tanpa context membuat debugging mustahil kalau panic.

```rust
// ❌ Jangan
memories.remove(0).unwrap()

// ✅ Benar
memories.remove(0).expect("guaranteed by prior count check")

// ✅ Lebih baik
if let Some(memory) = memories.into_iter().next() { ... }
```

### 6. Atomic File Writes

Untuk semua file I/O yang menulis data penting (index, config, model), gunakan pattern:

```rust
// Tulis ke .tmp dulu, lalu rename (atomic di POSIX)
let tmp_path = path.with_extension("tmp");
fs::write(&tmp_path, &data)?;
fs::rename(&tmp_path, &path)?;
```

### 7. SQLite-First Dual-Write Pattern

- `remember()`: Tulis ke SQLite **dulu**, baru vector index
- `forget()`: Acquire index lock **dulu**, baru SQLite delete
- Pattern ini mencegah inconsistency antara DB dan index

---

## Lessons Learned — Dari Pengalaman Nyata

### FTS5 + Vector Hybrid Search Non-Trivial

Menggabungkan dua sistem ranking itu tricky:
- **Vector cosine similarity:** range 0..1
- **BM25 (FTS5):** negatif unbounded (bisa -5, -10, dst)
- **Jangan clamp BM25 ke 0..1 langsung** — hancurkan ranking
- Gunakan **RRF (Reciprocal Rank Fusion)** — rank-based, tidak peduli scale asli
- Kalau perlu normalize ke 0..1, gunakan sigmoid: `1.0 / (1.0 + (-score).exp())`

### RwLock vs Mutex — Pilih Berdasarkan Workload

- `RwLock` untuk **read-heavy** (vector index: recall/search jauh lebih sering dari remember/forget)
- `Mutex` kalau operasi butuh `&mut self` (ONNX tokenizer)
- **Jangan asal ganti Mutex → RwLock.** Profile dulu.

### Score Normalization Harus Presisi

Bug terkecil di score calculation bisa membuat fitur tidak berfungsi sama sekali:
- `.min(1.0)` vs `.clamp(0.0, 1.0)` — beda besar kalau ada nilai negatif
- Selalu tulis **unit test** yang memverifikasi score range

### Server Mode = Hidden Surface Area

Kalau menambah parameter baru di CLI, **jangan lupa update server mode juga.** Bug #264: `--entity`, `--category`, `--meta` ditambahkan di CLI tapi lupa di server endpoint.

**Checklist kalau tambah CLI flag baru:**
1. Command module (`commands/remember.rs`)
2. Server endpoint (`commands/server.rs` — proxy body)
3. Server handler (`uteke-server/src/main.rs`)
4. API docs
5. CLI reference docs

### Metadata di JSON Blob — Post-Filter, Bukan SQL Filter

Entity, category, dan meta disimpan sebagai JSON di kolom `metadata`. Ini berarti:
- **Filtering dilakukan di Rust**, bukan SQL WHERE clause
- Tidak ada index pada field individual di dalam JSON
- Untuk dataset besar (>10K), pertimbangkan kolom terpisah

### Unit Tests Tidak Cukup — Stress Test Manual Wajib

Unit tests (107) tidak cover:
- Bulk insert 100+ memories (performance regression?)
- Concurrent access via server mode
- Unicode / special characters di content
- Schema migration dari DB versi lama
- Crash recovery (kill di tengah write)

Jalankan stress test manual setelah perubahan signifikan.

### Dokumentasi Cepat Outdated

CONTRIBUTING.md pernah bilang "2 crates" padahal sudah 3 sejak v0.0.4. Badge version tertinggal. Setelah merge ke develop, **selalu cek** apakah docs perlu update.

---

## Workflow yang Terbukti Berhasil

### Per-Issue Workflow

```
1. git checkout develop && git pull
2. git checkout -b <type>/<short-description>
3. Implementasi (baca modul terkait dulu)
4. cargo fmt && cargo clippy && cargo test
5. cora review --base origin/develop --format text  (local review)
6. Fix semua findings dari Cora
7. git add -A && git commit -m "type: description"
8. git push origin <branch>
9. gh pr create --base develop
10. Monitor CI (gh pr checks <number>)
11. Review PR comments (Cora, CodeRabbit)
12. Fix kalau ada findings baru
13. gh pr merge <number> --squash --delete-branch
14. Update docs kalau perlu
15. Pick next issue
```

### Branch Naming Convention

```
feat/<fitur-baru>
fix/<bug-yang-diperbaiki>
docs/<apa-yang-diupdate>
refactor/<apa-yang-di-refactor>
```

### Commit Message Convention

```
type: description (#issue-number)

type: feat, fix, docs, refactor, test, chore
```

Contoh:
```
feat: add FTS5 hybrid search with RRF (#250)
fix: BM25 score always returning 0.0
docs: update CLI reference for metadata flags
```

---

## Known Limitations

| Limitasi | Status | Detail |
|----------|--------|--------|
| usearch `ef` parameter tidak bisa di-set | External | usearch v2.25.3 Rust bindings tidak expose `ef` di `search()` |
| Embedder butuh `Mutex` | Architectural | ONNX tokenizer internal pakai `&mut self` |
| Metadata filtering post-filter | Design | Entity/category/meta di JSON blob, bukan SQL column |
| Consolidate O(n²) | Algorithm | Pairwise cosine, lambat di >1000 memories |
| FTS5-only mode score placeholder | Design | BM25 tidak bisa normalize ke 0..1, actual ranking via RRF |

---

## Reference Cepat

```bash
# Build
cargo build --workspace

# Test (107 unit tests)
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
