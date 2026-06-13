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

Override the config file path with the `--config` flag.

## Config File Format

```toml
# uteke.toml

[store]
# Store location (default: ~/.uteke)
path = "~/.uteke"

# Default namespace (default: "default")
namespace = "default"

[log]
# Log level: trace, debug, info, warn, error
level = "info"

# Log directory (default: ~/.uteke/logs)
dir = "~/.uteke/logs"

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

Configure the embedding backend:

```toml
[embedding]
# Backend: "onnx" (default), future: "openai", "ollama"
backend = "onnx"

# Model name (for ONNX backend)
model = "embeddinggemma-q4"

# Maximum sequence length in tokens
max_seq_length = 256
```

| Setting | Default | Description |
|---------|---------|-------------|
| `backend` | onnx | Embedding backend |
| `model` | embeddinggemma-q4 | Model identifier |
| `max_seq_length` | 256 | Max tokens per input |

## Recall Threshold

Control minimum similarity score for recall results:

```toml
[recall]
# Minimum similarity score (0.0-1.0). Memories below this score are excluded.
min_score = 0.0
```

| Setting | Default | Description |
|---------|---------|-------------|
| `min_score` | 0.0 | Minimum similarity score |

Use `--strict` flag or `--min 0.7` to override per-query.

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

[log]
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

[log]
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

# Override config file
uteke --config ./my-config.toml stats

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
