# 02 — Technical Architecture v2

**Uteke v2 · Library-First Approach**


---

## Tech Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Language  | **Rust** | Performance, safety, single binary, no runtime deps |
| Vector search | **HNSW** (hnsw crate) | Fast local search, no server needed |
| Storage   | **SQLite** (rusqlite) | Reliable, embedded, zero-config, single file |
| Embeddings | **candle-transformers** | Local inference, no API key, runs offline |
| CLI framework | **clap** | Standard Rust CLI, derive macros |
| Serialization | **serde + serde_json** | JSON I/O for CLI output |
| Logging   | **tracing** | Structured logging, standard Rust ecosystem |


---

## Architecture Tree

```
uteke/
├── Cargo.toml
├── src/
│   ├── lib.rs              # Public API: Uteke struct
│   ├── memory/
│   │   ├── mod.rs
│   │   ├── store.rs        # SQLite read/write
│   │   ├── vector.rs       # HNSW index management
│   │   └── types.rs        # Memory, SearchResult structs
│   ├── context/
│   │   ├── mod.rs
│   │   └── builder.rs      # Context assembly for LLM prompts
│   ├── embed/
│   │   ├── mod.rs
│   │   └── engine.rs       # candle-transformers embedding
│   └── cli/
│       ├── mod.rs
│       └── main.rs         # clap CLI entry point
├── tests/
│   ├── integration_test.rs
│   └── fixtures/
└── benches/
    └── retrieval_bench.rs
```


---

## Public Rust API

```rust
/// Uteke — Local-first AI memory engine
pub struct Uteke {
    db: SqlitePool,
    index: HnswIndex,
    embedder: EmbeddingEngine,
}

impl Uteke {
    /// Open or create a memory store at the given path
    pub fn open(path: &Path) -> Result<Self>;

    /// Store a new memory with optional tags
    pub fn remember(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<Value>,
    ) -> Result<MemoryId>;

    /// Retrieve relevant memories for a query
    pub fn recall(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
    ) -> Result<Vec<SearchResult>>;

    /// Full-text search (exact keyword matches)
    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>>;

    /// Delete a memory by ID
    pub fn forget(&self, id: MemoryId) -> Result<()>;

    /// List all memories, optionally filtered by tag
    pub fn list(
        &self,
        tag: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Memory>>;

    /// Get a single memory by ID
    pub fn get(&self, id: MemoryId) -> Result<Memory>;
}
```


---

## CLI Design

```
uteke remember "Rust async uses green threads via Tokio runtime" --tags rust,async
uteke recall "how does rust handle async" --limit 5
uteke search "green threads" --limit 10
uteke list --tag rust --limit 20
uteke forget abc123
uteke stats

# JSON output for programmatic use
uteke recall "async patterns" --json
```

| Command | Description | Output |
|---------|-------------|--------|
| `remember` | Store a memory | Memory ID |
| `recall` | Semantic search (embeddings) | Ranked results |
| `search` | Full-text keyword search | Ranked results |
| `list`  | List memories with optional tag filter | Paginated list |
| `forget` | Delete a memory | Confirmation |
| `stats` | Store statistics | Count, size, tags summary |


---

## SQLite Schema

```sql
CREATE TABLE IF NOT EXISTS memories (
    id          TEXT PRIMARY KEY,  -- UUID v4
    content     TEXT NOT NULL,
    embedding   BLOB,             -- Serialized f32 vector
    tags        TEXT,             -- JSON array: ["rust","async"]
    metadata    TEXT,             -- JSON object
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_memories_tags ON memories(tags);
CREATE INDEX idx_memories_created ON memories(created_at);
```


---

## Storage Layout

```
~/.uteke/
├── uteke.db           # SQLite database
├── uteke.db-wal       # WAL journal
├── config.toml        # User preferences (optional)
└── models/
    └── minilm-l6-v2/  # Cached embedding model (~90MB)
```


---

## Embedding Strategy

| Version | Model | Dimensions | Size | Speed | Notes |
|---------|-------|------------|------|-------|-------|
| **v2 MVP** | MiniLM-L6-v2 | 384d       | \~90MB | \~5ms/embed | All-MiniLM-L6-v2 via candle, runs local |
| **v2.1** | MiniLM-L12-v2 | 384d       | \~120MB | \~8ms/embed | Better quality, same dimensions |
| **v2.2** | Remote (opt-in) | Variable   | 0MB local | \~100ms | OpenAI/Voyage API for better quality |

**v2 MVP uses candle-transformers** — pure Rust/CUDA inference, no Python, no API key, works offline. The 90MB model downloads on first use and is cached locally.


---

## Performance Targets

| Metric | v2 Target | Measurement |
|--------|-----------|-------------|
| Binary size | < 2MB     | `strip` + release build |
| Cold start | < 200ms   | `time uteke stats` |
| Embed single memory | < 10ms    | candle on CPU |
| Recall (10k memories) | < 50ms    | HNSW search, top-5 |
| Recall (100k memories) | < 200ms   | HNSW scales logarithmically |
| Store single memory | < 15ms    | SQLite write + embed + index |
| RAM idle | < 10MB    | After loading index |
| RAM peak (embed) | < 150MB   | candle model in memory |


---

## Hermes Integration Pattern

Hermes agents (Python) interact with Uteke via CLI subprocess:

```python
import subprocess, json

class UtekeMemory:
    def __init__(self, store_path="~/.uteke"):
        self.store = store_path

    def recall(self, query, limit=5, tags=None):
        cmd = ["uteke", "recall", query, "--json", f"--limit={limit}"]
        if tags:
            cmd.extend(f"--tags={','.join(tags)}")
        result = subprocess.run(cmd, capture_output=True, text=True)
        return json.loads(result.stdout)

    def remember(self, content, tags=None):
        cmd = ["uteke", "remember", content]
        if tags:
            cmd.extend(f"--tags={','.join(tags)}")
        subprocess.run(cmd, capture_output=True, text=True)
```

No SDK needed. No HTTP server. Just `uteke` in PATH.

## EMBEDDING DECISION (29 May 2026 - CONFIRMED)

**Model: EmbeddingGemma ONNX Q4 (768d)** - Sudah proven di Hermes ecosystem.

| Property | Value |
|----------|-------|
| Model    | EmbeddingGemma Q4 quantized ONNX |
| Path     | /opt/data/models/embeddinggemma/onnx/model_q4.onnx |
| Size     | 188 MB |
| Dimensions | 768d (native), MRL-safe (512/256/128) |
| Runtime  | onnxruntime + tokenizers (NO optimum/transformers) |
| Cold start | 2-5s  |
| Embed speed | \~200ms per text |
| Peak memory | \~27 MB |
| Benchmark | 3ms semantic search, proven 941 nodes in production |

### Integration Pattern (Rust)

**v2: Python subprocess FFI** - Call existing embeddinggemma.py via subprocess. Fastest to implement, reuses proven pipeline. \~1 day effort.

**v2.1: onnxruntime-rs** - Native Rust ONNX inference. No Python dependency. Migrate when onnxruntime-rs API stabilizes.

### Why EmbeddingGemma?

* Already deployed and battle-tested in Hermes fleet
* 768d quality - much better than 384d alternatives
* Q4 quantized = 188MB, fits in memory easily
* Zero API cost, fully local
* MRL-safe: can truncate to 512d/256d if memory constrained