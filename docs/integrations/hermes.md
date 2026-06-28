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

## Troubleshooting & FAQ

### Mode C — pre_llm_call shell hook (automatic recall, no plugin)

This is the simplest integration: a lightweight Python script runs on every
LLM call, recalls relevant uteke memories, and injects them into the prompt.
No plugin, no daemon, no memory-provider config — just a shell hook.

**Architecture:**

```
User message arrives
  → Hermes fires pre_llm_call shell hook
  → handler.py reads user_message from stdin (JSON wire protocol)
  → handler.py runs: uteke recall --namespace {agent} --json "{query}"
  → handler.py outputs: {"context": "Recalled memories (uteke):\n1. ..."}
  → Hermes injects context into the user message before API call
```

#### Setup

1. **Create the hook handler** at a shared path (e.g. `/opt/data/hooks/uteke-recall/handler.py`):

```python
"""uteke-recall shell hook — recalls relevant memories on pre_llm_call.

Reads JSON context from stdin (Hermes shell hook wire protocol),
runs uteke recall, and outputs {"context": "..."} to stdout.

Hermes injects this context into the user message before each LLM call.
Race-safe: no shared file writes, per-call invocation.
"""

import json, pathlib, subprocess, sys

def _resolve_agent_name() -> str:
    """Extract agent name from 'hermes -p {agent}' in /proc/self/cmdline."""
    try:
        with open("/proc/self/cmdline", "rb") as f:
            parts = f.read().split(b"\x00")
        for i, part in enumerate(parts):
            if part == b"-p" and i + 1 < len(parts):
                name = parts[i + 1].decode("utf-8", errors="ignore").strip()
                if name:
                    return name
    except Exception:
        pass
    return "default"

AGENT = _resolve_agent_name()
UTEKE_BIN = pathlib.Path("/opt/data/.cargo/bin/uteke")

def _recall_uteke(query: str, limit: int = 5) -> list:
    """Run uteke recall and return list of {content, score}."""
    if not UTEKE_BIN.exists():
        return []
    cmd = [str(UTEKE_BIN), "recall", "--namespace", AGENT,
           "--limit", str(limit), "--json", query]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=15)
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
    if not isinstance(raw, dict):
        sys.exit(0)

    # Hermes wire protocol: user_message is in extra dict
    extra = raw.get("extra", {})
    if not isinstance(extra, dict):
        extra = {}
    message = extra.get("user_message") or raw.get("user_message", "")
    if not isinstance(message, str) or not message.strip():
        sys.exit(0)
    message = message.strip()[:500]

    if len(message) < 5:
        sys.exit(0)

    # Skip cron sessions
    session_id = raw.get("session_id", "")
    if isinstance(session_id, str) and session_id.startswith("cron_"):
        sys.exit(0)

    memories = _recall_uteke(message, limit=5)
    if not memories:
        sys.exit(0)

    lines = []
    for i, mem in enumerate(memories, 1):
        content = mem["content"]
        if len(content) > 200:
            content = content[:197] + "..."
        lines.append(f"{i}. [{mem['score']:.2f}] {content}")

    json.dump({"context": "Recalled memories (uteke):\n" + "\n".join(lines)},
              sys.stdout, ensure_ascii=False)

if __name__ == "__main__":
    main()
```

2. **Register the hook** in each agent's `config.yaml`:

```yaml
hooks:
  pre_llm_call:
    - command: "python3 /opt/data/hooks/uteke-recall/handler.py"
      timeout: 20
hooks_auto_accept: true
```

#### How it differs from Mode A and B

| | Mode A (uteke-tool) | Mode B (memory-provider) | Mode C (shell hook) |
|---|---|---|---|
| Recall | Manual `uteke(action="recall")` | Automatic every turn | Automatic every LLM call |
| Extraction | No | Yes (session end) | No |
| Plugin needed | Yes | Yes | **No** |
| Daemon needed | Yes (`uteke-serve`) | No | No |
| Inject mechanism | Tool output in context | MemoryProvider prefetch | `pre_llm_call` stdout |
| Config changes | Plugin + server URL | `memory.provider: uteke` | `hooks:` block only |

Mode C is the lightest option — it adds recall to any agent with just two
file edits (handler script + config.yaml hook block). Combine with Mode B
for both automatic recall AND automatic extraction.

#### Race safety

Each LLM call invokes the handler as a separate subprocess. The result is
returned via stdout (process boundary), not written to any shared file. This
makes it safe for concurrent sessions on the same agent profile — unlike
approaches that write to SOUL.md or shared files.

### Q: Memory provider not activating in gateway mode

**Symptom:** `memory.provider: uteke` is set in config.yaml but the "Memory provider 'uteke' activated" log never appears.

**Cause:** In Hermes gateway mode, the `memory.provider` initialization path (`agent_init.py`) may not execute — this is a known Hermes limitation. The provider works correctly in CLI mode (`hermes chat`).

**Workaround — Gateway Hook approach:**

Create a gateway hook that recalls uteke on every message and writes results to a context file:

1. Create `HERMES_HOME/hooks/uteke-recall/HOOK.yaml`:
   ```yaml
   name: uteke-recall
   description: "Auto-recall uteke memories on agent:start"
   events:
     - agent:start
   ```

2. Create `HERMES_HOME/hooks/uteke-recall/handler.py`:
   ```python
   import json, os, pathlib, subprocess

   AGENT = os.environ.get("HERMES_HOME", "").split("/")[-1] or "default"
   CONTEXT_FILE = pathlib.Path(f"/tmp/hermes/uteke-context/{AGENT}.md")
   UTEKE_BIN = pathlib.Path("/opt/data/.cargo/bin/uteke")  # or shutil.which("uteke")

   def handle(event_type, context):
       if event_type != "agent:start":
           return
       message = context.get("message", "").strip()
       if not message or len(message) < 5:
           return
       if context.get("session_id", "").startswith("cron_"):
           return

       result = subprocess.run(
           [str(UTEKE_BIN), "recall", "--namespace", AGENT, "--limit", "5", "--json", message],
           capture_output=True, text=True, timeout=15,
       )
       if result.returncode != 0:
           return

       memories = json.loads(result.stdout)
       CONTEXT_FILE.parent.mkdir(parents=True, exist_ok=True)
       lines = [f"# Uteke Recalled Memories - {AGENT}", ""]
       if not memories:
           lines.append("No relevant memories found.")
       else:
           for item in memories:
               mem = item.get("memory", {})
               lines.append(f"- ({item.get('score', 0):.3f}) {mem.get('content', '')}")
       CONTEXT_FILE.write_text("\\n".join(lines))
   ```

3. Add instruction to the agent's SOUL.md to read the context file.

4. Restart the gateway: `s6-svc -r /run/service/gateway-{agent}`

**Note:** The gateway hook system uses `emit()` (fire-and-forget), so the hook output is NOT automatically injected into the prompt. The agent must read the context file as a fallback. For true prompt injection, Hermes upstream would need to change `emit()` to `emit_collect()` for `agent:start` events.

### Q: Hook loaded but recall returns irrelevant results

**Possible causes:**

| Cause | Check | Fix |
|-------|-------|-----|
| Too few memories | `uteke stats --namespace {agent}` — < 100 total | Run more extractions |
| Near-duplicates | Multiple memories saying the same thing | `uteke dream --namespace {agent} --phases dedup` |
| Poor extraction | Facts too generic ("user is ajianaz" repeated) | Improve extraction prompts or manual curation |
| Wrong namespace | `HERMES_HOME` resolves differently than expected | Verify agent name resolution in handler.py |

### Q: Memory quality maintenance

```bash
# Weekly: dedup near-duplicates
uteke dream --namespace {agent} --phases dedup

# Monthly: full maintenance (lint + dedup + orphans + compact)
uteke dream --namespace {agent}

# One-time: system health check
uteke doctor

# Remove cold memories (>30 days, rarely accessed)
uteke aging --namespace {agent} --preview
uteke aging --namespace {agent} --cleanup
```

### Q: Hermes update safety

All uteke setup files are stored in `HERMES_HOME` (typically `~/.hermes/` or per-profile), which is **outside** the Hermes source tree. They survive Hermes updates:

- `~/.hermes/plugins/uteke/` — memory-provider plugin
- `~/.hermes/plugins/uteke-tool/` — tool plugin
- `~/.hermes/hooks/uteke-recall/` — gateway hook
- `~/.hermes/uteke.json` — provider config
- `~/.hermes/config.yaml` — `memory.provider` setting

### Q: Multiple agents on a shared gateway

When running multiple Hermes agents on a single gateway, each agent should use its own namespace. The hook handler resolves the agent name from `HERMES_HOME`:

```python
AGENT = os.environ.get("HERMES_HOME", "").split("/")[-1]
```

For per-profile gateways (`hermes -p {agent}`), `HERMES_HOME` is set to the profile directory, so this resolves correctly. For shared gateways, ensure each agent's SOUL.md specifies its namespace explicitly.

### Q: `uteke list` shows fewer memories than `uteke stats`

`uteke list` has a default output limit. Use `--json` and pipe to `jq` or Python to get the full count:

```bash
uteke list --namespace {agent} --json | python3 -c "import sys,json; print(len(json.load(sys.stdin)))"
```

`uteke stats` reports the true total from the database.

## Requirements

- uteke v0.3.0+ (includes `uteke-mcp` binary)
- Mode A (`uteke-tool`): `uteke-serve` running (daemon mode)
- Mode B (memory-provider): no daemon; just the `uteke` binary on `PATH`
- Python 3.7+ (stdlib only — no pip install needed)
