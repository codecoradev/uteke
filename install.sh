#!/usr/bin/env sh
# uteke installer — https://github.com/codecoradev/uteke
# Usage: curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh

set -e

REPO="codecoradev/uteke"
BINARY_NAME="uteke"
SERVER_BINARY_NAME="uteke-serve"
MCP_BINARY_NAME="uteke-mcp"
INSTALL_DIR="${UTEKE_INSTALL_DIR:-$HOME/.local/bin}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info() {
    printf "${GREEN}[INFO]${NC} %s\n" "$1"
}

warn() {
    printf "${YELLOW}[WARN]${NC} %s\n" "$1"
}

error() {
    printf "${RED}[ERROR]${NC} %s\n" "$1"
    exit 1
}

# Detect OS
detect_os() {
    case "$(uname -s)" in
        Linux*)  OS="linux";;
        Darwin*) OS="darwin";;
        *)       error "Unsupported operating system: $(uname -s)";;
    esac
}

# Detect architecture
detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)  ARCH="x86_64";;
        arm64|aarch64) ARCH="aarch64";;
        *)             error "Unsupported architecture: $(uname -m)";;
    esac
}

# Get latest release version
# Primary: parse the 302 redirect on /releases/latest (no API call, no rate limit).
# Fallback: the GitHub REST API (subject to 60 req/hour anonymous limit).
get_latest_version() {
    VERSION=$(curl -sI "https://github.com/${REPO}/releases/latest" \
        | grep -i '^location:' \
        | sed -E 's|.*/tag/([^[:space:]]+).*|\1|' \
        | tr -d '\r')

    if [ -z "$VERSION" ]; then
        warn "Redirect lookup failed, falling back to GitHub API..."
        VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name":' \
            | sed -E 's/.*"([^"]+)".*/\1/')
    fi

    if [ -z "$VERSION" ]; then
        error "Failed to get latest version. Set UTEKE_VERSION=vX.Y.Z to pin a version."
    fi
}

# Build target triple and archive name
get_target() {
    case "$OS" in
        linux)
            case "$ARCH" in
                x86_64)  TARGET="x86_64-unknown-linux-gnu";;
                aarch64) TARGET="aarch64-unknown-linux-gnu";;
            esac
            ;;
        darwin)
            # Only aarch64 (Apple Silicon) is currently published
            if [ "$ARCH" != "aarch64" ]; then
                warn "No pre-built binary for x86_64 macOS. Install via cargo:"
                warn "  cargo install --path crates/uteke-cli"
                exit 0
            fi
            TARGET="aarch64-apple-darwin"
            ;;
    esac
}

# Download and install
install() {
    info "Detected: $OS $ARCH"
    info "Target: $TARGET"
    info "Version: $VERSION"

    ARCHIVE_NAME="${BINARY_NAME}-${TARGET}-${VERSION}.tar.gz"
    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE_NAME}"
    TEMP_DIR=$(mktemp -d)
    ARCHIVE="${TEMP_DIR}/${ARCHIVE_NAME}"

    CHECKSUMS_URL="https://github.com/${REPO}/releases/download/${VERSION}/checksums-sha256.txt"
    CHECKSUM_FILE="${TEMP_DIR}/checksums-sha256.txt"

    info "Downloading from: $DOWNLOAD_URL"
    if ! curl -fsSL "$DOWNLOAD_URL" -o "$ARCHIVE"; then
        error "Failed to download ${ARCHIVE_NAME}"
    fi

    # Verify SHA256 checksum (prevents MITM / corrupted download).
    info "Downloading checksums..."
    if curl -fsSL "$CHECKSUMS_URL" -o "$CHECKSUM_FILE"; then
        info "Verifying SHA256 checksum..."
        EXPECTED=$(grep -F "$ARCHIVE_NAME" "$CHECKSUM_FILE" | awk '{print $1}')
        if [ -n "$EXPECTED" ]; then
            ACTUAL=$(sha256sum "$ARCHIVE" | awk '{print $1}')
            if [ "$ACTUAL" != "$EXPECTED" ]; then
                error "Checksum mismatch! Expected: ${EXPECTED}, got: ${ACTUAL}"
            fi
            info "Checksum verified: $EXPECTED"
        else
            warn "Checksum for ${ARCHIVE_NAME} not found in checksums file — skipping verification"
        fi
    else
        warn "Failed to download checksums — skipping verification"
    fi

    # Verify archive contents before extraction (CWE-22 path traversal).
    # Reject any entry with an absolute path or a ".." component.
    info "Verifying archive integrity..."
    if tar -tzf "$ARCHIVE" | grep -qE '^/|(^|/)\.\.(/|$)'; then
        error "Archive contains unsafe paths (absolute or directory traversal) — refusing to extract"
    fi

    info "Extracting..."
    tar -xzf "$ARCHIVE" -C "$TEMP_DIR"

    mkdir -p "$INSTALL_DIR"
    mv "${TEMP_DIR}/${BINARY_NAME}" "${INSTALL_DIR}/"
    if [ -f "${TEMP_DIR}/${SERVER_BINARY_NAME}" ]; then
        mv "${TEMP_DIR}/${SERVER_BINARY_NAME}" "${INSTALL_DIR}/"
    fi
    if [ -f "${TEMP_DIR}/${MCP_BINARY_NAME}" ]; then
        mv "${TEMP_DIR}/${MCP_BINARY_NAME}" "${INSTALL_DIR}/"
    fi

    # Install bundled ONNX Runtime shared libs (required for embeddings).
    # Release tarballs ship libonnxruntime.so* + providers_shared alongside the
    # binaries (see release.yml Package step). Without these, `uteke remember`
    # fails with "ONNX Runtime library not found" on distros with no system
    # lib (e.g. Arch/CachyOS) — see resolve_ort_lib() in ort_init.rs which
    # expects <exe_dir>/libonnxruntime.so on AVX2 CPUs.
    # Mirrors build_from_source() which already copies ort-lib/*.so.
    cp -a "${TEMP_DIR}"/libonnxruntime.so* "${INSTALL_DIR}/" 2>/dev/null || true
    cp -a "${TEMP_DIR}"/libonnxruntime_providers_shared.so* "${INSTALL_DIR}/" 2>/dev/null || true
    cp -a "${TEMP_DIR}"/libonnxruntime*.dylib "${INSTALL_DIR}/" 2>/dev/null || true

    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
    if [ -f "${INSTALL_DIR}/${SERVER_BINARY_NAME}" ]; then
        chmod +x "${INSTALL_DIR}/${SERVER_BINARY_NAME}"
    fi
    if [ -f "${INSTALL_DIR}/${MCP_BINARY_NAME}" ]; then
        chmod +x "${INSTALL_DIR}/${MCP_BINARY_NAME}"
    fi

    # Cleanup
    rm -rf "$TEMP_DIR"

    info "Successfully installed ${BINARY_NAME} to ${INSTALL_DIR}/${BINARY_NAME}"
    if [ -f "${INSTALL_DIR}/${SERVER_BINARY_NAME}" ]; then
        info "Successfully installed ${SERVER_BINARY_NAME} to ${INSTALL_DIR}/${SERVER_BINARY_NAME}"
    fi
    if [ -f "${INSTALL_DIR}/${MCP_BINARY_NAME}" ]; then
        info "Successfully installed ${MCP_BINARY_NAME} to ${INSTALL_DIR}/${MCP_BINARY_NAME}"
    fi

    # GLIBC compatibility check — verify binary actually runs
    if [ "$OS" = "linux" ]; then
        GLIBC_TEST=$("${INSTALL_DIR}/${BINARY_NAME}" --version 2>&1 || true)
        if echo "$GLIBC_TEST" | grep -qi "GLIBC.*not found"; then
            echo ""
            error "${GLIBC_TEST}

Your system's GLIBC is too old for the prebuilt binary.
Options:
  1. Build from source:
     curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sh -s -- --from-source
  2. Upgrade your OS to Ubuntu 22.04+ or Debian 12+
  3. Use Docker: docker run --rm ghcr.io/${REPO}:latest"
        fi
    fi
}

# Build from source fallback
build_from_source() {
    info "Building from source..."

    if ! command -v cargo >/dev/null 2>&1; then
        error "cargo not found. Install Rust: https://rustup.rs"
    fi

    info "Cloning repository..."
    TEMP_DIR=$(mktemp -d)

    git clone --depth 1 "https://github.com/${REPO}.git" "$TEMP_DIR" || error "Clone failed."

    cd "$TEMP_DIR"
    info "Building release binary (this may take a few minutes)..."

    # Download ORT library for linking
    ORT_VER="1.24.4"
    ORT_PKG=""
    case "$(uname -m)" in
        x86_64|amd64)  ORT_PKG="onnxruntime-linux-x64-${ORT_VER}.tgz" ;;
        arm64|aarch64) ORT_PKG="onnxruntime-linux-aarch64-${ORT_VER}.tgz" ;;
        *)             error "Unsupported architecture for source build: $(uname -m)" ;;
    esac
    info "Downloading ONNX Runtime ${ORT_VER}..."
    curl -fsSL "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VER}/${ORT_PKG}" -o /tmp/ort-pkg.tgz || error "Failed to download ORT"
    tar xzf /tmp/ort-pkg.tgz -C "$TEMP_DIR" || error "Failed to extract ORT"
    rm -f /tmp/ort-pkg.tgz
    mkdir -p ort-lib
    find onnxruntime-*/lib -name 'libonnxruntime.so*' \( -type f -o -type l \) -exec cp -a {} ort-lib/ \;
    export ORT_LIB_DIR=ort-lib

    cargo build --release -p uteke-cli -p uteke-server -p uteke-mcp || {
        cd /; rm -rf "$TEMP_DIR"; error "Build failed."
    }

    mkdir -p "$INSTALL_DIR"
    cp target/release/${BINARY_NAME} "${INSTALL_DIR}/"
    cp target/release/${SERVER_BINARY_NAME} "${INSTALL_DIR}/" 2>/dev/null || true
    cp target/release/${MCP_BINARY_NAME} "${INSTALL_DIR}/" 2>/dev/null || true
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

    # Copy ORT .so next to binary and set up loader path
    cp ort-lib/libonnxruntime.so* "${INSTALL_DIR}/" 2>/dev/null || true

    # Cleanup
    cd /
    rm -rf "$TEMP_DIR"

    info "✓ Installed from source to ${INSTALL_DIR}/${BINARY_NAME}"
}

# Verify installation
verify() {
    if command -v "$BINARY_NAME" >/dev/null 2>&1; then
        INSTALLED_VERSION=$("$BINARY_NAME" --version 2>/dev/null || echo "unknown")
        info "Verification: $INSTALLED_VERSION"
    else
        warn "Binary installed but not in PATH. Add to your shell profile:"
        case "${SHELL:-}" in
            */zsh)
                warn '  echo '\''export PATH="$HOME/.local/bin:$PATH"'\'' >> ~/.zshrc'
                warn '  source ~/.zshrc'
                ;;
            */bash)
                warn '  echo '\''export PATH="$HOME/.local/bin:$PATH"'\'' >> ~/.bashrc'
                warn '  source ~/.bashrc'
                ;;
            */fish)
                warn '  fish_add_path ~/.local/bin'
                ;;
            *)
                warn '  export PATH="$HOME/.local/bin:$PATH"'
                ;;
        esac
    fi
}

main() {
    # --from-source flag: skip binary download, compile directly
    if [ "${1:-}" = "--from-source" ]; then
        info "Installing ${BINARY_NAME} from source..."
        build_from_source
        verify
        echo ""
        info "Installation complete! Run '${BINARY_NAME} --help' to get started."
        return
    fi

    info "Installing ${BINARY_NAME}..."

    detect_os
    detect_arch
    get_target
    if [ -n "$UTEKE_VERSION" ]; then
        VERSION="$UTEKE_VERSION"
        info "Using pinned version from UTEKE_VERSION: $VERSION"
    else
        get_latest_version
    fi
    install
    verify

    echo ""
    info "Installation complete! Run '${BINARY_NAME} --help' to get started."
}

main "$@"
