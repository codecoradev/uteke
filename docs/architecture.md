---
title: Architecture
---

# Architecture

## System Overview

```
┌─────────────────────────────────────────────────────┐
│                    CLI (clap)                        │
│  uteke-cli crate — auto-routes to server if running │
├─────────────────────────────────────────────────────┤
│              HTTP API (uteke-serve)                  │
│  /health /remember /recall /search /list /forget    │
│  /stats /namespaces — CORS enabled, ~42ms recall    │
├─────────────────────────────────────────────────────┤
│                    Uteke API                         │
│          uteke-core crate (lib)                      │
├──────────┬──────────┬───────────┬────────────────────┤
│   ONNX   │  usearch │   FTS5    │      SQLite        │
│ Embedding│ Vector   │ Full-Text │   Metadata Store   │
│ (768d)   │ Index    │  Search   │    (rusqlite)      │
│          │ (HNSW)   │ (virtual  │                    │
│          │ RwLock   │   table)  │                    │
├──────────┴──────────┴───────────┴────────────────────┤
│              ~/.uteke/ (local storage)               │
│ uteke.db │ uteke_index.usearch │ embeddinggemma-q4/ │
└─────────────────────────────────────────────────────┘
```

## Workspace Crates

| Crate | Purpose |
|-------|---------|
| `uteke-core` | Library — storage, embedding, vector search, FTS5, operations |
| `uteke-cli` | CLI binary — clap commands, JSON output, server proxy |
| `uteke-server` | HTTP server — persistent daemon for fast agent access |

## Components

| Component | Technology | Detail |
|-----------|-----------|--------|
| Language | Rust (no unsafe) | Memory-safe, fast, single binary |
| Vector Index | usearch + RwLock | Persistent HNSW, concurrent reads via RwLock |
| Full-Text Search | SQLite FTS5 | Built-in, auto-created, phrase + token-OR fallback |
| Hybrid Search | RRF (k=60) | Merges vector + FTS5 via Reciprocal Rank Fusion |
| Storage | SQLite (rusqlite) | Embedded, zero-config, battle-tested |
| Embedding | EmbeddingGemma Q4 ONNX | 768d vectors, multilingual, downloaded on first run |
| Namespaces | SQLite column | Multi-agent isolation, zero overhead |
| Tiered Memory | Access tracking | Hot/Warm/Cold scoring boost |
| CLI | clap | Standard Rust CLI |

## How It Works

1. **`remember`** — Text is embedded into a 768d vector via ONNX → stored in SQLite + indexed in usearch
2. **`recall`** — Query is embedded → usearch finds nearest neighbors + FTS5 finds keyword matches → RRF merges both result sets → hot memories get +0.1 score boost → returns ranked results
3. **`search`** — SQLite FTS5 keyword search (phrase match + token-OR fallback, scoped to namespace)
4. **`forget`** — Incremental delete from usearch + SQLite (no rebuild)
5. Everything lives in `~/.uteke/` — fully local, fully yours

## Data Flow

### Write Path (remember)

```
Content → ONNX embed (Mutex) → 768d vector
                                ↓
                    SQLite INSERT (metadata + vector)
                                ↓
                    usearch INSERT (RwLock write)
                                ↓
                    usearch SAVE (atomic: .tmp + rename)
```

SQLite-first dual-write: metadata persisted before index. If index write fails, data is still in SQLite and can be recovered via `uteke repair`.

### Read Path (recall)

```
Query → Recall Cache check (TTL 5min, LRU 256)
         ↓ miss
       ONNX embed (Mutex) → 768d vector
                                ↓
              ┌─────────────────┴──────────────────┐
              ↓                                     ↓
     usearch SEARCH (RwLock read)         FTS5 SEARCH (SQLite)
     → top-k vectors + scores             → top-k rows + BM25
              ↓                                     ↓
              └───────────── RRF Merge ─────────────┘
                            ↓
                    Tier boost (+0.1 hot)
                            ↓
                    Entity/category filter
                            ↓
                    Ranked results
```

### Delete Path (forget)

```
ID → usearch REMOVE (RwLock write, acquired first)
                           ↓
              SQLite DELETE (by specific ID)
                           ↓
              usearch SAVE (atomic)
```

Index lock acquired **before** SQLite delete — narrows the inconsistency window.

## Key Design Decisions

### RwLock for Vector Index (not Mutex)

Read-heavy workload: recall/search operations far outnumber remember/forget. Multiple concurrent recalls share a read lock. Embedder remains `Mutex` because ONNX tokenizer requires `&mut self` internally.

### FTS5 + Vector Hybrid via RRF

Two systems with incompatible score scales (cosine 0..1 vs BM25 unbounded). Reciprocal Rank Fusion solves this by ranking based on position, not score magnitude. k=60 is the standard literature value.

### Atomic File Writes

All critical file I/O uses the `.tmp` + `rename` pattern. On POSIX filesystems, `rename` is atomic — a crash mid-write never leaves a corrupt file, only the old version.

### Schema Versioning

Integer counter in `schema_version` table. Migrations run automatically on upgrade. Currently at v5 (memory_tags junction table). Zero data loss guaranteed.

### Rooms

Rooms group related memories with author attribution. Stored in `rooms` and `room_memories` tables (schema v4). Semantic room recall over-fetches via `recall_hybrid()`, then post-filters to room memory IDs. Room summaries use tag co-occurrence clustering — no LLM call required. Room documents group memories by `memory_type` into sections (Decisions, Facts, Procedures, etc).

## Performance

| Mode | Recall | Setup |
|------|--------|-------|
| **Library (Rust)** | **~30ms** | In-process, no startup |
| **Server (HTTP)** | **~42ms** | One-time ~2s init |
| **CLI (binary)** | **~3s** | Per-invocation (model load) |

### CLI vs Server

| Metric | CLI (cold) | Server (warm) | Speedup |
|--------|-----------|---------------|---------|
| **Insert 100** | ~316s (0.3/s) | 7.7s (13/s) | **41x** |
| **Recall (avg)** | 3,158ms | 42ms | **75x** |
| **Search (avg)** | 3,158ms | 9ms | **367x** |

### Scaling (warm server)

| Data Size | Recall (avg) | Search (avg) |
|-----------|-------------|-------------|
| 100 memories | 42ms | 9ms |
| 1,000 memories | 49ms | 13ms |
| 10,000 memories* | ~55ms (est.) | ~20ms (est.) |

*\*10K estimated — HNSW vector search is O(log n)*

Benchmarked on Oracle Cloud ARM (Ampere Altra), CPU-only, no GPU.

## File Structure

```
~/.uteke/
├── uteke.db                    # SQLite (memories + metadata + FTS5)
├── uteke_index.usearch         # Persistent HNSW vector index
├── uteke_index.keys            # Index key mapping (atomic save)
├── embeddinggemma-q4/           # Local ONNX embedding model (~188MB)
│   └── onnx/                    # model_q4.onnx + model_q4.onnx_data
└── logs/
    ├── uteke.log               # Current log
    └── uteke.log.YYYY-MM-DD    # Rotated logs
```

## Known Limitations

| Limitation | Status | Detail |
|-----------|--------|--------|
| usearch `ef` not configurable | External | usearch v2.25.3 Rust bindings don't expose `ef` in `search()` |
| Embedder requires Mutex | Architectural | ONNX tokenizer internally uses `&mut self` |
| Consolidate is O(n²) | Algorithm | Pairwise cosine — slow above 1K memories |
| FTS5-only score is placeholder | Design | BM25 can't normalize to 0..1; actual ranking via RRF |
