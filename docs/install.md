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
UTEKE_VERSION=v0.17.0 curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh
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
curl -sL https://github.com/codecoradev/uteke/releases/latest/download/uteke-x86_64-unknown-linux-gnu-v0.17.0.tar.gz | tar xz
mv uteke ~/.local/bin/

# Linux (x86_64, legacy — no AVX2/SSE4.2)
curl -sL https://github.com/codecoradev/uteke/releases/latest/download/uteke-x86_64-unknown-linux-gnu-legacy-v0.17.0.tar.gz | tar xz
mv uteke ~/.local/bin/

# Linux (aarch64 / ARM)
curl -sL https://github.com/codecoradev/uteke/releases/latest/download/uteke-aarch64-unknown-linux-gnu-v0.17.0.tar.gz | tar xz
mv uteke ~/.local/bin/

# macOS (Apple Silicon)
curl -sL https://github.com/codecoradev/uteke/releases/latest/download/uteke-aarch64-apple-darwin-v0.17.0.tar.gz | tar xz
mv uteke ~/.local/bin/
```

Supported platforms:

| Platform | Architecture | Format |
|----------|-------------|--------|
| Linux | x86_64 | tar.gz |
| Linux | x86_64 (legacy, no AVX2) | tar.gz |
| Linux | aarch64 (ARM) | tar.gz |
| macOS | aarch64 (Apple Silicon) | tar.gz |
| Windows | x86_64 | zip |

### Windows (PowerShell)

```powershell
# Quick install via PowerShell
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
irm https://raw.githubusercontent.com/codecoradev/uteke/main/install.ps1 | iex
```

Or download the `.zip` binary from [Releases](https://github.com/codecoradev/uteke/releases) and extract it to a directory on your `PATH`.

## Docker

```bash
docker pull ghcr.io/codecoradev/uteke:latest
```

> 💡 See the [Docker guide](/docker) for `docker compose` setup, environment variables, and volume persistence.

## First Run

On first run, uteke downloads the embedding model (~200MB). No API keys needed — fully offline.

```bash
uteke doctor   # Verify installation
uteke --version
```

## Verify Installation

```bash
$ uteke --version
uteke 0.14.3

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
