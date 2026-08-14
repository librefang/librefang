#!/usr/bin/env bash
# Publish one release-pinned AUR package from packaging/aur/<package>/.
#
# Designed to run inside `archlinux:base-devel`. It self-bootstraps: when
# invoked as root it installs the Arch packaging tools, creates an
# unprivileged `builder` user (makepkg refuses to run as root), and
# re-execs itself as that user. The builder phase patches the committed
# PKGBUILD to the current release, regenerates the checksums and
# `.SRCINFO`, then pushes the result to the matching AUR git repository.
#
# The committed files under packaging/aur/<package>/ are the source of
# truth for everything except the per-release values (pkgver, sha256sums,
# the desktop bundle version, the pinned Docker image tag) which are
# derived here so a release never has to hand-edit the package.
#
# Required environment:
#   RELEASE_TAG           e.g. v2026.6.26-beta.24
#   AUR_KEY_FILE          path to the AUR SSH private key (root phase reads it)
#   AUR_KNOWN_HOSTS_FILE  path to a known_hosts file pinning aur.archlinux.org
# Optional:
#   GITHUB_REPOSITORY     owner/repo for the release API (default librefang/librefang)
#   GH_API_TOKEN          raises the unauthenticated GitHub API rate limit
#   AUR_GIT_NAME          commit author name  (default "LibreFang Release Bot")
#   AUR_GIT_EMAIL         commit author email (default "release-bot@librefang.ai")
#
# Usage: publish-to-aur.sh <librefang-bin|librefang-desktop-bin|librefang-docker>
set -euo pipefail

PKG="${1:?usage: publish-to-aur.sh <package>}"
: "${RELEASE_TAG:?RELEASE_TAG is required}"
REPO="${GITHUB_REPOSITORY:-librefang/librefang}"

# ── Root phase: install tools, drop privileges, re-exec as builder ─────────
if [[ "$(id -u)" -eq 0 ]]; then
  # Refresh archlinux-keyring in the same transaction so a slightly stale
  # base image can still verify freshly-signed packages.
  pacman -Syu --noconfirm --needed archlinux-keyring git openssh jq pacman-contrib >/dev/null

  useradd --create-home --shell /bin/bash builder 2>/dev/null || true

  install -d -o builder -g builder -m 700 /home/builder/.ssh
  install -o builder -g builder -m 600 "${AUR_KEY_FILE:?AUR_KEY_FILE is required}" /home/builder/.ssh/aur
  install -o builder -g builder -m 644 "${AUR_KNOWN_HOSTS_FILE:?AUR_KNOWN_HOSTS_FILE is required}" /home/builder/.ssh/known_hosts

  exec sudo -u builder \
    env HOME=/home/builder \
        RELEASE_TAG="$RELEASE_TAG" \
        GITHUB_REPOSITORY="$REPO" \
        GH_API_TOKEN="${GH_API_TOKEN:-}" \
        AUR_GIT_NAME="${AUR_GIT_NAME:-}" \
        AUR_GIT_EMAIL="${AUR_GIT_EMAIL:-}" \
        bash "$0" "$PKG"
fi

# ── Builder phase ──────────────────────────────────────────────────────────
VER_RAW="${RELEASE_TAG#v}"   # 2026.6.26-beta.24  (matches the git tag minus the v)
VER_PKG="${VER_RAW/-/_}"     # 2026.6.26_beta.24  (Arch pkgver cannot contain '-')
[[ "$VER_RAW" =~ ^[0-9]{4}\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || {
  echo "::error::invalid release tag '$RELEASE_TAG'" >&2
  exit 1
}
echo "Publishing AUR package '$PKG' for release $RELEASE_TAG (pkgver=$VER_PKG)"

SRC="${AUR_SOURCE_ROOT:-/repo/packaging/aur}/$PKG"
[[ -f "$SRC/PKGBUILD" ]] || { echo "::error::no PKGBUILD at $SRC"; exit 1; }

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT
WORK="$TMP_ROOT/work"
mkdir -p "$WORK"
# Plain -R (not -a): the source tree is bind-mounted with a foreign owner, and
# preserving ownership as the unprivileged builder would fail under `set -e`.
# File modes are irrelevant — the PKGBUILD installs each file with an explicit
# `install -m`, so the source-side bits never reach the package.
cp -R "$SRC"/. "$WORK"/
cd "$WORK"

# Capture the committed file set BEFORE updpkgsums downloads sources, so the
# generated LICENSE / tarball / .deb never get pushed to AUR.
case "$PKG" in
  librefang-bin)
    SRC_FILES=(PKGBUILD .SRCINFO librefang-bin.install librefang.env librefang.service librefang.sysusers librefang.tmpfiles)
    ;;
  librefang-desktop-bin)
    SRC_FILES=(PKGBUILD .SRCINFO)
    ;;
  librefang-docker)
    SRC_FILES=(PKGBUILD .SRCINFO librefang-docker librefang-docker.env librefang-docker.install librefang-docker.service)
    ;;
  *)
    echo "::error::unknown package '$PKG'" >&2
    exit 1
    ;;
esac
for source_file in "${SRC_FILES[@]}"; do
  [[ -f "$source_file" ]] || { echo "::error::missing allowlisted source '$source_file'" >&2; exit 1; }
done

api_release_json() {
  local hdr=(-H "Accept: application/vnd.github+json")
  [[ -n "${GH_API_TOKEN:-}" ]] && hdr+=(-H "Authorization: Bearer $GH_API_TOKEN")
  curl -fsSL --retry 3 "${hdr[@]}" \
    "https://api.github.com/repos/$REPO/releases/tags/$RELEASE_TAG"
}

# Wait until a release asset whose name ends with $1 is visible; echo its name.
# `needs:` in the workflow already orders us after the build job, but asset
# visibility can lag the job's completion by a few seconds.
wait_for_asset() {
  local suffix="$1" name
  for attempt in $(seq 1 18); do
    name="$(api_release_json | jq -r --arg s "$suffix" \
      '[.assets[].name | select(endswith($s))][0] // empty')"
    if [[ -n "$name" ]]; then
      echo "$name"
      return 0
    fi
    echo "Waiting for asset *$suffix on $RELEASE_TAG ($attempt/18)..." >&2
    sleep 10
  done
  echo "::error::asset *$suffix not found on $RELEASE_TAG after 180s" >&2
  return 1
}

rewrite_file() {
  local file="$1" tmp
  shift
  tmp="$(mktemp "$TMP_ROOT/rewrite.XXXXXX")"
  sed "$@" "$file" > "$tmp"
  cp "$tmp" "$file"
  rm -f "$tmp"
}

rewrite_file PKGBUILD "s/^pkgver=.*/pkgver=$VER_PKG/"
rewrite_file PKGBUILD "s/^pkgrel=.*/pkgrel=1/"

case "$PKG" in
  librefang-bin)
    wait_for_asset "librefang-x86_64-unknown-linux-gnu.tar.gz" >/dev/null
    ;;
  librefang-desktop-bin)
    # The Tauri bundle version differs from the release tag; read it off the
    # actual .deb asset name (LibreFang_<bundle-ver>_amd64.deb).
    DEB="$(wait_for_asset "_amd64.deb")" || exit 1
    DV="${DEB#LibreFang_}"; DV="${DV%_amd64.deb}"
    [[ "$DEB" == LibreFang_*_amd64.deb && "$DV" =~ ^[0-9][0-9A-Za-z._-]*$ ]] || {
      echo "::error::could not parse safe bundle version from '$DEB'"
      exit 1
    }
    rewrite_file PKGBUILD "s/^_desktop_ver=.*/_desktop_ver=$DV/"
    echo "Desktop bundle version: $DV"
    ;;
  librefang-docker)
    # No release asset to download — re-pin the embedded image tag in the
    # helper + env (their sha256sums then change and are regenerated below).
    rewrite_file librefang-docker -E \
      "s#(ghcr\.io/librefang/librefang:)[A-Za-z0-9._-]+#\1$VER_RAW#g"
    rewrite_file librefang-docker.env -E \
      "s#(ghcr\.io/librefang/librefang:)[A-Za-z0-9._-]+#\1$VER_RAW#g"
    ;;
  *)
    echo "::error::unknown package '$PKG'"; exit 1 ;;
esac

# Regenerate checksums + .SRCINFO from the patched PKGBUILD. AUR rejects any
# push whose .SRCINFO does not match `makepkg --printsrcinfo`, so always
# produce it the same way.
checksums_updated=false
for attempt in 1 2 3; do
  if updpkgsums; then
    checksums_updated=true
    break
  fi
  echo "Waiting for release assets to become downloadable ($attempt/3)..." >&2
  sleep 10
done
if [[ "$checksums_updated" != true ]]; then
  echo "::error::updpkgsums could not download and checksum release assets after 3 attempts" >&2
  exit 1
fi
makepkg --printsrcinfo > .SRCINFO

grep -qx "pkgver=$VER_PKG" PKGBUILD || { echo "::error::pkgver patch did not stick"; exit 1; }

# ── Push to AUR ─────────────────────────────────────────────────────────────
export GIT_SSH_COMMAND="ssh -i $HOME/.ssh/aur -o IdentitiesOnly=yes -o UserKnownHostsFile=$HOME/.ssh/known_hosts -o StrictHostKeyChecking=yes"
git config --global user.name "${AUR_GIT_NAME:-LibreFang Release Bot}"
git config --global user.email "${AUR_GIT_EMAIL:-release-bot@librefang.ai}"
git config --global init.defaultBranch master

CLONE="$TMP_ROOT/aur"
git clone --quiet "ssh://aur@aur.archlinux.org/$PKG.git" "$CLONE"

# Copy only the committed source files (never downloaded artifacts).
for f in "${SRC_FILES[@]}"; do
  cp "$WORK/$f" "$CLONE/$f"
done

cd "$CLONE"
git add -A
if git diff --cached --quiet; then
  echo "AUR/$PKG already up to date at $VER_RAW — nothing to push."
  exit 0
fi
git commit --quiet -m "Update to $VER_RAW"
push_succeeded=false
for attempt in 1 2 3; do
  if git push origin HEAD:master; then
    push_succeeded=true
    break
  fi
  echo "AUR push raced with another update; fetching and rebasing ($attempt/3)..." >&2
  if ! git fetch origin master; then
    echo "AUR fetch failed; retrying push without changing the local commit..." >&2
    continue
  fi
  if ! git rebase origin/master; then
    git rebase --abort || true
    echo "::error::AUR/$PKG changed concurrently and could not be rebased safely" >&2
    exit 1
  fi
done
if [[ "$push_succeeded" != true ]]; then
  echo "::error::failed to push AUR/$PKG after 3 attempts" >&2
  exit 1
fi
echo "Pushed AUR/$PKG $VER_RAW."
