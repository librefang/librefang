#!/usr/bin/env python3
"""
enforce-branding.py — Restore BossFang ember palette after upstream merges.

Usage:
    python3 scripts/enforce-branding.py            # apply replacements
    python3 scripts/enforce-branding.py --check    # read-only audit, exit 1 if any upstream token remains

What it does:
  Scans every *.ts, *.tsx, *.css, *.html, *.json, and *.rs file under
  crates/librefang-api/dashboard/src/, crates/librefang-api/static/,
  crates/librefang-desktop/frontend/, and crates/librefang-desktop/src/ for
  upstream LibreFang sky-blue brand tokens and replaces them with BossFang
  ember equivalents. Safe to run multiple times (idempotent).

  --check mode performs the same scan without writing — exits 0 if everything
  is clean, exits 1 if any upstream token survives. Designed for the pre-push
  hook so a forgotten merge can't ship.

After running, review the diff with:
    git diff
Then commit any changes with:
    git add -p && git commit -m "chore(dashboard): enforce BossFang brand tokens"

Upstream tokens replaced → BossFang tokens:
  Sky-blue primary:
    #0284c7  →  #E04E28  (Muted Ember, light mode)
    #38bdf8  →  #FF6A3D  (Bright Ember, dark mode)
    #0ea5e9  →  #FF6A3D  (Sky 500, also replaced with Bright Ember)
  Sky-blue rgba variants:
    rgba(14, 165, 233, …)  →  rgba(255, 106, 61, …)
    rgba(2, 132, 199, …)   →  rgba(224, 78, 40, …)
    rgba(56, 189, 248, …)  →  rgba(255, 106, 61, …)
  Upstream dark background:
    #020617  →  #0B0F14  (Deep Charcoal)
    rgba(2, 6, 23, …)  →  rgba(11, 15, 20, …)

NOT handled automatically (fix manually):
  - New components that render an SVG fang glyph inside a gradient box as the
    logo — replace with <img src="/boss-libre.png" alt="BossFang" ...>.
  - Product name "librefang" / "LibreFang" in user-visible JSX strings — the
    script does not touch string literals to avoid breaking internal identifiers
    (function names, CSS class names) that are legitimately named "librefang".
"""

from __future__ import annotations

import sys
from pathlib import Path

# ── Repository layout ────────────────────────────────────────────────────────

REPO_ROOT = Path(__file__).resolve().parent.parent
SCAN_DIRS = [
    REPO_ROOT / "crates/librefang-api/dashboard/src",
    REPO_ROOT / "crates/librefang-api/static",
    REPO_ROOT / "crates/librefang-desktop/frontend",
    REPO_ROOT / "crates/librefang-desktop/src",
]
EXTENSIONS = {".ts", ".tsx", ".css", ".html", ".json", ".rs"}

# ── Replacement table ─────────────────────────────────────────────────────────
#
# Order matters: longer / more-specific patterns must come first so that a
# shorter pattern cannot match inside an already-replaced longer one.

REPLACEMENTS: list[tuple[str, str]] = [
    # rgba variants — must precede bare hex to avoid partial matches
    ("rgba(14, 165, 233",  "rgba(255, 106, 61"),   # Sky 500 w/ spaces
    ("rgba(2, 132, 199",   "rgba(224, 78, 40)"),    # Sky 600 w/ spaces  (note: keep trailing ")" out — value continues)
    ("rgba(56, 189, 248",  "rgba(255, 106, 61"),    # Sky 400 w/ spaces
    ("rgba(14,165,233",    "rgba(255,106,61"),       # Sky 500 compact
    ("rgba(2,132,199",     "rgba(224,78,40"),        # Sky 600 compact
    ("rgba(56,189,248",    "rgba(255,106,61"),       # Sky 400 compact
    # Dark background rgba
    ("rgba(2, 6, 23,",     "rgba(11, 15, 20,"),     # #020617 w/ spaces
    ("rgba(2,6,23,",       "rgba(11,15,20,"),        # compact
    # Bare hex — sky-blue primary
    ("#0284c7",  "#E04E28"),   # Sky 600 → Muted Ember (light primary)
    ("#38bdf8",  "#FF6A3D"),   # Sky 400 → Bright Ember (dark primary)
    ("#0ea5e9",  "#FF6A3D"),   # Sky 500 → Bright Ember
    # Upstream dark background
    ("#020617",  "#0B0F14"),   # Slate 950 → Deep Charcoal
    # Upstream purple avatar gradient → BossFang ember avatar gradient
    # linear-gradient(135deg,#a78bfa,#7c3aed) is used for user avatar circles
    # in App.tsx and must be replaced with the ember equivalent.
    ("linear-gradient(135deg,#a78bfa,#7c3aed)",  "linear-gradient(135deg,#FF6A3D,#E04E28)"),
]

# ── Fix rgba(2, 132, 199) closing paren issue ─────────────────────────────────
# The replacement above for "rgba(2, 132, 199" intentionally stops before the
# trailing comma/value so the rest of the rgba(...) is preserved. The extra ")"
# in the new string was a typo — corrected here:
REPLACEMENTS = [
    (old, new.rstrip(")")) if new.endswith(")") and not old.endswith(")") else (old, new)
    for old, new in REPLACEMENTS
]


def enforce_file(path: Path) -> bool:
    """Apply all replacements to *path*. Returns True if the file was modified."""
    try:
        original = path.read_text(encoding="utf-8")
    except (UnicodeDecodeError, PermissionError):
        return False

    content = original
    for old, new in REPLACEMENTS:
        content = content.replace(old, new)

    if content == original:
        return False

    path.write_text(content, encoding="utf-8")
    return True


def audit_file(path: Path) -> list[str]:
    """Read-only scan. Returns the upstream-token LHS strings found in *path*."""
    try:
        content = path.read_text(encoding="utf-8")
    except (UnicodeDecodeError, PermissionError):
        return []
    return [old for old, _ in REPLACEMENTS if old in content]


def scan_dir(root: Path) -> list[Path]:
    if not root.exists():
        return []
    return [p for p in root.rglob("*") if p.is_file() and p.suffix in EXTENSIONS]


def main() -> int:
    check_mode = "--check" in sys.argv[1:]
    changed_or_offending: list[tuple[Path, list[str]]] = []

    for scan_root in SCAN_DIRS:
        label = scan_root.relative_to(REPO_ROOT)
        files = scan_dir(scan_root)
        if not files:
            print(f"[enforce-branding] {label}: no files found — skipping")
            continue
        verb = "auditing" if check_mode else "scanning"
        print(f"[enforce-branding] {verb} {label} ({len(files)} files) …")
        for f in files:
            if check_mode:
                hits = audit_file(f)
                if hits:
                    changed_or_offending.append((f, hits))
            else:
                if enforce_file(f):
                    changed_or_offending.append((f, []))

    if not changed_or_offending:
        if check_mode:
            print("[enforce-branding] ✓ --check: no upstream tokens detected")
        else:
            print("[enforce-branding] ✓ all brand tokens already correct — nothing changed")
        return 0

    if check_mode:
        print(f"\n[enforce-branding] ✗ --check: {len(changed_or_offending)} file(s) contain upstream tokens:")
        for p, hits in changed_or_offending:
            rel = p.relative_to(REPO_ROOT)
            print(f"  {rel}")
            for h in hits:
                print(f"    found: {h!r}")
        print("\nRun `python3 scripts/enforce-branding.py` (without --check) to fix automatically.")
        print("Anything left after a fix run is a manual case (e.g. new SVG fang glyph).")
        return 1

    print(f"\n[enforce-branding] ✓ {len(changed_or_offending)} file(s) patched:")
    for p, _ in changed_or_offending:
        print(f"  {p.relative_to(REPO_ROOT)}")
    print("\nReview with:  git diff")
    print("Commit with:  git add -p && git commit -m 'chore(dashboard): enforce BossFang brand tokens'")
    return 0


if __name__ == "__main__":
    sys.exit(main())
