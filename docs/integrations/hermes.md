# Hermes Plugin Integration

[Uteke](https://github.com/codecoradev/uteke) can be used as a complementary memory layer for [Hermes Agent](https://github.com/codecoradev/hermes) ecosystems.

## Architecture

Uteke integrates with Hermes in two modes. Pick the one that matches how you
want memory to behave.

```
Hermes Agent
├── Mode A: uteke-tool (manual)    → agent calls uteke(action=...) for explicit remember/recall
├── Mode C: uteke-memory (plugin)  → automatic recall via pre_llm_call hook every turn
└── ~~Mode B: memory-provider~~     → removed 2026-06-29
```

| | Mode A (uteke-tool) | Mode C (uteke-memory plugin) | ~~Mode B~~ (memory-provider) |
|---|---|---|---|
| Install | `uteke init --agent hermes` | Plugin at `~/.hermes/plugins/uteke-memory/` | ~~removed~~ |
| Invocation | Agent calls `uteke(action="recall")` | Automatic (plugin hook) | ~~Automatic (provider)~~ |
| Capture | Agent decides what to store | Manual (`uteke remember` via Mode A) | ~~Auto-extract on session end~~ |
| Transport | HTTP to `uteke-serve` | subprocess or HTTP | ~~CLI subprocess~~ |
| Daemon | Requires `uteke-serve` | No (subprocess) / optional (HTTP) | ~~No~~ |
| Rooms / multi-agent | Yes | Yes | ~~No~~ |
| Best for | Explicit, on-demand memory | Lightweight auto-recall | ~~Drop-in replacement~~ |

**Recommended:** Mode A + Mode C side by side — automatic recall via plugin hook,
manual store via tool. Both read the same uteke store.

## Mode A — uteke-tool (manual actions, multi-agent rooms)

### Quick Setup

#### 1. Install uteke

```bash
curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh
```

#### 2. Auto-install Hermes plugin (v0.3.0+)

```bash
uteke init --agent hermes
```

This generates the plugin directly to `~/.hermes/plugins/uteke-tool/` with:
- `plugin.yaml` — manifest
- `tool.py` — Python entry point (stdlib only, no `requests` dependency)
- `README.md` — usage guide

#### 3. Start the server

```bash
uteke-serve --port 8767
```

#### 4. Start a new Hermes session

The plugin loads automatically.

### Usage

#### Memory Operations

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

#### Room Operations (v0.3.0+, #395, #410)

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

#### MCP Server (Alternative)

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
- `uteke_room_memories` — list memories in a room (#569)

### MCP Client Configuration Examples

#### Claude Desktop (stdio transport)

Create or edit `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS)
or `%APPDATA%\Claude\claude_desktop_config.json` (Windows):

```json
{
  "mcpServers": {
    "uteke": {
      "command": "uteke-mcp"
    }
  }
}
```

#### Claude Desktop (HTTP transport)

```json
{
  "mcpServers": {
    "uteke": {
      "url": "http://127.0.0.1:8767/mcp"
    }
  }
}
```

#### Cursor

Create or edit `.cursor/mcp.json` in your project root:

```json
{
  "mcpServers": {
    "uteke": {
      "command": "uteke-mcp"
    }
  }
}
```

Or with HTTP transport:

```json
{
  "mcpServers": {
    "uteke": {
      "url": "http://127.0.0.1:8767/mcp"
    }
  }
}
```

#### Hermes Native MCP Client (HTTP transport)

```bash
# Register with Hermes using HTTP transport (requires uteke-serve running)
hermes mcp add uteke --url http://127.0.0.1:8767/mcp
```

Or with stdio transport:

```bash
# Register with Hermes using stdio transport
hermes mcp add uteke --command uteke-mcp
```

> **Tip:** HTTP transport is recommended when `uteke-serve` is already running — it avoids
> subprocess overhead and works across machines. Stdio transport is simpler for local,
> single-agent setups where no daemon is desired.

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

## Memory-Provider for Other Agents

The `--memory-provider` pattern also works for non-Hermes agents (#575, #577):

```bash
# pi (pi.dev)
uteke init --agent pi --memory-provider

# Claude Code
uteke init --agent claude --memory-provider

# Cursor
uteke init --agent cursor --memory-provider
```

This installs uteke as the agent's default memory provider — relevant memories are recalled and injected automatically every turn. No daemon needed; talks to the `uteke` binary directly via subprocess.

> **Note:** For Hermes, use Mode A (uteke-tool) or Mode C (uteke-memory plugin) instead — the
> Hermes memory-provider plugin has been removed (see [Mode B](#mode-b--memory-provider-deprecated)).

## ~~Mode B~~ — memory-provider (deprecated)

> **DEPRECATED for Hermes (removed 2026-06-29).** Use [Mode A](#mode-a--uteke-tool-manual-actions-multi-agent-rooms) +
> [Mode C](#mode-c--uteke-memory-plugin-pre_llm_call-hook-automatic-recall) instead.
>
> The `--memory-provider` pattern remains supported for **pi**, **Claude Code**, and **Cursor**.
> See [Memory-Provider for Other Agents](#memory-provider-for-other-agents).
> The template source lives at [`extensions/hermes-memory-provider/`](../extensions/hermes-memory-provider/).
>
> Historical reference: Mode B made uteke Hermes's long-term memory backend via
> `uteke init --agent hermes --memory-provider` + `memory.provider: uteke` config.
> Automatic recall every turn, auto-extract facts on session end. No daemon needed.

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

## Mode C — uteke-memory plugin (pre_llm_call hook, automatic recall)

Mode C is the recommended auto-recall integration: a Hermes Python plugin that
registers a `pre_llm_call` hook. Every turn, before the LLM call, it runs
`uteke recall` on the user message and injects the results into the user message
— no shell hook, no daemon, no memory-provider config.

### Why a Plugin Instead of a Shell Hook?

| Aspect | Old shell hook | Plugin (pre_llm_call) |
|--------|---------------|----------------------|
| Registration | `hooks.pre_llm_call` in config.yaml | `ctx.register_hook("pre_llm_call", cb)` |
| Runs in | Subprocess (separate process) | Gateway process (in-process) |
| Env vars | ❌ Does NOT bridge `HERMES_SESSION_*` | ✅ Full gateway process env |
| Contextvar access | ❌ No access to contextvars | ✅ Full access (thread_id, platform, etc.) |
| Blocking | Yes (subprocess.run) | Yes, but faster (no process spawn) |
| Agent name detection | Hacky (`cwd` in payload) | Reliable (`HERMES_HOME`, `ctx.profile_name`) |
| Performance | Process spawn per turn | In-process function call |

The plugin approach replaces both the old Mode B (MemoryProvider, removed) and
the Mode C shell hook. It runs inside the gateway process, has full access to
contextvars, avoids subprocess spawn overhead, and is the standard Hermes plugin
pattern.

### Quick Setup

#### 1. Install uteke

```bash
curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh
```

#### 2. Install the plugin

The plugin files live at `extensions/hermes-memory-provider/` in the uteke repo.
Copy them to your Hermes plugins directory:

```bash
# Copy from uteke repo
cp -r extensions/hermes-memory-provider ~/.hermes/plugins/uteke-memory/
# Remove .tmpl extension
cd ~/.hermes/plugins/uteke-memory/
for f in *.tmpl; do mv "$f" "${f%.tmpl}"; done
```

Or generate from `uteke init`:

```bash
uteke init --agent hermes
```

#### 3. Enable in Hermes config

In `~/.hermes/profiles/<profile>/config.yaml` (or global `config.yaml`):

```yaml
plugins:
  enabled:
    - uteke-memory
```

No `hooks:` config needed. No `memory.provider` config needed. Just enable the plugin.

#### 4. Verify

```bash
hermes plugins list
# Should show: uteke-memory ... enabled

# Start a new session — recall should work automatically
hermes chat
```

### Plugin Config

Config via `~/.hermes/uteke.json` (preferred) or environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `UTEKE_BIN` | (search PATH) | Path to uteke binary |
| `UTEKE_HOME` | (inherit) | HOME dir for uteke store (`~/.codecora/uteke`) |
| `UTEKE_NAMESPACE` | (agent profile name) | Memory namespace |
| `UTEKE_SERVER_URL` | (empty = subprocess) | uteke-serve HTTP URL |
| `UTEKE_TOKEN` | (empty) | Auth token for uteke-serve |
| `UTEKE_RECALL_LIMIT` | `5` | Memories to recall per turn |
| `UTEKE_RECALL_MIN_SCORE` | `0.40` | Min score to include |
| `UTEKE_RECALL_TIMEOUT` | `15` | Max seconds per recall call |

Example `~/.hermes/uteke.json`:

```json
{
  "server_url": "http://uteke:8767",
  "token": "your-bearer-token",
  "recall_limit": 5,
  "recall_min_score": 0.40
}
```

### How It Works

1. On plugin load (`register(ctx)`), the recall manager initializes: loads config,
   resolves transport (subprocess vs HTTP), finds uteke binary.
2. On every turn, Hermes calls `_pre_llm_call(**kwargs)` in-process.
3. The hook truncates the user message to 500 chars, runs `uteke recall`,
   filters results by min_score, and formats them as `<recalled-memories>` XML.
4. Hermes injects the returned `{"context": "..."}` into the user message
   before sending to the LLM. This preserves the system prompt cache prefix.
5. A circuit breaker pauses recall after 5 consecutive failures for 120 seconds.

### Mode Comparison Summary

| | Mode A | Mode C | ~~Mode B~~ |
|---|---|---|---|
| **What** | Manual tool | Plugin hook (recall only) | ~~Full memory provider~~ |
| **Recall** | Agent calls `uteke(action="recall")` | Automatic (via plugin hook) | ~~Automatic~~ |
| **Extraction** | Manual `uteke(action="remember")` | Manual (combine with Mode A) | ~~Automatic (session end)~~ |
| **Daemon** | `uteke-serve` required | No (subprocess) / optional (HTTP) | ~~No~~ |
| **Replaces Hermes memory** | No | No | ~~Yes~~ |
| **Best for** | On-demand memory, multi-agent rooms | Lightweight auto-recall | ~~Drop-in replacement~~ |

**Recommended:** Mode A + Mode C — automatic recall via plugin, manual
store via tool. Keeps Hermes's built-in memory while adding uteke recall.

## Requirements

- uteke v0.3.0+ (includes `uteke-mcp` binary)
- Mode A (`uteke-tool`): `uteke-serve` running (daemon mode)
- Mode C (plugin hook): `uteke` binary on `PATH`, no daemon
- Python 3.7+ (stdlib only — no pip install needed)
