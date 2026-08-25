#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
PUBLISHER="$ROOT/packaging/aur/publish-to-aur.sh"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/bin" "$TMP/source"
cp -R "$ROOT/packaging/aur/librefang-bin" "$TMP/source/"
cp -R "$ROOT/packaging/aur/librefang-desktop-bin" "$TMP/source/"
touch "$TMP/source/librefang-bin/.DS_Store"

cat >"$TMP/bin/id" <<'MOCK'
#!/bin/sh
printf '1000\n'
MOCK
cat >"$TMP/bin/curl" <<'MOCK'
#!/bin/sh
if [ "${MOCK_NO_ASSET:-}" = 1 ]; then
  printf '%s\n' '{"assets":[]}'
  exit 0
fi
printf '%s\n' '{"assets":[{"name":"librefang-x86_64-unknown-linux-gnu.tar.gz"}]}'
MOCK
cat >"$TMP/bin/updpkgsums" <<'MOCK'
#!/bin/sh
count=$(cat "$MOCK_UPDATE_COUNT" 2>/dev/null || printf 0)
count=$((count + 1))
printf '%s\n' "$count" >"$MOCK_UPDATE_COUNT"
[ "$count" -ge 2 ]
MOCK
cat >"$TMP/bin/makepkg" <<'MOCK'
#!/bin/sh
printf 'pkgbase = librefang-bin\n'
MOCK
cat >"$TMP/bin/sleep" <<'MOCK'
#!/bin/sh
:
MOCK
cat >"$TMP/bin/git" <<'MOCK'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >>"$MOCK_GIT_LOG"
case "${1:-}" in
  clone)
    mkdir -p "${@: -1}"
    ;;
  diff)
    exit 1
    ;;
  push)
    [[ ! -e .DS_Store ]] || { echo 'stray dotfile copied' >&2; exit 2; }
    count=$(cat "$MOCK_PUSH_COUNT" 2>/dev/null || printf 0)
    count=$((count + 1))
    printf '%s\n' "$count" >"$MOCK_PUSH_COUNT"
    [[ "$count" -ge 2 ]]
    ;;
esac
MOCK
chmod +x "$TMP/bin/"*

export PATH="$TMP/bin:$PATH"
export HOME="$TMP/home"
export AUR_SOURCE_ROOT="$TMP/source"
export RELEASE_TAG=v2026.7.31
export MOCK_UPDATE_COUNT="$TMP/update-count"
export MOCK_PUSH_COUNT="$TMP/push-count"
export MOCK_GIT_LOG="$TMP/git.log"
mkdir -p "$HOME/.ssh"

if ! "$PUBLISHER" librefang-bin >"$TMP/output" 2>&1; then
  cat "$TMP/output" >&2
  exit 1
fi
grep -q 'Waiting for release assets to become downloadable' "$TMP/output"
grep -q 'AUR push raced with another update' "$TMP/output"
grep -q '^fetch origin master$' "$MOCK_GIT_LOG"
grep -qx '2' "$MOCK_UPDATE_COUNT"
grep -qx '2' "$MOCK_PUSH_COUNT"

if RELEASE_TAG='v2026.7.31&bad' "$PUBLISHER" librefang-bin >"$TMP/invalid" 2>&1; then
  echo 'invalid release tag was accepted' >&2
  exit 1
fi
grep -q 'invalid release tag' "$TMP/invalid"

if MOCK_NO_ASSET=1 "$PUBLISHER" librefang-desktop-bin >"$TMP/missing" 2>&1; then
  echo 'publisher accepted a missing desktop asset' >&2
  exit 1
fi
grep -q 'asset \*_amd64.deb not found' "$TMP/missing"
! grep -q 'could not parse safe bundle version' "$TMP/missing"
