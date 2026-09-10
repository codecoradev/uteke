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
# Linux (x86_64) — tarball also contains libonnxruntime.so* (required for embeddings)
curl -sL https://github.com/codecoradev/uteke/releases/latest/download/uteke-x86_64-unknown-linux-gnu-v0.17.0.tar.gz | tar xz
mv uteke uteke-serve uteke-mcp ~/.local/bin/
mv libonnxruntime.so* libonnxruntime_providers_shared.so* ~/.local/bin/

# Linux (x86_64, legacy — no AVX2/SSE4.2)
curl -sL https://github.com/codecoradev/uteke/releases/latest/download/uteke-x86_64-unknown-linux-gnu-legacy-v0.17.0.tar.gz | tar xz
mv uteke uteke-serve uteke-mcp ~/.local/bin/
mv libonnxruntime.so* ~/.local/bin/
mkdir -p ~/.local/bin/ort-legacy && mv ort-legacy/libonnxruntime.so* ~/.local/bin/ort-legacy/

# Linux (aarch64 / ARM)
curl -sL https://github.com/codecoradev/uteke/releases/latest/download/uteke-aarch64-unknown-linux-gnu-v0.17.0.tar.gz | tar xz
mv uteke uteke-serve uteke-mcp ~/.local/bin/
mv libonnxruntime.so* libonnxruntime_providers_shared.so* ~/.local/bin/

# macOS (Apple Silicon)
curl -sL https://github.com/codecoradev/uteke/releases/latest/download/uteke-aarch64-apple-darwin-v0.17.0.tar.gz | tar xz
mv uteke uteke-serve uteke-mcp ~/.local/bin/
mv libonnxruntime*.dylib ~/.local/bin/
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

The install script deploys three binaries plus the bundled ONNX Runtime shared library (required for local embeddings, no API keys needed):

| Binary | Purpose |
|--------|---------|
| `uteke` | Core CLI — remember, recall, search, list, etc. |
| `uteke-serve` | HTTP server for remote access and MCP over HTTP |
| `uteke-mcp` | Standalone MCP server for AI agent integration |
| `libonnxruntime.so*` (Linux) / `libonnxruntime*.dylib` (macOS) / `onnxruntime.dll` (Windows) | ONNX Runtime (currently v1.24.4, matches `ort` crate `2.0.0-rc.12`) — resolved via `<exe_dir>/libonnxruntime.so`, system paths, or `ORT_LIB_PATH` |

> **Arch / CachyOS note:** there is no system `libonnxruntime.so` by default and `pip install onnxruntime` may have no wheel for newer Python (e.g. 3.14), so you must keep the bundled `.so` next to the binary in `~/.local/bin/`. If you see `ONNX Runtime library not found` on `uteke remember`, re-run the installer above or set `ORT_LIB_PATH=/path/to/libonnxruntime.so`. Then `uteke doctor` should show `Embedding model: embeddinggemma-q4`.
