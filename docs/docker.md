---
title: Docker
---

# Docker

Uteke ships as a lightweight multi-arch Docker image (~10MB). The embedding model (~208MB) downloads automatically on first run and is cached in the volume — subsequent updates are instant.

## Quick Start

> ⚠️ **Security**: The default config listens on `127.0.0.1` (localhost only). For network access, set `UTEKE_AUTH_TOKEN` (see [Authentication](#with-authentication)).

```bash
# Pull and run (GHCR)
docker run -d --name uteke \
  -p 127.0.0.1:8767:8767 \
  -v uteke-data:/data \
  ghcr.io/codecoradev/uteke:latest

# Or pull from Docker Hub
docker run -d --name uteke \
  -p 127.0.0.1:8767:8767 \
  -v uteke-data:/data \
  codecoradev/uteke:latest

# Verify it's running
curl http://localhost:8767/health

# Store a memory
curl -X POST http://localhost:8767/remember \
  -H "Content-Type: application/json" \
  -d '{"content": "Deployed v2.0 to production"}'

# Recall
curl -X POST http://localhost:8767/recall \
  -H "Content-Type: application/json" \
  -d '{"query": "deployment"}'
```

## Docker Compose

```bash
# Clone and use the included docker-compose.yml
docker compose up -d

# Or create your own:
cat > docker-compose.yml << 'EOF'
services:
  uteke:
    image: ghcr.io/codecoradev/uteke:latest
    ports:
      - "127.0.0.1:8767:8767"
    volumes:
      - uteke-data:/data
    restart: unless-stopped

volumes:
  uteke-data:
EOF

docker compose up -d
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `UTEKE_HOME` | `/data` | Data directory (set in Dockerfile) |
| `UTEKE_AUTH_TOKEN` | — | Bearer token for API authentication |
| `UTEKE_NAMESPACE` | `default` | Default namespace |

### With authentication

```bash
# Read token securely (not stored in shell history)
read -s UTEKE_AUTH_TOKEN
export UTEKE_AUTH_TOKEN

docker run -d --name uteke \
  -p 127.0.0.1:8767:8767 \
  -v uteke-data:/data \
  -e UTEKE_AUTH_TOKEN \
  ghcr.io/codecoradev/uteke:latest

# Now all requests need Authorization header
curl -H "Authorization: Bearer $UTEKE_AUTH_TOKEN" \
  http://localhost:8767/health
```

## Persistence

Data is stored in the `/data` volume. Mount it for persistence:

```bash
# Named volume (managed by Docker)
docker run -v uteke-data:/data ...

# Bind mount (explicit path)
docker run -v /path/to/uteke:/data ...
```

The volume contains:
- `uteke.db` — SQLite database (memories, metadata, FTS5)
- `uteke_index.usearch` — HNSW vector index
- `uteke_index.keys` — Index key mapping
- `models/embeddinggemma-q4/` — ONNX embedding model (~188MB)

## Multi-Architecture

Images are built for:
- `linux/amd64` — Intel/AMD servers
- `linux/arm64` — Apple Silicon, ARM servers (Ampere, Graviton)

Docker automatically pulls the correct architecture.

## Image Registries

| Registry | Image |
|----------|-------|
| **GitHub Container Registry** | `ghcr.io/codecoradev/uteke:latest` |
| **Docker Hub** | `codecoradev/uteke:latest` |

## Image Tags

| Tag | Description |
|-----|-------------|
| `latest` | Latest stable release |
| `v0.7.0` | Specific version |
| `v0.6.6` | Specific version |
| `0.6` | Minor version (latest patch) |
| `slim` | Slim image (no embedded model — mount model volume separately, see below) |

## CLI in Docker

The container runs `uteke-serve` by default. To run CLI commands:

```bash
# Run a one-off CLI command
docker exec uteke uteke recall "deployment" --limit 5

# Or override the entrypoint
docker run --rm -v uteke-data:/data \
  --entrypoint uteke \
  ghcr.io/codecoradev/uteke:latest \
  stats
```

## Health Check

```bash
curl http://localhost:8767/health
# → {"status":"healthy","memories":42,"index_size":1024}
```

Docker Compose includes a built-in health check (`curl` is pre-installed in the image):
```yaml
healthcheck:
  test: ["CMD", "curl", "-f", "http://localhost:8767/health"]
  interval: 30s
  timeout: 5s
  retries: 3
```

## Behind a Reverse Proxy

## MCP (Model Context Protocol)

The Docker image includes the `uteke-mcp` binary for MCP-based AI agent integration.

### HTTP Transport (via uteke-serve)

HTTP transport is available through `uteke-serve` at the `/mcp` endpoint. Start the container normally and point your MCP client at the server:

```bash
# Start uteke with MCP endpoint enabled (default)
docker run -d --name uteke \
  -p 127.0.0.1:8767:8767 \
  -v uteke-data:/data \
  ghcr.io/codecoradev/uteke:latest

# The MCP endpoint is available at:
# http://localhost:8767/mcp (Streamable HTTP transport)
```

Example client configurations:

```jsonc
// Claude Desktop — claude_desktop_config.json
{
  "mcpServers": {
    "uteke": {
      "url": "http://localhost:8767/mcp"
    }
  }
}
```

```jsonc
// Cursor — .cursor/mcp.json
{
  "mcpServers": {
    "uteke": {
      "url": "http://localhost:8767/mcp"
    }
  }
}
```

> **Note**: When running uteke in Docker and the MCP client on the host, `localhost` works because of the port mapping. For remote setups, replace `localhost` with the server's hostname or IP and configure `UTEKE_AUTH_TOKEN`.

### Stdio Transport (via uteke-mcp)

The `uteke-mcp` binary in the container provides stdio transport for clients that require subprocess-based MCP. Use `--entrypoint` to run it:

```bash
docker run --rm -v uteke-data:/data \
  --entrypoint uteke-mcp \
  -i ghcr.io/codecoradev/uteke:latest
```

For Claude Desktop or Cursor with stdio transport:

```jsonc
// Claude Desktop — claude_desktop_config.json
{
  "mcpServers": {
    "uteke": {
      "command": "docker",
      "args": [
        "run", "--rm", "-i",
        "-v", "uteke-data:/data",
        "--entrypoint", "uteke-mcp",
        "ghcr.io/codecoradev/uteke:latest"
      ]
    }
  }
}
```

### Available MCP Tools

Both transports expose the same tools (MCP protocol version `2025-06-18`):

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
| `uteke_doc_update` | Partial document update with chunk rebuild (#589) |
| `uteke_doc_move` | Move document to new parent (#438) |
| `uteke_graph` | Get nodes + edges JSON for visualization |
| `uteke_room_recall` | Semantic recall within a room |
| `uteke_room_memories` | List memories in a room (#569) |
| `uteke_room_create` | Create a room |
| `uteke_room_delete` | Delete a room |
| `uteke_room_stats` | Room statistics |
| `uteke_room_summary` | Room topic summary (tag clustering, no LLM) |
| `uteke_room_document` | Generate structured document from room |
| `uteke_tags_list` | List all tags with counts (#566) |
| `uteke_tags_rename` | Rename a tag across all memories (#566) |
| `uteke_tags_delete` | Delete a tag from all memories (#566) |
| `uteke_pin` | Pin a memory (prevent decay) (#566) |
| `uteke_unpin` | Unpin a memory (#566) |



See [TLS & Reverse Proxy](/tls) for Caddy, Nginx, and Cloudflare Tunnel setup.
