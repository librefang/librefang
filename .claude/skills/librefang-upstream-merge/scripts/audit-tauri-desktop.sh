#!/usr/bin/env bash
# audit-tauri-desktop.sh — Phase 3c of the librefang-upstream-merge skill.
#
# Verifies the four Tauri configs and the minisign pubkey survived the
# merge unchanged. See references/tauri-desktop-checklist.md for fixes
# when a check fails.
#
# Exit codes:
#   0 — all four checks pass
#   1 — one or more checks failed; specifics on stderr

set -euo pipefail

toplevel="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
desktop="$toplevel/crates/librefang-desktop"

fail_count=0

fail() { echo "[tauri-audit] FAIL: $*" >&2; fail_count=$(( fail_count + 1 )); }
ok()   { echo "[tauri-audit] ok:   $*"; }

# Sanity check: the desktop crate must exist.
if [ ! -d "$desktop" ]; then
  fail "$desktop missing — is this a librefang fork worktree?"
  exit 1
fi

# Helper: extract a JSON top-level key value (Python avoids a jq dep).
json_field() {
  local file="$1" key="$2"
  python3 -c "import json,sys; print(json.load(open('$file')).get('$key',''))" 2>/dev/null
}

# 1a. tauri.conf.json — productName
val="$(json_field "$desktop/tauri.conf.json" productName)"
if [ "$val" = "BossFang" ]; then
  ok "tauri.conf.json productName = BossFang"
else
  fail "tauri.conf.json productName = '$val' (expected 'BossFang')"
fi

# 1b. tauri.conf.json — identifier
val="$(json_field "$desktop/tauri.conf.json" identifier)"
if [ "$val" = "ai.bossfang.desktop" ]; then
  ok "tauri.conf.json identifier = ai.bossfang.desktop"
else
  fail "tauri.conf.json identifier = '$val' (expected 'ai.bossfang.desktop')"
fi

# 2a. tauri.desktop.conf.json — identifier
val="$(json_field "$desktop/tauri.desktop.conf.json" identifier)"
if [ "$val" = "ai.bossfang.desktop" ]; then
  ok "tauri.desktop.conf.json identifier = ai.bossfang.desktop"
else
  fail "tauri.desktop.conf.json identifier = '$val' (expected 'ai.bossfang.desktop')"
fi

# 2b. tauri.desktop.conf.json — updater endpoint host
endpoint="$(python3 -c "
import json
d=json.load(open('$desktop/tauri.desktop.conf.json'))
print(d.get('plugins',{}).get('updater',{}).get('endpoints',[''])[0])
" 2>/dev/null)"
case "$endpoint" in
  https://github.com/GQAdonis/librefang/*) ok "updater endpoint → $endpoint" ;;
  '') fail "updater endpoint missing or unreadable" ;;
  *) fail "updater endpoint → $endpoint (expected github.com/GQAdonis/librefang/...)" ;;
esac

# 2c. tauri.desktop.conf.json — pubkey key ID
pubkey_b64="$(python3 -c "
import json
d=json.load(open('$desktop/tauri.desktop.conf.json'))
print(d.get('plugins',{}).get('updater',{}).get('pubkey',''))
" 2>/dev/null)"
if [ -n "$pubkey_b64" ]; then
  decoded="$(echo "$pubkey_b64" | base64 -d 2>/dev/null | head -1 || true)"
  case "$decoded" in
    *E329A6B2863F1707*) ok "Tauri minisign key ID = E329A6B2863F1707 (BossFang)" ;;
    *BC91908BD3F1520D*) fail "Tauri minisign key ID is BC91908BD3F1520D (UPSTREAM key — must rotate to BossFang E329A6B2863F1707)" ;;
    *) fail "Tauri minisign key ID unrecognised: '$decoded'" ;;
  esac
else
  fail "Tauri pubkey field missing or empty"
fi

# 3. tauri.ios.conf.json — identifier
if [ -f "$desktop/tauri.ios.conf.json" ]; then
  val="$(json_field "$desktop/tauri.ios.conf.json" identifier)"
  if [ "$val" = "ai.bossfang.app" ]; then
    ok "tauri.ios.conf.json identifier = ai.bossfang.app"
  else
    fail "tauri.ios.conf.json identifier = '$val' (expected 'ai.bossfang.app')"
  fi
fi

# 4. tauri.android.conf.json — identifier
if [ -f "$desktop/tauri.android.conf.json" ]; then
  val="$(json_field "$desktop/tauri.android.conf.json" identifier)"
  if [ "$val" = "ai.bossfang.app" ]; then
    ok "tauri.android.conf.json identifier = ai.bossfang.app"
  else
    fail "tauri.android.conf.json identifier = '$val' (expected 'ai.bossfang.app')"
  fi
fi

# 5. Icons — confirm files are present (size > 0). We can't easily verify
# they're the BossFang artwork without a hash baseline, but we can spot
# upstream overwriting them with a 0-byte / placeholder.
for icon in icon.png icon.ico 32x32.png 128x128.png "128x128@2x.png"; do
  path="$desktop/icons/$icon"
  if [ ! -f "$path" ]; then
    fail "icons/$icon missing"
    continue
  fi
  size="$(stat -f%z "$path" 2>/dev/null || stat -c%s "$path" 2>/dev/null || echo 0)"
  if [ "$size" -lt 100 ]; then
    fail "icons/$icon suspiciously small ($size bytes) — verify it's the BossFang artwork"
  else
    ok "icons/$icon present (${size} bytes)"
  fi
done

echo
if [ "$fail_count" -eq 0 ]; then
  echo "[tauri-audit] all checks passed"
  exit 0
else
  echo "[tauri-audit] $fail_count check(s) failed — see references/tauri-desktop-checklist.md for fixes"
  exit 1
fi
