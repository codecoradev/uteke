# Product Vision & Architecture

## Vision Statement

"Make every AI agent exponentially productive by giving it a brain that remembers, reasons, and synchronizes — across sessions, devices, and teams."

## Design Principles


1. **Local-First** — Data stays on user's device by default. No cloud dependency.
2. **Zero Config** — Works out of the box. Git sync = zero infrastructure.
3. **Embedded** — 2MB binary, no runtime. Embeds in any editor or app.
4. **Multi-Framework** — Works with Cursor, Claude Code, Copilot, any AI tool.
5. **Performant** — <5ms retrieval, <15MB RAM idle. Feels invisible.

## Architecture Overview

### 4-Layer Architecture

```
LAYER 4: Integration
  VS Code Extension | Cursor Plugin | CLI Tool | SDK (Rust/Py/Go/TS) | REST API
                    |
LAYER 3: Core Engine (Rust)
  +-- Memory Engine: Vector (HNSW) + Structured (SQLite) + Graph + Temporal
  +-- Context Manager: Retrieve + Rank + Compress + Token Budget
  +-- Sync Engine: CRDT + Git Adapter + WebSocket + Encryption
                    |
LAYER 2: Storage
  SQLite (local) | HNSW Index | WAL Log | .uteke/ directory
                    |
LAYER 1: Sync Targets
  Git Repository (FREE) | Self-Hosted Server (Docker) | Managed Cloud (future)
```

### Core Module: Memory Engine

Fungsi: Menyimpan dan mengambil semua jenis knowledge agent.

| Memory Type | Storage | Use Case |
|-------------|---------|----------|
| **Semantic** | HNSW vector index (768d) | "Apa yang mirip dengan ini?" |
| **Structured** | SQLite (typed schema) | "Berapa budget Q1?" |
| **Graph**   | Adjacency list in SQLite | "Tool apa yang sering dipakai bareng?" |
| **Temporal** | Time-indexed SQLite rows | "Apa yang berubah sejak minggu lalu?" |

### Core Module: Context Manager

Fungsi: Mengisi context window dengan data yang paling relevant.

Pipeline:


1. **Retrieval** — Hybrid search: vector similarity + structured query + graph traversal
2. **Ranking** — Score berdasarkan relevance, recency, frequency, user priority
3. **Compression** — Hapus duplikat, merge overlapping context, summarize verbose entries
4. **Token Budget** — Sesuaikan output dengan context window limit model
5. **Priority Queue** — Pastikan critical context (system config, active task) selalu masuk

### Core Module: Sync Engine

Fungsi: Menjaga konsistensi data antar device.

| Sync Method | Speed | Complexity | Infrastructure |
|-------------|-------|------------|----------------|
| **Git-based** | Async (push/pull) | Low        | Git repo (existing) |
| **Self-hosted** | Real-time (WebSocket) | Medium     | Docker container |
| **Cloud**   | Real-time | Zero       | SaaS (managed) |

CRDT (Conflict-free Replicated Data Types) memastikan sync tanpa konflik:

* Device A edit v2, Device B edit v3 (diverged) → auto-merge → v4
* Last-Write-Wins untuk field yang sama
* Union merge untuk collections
* No manual conflict resolution needed

## Data Flow

```
Developer code in IDE
    → Uteke captures context (file changes, terminal output, LLM calls)
    → Embed (local model or API) → vector + metadata
    → Store (SQLite + HNSW index in .uteke/)
    → On next session: Agent queries Uteke → relevant memory retrieved
    → Context window filled with 95% relevant data
    → Exponential productivity gain
```

## Integration Points

| Integration | Method | Priority |
|-------------|--------|----------|
| VS Code     | Extension API | P0 (launch) |
| Cursor      | Same as VS Code extension | P0 (launch) |
| CLI         | Binary with pipe support | P1       |
| Python SDK  | PyO3 bindings | P1       |
| Go SDK      | CGo bindings | P2       |
| REST API    | HTTP server mode | P2       |