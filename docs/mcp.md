---
title: MCP Server
---
# MCP Server (Model Context Protocol)

Expose uteke memories as tools to AI coding agents — Claude Code, Claude Desktop, Cursor, Copilot, and any MCP-compatible client.

## Quick Start

### Option A: Stdio Transport (recommended for local agents)

No daemon needed — uteke-mcp communicates over stdin/stdout:

```jsonc
// Claude Code — .mcp.json or ~/.claude/settings.json
{
  "mcpServers": {
    "uteke": {
      "command": "uteke-mcp"
    }
  }
}
```

### Option B: HTTP Transport (for remote/shared access)

Requires `uteke-serve` running. Uses Streamable HTTP transport:

```jsonc
{
  "mcpServers": {
    "uteke": {
      "url": "http://127.0.0.1:8767/mcp"
    }
  }
}
```

## Client Configuration

### Claude Code

Create or edit `.mcp.json` in your project root, or `~/.claude/settings.json` for global access:

```jsonc
// Stdio (recommended)
{
  "mcpServers": {
    "uteke": {
      "command": "uteke-mcp"
    }
  }
}
```

```jsonc
// HTTP (requires uteke-serve)
{
  "mcpServers": {
    "uteke": {
      "url": "http://127.0.0.1:8767/mcp"
    }
  }
}
```

### Claude Desktop

Create or edit `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%\Claude\claude_desktop_config.json` (Windows):

```jsonc
{
  "mcpServers": {
    "uteke": {
      "command": "uteke-mcp"
    }
  }
}
```

### Cursor

Create or edit `.cursor/mcp.json` in your project root:

```jsonc
{
  "mcpServers": {
    "uteke": {
      "command": "uteke-mcp"
    }
  }
}
```

### Hermes

Register uteke as an MCP server with the Hermes native client:

```bash
# Stdio transport
hermes mcp add uteke --command uteke-mcp

# HTTP transport (requires uteke-serve running)
hermes mcp add uteke --url http://127.0.0.1:8767/mcp
```

## MCP is Tool-Based (Not Automatic)

The MCP server exposes uteke as **tools** — the agent decides when to call `uteke_recall`, `uteke_remember`, etc. This means:

- ✅ **On-demand:** The agent calls memory tools only when needed (e.g., "recall project context before coding")
- ✅ **Agent-controlled:** The agent decides what to store and when to query
- ❌ **No auto recall:** Memories are NOT automatically injected every turn
- ❌ **No auto extract:** Facts are NOT automatically extracted from conversations

If you need **automatic recall** (memories injected every LLM call without the agent asking), use **Mode C (shell hook)** instead:

```bash
uteke init --agent hermes  # Installs pre_llm_call hook
```

### Which Integration Mode to Use?

| Need | Use This | How |
|------|----------|-----|
| On-demand memory, coding agents | **MCP (this page)** | `uteke-mcp` or `POST /mcp` |
| Automatic recall every turn | **Mode C (shell hook)** | `uteke init --agent hermes` |
| Automatic recall + auto extract | ~~Mode B (memory-provider)~~ | ❌ Deprecated (removed 2026-06-29) |
| Multi-agent shared rooms | **MCP + Mode C combo** | Mode C for auto recall, MCP for room operations |

> **Tip:** MCP and Mode C work great together. Use Mode C for automatic recall on every LLM call, and MCP for explicit tool-based operations like `uteke_doc_create`, `uteke_graph`, or `uteke_room_recall`.

## Available Tools

Both transports expose the same 15 tools (MCP protocol version `2025-06-18`):

| Tool | Description |
|------|-------------|
| `uteke_remember` | Store a memory (supports type, room, author, tags) |
| `uteke_recall` | Semantic search (supports tags filter, min_score) |
| `uteke_search` | Text search with optional tag filter |
| `uteke_list` | List memories (supports pagination via offset) |
| `uteke_forget` | Delete a memory |
| `uteke_stats` | Memory store statistics |
| `uteke_context` | AI-optimized context output for prompts |
| `uteke_dream` | One-command maintenance pipeline (lint → backlinks → dedup → orphans) |
| `uteke_doc_create` | Create a document (wiki/knowledge base entry) |
| `uteke_doc_get` | Retrieve a document by ID |
| `uteke_doc_list` | List all documents |
| `uteke_doc_search` | Search documents |
| `uteke_doc_delete` | Delete a document |
| `uteke_graph` | Get nodes + edges JSON for visualization |
| `uteke_room_recall` | Semantic recall within a room |

## Transport Comparison

| | Stdio | HTTP |
|---|---|---|
| Binary | `uteke-mcp` | `uteke-serve` |
| Daemon needed | No | Yes |
| Remote access | No | Yes |
| Protocol | JSON-RPC over stdin/stdout | Streamable HTTP (POST `/mcp`) |
| Best for | Local agents, single machine | Shared/team, remote access |

> **Tip:** HTTP transport is recommended when `uteke-serve` is already running — it avoids subprocess overhead and works across machines. Stdio transport is simpler for local, single-agent setups where no daemon is desired.

## Remote Access (VPS / Server / Domain)

### Connect from a local agent to a remote uteke-serve

When `uteke-serve` runs on a VPS or remote server, your local MCP client connects over HTTP:

```bash
# On the server — start uteke-serve with auth enabled
UTEKE_AUTH_TOKEN=your-secret uteke-serve --host 0.0.0.0 --port 8767
```

> **Note:** In production, use environment variables for tokens — never commit secrets to config files. Some clients support env var substitution (e.g., `$UTEKE_AUTH_TOKEN`).

```jsonc
// Claude Code — .mcp.json
{
  "mcpServers": {
    "uteke": {
      "url": "https://uteke.yourdomain.com/mcp",
      "headers": {
        "Authorization": "Bearer your-secret"
      }
    }
  }
}
```

```jsonc
// Claude Desktop
{
  "mcpServers": {
    "uteke": {
      "url": "https://uteke.yourdomain.com/mcp",
      "headers": {
        "Authorization": "Bearer your-secret"
      }
    }
  }
}
```

```jsonc
// Cursor
{
  "mcpServers": {
    "uteke": {
      "url": "https://uteke.yourdomain.com/mcp",
      "headers": {
        "Authorization": "Bearer your-secret"
      }
    }
  }
}
```

### Docker on a VPS

```bash
docker run -d --name uteke \
  -p 8767:8767 \
  -e UTEKE_AUTH_TOKEN=your-secret \
  -v uteke-data:/data \
  ghcr.io/codecoradev/uteke:latest
```

Point your MCP client at `http://YOUR_VPS_IP:8767/mcp` (or use a domain with TLS — see below).

### Domain + TLS (Recommended for Production)

For HTTPS with a domain, use a reverse proxy (Caddy, Nginx, or Cloudflare Tunnel) in front of uteke-serve:

```bash
# uteke-serve still on localhost — the proxy handles TLS
UTEKE_AUTH_TOKEN=your-secret uteke-serve --host 127.0.0.1 --port 8767
```

See [TLS & Reverse Proxy](/tls) for full setup guides (Caddy, Nginx, Cloudflare Tunnel).

### Important: Network Security

| Setting | Value | Why |
|---------|-------|-----|
| `--host` | `127.0.0.1` (local) or `0.0.0.0` (remote) | Bind to localhost unless you need remote access |
| `UTEKE_AUTH_TOKEN` | Set a strong token | **Required** for remote access — without it, anyone can read/write your memories |
| TLS | Use reverse proxy + HTTPS | Encrypts traffic in transit — **strongly recommended** for remote setups |
| Firewall | Allow only port 8767/tcp | Restrict access at the network level |

## Namespace Isolation

The MCP server uses the `default` namespace by default. Each agent can use its own isolated namespace by passing a `namespace` argument to any tool call. Memories in one namespace are never visible to another.

This is the MCP equivalent of the CLI's `--namespace` flag — see [Multi-Agent Isolation](/getting-started#multi-agent-isolation) for details.

## Docker

When running uteke in Docker, both transports are available:

- **HTTP:** Start the container normally — the MCP endpoint is at `http://localhost:8767/mcp` via port mapping.
- **Stdio:** Use `--entrypoint uteke-mcp` to run the MCP binary directly inside the container.

See [Docker guide — MCP](/docker#mcp-model-context-protocol) for full Docker-specific configuration and examples.

## Troubleshooting

| Problem | Solution |
|---------|----------|
| `uteke-mcp: command not found` | Run `install.sh` or `cargo install -p uteke-mcp` to install the binary |
| `Permission denied` | `chmod +x $(which uteke-mcp)` or ensure the binary is on your `PATH` |
| `Connection refused` (HTTP) | Ensure `uteke-serve` is running: `uteke-serve` |
| Client can't see tools | Verify the MCP config JSON is valid and the client has been restarted |
| Slow first query | The embedding model (~188MB) downloads on first use — subsequent calls are ~30ms |

See also: [Architecture — MCP Transport](/architecture#mcp-transport-381)
