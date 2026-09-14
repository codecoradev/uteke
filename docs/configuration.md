---
title: Configuration
---

# Configuration

Uteke supports `uteke.toml` configuration with layered resolution.

## Resolution Order

Uteke searches for config in this order. Last match wins (highest priority):

1. **(built-in defaults)** — Hardcoded defaults
2. **`~/.codecora/uteke/uteke.toml`** — Global user-level config
3. **`.uteke/uteke.toml`** — Project-level (in current working directory)

Config file path is auto-resolved (no `--config` flag). Layered merge: each file overlays the previous, with field-level granularity (only keys explicitly present override).

## Config File Format

```toml
# uteke.toml

[store]
# Store location (default: ~/.codecora/uteke)
path = "~/.codecora/uteke"

# Default namespace (default: "default")
namespace = "default"

[logging]
# Log level: trace, debug, info, warn, error
level = "warn"

# Optional log file path. Empty = stderr only.
# file = ""

[server]
# Enable CLI auto-routing to server
enabled = false

# Server host
host = "127.0.0.1"

# Server port
port = 8767

[vector]
# Vector search engine when the binary ships BOTH engines (official builds):
# "usearch" (default — HNSW, C++ FFI, fastest) or "vecq" (training-free
# quantization, zero C++ dep — mobile/slim builds). Switching engines on an
# existing store auto-rebuilds the index from SQLite on next open (#1168).
# Env override: UTEKE_VECTOR_BACKEND=usearch|vecq
backend = "usearch"
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
max_seq_length = 2048
api_key = ""                  # OpenAI only (or use UTEKE_EMBEDDING_API_KEY)
base_url = ""                 # custom endpoint (Azure OpenAI, Ollama URL, proxy)
endpoint_path = ""            # custom API path (default: /embeddings for OpenAI)
dims = 0                     # 0 = use model default (override only if you know)
```

| Setting | Default | Description |
|---------|---------|-------------|
| `backend` | `onnx` | `onnx`, `openai`, or `ollama` |
| `model` | `embeddinggemma-q4` | Backend-specific model name |
| `max_seq_length` | `2048` | Max tokens per input |
| `api_key` | `""` | OpenAI API key (ONNX/Ollama ignore) |
| `base_url` | `""` | Custom endpoint. Empty = backend default |
| `endpoint_path` | `""` | Custom API path appended to base_url. Empty = `/embeddings` (OpenAI) |
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

## Embed Fallback

When the primary embedding backend fails (model not found, OOM, network error),
uteke can transparently retry with a fallback endpoint. This is opt-in — if
unconfigured, no cloud calls are made.

```toml
[embed_fallback]
api_key = ""                  # or use UTEKE_EMBED_FALLBACK_API_KEY
base_url = ""                 # e.g. "https://api.openai.com/v1"
endpoint_path = ""            # path appended to base_url. Empty = "/embeddings"
model = ""                    # e.g. "text-embedding-3-small"
```

| Setting | Default | Description |
|---------|---------|-------------|
| `api_key` | `""` | API key for the fallback endpoint |
| `base_url` | `""` | Fallback API endpoint (OpenAI-compatible) |
| `endpoint_path` | `""` | Path appended to `base_url`. Empty = `/embeddings` |
| `model` | `""` | Fallback embedding model |

Fallback is **active** when all three of `api_key`, `base_url`, and `model` are non-empty. No `enabled` flag needed — empty fields mean inactive.

**Environment variables** take precedence: `UTEKE_EMBED_FALLBACK_API_KEY`, `UTEKE_EMBED_FALLBACK_BASE_URL`, `UTEKE_EMBED_FALLBACK_ENDPOINT_PATH`, `UTEKE_EMBED_FALLBACK_MODEL`.

**Dimension validation** — if the fallback produces different dimensions than the primary, uteke rejects it at startup with a clear error. Both backends must produce vectors of the same dimensionality.

## Fact Extraction

Configure LLM-backed fact extraction for `uteke import --extract`. This is
**opt-in**: the section is inert unless you pass `--extract`. When you do, uteke
sends source text to an OpenAI-compatible chat-completions endpoint and stores
the distilled atomic facts. This is the only feature that makes outbound LLM
calls; everything else stays offline.

```toml
[extraction]
model = "gpt-4o-mini"        # chat model (or UTEKE_EXTRACTION_MODEL)
api_key = ""                 # or UTEKE_EXTRACTION_API_KEY; falls back to the
                             # embedding / OPENAI_API_KEY credential
base_url = ""                # OpenAI-compatible base URL. Empty = OpenAI default
endpoint_path = ""           # custom API path. Empty = /chat/completions
max_facts = 0                # cap facts per document. 0 = built-in default
```

| Setting | Default | Description |
|---------|---------|-------------|
| `model` | `""` | Chat model used to distill facts |
| `api_key` | `""` | API key (falls back to embedding/`OPENAI_API_KEY`) |
| `base_url` | `""` | OpenAI-compatible base URL. Empty = OpenAI default |
| `endpoint_path` | `""` | API path appended to base_url. Empty = `/chat/completions` |
| `max_facts` | `0` | Cap facts kept per document. 0 = built-in default |

Resolution order per field: CLI flag (`--extract-*`) > `UTEKE_EXTRACTION_*` env
var > `[extraction]` config > built-in default.

## Recall Threshold

Control minimum similarity score for recall results:

```toml
[recall]
# Minimum similarity score (0.0-1.0). Memories below this score are excluded.
# Default: 0.3 (balanced). Use 0.0 to disable filtering.
min_score = 0.0

# Strict-mode threshold (used with `--strict` flag)
min_score_strict = 0.5

# Default recall strategy for `uteke recall` when --strategy is not given.
# One of: fusion | vector | fts5 | hybrid | graph.
#   fusion — weighted RRF of the vector and hybrid rankings (default since 0.16.0;
#            LongMemEval 500Q R@5 0.946 vs 0.854 hybrid, #1123)
#   vector — vector similarity only (original behavior)
#   fts5   — full-text search only
#   hybrid — vector + FTS5 fused via Reciprocal Rank Fusion (k=60)
#   graph  — hybrid + graph-signal reranking (#378): well-connected memories
#            get a subtle log-scaled score boost
default_strategy = "fusion"

# Graph-augmented reranking weights (only affect the `graph` strategy).
# Boosts are additive + log-scaled, so 0.1 is subtle and saturates quickly.
graph_density_weight = 0.1    # edge-count boost
graph_authority_weight = 0.1  # incoming-edge (referenced-by) boost
graph_rerank_enabled = true   # master switch; false → graph acts like hybrid
```

| Setting | Default | Description |
|---------|---------|-------------|
| `min_score` | 0.0 | Minimum similarity score (0.0-1.0). **CLI only.** |
| `min_score_strict` | 0.5 | Strict-mode threshold (used with `--strict`). **CLI only.** |
| `default_strategy` | `fusion` | Default recall strategy (`vector\|fts5\|hybrid\|graph\|fusion`). Since 0.16.0 `fusion` (weighted RRF: vector×1.7 + hybrid×1, #1123) is the default — LongMemEval fast50 R@5 0.98 vs 0.9267 hybrid |
| `graph_density_weight` | 0.1 | Edge-density boost weight (graph strategy only) |
| `graph_authority_weight` | 0.1 | Incoming-edge authority boost weight (graph strategy only) |
| `graph_rerank_enabled` | true | Master switch for graph reranking |

> **⚠️ Threshold defaults differ across interfaces (#995)**
>
> | Interface | Default `min_score` | Notes |
> |-----------|---------------------|-------|
> | **CLI** | `0.0` (from `[recall] min_score`) | Reads from `uteke.toml`. Set `--min` or `[recall] min_score` to enable. |
> | **HTTP API / Server** | `0.0` (`DEFAULT_MIN_SCORE`) | Server has its own constant, ignores `[recall] min_score`. |
>
> **Note (since 0.17.x, #1223):** default recall scores under the fusion strategy are rank-based (RRF contribution + salience/recency boosts, typically ~0.0-0.2), not raw cosine similarity. A legacy cosine-era default threshold of 0.3 silently filtered out nearly all results, so the default is now 0.0 across every surface. Set a threshold explicitly (config `min_score`, `--min`, `--strict`, or HTTP `min_score`) only when you know the score scale of the active strategy.
> | **MCP** | `0.0` | MCP server hardcodes 0.0. |
>
> This means a score of 0.25 is returned by API/MCP but **filtered out** by CLI.
> For benchmark/retrieval evaluation, always pass `--min 0.0` explicitly to
> evaluate raw ranking quality without UX threshold filtering.

### Salience + Recency Boost (#352)

Dual-axis recall ranking boost. Applied **after** the RRF merge and recall cache lookup.

- **Salience** — higher score for high-value memory types (decision > insight > fact > note). Per-type decay rates are hardcoded in `type_half_life_days()`.
- **Recency** — exponential decay `exp(-age/τ)` where τ is a per-type time constant.

Opt-in per query via `--salience` / `--recency` CLI flags. The `dream` cycle's `compact` phase can use these for smarter pruning.

| Setting | Default | Description |
|---------|---------|-------------|
| `salience_weight` | 0.15 | Salience boost weight (0 = disable) |
| `recency_weight` | 0.15 | Recency boost weight (0 = disable) |
| `jaccard_weight` | 0.0 | Jaccard token-overlap reranking weight (0 = disable) |

Enabled by default (0.15). Override per-query with `--strict`, `--min <score>`, or `--strategy <name>`.

## Environment Variables

Environment variables override config file values. Applied in `Config::load()` after config file merge. CLI flags override env vars.

Resolution order (highest priority first):
1. CLI flag (`--min`, `--host`, `--port`)
2. Environment variable (`UTEKE_*`)
3. Config file (`uteke.toml`)
4. Built-in default

| Env Var | Config Equivalent | Default | Description |
|---------|-------------------|---------|-------------|
| `UTEKE_HOME` | — | `~/.codecora/uteke` | Data directory |
| `UTEKE_NAMESPACE` | `[store] namespace` | `default` | Default namespace (applied in CLI) |
| `UTEKE_AUTH_TOKEN` | — | — | Server auth token (applied in server) |
| `UTEKE_LOG_LEVEL` | `[logging] level` | `warn` | Log level (trace/debug/info/warn/error) |
| `UTEKE_VECTOR_BACKEND` | `[vector] backend` | compiled-in default | Vector engine when both are compiled in: `usearch`, `vecq`. Ignored (with a warning) on single-engine builds or unknown values. Switching engines on an existing store auto-rebuilds the index from SQLite |
| `UTEKE_SERVER_HOST` | `[server] host` | `127.0.0.1` | Server bind address |
| `UTEKE_SERVER_PORT` | `[server] port` | `8767` | Server port |
| `UTEKE_RECALL_MIN_SCORE` | `[recall] min_score` | `0.0` | Default similarity threshold |
| `UTEKE_RECALL_MIN_SCORE_STRICT` | `[recall] min_score_strict` | `0.5` | Strict threshold |
| `UTEKE_RECALL_STRATEGY` | `[recall] default_strategy` | `fusion` | Default recall strategy (`vector\|fts5\|hybrid\|graph\|fusion`) |
| `UTEKE_GRAPH_DENSITY_WEIGHT` | `[recall] graph_density_weight` | `0.1` | Edge-density boost weight |
| `UTEKE_GRAPH_AUTHORITY_WEIGHT` | `[recall] graph_authority_weight` | `0.1` | Incoming-edge authority boost weight |
| `UTEKE_GRAPH_RERANK_ENABLED` | `[recall] graph_rerank_enabled` | `true` | Master switch for graph reranking |
| `UTEKE_EMBEDDING_BACKEND` | `[embedding] backend` | `onnx` | Embedding backend: onnx, openai, ollama |
| `UTEKE_EMBEDDING_MODEL` | `[embedding] model` | backend-specific | Override model name |
| `UTEKE_EMBEDDING_API_KEY` | `[embedding] api_key` | — | API key (OpenAI). Fallback: `OPENAI_API_KEY` |
| `UTEKE_EMBEDDING_BASE_URL` | `[embedding] base_url` | backend-specific | Custom endpoint URL |
| `UTEKE_EMBEDDING_ENDPOINT_PATH` | `[embedding] endpoint_path` | — | Custom API path (default: `/embeddings`) |
| `UTEKE_EMBEDDING_DIMS` | `[embedding] dims` | `0` (auto) | Force embedding dimensionality |
| `UTEKE_MAX_SEQ_LENGTH` | `[embedding] max_seq_length` | `2048` | Max tokens per embedding input |
| `UTEKE_EXTRACTION_MODEL` | `[extraction] model` | — | Chat model for `import --extract` |
| `UTEKE_EXTRACTION_API_KEY` | `[extraction] api_key` | — | API key. Fallback: embedding key / `OPENAI_API_KEY` |
| `UTEKE_EXTRACTION_BASE_URL` | `[extraction] base_url` | OpenAI default | OpenAI-compatible endpoint base URL |
| `UTEKE_EXTRACTION_ENDPOINT_PATH` | `[extraction] endpoint_path` | — | Custom API path (default: `/chat/completions`) |
| `UTEKE_EXTRACTION_MAX_FACTS` | `[extraction] max_facts` | `0` (default) | Cap facts kept per document |

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
path = "~/.codecora/uteke"
default_namespace = "default"
log_level = "info"

↓ Auto-migrated to ↓

[store]
path = "~/.codecora/uteke"
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

Logs are written to `~/.codecora/uteke/logs/uteke.log` with daily rotation:

```
~/.codecora/uteke/logs/
├── uteke.log              # Current log
├── uteke.log.2026-05-29   # Yesterday's log
└── uteke.log.2026-05-28   # Two days ago
```

Non-blocking async writer — logging never blocks memory operations. Rotated files are kept until manually deleted.

## Configurable Limits (#404)

All hardcoded limits can be overridden via env vars or the `[limits]` section:

```toml
[limits]
max_content_length = 100000   # Max memory content (chars). 0 = disable
max_tags_count = 20           # Max tags per memory
max_tag_length = 50           # Max single tag length (chars)
max_payload_size = 10485760   # Max server payload (bytes, default 10MB)
default_recall_limit = 5      # Default recall limit
```

Environment variables override config values:

| Env Var | Default | Description |
|---------|---------|-------------|
| `UTEKE_MAX_CONTENT_LENGTH` | 100000 | Max memory content length |
| `UTEKE_MAX_TAGS_COUNT` | 20 | Max tags per memory |
| `UTEKE_MAX_TAG_LENGTH` | 50 | Max tag length |
| `UTEKE_MAX_PAYLOAD_SIZE` | 10485760 | Max server payload |
| `UTEKE_DEFAULT_RECALL_LIMIT` | 5 | Default recall limit |

## View-Only API Token (#409)

The server supports dual-role authentication:

```toml
[server]
enabled = true
host = "127.0.0.1"
port = 8767
```

```bash
# Start with admin + read-only tokens
uteke-serve --auth-token admin-secret --read-only-token viewer-key

# Or via env vars
UTEKE_AUTH_TOKEN=admin-secret UTEKE_READ_ONLY_TOKEN=viewer-key uteke-serve
```

Read-only tokens can only access GET endpoints (recall, search, list, stats, graph, health).
POST/DELETE operations return `403 Forbidden`.

## Aging (#247)

Controls automatic memory lifecycle management. Disabled by default — opt-in for long-running stores.

```toml
[aging]
enabled = false             # Set true to enable automatic cleanup
max_age_days = 365          # Prune memories older than this
max_access_count = 10       # Only prune if accessed fewer than this many times
max_cold_count = 1000       # Max cold memories to keep before triggering cleanup
```

| Setting | Default | Description |
|---------|---------|-------------|
| `enabled` | false | Enable automatic aging |
| `max_age_days` | 365 | Maximum age in days before pruning |
| `max_access_count` | 10 | Max access count to be considered "cold" |
| `max_cold_count` | 1000 | Max cold memories before cleanup triggers |

## Maintenance Daemon (#442)

Controls auto-aging and auto-dream background tasks when running in server mode.

```toml
[maintenance]
auto_aging_enabled = false       # Periodically clean up cold, stale memories
auto_aging_interval_hours = 24   # How often to run aging
auto_dream_enabled = true        # Periodically run dream cycle (lint → dedup → orphans)
auto_dream_interval_days = 7     # How often to dream
```

| Setting | Default | Description |
|---------|---------|-------------|
| `auto_aging_enabled` | false | Enable auto-aging in server mode |
| `auto_aging_interval_hours` | 24 | Aging check interval |
| `auto_dream_enabled` | true | Enable auto-dream in server mode |
| `auto_dream_interval_days` | 7 | Dream cycle interval |

## Dream Pipeline Thresholds (#731)

Configurable thresholds for the dream maintenance pipeline (contradiction scan, dedup, orphan detection). All values were hardcoded before v0.9.0.

```toml
[dream]
contradict_similarity_threshold = 0.6    # Above this cosine = NOT a contradiction
contradict_tag_jaccard_min = 0.4         # Min tag overlap to consider contradiction
contradict_max_memories = 200            # Max memories for O(n²) scan
dedup_threshold = 0.92                   # Cosine above this = merge candidate
orphan_importance_threshold = 0.15       # Below this + no edges = orphan
```

| Setting | Default | Description |
|---------|---------|-------------|
| `contradict_similarity_threshold` | 0.6 | Cosine threshold for contradiction detection |
| `contradict_tag_jaccard_min` | 0.4 | Min Jaccard tag overlap for contradiction |
| `contradict_max_memories` | 200 | Max memories loaded for contradiction scan |
| `dedup_threshold` | 0.92 | Cosine threshold for dedup merging |
| `orphan_importance_threshold` | 0.15 | Importance threshold for orphan flagging |

## Update Notification (#917)

Uteke checks GitHub for newer releases at startup. The result is cached for 24 hours to avoid repeated network calls.

```toml
update_check = true    # Set to false to disable startup update notification
```

| Setting | Default | Description |
|---------|---------|-------------|
| `update_check` | true | Enable background update notification on startup |

When an update is available, a banner is printed to **stderr** (does not interfere with stdout pipes). The check is non-blocking: it runs in a background thread joined before process exit, so it never delays command execution.

### Skipping in batch/subprocess mode (#1006)

Set the `UTEKE_NO_UPDATE_CHECK=1` environment variable to skip the update check entirely. This is useful for benchmarks, scripts, or automated pipelines that invoke `uteke` many times via subprocess:

```bash
export UTEKE_NO_UPDATE_CHECK=1
```

## Memory Lifecycle (#928–#937)

Uteke uses a soft-delete lifecycle model. Memories are never hard-deleted directly by automated processes. They transition through a deprecated state before eventual pruning.

### Lifecycle States

```
remember() → ACTIVE → soft_deprecate() → DEPRECATED (hidden, restorable) → prune() → HARD DELETE
```

| State | Visible in recall? | Restorable? | TTL |
|-------|-------------------|-------------|-----|
| **ACTIVE** | ✅ Yes | N/A | — |
| **DEPRECATED** | ❌ Hidden | ✅ `promote()` | 30 days (configurable) |
| **PRUNED** | ❌ Gone | ❌ No | — |

### Configuration

```toml
[lifecycle]
# Master switch: when true, ALL delete operations become soft-deletes.
# forget(), bulk_forget, aging_cleanup, consolidate, delete() all redirect to deprecate.
soft_delete_only = true

# Auto-lifecycle background thread (server mode only).
auto_aging_enabled = true
auto_aging_interval_hours = 168          # Run cycle every 7 days

# What qualifies as "aged" → candidate for deprecation.
min_age_days = 90                        # Must be at least 90 days old
max_access_count = 3                     # Accessed 3 times or fewer

# Rate limiting: max % of active memories deprecated per cycle.
max_deprecate_percent = 1.0              # 1% of active per cycle
min_deprecate_per_cycle = 1              # Floor (always at least 1)
max_deprecate_per_cycle = 50             # Ceiling (never more than 50)

# Deprecated TTL: how long before soft-deleted memories are pruned (hard delete).
deprecated_ttl_days = 30                 # 30 days in deprecated state
auto_prune_enabled = true                # Auto-prune expired deprecated memories

# Dream dedup: when consolidating duplicates, soft-delete instead of hard delete.
dream_dedup_soft_delete = true
```

### Field Reference

| Setting | Default | Description |
|---------|---------|-------------|
| `soft_delete_only` | true | Master switch: all deletes become soft-deletes |
| `auto_aging_enabled` | true | Enable auto-lifecycle background thread (server) |
| `auto_aging_interval_hours` | 168 | Hours between auto-lifecycle cycles |
| `min_age_days` | 90 | Minimum age (days) to be eligible for deprecation |
| `max_access_count` | 3 | Max access count for deprecation eligibility |
| `max_deprecate_percent` | 1.0 | Max % of active memories deprecated per cycle |
| `min_deprecate_per_cycle` | 1 | Floor for per-cycle deprecation cap |
| `max_deprecate_per_cycle` | 50 | Ceiling for per-cycle deprecation cap |
| `deprecated_ttl_days` | 30 | Days before deprecated memories are pruned |
| `auto_prune_enabled` | true | Auto-prune expired deprecated memories in lifecycle cycle |
| `dream_dedup_soft_delete` | true | Consolidation soft-deletes instead of hard-deletes |

### How the Lifecycle Cycle Works

Each cycle runs two phases:

1. **Deprecate phase**: Find aged memories (old + rarely accessed) → apply percentage cap → batch deprecate.
2. **Prune phase**: Find deprecated memories past TTL → hard delete (irreversible).

The percentage cap ensures **at most 1–2% of active memories** are deprecated per cycle, preventing sudden data loss. The cap is calculated as:

```
cap = clamp(total_active × max_deprecate_percent / 100, min_per_cycle, max_per_cycle)
```

### Hard Delete Paths

Hard delete only occurs in **two explicitly controlled paths**:

1. **`prune()`**: Deletes deprecated memories whose TTL has expired. Only runs when `auto_prune_enabled = true`.
2. **`forget()`**: Bypasses soft-delete only when `soft_delete_only = false` (default is `true`, so forget = soft-delete by default).

All other paths (`delete()`, `bulk_delete()`, `aging_cleanup()`, `consolidate()`) respect `soft_delete_only` and deprecate instead.
