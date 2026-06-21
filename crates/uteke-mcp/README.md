# uteke-mcp

MCP (Model Context Protocol) server for [uteke](https://github.com/codecoradev/uteke) — expose persistent semantic memory to AI coding agents.

## Overview

`uteke-mcp` bridges the [uteke](https://github.com/codecoradev/uteke) memory engine with MCP-compatible AI agents (Claude Desktop, Cursor, etc.) via JSON-RPC over stdio.

## Transports

- **stdio** — `uteke-mcp` binary (JSON-RPC over stdin/stdout)
- **HTTP** — `POST /mcp` endpoint on `uteke-serve` (Streamable HTTP transport, protocol `2025-06-18`)

## Quick Start

```bash
# Build
cargo build -p uteke-mcp

# Run as stdio MCP server
uteke-mcp

# Or add to your agent config
hermes mcp add uteke --command uteke-mcp
```

## Tools Exposed

| Tool | Description |
|------|-------------|
| `uteke_remember` | Store a memory |
| `uteke_recall` | Semantic recall by meaning |
| `uteke_search` | Keyword text search |
| `uteke_stats` | Memory store statistics |
| `uteke_forget` | Delete a memory by ID |
| `uteke_list` | List memories |

## License

Apache-2.0
