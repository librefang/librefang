#!/bin/sh
# LibreFang installer - works on Linux, macOS, WSL
# Usage: curl -fsSL https://librefang.ai/install.sh | sh
#
# Environment variables:
#   LIBREFANG_INSTALL_DIR         custom install directory (default: ~/.librefang/bin)
#   LIBREFANG_VERSION             install a specific version tag (default: latest)
#   LIBREFANG_AUTO_START          auto-start daemon after install (default: 1)
#                                 accepts: 1/true/yes/on (others disable)
#   LIBREFANG_INSTALLER_SOURCE_ONLY
#                                 test hook; do not auto-run install()

set -eu

REPO="librefang/librefang"
INSTALL_DIR="${LIBREFANG_INSTALL_DIR:-$HOME/.librefang/bin}"

command_exists() {
    command -v "$1" >/dev/null 2>&1
}

is_enabled() {
    case "${1:-}" in
        1|true|TRUE|yes|YES|on|ON) return 0 ;;
        *) return 1 ;;
    esac
}

detect_platform() {
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)
    case "$ARCH" in
        x86_64|amd64) ARCH="x86_64" ;;
        aarch64|arm64) ARCH="aarch64" ;;
        *) echo "  Unsupported architecture: $ARCH"; exit 1 ;;
    esac

    case "$OS" in
        linux)
            # Prefer musl (fully static) binaries. Fall back to gnu if needed.
            PLATFORM="${ARCH}-unknown-linux-musl"
            PLATFORM_FALLBACK="${ARCH}-unknown-linux-gnu"
            ;;
        darwin)
            PLATFORM="${ARCH}-apple-darwin"
            ;;
        mingw*|msys*|cygwin*)
            echo ""
            echo "  For Windows, use PowerShell instead:"
            echo "    irm https://librefang.ai/install.ps1 | iex"
            echo ""
            echo "  Or download the .msi installer from:"
            echo "    https://github.com/$REPO/releases/latest"
            echo ""
            echo "  Or install via cargo:"
            echo "    cargo install --git https://github.com/$REPO librefang-cli"
            exit 1
            ;;
        *)
            echo "  Unsupported OS: $OS"
            exit 1
            ;;
    esac
}

detect_user_shell() {
    USER_SHELL=""

    # For `curl ... | sh`, $SHELL can be stale. Prefer parent process shell.
    if command_exists ps; then
        PARENT_COMM=$(ps -p "$PPID" -o comm= 2>/dev/null | awk '{print $1}')
        PARENT_COMM="${PARENT_COMM##*/}"
        case "$PARENT_COMM" in
            zsh|bash|fish)
                USER_SHELL="$PARENT_COMM"
                ;;
            sh|dash|ash)
                GRANDPARENT_PID=$(ps -p "$PPID" -o ppid= 2>/dev/null | tr -d '[:space:]')
                if [ -n "$GRANDPARENT_PID" ]; then
                    GRANDPARENT_COMM=$(ps -p "$GRANDPARENT_PID" -o comm= 2>/dev/null | awk '{print $1}')
                    GRANDPARENT_COMM="${GRANDPARENT_COMM##*/}"
                    case "$GRANDPARENT_COMM" in
                        zsh|bash|fish) USER_SHELL="$GRANDPARENT_COMM" ;;
                    esac
                fi
                ;;
        esac
    fi

    if [ -z "$USER_SHELL" ]; then
        USER_SHELL="${SHELL:-}"
    fi
    if [ -z "$USER_SHELL" ] && command_exists getent; then
        USER_SHELL=$(getent passwd "$(id -un)" 2>/dev/null | cut -d: -f7)
    fi
    if [ -z "$USER_SHELL" ] && [ -f /etc/passwd ]; then
        USER_SHELL=$(grep "^$(id -un):" /etc/passwd 2>/dev/null | cut -d: -f7)
    fi

    printf "%s\n" "$USER_SHELL"
}

shell_rc_from_shell() {
    case "${1:-}" in
        */zsh|zsh) printf "%s\n" "$HOME/.zshrc" ;;
        */bash|bash) printf "%s\n" "$HOME/.bashrc" ;;
        */fish|fish) printf "%s\n" "$HOME/.config/fish/config.fish" ;;
        *) printf "\n" ;;
    esac
}

choose_shell_rc() {
    SHELL_RC=$(shell_rc_from_shell "${1:-}")
    if [ -n "$SHELL_RC" ]; then
        printf "%s\n" "$SHELL_RC"
        return 0
    fi

    # Check bash/zsh first (more common defaults), fish last.
    if [ -f "$HOME/.bashrc" ]; then
        printf "%s\n" "$HOME/.bashrc"
    elif [ -f "$HOME/.zshrc" ]; then
        printf "%s\n" "$HOME/.zshrc"
    elif [ -f "$HOME/.config/fish/config.fish" ]; then
        printf "%s\n" "$HOME/.config/fish/config.fish"
    else
        printf "\n"
    fi
}

start_daemon_if_needed() {
    START_OUTPUT=$("$INSTALL_DIR/librefang" start 2>&1) && START_EXIT=0 || START_EXIT=$?
    if [ -n "$START_OUTPUT" ]; then
        printf "%s\n" "$START_OUTPUT"
    fi

    if [ "$START_EXIT" -eq 0 ]; then
        return 0
    fi
    if printf "%s" "$START_OUTPUT" | grep -Eiq "already running"; then
        echo "  Daemon already running; leaving it as-is."
        return 0
    fi
    return "$START_EXIT"
}

install() {
    detect_platform

    echo ""
    echo "  LibreFang Installer"
    echo "  ==================="
    echo ""

    REQUESTED_VERSION="${LIBREFANG_VERSION:-}"
    if [ -n "$REQUESTED_VERSION" ]; then
        VERSION="$REQUESTED_VERSION"
        echo "  Using specified version: $VERSION"
    else
        echo "  Fetching latest release..."
        VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null | grep '"tag_name"' | head -1 | cut -d '"' -f 4 || true)
    fi

    if [ -z "$VERSION" ]; then
        echo "  No GitHub Releases are published for $REPO yet."
        echo "  Install from source instead:"
        echo "    cargo install --git https://github.com/$REPO librefang-cli"
        exit 1
    fi

    URL="https://github.com/$REPO/releases/download/$VERSION/librefang-$PLATFORM.tar.gz"
    CHECKSUM_URL="$URL.sha256"

    echo "  Installing LibreFang $VERSION for $PLATFORM..."
    mkdir -p "$INSTALL_DIR"

    TMPDIR=$(mktemp -d)
    ARCHIVE="$TMPDIR/librefang.tar.gz"
    CHECKSUM_FILE="$TMPDIR/checksum.sha256"

    cleanup() { rm -rf "$TMPDIR"; }
    trap cleanup 0

    if ! curl -fsSL "$URL" -o "$ARCHIVE" 2>/dev/null; then
        if [ -n "${PLATFORM_FALLBACK:-}" ]; then
            echo "  Static (musl) binary not available, trying glibc build..."
            PLATFORM="$PLATFORM_FALLBACK"
            URL="https://github.com/$REPO/releases/download/$VERSION/librefang-$PLATFORM.tar.gz"
            CHECKSUM_URL="$URL.sha256"
            if ! curl -fsSL "$URL" -o "$ARCHIVE" 2>/dev/null; then
                echo "  Download failed. The release may not exist for your platform."
                echo "  Install from source instead:"
                echo "    cargo install --git https://github.com/$REPO librefang-cli"
                exit 1
            fi
        else
            echo "  Download failed. The release may not exist for your platform."
            echo "  Install from source instead:"
            echo "    cargo install --git https://github.com/$REPO librefang-cli"
            exit 1
        fi
    fi

    if curl -fsSL "$CHECKSUM_URL" -o "$CHECKSUM_FILE" 2>/dev/null; then
        EXPECTED=$(cut -d ' ' -f 1 < "$CHECKSUM_FILE")
        if command_exists sha256sum; then
            ACTUAL=$(sha256sum "$ARCHIVE" | cut -d ' ' -f 1)
        elif command_exists shasum; then
            ACTUAL=$(shasum -a 256 "$ARCHIVE" | cut -d ' ' -f 1)
        else
            ACTUAL=""
        fi

        if [ -n "$ACTUAL" ]; then
            if [ "$EXPECTED" != "$ACTUAL" ]; then
                echo "  Checksum verification FAILED!"
                echo "    Expected: $EXPECTED"
                echo "    Got:      $ACTUAL"
                exit 1
            fi
            echo "  Checksum verified."
        else
            echo "  No sha256sum/shasum found, skipping checksum verification."
        fi
    fi

    tar xzf "$ARCHIVE" -C "$INSTALL_DIR"
    chmod +x "$INSTALL_DIR/librefang"

    # Ad-hoc codesign on macOS (prevents SIGKILL on Apple Silicon).
    # Remove quarantine xattr before signing.
    if [ "$OS" = "darwin" ]; then
        if command_exists xattr; then
            xattr -cr "$INSTALL_DIR/librefang" 2>/dev/null || true
        fi
        if command_exists codesign; then
            if ! codesign --force --sign - "$INSTALL_DIR/librefang"; then
                echo ""
                echo "  Warning: ad-hoc code signing failed."
                echo "  On Apple Silicon, the binary may be killed (SIGKILL) by Gatekeeper."
                echo "  Try manually: xattr -cr $INSTALL_DIR/librefang && codesign --force --sign - $INSTALL_DIR/librefang"
                echo ""
            fi
        fi
    fi

    USER_SHELL=$(detect_user_shell)
    SHELL_RC=$(choose_shell_rc "$USER_SHELL")

    if [ -n "$SHELL_RC" ]; then
        # Determine syntax from the TARGET FILE, not $USER_SHELL — this
        # prevents Bash syntax from ever being written to config.fish even
        # when shell detection mis-identifies the user's shell.
        case "$SHELL_RC" in
            */config.fish)
                mkdir -p "$(dirname "$SHELL_RC")"

                # Self-heal: remove old Bash-style PATH exports from fish config.
                if [ -f "$SHELL_RC" ]; then
                    TMP_FISH_RC=$(mktemp)
                    grep -vE '^[[:space:]]*export[[:space:]]+PATH=.*(librefang|openfang)' "$SHELL_RC" > "$TMP_FISH_RC" || true
                    if ! cmp -s "$SHELL_RC" "$TMP_FISH_RC" 2>/dev/null; then
                        cat "$TMP_FISH_RC" > "$SHELL_RC"
                        echo "  Removed incompatible Bash PATH export from $SHELL_RC"
                    fi
                    rm -f "$TMP_FISH_RC"
                fi

                if ! grep -q "librefang" "$SHELL_RC" 2>/dev/null; then
                    echo "fish_add_path \"$INSTALL_DIR\"" >> "$SHELL_RC"
                    echo "  Added $INSTALL_DIR to PATH in $SHELL_RC"
                fi
                ;;
            *)
                if ! grep -q "librefang" "$SHELL_RC" 2>/dev/null; then
                    echo "export PATH=\"$INSTALL_DIR:\$PATH\"" >> "$SHELL_RC"
                    echo "  Added $INSTALL_DIR to PATH in $SHELL_RC"
                fi
                ;;
        esac
    fi

    SESSION_NEEDS_PATH_REFRESH=0
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *) SESSION_NEEDS_PATH_REFRESH=1 ;;
    esac

    if "$INSTALL_DIR/librefang" --version >/dev/null 2>&1; then
        INSTALLED_VERSION=$("$INSTALL_DIR/librefang" --version 2>/dev/null || echo "$VERSION")
        echo ""
        echo "  LibreFang installed successfully! ($INSTALLED_VERSION)"
    else
        echo ""
        echo "  LibreFang binary installed to $INSTALL_DIR/librefang"
    fi

    echo ""
    echo "  Get started now:"
    echo "    $INSTALL_DIR/librefang init"
    if [ "$SESSION_NEEDS_PATH_REFRESH" -eq 1 ]; then
        echo ""
        echo "  To use 'librefang' in this shell, run:"
        case "$USER_SHELL" in
            */fish|fish)
                echo "    fish_add_path \"$INSTALL_DIR\""
                ;;
            *)
                echo "    export PATH=\"$INSTALL_DIR:\$PATH\""
                ;;
        esac
        if [ -n "$SHELL_RC" ]; then
            echo "  New shells will pick it up from $SHELL_RC."
        fi
        echo ""
        echo "  After refreshing PATH, you can also run:"
        echo "    librefang init"
    else
        echo ""
        echo "  Or run:"
        echo "    librefang init"
    fi

    echo ""
    echo "  The setup wizard will guide you through provider selection"
    echo "  and configuration."
    echo ""

    # Auto-initialize (sync registry, generate config)
    echo "  Initializing LibreFang..."
    "$INSTALL_DIR/librefang" init 2>/dev/null || true

    AUTO_START="${LIBREFANG_AUTO_START:-1}"
    if is_enabled "$AUTO_START"; then
        # Register boot service so LibreFang starts on login/reboot
        echo "  Registering boot service..."
        "$INSTALL_DIR/librefang" service install 2>/dev/null || true

        echo "  Starting daemon in background..."
        if start_daemon_if_needed; then
            echo ""
            echo "  Next steps:"
            echo "    1. Chat:              $INSTALL_DIR/librefang chat"
            echo "    2. Stop daemon:       $INSTALL_DIR/librefang stop"
        else
            echo ""
            echo "  Warning: automatic daemon start failed."
            echo "  Start it manually with:"
            echo "    $INSTALL_DIR/librefang start"
        fi
        echo ""
    fi
}

if [ "${LIBREFANG_INSTALLER_SOURCE_ONLY:-0}" = "1" ]; then
    return 0 2>/dev/null || exit 0
fi

install
