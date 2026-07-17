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
│  /stats /namespaces /room/* /mcp — CORS, ~42ms     │
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
| `uteke-mcp` | MCP JSON-RPC server — stdio + Streamable HTTP transport (#381) |

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
                    Entity/category filter (#667)
                    (pushed into core recall loop)
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


### Extract / Import / Export (v0.7.0)

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/extract` | POST | Write | LLM fact extraction + auto-store (1MB body limit) |
| `/import` | POST | Write | JSONL import with re-embedding (5MB body limit) |
| `/export` | GET | Read | JSONL export (optional `?namespace=` filter) |

### Maintenance & Monitoring (v0.7.0)

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/prune` | POST | Write | TTL-based deprecated memory cleanup |
| `/consolidate` | POST | Write | Near-duplicate merging |
| `/aging` | POST | Write | Memory lifecycle: status, preview, cleanup |
| `/importance` | POST | Write | Recalculate importance scores |
| `/orphans` | POST | Read | Find disconnected, low-importance memories |
| `/rebuild-backlinks` | POST | Write | Rebuild `referenced_by` from forward edges |

### Document Update (v0.7.0)

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/doc/update` | POST | Write | Partial document update with chunk rebuild |

### Memory Mutation (v0.7.4)

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/memory` | PUT | Write | Partial memory update (content, tags, metadata, importance, pinned, memory_type) |
| `/memory/pin` | POST | Write | Pin or unpin a memory by ID |
| `/memory/importance` | POST | Write | Set importance score (0.0–1.0) for a memory |

### Room ↔ Document Junction (v0.7.4)

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/room/document/list` | POST | Read | List document slugs linked to a room |
| `/room/document/add` | PUT | Write | Link a document to a room |
| `/room/document/remove` | DELETE | Write | Unlink a document from a room |
| `/doc/room/list` | POST | Read | List rooms linked to a document |

### Cross-Entity References (v0.7.4)

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/memory/doc-refs` | POST | Read | Get document slugs referenced by a memory |
| `/doc/mem-refs` | POST | Read | Get memory IDs that reference a document |

## Key Design Decisions

### RwLock for Vector Index (not Mutex)

Read-heavy workload: recall/search operations far outnumber remember/forget. Multiple concurrent recalls share a read lock. Embedder remains `Mutex` because ONNX tokenizer requires `&mut self` internally.

### Cross-Process File Lock (#543)

The RwLock provides intra-process concurrency only (multiple threads within a single `uteke-serve` process). For cross-process safety (multiple `uteke-serve` instances or CLI + server sharing the same store), a file-based lock (`fs2` crate) guards the SQLite database and usearch index files. The lock is acquired on store open and held until the process exits or the store is dropped. This prevents concurrent writes from corrupting the index or database when multiple processes access the same `~/.uteke/` directory.

### FTS5 + Vector Hybrid via RRF

Two systems with incompatible score scales (cosine 0..1 vs BM25 unbounded). Reciprocal Rank Fusion solves this by ranking based on position, not score magnitude. k=60 is the standard literature value.

### Graph-Augmented Reranking (#378)

The `graph` recall strategy layers graph signals on top of the RRF result.
`compute_graph_signals()` issues a single batched query over `memory_edges`
(derived from `[[slug]]` / `@tag` / `^id` auto-wiring, #346) and computes
per-memory density/authority counts. `rerank_with_graph()` then applies an
additive, log-scaled boost (`ln(1+x) * weight`) so well-connected memories
drift upward while isolated ones are untouched. The boost saturates quickly
(going 1→10 edges ≈ 100→1000 in lift), preventing hub dominance. The recall
cache is strategy-keyed, so `graph` entries never collide with `hybrid`/
`vector`/`fts5`.

### Atomic File Writes

All critical file I/O uses the `.tmp` + `rename` pattern. On POSIX filesystems, `rename` is atomic — a crash mid-write never leaves a corrupt file, only the old version.

### Schema Versioning

Integer counter in `schema_version` table. Migrations run automatically on upgrade. Currently at v15. Schema history: v4 rooms, v5 memory_tags junction, v6 content_type column, v7 knowledge graph, v8 `memory_edges` + `slug` (#346), v9 `timeline_events` table (#347), v10 `source` + `source_type` (#348), v11 documents + document_chunks (#406), v12 hierarchy (parent_id on documents, room tables added to SCHEMA constant), v13 global documents (namespace deprecated, author column, slug uniqueness), v14 `memory_type` added to FTS5 index (#662), v15 `room_documents` junction table for room↔document linking (#689). Zero data loss guaranteed.

### Rooms

Rooms group related memories with author attribution. Stored in `rooms` and `room_memories` tables (schema v4). Semantic room recall over-fetches via `recall_hybrid()`, then post-filters to room memory IDs. Room summaries use tag co-occurrence clustering — no LLM call required. Room documents group memories by `memory_type` into sections (Decisions, Facts, Procedures, etc).

Rooms can also be linked to documents via the `room_documents` junction table (schema v15, #689), enabling bidirectional room↔document associations. Documents reference memories via `[[doc-slug]]` wikilinks, which are auto-resolved to cross-entity edges (#691).

### Memory Types (#349)

Every memory has a `memory_type`: `fact`, `procedure`, `preference`, `decision`, `context`, `note`, `insight`, `reference`, `event`. Type is set explicitly via `--type` or auto-inferred from content patterns (questions → `context`, URLs → `reference`, dates → `event`, etc.).

### Timeline Events (#347)

The `timeline_events` table (schema v9) records an append-only audit log per memory: creation, updates, type changes, supersession, edge additions, pin/unpin. Events store metadata as JSON. Consolidation events attach to the OLD memory with `replaced_by` metadata.

### Salience + Recency (#352)

Dual-axis recall boost applied after the RRF merge:
- **Salience**: higher-scored by type weight (decision > insight > fact > note)
- **Recency**: exponential decay `exp(-age/τ)` where τ is a per-type time constant

Both are enabled by default (weight 0.1). Use `--no-salience` / `--no-recency` to disable. Previously opt-in (v0.7.3), now opt-out (#721). Config leaks are prevented by setting/resetting config around each query.

### Dream Cycle (#353)

The `dream` command runs a coordinated maintenance pipeline: lint → backlinks → dedup → contradict → orphans → compact → verify. Phase 4 (Contradict) scans for contradictory memories via tag overlap + embedding divergence (#720). Phases run in canonical dependency order regardless of user-supplied order. Individual phase errors are recorded but don't abort the pipeline. Namespace-scoped phases (lint, dedup, contradict, orphans, compact) only affect the target namespace; global phases (backlinks, verify) run across all namespaces.

### Citation & Source Attribution (#348)

The `source` and `source_type` columns (schema v10) track where a memory came from. Source types: `user`, `url`, `file`, `import`, `derived`, `system`, `unknown`. Set via `--source` / `--source-type` on remember or `set_source()` post-insert.

### MCP Transport (#381)

The `uteke-mcp` crate provides a shared JSON-RPC handler used by both:
- **stdio binary** (`uteke-mcp`) — for local agents (Claude Desktop, Cursor)
- **HTTP endpoint** (`POST /mcp` on `uteke-serve`) — for remote MCP clients

Protocol version: `2025-06-18` (Streamable HTTP spec). 1 MiB body limit enforced on HTTP endpoint. JSON-RPC 2.0 strict compliance (v0.6.7, #573/#576): tagged union responses, no notification response, Claude Code compatible.

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

In Docker, the data directory defaults to `/data` (set via `UTEKE_HOME`). The `slim` image variant does not bundle the embedding model — mount the model directory separately:
```bash
docker run -d --name uteke \
  -v uteke-data:/data \
  -v ./models/embeddinggemma-q4:/data/embeddinggemma-q4 \
  ghcr.io/codecoradev/uteke:slim
```

## Known Limitations

| Limitation | Status | Detail |
|-----------|--------|--------|
| usearch `ef` not configurable | External | usearch v2.25.3 Rust bindings don't expose `ef` in `search()` |
| Embedder requires Mutex | Architectural | ONNX tokenizer internally uses `&mut self` |
| Consolidate is O(n²) | Algorithm | Pairwise cosine — slow above 1K memories |
| FTS5-only score is placeholder | Design | BM25 can't normalize to 0..1; actual ranking via RRF |

## Document Engine (#406)

Full markdown content → SQLite (`documents` table), chunked summaries → embeddings (`document_chunks`).

### Schema (v11)

```sql
-- Full documents with unlimited content
documents: id, slug, title, content, namespace, tags, metadata,
           version, content_type, created_at, updated_at

-- Chunked sections with per-chunk embeddings
document_chunks: id, document_id, chunk_index, heading, content,
                 embedding BLOB, char_start, char_end, tags
```

### Chunking Pipeline

1. Document content → `chunk_markdown()` (#405) splits by headings
2. Each chunk → `embedder.embed()` creates section-level embedding
3. Chunks stored with heading, content, char offsets, embedding BLOB

Chunk size derived from `embedder.max_seq_len()` (#407): ~4 chars per token.

- ONNX (256 tokens): 1,024 chars/chunk
- OpenAI (8191 tokens): 32,764 chars/chunk

### Search Paths

1. Direct ID/slug → SQLite PK → full content
2. Semantic → embed query → ANN chunk search → resolve to document
3. FTS5 → full text across all document content
4. Graph → BFS from entity → connected documents

## Cosine Auto-Linking (#401)

After every `remember()`, the vector index is searched for the top-20 most similar memories:

- Cosine ≥ 0.80 → `similar_to` edge
- Cosine ≥ 0.92 → `possible_duplicate` edge

Edges are namespace-scoped (no cross-namespace links). Best-effort: errors logged, never fail `remember()`.

## Graph API (#408)

`GET /graph` returns all graph nodes, edges, and stats as JSON for visualization clients:

```json
{
  "nodes": [{ "id": "...", "label": "...", "entity_type": "..." }],
  "edges": [{ "source_id": "...", "target_id": "...", "relation": "..." }],
  "stats": { "node_count": N, "edge_count": N }
}
```
