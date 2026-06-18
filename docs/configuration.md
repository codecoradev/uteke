---
title: Configuration
---

# Configuration

Uteke supports `uteke.toml` configuration with layered resolution.

## Resolution Order

Uteke searches for config in this order. Last match wins (highest priority):

1. **(built-in defaults)** — Hardcoded defaults
2. **`~/.uteke/uteke.toml`** — Global user-level config
3. **`.uteke/uteke.toml`** — Project-level (in current working directory)

Config file path is auto-resolved (no `--config` flag). Layered merge: each file overlays the previous, with field-level granularity (only keys explicitly present override).

## Config File Format

```toml
# uteke.toml

[store]
# Store location (default: ~/.uteke)
path = "~/.uteke"

# Default namespace (default: "default")
namespace = "default"

[logging]
# Log level: trace, debug, info, warn, error
level = "info"

# Optional log file path. Empty = stderr only.
# file = ""

[server]
# Enable CLI auto-routing to server
enabled = false

# Server host
host = "127.0.0.1"

# Server port
port = 8767
```

## Server Mode

When `[server] enabled = true`, the CLI automatically routes commands through the running HTTP server:

```bash
# Start server
uteke-serve --port 8767

# CLI commands now route via HTTP (21ms vs 980ms cold start)
uteke recall "what was that context?"
uteke remember "New finding" --tags research
uteke stats
```

If the server is not running, CLI falls back to local store automatically.

| Setting | Default | Description |
|---------|---------|-------------|
| `enabled` | false | Enable CLI→server routing |
| `host` | 127.0.0.1 | Server bind address |
| `port` | 8767 | Server port |

## Embedding Backend

Configure the embedding backend. Three backends are supported:

- **`onnx`** (default) — fully offline, EmbeddingGemma Q4, 768d. Zero API keys, zero network.
- **`openai`** — OpenAI `text-embedding-3-small` (1536d) or `text-embedding-3-large` (3072d). Requires API key.
- **`ollama`** — local Ollama server with models like `nomic-embed-text` (768d) or `mxbai-embed-large` (1024d). No API key, runs on `http://localhost:11434`.

```toml
[embedding]
backend = "onnx"              # onnx | openai | ollama
model = "embeddinggemma-q4"   # backend-specific
max_seq_length = 256
api_key = ""                  # OpenAI only (or use UTEKE_EMBEDDING_API_KEY)
base_url = ""                 # custom endpoint (Azure OpenAI, Ollama URL, proxy)
dims = 0                     # 0 = use model default (override only if you know)
```

| Setting | Default | Description |
|---------|---------|-------------|
| `backend` | `onnx` | `onnx`, `openai`, or `ollama` |
| `model` | `embeddinggemma-q4` | Backend-specific model name |
| `max_seq_length` | `256` | Max tokens per input |
| `api_key` | `""` | OpenAI API key (ONNX/Ollama ignore) |
| `base_url` | `""` | Custom endpoint. Empty = backend default |
| `dims` | `0` | Force dims. 0 = backend/model default |

### Backend-specific defaults

When you set `backend = "openai"` or `"ollama"` and leave `model`/`base_url`/`dims` empty, uteke picks:

| Backend | Model | Base URL | Dims |
|---|---|---|---|
| `onnx` | `embeddinggemma-q4` | (local) | 768 |
| `openai` | `text-embedding-3-small` | `https://api.openai.com/v1` | 1536 |
| `ollama` | `nomic-embed-text` | `http://localhost:11434` | 768 |

### Azure OpenAI

Set `backend = "openai"`, `base_url = "https://<your-resource>.openai.azure.com/openai/deployments/<deployment>?api-version=2024-10-21"` and `api_key` to your Azure key. The request path `/embeddings` is appended automatically. Azure requires the `api-version` query param — include it in `base_url`.

### Dim mismatch detection

If you open an existing store with a different backend (different dims), the first embedding operation returns a clear error instead of silently corrupting the index:

```
Embedding dimension mismatch: index has 768d vectors but backend 'openai' produces 1536d.
Rebuild the index (`uteke repair`) or switch backend.
```

To migrate, run `uteke repair` after switching backends — it rebuilds the vector index from the SQLite source of truth using the new backend's embeddings. Because the dim-mismatch guard will block any embed-based operation on first contact, set `UTEKE_ALLOW_DIM_MISMATCH=1` once to let `uteke repair` open the store with the new backend:

```bash
UTEKE_ALLOW_DIM_MISMATCH=1 uteke repair
```

## Recall Threshold

Control minimum similarity score for recall results:

```toml
[recall]
# Minimum similarity score (0.0-1.0). Memories below this score are excluded.
# Default: 0.3 (balanced). Use 0.0 to disable filtering.
min_score = 0.3

# Strict-mode threshold (used with `--strict` flag)
min_score_strict = 0.5

# Default recall strategy for `uteke recall` when --strategy is not given.
# One of: vector | fts5 | hybrid | graph.
#   vector — vector similarity only (original behavior, default)
#   fts5   — full-text search only
#   hybrid — vector + FTS5 fused via Reciprocal Rank Fusion
#   graph  — hybrid + graph-signal reranking (#378): well-connected memories
#            get a subtle log-scaled score boost
default_strategy = "vector"

# Graph-augmented reranking weights (only affect the `graph` strategy).
# Boosts are additive + log-scaled, so 0.1 is subtle and saturates quickly.
graph_density_weight = 0.1    # edge-count boost
graph_authority_weight = 0.1  # incoming-edge (referenced-by) boost
graph_rerank_enabled = true   # master switch; false → graph acts like hybrid
```

| Setting | Default | Description |
|---------|---------|-------------|
| `min_score` | 0.3 | Minimum similarity score (0.0-1.0) |
| `min_score_strict` | 0.5 | Strict-mode threshold (used with `--strict`) |
| `default_strategy` | `vector` | Default recall strategy (`vector\|fts5\|hybrid\|graph`) |
| `graph_density_weight` | 0.1 | Edge-density boost weight (graph strategy only) |
| `graph_authority_weight` | 0.1 | Incoming-edge authority boost weight (graph strategy only) |
| `graph_rerank_enabled` | true | Master switch for graph reranking |

Use `--strict` flag, `--min <score>`, or `--strategy <name>` to override per-query.

## Environment Variables

Environment variables override config file values. Applied in `Config::load()` after config file merge. CLI flags override env vars.

Resolution order (highest priority first):
1. CLI flag (`--min`, `--host`, `--port`)
2. Environment variable (`UTEKE_*`)
3. Config file (`uteke.toml`)
4. Built-in default

| Env Var | Config Equivalent | Default | Description |
|---------|-------------------|---------|-------------|
| `UTEKE_HOME` | — | `~/.uteke` | Data directory |
| `UTEKE_NAMESPACE` | `[store] namespace` | `default` | Default namespace (applied in CLI) |
| `UTEKE_AUTH_TOKEN` | — | — | Server auth token (applied in server) |
| `UTEKE_LOG_LEVEL` | `[logging] level` | `warn` | Log level (trace/debug/info/warn/error) |
| `UTEKE_SERVER_HOST` | `[server] host` | `127.0.0.1` | Server bind address |
| `UTEKE_SERVER_PORT` | `[server] port` | `8767` | Server port |
| `UTEKE_RECALL_MIN_SCORE` | `[recall] min_score` | `0.3` | Default similarity threshold |
| `UTEKE_RECALL_MIN_SCORE_STRICT` | `[recall] min_score_strict` | `0.5` | Strict threshold |
| `UTEKE_RECALL_STRATEGY` | `[recall] default_strategy` | `vector` | Default recall strategy (`vector\|fts5\|hybrid\|graph`) |
| `UTEKE_GRAPH_DENSITY_WEIGHT` | `[recall] graph_density_weight` | `0.1` | Edge-density boost weight |
| `UTEKE_GRAPH_AUTHORITY_WEIGHT` | `[recall] graph_authority_weight` | `0.1` | Incoming-edge authority boost weight |
| `UTEKE_GRAPH_RERANK_ENABLED` | `[recall] graph_rerank_enabled` | `true` | Master switch for graph reranking |
| `UTEKE_EMBEDDING_BACKEND` | `[embedding] backend` | `onnx` | Embedding backend: onnx, openai, ollama |
| `UTEKE_EMBEDDING_MODEL` | `[embedding] model` | backend-specific | Override model name |
| `UTEKE_EMBEDDING_API_KEY` | `[embedding] api_key` | — | API key (OpenAI). Fallback: `OPENAI_API_KEY` |
| `UTEKE_EMBEDDING_BASE_URL` | `[embedding] base_url` | backend-specific | Custom endpoint URL |
| `UTEKE_EMBEDDING_DIMS` | `[embedding] dims` | `0` (auto) | Force embedding dimensionality |

### Docker Example

```bash
docker run -d --name uteke \
  -p 127.0.0.1:8767:8767 \
  -v uteke-data:/data \
  -e UTEKE_LOG_LEVEL=info \
  -e UTEKE_RECALL_MIN_SCORE=0.5 \
  ghcr.io/codecoradev/uteke:latest
```

## Config Migration

If you have an older flat-format config (pre-v0.0.4), uteke auto-migrates it on first run:

```toml
# Old format (auto-detected and migrated)
path = "~/.uteke"
default_namespace = "default"
log_level = "info"

↓ Auto-migrated to ↓

[store]
path = "~/.uteke"
namespace = "default"

[logging]
level = "info"
```

No manual action needed — old config keys are automatically converted to the new sectioned format.

## Namespace Resolution

Namespace is resolved in this order (highest priority first):

1. **`--namespace flag`** — CLI flag (highest priority)
2. **`UTEKE_NAMESPACE`** — Environment variable
3. **`uteke.toml [store] namespace`** — Config file
4. **`"default"`** — Built-in default

Switch default namespace permanently with `uteke namespace switch <name>` — this updates the config file.

## Per-Project Config

Place a `.uteke/uteke.toml` in your project root to override defaults for that project:

```toml
# my-project/.uteke/uteke.toml
[store]
path = "./.uteke"
namespace = "my-project"

[logging]
level = "warn"

[server]
enabled = true
port = 8767
```

Combined with shell hooks, this enables automatic project-scoped memory — each project gets its own isolated memory store.

## CLI Flag Override

CLI flags always take precedence over config file values:

```bash
# Override store path
uteke --store /path/to/project/.uteke remember "project note"

# Override namespace
uteke --namespace agent-1 recall "context"

# Override namespace via env
UTEKE_NAMESPACE=agent-1 uteke recall "context"
```

## File Logging

Logs are written to `~/.uteke/logs/uteke.log` with daily rotation:

```
~/.uteke/logs/
├── uteke.log              # Current log
├── uteke.log.2026-05-29   # Yesterday's log
└── uteke.log.2026-05-28   # Two days ago
```

Non-blocking async writer — logging never blocks memory operations. Rotated files are kept until manually deleted.

## Memory Edges & Backlinks

Memory references are auto-wired on every `remember()` — patterns like `[[slug]]`, `@tag`, `^<uuid>`, and `><uuid>` are parsed (no LLM) and resolved to typed edges in the `memory_edges` table (#346).

Since #350, every forward edge (`references`, `tagged_as`, `supersedes`, `replies_to`) automatically gets an inverse `referenced_by` edge from target → source. Both inserts are wrapped in a single SQLite transaction so the pair is atomic.

```bash
# Inspect edges (both directions)
uteke edges <memory-id>

# Show only incoming edges (backlinks)
uteke edges <memory-id> --direction incoming

# Multi-hop BFS traversal
uteke edges <memory-id> --deep 2

# Repair pre-#350 stores or after manual edge inserts
uteke rebuild-backlinks

# Script-friendly count
uteke rebuild-backlinks --quiet
```

Re-running `rebuild-backlinks` is a no-op — all inserts are `INSERT OR IGNORE` with a `UNIQUE(source_id, target_id, edge_type)` constraint.
