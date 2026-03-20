#!/usr/bin/env bash
# Publish @librefang/cli platform packages + wrapper to npm.
# Called from CI with: VERSION=0.6.7 REPO=librefang/librefang TAG=v0.6.7-20260320
set -euo pipefail

: "${VERSION:?}"
: "${REPO:?}"
: "${TAG:?}"

# Retry download (brief retries for GitHub CDN propagation)
download_asset() {
  local url=$1 dest=$2
  for i in $(seq 1 5); do
    if curl -fsSL -o "$dest" "$url" 2>/dev/null; then
      return 0
    fi
    echo "  Retrying download in 10s... ($i/5)"
    sleep 10
  done
  echo "ERROR: Failed to download $url"
  return 1
}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# Rust target → npm package info
declare -A TARGETS=(
  ["x86_64-unknown-linux-gnu"]="linux x64 tar.gz librefang"
  ["aarch64-unknown-linux-gnu"]="linux arm64 tar.gz librefang"
  ["x86_64-unknown-linux-musl"]="linux-musl x64 tar.gz librefang"
  ["aarch64-unknown-linux-musl"]="linux-musl arm64 tar.gz librefang"
  ["x86_64-apple-darwin"]="darwin x64 tar.gz librefang"
  ["aarch64-apple-darwin"]="darwin arm64 tar.gz librefang"
  ["x86_64-pc-windows-msvc"]="win32 x64 zip librefang.exe"
  ["aarch64-pc-windows-msvc"]="win32 arm64 zip librefang.exe"
)

# npm os field needs "linux" not "linux-musl"
npm_os() {
  local p=$1
  case "$p" in
    linux-musl) echo "linux" ;;
    *) echo "$p" ;;
  esac
}

# npm package suffix
npm_suffix() {
  local p=$1 a=$2
  case "$p" in
    linux-musl) echo "linux-${a}-musl" ;;
    *) echo "${p}-${a}" ;;
  esac
}

for target in "${!TARGETS[@]}"; do
  read -r plat arch ext exe <<< "${TARGETS[$target]}"
  suffix=$(npm_suffix "$plat" "$arch")
  pkg_name="@librefang/cli-${suffix}"
  pkg_dir="$WORK/$suffix"

  echo "=== Publishing $pkg_name ==="

  # Check if already published
  if npm view "${pkg_name}@${VERSION}" version 2>/dev/null; then
    echo "  Already published, skipping"
    continue
  fi

  mkdir -p "$pkg_dir/bin"

  # Download binary from GitHub Release
  asset="librefang-${target}.${ext}"
  url="https://github.com/${REPO}/releases/download/${TAG}/${asset}"
  echo "  Downloading $url"
  download_asset "$url" "$pkg_dir/$asset"

  # Extract binary
  if [ "$ext" = "tar.gz" ]; then
    tar xzf "$pkg_dir/$asset" -C "$pkg_dir/bin"
  else
    unzip -q -o "$pkg_dir/$asset" -d "$pkg_dir/bin"
  fi
  chmod +x "$pkg_dir/bin/$exe"
  rm -f "$pkg_dir/$asset"

  # Generate package.json
  os_field=$(npm_os "$plat")
  cat > "$pkg_dir/package.json" <<EOF
{
  "name": "${pkg_name}",
  "version": "${VERSION}",
  "description": "LibreFang CLI binary for ${suffix}",
  "license": "MIT",
  "repository": {
    "type": "git",
    "url": "https://github.com/${REPO}"
  },
  "os": ["${os_field}"],
  "cpu": ["${arch}"],
  "bin": {
    "librefang": "./bin/${exe}"
  },
  "files": ["bin/${exe}"]
}
EOF

  npm publish "$pkg_dir" --access public
  echo "  Published ${pkg_name}@${VERSION}"
done

# Publish wrapper package
echo "=== Publishing @librefang/cli ==="
if npm view "@librefang/cli@${VERSION}" version 2>/dev/null; then
  echo "  Already published, skipping"
  exit 0
fi

WRAPPER_DIR="$WORK/cli-wrapper"
cp -r "$(dirname "$0")/../packages/cli-npm" "$WRAPPER_DIR"

# Update version in wrapper and all optionalDependencies
cd "$WRAPPER_DIR"
npm version "$VERSION" --no-git-tag-version --allow-same-version
node -e "
const pkg = require('./package.json');
for (const dep in pkg.optionalDependencies) {
  pkg.optionalDependencies[dep] = '${VERSION}';
}
require('fs').writeFileSync('package.json', JSON.stringify(pkg, null, 2) + '\n');
"

npm publish --access public
echo "  Published @librefang/cli@${VERSION}"
