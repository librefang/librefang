#!/usr/bin/env python3
"""
enforce-branding.py — Restore BossFang brand surface after upstream merges.

Usage:
    python3 scripts/enforce-branding.py            # apply replacements
    python3 scripts/enforce-branding.py --check    # read-only audit, exit 1 if any upstream token remains

What it does:
  Two scopes, two replacement passes:

  1. Color-token pass (dashboard / desktop frontend)
     Scans *.ts, *.tsx, *.css, *.html, *.json, *.rs under
     crates/librefang-api/dashboard/src/, crates/librefang-api/static/,
     crates/librefang-desktop/frontend/, crates/librefang-desktop/src/
     for upstream sky-blue tokens and rewrites them to BossFang ember.

  2. Docs prose pass (Markdown + JSX in the docs site)
     Scans *.mdx, *.ts, *.tsx under docs/src/app/ and docs/src/components/
     for the product name "LibreFang" (PascalCase, word-bounded) and
     rewrites to "BossFang". Skips fenced code blocks (``` … ```) and
     inline code spans (`…`) in .mdx files so Rust/TOML/CLI examples and
     identifier-shaped references (LibreFangKernel, LibreFangError,
     librefang_runtime) stay intact. Layer Internal struct names are
     preserved automatically via word-boundary regex: \bLibreFang\b
     does not match the K in LibreFangKernel.

  Both modes are safe to run multiple times (idempotent).

  --check mode performs the same scans without writing — exits 0 if
  everything is clean, exits 1 if any upstream token survives. Designed
  for the pre-push hook so a forgotten merge can't ship.

Color tokens replaced → BossFang tokens:
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

Prose tokens replaced:
  \bLibreFang\b  →  BossFang   (word-bounded; preserves LibreFangKernel etc.)

NOT handled automatically (fix manually):
  - New components that render an SVG fang glyph inside a gradient box as
    the logo — replace with <img src="/boss-libre.png" alt="BossFang" ...>.
  - Lowercase "librefang" in prose — too risky (matches crate names,
    paths, identifier prefixes). Audit and flip per case in follow-up.
  - URLs to upstream-owned domains (librefang.ai, deploy.librefang.ai,
    discord.gg/librefang) — those are upstream services we can't claim;
    GitHub permalinks to librefang/librefang issues/PRs are historical
    references that must remain stable. Audit case-by-case.
  - LibreFang inside backticked inline code (`"LibreFang Agent OS"` etc.) —
    these reference user-visible UI strings; the docs may go stale until a
    follow-up PR updates them with context.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# ── Repository layout ────────────────────────────────────────────────────────

REPO_ROOT = Path(__file__).resolve().parent.parent

# Color-token pass: existing dashboard / desktop surface.
COLOR_SCAN_DIRS = [
    REPO_ROOT / "crates/librefang-api/dashboard/src",
    REPO_ROOT / "crates/librefang-api/static",
    REPO_ROOT / "crates/librefang-desktop/frontend",
    REPO_ROOT / "crates/librefang-desktop/src",
]
COLOR_EXTENSIONS = {".ts", ".tsx", ".css", ".html", ".json", ".rs"}

# Prose pass: docs site (M4 of the rebrand-completion roadmap).
PROSE_SCAN_DIRS = [
    REPO_ROOT / "docs/src/app",
    REPO_ROOT / "docs/src/components",
]
PROSE_EXTENSIONS = {".mdx", ".ts", ".tsx"}

# ── Color replacement table ───────────────────────────────────────────────────
#
# Order matters: longer / more-specific patterns must come first so that a
# shorter pattern cannot match inside an already-replaced longer one.

COLOR_REPLACEMENTS: list[tuple[str, str]] = [
    # rgba variants — must precede bare hex to avoid partial matches
    ("rgba(14, 165, 233",  "rgba(255, 106, 61"),   # Sky 500 w/ spaces
    ("rgba(2, 132, 199",   "rgba(224, 78, 40"),    # Sky 600 w/ spaces
    ("rgba(56, 189, 248",  "rgba(255, 106, 61"),   # Sky 400 w/ spaces
    ("rgba(14,165,233",    "rgba(255,106,61"),     # Sky 500 compact
    ("rgba(2,132,199",     "rgba(224,78,40"),      # Sky 600 compact
    ("rgba(56,189,248",    "rgba(255,106,61"),     # Sky 400 compact
    # Dark background rgba
    ("rgba(2, 6, 23,",     "rgba(11, 15, 20,"),    # #020617 w/ spaces
    ("rgba(2,6,23,",       "rgba(11,15,20,"),      # compact
    # Bare hex — sky-blue primary
    ("#0284c7",  "#E04E28"),   # Sky 600 → Muted Ember (light primary)
    ("#38bdf8",  "#FF6A3D"),   # Sky 400 → Bright Ember (dark primary)
    ("#0ea5e9",  "#FF6A3D"),   # Sky 500 → Bright Ember
    # Upstream dark background
    ("#020617",  "#0B0F14"),   # Slate 950 → Deep Charcoal
    # Upstream purple avatar gradient → BossFang ember avatar gradient
    ("linear-gradient(135deg,#a78bfa,#7c3aed)",
     "linear-gradient(135deg,#FF6A3D,#E04E28)"),
]

# ── Prose replacement table ───────────────────────────────────────────────────
#
# Each entry is (compiled_regex, replacement). The regex is applied to text
# *outside* fenced code blocks and (for .mdx files) inline code spans.

PROSE_REPLACEMENTS: list[tuple[re.Pattern[str], str]] = [
    # Product name. Word-bounded so LibreFangKernel / LibreFangError /
    # LibreFangConfig (Layer Internal struct names) are not touched.
    (re.compile(r"\bLibreFang\b"), "BossFang"),
]

# Audit pattern for --check mode (no replacement, just detection).
PROSE_AUDIT_PATTERNS: list[re.Pattern[str]] = [p for p, _ in PROSE_REPLACEMENTS]


# ── Code-fence and inline-code awareness ──────────────────────────────────────

FENCE_RE = re.compile(r"^(\s*)(```|~~~)")
INLINE_CODE_RE = re.compile(r"`[^`\n]+`")


def replace_prose_in_mdx(content: str) -> str:
    """Apply prose replacements to .mdx content, skipping fenced + inline code."""
    out_lines: list[str] = []
    in_fence = False
    fence_marker: str | None = None

    for line in content.split("\n"):
        if in_fence:
            out_lines.append(line)
            m = FENCE_RE.match(line)
            if m and m.group(2) == fence_marker:
                in_fence = False
                fence_marker = None
            continue

        m = FENCE_RE.match(line)
        if m:
            in_fence = True
            fence_marker = m.group(2)
            out_lines.append(line)
            continue

        # Prose line — process around inline-code spans.
        out_lines.append(_replace_outside_inline_code(line))

    return "\n".join(out_lines)


def _replace_outside_inline_code(line: str) -> str:
    """Apply PROSE_REPLACEMENTS to text outside `…` inline code spans."""
    out: list[str] = []
    last = 0
    for m in INLINE_CODE_RE.finditer(line):
        prose = line[last : m.start()]
        for pat, repl in PROSE_REPLACEMENTS:
            prose = pat.sub(repl, prose)
        out.append(prose)
        out.append(m.group(0))  # inline-code chunk preserved verbatim
        last = m.end()
    tail = line[last:]
    for pat, repl in PROSE_REPLACEMENTS:
        tail = pat.sub(repl, tail)
    out.append(tail)
    return "".join(out)


def audit_prose_in_mdx(content: str) -> list[str]:
    """Read-only scan: returns the LHS strings of any prose-mode hit."""
    in_fence = False
    fence_marker: str | None = None
    hits: list[str] = []

    for line in content.split("\n"):
        if in_fence:
            m = FENCE_RE.match(line)
            if m and m.group(2) == fence_marker:
                in_fence = False
                fence_marker = None
            continue

        m = FENCE_RE.match(line)
        if m:
            in_fence = True
            fence_marker = m.group(2)
            continue

        # Process prose with inline-code skipping.
        last = 0
        for im in INLINE_CODE_RE.finditer(line):
            prose = line[last : im.start()]
            for pat in PROSE_AUDIT_PATTERNS:
                for found in pat.findall(prose):
                    hits.append(found if isinstance(found, str) else "".join(found))
            last = im.end()
        tail = line[last:]
        for pat in PROSE_AUDIT_PATTERNS:
            for found in pat.findall(tail):
                hits.append(found if isinstance(found, str) else "".join(found))

    return sorted(set(hits))


def replace_prose_in_tsx(content: str) -> str:
    """Apply prose replacements to .ts/.tsx content (no fence/inline-code awareness needed)."""
    for pat, repl in PROSE_REPLACEMENTS:
        content = pat.sub(repl, content)
    return content


def audit_prose_in_tsx(content: str) -> list[str]:
    hits: list[str] = []
    for pat in PROSE_AUDIT_PATTERNS:
        for found in pat.findall(content):
            hits.append(found if isinstance(found, str) else "".join(found))
    return sorted(set(hits))


# ── File-level dispatch ───────────────────────────────────────────────────────


def enforce_color_file(path: Path) -> bool:
    """Apply color replacements. Returns True if the file was modified."""
    try:
        original = path.read_text(encoding="utf-8")
    except (UnicodeDecodeError, PermissionError):
        return False

    content = original
    for old, new in COLOR_REPLACEMENTS:
        content = content.replace(old, new)

    if content == original:
        return False
    path.write_text(content, encoding="utf-8")
    return True


def audit_color_file(path: Path) -> list[str]:
    try:
        content = path.read_text(encoding="utf-8")
    except (UnicodeDecodeError, PermissionError):
        return []
    return [old for old, _ in COLOR_REPLACEMENTS if old in content]


def enforce_prose_file(path: Path) -> bool:
    try:
        original = path.read_text(encoding="utf-8")
    except (UnicodeDecodeError, PermissionError):
        return False

    if path.suffix == ".mdx":
        content = replace_prose_in_mdx(original)
    else:
        content = replace_prose_in_tsx(original)

    if content == original:
        return False
    path.write_text(content, encoding="utf-8")
    return True


def audit_prose_file(path: Path) -> list[str]:
    try:
        content = path.read_text(encoding="utf-8")
    except (UnicodeDecodeError, PermissionError):
        return []
    if path.suffix == ".mdx":
        return audit_prose_in_mdx(content)
    return audit_prose_in_tsx(content)


def scan_dir(root: Path, extensions: set[str]) -> list[Path]:
    if not root.exists():
        return []
    return [p for p in root.rglob("*") if p.is_file() and p.suffix in extensions]


def main() -> int:
    check_mode = "--check" in sys.argv[1:]
    offending: list[tuple[Path, list[str]]] = []
    changed: list[Path] = []

    # ── Pass 1: color tokens ──────────────────────────────────────────────────
    for scan_root in COLOR_SCAN_DIRS:
        label = scan_root.relative_to(REPO_ROOT)
        files = scan_dir(scan_root, COLOR_EXTENSIONS)
        if not files:
            print(f"[enforce-branding] {label}: no files found — skipping")
            continue
        verb = "auditing" if check_mode else "scanning"
        print(f"[enforce-branding] {verb} {label} (color tokens, {len(files)} files) …")
        for f in files:
            if check_mode:
                hits = audit_color_file(f)
                if hits:
                    offending.append((f, hits))
            else:
                if enforce_color_file(f):
                    changed.append(f)

    # ── Pass 2: docs prose ────────────────────────────────────────────────────
    for scan_root in PROSE_SCAN_DIRS:
        label = scan_root.relative_to(REPO_ROOT)
        files = scan_dir(scan_root, PROSE_EXTENSIONS)
        if not files:
            print(f"[enforce-branding] {label}: no files found — skipping")
            continue
        verb = "auditing" if check_mode else "scanning"
        print(f"[enforce-branding] {verb} {label} (docs prose, {len(files)} files) …")
        for f in files:
            if check_mode:
                hits = audit_prose_file(f)
                if hits:
                    offending.append((f, hits))
            else:
                if enforce_prose_file(f):
                    changed.append(f)

    if check_mode:
        if not offending:
            print("[enforce-branding] ✓ --check: no upstream tokens detected")
            return 0
        print(f"\n[enforce-branding] ✗ --check: {len(offending)} file(s) contain upstream tokens:")
        for p, hits in offending:
            rel = p.relative_to(REPO_ROOT)
            print(f"  {rel}")
            for h in hits:
                print(f"    found: {h!r}")
        print("\nRun `python3 scripts/enforce-branding.py` (without --check) to fix automatically.")
        print("Anything left after a fix run is a manual case (e.g. new SVG fang glyph,")
        print("or LibreFang inside inline code that documents UI strings).")
        return 1

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
