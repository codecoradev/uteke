# Technical Specification

## Tech Stack

| Component | Technology | Why |
|-----------|------------|-----|
| Core Language | Rust       | Zero-cost abstractions, memory safety, single binary |
| Vector Search | HNSW (hnsw crate) | Sub-ms search, 768d embeddings, no external dependency |
| Structured Storage | SQLite (rusqlite) | Embedded, ACID, zero config, battle-tested |
| Graph Storage | SQLite adjacency table | Simple, fast, no separate DB needed |
| CRDT Sync | automerge-rs | Battle-tested CRDT library, Rust native |
| Embedding | Local (embeddings-fast-rs) or API | Local = offline capable, API = higher quality |
| Serialization | serde + serde_json | Standard Rust ecosystem |
| CLI       | clap       | Best Rust CLI framework |
| HTTP Server | axum       | Fast, async, tower-based |
| WebSocket | tokio-tungstenite | Async WebSocket for real-time sync |
| Encryption | aes-gcm + ring | E2E encryption for cloud sync |

## Storage Architecture

### .uteke/ Directory Structure

```
.uteke/
  ├── memory.db          # SQLite: structured, graph, temporal data
  ├── vectors.hnsw       # HNSW index file (memory-mapped)
  ├── vectors.data       # Raw vector data (mmap)
  ├── wal/               # Write-ahead log for crash recovery
  ├── config.toml        # User preferences
  └── sync.lock          # CRDT sync state
```

### SQLite Schema (memory.db)

```sql
-- Structured memories
CREATE TABLE memories (
    id TEXT PRIMARY KEY,
    agent TEXT NOT NULL,
    category TEXT NOT NULL,
    title TEXT,
    content TEXT,
    metadata JSON,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    access_count INTEGER DEFAULT 0,
    importance REAL DEFAULT 0.5
);

-- Graph edges (entity relationships)
CREATE TABLE edges (
    source TEXT NOT NULL,
    target TEXT NOT NULL,
    relation TEXT NOT NULL,
    weight REAL DEFAULT 1.0,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (source, target, relation)
);

-- Temporal events (time-indexed)
CREATE TABLE events (
    id TEXT PRIMARY KEY,
    agent TEXT NOT NULL,
    event_type TEXT NOT NULL,
    data JSON,
    timestamp INTEGER NOT NULL
);

-- Device registry
CREATE TABLE devices (
    id TEXT PRIMARY KEY,
    name TEXT,
    last_sync INTEGER,
    crdt_state BLOB
);
```

## API Design

### Embedded API (library mode)

```rust
use uteke::{Uteke, Memory, Query};

// Initialize (creates .uteke/ in cwd)
let uteke = Uteke::open(".")?;

// Store a memory
uteke.remember(Memory {
    agent: "claude-code",
    category: "semantic",
    title: "API design decision",
    content: "Use REST not GraphQL for public API...",
    metadata: json!({"project": "uteke", "confidence": 0.9}),
})?;

// Recall relevant context
let results = uteke.recall(Query {
    agent: "claude-code",
    query: "API architecture decisions",
    limit: 10,
    min_relevance: 0.7,
})?;

// Smart context for LLM
let context = uteke.build_context(
    agent: "claude-code",
    task: "Implement REST API endpoint",
    token_budget: 8000,
)?;
```

### REST API (server mode)

```
POST /api/v1/memories       — Store memory
GET  /api/v1/memories/:id    — Get memory
POST /api/v1/recall          — Recall relevant memories
POST /api/v1/context/build   — Build smart context for LLM
GET  /api/v1/health          — Health check
POST /api/v1/sync/push       — Push changes
POST /api/v1/sync/pull       — Pull changes
GET  /api/v1/devices         — List registered devices
```

## Performance Targets

| Metric | Target | Benchmark Method |
|--------|--------|------------------|
| Cold start | < 100ms | Time from open() to first query |
| Memory insert | < 1ms  | Single memory store |
| Vector search (10K) | < 5ms  | Top-10 recall, 768d |
| Vector search (100K) | < 15ms | Top-10 recall, 768d |
| Context build | < 50ms | Full pipeline (retrieve+rank+compress) |
| RAM idle | < 15MB | Empty database   |
| RAM (10K vectors) | < 50MB | After 10K 768d embeddings |
| RAM (100K vectors) | < 200MB | After 100K 768d embeddings |
| Binary size | < 5MB  | Stripped release build |
| Disk per 10K memories | < 20MB | SQLite + HNSW + vectors |

## Cross-Compilation Targets

| Platform | Architecture | Status |
|----------|--------------|--------|
| Linux    | x86_64, aarch64 | P0     |
| macOS    | x86_64, aarch64 (Apple Silicon) | P0     |
| Windows  | x86_64, aarch64 | P1     |
| WASM     | wasm32       | P2 (for browser extension) |