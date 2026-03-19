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

# choose_shell_rc fallback order: bashrc -> zshrc -> fish
mkdir -p "$TMP_HOME/.config/fish"
: > "$TMP_HOME/.config/fish/config.fish"
: > "$TMP_HOME/.zshrc"
: > "$TMP_HOME/.bashrc"
[ "$(choose_shell_rc "")" = "$TMP_HOME/.bashrc" ] || fail "fallback should prefer .bashrc"
rm -f "$TMP_HOME/.bashrc"
[ "$(choose_shell_rc "")" = "$TMP_HOME/.zshrc" ] || fail "fallback should pick .zshrc when .bashrc missing"
rm -f "$TMP_HOME/.zshrc"
[ "$(choose_shell_rc "")" = "$TMP_HOME/.config/fish/config.fish" ] || fail "fallback should pick fish config last"
pass "choose_shell_rc fallback order"

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

echo "All install.sh tests passed."
