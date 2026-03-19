#!/bin/sh
# LibreFang installer — works on Linux, macOS, WSL, and minimal containers
# Usage: curl -fsSL https://librefang.ai/install.sh | sh
#
# Environment variables:
#   LIBREFANG_INSTALL_DIR — custom install directory (default: ~/.librefang/bin)
#   LIBREFANG_VERSION    — install a specific version tag (default: latest)

# Use POSIX-compatible syntax for max compatibility (dash, ash, busybox, etc.)
# Avoid: pipefail, [[ ]], (( )), source, local, etc.

REPO="librefang/librefang"
INSTALL_DIR="${LIBREFANG_INSTALL_DIR:-$HOME/.librefang/bin}"

# Simple error handling without pipefail
warn() {
    echo "  $*" >&2
}

die() {
    warn "$@"
    exit 1
}

detect_platform() {
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)

    # Normalize architecture
    case "$ARCH" in
        x86_64|amd64) ARCH="x86_64" ;;
        aarch64|arm64) ARCH="aarch64" ;;
        *) die "Unsupported architecture: $ARCH" ;;
    esac

    # Detect platform
    case "$OS" in
        linux)
            # Prefer musl (fully static) binaries - work on any distro
            # without glibc/libssl dependencies. Fall back to gnu.
            PLATFORM="${ARCH}-unknown-linux-musl"
            PLATFORM_FALLBACK="${ARCH}-unknown-linux-gnu"
            ;;
        darwin) PLATFORM="${ARCH}-apple-darwin" ;;
        mingw*|msys*|cygwin*)
            echo ""
            echo "  For Windows, use PowerShell instead:"
            echo "    irm https://librefang.ai/install.ps1 | iex"
            echo ""
            echo "  Or download the .msi from:"
            echo "    https://github.com/$REPO/releases/latest"
            echo ""
            echo "  Or install via cargo:"
            echo "    cargo install --git https://github.com/$REPO librefang-cli"
            exit 1
            ;;
        *) die "Unsupported OS: $OS" ;;
    esac
}

# Cross-platform command check
has_command() {
    command -v "$1" >/dev/null 2>&1
}

# Download with fallback options
do_curl() {
    if has_command curl; then
        curl -fsSL "$1" -o "$2"
    elif has_command wget; then
        wget -q -O "$2" "$1"
    else
        die "Neither curl nor wget found. Install one to continue."
    fi
}

install() {
    detect_platform

    echo ""
    echo "  LibreFang Installer"
    echo "  ==================="
    echo ""

    # Get latest version
    REQUESTED_VERSION="${LIBREFANG_VERSION:-}"
    if [ -n "$REQUESTED_VERSION" ]; then
        VERSION="$REQUESTED_VERSION"
        echo "  Using specified version: $VERSION"
    else
        echo "  Fetching latest release..."
        # Use curl or wget to fetch API response
        API_RESPONSE=$(mktemp)
        if has_command curl; then
            curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" -o "$API_RESPONSE" 2>/dev/null
        elif has_command wget; then
            wget -q -O "$API_RESPONSE" "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null
        fi
        if [ -s "$API_RESPONSE" ]; then
            VERSION=$(grep '"tag_name"' "$API_RESPONSE" | head -1 | cut -d'"' -f4)
        fi
        rm -f "$API_RESPONSE"
    fi

    if [ -z "$VERSION" ]; then
        die "No GitHub Releases found for $REPO. Install from source: cargo install --git https://github.com/$REPO librefang-cli"
    fi

    URL="https://github.com/$REPO/releases/download/$VERSION/librefang-$PLATFORM.tar.gz"
    echo "  Installing LibreFang $VERSION for $PLATFORM..."

    # Create install dir
    mkdir -p "$INSTALL_DIR" || die "Cannot create $INSTALL_DIR"

    # Download to temp
    TMPDIR=$(mktemp -d)
    ARCHIVE="$TMPDIR/librefang.tar.gz"

    # Cleanup on exit
    trap "rm -rf $TMPDIR" EXIT

    # Try downloading
    if ! do_curl "$URL" "$ARCHIVE"; then
        # Fall back from musl to gnu if musl asset not available
        if [ -n "$PLATFORM_FALLBACK" ]; then
            echo "  Static (musl) binary not found, trying glibc build..."
            PLATFORM="$PLATFORM_FALLBACK"
            URL="https://github.com/$REPO/releases/download/$VERSION/librefang-$PLATFORM.tar.gz"
            if ! do_curl "$URL" "$ARCHIVE"; then
                die "Download failed. Install from source: cargo install --git https://github.com/$REPO librefang-cli"
            fi
        else
            die "Download failed. Install from source: cargo install --git https://github.com/$REPO librefang-cli"
        fi
    fi

    # Extract
    tar xzf "$ARCHIVE" -C "$INSTALL_DIR" || die "Failed to extract archive"
    chmod +x "$INSTALL_DIR/librefang"

    # Verify installation
    if [ -x "$INSTALL_DIR/librefang" ]; then
        echo ""
        echo "  LibreFang installed to $INSTALL_DIR/librefang"
    fi

    echo ""
    echo "  Get started:"
    echo "    $INSTALL_DIR/librefang init"
    echo ""
    echo "  Add to PATH:"
    echo "    export PATH=\"$INSTALL_DIR:\$PATH\""
    echo ""
    echo "  The setup wizard will guide you through configuration."
    echo ""
}

install
