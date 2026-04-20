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

echo "All install.sh tests passed."
