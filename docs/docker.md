---
title: Docker
---

# Docker

Uteke ships as a lightweight multi-arch Docker image (~10MB). The embedding model (~200MB) downloads automatically on first run and is cached in the volume, so subsequent updates are instant.

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
| `UTEKE_READ_ONLY_TOKEN` | — | Bearer token that is only allowed to read (GET and `POST /recall`) |
| `UTEKE_EMBEDDING_BACKEND` | `onnx` | `onnx` (local model), `openai` (any OpenAI-compatible endpoint) or `ollama`. Read by the server since v0.20.1 |
| `UTEKE_EMBEDDING_BASE_URL` | — | Endpoint of the external embedder, e.g. `http://embor:8355/v1` |
| `UTEKE_EMBEDDING_MODEL` | — | Model name sent to the endpoint |
| `UTEKE_EMBEDDING_DIMS` | backend default | Vector size; must match the existing store |
| `UTEKE_EMBEDDING_API_KEY` | — | Key for the endpoint (also read from `OPENAI_API_KEY`) |
| `UTEKE_EMBEDDING_ENDPOINT_PATH` | `/embeddings` | Path appended to the base URL |

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

### Passing variables with Docker Compose

Compose reads a `.env` file **only to fill in `${VAR}` references inside the
compose file**. Nothing in `.env` reaches the container by itself. Pass what the
container needs in one of two ways:

```yaml
services:
  uteke:
    image: ghcr.io/codecoradev/uteke:latest
    # (a) load every variable of .env into the container ...
    env_file: .env
    environment:
      # (b) ... or list them one by one; ${VAR} is filled from .env
      - UTEKE_AUTH_TOKEN=${UTEKE_TOKEN}
```

The server reads `UTEKE_AUTH_TOKEN`; a variable called `UTEKE_TOKEN` in `.env`
does nothing unless it is mapped like above. If the token never arrives the
server still starts, now **without authentication**, and the only hint is this
line in the log:

```
WARN uteke_serve: Authentication: disabled — set --auth-token or UTEKE_AUTH_TOKEN for production
```

Check the log after every change to the compose file or `.env`, and recreate the
container (`docker compose up -d --force-recreate`) so the new environment is used.

### Using an external embedder (OpenAI-compatible or Ollama)

By default the image downloads a ~208 MB ONNX model into `/data/models` on the
first start. With an external embedder none of that is needed:

```env
UTEKE_EMBEDDING_BACKEND=openai
UTEKE_EMBEDDING_BASE_URL=http://embor:8355/v1
UTEKE_EMBEDDING_MODEL=embeddinggemma-q4
UTEKE_EMBEDDING_DIMS=768
# UTEKE_EMBEDDING_API_KEY=...   # if the endpoint needs a bearer token
```

(Load them with `env_file` as above, or put an `[embedding]` section in
`/data/uteke.toml`; the entrypoint and the server both read it, and the
environment variable wins over the file.) This needs **v0.20.1 or later**: earlier
servers ignored the setting and always used the local model.

- The log should say `Embedding backend is 'openai': skipping the local ONNX
  model download.` and `Embedding backend: openai`. If it says `downloading
  embedding model` instead, the variables did not reach the container.
- `UTEKE_EMBEDDING_DIMS` must equal the vector size of your store. The URL ends in
  `/v1` and uteke appends `/embeddings` (see `UTEKE_EMBEDDING_ENDPOINT_PATH`).
- The embedder must be reachable from the container (same Docker network). If it
  is down, `remember` still stores the memory but answers `"embedding_written":
  false` with a warning, and `recall` fails until it is back.
- Check it with one write: `POST /remember` should return `"embedding_written":
  true` and `"warning": null`.

The data directory still has to be writable: the image runs as `uteke` (uid and
gid 1000) and writes `uteke.db`, the index files and `embed_cache.db` to `/data`
whatever the embedder. For a bind mount, `chown -R 1000:1000` the host directory.

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
- `uteke_index.usearch` — HNSW vector index (default engine)
- `uteke_index.vecq` — quantized index (created if you switch engines)
- `uteke_index.keys` — Index key mapping
- `models/embeddinggemma-q4/` — ONNX embedding model (~200MB; only with the `onnx` backend)

### Choosing the vector engine (v0.17.0+)

Both engines ship in the image. Pick one via env var — switching leaves your
data untouched; the new engine rebuilds its index from SQLite on the next start:

```yaml
environment:
  - UTEKE_VECTOR_BACKEND=vecq   # or "usearch" (default)
```

`usearch` (HNSW) has the best query latency at scale; `vecq` (4-bit + residual,
pure Rust) builds indexes ~16x faster with ~3x smaller files. See
[configuration](configuration.md#environment-variables) for details.

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
| `v0.12.0` | Specific version |
| `0.12` | Minor version (latest patch) |
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
| `uteke_room_summary_document` | Generate summary document from room (→ `POST /room/summary-document`) |
| `uteke_tags_list` | List all tags with counts (#566) |
| `uteke_tags_rename` | Rename a tag across all memories (#566) |
| `uteke_tags_delete` | Delete a tag from all memories (#566) |
| `uteke_pin` | Pin a memory (prevent decay) (#566) |
| `uteke_unpin` | Unpin a memory (#566) |



See [TLS & Reverse Proxy](/tls) for Caddy, Nginx, and Cloudflare Tunnel setup.
