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

# Terminal colors — disabled when stdout is not a tty or NO_COLOR is set.
# https://no-color.org/
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    C_GREEN=$(printf '\033[32m')
    C_YELLOW=$(printf '\033[33m')
    C_RED=$(printf '\033[31m')
    C_BOLD=$(printf '\033[1m')
    C_RESET=$(printf '\033[0m')
else
    C_GREEN='' C_YELLOW='' C_RED='' C_BOLD='' C_RESET=''
fi

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
        *) echo "  ${C_RED}Unsupported architecture: $ARCH${C_RESET}"; exit 1 ;;
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
            echo "  ${C_RED}Unsupported OS: $OS${C_RESET}"
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

    # When detect_user_shell returns empty (rare — curl|sh with unusual ps
    # output), fall back to $SHELL before guessing by file existence. $SHELL
    # is set by login and is usually accurate even inside the sh subshell.
    SHELL_RC=$(shell_rc_from_shell "${SHELL:-}")
    if [ -n "$SHELL_RC" ]; then
        printf "%s\n" "$SHELL_RC"
        return 0
    fi

    # Last resort: pick by file existence. Prefer .zshrc: bashrc exists on
    # many distros by default even for zsh users, so bashrc-first would
    # quietly write PATH into the wrong rc for anyone whose shell detection
    # failed upstream (then zsh can't see librefang).
    if [ -f "$HOME/.zshrc" ]; then
        printf "%s\n" "$HOME/.zshrc"
    elif [ -f "$HOME/.bashrc" ]; then
        printf "%s\n" "$HOME/.bashrc"
    elif [ -f "$HOME/.config/fish/config.fish" ]; then
        printf "%s\n" "$HOME/.config/fish/config.fish"
    else
        printf "\n"
    fi
}

start_daemon_if_needed() {
    START_OUTPUT=$("$INSTALL_DIR/librefang" start 2>&1) && START_EXIT=0 || START_EXIT=$?

    if [ "$START_EXIT" -eq 0 ]; then
        return 0
    fi
    if printf "%s" "$START_OUTPUT" | grep -Eiq "already running"; then
        echo "  ${C_GREEN}Daemon is already running — no action needed.${C_RESET}"
        return 0
    fi
    # Only dump raw output on unexpected failures; filter out tracing
    # log lines (timestamps like "2026-04-20T...") that clutter the
    # installer output.
    if [ -n "$START_OUTPUT" ]; then
        printf "%s\n" "$START_OUTPUT" | grep -vE '^[0-9]{4}-[0-9]{2}-[0-9]{2}T' || true
    fi
    return "$START_EXIT"
}

install() {
    detect_platform

    echo ""
    echo "  ${C_BOLD}LibreFang Installer${C_RESET}"
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

    # Detect previous version for upgrade messaging.
    OLD_VERSION=""
    if [ -x "$INSTALL_DIR/librefang" ]; then
        OLD_VERSION=$("$INSTALL_DIR/librefang" --version 2>/dev/null || true)
    fi

    echo "  Installing LibreFang $VERSION for $PLATFORM..."
    mkdir -p "$INSTALL_DIR"

    TMPDIR=$(mktemp -d)
    ARCHIVE="$TMPDIR/librefang.tar.gz"
    CHECKSUM_FILE="$TMPDIR/checksum.sha256"

    cleanup() { rm -rf "$TMPDIR"; }
    trap cleanup 0

    # Show a progress bar for the binary download (typically ~60 MB).
    # Use --progress-bar when stderr is a terminal, otherwise stay silent.
    if [ -t 2 ]; then
        CURL_PROGRESS="--progress-bar"
    else
        CURL_PROGRESS="-s"
    fi

    if ! curl -fL $CURL_PROGRESS "$URL" -o "$ARCHIVE"; then
        if [ -n "${PLATFORM_FALLBACK:-}" ]; then
            echo "  ${C_YELLOW}Static (musl) binary not available, trying glibc build...${C_RESET}"
            PLATFORM="$PLATFORM_FALLBACK"
            URL="https://github.com/$REPO/releases/download/$VERSION/librefang-$PLATFORM.tar.gz"
            CHECKSUM_URL="$URL.sha256"
            if ! curl -fL $CURL_PROGRESS "$URL" -o "$ARCHIVE"; then
                echo "  ${C_RED}Download failed.${C_RESET}"
                echo "    URL: $URL"
                echo "  Install from source instead:"
                echo "    cargo install --git https://github.com/$REPO librefang-cli"
                exit 1
            fi
        else
            echo "  ${C_RED}Download failed.${C_RESET}"
            echo "    URL: $URL"
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
                echo "  ${C_RED}Checksum verification FAILED!${C_RESET}"
                echo "    Expected: $EXPECTED"
                echo "    Got:      $ACTUAL"
                exit 1
            fi
            echo "  ${C_GREEN}Checksum verified.${C_RESET}"
        else
            echo "  ${C_YELLOW}No sha256sum/shasum found, skipping checksum verification.${C_RESET}"
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
                echo "  ${C_YELLOW}Warning: ad-hoc code signing failed.${C_RESET}"
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

                # Match the actual install path, not any line mentioning
                # "librefang" — otherwise usernames, oh-my-zsh plugin paths,
                # or comments containing the word silently skip the append.
                if ! grep -qE "\.librefang/bin" "$SHELL_RC" 2>/dev/null; then
                    echo "fish_add_path \"$INSTALL_DIR\"" >> "$SHELL_RC"
                    echo "  ${C_GREEN}Added $INSTALL_DIR to PATH in $SHELL_RC${C_RESET}"
                fi
                ;;
            *)
                if ! grep -qE "\.librefang/bin" "$SHELL_RC" 2>/dev/null; then
                    echo "export PATH=\"$INSTALL_DIR:\$PATH\"" >> "$SHELL_RC"
                    echo "  ${C_GREEN}Added $INSTALL_DIR to PATH in $SHELL_RC${C_RESET}"
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
        if [ -n "$OLD_VERSION" ] && [ "$OLD_VERSION" != "$INSTALLED_VERSION" ]; then
            echo "  ${C_GREEN}LibreFang upgraded successfully!${C_RESET} ($OLD_VERSION -> ${C_BOLD}$INSTALLED_VERSION${C_RESET})"
        else
            echo "  ${C_GREEN}LibreFang installed successfully!${C_RESET} (${C_BOLD}$INSTALLED_VERSION${C_RESET})"
        fi
    else
        echo ""
        echo "  LibreFang binary installed to $INSTALL_DIR/librefang"
    fi

    # Auto-initialize (sync registry, generate config).
    # When piped through `curl | sh`, stdin is not a TTY so librefang init
    # cannot prompt for provider keys and silently falls back to defaults.
    # Only run init interactively when stdin is a real terminal.
    if [ -t 0 ]; then
        echo ""
        echo "  The setup wizard will guide you through provider selection"
        echo "  and configuration."
        echo ""
        echo "  Running setup wizard..."
        "$INSTALL_DIR/librefang" init || true
    fi

    AUTO_START="${LIBREFANG_AUTO_START:-1}"
    if is_enabled "$AUTO_START"; then
        # Register boot service so LibreFang starts on login/reboot.
        # Suppress verbose output (systemd hints, ✔ lines) — only show
        # errors so the installer output stays clean.
        echo "  Registering boot service..."
        SVC_OUTPUT=$("$INSTALL_DIR/librefang" service install 2>&1) || {
            echo "  ${C_YELLOW}Warning: boot service registration failed.${C_RESET}"
            if [ -n "$SVC_OUTPUT" ]; then
                printf "%s\n" "$SVC_OUTPUT" | sed 's/^/    /'
            fi
        }

        echo "  Starting daemon in background..."
        start_daemon_if_needed || {
            echo ""
            echo "  ${C_YELLOW}Warning: automatic daemon start failed.${C_RESET}"
            echo "  Start it manually with:"
            echo "    $INSTALL_DIR/librefang start"
        }
    fi

    # -- Post-install: activate PATH in current session ------------------------
    #
    # Interactive mode (user ran `sh install.sh`):
    #   Restart the shell via `exec` so the rc file is re-read and PATH
    #   takes effect immediately — no manual action required.
    #
    # Pipe mode (`curl … | sh`):
    #   `exec` would replace the sh subshell with a login shell whose stdin
    #   is still the pipe (already drained) — the shell would exit or hang.
    #   Print a prominent banner instead.

    if [ -t 0 ]; then
        # Interactive --------------------------------------------------------
        echo ""
        echo "  Next steps:"
        echo "    librefang chat       # start chatting"
        echo "    librefang stop       # stop the daemon"
        echo ""
        echo "  Installed to: $INSTALL_DIR"
        if [ -n "$SHELL_RC" ]; then
            echo "  Uninstall:    rm -rf \"\$HOME/.librefang\" && remove the PATH line from $SHELL_RC"
        else
            echo "  Uninstall:    rm -rf \"\$HOME/.librefang\""
        fi

        if [ "$SESSION_NEEDS_PATH_REFRESH" -eq 1 ]; then
            # Pick a shell to exec into.  Prefer $SHELL (login shell, survives
            # subshells) over the detected USER_SHELL.  Only exec when we
            # actually wrote the PATH to an rc file the shell will read.
            RESTART_SHELL="${SHELL:-}"
            [ -n "$RESTART_SHELL" ] || RESTART_SHELL="$USER_SHELL"

            if [ -n "$RESTART_SHELL" ] && [ -n "$SHELL_RC" ] && command_exists "$RESTART_SHELL"; then
                echo ""
                echo "  Restarting your shell to activate PATH..."
                # exec replaces the process — EXIT trap won't fire.
                # Clean up the download temp dir manually.
                rm -rf "$TMPDIR" 2>/dev/null || true
                case "$RESTART_SHELL" in
                    */fish|fish) exec "$RESTART_SHELL" --login ;;
                    *)           exec "$RESTART_SHELL" -l ;;
                esac
            else
                # Cannot exec — fall back to a manual hint.
                echo ""
                echo "  To activate PATH in this session, run:"
                case "$USER_SHELL" in
                    */fish|fish) echo "    fish_add_path \"$INSTALL_DIR\"" ;;
                    *)           echo "    export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
                esac
            fi
        fi
        echo ""
    else
        # Pipe mode ----------------------------------------------------------
        echo ""
        echo "  Next steps:"
        echo "    1. Refresh your PATH (see below)"
        echo "    2. librefang init       # setup wizard"
        echo "    3. librefang chat       # start chatting"
        echo ""
        echo "  Installed to: $INSTALL_DIR"
        if [ -n "$SHELL_RC" ]; then
            echo "  Uninstall:    rm -rf \"\$HOME/.librefang\" && remove the PATH line from $SHELL_RC"
        else
            echo "  Uninstall:    rm -rf \"\$HOME/.librefang\""
        fi

        if [ "$SESSION_NEEDS_PATH_REFRESH" -eq 1 ]; then
            echo ""
            echo "  ========================================================"
            echo "  ${C_BOLD}To use 'librefang', first refresh your PATH:${C_RESET}"
            echo ""
            case "$USER_SHELL" in
                */fish|fish) echo "    fish_add_path \"$INSTALL_DIR\"" ;;
                *)           echo "    export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
            esac
            echo ""
            if [ -n "$SHELL_RC" ]; then
                echo "  Or just open a new terminal — $SHELL_RC already"
                echo "  has the PATH entry and new shells will pick it up."
            fi
            echo "  ========================================================"
        fi
        echo ""
    fi
}

if [ "${LIBREFANG_INSTALLER_SOURCE_ONLY:-0}" = "1" ]; then
    return 0 2>/dev/null || exit 0
fi

install
