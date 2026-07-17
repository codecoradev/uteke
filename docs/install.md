---
title: Installation
---

# Installation

## Quick Install (Recommended)

One-liner install — no Rust required. Installs `uteke`, `uteke-serve`, and `uteke-mcp`:

```bash
curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh
```

Installs to `~/.local/bin`. Add to PATH if needed:

```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
```

Pin a specific version:

```bash
UTEKE_VERSION=v0.8.0 curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh
```

## Install via Cargo

If you have [Rust](https://rustup.rs) 1.85+ installed:

```bash
cargo install --git https://github.com/codecoradev/uteke
```

This compiles uteke from source and installs it to Cargo's binary directory (typically `~/.cargo/bin/`).

## Pre-built Binary

Download from [GitHub Releases](https://github.com/codecoradev/uteke/releases):

```bash
# Linux (x86_64)
curl -sL https://github.com/codecoradev/uteke/releases/latest/download/uteke-linux-x86_64.tar.gz | tar xz
mv uteke ~/.local/bin/

# macOS (Apple Silicon)
curl -sL https://github.com/codecoradev/uteke/releases/latest/download/uteke-darwin-aarch64.tar.gz | tar xz
mv uteke ~/.local/bin/
```

Supported platforms:

| Platform | Architecture | Format |
|----------|-------------|--------|
| Linux | x86_64 | tar.gz |
| Linux | aarch64 (ARM) | tar.gz |
| macOS | aarch64 (Apple Silicon) | tar.gz |

## Docker

```bash
docker pull ghcr.io/codecoradev/uteke:latest
```

> 💡 See the [Docker guide](/docker) for `docker compose` setup, environment variables, and volume persistence.

## First Run

On first run, uteke downloads the embedding model (~188MB). No API keys needed — fully offline.

```bash
uteke doctor   # Verify installation
uteke --version
```

## Verify Installation

```bash
$ uteke --version
uteke 0.8.0

$ uteke doctor
✓ Database     OK
✓ Index         OK
✓ Model         OK (embeddinggemma-q4, 768d)
✓ Consistency   OK
```

## Updating

```bash
uteke upgrade          # Check + upgrade to latest release
uteke upgrade --yes    # Skip confirmation
```

## What Gets Installed

The install script deploys three binaries:

| Binary | Purpose |
|--------|---------|
| `uteke` | Core CLI — remember, recall, search, list, etc. |
| `uteke-serve` | HTTP server for remote access and MCP over HTTP |
| `uteke-mcp` | Standalone MCP server for AI agent integration |
