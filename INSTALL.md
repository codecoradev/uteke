# Uteke Installation Guide

## Prerequisites

- **Rust** 1.75+ — [Install Rust](https://rustup.rs/)
- **C compiler** — Required by `rusqlite` (bundled SQLite)
  - macOS: Xcode Command Line Tools (`xcode-select --install`)
  - Linux: `gcc` or `clang` (`sudo apt install build-essential`)

## Installation

### Build from Source (recommended)

```bash
# Clone
git clone https://github.com/ajianaz/uteke.git
cd uteke

# Build release binary
cargo build --release

# Install to ~/.cargo/bin
cargo install --path crates/uteke-cli

# Verify
uteke --version
```

### Pre-built Binaries

Download from [GitHub Releases](https://github.com/ajianaz/uteke/releases):

| Platform | File |
|----------|------|
| macOS (Apple Silicon) | `uteke-aarch64-apple-darwin.tar.gz` |
| macOS (Intel) | `uteke-x86_64-apple-darwin.tar.gz` |
| Linux (x86_64) | `uteke-x86_64-unknown-linux-musl.tar.gz` |
| Linux (ARM64) | `uteke-aarch64-unknown-linux-gnu.tar.gz` |

```bash
# Download and extract (example: macOS ARM64)
curl -fsSL https://github.com/ajianaz/uteke/releases/latest/download/uteke-aarch64-apple-darwin.tar.gz | tar xz

# Move to PATH
sudo mv uteke /usr/local/bin/

# Verify
uteke --version
```

### Quick Install (Linux/macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/ajianaz/uteke/develop/scripts/install.sh | sh
```

> Installs to `~/.local/bin`. Add to PATH if needed:
> ```bash
> echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc  # or ~/.zshrc
> ```

## First Run

On first `remember` command, Uteke automatically downloads the embedding model (~188MB):

```bash
uteke remember "My first memory" --tags test
```

Model cached at `~/.uteke/models/embeddinggemma-q4/`.

## Verify Installation

```bash
uteke --version        # Should show "uteke 0.1.0"
uteke stats            # Should show store statistics

# Quick smoke test
uteke remember "Hello world" --tags test
uteke recall "hello"
uteke forget <id-from-above>
```

## Configuration

Config file: `~/.uteke/config.toml` (auto-created on first run)

```toml
[store]
# path = "~/.uteke"  # Default store location

[embedding]
# model = "embeddinggemma-q4"
# max_seq_length = 256
```

## Shell Completions

```bash
uteke completions bash  > ~/.local/share/bash-completion/completions/uteke
uteke completions zsh   > ~/.zfunc/_uteke
uteke completions fish  > ~/.config/fish/completions/uteke.fish
```

## Python Integration

Zero-dependency wrapper (stdlib only, Python 3.8+):

```python
from python_hermes import UtekeMemory

mem = UtekeMemory()
mid = mem.remember("Deploy v2.1 to staging", tags=["deploy"])
results = mem.recall("deployment steps")
```

See [`examples/python_hermes.py`](examples/python_hermes.py).

## Troubleshooting

### Build fails: `cc` not found
```bash
# macOS
xcode-select --install

# Ubuntu/Debian
sudo apt install build-essential
```

### Model download fails
Check internet connection. Model downloaded from HuggingFace:
```
https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX
```

Manual download:
```bash
mkdir -p ~/.uteke/models/embeddinggemma-q4/onnx
# Download model_q4.onnx and model_q4.onnx_data to above directory
```

### `uteke: command not found`
```bash
# Add Cargo bin to PATH
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

## Uninstall

```bash
# Remove binary
cargo uninstall uteke

# Remove all data (memories, models, config)
rm -rf ~/.uteke
```
