#!/usr/bin/env bash
# Build and publish platform-specific wheels to PyPI.
# Called from CI with: VERSION=0.6.7 REPO=librefang/librefang TAG=v0.6.7-20260320
set -euo pipefail

: "${VERSION:?}"
: "${REPO:?}"
: "${TAG:?}"

WORK=$(mktemp -d)
DIST="$WORK/dist"
mkdir -p "$DIST"
trap 'rm -rf "$WORK"' EXIT

PKG_NAME="librefang"

# Rust target → (wheel platform tag, archive extension, binary name)
declare -A TARGETS=(
  ["x86_64-unknown-linux-gnu"]="manylinux_2_17_x86_64.manylinux2014_x86_64 tar.gz librefang"
  ["aarch64-unknown-linux-gnu"]="manylinux_2_17_aarch64.manylinux2014_aarch64 tar.gz librefang"
  ["x86_64-unknown-linux-musl"]="musllinux_1_2_x86_64 tar.gz librefang"
  ["aarch64-unknown-linux-musl"]="musllinux_1_2_aarch64 tar.gz librefang"
  ["x86_64-apple-darwin"]="macosx_10_12_x86_64 tar.gz librefang"
  ["aarch64-apple-darwin"]="macosx_11_0_arm64 tar.gz librefang"
  ["x86_64-pc-windows-msvc"]="win_amd64 zip librefang.exe"
  ["aarch64-pc-windows-msvc"]="win_arm64 zip librefang.exe"
)

build_wheel() {
  local target=$1 platform_tag=$2 ext=$3 exe=$4
  local wheel_dir="$WORK/wheel-$target"
  local data_dir="${PKG_NAME}-${VERSION}.data/scripts"
  local dist_info="${PKG_NAME}-${VERSION}.dist-info"

  rm -rf "$wheel_dir"
  mkdir -p "$wheel_dir/$data_dir" "$wheel_dir/$dist_info"

  # Download and extract binary
  local asset="librefang-${target}.${ext}"
  local url="https://github.com/${REPO}/releases/download/${TAG}/${asset}"
  echo "  Downloading $url"
  curl -fsSL -o "$wheel_dir/$asset" "$url"

  if [ "$ext" = "tar.gz" ]; then
    tar xzf "$wheel_dir/$asset" -C "$wheel_dir/$data_dir"
  else
    unzip -q -o "$wheel_dir/$asset" -d "$wheel_dir/$data_dir"
  fi
  chmod +x "$wheel_dir/$data_dir/$exe"
  rm -f "$wheel_dir/$asset"

  # METADATA
  cat > "$wheel_dir/$dist_info/METADATA" <<EOF
Metadata-Version: 2.1
Name: ${PKG_NAME}
Version: ${VERSION}
Summary: LibreFang Agent OS CLI
Home-page: https://librefang.ai
License: MIT
Project-URL: Repository, https://github.com/${REPO}
Requires-Python: >=3.8
Description-Content-Type: text/plain

LibreFang CLI binary distribution. See https://github.com/${REPO}
EOF

  # WHEEL
  cat > "$wheel_dir/$dist_info/WHEEL" <<EOF
Wheel-Version: 1.0
Generator: librefang-release
Root-Is-Purelib: false
Tag: py3-none-${platform_tag}
EOF

  # RECORD (hashes computed after all files are in place)
  cd "$wheel_dir"
  : > "$dist_info/RECORD"
  find . -type f ! -path "./$dist_info/RECORD" | sort | while read -r f; do
    f="${f#./}"
    local sha
    sha=$(python3 -c "
import hashlib, base64
h = hashlib.sha256(open('$f','rb').read()).digest()
print('sha256=' + base64.urlsafe_b64encode(h).decode().rstrip('='))
")
    local size
    size=$(wc -c < "$f" | tr -d ' ')
    echo "$f,$sha,$size" >> "$dist_info/RECORD"
  done
  echo "$dist_info/RECORD,," >> "$dist_info/RECORD"

  # Build wheel (just a zip with .whl extension)
  local wheel_name="${PKG_NAME}-${VERSION}-py3-none-${platform_tag}.whl"
  cd "$wheel_dir"
  zip -q -r "$DIST/$wheel_name" .
  echo "  Built $wheel_name"
  cd /
}

for target in "${!TARGETS[@]}"; do
  read -r platform_tag ext exe <<< "${TARGETS[$target]}"
  echo "=== Building wheel for $target ($platform_tag) ==="
  build_wheel "$target" "$platform_tag" "$ext" "$exe"
done

echo ""
echo "=== Uploading to PyPI ==="
pip install --quiet twine
twine upload --skip-existing "$DIST"/*.whl
echo "Done."
