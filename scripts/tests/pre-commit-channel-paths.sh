#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
HOOK="$REPO_ROOT/scripts/hooks/pre-commit"
test -x "$HOOK" || { echo "FAIL: hook is not executable: $HOOK" >&2; exit 1; }

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/bin" "$WORK/crates/librefang-channels/src/commented" \
    "$WORK/crates/librefang-channels/src/new channel/deeper" \
    "$WORK/crates/librefang-channels/src/lookalike"

cat >"$WORK/bin/rustfmt" <<'EOF'
#!/bin/sh
exit 0
EOF
cat >"$WORK/bin/gitleaks" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod +x "$WORK/bin/rustfmt" "$WORK/bin/gitleaks"

cd "$WORK"
git init -q
git config user.email test@librefang.local
git config user.name test
git config commit.gpgsign false
printf '%s\n' 'edition = "2021"' >rustfmt.toml
: >crates/librefang-channels/src/channels-allowlist.txt
printf '%s\n' '// impl ChannelAdapter for MentionedOnly {}' \
    >crates/librefang-channels/src/commented/mod.rs
printf '%s\n' 'impl NotChannelAdapter for MentionedOnly {}' \
    >crates/librefang-channels/src/lookalike/mod.rs
git add rustfmt.toml crates/librefang-channels/src

PATH="$WORK/bin:$PATH" "$HOOK" >"$WORK/comment.log" 2>&1
git commit -q -m init

VIOLATION='crates/librefang-channels/src/new channel/deeper/adapter.rs'
printf '%s\n' 'impl ChannelAdapter for NewChannel {}' >"$VIOLATION"
git add -- "$VIOLATION"
if PATH="$WORK/bin:$PATH" "$HOOK" >"$WORK/violation.log" 2>&1; then
    echo "FAIL: deeply nested unallowlisted adapter was accepted" >&2
    exit 1
fi
if ! grep -Fq "  - $VIOLATION" "$WORK/violation.log"; then
    echo "FAIL: violation did not preserve the exact path" >&2
    cat "$WORK/violation.log" >&2
    exit 1
fi

printf '%s\n' 'new channel' \
    >crates/librefang-channels/src/channels-allowlist.txt
git add crates/librefang-channels/src/channels-allowlist.txt
: >crates/librefang-channels/src/channels-allowlist.txt
if ! PATH="$WORK/bin:$PATH" "$HOOK" >"$WORK/allowlisted.log" 2>&1; then
    echo "FAIL: staged allowlist entry was ignored in favor of the working tree" >&2
    cat "$WORK/allowlisted.log" >&2
    exit 1
fi

echo "PASS: pre-commit channel paths, impl matching, and staged allowlist"
