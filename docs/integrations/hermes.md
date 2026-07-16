# Hermes Plugin Integration

[Uteke](https://github.com/codecoradev/uteke) can be used as a complementary memory layer for [Hermes Agent](https://github.com/codecoradev/hermes) ecosystems.

## Architecture

Uteke integrates with Hermes in three modes. Pick the one that matches how you
want memory to behave.

```
Hermes Agent
├── Mode A: uteke-tool (recommended)  → manual uteke(action=...) calls via uteke-serve
├── Mode C: shell hook (recommended)   → automatic recall on pre_llm_call, no daemon
└── Mode B: memory-provider (deprecated — Hermes only, removed 2026-06-29)
```

| | Mode A (uteke-tool) | Mode C (shell hook) | ~~Mode B~~ (memory-provider) |
|---|---|---|---|
| Install | `uteke init --agent hermes` | Write handler script | ~~`uteke init --agent hermes --memory-provider`~~ |
| Invocation | Agent calls `uteke(action="recall")` | Automatic (hook) | ~~Automatic (provider)~~ |
| Capture | Agent decides what to store | Manual (`uteke remember`) | ~~Auto-extract on session end~~ |
| Transport | HTTP to `uteke-serve` | CLI subprocess | ~~CLI subprocess~~ |
| Daemon | Requires `uteke-serve` | No | ~~No~~ |
| Rooms / multi-agent | Yes | Yes (via uteke-serve) | ~~No~~ |
| Best for | Explicit, on-demand memory | Lightweight auto-recall | ~~Drop-in replacement~~ |

**Recommended:** Mode A + Mode C side by side — automatic recall via hook, manual
store via tool. Both read the same uteke store.

> **Mode B removed for Hermes.** The memory-provider plugin was removed on
> 2026-06-29. Use Mode A + Mode C instead. The `--memory-provider` pattern
> remains supported for **pi**, **Claude Code**, and **Cursor** — see
> [Memory-Provider for Other Agents](#memory-provider-for-other-agents).

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

> **Note:** For Hermes, use Mode A (uteke-tool) or Mode C (shell hook) instead — the
> Hermes memory-provider plugin has been removed (see [Mode B](#mode-b--memory-provider-deprecated)).

## ~~Mode B~~ — memory-provider (deprecated)

> **DEPRECATED for Hermes (removed 2026-06-29).** Use [Mode A](#mode-a--uteke-tool-manual-actions-multi-agent-rooms) +
> [Mode C](#mode-c--pre_llm_call-shell-hook-automatic-recall-no-plugin) instead.
>
> The `--memory-provider` pattern remains supported for **pi**, **Claude Code**, and **Cursor**.
> See [Memory-Provider for Other Agents](#memory-provider-for-other-agents).
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

## Mode C — pre_llm_call Shell Hook (automatic recall, no plugin)

Mode C is the lightest integration: a standalone Python script registered as a
Hermes `pre_llm_call` shell hook. It runs `uteke recall` on the user message
before every LLM call and injects the results into the prompt — no plugin, no
daemon, no memory-provider config.

### Why Mode C over Mode B?

| Aspect | Mode B (memory-provider) | Mode C (shell hook) |
|--------|--------------------------|---------------------|
| Recall | Automatic (via provider) | Automatic (via hook) |
| Extraction | Automatic (session end) | Not included (use `uteke-tool` or manual) |
| Plugin needed | Yes (`plugins/uteke/`) | No |
| `memory.provider` config | Required | Not needed |
| Per-agent namespace | Single global | Per-agent via `/proc/self/cmdline` |
| Race-safe | Yes (process boundary) | Yes (process boundary) |
| Intervention level | Replaces Hermes memory | Complements Hermes memory |

Mode C is ideal when you want **automatic recall without replacing Hermes's
built-in memory system**. It injects context at API-call time without touching
the system prompt, preserving prompt caching.

### Quick Setup

#### 1. Install uteke

```bash
curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh
```

#### 2. Create the hook handler

Save as `~/.hermes/hooks/uteke-recall/handler.py` (or any path):

```python
"""uteke-recall shell hook — recalls relevant memories on pre_llm_call."""

import json
import pathlib
import subprocess
import sys

def _resolve_agent_name(cwd: str = "") -> str:
    """Extract agent name from Hermes hook payload cwd.

    The /proc/self/cmdline approach is unreliable — shell hooks run as
    child subprocesses whose cmdline is the handler script, not the
    gateway. Use the cwd payload field (always set by Hermes) instead.

    Falls back to "default" if cwd is empty or resolution fails.
    """
    if cwd:
        p = pathlib.Path(cwd)
        for parent in [p, *p.parents]:
            if parent.name and parent.name != "profiles":
                return parent.name
    return "default"

def _recall_uteke(query: str, agent: str, limit: int = 5) -> list:
    try:
        proc = subprocess.run(
            ["uteke", "recall", "--namespace", agent,
             "--limit", str(limit), "--json", query],
            capture_output=True, text=True, timeout=15,
        )
        if proc.returncode != 0:
            return []
        data = json.loads(proc.stdout)
        if not isinstance(data, list):
            return []
        return [{"content": m.get("memory", {}).get("content", ""),
                 "score": m.get("score", 0)} for m in data
                if isinstance(m, dict) and "memory" in m]
    except Exception:
        return []

def main():
    try:
        raw = json.loads(sys.stdin.read())
    except Exception:
        sys.exit(0)

    extra = raw.get("extra", raw)
    message = extra.get("user_message") or raw.get("user_message", "")
    if not isinstance(message, str) or not message.strip() or len(message) < 5:
        sys.exit(0)

    agent = _resolve_agent_name(raw.get("cwd", ""))
    memories = _recall_uteke(message.strip()[:500], agent, limit=5)
    if not memories:
        sys.exit(0)

    lines = []
    for i, mem in enumerate(memories, 1):
        content = mem["content"][:200] + ("..." if len(mem["content"]) > 200 else "")
        lines.append(f"{i}. [{mem['score']:.2f}] {content}")

    json.dump({"context": "Recalled memories (uteke):\n" + "\n".join(lines)},
              sys.stdout, ensure_ascii=False)

if __name__ == "__main__":
    main()
```

#### 3. Register the hook in Hermes config

In `~/.hermes/profiles/<profile>/config.yaml` (or global `config.yaml`):

```yaml
hooks:
  pre_llm_call:
    - command: "python3 /path/to/handler.py"
      timeout: 20
hooks_auto_accept: true
```

#### 4. Verify

```bash
echo '{"user_message": "test recall", "session_id": "verify"}' | \
  python3 /path/to/handler.py
# Expected: {"context": "Recalled memories (uteke):\n1. [0.xx] ..."}
```

### Hook Wire Protocol

Hermes sends JSON to **stdin** on every `pre_llm_call`:

```json
{
  "hook_event_name": "pre_llm_call",
  "session_id": "...",
  "extra": {
    "user_message": "...",
    "is_first_turn": true,
    "model": "..."
  }
}
```

The handler returns JSON on **stdout**:

```json
{"context": "Optional text to inject into the user message"}
```

No stdout (exit 0) = observer mode, no injection.

### Mode Comparison Summary

| | Mode A | Mode C | ~~Mode B~~ |
|---|---|---|---|
| **What** | Manual tool | Shell hook (recall only) | ~~Full memory provider~~ |
| **Recall** | Agent calls `uteke(action="recall")` | Automatic (via hook) | ~~Automatic~~ |
| **Extraction** | Manual `uteke(action="remember")` | Manual (combine with Mode A) | ~~Automatic (session end)~~ |
| **Daemon** | `uteke-serve` required | No | ~~No~~ |
| **Replaces Hermes memory** | No | No | ~~Yes~~ |
| **Best for** | On-demand memory, multi-agent rooms | Lightweight auto-recall | ~~Drop-in replacement~~ |

**Recommended:** Mode A + Mode C — automatic recall via hook, manual
store via tool. Keeps Hermes's built-in memory while adding uteke recall.

## Requirements

- uteke v0.3.0+ (includes `uteke-mcp` binary)
- Mode A (`uteke-tool`): `uteke-serve` running (daemon mode)
- Mode C (shell hook): `uteke` binary on `PATH`, no daemon
- Python 3.7+ (stdlib only — no pip install needed)
