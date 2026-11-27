### 📦 Downloads

Each archive contains three binaries + ONNX Runtime shared library:

| Binary | Description |
|--------|-------------|
| `uteke` | CLI tool |
| `uteke-serve` | HTTP server daemon |
| `uteke-mcp` | MCP server (stdio + HTTP) |
| `libonnxruntime.so*` / `libonnxruntime*.dylib` / `onnxruntime*.dll` | ONNX Runtime |

| Platform | File |
|----------|------|
| Linux x86_64 (AVX2) | `uteke-x86_64-unknown-linux-gnu-__VER__.tar.gz` |
| Linux ARM64 | `uteke-aarch64-unknown-linux-gnu-__VER__.tar.gz` |
| macOS Apple Silicon | `uteke-aarch64-apple-darwin-__VER__.tar.gz` |
| Windows x86_64 | `uteke-x86_64-pc-windows-msvc-__VER__.zip` |

### 🚀 Quick Start

```bash
# Quick install (Linux / macOS)
curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh

# Pin a specific version
UTEKE_VERSION=__VER__ curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh

# Store a memory
uteke remember "Important context" --tags project

# Recall by meaning
uteke recall "what was that context?"

# Start server for fast AI agent access
uteke-serve --port 8767
```

**Full changelog:** https://github.com/codecoradev/uteke/blob/main/CHANGELOG.md
