#!/usr/bin/env bash
# Build, GPG-sign, and publish the official LibreFang pacman repository.
#
# This is the project-maintained Arch binary repository — distinct from the
# AUR packages under packaging/aur/. It exists because AUR account
# registration was closed with no reopening date (see #6334), so the AUR
# automation in #6341 cannot publish yet. This repository ships the same
# release-pinned binary packages directly, installable with `pacman -Syu`
# once a user adds the `[librefang]` section to /etc/pacman.conf.
#
# It reuses the committed PKGBUILDs under packaging/aur/<package>/ as the
# single source of truth for each package — only the per-release values
# (pkgver, sha256sums, the desktop bundle version, the pinned Docker image
# tag) are derived here, exactly like the AUR publisher. The difference is
# the tail: instead of pushing each PKGBUILD to its own AUR git repository,
# this builds the .pkg.tar.zst, GPG-signs it, folds it into a per-arch shared
# pacman database (repo-add), and syncs the result to Cloudflare R2.
#
# Multi-arch: publishes one repo per architecture under arch/<arch>/.
#   x86_64  : librefang-bin, librefang-desktop-bin, librefang-docker
#   aarch64 : librefang-bin, librefang-docker   (no ARM Linux desktop bundle
#             exists upstream, so librefang-desktop-bin is x86_64-only)
# The packages just repackage prebuilt release artifacts (no compilation), so
# the aarch64 packages are built on the x86_64 runner by repointing the source
# tarball and setting CARCH — makepkg's arch field is metadata only. The host
# strip cannot process foreign binaries, so aarch64 builds disable !strip.
#
# Designed to run inside `archlinux:base-devel`. It self-bootstraps: when
# invoked as root it installs the packaging + upload tooling, creates an
# unprivileged `builder` user (makepkg refuses to run as root), and re-execs
# itself as that user.
#
# Required environment:
#   RELEASE_TAG          e.g. v2026.6.26-beta.24
#   GPG_KEY_FILE         path to the ascii-armored signing key (passphrase-less)
#   GPG_KEY_ID           signing key / subkey id used by makepkg --sign + repo-add
#   R2_ACCOUNT_ID        Cloudflare account id (R2 S3 endpoint host)
#   R2_ACCESS_KEY_ID     R2 S3 access key id
#   R2_SECRET_ACCESS_KEY R2 S3 secret access key
#   R2_BUCKET            R2 bucket name (e.g. librefang-packages)
# Optional:
#   REPO_NAME            pacman db name (default "librefang")
#   ARCHES               space-separated arches to publish (default "x86_64 aarch64")
#   REPO_PREFIX_BASE     object prefix base inside the bucket (default "arch"; the
#                        per-arch repo lands at "$REPO_PREFIX_BASE/<arch>")
#   RETAIN               old package versions to keep per pkgname (default 5)
#   GITHUB_REPOSITORY    owner/repo for the release API (default librefang/librefang)
#   GH_API_TOKEN         raises the unauthenticated GitHub API rate limit
#
# Usage: publish-arch-repo.sh   (builds every package for every arch; no args)
set -euo pipefail

: "${RELEASE_TAG:?RELEASE_TAG is required}"
REPO="${GITHUB_REPOSITORY:-librefang/librefang}"
REPO_NAME="${REPO_NAME:-librefang}"
ARCHES="${ARCHES:-x86_64 aarch64}"
REPO_PREFIX_BASE="${REPO_PREFIX_BASE:-arch}"
RETAIN="${RETAIN:-5}"

# Which packages belong in a given arch's repo. librefang-docker is arch=any
# and lands in every arch path; librefang-desktop-bin is x86_64-only because
# upstream ships no ARM Linux desktop bundle.
packages_for_arch() {
  case "$1" in
    x86_64) echo "librefang-bin librefang-desktop-bin librefang-docker" ;;
    *)      echo "librefang-bin librefang-docker" ;;
  esac
}

# ── Root phase: install tools, drop privileges, re-exec as builder ─────────
if [[ "$(id -u)" -eq 0 ]]; then
  # Refresh archlinux-keyring in the same transaction so a slightly stale
  # base image can still verify freshly-signed packages.
  pacman -Syu --noconfirm --needed \
    archlinux-keyring base-devel pacman-contrib jq rclone >/dev/null

  useradd --create-home --shell /bin/bash builder 2>/dev/null || true

  # Hand the signing key to the builder with tight perms.
  install -d -o builder -g builder -m 700 /home/builder/keys
  install -o builder -g builder -m 600 \
    "${GPG_KEY_FILE:?GPG_KEY_FILE is required}" /home/builder/keys/signing.asc

  exec sudo -u builder \
    env HOME=/home/builder \
        RELEASE_TAG="$RELEASE_TAG" \
        GITHUB_REPOSITORY="$REPO" \
        GH_API_TOKEN="${GH_API_TOKEN:-}" \
        GPG_KEY_FILE=/home/builder/keys/signing.asc \
        GPG_KEY_ID="${GPG_KEY_ID:?GPG_KEY_ID is required}" \
        R2_ACCOUNT_ID="${R2_ACCOUNT_ID:?R2_ACCOUNT_ID is required}" \
        R2_ACCESS_KEY_ID="${R2_ACCESS_KEY_ID:?R2_ACCESS_KEY_ID is required}" \
        R2_SECRET_ACCESS_KEY="${R2_SECRET_ACCESS_KEY:?R2_SECRET_ACCESS_KEY is required}" \
        R2_BUCKET="${R2_BUCKET:?R2_BUCKET is required}" \
        REPO_NAME="$REPO_NAME" ARCHES="$ARCHES" \
        REPO_PREFIX_BASE="$REPO_PREFIX_BASE" RETAIN="$RETAIN" \
        bash "$0"
fi

# ── Builder phase ──────────────────────────────────────────────────────────
VER_RAW="${RELEASE_TAG#v}"   # 2026.6.26-beta.24  (matches the git tag minus the v)
VER_PKG="${VER_RAW/-/_}"     # 2026.6.26_beta.24  (Arch pkgver cannot contain '-')
echo "Publishing pacman repo '$REPO_NAME' for release $RELEASE_TAG (pkgver=$VER_PKG); arches: $ARCHES"

STAGING_BASE="$(mktemp -d)/repo"
BUILDROOT="$(mktemp -d)/build"
mkdir -p "$STAGING_BASE" "$BUILDROOT"

# ── GPG: import the signing key, mark it trusted for makepkg + repo-add ────
gpg --batch --quiet --import "$GPG_KEY_FILE"
# Ownertrust must address the primary key fingerprint, not the subkey id.
FPR="$(gpg --batch --with-colons --fingerprint "$GPG_KEY_ID" \
        | awk -F: '/^fpr:/ {print $10; exit}')"
[[ -n "$FPR" ]] || { echo "::error::could not resolve fingerprint for $GPG_KEY_ID"; exit 1; }
printf '%s:6:\n' "$FPR" | gpg --batch --import-ownertrust

# ── Cloudflare R2 over rclone's S3 backend (config via env, no config file) ─
export RCLONE_CONFIG_R2_TYPE=s3
export RCLONE_CONFIG_R2_PROVIDER=Cloudflare
export RCLONE_CONFIG_R2_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID"
export RCLONE_CONFIG_R2_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export RCLONE_CONFIG_R2_ENDPOINT="https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com"
export RCLONE_CONFIG_R2_ACL=private
# R2's S3 API rejects the trailing-checksum header rclone adds by default.
export RCLONE_S3_NO_CHECK_BUCKET=true

api_release_json() {
  local hdr=(-H "Accept: application/vnd.github+json")
  [[ -n "${GH_API_TOKEN:-}" ]] && hdr+=(-H "Authorization: Bearer $GH_API_TOKEN")
  curl -fsSL --retry 3 "${hdr[@]}" \
    "https://api.github.com/repos/$REPO/releases/tags/$RELEASE_TAG"
}

# Wait until a release asset whose name ends with $1 is visible; echo its name.
# `needs:` already orders us after the build jobs, but asset visibility can
# lag a job's completion by a few seconds.
wait_for_asset() {
  local suffix="$1" name
  for attempt in $(seq 1 18); do
    name="$(api_release_json | jq -r --arg s "$suffix" \
      '[.assets[].name | select(endswith($s))][0] // empty')"
    if [[ -n "$name" ]]; then echo "$name"; return 0; fi
    echo "Waiting for asset *$suffix on $RELEASE_TAG ($attempt/18)..." >&2
    sleep 10
  done
  echo "::error::asset *$suffix not found on $RELEASE_TAG after 180s" >&2
  return 1
}

# Build + sign one package from its committed PKGBUILD for arch $2. On success
# the signed .pkg.tar.zst (+ .sig) land in $STAGING (the per-arch dir, set by
# the caller). Returns non-zero on failure so the caller can skip just that
# package without sinking the rest. CARCH is set per-arch in ~/.makepkg.conf
# by the caller before this runs.
build_pkg() {
  local pkg="$1" arch="$2"
  local src="/repo/packaging/aur/$pkg"
  [[ -f "$src/PKGBUILD" ]] || { echo "::warning::no PKGBUILD at $src — skipping $pkg"; return 1; }

  local work="$BUILDROOT/$arch/$pkg"
  mkdir -p "$work"
  # Plain -R (not -a): the source tree is bind-mounted with a foreign owner,
  # and preserving ownership as the unprivileged builder fails under set -e.
  # Modes are irrelevant — the PKGBUILD installs each file with explicit -m.
  cp -R "$src"/. "$work"/
  cd "$work"

  sed -i "s/^pkgver=.*/pkgver=$VER_PKG/" PKGBUILD
  sed -i "s/^pkgrel=.*/pkgrel=1/" PKGBUILD

  case "$pkg" in
    librefang-bin)
      # Repoint the source tarball + arch field to the target arch. These are
      # prebuilt binaries, so the only per-arch change is which tarball to pull
      # and the metadata arch. The host strip can't process foreign binaries —
      # disable it (the release tarball is already stripped upstream anyway).
      if [[ "$arch" != x86_64 ]]; then
        sed -i "s/x86_64-unknown-linux-gnu/${arch}-unknown-linux-gnu/g" PKGBUILD
        sed -i "s/^arch=.*/arch=('$arch')/" PKGBUILD
        sed -i "s/^options=.*/options=('!debug' '!strip')/" PKGBUILD
      fi
      wait_for_asset "librefang-${arch}-unknown-linux-gnu.tar.gz" >/dev/null
      ;;
    librefang-desktop-bin)
      # The Tauri bundle version differs from the release tag; read it off the
      # actual .deb asset name (LibreFang_<bundle-ver>_amd64.deb).
      local deb dv
      deb="$(wait_for_asset "_amd64.deb")"
      dv="${deb#LibreFang_}"; dv="${dv%_amd64.deb}"
      [[ -n "$dv" ]] || { echo "::error::could not parse bundle version from '$deb'"; return 1; }
      sed -i "s/^_desktop_ver=.*/_desktop_ver=$dv/" PKGBUILD
      echo "Desktop bundle version: $dv"
      ;;
    librefang-docker)
      # No release asset to download — re-pin the embedded image tag in the
      # helper + env (their sha256sums then change and are regenerated below).
      # arch=any; the same package lands in every arch's repo.
      sed -i -E "s#(ghcr\.io/librefang/librefang:)[A-Za-z0-9._-]+#\1$VER_RAW#g" \
        librefang-docker librefang-docker.env
      ;;
    *)
      echo "::error::unknown package '$pkg'"; return 1 ;;
  esac

  updpkgsums
  grep -qx "pkgver=$VER_PKG" PKGBUILD || { echo "::error::pkgver patch did not stick for $pkg"; return 1; }

  # --nodeps: these repackage prebuilt release artifacts, so runtime depends
  # need not be installed in the build container. --sign with the imported key.
  makepkg --force --nodeps --nocheck --sign --key "$GPG_KEY_ID" --noconfirm

  cp ./*.pkg.tar.zst ./*.pkg.tar.zst.sig "$STAGING"/
}

# Keep the newest $RETAIN package files per pkgname in $R2_DEST; prune the rest
# (best-effort — a prune failure must not fail the release, the new packages
# are already published). repo-add already replaced each pkgname's db entry
# with this run's version, so older files are orphans the db never references;
# pruning is a pure R2 cleanup and must NOT call repo-remove (which addresses
# by pkgname and would drop the current entry). Files are kept (not deleted at
# once) only to allow manual `pacman -U <url>` downgrades.
prune_old() {
  local listing pkgname
  listing="$(rclone lsf "$R2_DEST" --files-only --include '*.pkg.tar.zst' \
    --format 'tp' --separator ';' 2>/dev/null | sort -r)" || return 0
  declare -A seen=()
  while IFS=';' read -r _mtime path; do
    [[ -n "$path" ]] || continue
    pkgname="$(printf '%s' "${path%.pkg.tar.zst}" | rev | cut -d- -f4- | rev)"
    seen["$pkgname"]=$(( ${seen["$pkgname"]:-0} + 1 ))
    if (( seen["$pkgname"] > RETAIN )); then
      echo "Pruning old package: $path (keeping newest $RETAIN of $pkgname)"
      rclone deletefile "$R2_DEST/$path" 2>/dev/null || true
      rclone deletefile "$R2_DEST/$path.sig" 2>/dev/null || true
    fi
  done <<< "$listing"
}

# ── Per-arch: build → fold into that arch's db → upload arch/<arch>/ ────────
published=0
for arch in $ARCHES; do
  echo "═══ arch: $arch ═══"
  # makepkg reads CARCH from makepkg.conf (not the environment); override it
  # per-arch in the builder's own config so package names + the arch check use
  # the target arch. Everything else is inherited from /etc/makepkg.conf.
  printf 'CARCH="%s"\n' "$arch" > "$HOME/.makepkg.conf"

  STAGING="$STAGING_BASE/$arch"
  mkdir -p "$STAGING"
  R2_DEST="r2:${R2_BUCKET}/${REPO_PREFIX_BASE}/${arch}"

  built=()
  for pkg in $(packages_for_arch "$arch"); do
    if build_pkg "$pkg" "$arch"; then
      built+=("$pkg")
    else
      echo "::warning::build failed for $pkg ($arch) — not published this run"
    fi
  done
  if [[ ${#built[@]} -eq 0 ]]; then
    echo "::warning::no packages built for $arch — skipping"
    continue
  fi
  echo "Built ($arch): ${built[*]}"

  cd "$STAGING"

  # Pull the existing db first so the update is incremental (cold start on the
  # first run simply finds none and creates a fresh one).
  rclone copy "$R2_DEST" "$STAGING" \
    --include "${REPO_NAME}.db*" --include "${REPO_NAME}.files*" 2>/dev/null || true

  repo-add --sign --key "$GPG_KEY_ID" "$STAGING/${REPO_NAME}.db.tar.gz" "$STAGING"/*.pkg.tar.zst

  # Object storage has no symlinks. repo-add --sign writes four — db, db.sig,
  # files, files.sig — each pointing at its .tar.gz / .tar.gz.sig target.
  # Materialise every one as a real object so pacman can fetch the db AND its
  # detached signature by plain name; rclone skips symlinks, and a missing
  # .db.sig breaks signed-db verification (SigLevel) on the client.
  for link in "${REPO_NAME}.db" "${REPO_NAME}.db.sig" "${REPO_NAME}.files" "${REPO_NAME}.files.sig"; do
    f="$STAGING/$link"
    [[ -L "$f" ]] && cp --remove-destination "$(readlink -f "$f")" "$f"
  done

  rclone copy "$STAGING" "$R2_DEST" \
    --include "*.pkg.tar.zst" --include "*.pkg.tar.zst.sig" \
    --include "${REPO_NAME}.db*" --include "${REPO_NAME}.files*"
  echo "Uploaded $arch repo for $VER_RAW to $R2_DEST"

  prune_old || echo "::warning::retention prune hit an error for $arch — packages published, cleanup skipped"
  published=$(( published + 1 ))
done

[[ $published -gt 0 ]] || { echo "::error::no arch published — nothing uploaded"; exit 1; }

# ── Publish the signing public key once (arch-independent, bucket root) so the
#    install docs can point users at a stable URL (idempotent). ──────────────
gpg --batch --armor --export "$GPG_KEY_ID" > "$STAGING_BASE/${REPO_NAME}.gpg"
rclone copyto "$STAGING_BASE/${REPO_NAME}.gpg" "r2:${R2_BUCKET}/${REPO_NAME}.gpg"
echo "Published signing public key to r2:${R2_BUCKET}/${REPO_NAME}.gpg"
