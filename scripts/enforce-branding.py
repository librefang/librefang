#!/usr/bin/env python3
"""
enforce-branding.py — Restore BossFang ember palette after upstream merges.

Usage:
    python3 scripts/enforce-branding.py

What it does:
  Scans every *.ts, *.tsx, *.css, *.html, and *.json file under
  crates/librefang-api/dashboard/src/ and crates/librefang-api/static/ for
  upstream LibreFang sky-blue brand tokens and replaces them with BossFang
  ember equivalents. Safe to run multiple times (idempotent).

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
]
EXTENSIONS = {".ts", ".tsx", ".css", ".html", ".json"}

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


def scan_dir(root: Path) -> list[Path]:
    if not root.exists():
        return []
    return [p for p in root.rglob("*") if p.is_file() and p.suffix in EXTENSIONS]


def main() -> int:
    changed: list[Path] = []

    for scan_root in SCAN_DIRS:
        label = scan_root.relative_to(REPO_ROOT)
        files = scan_dir(scan_root)
        if not files:
            print(f"[enforce-branding] {label}: no files found — skipping")
            continue
        print(f"[enforce-branding] scanning {label} ({len(files)} files) …")
        for f in files:
            if enforce_file(f):
                changed.append(f)

    if not changed:
        print("[enforce-branding] ✓ all brand tokens already correct — nothing changed")
        return 0

    print(f"\n[enforce-branding] ✓ {len(changed)} file(s) patched:")
    for p in changed:
        print(f"  {p.relative_to(REPO_ROOT)}")
    print("\nReview with:  git diff")
    print("Commit with:  git add -p && git commit -m 'chore(dashboard): enforce BossFang brand tokens'")
    return 0


if __name__ == "__main__":
    sys.exit(main())
