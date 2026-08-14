#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
PUBLISHER="$ROOT/packaging/arch-repo/publish-arch-repo.sh"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/bin" "$TMP/source/packaging" "$TMP/home"
cp -R "$ROOT/packaging/aur" "$TMP/source/packaging/"

cat >"$TMP/bin/id" <<'MOCK'
#!/bin/sh
printf '1000\n'
MOCK
cat >"$TMP/bin/gpg" <<'MOCK'
#!/bin/sh
case "$*" in
  *--with-colons*) printf 'fpr:::::::::0123456789ABCDEF:\n' ;;
  *--armor*--export*) printf '%s\n' 'PUBLIC KEY' ;;
esac
MOCK
cat >"$TMP/bin/curl" <<'MOCK'
#!/bin/sh
printf '%s\n' '{"assets":[]}'
MOCK
cat >"$TMP/bin/jq" <<'MOCK'
#!/bin/sh
case "$*" in
  *_amd64.deb*) printf '%s\n' 'LibreFang_2026.7.31_amd64.deb' ;;
  *) printf '%s\n' 'librefang-x86_64-unknown-linux-gnu.tar.gz' ;;
esac
MOCK
cat >"$TMP/bin/updpkgsums" <<'MOCK'
#!/bin/sh
:
MOCK
cat >"$TMP/bin/makepkg" <<'MOCK'
#!/bin/sh
pkg=${PWD##*/}
touch "${pkg}-2026.7.31-1-x86_64.pkg.tar.zst"
touch "${pkg}-2026.7.31-1-x86_64.pkg.tar.zst.sig"
MOCK
cat >"$TMP/bin/repo-add" <<'MOCK'
#!/bin/sh
db=$4
dir=${db%/*}
name=${db##*/}; name=${name%.db.tar.gz}
touch "$dir/$name.db.tar.gz" "$dir/$name.db.tar.gz.sig"
touch "$dir/$name.files.tar.gz" "$dir/$name.files.tar.gz.sig"
ln -s "$name.db.tar.gz" "$dir/$name.db"
if [ "${MOCK_MISSING_DB_SIG:-}" != 1 ]; then
  ln -s "$name.db.tar.gz.sig" "$dir/$name.db.sig"
fi
ln -s "$name.files.tar.gz" "$dir/$name.files"
ln -s "$name.files.tar.gz.sig" "$dir/$name.files.sig"
MOCK
cat >"$TMP/bin/readlink" <<'MOCK'
#!/bin/sh
path=$2
target=$(/usr/bin/readlink "$path")
printf '%s/%s\n' "${path%/*}" "$target"
MOCK
cat >"$TMP/bin/cp" <<'MOCK'
#!/bin/sh
if [ "${1:-}" = --remove-destination ]; then
  dest=$3
  rm -f "$dest"
  /bin/cp "$2" "$dest"
else
  /bin/cp "$@"
fi
MOCK
cat >"$TMP/bin/rclone" <<'MOCK'
#!/bin/sh
case "$1" in
  lsf) exit 0 ;;
  copy|copyto) printf '%s\n' "$*" >>"$MOCK_RCLONE_LOG" ;;
esac
MOCK
cat >"$TMP/bin/sleep" <<'MOCK'
#!/bin/sh
:
MOCK
chmod +x "$TMP/bin/"*

export PATH="$TMP/bin:$PATH"
export HOME="$TMP/home"
export ARCH_REPO_SOURCE_ROOT="$TMP/source"
export RELEASE_TAG=v2026.7.31-rc.1
export ARCHES=x86_64
export RETAIN=2
export GPG_KEY_FILE="$TMP/signing.asc"
export GPG_KEY_ID=0123456789ABCDEF
export R2_ACCOUNT_ID=account
export R2_ACCESS_KEY_ID=access
export R2_SECRET_ACCESS_KEY=secret
export R2_BUCKET=bucket
export MOCK_RCLONE_LOG="$TMP/rclone.log"
touch "$GPG_KEY_FILE"

if ! "$PUBLISHER" >"$TMP/output" 2>&1; then
  cat "$TMP/output" >&2
  exit 1
fi
grep -q 'pkgver=2026.7.31_rc.1' "$TMP/output"
grep -q 'Uploaded x86_64 repo' "$TMP/output"
grep -q 'copyto .*librefang.gpg' "$MOCK_RCLONE_LOG"

if MOCK_MISSING_DB_SIG=1 "$PUBLISHER" >"$TMP/missing-link" 2>&1; then
  echo 'publisher accepted missing signed database metadata' >&2
  exit 1
fi
grep -q 'repo-add did not create librefang.db.sig' "$TMP/missing-link"

if ARCHES='x86_64;touch /tmp/nope' "$PUBLISHER" >"$TMP/bad-arch" 2>&1; then
  echo 'unsafe architecture was accepted' >&2
  exit 1
fi
grep -q 'unsupported architecture' "$TMP/bad-arch"

if RELEASE_TAG='v2026.7.31&bad' "$PUBLISHER" >"$TMP/bad-tag" 2>&1; then
  echo 'unsafe release tag was accepted' >&2
  exit 1
fi
grep -q 'invalid release tag' "$TMP/bad-tag"
