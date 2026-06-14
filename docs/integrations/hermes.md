# Hermes Plugin Integration

[Uteke](https://github.com/codecoradev/uteke) can be used as a complementary memory layer for [Hermes Agent](https://github.com/codecoradev/hermes) ecosystems.

## Architecture

```
Hermes Agent
├── hybrid-memory (existing)  → entities + relationships + knowledge lifecycle
└── uteke-tool (new, opt-in)  → quick semantic recall via uteke-serve
```

## Setup

### 1. Install uteke

```bash
curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh
```

### 2. Enable server mode

```toml
# ~/.uteke/uteke.toml
[server]
enabled = true
host = "127.0.0.1"
port = 8767
```

Start the daemon:

```bash
uteke-serve --port 8767
```

### 3. Add plugin to Hermes config

```yaml
# hermes config.yaml
plugins:
  enabled:
    - agentboard-tool
    - memory-tool
    - uteke-tool  # opt-in
```

### 4. Plugin directory

Create `uteke-tool` plugin in your Hermes plugins directory:

```
plugins/uteke-tool/
├── plugin.yaml
├── tool.py
└── README.md
```

#### plugin.yaml

```yaml
name: uteke-tool
description: Semantic memory recall and storage via uteke
version: 0.1.0
author: CodeCoraDev
```

#### tool.py

```python
import json
import requests

UTEKE_URL = "http://127.0.0.1:8767"

def uteke(action="recall", content="", tags="", namespace="hermes", limit=5):
    """Call uteke server for memory operations."""
    if action == "remember":
        resp = requests.post(f"{UTEKE_URL}/remember", json={
            "content": content,
            "tags": tags.split(",") if tags else [],
            "namespace": namespace
        })
        return f"Stored: {content[:50]}..."

    elif action == "recall":
        resp = requests.post(f"{UTEKE_URL}/recall", json={
            "query": content,
            "limit": limit,
            "namespace": namespace
        })
        results = resp.json()
        if isinstance(results, list) and results:
            memories = [m["memory"]["content"] for m in results]
            return "\n".join(memories)
        return "No memories found."

    elif action == "stats":
        resp = requests.get(f"{UTEKE_URL}/stats?namespace={namespace}")
        return json.dumps(resp.json(), indent=2)

    return f"Unknown action: {action}"
```

## Usage

```
# Store a memory
uteke(action="remember", content="User prefers dark mode", tags="preference,ui")

# Recall relevant context
uteke(action="recall", content="user preferences")

# Check stats
uteke(action="stats")
```

## How It Works

- **Remember**: POST to `/remember` — content is embedded (EmbeddingGemma Q4, 768d) and stored in SQLite + HNSW vector index
- **Recall**: POST to `/recall` — semantic search via hybrid RRF (vector + FTS5), returns ranked results
- **Stats**: GET `/stats` — memory count, namespace breakdown

## Why Uteke Alongside Hybrid-Memory?

| Feature | hybrid-memory | uteke |
|---------|--------------|-------|
| Primary use | Entity lifecycle, relationships | Fast semantic recall |
| Storage | SQLite + Qdrant | SQLite + usearch |
| Embedding | Cloud (OpenAI) | Local ONNX (offline) |
| Latency | ~100ms (network) | ~42ms (local daemon) |
| Best for | Complex graph queries | Quick context injection |

Uteke complements hybrid-memory by providing instant offline semantic search — no API calls needed for every recall.

## Requirements

- uteke binary in `$PATH`
- `uteke-serve` running (daemon mode)
- Python `requests` library
