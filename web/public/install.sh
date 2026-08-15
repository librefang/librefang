#!/bin/sh
# LibreFang installer - works on Linux, macOS, WSL
# Usage: curl -fsSL https://librefang.ai/install.sh | sh
#
# Environment variables:
#   LIBREFANG_INSTALL_DIR         custom install directory (default: ~/.librefang/bin)
#   LIBREFANG_VERSION             install a specific version tag (default: latest)
#   LIBREFANG_PREFERRED_VERSION   prefer a version tag, falling back when it ships no package
#   LIBREFANG_AUTO_START          auto-start daemon after install (default: 1)
#                                 accepts: 1/true/yes/on (others disable)
#   LIBREFANG_OS_RELEASE          test hook; os-release file to parse (default: /etc/os-release)
#   LIBREFANG_NIXOS_MARKER        test hook; NixOS marker path (default: /etc/NIXOS)
#   LIBREFANG_INSTALLER_SOURCE_ONLY
#                                 test hook; do not auto-run install()

set -eu

REPO="librefang/librefang"
INSTALL_DIR="${LIBREFANG_INSTALL_DIR:-$HOME/.librefang/bin}"

# Distro-detection inputs, indirected through variables so the test suite can point them at fixtures.
# The real paths are absolute, which no PATH mock can reach.
OS_RELEASE_FILE="${LIBREFANG_OS_RELEASE:-/etc/os-release}"
NIXOS_MARKER_FILE="${LIBREFANG_NIXOS_MARKER:-/etc/NIXOS}"

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

# --- Distro detection -----------------------------------------------------

# Print the unquoted, lowercased value of the $1 field of the os-release file, or nothing when the file or the field is absent.
# Parsed rather than sourced: os-release is shell-syntax but sourcing it would execute a root-owned file inside the installer and would trip `set -u` on any field the caller does not read.
os_release_field() {
    [ -r "$OS_RELEASE_FILE" ] || return 0
    grep -E "^$1=" "$OS_RELEASE_FILE" 2>/dev/null \
        | head -n 1 \
        | cut -d '=' -f 2- \
        | tr -d "\"'" \
        | tr '[:upper:]' '[:lower:]'
}

# Set DISTRO to the lowercased os-release ID (or "unknown") and DISTRO_LIKE to its ID_LIKE family list.
# A host with no os-release file at all — macOS, minimal containers — degrades to "unknown" and installs exactly as before; distro detection must never be able to fail the install.
detect_distro() {
    DISTRO_LIKE=$(os_release_field ID_LIKE)
    DISTRO=$(os_release_field ID)
    [ -n "$DISTRO" ] || DISTRO="unknown"

    # /etc/NIXOS is the authoritative marker: every NixOS system has it and nothing else does, so it outranks an ID inherited from a container base layer.
    if [ -e "$NIXOS_MARKER_FILE" ]; then
        DISTRO="nixos"
    fi
}

# Return 0 when the host is in the Debian family, by its own ID or by the ID_LIKE list a derivative inherits.
is_debian_family() {
    case " ${DISTRO:-} ${DISTRO_LIKE:-} " in
        *" debian "*|*" ubuntu "*|*" deepin "*) return 0 ;;
    esac
    return 1
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

    # Remember the primary so per-tag resolution can retry the fallback without losing it.
    PLATFORM_PRIMARY="$PLATFORM"
}

# --- Distro-specific install policy ---------------------------------------

# Print the platform variant to fall back to when the primary ships no package, or nothing when this host has none it could execute.
# Single source of truth for the fallback decision, consulted at each point of use rather than applied by mutating PLATFORM_FALLBACK: the detected distro comes from detect_distro, which does not depend on detect_platform, so the outcome cannot change with the order those two run in.
effective_platform_fallback() {
    [ -n "${PLATFORM_FALLBACK:-}" ] || return 0

    # NixOS ships no ELF interpreter at the FHS path a distro-generic build hardcodes — /lib64/ld-linux-x86-64.so.2 on x86_64, /lib/ld-linux-aarch64.so.1 on aarch64 — so the dynamically linked gnu build can never exec there on either architecture.
    # Offered as a fallback it downloads ~60 MB, fails its post-install `--version` check, gets rolled back, and reports only "The new binary failed to run" — which tells a NixOS user nothing.
    # The musl build is fully static and runs on NixOS unmodified, so that variant and only that one is considered there.
    [ "${DISTRO:-unknown}" != "nixos" ] || return 0

    printf "%s\n" "$PLATFORM_FALLBACK"
}

# Return 0 when a failed download of PLATFORM should be retried against the fallback variant.
# Three states: no fallback this host could execute (nothing to retry), a fallback that PLATFORM already holds, and a fallback still worth trying.
should_try_platform_fallback() {
    _stpf=$(effective_platform_fallback)
    [ -n "$_stpf" ] || return 1

    # resolve_platform_for_tag may already have switched PLATFORM to the fallback during probing; retrying then re-fetches the identical URL and reports the musl build as missing when musl was never the target.
    [ "${PLATFORM:-}" != "$_stpf" ] || return 1

    return 0
}

# Tell the user which platform variants the detected distro rules out, before any of them is downloaded.
# Reports only: the decision itself lives in effective_platform_fallback, so nothing here is load-bearing and skipping this call would cost the explanation, not the behaviour.
apply_distro_platform_policy() {
    [ "${DISTRO:-unknown}" = "nixos" ] || return 0
    [ -n "${PLATFORM_FALLBACK:-}" ] || return 0

    echo "  NixOS detected — the glibc build will not be offered as a fallback."
    echo "  NixOS ships no /lib64/ld-linux-x86-64.so.2, so a dynamically linked binary cannot start there."
    echo "  The fully static musl build is used instead; it runs on NixOS unmodified."
}

# Print the fallback install instructions for the detected distro.
# On NixOS the flake is the working route, so pointing at `cargo install` there would send the user down the same dynamic-linking dead end.
print_source_install_hint() {
    if [ "${DISTRO:-unknown}" = "nixos" ]; then
        echo "  On NixOS, install from this repository's flake instead:"
        echo "    nix profile install github:$REPO#librefang-cli"
        echo "  Or run it without installing anything:"
        echo "    nix run github:$REPO"
        echo "  For a daemon that survives reboots, import the flake's nixosModule and set:"
        echo "    services.librefang.enable = true;"
        return 0
    fi

    echo "  Install from source instead:"
    echo "    cargo install --git https://github.com/$REPO librefang-cli"
}

# Print the WebKitGTK API series pkg-config reports on this host ("4.1" or "4.0"), or nothing when pkg-config is absent or reports neither.
probe_webkit2gtk_pkgconfig() {
    command_exists pkg-config || return 0
    for _wk_api in 4.1 4.0; do
        if pkg-config --exists "webkit2gtk-$_wk_api" 2>/dev/null; then
            printf "%s\n" "$_wk_api"
            return 0
        fi
    done
    return 0
}

# Print the first WebKitGTK dev package this host's apt repositories offer an installable candidate for, or nothing.
# Which webkit2gtk series a Debian derivative carries differs per release, so the repositories are asked instead of a name being asserted from here.
webkit2gtk_apt_candidate() {
    command_exists apt-cache || return 0
    for _wk_pkg in libwebkit2gtk-4.1-dev libwebkit2gtk-4.0-dev; do
        # LC_ALL=C because apt-cache translates its field labels and this parse matches the English "Candidate:"; a known package with no installable version reports "(none)" and must not count.
        if LC_ALL=C apt-cache policy "$_wk_pkg" 2>/dev/null \
            | grep -qE '^[[:space:]]*Candidate:[[:space:]]+[^([:space:]]'; then
            printf "%s\n" "$_wk_pkg"
            return 0
        fi
    done
    return 0
}

# Return 0 when this looks like a graphical session, i.e. someone who may want the desktop app rather than only the CLI and the web dashboard.
# Deliberately not keyed on stdin being a tty: `curl … | sh` is the primary install path and never has one.
likely_wants_desktop_app() {
    if [ -n "${DISPLAY:-}" ] || [ -n "${WAYLAND_DISPLAY:-}" ]; then
        return 0
    fi
    case "${XDG_SESSION_TYPE:-}" in
        x11|wayland) return 0 ;;
    esac
    return 1
}

# Debian-family hint about the desktop app's GTK / WebKit dependencies.
# Every WebKitGTK claim below is the result of a probe of this host, never a hardcoded assumption about what a distro ships.
print_debian_desktop_hint() {
    is_debian_family || return 0
    likely_wants_desktop_app || return 0

    echo ""
    if [ "${DISTRO:-}" = "deepin" ]; then
        # Which variant was actually installed is read off PLATFORM rather than assumed: resolve_platform_for_tag switches it to the gnu fallback whenever a release ships no musl package, and only NixOS suppresses that fallback.
        case "${PLATFORM:-}" in
            *-musl)
                echo "  deepin note: the CLI installed above is the static musl build, so the host's glibc version is irrelevant to it."
                ;;
            *)
                echo "  deepin note: this release shipped no static musl package, so the CLI installed above is the glibc build and does depend on the host's glibc."
                ;;
        esac
        echo "  The separate desktop app is the part that needs a GTK / WebKit stack from deepin's own repositories."
    else
        echo "  The separate desktop app additionally needs a GTK / WebKit stack from your distribution."
    fi

    # The desktop app is Tauri 2 and links the webkit2gtk-4.1 series: see the webkitgtk_4_1 input in flake.nix and libwebkit2gtk-4.1-dev on this project's own Debian-family CI runners.
    _dh_api=$(probe_webkit2gtk_pkgconfig)
    case "$_dh_api" in
        4.1)
            echo "  pkg-config reports webkit2gtk-4.1 here, which is the series the desktop app links."
            ;;
        4.0)
            echo "  pkg-config reports webkit2gtk-4.0 here but not webkit2gtk-4.1, and the desktop app links 4.1."
            echo "  Ask this machine what its release actually carries before installing the desktop app:"
            echo "    apt-cache search libwebkit2gtk"
            ;;
        *)
            echo "  pkg-config reports neither webkit2gtk-4.1 nor webkit2gtk-4.0 here; note the runtime library can be installed without the -dev package that ships the .pc file."
            _dh_pkg=$(webkit2gtk_apt_candidate)
            if [ -n "$_dh_pkg" ]; then
                echo "  Your repositories do offer an installable $_dh_pkg:"
                echo "    sudo apt-get install -y $_dh_pkg"
            else
                echo "  Which webkit2gtk series your repositories carry could not be determined here, so no package name is guessed."
                echo "  Ask this machine directly:"
                echo "    apt-cache search libwebkit2gtk"
                echo "    pkg-config --list-all | grep -i webkit"
            fi
            ;;
    esac
    echo "  For the full environment audit, including the rest of the desktop dependencies, run:"
    echo "    librefang doctor"
}

# --- Release resolution ---------------------------------------------------

# Newest-first list of release tags. Isolated so tests can mock `curl`.
fetch_release_tags() {
    curl -fsSL "https://api.github.com/repos/$REPO/releases?per_page=30" 2>/dev/null \
        | grep '"tag_name"' \
        | cut -d '"' -f 4
}

# Return 0 when the archive and its .sha256 exist for $1=tag $2=platform; the archive is probed with a 1-byte range request the CDN honors, confirming a present asset without fetching the whole file.
asset_available() {
    _aa_url="https://github.com/$REPO/releases/download/$1/librefang-$2.tar.gz"
    curl -fsSL -r 0-0 -o /dev/null "$_aa_url" 2>/dev/null \
        && curl -fsSL -o /dev/null "$_aa_url.sha256" 2>/dev/null
}

# Set PLATFORM to the first variant (primary, then fallback) that ships a package for $1=tag; returns 0 on success.
resolve_platform_for_tag() {
    for _pf in "${PLATFORM_PRIMARY:-$PLATFORM}" "$(effective_platform_fallback)"; do
        [ -n "$_pf" ] || continue
        if asset_available "$1" "$_pf"; then
            PLATFORM="$_pf"
            return 0
        fi
    done
    return 1
}

# Resolve VERSION+PLATFORM: LIBREFANG_VERSION is a hard pin; LIBREFANG_PREFERRED_VERSION is a soft hint with fallback.
resolve_installable_version() {
    if [ -n "${LIBREFANG_VERSION:-}" ]; then
        VERSION="$LIBREFANG_VERSION"
        echo "  Using specified version: $VERSION"
        return 0
    fi

    _preferred="${LIBREFANG_PREFERRED_VERSION:-}"
    if [ -n "$_preferred" ] && resolve_platform_for_tag "$_preferred"; then
        VERSION="$_preferred"
        return 0
    fi

    echo "  Fetching latest release..."
    _scanned=0
    for _tag in $(fetch_release_tags); do
        _scanned=$((_scanned + 1))
        [ "$_scanned" -le 10 ] || break
        if resolve_platform_for_tag "$_tag"; then
            VERSION="$_tag"
            if [ -n "$_preferred" ] && [ "$_tag" != "$_preferred" ]; then
                echo "  ${C_YELLOW}Release $_preferred has no $PLATFORM package yet; falling back to $_tag.${C_RESET}"
            elif [ "$_scanned" -gt 1 ]; then
                echo "  ${C_YELLOW}Newest release has no $PLATFORM package yet; using $_tag.${C_RESET}"
            fi
            return 0
        fi
    done
    return 1
}

# Atomically replace $2 with $1, rolling back to $2's backup if the new binary fails to run.
install_binary_with_rollback() {
    _src="$1"
    _dest="$2"
    _backup=""

    if [ -e "$_dest" ]; then
        _backup="$_dest.bak"
        rm -f "$_backup"
        if ! cp "$_dest" "$_backup"; then
            echo "  ${C_RED}Could not back up the existing binary at $_dest.${C_RESET}"
            return 1
        fi
    fi

    # cp to same filesystem first so the mv rename is atomic even when $_src is on a different mount.
    _staged="$_dest.new.$$"
    if ! cp "$_src" "$_staged"; then
        echo "  ${C_RED}Could not write the new binary into $(dirname "$_dest").${C_RESET}"
        rm -f "$_staged"
        if [ -n "$_backup" ]; then rm -f "$_backup"; fi
        return 1
    fi
    chmod +x "$_staged"

    if ! mv -f "$_staged" "$_dest"; then
        echo "  ${C_RED}Could not install the new binary to $_dest.${C_RESET}"
        rm -f "$_staged"
        if [ -n "$_backup" ]; then mv -f "$_backup" "$_dest"; fi
        return 1
    fi

    # Confirm the freshly installed binary actually runs; roll back if not.
    if ! "$_dest" --version >/dev/null 2>&1; then
        if [ -n "$_backup" ]; then
            mv -f "$_backup" "$_dest"
            echo "  ${C_RED}The new binary failed to run; rolled back to the previous version.${C_RESET}"
        else
            # Fresh install with nothing to roll back to: remove the broken
            # binary so a non-runnable librefang is not left on PATH.
            rm -f "$_dest"
            echo "  ${C_RED}The new binary failed to run.${C_RESET}"
        fi
        return 1
    fi

    if [ -n "$_backup" ]; then rm -f "$_backup"; fi
    return 0
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

# Return 0 when the current process still needs INSTALL_DIR added to PATH.
# Run in a subshell so the parser variables cannot leak into install().
session_needs_path_refresh() (
    _snpr_target=$1
    _snpr_remaining=${PATH:-}

    while :; do
        case "$_snpr_remaining" in
            *:*)
                _snpr_entry=${_snpr_remaining%%:*}
                _snpr_remaining=${_snpr_remaining#*:}
                _snpr_last=0
                ;;
            *)
                _snpr_entry=$_snpr_remaining
                _snpr_last=1
                ;;
        esac

        [ "$_snpr_entry" != "$_snpr_target" ] || return 1
        [ "$_snpr_last" -eq 0 ] || return 0
    done
)

# Prefer the configured login shell, falling back to the detected parent shell.
restart_shell_for() {
    if [ -n "${SHELL:-}" ]; then
        printf "%s\n" "$SHELL"
    else
        printf "%s\n" "${1:-}"
    fi
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
    detect_distro
    detect_platform

    echo ""
    echo "  ${C_BOLD}LibreFang Installer${C_RESET}"
    echo "  ==================="
    echo ""

    apply_distro_platform_policy

    if ! resolve_installable_version; then
        echo "  ${C_RED}No installable release with a $PLATFORM package was found.${C_RESET}"
        echo "  The latest release may still be building its assets, or none is"
        echo "  published for $REPO yet."
        if [ "$DISTRO" = "nixos" ]; then
            echo "  Only the static musl build is usable on NixOS, so the glibc build was not considered."
        fi
        print_source_install_hint
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
        if should_try_platform_fallback; then
            echo "  ${C_YELLOW}Static (musl) binary not available, trying glibc build...${C_RESET}"
            PLATFORM=$(effective_platform_fallback)
            URL="https://github.com/$REPO/releases/download/$VERSION/librefang-$PLATFORM.tar.gz"
            CHECKSUM_URL="$URL.sha256"
            if ! curl -fL $CURL_PROGRESS "$URL" -o "$ARCHIVE"; then
                echo "  ${C_RED}Download failed.${C_RESET}"
                echo "    URL: $URL"
                print_source_install_hint
                exit 1
            fi
        else
            echo "  ${C_RED}Download failed.${C_RESET}"
            echo "    URL: $URL"
            print_source_install_hint
            exit 1
        fi
    fi

    if ! curl -fsSL "$CHECKSUM_URL" -o "$CHECKSUM_FILE" 2>/dev/null; then
        echo "  ${C_RED}SHA256 checksum file not found on release.${C_RESET}"
        echo "    URL: $CHECKSUM_URL"
        echo "  Refusing to install an unverified binary."
        exit 1
    fi

    EXPECTED=$(cut -d ' ' -f 1 < "$CHECKSUM_FILE")
    if command_exists sha256sum; then
        ACTUAL=$(sha256sum "$ARCHIVE" | cut -d ' ' -f 1)
    elif command_exists shasum; then
        ACTUAL=$(shasum -a 256 "$ARCHIVE" | cut -d ' ' -f 1)
    else
        echo "  ${C_RED}No sha256sum or shasum found in PATH.${C_RESET}"
        echo "  Install GNU coreutils (or perl) and retry."
        exit 1
    fi

    if [ "$EXPECTED" != "$ACTUAL" ]; then
        echo "  ${C_RED}Checksum verification FAILED!${C_RESET}"
        echo "    Expected: $EXPECTED"
        echo "    Got:      $ACTUAL"
        exit 1
    fi
    echo "  ${C_GREEN}Checksum verified.${C_RESET}"

    # Extract to staging so the build is verified before touching the live install.
    STAGE="$TMPDIR/stage"
    mkdir -p "$STAGE"
    tar xzf "$ARCHIVE" -C "$STAGE"

    NEW_BIN="$STAGE/librefang"
    if [ ! -f "$NEW_BIN" ]; then
        echo "  ${C_RED}Archive did not contain the librefang binary.${C_RESET}"
        exit 1
    fi
    chmod +x "$NEW_BIN"

    # The Rust Telegram sidecar binary ships inside the same tarball since
    # the release pipeline bundles it. Older tarballs lack it, so install it
    # only when present and stay silent otherwise (backward compatible).
    NEW_SIDECAR="$STAGE/librefang-sidecar-telegram"
    if [ -f "$NEW_SIDECAR" ]; then
        chmod +x "$NEW_SIDECAR"
    fi

    # Ad-hoc codesign on macOS (prevents SIGKILL on Apple Silicon); sign staged binary before run or install.
    if [ "$OS" = "darwin" ]; then
        if command_exists xattr; then
            xattr -cr "$NEW_BIN" 2>/dev/null || true
            [ -f "$NEW_SIDECAR" ] && xattr -cr "$NEW_SIDECAR" 2>/dev/null || true
        fi
        if command_exists codesign; then
            if ! codesign --force --sign - "$NEW_BIN"; then
                echo ""
                echo "  ${C_YELLOW}Warning: ad-hoc code signing failed.${C_RESET}"
                echo "  On Apple Silicon, the binary may be killed (SIGKILL) by Gatekeeper."
                echo "  Try manually: xattr -cr $INSTALL_DIR/librefang && codesign --force --sign - $INSTALL_DIR/librefang"
                echo ""
            fi
            if [ -f "$NEW_SIDECAR" ]; then
                codesign --force --sign - "$NEW_SIDECAR" 2>/dev/null || true
            fi
        fi
    fi

    # Atomic replace with rollback: a failing new binary restores the backup rather than leaving nothing installed.
    if ! install_binary_with_rollback "$NEW_BIN" "$INSTALL_DIR/librefang"; then
        print_source_install_hint
        exit 1
    fi

    # Sidecar install is best-effort: a failure must not roll back the already-verified main binary.
    if [ -f "$NEW_SIDECAR" ]; then
        SIDECAR_DEST="$INSTALL_DIR/librefang-sidecar-telegram"
        SIDECAR_TMP="$SIDECAR_DEST.new.$$"
        if cp "$NEW_SIDECAR" "$SIDECAR_TMP" 2>/dev/null; then
            chmod +x "$SIDECAR_TMP" 2>/dev/null || true
            mv -f "$SIDECAR_TMP" "$SIDECAR_DEST" 2>/dev/null || rm -f "$SIDECAR_TMP"
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
    if session_needs_path_refresh "$INSTALL_DIR"; then
        SESSION_NEEDS_PATH_REFRESH=1
    fi

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
        if [ "$DISTRO" = "nixos" ]; then
            # The user-level unit works on NixOS — systemd honours ~/.config/systemd/user — but it is imperative state outside the system configuration.
            echo "  On NixOS this writes a user-level systemd unit, which works but lives outside your system configuration."
            echo "  The declarative route is preferred: import the flake's nixosModule and set services.librefang.enable = true."
        fi
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

    print_debian_desktop_hint

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
            RESTART_SHELL=$(restart_shell_for "$USER_SHELL")

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
