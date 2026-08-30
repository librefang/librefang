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

# flake.nix is the fourth consumer of this pin and the one that used to escape it: it built with `rust-bin.stable.latest`, resolved out of the pinned rust-overlay input, so a lock older than the MSRV handed every `nix build` a toolchain Cargo then rejected — main was red for days with "rustc 1.94.0 is not supported ... requires rustc 1.94.1" and nothing here noticed.
# It now reads rust-toolchain.toml, and this keeps it that way.
if ! grep -q 'builtins.fromTOML (builtins.readFile ./rust-toolchain.toml)' flake.nix; then
  echo 'flake.nix does not derive its Rust channel from rust-toolchain.toml' >&2
  exit 1
fi
if grep -q 'rust-bin.stable.latest' flake.nix; then
  echo 'flake.nix builds with rust-bin.stable.latest, which can drift below the MSRV' >&2
  exit 1
fi

if [[ "${1:-}" == --print ]]; then
  printf '%s\n' "$cargo_msrv"
else
  echo "Rust toolchain contract: $cargo_msrv"
fi
