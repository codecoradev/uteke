# Uteke v2 Blueprint — Full Document

> Source: https://docs.azfirazka.com/s/eabdb09a-d6ed-480b-b30a-d7e304e95876
> Date: 29 May 2026
> Author: Anaz S. Aji
> BMAD Validation: 7.0/10 GO

---

## Executive Summary

**Uteke** is a local-first AI memory engine written in Rust. Library-first approach — solve personal problem first, share second.

**Core concept:** Store, search, retrieve knowledge for AI agents without any cloud dependency.

- **Effort:** ~100-130 hours, 4-6 weeks part-time
- **Stack:** Rust + SQLite + HNSW + EmbeddingGemma ONNX Q4 (768d)
- **License:** MIT
- **Success criterion:** "Do I use it every day?"

---

## 01 — Problem Statement & Personal Use Case

### The Three Frustrations

1. **Memory Loss Per Session** — Every new LLM session starts from zero. Context rebuilt every time.
2. **Context Window Limitation** — Token limits mean decisions get dropped.
3. **Cross-Tool Fragmentation** — Claude Code, ChatGPT, Gemini — each has its own context. Knowledge siloed.

### Personal Use Cases

**UC1: Hermes Fleet Memory (HIGH)**
- AI agents operate independently, no memory of previous work
- `uteke recall --tags "hermes,deployment" --last 7d`
- Fleet shares common memory store via filesystem

**UC2: Personal Project Context (HIGH)**
- BOND, Gofin, MikroSaaS — scattered knowledge
- Uteke becomes structured persistent memory

**UC3: Research Knowledge Persistence (MEDIUM)**
- `uteke remember --tags "rust,async" "Tokio's spawn_blocking is for CPU-bound work"`

### Library-First Advantage

| Problem | Full Product | Library-First |
|---------|-------------|---------------|
| Memory loss | Build API, wait feedback | Use it today |
| Context limits | Service | Local CLI |
| Fragmentation | Build integrations | Shell out to `uteke` |
| Dev speed | 6+ months | 4-6 weeks |
| Risk | High investment | Ship → validate → iterate |

---

## 02 — Technical Architecture v2

### Tech Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Language | **Rust** | Performance, safety, single binary |
| Vector search | **HNSW** (hnsw crate) | Fast local search, no server |
| Storage | **SQLite** (rusqlite) | Embedded, zero-config |
| Embeddings | **EmbeddingGemma ONNX Q4** | Proven in Hermes, 768d, 188MB |
| CLI framework | **clap** | Standard Rust CLI |
| Serialization | **serde + serde_json** | JSON I/O |
| Logging | **tracing** | Structured logging |

### Architecture Tree

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
│   │   └── engine.rs       # Embedding engine
│   └── cli/
│       ├── mod.rs
│       └── main.rs         # clap CLI entry point
├── tests/
│   ├── integration_test.rs
│   └── fixtures/
└── benches/
    └── retrieval_bench.rs
```

### Public Rust API

```rust
pub struct Uteke {
    db: SqlitePool,
    index: HnswIndex,
    embedder: EmbeddingEngine,
}

impl Uteke {
    pub fn open(path: &Path) -> Result<Self>;
    pub fn remember(&self, content: &str, tags: &[&str], metadata: Option<Value>) -> Result<MemoryId>;
    pub fn recall(&self, query: &str, limit: usize, tags_filter: Option<&[&str]>) -> Result<Vec<SearchResult>>;
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>>;
    pub fn forget(&self, id: MemoryId) -> Result<()>;
    pub fn list(&self, tag: Option<&str>, limit: usize, offset: usize) -> Result<Vec<Memory>>;
    pub fn get(&self, id: MemoryId) -> Result<Memory>;
}
```

### CLI Design

```
uteke remember "..." --tags rust,async
uteke recall "..." --limit 5
uteke search "green threads" --limit 10
uteke list --tag rust --limit 20
uteke forget abc123
uteke stats
uteke recall "async patterns" --json
```

### SQLite Schema

```sql
CREATE TABLE IF NOT EXISTS memories (
    id          TEXT PRIMARY KEY,
    content     TEXT NOT NULL,
    embedding   BLOB,
    tags        TEXT,
    metadata    TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_memories_tags ON memories(tags);
CREATE INDEX idx_memories_created ON memories(created_at);
```

### Storage Layout

```
~/.uteke/
├── uteke.db
├── uteke.db-wal
├── config.toml
└── models/
    └── embeddinggemma/
        └── model_q4.onnx   # 188MB
```

### Embedding Decision (CONFIRMED 29 May 2026)

**Model: EmbeddingGemma ONNX Q4 (768d)**

| Property | Value |
|----------|-------|
| Path | /opt/data/models/embeddinggemma/onnx/model_q4.onnx |
| Size | 188 MB |
| Dimensions | 768d (native), MRL-safe |
| Runtime | onnxruntime + tokenizers |
| Cold start | 2-5s |
| Embed speed | ~200ms per text |
| Peak memory | ~27 MB |
| Benchmark | 3ms semantic search, 941 nodes in production |

**Integration Pattern:**
- **v2:** Python subprocess FFI (call embeddinggemma.py)
- **v2.1:** onnxruntime-rs native Rust

### Performance Targets

| Metric | Target |
|--------|--------|
| Binary size | < 2MB |
| Cold start | < 200ms |
| Embed single | < 10ms |
| Recall (10k) | < 50ms |
| Recall (100k) | < 200ms |
| Store single | < 15ms |
| RAM idle | < 10MB |
| RAM peak | < 150MB |

### Hermes Integration

```python
import subprocess, json

class UtekeMemory:
    def recall(self, query, limit=5, tags=None):
        cmd = ["uteke", "recall", query, "--json", f"--limit={limit}"]
        if tags:
            cmd.extend(f"--tags={','.join(tags)}")
        result = subprocess.run(cmd, capture_output=True, text=True)
        return json.loads(result.stdout)
```

---

## 03 — Development Plan v2 (4-6 Weeks)

### Week 1-2: Core Engine

| Day | Task | Deliverable |
|-----|------|-------------|
| 1-2 | Project setup | Cargo workspace, CI, test framework |
| 3-4 | Memory struct, SQLite store | CRUD operations |
| 5-6 | HNSW integration | Vector search |
| 7-8 | Embedding engine | Local text embedding |
| 9-10 | Wire everything | End-to-end remember→recall |
| 11-14 | Tests, edge cases, benchmarks | >80% coverage |

**Exit:** `Uteke::open() → remember() → recall()` works end-to-end.

### Week 3-4: CLI + Polish

| Day | Task | Deliverable |
|-----|------|-------------|
| 15-16 | clap CLI | All commands functional |
| 17-18 | JSON output, error formatting | Scriptable |
| 19-20 | Tags, config, model download UX | First-run experience |
| 21-22 | Error handling, logging | Robust CLI |
| 23-26 | Integration tests, docs | Release-ready |

**Exit:** `cargo install uteke` works without reading source.

### Week 5-6: OSS Release + Hermes Integration

| Day | Task | Deliverable |
|-----|------|-------------|
| 27-28 | GitHub repo, CI/CD | Green builds |
| 29-30 | Documentation | Complete docs |
| 31-32 | Hermes integration | Real agent test |
| 33-34 | Launch prep | HN post, social |
| 35-36 | Buffer | Ship with confidence |

**Exit:** Public repo, passing CI, README with demo, Hermes using it.

### NOT in v2 (Deferred)

- SDK bindings (Python/JS) → CLI subprocess sufficient
- VS Code extension
- Sync engine
- REST API server
- Authentication
- Cloud storage
- Knowledge graph
- Billing

### Confirmed Decisions (29 May 2026)

1. **Rust langsung** — No Python prototype spike
2. **EmbeddingGemma ONNX Q4 (768d)** — Already proven, no evaluation needed

---

## 04 — Open Source Strategy & Community

- **License:** MIT — maximum adoption
- **Brand:** Uteke (Swahili: "to remember")
- **Tagline:** "Local-first memory for AI agents"
- **Domain:** uteke.dev
- **Repo:** github.com/ajianaz/uteke

### Launch Phases

1. **Soft Launch (Week 5-6)** — GitHub + Reddit + HN + Twitter
2. **Content Marketing (Month 2-3)** — Blog posts, video walkthrough
3. **Community Building (Month 3-6)** — PRs, GitHub Discussions

---

## 05 — Future Roadmap (Demand-Gated)

**Philosophy:** "Don't build it until someone asks for it twice."

### Phase A (100+ stars or 3 months daily use)
- v2.1: Better embeddings, streaming, dedup, import/export
- v2.2: Remote embedding (OpenAI/Voyage opt-in)
- v2.3: Context builder for LLM prompts

### Phase B (500+ stars)
- v3.0: Python SDK (PyO3), Node.js SDK (napi-rs), extension system
- v3.1: File-based sync (CRDT)
- v3.2: Editor integration (VS Code, Neovim)

### Phase C (1000+ stars or funding)
- v4.0: Team features, cloud sync (opt-in)
- v4.1: Freemium monetization
- v4.2: Auto-summarization, knowledge graph

---

## 06 — Risk Assessment v2

**v1 average risk: ~7/10 → v2 average risk: ~3/10**

| Risk | v1 | v2 | Why |
|------|----|----|-----|
| Scope creep | CRITICAL | MEDIUM | Hard boundaries |
| Low adoption | HIGH | LOW | Personal use validates |
| Incumbent competition | HIGH | LOW | No direct "local Rust" competitor |
| Solo burnout | HIGH | LOW | 100h not 400+h |
| Monetization | HIGH | NONE | Not a product |

### Kill Gates

1. **Week 1:** Embedding feasibility
2. **Week 2:** Core engine works (remember→recall)
3. **Week 6:** Personal utility (do I use it daily?)

### Competitor Landscape

- MemGPT: Cloud, Python, complex → Uteke is local, Rust, CLI
- ChromaDB: Python, needs server → Uteke is zero-dep single binary
- Zep: Cloud, paid → Uteke is free, offline
- Obsidian: Note-taking, no semantic search

---

## BMAD Round 2 Validation

| Stage | Score | Verdict |
|-------|-------|---------|
| BMAD 6-Persona (v1) | 6.5/10 | CONDITIONAL GO |
| CFO Round 1 (v1) | 5.0/10 | ⬇️ DOWNGRADE |
| Bad Sector Round 1 (v1) | 5.0/10 | ⬇️ CONFIRMED |
| **Revision v1→v2** | — | Library-first |
| CFO Round 2 (v2) | **7.5/10** | **✅ PROCEED** |
| Bad Sector Round 2 (v2) | **6.5/10** | **✅ PROCEED** |
| **FINAL** | **7.0/10** | **✅ GO** |

---

*"Dogfooding isn't a strategy. It's a prerequisite."*
