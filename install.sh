#!/usr/bin/env sh
# uteke installer — https://github.com/codecoradev/uteke
# Usage: curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh

set -e

REPO="codecoradev/uteke"
BINARY_NAME="uteke"
SERVER_BINARY_NAME="uteke-serve"
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

    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
    if [ -f "${INSTALL_DIR}/${SERVER_BINARY_NAME}" ]; then
        chmod +x "${INSTALL_DIR}/${SERVER_BINARY_NAME}"
    fi

    # Cleanup
    rm -rf "$TEMP_DIR"

    info "Successfully installed ${BINARY_NAME} to ${INSTALL_DIR}/${BINARY_NAME}"
    if [ -f "${INSTALL_DIR}/${SERVER_BINARY_NAME}" ]; then
        info "Successfully installed ${SERVER_BINARY_NAME} to ${INSTALL_DIR}/${SERVER_BINARY_NAME}"
    fi
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

main
