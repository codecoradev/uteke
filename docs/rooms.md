# Rooms: Shared Memory for Multi-Agent Collaboration

> **Other memory layers are single-player.** They store facts under a flat `user_id` and every recall is scoped to one agent's view. Uteke Rooms let multiple AI agents share a memory space: meeting notes, project decisions, architecture choices, searchable by everyone, attributed by author.

## What is a Room?

A Room is a named, shared memory context. Multiple agents can read and write to the same Room, and every memory carries author attribution so you always know who said what.

| Concept | Single-agent memory | Uteke Rooms |
|---|---|---|
| **Scope** | One agent's view | Multiple agents, one shared space |
| **Attribution** | None | `author` per memory |
| **Cross-search** | ❌ No | ✅ Any agent can recall across rooms |
| **Use case** | Personal notes | Team knowledge, multi-agent workflows |

## Quick Start

```bash
# Create a room (add a description later with `uteke room update`)
uteke room create engineering --title "Engineering team decisions"

# Remember into the room, attributed to an author
uteke remember "Postgres migration deferred to Q4 due to testing capacity" \
  --room engineering \
  --author alice

# Recall from the room. Any agent can do this
uteke room recall engineering "postgres migration"

# List all rooms
uteke room list

# Get room summary (auto-generated overview of recent activity)
uteke room summary engineering
```

## Why Rooms Matter

### The Problem with Per-Agent Memory

Traditional memory tools store everything under a flat `user_id`. This works for single-agent chatbots, but breaks down when:

- **Multiple agents work on the same project**: Agent A can't see Agent B's memories
- **A team shares an AI assistant**: everyone's memories are siloed
- **You want cross-agent knowledge transfer**: there's no shared space

### The Rooms Solution

Rooms create a shared memory layer that every agent can read and write:

```
┌─────────────────────────────────────────┐
│           Room: "engineering"           │
│                                         │
│  Alice: "Migrated to Postgres 16"       │
│  Bob:   "API rate limit set to 1000/m"  │
│  Hermes:"Deploy scheduled for Friday"   │
│                                         │
│  → Any agent can recall all entries     │
│  → Author attribution on every memory   │
└─────────────────────────────────────────┘
```

## Use Cases

### 1. Engineering Team Knowledge Base

```bash
# Alice stores a decision
uteke remember "We chose Redis for caching, not Memcached" \
  --room engineering --author alice --tags decision,infra

# Bob adds context
uteke remember "Redis cluster: 3 nodes, 2 replicas each" \
  --room engineering --author bob --tags infra,redis

# New team member's agent recalls the history
uteke room recall engineering "caching decision"
```

### 2. Multi-Agent Project Coordination

```bash
# Hermes agent stores project context
uteke remember "Client demo moved to Thursday 2pm" \
  --room project-alpha --author hermes

# Claude agent picks it up
uteke room recall project-alpha "client demo"
# → "Client demo moved to Thursday 2pm" (author: hermes)
```

### 3. Sharing an Existing Memory

```bash
# A memory written earlier, outside any room, can join one without being copied
uteke room link 01a1b2c3 weekly-standup --author alice

# List everything in the room, newest last
uteke room recall weekly-standup

# Only what one author contributed
uteke room recall weekly-standup --author alice
```

## Room Management

| Command | Description |
|---|---|
| `uteke room create <id> [--title <text>]` | Create a new room |
| `uteke room update <id> [--title <text>] [--description <text>]` | Change the title or description |
| `uteke room rename <old-id> <new-id>` | Rename a room; its memory and document links move with it |
| `uteke room list` | List all rooms |
| `uteke room recall <id> ["<query>"]` | All memories in the room, or the ones matching a query |
| `uteke room link <memory-id> <id>` | Add an existing memory to the room without copying it |
| `uteke room move-memory <memory-id> --from <id> --to <id>` | Move a memory between rooms |
| `uteke room stats <id>` | Memory count and participants |
| `uteke room summary <id>` | Topic clusters, top tags and decisions (no LLM) |
| `uteke room document <id>` | Turn the room's memories into a document |
| `uteke room delete <id>` | Delete the room; its memories are kept, only the links are removed |

`uteke room consolidate`, `add-document`, `remove-document`, `list-documents` and
`list-rooms` are covered in the [CLI Reference](/cli-reference).

## API Reference

### Create Room

```bash
uteke room create <id> [--title <text>]
```

### Remember into Room

```bash
uteke remember "<content>" \
  --room <room-name> \
  --author <author-name> \
  [--tags tag1,tag2] \
  [--entity <entity>] \
  [--category <category>]
```

### Recall from Room

```bash
uteke room recall <room-id> ["<query>"] [--author <name>] [--limit <n>] [--min <score>]
```

Without a query the room's memories come back in the order they joined. `--limit`
defaults to 0, which means all of them.

### HTTP API

```http
POST /room/create
Content-Type: application/json

{"room_id": "engineering", "title": "Team decisions"}

POST /room/remember
Content-Type: application/json

{
  "room_id": "engineering",
  "content": "Postgres migration deferred to Q4",
  "author": "alice",
  "tags": ["decision", "infra"]
}

POST /room/recall
Content-Type: application/json

{"room_id": "engineering", "query": "migration", "limit": 10}
```

## Rooms, Tags and Author Instead of Namespaces

Namespaces are deprecated and will be removed in a future 0.x release (see the
[CHANGELOG](https://github.com/codecoradev/uteke/blob/main/CHANGELOG.md)). Organise
memories with three things instead:

| Need | Use |
|---|---|
| A shared context several agents read and write | A room |
| Who wrote a memory | `--author` (works with or without a room) |
| Grouping and filtering | Tags, e.g. `--tags project:alpha` |

Room recall already searches across every namespace, so a room works the same
whether or not the store still has more than one namespace.

## See Also

- [Quick Start](/getting-started): Get up and running in 30 seconds
- [Multi-Agent](/multi-agent): How multiple agents coexist with uteke
- [CLI Reference](/cli-reference): Full command documentation
- [HTTP API](/api-reference): REST API for server mode
