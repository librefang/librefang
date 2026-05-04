#!/usr/bin/env bash
# Fail if the registry pubkey is not byte-identical across the four
# locations that have to stay in lockstep:
#
#   1. crates/librefang-runtime/src/plugin_manager.rs  EMBEDDED_REGISTRY_PUBKEY
#   2. web/workers/registry-worker/wrangler.toml       REGISTRY_PUBLIC_KEY
#   3. web/workers/marketplace-worker/wrangler.toml    REGISTRY_PUBLIC_KEY
#   4. web/public/_worker.js                           REGISTRY_PUBLIC_KEY
#
# A drift here means the daemon trusts one key, the workers sign with /
# serve a different one, and verification fails for whichever side lags.
# Catches the silent footgun called out by the PR review (MEDIUM #15).
#
# Run via: scripts/check-pubkey-lockstep.sh
# Wire into CI as a fast-fail step before any cargo / wrangler build.

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"

extract() {
  local path="$1" name="$2"
  if [ ! -f "$repo_root/$path" ]; then
    echo "::error file=$path::missing — pubkey lockstep check cannot run" >&2
    exit 1
  fi
  # Match the name then either '...' or "...", capture body.
  local body
  # Anchor at start-of-line + word-boundary on $name so a future
  # `LEGACY_REGISTRY_PUBLIC_KEY` doesn't false-positive (PR re-review
  # HIGH-NEW-E). Accept any of:
  #   NAME = "..."          (TOML / JS)
  #   const NAME: T = "..." (Rust top-level const)
  #   NAME: "..."           (Rust struct field — used by EmbeddedPubkey)
  # but no other quoted strings between $name and the value, so base64-
  # shaped fragments inside comments don't get picked up. Length-check
  # the captured value to exactly 44 chars (Ed25519 raw 32 bytes -> b64
  # with padding) — wrong length is wrong key.
  body="$(perl -ne '
    if (/^[ \t]*(?:const[ \t]+)?'"$name"'\b[^"\x27\n=:]*[=:][^"\x27\n]*['\''"]([A-Za-z0-9+\/=]{44})['\''"]/) {
      print $1; exit
    }
  ' "$repo_root/$path" 2>/dev/null || true)"
  if [ -z "$body" ]; then
    echo "::error file=$path::$name constant not found" >&2
    exit 1
  fi
  echo "$body"
}

# The daemon now embeds a SLICE of pubkeys (each `EmbeddedPubkey { b64,
# expires_at }`). Slot 0 is the active key. We pull the FIRST `b64: "..."`
# field — the active slot — and compare against the worker side.
daemon=$(extract \
  "crates/librefang-runtime/src/plugin_manager.rs" \
  'b64')
registry_worker=$(extract \
  "web/workers/registry-worker/wrangler.toml" \
  'REGISTRY_PUBLIC_KEY')
marketplace_worker=$(extract \
  "web/workers/marketplace-worker/wrangler.toml" \
  'REGISTRY_PUBLIC_KEY')
pages_worker=$(extract \
  "web/public/_worker.js" \
  'REGISTRY_PUBLIC_KEY')

drifted=0
expect_eq() {
  local name="$1" actual="$2" expected="$3"
  if [ "$actual" != "$expected" ]; then
    echo "::error::$name pubkey ($actual) drifts from daemon ($expected)" >&2
    drifted=1
  fi
}

expect_eq "registry-worker"   "$registry_worker"    "$daemon"
expect_eq "marketplace-worker" "$marketplace_worker" "$daemon"
expect_eq "pages-worker"       "$pages_worker"       "$daemon"

if [ "$drifted" -ne 0 ]; then
  exit 1
fi
echo "pubkey lockstep OK ($daemon)"
