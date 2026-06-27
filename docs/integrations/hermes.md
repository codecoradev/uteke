# Hermes Plugin Integration

[Uteke](https://github.com/codecoradev/uteke) can be used as a complementary memory layer for [Hermes Agent](https://github.com/codecoradev/hermes) ecosystems.

## Architecture

Uteke integrates with Hermes in two ways. Pick the one that matches how you
want memory to behave.

```
Hermes Agent
├── uteke-tool (opt-in)            → manual uteke(action=...) calls via uteke-serve
└── uteke memory-provider (opt-in) → automatic recall + extraction, no daemon
```

| | `uteke-tool` | memory-provider |
|---|---|---|
| Install | `uteke init --agent hermes` | `uteke init --agent hermes --memory-provider` |
| Invocation | Agent calls `uteke(action="recall", ...)` explicitly | Automatic — recall injected every turn |
| Capture | Agent decides what to store | Auto-extracts facts on session end / pre-compress |
| Transport | HTTP to `uteke-serve` daemon | Direct subprocess to the `uteke` binary |
| Daemon | Requires `uteke-serve` running | None |
| Rooms / multi-agent | Yes | No (single-agent memory) |
| Best for | Explicit, on-demand memory + multi-agent rooms | Replacing Hermes's default memory entirely |

You can run the tool plugin and the memory-provider side by side — they read
the same uteke store, just through different paths.

## Mode A — uteke-tool (manual actions, multi-agent rooms)

## Quick Setup

### 1. Install uteke

```bash
curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh
```

### 2. Auto-install Hermes plugin (v0.3.0+)

```bash
uteke init --agent hermes
```

This generates the plugin directly to `~/.hermes/plugins/uteke-tool/` with:
- `plugin.yaml` — manifest
- `tool.py` — Python entry point (stdlib only, no `requests` dependency)
- `README.md` — usage guide

### 3. Start the server

```bash
uteke-serve --port 8767
```

### 4. Start a new Hermes session

The plugin loads automatically.

## Usage

### Memory Operations

```python
# Store a memory
uteke(action="remember", content="User prefers dark mode", tags="preference,ui")

# Semantic recall
uteke(action="recall", content="user preferences")

# Keyword search
uteke(action="search", content="dark mode")

# List memories
uteke(action="list", limit=10)

# Delete a memory
uteke(action="forget", id="abc12345")

# Stats
uteke(action="stats")
```

### Room Operations (v0.3.0+, #395, #410)

Rooms enable multi-agent collaborative memory — multiple agents share a room and contribute memories with author attribution.

```python
# Create a shared room
uteke(action="room_remember", room_id="sprint-planning", content="Deploy scheduled for Friday", author="agent1")
uteke(action="room_create", room_id="sprint-planning", title="Sprint Planning")

# Add a memory to a room (use remember with room_id)
uteke(action="remember", content="Deploy at 3pm", room_id="sprint-planning", namespace="team")

# Recall from a room
uteke(action="room_recall", room_id="sprint-planning", content="deploy deadline")

# List all rooms (cross-namespace)
uteke(action="room_list")

# Room analytics
uteke(action="room_stats", room_id="sprint-planning")
uteke(action="room_summary", room_id="sprint-planning")

# Delete a room (memories preserved)
uteke(action="room_delete", room_id="sprint-planning")
```

### MCP Server (Alternative)

For MCP-compatible agents, use the uteke MCP server instead of the HTTP plugin:

```bash
# Register with Hermes
hermes mcp add uteke --command uteke-mcp

# Or use the HTTP transport
hermes mcp add uteke --url http://127.0.0.1:8767/mcp
```

The MCP server provides the same tools via JSON-RPC (protocol version `2025-06-18`):
- `uteke_remember` — store memory (supports type, room, author, tags)
- `uteke_recall` — semantic search (supports tags filter, min_score)
- `uteke_list` — list memories (supports pagination via offset)
- `uteke_forget` — delete memory
- `uteke_stats` — store statistics

## Available Actions

| Action | Description |
|--------|-------------|
| `remember` | Store a new memory |
| `recall` | Semantic search |
| `search` | Keyword search |
| `list` | List memories |
| `forget` | Delete memory |
| `stats` | Store statistics |
| `room_remember` | Store memory in a room with author |
| `room_create` | Create a room |
| `room_recall` | Recall from a room |
| `room_list` | List all rooms |
| `room_summary` | Room topic summary |
| `room_stats` | Room statistics |
| `room_delete` | Delete a room |

## Mode B — memory-provider (uteke as Hermes's default memory)

This mode makes uteke Hermes's long-term memory backend. There are no manual
`uteke(...)` calls: relevant memories are recalled and injected into the prompt
automatically every turn, and the transcript is distilled into atomic facts
when a session ends or is about to be compacted. It talks to the `uteke` binary
directly, so no `uteke-serve` daemon is needed.

### 1. Install uteke

```bash
curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh
```

### 2. Generate the memory-provider plugin

```bash
uteke init --agent hermes --memory-provider
```

This writes `~/.hermes/plugins/uteke/`:
- `__init__.py` — the `MemoryProvider` implementation (`register()` entry point)
- `plugin.yaml` — manifest declaring the `on_session_end` / `on_pre_compress` hooks

### 3. Set uteke as the memory provider

In `~/.hermes/config.yaml` (or a per-profile config):

```yaml
memory:
  provider: uteke
```

Only one external memory provider can be active at a time. Setting this
replaces Hermes's built-in memory with uteke.

### 4. (Optional) Enable LLM fact extraction

Recall works fully offline with no extra config. To also distill sessions into
atomic facts on session end, configure an OpenAI-compatible chat endpoint in
`~/.hermes/uteke.json`:

```json
{
  "namespace": "default",
  "extract": true,
  "extract_model": "your-chat-model",
  "extract_base_url": "https://your-endpoint/v1",
  "extract_api_key": "sk-..."
}
```

Equivalent environment variables also work: `UTEKE_NAMESPACE`,
`UTEKE_EXTRACT`, `UTEKE_EXTRACT_MODEL`, `UTEKE_EXTRACT_BASE_URL`,
`UTEKE_EXTRACT_API_KEY`, `UTEKE_BIN`, `UTEKE_HOME`, `UTEKE_RECALL_LIMIT`,
`UTEKE_RECALL_MIN_SCORE`. Extraction is opt-in: with `extract` off (the
default), the plugin only recalls and never makes a network call.

### 5. Start a new Hermes session

The plugin loads automatically. You should see a "Memory provider 'uteke'
activated" line in the logs. From then on, recall is automatic.

### Configuration reference (memory-provider)

| Key (`uteke.json`) | Env var | Default | Description |
|---|---|---|---|
| `bin` | `UTEKE_BIN` | search `PATH` | Path to the `uteke` binary |
| `uteke_home` | `UTEKE_HOME` | process `HOME` | Dir holding the `~/.uteke` store |
| `namespace` | `UTEKE_NAMESPACE` | `default` | Memory namespace |
| `extract` | `UTEKE_EXTRACT` | `true` | Run LLM extraction on session end |
| `extract_model` | `UTEKE_EXTRACT_MODEL` | — | Chat model for extraction |
| `extract_base_url` | `UTEKE_EXTRACT_BASE_URL` | — | OpenAI-compatible base URL |
| `extract_api_key` | `UTEKE_EXTRACT_API_KEY` | — | API key (secret) |
| `recall_limit` | `UTEKE_RECALL_LIMIT` | `6` | Memories prefetched per turn |
| `recall_min_score` | `UTEKE_RECALL_MIN_SCORE` | `0.45` | Drop recall hits below this score |

The provider has a built-in circuit breaker: after repeated subprocess
failures it pauses calls for a cooldown so a misconfigured endpoint or missing
binary never blocks the agent.

## Configuration (uteke-tool)

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `UTEKE_SERVER_URL` | `http://127.0.0.1:8767` | uteke server URL |

(For memory-provider configuration, see the reference table under Mode B.)

## How It Works (uteke-tool)

- **Remember**: POST to `/remember` — content is embedded (EmbeddingGemma Q4, 768d) and stored in SQLite + HNSW vector index
- **Recall**: POST to `/recall` — semantic search via hybrid RRF (vector + FTS5), returns ranked results
- **Rooms**: Cross-namespace collaboration spaces — rooms span namespaces, enabling multi-agent coordination
- **MCP**: JSON-RPC over stdio or HTTP — standard MCP protocol for AI agent integration

The memory-provider plugin (Mode B) skips the HTTP layer entirely and shells
out to the `uteke` binary: `recall --json` for prefetch, `import --extract` for
session-end distillation.

## Requirements

- uteke v0.3.0+ (includes `uteke-mcp` binary)
- Mode A (`uteke-tool`): `uteke-serve` running (daemon mode)
- Mode B (memory-provider): no daemon; just the `uteke` binary on `PATH`
- Python 3.7+ (stdlib only — no pip install needed)
