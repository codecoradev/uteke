# TLS & Reverse Proxy

Uteke server (`uteke-serve`) runs plain HTTP by default. For production deployments, use a reverse proxy to add TLS termination, rate limiting, and other security features.

## Quick Setup with Caddy

[Caddy](https://caddyserver.com/) provides automatic HTTPS with Let's Encrypt.

### Caddyfile

```
uteke.yourdomain.com {
    reverse_proxy localhost:8767
}
```

Caddy automatically provisions and renews TLS certificates.

### Docker Compose

```yaml
version: "3"
services:
  uteke:
    image: ghcr.io/codecoradev/uteke:latest
    command: uteke-serve --host 0.0.0.0 --auth-token YOUR_SECRET_TOKEN
    volumes:
      - uteke-data:/data
    environment:
      - UTEKE_AUTH_TOKEN=${UTEKE_AUTH_TOKEN}

  caddy:
    image: caddy:2
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile
      - caddy-data:/data
      - caddy-config:/config

volumes:
  uteke-data:
  caddy-data:
  caddy-config:
```

## Quick Setup with Nginx

```nginx
server {
    listen 443 ssl http2;
    server_name uteke.yourdomain.com;

    ssl_certificate /etc/letsencrypt/live/uteke.yourdomain.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/uteke.yourdomain.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8767;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

## Security Checklist

| Item | Status | Details |
|------|--------|---------|
| TLS encryption | ✅ Via reverse proxy | Caddy (auto) or Nginx (manual Let's Encrypt) |
| Authentication | ✅ Built-in | `--auth-token` flag or `uteke.toml [server] auth_token` |
| CORS | ✅ Built-in | `--cors-origin` or `uteke.toml [server] cors_origins` |
| Bind address | ✅ Configurable | `--host 127.0.0.1` for local-only access |
| Rate limiting | ⚠️ Via reverse proxy | Configure in Caddy/Nginx |
| File permissions | ✅ Built-in | Owner-only (0700/0600) on data directories |

## Configuration

### Bind to Localhost Only (Recommended)

```bash
# Only accessible from the same machine
uteke-serve --host 127.0.0.1 --auth-token YOUR_TOKEN
```

### Allow External Access (Behind Reverse Proxy)

```bash
# Listen on all interfaces (use with reverse proxy + auth)
uteke-serve --host 0.0.0.0 --auth-token YOUR_TOKEN --cors-origin https://uteke.yourdomain.com
```

### Environment Variables

| Variable | Purpose |
|----------|---------|
| `UTEKE_AUTH_TOKEN` | Bearer token for API authentication |
| `UTEKE_HOME` | Data directory (default: `~/.uteke`) |

## Cloudflare Tunnel

For zero-config TLS without a reverse proxy:

```bash
cloudflared tunnel --url http://localhost:8767
```

Cloudflare Tunnel provides TLS termination, DDoS protection, and access logging — no certificate management needed.

---

[← Back to docs](./)
