---
title: Docker
---

# Docker

Uteke ships as a multi-arch Docker image with the embedding model pre-baked. No download needed on first run.

## Quick Start

```bash
# Pull and run
docker run -d --name uteke \
  -p 8767:8767 \
  -v uteke-data:/data \
  ghcr.io/codecoradev/uteke:latest

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
      - "8767:8767"
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
docker run -d --name uteke \
  -p 8767:8767 \
  -v uteke-data:/data \
  -e UTEKE_AUTH_TOKEN=$YOUR_TOKEN \
  ghcr.io/codecoradev/uteke:latest

# Now all requests need Authorization header
curl -H "Authorization: Bearer $YOUR_TOKEN" \
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
- `models/embeddinggemma-q4/` — ONNX embedding model (~208MB)

## Multi-Architecture

Images are built for:
- `linux/amd64` — Intel/AMD servers
- `linux/arm64` — Apple Silicon, ARM servers (Ampere, Graviton)

Docker automatically pulls the correct architecture.

## Image Tags

| Tag | Description |
|-----|-------------|
| `latest` | Latest stable release |
| `v0.2.0` | Specific version |
| `0.2` | Minor version (latest patch) |

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

Docker Compose includes a built-in health check:
```yaml
healthcheck:
  test: ["CMD", "curl", "-f", "http://localhost:8767/health"]
  interval: 30s
  timeout: 5s
  retries: 3
```

## Behind a Reverse Proxy

See [TLS & Reverse Proxy](/tls) for Caddy, Nginx, and Cloudflare Tunnel setup.
