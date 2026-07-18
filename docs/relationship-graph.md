---
title: Relationship Graph
---

# Relationship Graph

Link related memories with typed edges and traverse the graph.

## How It Works

Memories can reference other memories using metadata relationships. This creates a directed graph where you can traverse connections.

Supported relationship types:

| Edge | Meaning |
|------|---------|
| `rel:supersedes` | This memory replaces an older one |
| `rel:contradicts` | This memory contradicts another |
| `rel:references` | This memory references another |
| `rel:derived-from` | This memory was derived from another |

## Linking Memories

```bash
# Store a memory that supersedes an old one
uteke remember "API rate limit is now 2000/min" \
  --meta "rel:supersedes:old-memory-id"
```

## Traversal

Recall with relationship traversal:

```bash
# Recall with related memories, up to 2 hops
uteke recall "rate limit" --related --depth 2
```

The `--related` flag includes connected memories in the result. `--depth` controls how many hops to traverse (default: 1).

## Use Cases

- **Knowledge evolution** — track when information changes and what it replaces
- **Conflict detection** — find contradictions in your knowledge base
- **Context enrichment** — automatically pull in related context during recall

## See Also

- [Time-Travel Queries](/time-travel) — temporal filtering for memory snapshots
- [CLI Reference](/cli-reference) — full command options
