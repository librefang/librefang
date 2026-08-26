#!/usr/bin/env bash
set -euo pipefail

extract_between() {
  local file="$1" start="$2" key="$3"
  awk -v start="$start" -v key="$key" '
    $0 == start { active = 1; next }
    active && /^\[/ { exit }
    active && $0 ~ "^" key "[[:space:]]*=" {
      value = $0
      sub(/^[^=]*=[[:space:]]*"/, "", value)
      sub(/".*$/, "", value)
      print value
      exit
    }
  ' "$file"
}

cargo_msrv=$(extract_between Cargo.toml '[workspace.package]' rust-version)
mise_rust=$(extract_between mise.toml '[tools]' rust)
toolchain_rust=$(extract_between rust-toolchain.toml '[toolchain]' channel)
locked_rust=$(extract_between mise.lock '[[tools.rust]]' version)

[[ -n "$cargo_msrv" && -n "$mise_rust" && -n "$toolchain_rust" && -n "$locked_rust" ]] || {
  echo 'failed to read every Rust toolchain version' >&2
  exit 1
}

for entry in "mise.toml:$mise_rust" "rust-toolchain.toml:$toolchain_rust" "mise.lock:$locked_rust"; do
  file=${entry%%:*}
  version=${entry#*:}
  [[ "$version" == "$cargo_msrv" ]] || {
    echo "$file pins Rust $version, but Cargo.toml declares MSRV $cargo_msrv" >&2
    exit 1
  }
done

if [[ "${1:-}" == --print ]]; then
  printf '%s\n' "$cargo_msrv"
else
  echo "Rust toolchain contract: $cargo_msrv"
fi
