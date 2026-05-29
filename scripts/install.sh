#!/usr/bin/env bash
#
# Uteke quick install script
# Usage: curl -fsSL https://raw.githubusercontent.com/ajianaz/uteke/develop/scripts/install.sh | sh
#
set -euo pipefail

REPO="ajianaz/uteke"
INSTALL_DIR="${HOME}/.local/bin"
BINARY_NAME="uteke"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

info()  { echo -e "${GREEN}[info]${NC}  $*"; }
warn()  { echo -e "${YELLOW}[warn]${NC}  $*"; }
error() { echo -e "${RED}[error]${NC} $*" >&2; exit 1; }

# Detect platform
detect_platform() {
    local os arch

    case "$(uname -s)" in
        Linux*)  os="unknown-linux-gnu" ;;
        Darwin*) os="apple-darwin" ;;
        *)       error "Unsupported OS: $(uname -s)" ;;
    esac

    case "$(uname -m)" in
        x86_64*|amd64*)  arch="x86_64" ;;
        aarch64*|arm64*)  arch="aarch64" ;;
        *)                error "Unsupported architecture: $(uname -m)" ;;
    esac

    echo "${arch}-${os}"
}

# Get latest release version
get_latest_version() {
    local version
    version=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null | grep '"tag_name"' | head -1 | sed -E 's/.*"([^"]+)".*/\1/')

    if [ -z "$version" ]; then
        # Fallback: try to get from releases page
        version=$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest" 2>/dev/null | sed 's|.*/v\?||')
    fi

    if [ -z "$version" ]; then
        error "Could not determine latest version. Check your internet connection."
    fi

    echo "$version"
}

# Download and install
install_uteke() {
    local platform version url ext

    platform=$(detect_platform)
    info "Detected platform: ${platform}"

    # Check if already installed
    if command -v "${BINARY_NAME}" &>/dev/null; then
        local current_version
        current_version=$("${BINARY_NAME}" --version 2>/dev/null | head -1 | awk '{print $NF}' || true)
        if [ -n "$current_version" ]; then
            warn "Uteke ${current_version} is already installed at $(command -v ${BINARY_NAME})"
            info "Reinstalling..."
        fi
    fi

    version=$(get_latest_version)
    info "Latest version: ${version}"

    # Determine archive extension
    case "$(uname -s)" in
        Darwin*) ext="tar.gz" ;;
        Linux*)  ext="tar.gz" ;;
    esac

    local artifact="${BINARY_NAME}-${platform}"
    url="https://github.com/${REPO}/releases/download/${version}/${artifact}.${ext}"

    info "Downloading ${url}..."

    # Create temp directory
    local tmp_dir
    tmp_dir=$(mktemp -d)
    trap 'rm -rf "${tmp_dir}"' EXIT

    local archive="${tmp_dir}/${artifact}.${ext}"

    # Download
    curl -fsSL -o "${archive}" "${url}" || error "Download failed. The release may not include your platform yet."

    # Extract
    info "Extracting..."
    tar xzf "${archive}" -C "${tmp_dir}" || error "Extraction failed."

    # Find the binary
    local binary
    binary=$(find "${tmp_dir}" -name "${BINARY_NAME}" -type f | head -1)

    if [ -z "${binary}" ]; then
        # Maybe it's the artifact name
        binary="${tmp_dir}/${artifact}"
    fi

    if [ ! -f "${binary}" ]; then
        error "Could not find uteke binary in archive."
    fi

    # Install
    mkdir -p "${INSTALL_DIR}"
    cp "${binary}" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

    info "Installed to ${INSTALL_DIR}/${BINARY_NAME}"

    # Verify
    if command -v "${BINARY_NAME}" &>/dev/null; then
        local installed_version
        installed_version=$("${BINARY_NAME}" --version 2>/dev/null || echo "unknown")
        info "✓ Uteke ${installed_version} installed successfully!"
    else
        warn "Uteke installed but not in PATH."
        echo ""
        echo "  Add to your shell config:"
        echo ""
        echo "    echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc  # or ~/.zshrc"
        echo "    source ~/.bashrc  # or source ~/.zshrc"
        echo ""
    fi
}

# Build from source fallback
install_from_source() {
    info "Installing from source..."

    if ! command -v cargo &>/dev/null; then
        error "cargo not found. Install Rust: https://rustup.rs"
    fi

    info "Cloning repository..."
    local tmp_dir
    tmp_dir=$(mktemp -d)
    trap 'rm -rf "${tmp_dir}"' EXIT

    git clone --depth 1 "https://github.com/${REPO}.git" "${tmp_dir}" || error "Clone failed."

    cd "${tmp_dir}"
    info "Building release binary (this may take a few minutes)..."
    cargo build --release -p uteke-cli || error "Build failed."

    mkdir -p "${INSTALL_DIR}"
    cp target/release/${BINARY_NAME} "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

    info "✓ Installed from source to ${INSTALL_DIR}/${BINARY_NAME}"
}

# Main
main() {
    echo ""
    echo "  🧠 Uteke — Local-first AI memory engine"
    echo ""
    echo "  https://github.com/${REPO}"
    echo ""

    # Try binary release first, fallback to source
    if install_uteke 2>/dev/null; then
        :
    else
        warn "Binary release not available for your platform."
        info "Falling back to building from source..."
        install_from_source
    fi
}

main "$@"
