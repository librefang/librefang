#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
INSTALLER_PATH="$ROOT_DIR/web/public/install.sh"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

pass() {
    echo "PASS: $*"
}

TMP_HOME=$(mktemp -d)
HOME="$TMP_HOME" LIBREFANG_INSTALLER_SOURCE_ONLY=1 . "$INSTALLER_PATH"

# shell_rc_from_shell mappings
[ "$(shell_rc_from_shell zsh)" = "$TMP_HOME/.zshrc" ] || fail "zsh rc mapping"
[ "$(shell_rc_from_shell /bin/bash)" = "$TMP_HOME/.bashrc" ] || fail "bash rc mapping"
[ "$(shell_rc_from_shell fish)" = "$TMP_HOME/.config/fish/config.fish" ] || fail "fish rc mapping"
pass "shell_rc_from_shell mappings"

# choose_shell_rc: $SHELL fallback when detect_user_shell came back empty.
# Real-world hit: curl|sh pipelines where `ps -p $PPID -o comm=` returns
# something unexpected and USER_SHELL ends up blank.
mkdir -p "$TMP_HOME/.config/fish"
: > "$TMP_HOME/.config/fish/config.fish"
: > "$TMP_HOME/.zshrc"
: > "$TMP_HOME/.bashrc"
[ "$(SHELL=/usr/bin/zsh choose_shell_rc "")" = "$TMP_HOME/.zshrc" ] \
    || fail "empty arg + SHELL=zsh should pick .zshrc"
[ "$(SHELL=/bin/bash choose_shell_rc "")" = "$TMP_HOME/.bashrc" ] \
    || fail "empty arg + SHELL=bash should pick .bashrc"
[ "$(SHELL=/usr/bin/fish choose_shell_rc "")" = "$TMP_HOME/.config/fish/config.fish" ] \
    || fail "empty arg + SHELL=fish should pick fish config"
pass "choose_shell_rc uses \$SHELL when detect returned empty"

# File-existence fallback: when both the arg and $SHELL are unusable, prefer
# .zshrc > .bashrc > fish. Old order (bashrc first) silently wrote PATH into
# .bashrc for zsh users whose shell detection had failed upstream — zsh then
# can't see librefang in new shells.
[ "$(SHELL= choose_shell_rc "")" = "$TMP_HOME/.zshrc" ] \
    || fail "file fallback should prefer .zshrc over .bashrc"
rm -f "$TMP_HOME/.zshrc"
[ "$(SHELL= choose_shell_rc "")" = "$TMP_HOME/.bashrc" ] \
    || fail "file fallback should pick .bashrc when .zshrc missing"
rm -f "$TMP_HOME/.bashrc"
[ "$(SHELL= choose_shell_rc "")" = "$TMP_HOME/.config/fish/config.fish" ] \
    || fail "file fallback should pick fish config last"
pass "choose_shell_rc file-existence fallback order"

# The "already installed" check must match the install path, not any line
# mentioning the word "librefang". Prior `grep -q "librefang"` was too loose:
# a user named `librefang` (HOME=/home/librefang) caused any .zshrc line
# containing that path fragment — oh-my-zsh cache vars, plugin paths, a
# comment — to silently suppress the PATH append, leaving the shell with no
# way to find the binary.
: > "$TMP_HOME/.zshrc"
: > "$TMP_HOME/.bashrc"
echo 'ZSH_CACHE_DIR="/home/librefang/.cache/oh-my-zsh"' >> "$TMP_HOME/.zshrc"
echo '# user note: librefang install coming soon' >> "$TMP_HOME/.zshrc"
grep -qE "\.librefang/bin" "$TMP_HOME/.zshrc" \
    && fail "rc with only librefang-in-path words should not match \.librefang/bin"

echo 'export PATH="/home/alice/.librefang/bin:$PATH"' >> "$TMP_HOME/.zshrc"
grep -qE "\.librefang/bin" "$TMP_HOME/.zshrc" \
    || fail "rc with real librefang/bin PATH export should match"
pass "already-installed check uses precise \.librefang/bin pattern"

# auto-start flag parser
for truthy in 1 true TRUE yes YES on ON; do
    is_enabled "$truthy" || fail "is_enabled should accept $truthy"
done
for falsy in 0 false FALSE no NO off OFF ""; do
    if is_enabled "$falsy"; then
        fail "is_enabled should reject $falsy"
    fi
done
pass "LIBREFANG_AUTO_START flag parser"

# parent-shell detection regression test with mocked ps:
# 1st comm query -> "sh", ppid query -> "222", 2nd comm query -> "zsh"
FAKE_BIN=$(mktemp -d)
FAKE_PS_STATE="$FAKE_BIN/ps-state"
cat > "$FAKE_BIN/ps" <<'PS_EOF'
#!/bin/sh
case "$*" in
  *" -o ppid="*) echo "222"; exit 0 ;;
esac

STATE_FILE="${FAKE_PS_STATE:?}"
COUNT=0
if [ -f "$STATE_FILE" ]; then
  COUNT=$(cat "$STATE_FILE" 2>/dev/null || echo 0)
fi
COUNT=$((COUNT + 1))
echo "$COUNT" > "$STATE_FILE"

if [ "$COUNT" -eq 1 ]; then
  echo "sh"
else
  echo "zsh"
fi
PS_EOF
chmod +x "$FAKE_BIN/ps"

rm -f "$FAKE_PS_STATE"
DETECTED=$(HOME="$TMP_HOME" PATH="$FAKE_BIN:$PATH" SHELL=/bin/bash FAKE_PS_STATE="$FAKE_PS_STATE" INSTALLER_PATH="$INSTALLER_PATH" LIBREFANG_INSTALLER_SOURCE_ONLY=1 sh -c '. "$INSTALLER_PATH"; detect_user_shell')
[ "$DETECTED" = "zsh" ] || fail "detect_user_shell expected zsh, got: $DETECTED"
pass "detect_user_shell handles curl|sh parent shell"

# SESSION_NEEDS_PATH_REFRESH: detects when install dir is not in PATH
SESSION_NEEDS_PATH_REFRESH=0
case ":$PATH:" in
    *":/nonexistent/test/.librefang/bin:"*) ;;
    *) SESSION_NEEDS_PATH_REFRESH=1 ;;
esac
[ "$SESSION_NEEDS_PATH_REFRESH" -eq 1 ] \
    || fail "SESSION_NEEDS_PATH_REFRESH should be 1 for missing dir"

# SESSION_NEEDS_PATH_REFRESH: 0 when dir already present
FIRST_PATH_ENTRY=$(printf "%s" "$PATH" | cut -d: -f1)
SESSION_NEEDS_PATH_REFRESH=0
case ":$PATH:" in
    *":$FIRST_PATH_ENTRY:"*) ;;
    *) SESSION_NEEDS_PATH_REFRESH=1 ;;
esac
[ "$SESSION_NEEDS_PATH_REFRESH" -eq 0 ] \
    || fail "SESSION_NEEDS_PATH_REFRESH should be 0 for existing dir"
pass "SESSION_NEEDS_PATH_REFRESH detection"

# RESTART_SHELL: prefers $SHELL over USER_SHELL
RESTART_SHELL="${SHELL:-}"
[ -n "$RESTART_SHELL" ] || fail "SHELL should be set in test env"
pass "RESTART_SHELL prefers \$SHELL"

# RESTART_SHELL: falls back to USER_SHELL when SHELL is empty
USER_SHELL="zsh"
RESTART_SHELL=""
[ -n "$RESTART_SHELL" ] || RESTART_SHELL="$USER_SHELL"
[ "$RESTART_SHELL" = "zsh" ] \
    || fail "RESTART_SHELL should fall back to USER_SHELL, got: $RESTART_SHELL"
pass "RESTART_SHELL falls back to USER_SHELL when SHELL is empty"

# --- resolve_installable_version: asset-aware fallback --------------------
FAKE_CURL_BIN=$(mktemp -d)
cat > "$FAKE_CURL_BIN/curl" <<'CURL_EOF'
#!/bin/sh
# Mock curl for resolution tests. Driven by env:
#   MOCK_TAGS         newline-separated tags, newest first (release list)
#   MOCK_GOOD_TAGS    space-separated tags that have downloadable assets
#   MOCK_BAD_PLATFORM platform substring whose asset always 404s (optional)
for arg in "$@"; do
    case "$arg" in
        *"/releases?per_page="*)
            printf '%s\n' "${MOCK_TAGS:-}" | while IFS= read -r t; do
                [ -n "$t" ] && printf '    "tag_name": "%s",\n' "$t"
            done
            exit 0
            ;;
        *"/releases/download/"*)
            _t="${arg#*/releases/download/}"
            _t="${_t%%/*}"
            # The tarball probe must use a 1-byte range request; fail loudly if a
            # regression drops it (which would start pulling full archives). The
            # checksum probe (.sha256) is exempt — it is fetched in full.
            case "$arg" in
                *.tar.gz)
                    case " $* " in
                        *" -r 0-0 "*) ;;
                        *) echo "mock curl: tarball probe missing -r 0-0" >&2; exit 99 ;;
                    esac
                    ;;
            esac
            case " ${MOCK_GOOD_TAGS:-} " in
                *" $_t "*) ;;
                *) exit 22 ;;
            esac
            if [ -n "${MOCK_BAD_PLATFORM:-}" ]; then
                case "$arg" in
                    *"$MOCK_BAD_PLATFORM"*) exit 22 ;;
                esac
            fi
            exit 0
            ;;
    esac
done
exit 0
CURL_EOF
chmod +x "$FAKE_CURL_BIN/curl"

OLD_PATH="$PATH"
PATH="$FAKE_CURL_BIN:$PATH"
PLATFORM_PRIMARY="x86_64-unknown-linux-musl"
PLATFORM_FALLBACK="x86_64-unknown-linux-gnu"
MOCK_TAGS=$(printf '%s\n' "v3-stuck" "v2-good" "v1-good")
export MOCK_TAGS MOCK_GOOD_TAGS MOCK_BAD_PLATFORM
unset LIBREFANG_VERSION LIBREFANG_PREFERRED_VERSION

# Newest (v3-stuck) ships no assets -> fall back to v2-good.
MOCK_GOOD_TAGS="v2-good v1-good"; MOCK_BAD_PLATFORM=""
PLATFORM="$PLATFORM_PRIMARY"; VERSION=""
resolve_installable_version >/dev/null 2>&1 || fail "resolve should succeed when an older release is installable"
[ "$VERSION" = "v2-good" ] || fail "resolve should skip stuck newest, got: $VERSION"
pass "resolve_installable_version skips a stuck newest release"

# Platform fallback within a release: primary (musl) missing, fallback (gnu) ok.
MOCK_GOOD_TAGS="v2-good v1-good"; MOCK_BAD_PLATFORM="$PLATFORM_PRIMARY"
PLATFORM="$PLATFORM_PRIMARY"; VERSION=""
resolve_installable_version >/dev/null 2>&1 || fail "resolve should fall back to the gnu platform"
[ "$VERSION" = "v2-good" ] || fail "resolve version with platform fallback, got: $VERSION"
[ "$PLATFORM" = "$PLATFORM_FALLBACK" ] || fail "resolve should select the gnu platform, got: $PLATFORM"
pass "resolve_installable_version falls back across platform variants"

# Explicit LIBREFANG_VERSION is a hard pin honored verbatim (no asset probe).
MOCK_GOOD_TAGS=""; MOCK_BAD_PLATFORM=""
export LIBREFANG_VERSION="v9-pinned"; VERSION=""; PLATFORM="$PLATFORM_PRIMARY"
resolve_installable_version >/dev/null 2>&1 || fail "hard pin should always resolve"
[ "$VERSION" = "v9-pinned" ] || fail "hard pin should set VERSION verbatim, got: $VERSION"
unset LIBREFANG_VERSION
pass "resolve_installable_version honors an explicit hard pin"

# LIBREFANG_PREFERRED_VERSION is a soft hint: used when its package exists, falls back when stuck.
MOCK_GOOD_TAGS="v2-good v1-good"; MOCK_BAD_PLATFORM=""
export LIBREFANG_PREFERRED_VERSION="v2-good"; VERSION=""; PLATFORM="$PLATFORM_PRIMARY"
resolve_installable_version >/dev/null 2>&1 || fail "preferred installable should resolve"
[ "$VERSION" = "v2-good" ] || fail "preferred installable should be used, got: $VERSION"
export LIBREFANG_PREFERRED_VERSION="v3-stuck"; VERSION=""; PLATFORM="$PLATFORM_PRIMARY"
resolve_installable_version >/dev/null 2>&1 || fail "stuck preferred should fall back"
[ "$VERSION" = "v2-good" ] || fail "stuck preferred should fall back to v2-good, got: $VERSION"
unset LIBREFANG_PREFERRED_VERSION
pass "resolve_installable_version treats preferred version as a soft hint"

# No installable release at all -> non-zero so install() can error out.
MOCK_GOOD_TAGS=""; MOCK_BAD_PLATFORM=""
PLATFORM="$PLATFORM_PRIMARY"; VERSION=""
if resolve_installable_version >/dev/null 2>&1; then
    fail "resolve should fail when no release ships a package"
fi
pass "resolve_installable_version fails when nothing is installable"

PATH="$OLD_PATH"

# --- install_binary_with_rollback: atomic replace + rollback -------------
RB_DIR=$(mktemp -d)
RB_DEST="$RB_DIR/librefang"
cat > "$RB_DEST" <<'OLD_EOF'
#!/bin/sh
[ "$1" = "--version" ] && echo "old 1.0"
OLD_EOF
chmod +x "$RB_DEST"

RB_GOOD="$RB_DIR/new-good"
cat > "$RB_GOOD" <<'GOOD_EOF'
#!/bin/sh
[ "$1" = "--version" ] && echo "new 2.0"
GOOD_EOF
chmod +x "$RB_GOOD"

install_binary_with_rollback "$RB_GOOD" "$RB_DEST" >/dev/null 2>&1 \
    || fail "install_binary_with_rollback should succeed for a working binary"
[ "$("$RB_DEST" --version)" = "new 2.0" ] || fail "working upgrade should install the new binary"
[ ! -e "$RB_DEST.bak" ] || fail "backup should be removed after a successful upgrade"
pass "install_binary_with_rollback installs a working new binary"

RB_BAD="$RB_DIR/new-bad"
cat > "$RB_BAD" <<'BAD_EOF'
#!/bin/sh
exit 1
BAD_EOF
chmod +x "$RB_BAD"

if install_binary_with_rollback "$RB_BAD" "$RB_DEST" >/dev/null 2>&1; then
    fail "install_binary_with_rollback should fail for a broken binary"
fi
[ "$("$RB_DEST" --version)" = "new 2.0" ] || fail "broken upgrade should roll back to the previous binary"
[ ! -e "$RB_DEST.bak" ] || fail "backup should be cleaned up after a rollback"
pass "install_binary_with_rollback rolls back a broken new binary"

# Fresh install (no existing binary) with a broken new binary must NOT leave a
# non-runnable binary on PATH — there is nothing to roll back to.
RB_FRESH="$RB_DIR/fresh/librefang"
mkdir -p "$RB_DIR/fresh"
if install_binary_with_rollback "$RB_BAD" "$RB_FRESH" >/dev/null 2>&1; then
    fail "install_binary_with_rollback should fail for a broken fresh install"
fi
[ ! -e "$RB_FRESH" ] || fail "broken fresh install should not leave a binary behind"
[ ! -e "$RB_FRESH.bak" ] || fail "broken fresh install should not leave a backup behind"
pass "install_binary_with_rollback removes a broken fresh install"

echo "All install.sh tests passed."
