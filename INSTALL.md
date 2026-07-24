# Uteke Installation Guide

## Prerequisites

- **Rust** 1.75+ — [Install Rust](https://rustup.rs/)
- **C compiler** — Required by `rusqlite` (bundled SQLite)
  - macOS: Xcode Command Line Tools (`xcode-select --install`)
  - Linux: `gcc` or `clang` (`sudo apt install build-essential`)
  - Windows: Visual Studio Build Tools or `winget install Microsoft.VisualStudio.2022.BuildTools`

## Installation

### Build from Source (recommended)

```bash
# Clone
git clone https://github.com/codecoradev/uteke.git
cd uteke

# Build release binary
cargo build --release

# Install to ~/.cargo/bin
cargo install --path crates/uteke-cli

# Verify
uteke --version
```

### Pre-built Binaries

Download from [GitHub Releases](https://github.com/codecoradev/uteke/releases):

| Platform | File |
|----------|------|
| macOS (Apple Silicon) | `uteke-{version}-aarch64-apple-darwin.tar.gz` |
| Linux (x86_64) | `uteke-{version}-x86_64-unknown-linux-gnu.tar.gz` |
| Linux (ARM64) | `uteke-{version}-aarch64-unknown-linux-gnu.tar.gz` |
| Windows (x86_64) | `uteke-{version}-x86_64-pc-windows-msvc.zip` |
| Windows (ARM64) | `uteke-{version}-aarch64-pc-windows-msvc.zip` |

> **Note:** Intel Mac is not supported via pre-built binaries (ort-sys doesn't provide prebuilts). Intel Mac users can build from source.

### Quick Install (Linux/macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh
```

> Installs `uteke` and `uteke-serve` to `~/.local/bin`. Add to PATH if needed:
> ```bash
> echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc  # or ~/.zshrc
> ```

Pin a specific version with `UTEKE_VERSION`:

```bash
UTEKE_VERSION=v0.0.7 curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh
```

### Windows Setup

#### Option A: Pre-built Binary

```powershell
# Download from GitHub Releases
# Example (PowerShell):
Invoke-WebRequest -Uri "https://github.com/codecoradev/uteke/releases/latest/download/uteke-x86_64-pc-windows-msvc.zip" -OutFile "uteke.zip"

# Extract
Expand-Archive -Path uteke.zip -DestinationPath uteke

# Move to a directory in PATH (e.g. C:\Users\you\AppData\Local\bin)
mkdir -p C:\Users\you\AppData\Local\bin
move uteke\uteke.exe C:\Users\you\AppData\Local\bin\

# Add to PATH (current session)
$env:PATH += ";C:\Users\you\AppData\Local\bin"

# Permanent PATH (add to profile)
[Environment]::SetEnvironmentVariable("PATH", $env:PATH + ";C:\Users\you\AppData\Local\bin", "User")
```

#### Option B: Build from Source

```powershell
# Install Rust via rustup
winget install Rustlang.Rustup

# Clone and build
git clone https://github.com/codecoradev/uteke.git
cd uteke
cargo build --release

# Binary at target\release\uteke.exe
# Copy somewhere in PATH
copy target\release\uteke.exe C:\Users\you\AppData\Local\bin\
```

> **Note:** On Windows, data is stored at `C:\Users\you\.uteke\`.

## First Run

On first `remember` command, Uteke automatically downloads the embedding model (~188MB):

```bash
uteke remember "My first memory" --tags test
```

Model cached at:
- **macOS/Linux:** `~/.codecora/uteke/models/embeddinggemma-q4/`
- **Windows:** `C:\Users\you\.uteke\models\embeddinggemma-q4\`

## Verify Installation

```bash
uteke --version        # Should show "uteke 0.0.2"
uteke stats            # Should show store statistics

# Quick smoke test
uteke remember "Hello world" --tags test
uteke recall "hello"
uteke forget <id-from-above>
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
from python_uteke import UtekeMemory

mem = UtekeMemory()
mid = mem.remember("Deploy v2.1 to staging", tags=["deploy"])
results = mem.recall("deployment steps")
```

See [`examples/python_uteke.py`](examples/python_uteke.py).

## Troubleshooting

### Build fails: `cc` not found
```bash
# macOS
xcode-select --install

# Ubuntu/Debian
sudo apt install build-essential

# Windows
# Install Visual Studio Build Tools via winget or visualstudio.com
```

### Model download fails
Check internet connection. Model downloaded from HuggingFace:
```
https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX
```

Manual download:
```bash
# macOS/Linux
mkdir -p ~/.codecora/uteke/models/embeddinggemma-q4/onnx
# Download model_q4.onnx and model_q4.onnx_data to above directory

# Windows
mkdir C:\Users\you\.uteke\models\embeddinggemma-q4\onnx
# Download model_q4.onnx and model_q4.onnx_data to above directory
```

### `uteke: command not found`
```bash
# macOS/Linux — Add Cargo bin to PATH
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc

# Windows — Add to PATH via PowerShell
[Environment]::SetEnvironmentVariable("PATH", $env:PATH + ";$env:USERPROFILE\.cargo\bin", "User")
```

## Uninstall

```bash
# Remove binary
cargo uninstall uteke

# Remove all data (memories, models, config)
# macOS/Linux
rm -rf ~/.codecora/uteke

# Windows
Remove-Item -Recurse -Force "$env:USERPROFILE\.uteke"
```
