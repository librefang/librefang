#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
PKGBUILD="$ROOT/packaging/aur/librefang-desktop-bin/PKGBUILD"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

error() { printf '%s\n' "$*" >&2; }
install() { :; }
bsdtar() { printf '%s\n' "$*" >>"$TMP/bsdtar.log"; }

source "$PKGBUILD"
srcdir="$TMP/src"
pkgdir="$TMP/pkg"
mkdir -p "$srcdir" "$pkgdir"

if package 2>"$TMP/missing.err"; then
  echo "package accepted a missing data archive" >&2
  exit 1
fi
grep -q 'found 0' "$TMP/missing.err"

touch "$srcdir/data.tar.zst"
package
grep -q -- '--no-same-owner -xf' "$TMP/bsdtar.log"
grep -q 'data.tar.zst' "$TMP/bsdtar.log"

touch "$srcdir/data.tar.xz"
if package 2>"$TMP/multiple.err"; then
  echo "package accepted multiple data archives" >&2
  exit 1
fi
grep -q 'found 2' "$TMP/multiple.err"

grep -qx 'librefang-desktop=2026.6.24_beta.23' <(printf '%s\n' "${provides[@]}")
