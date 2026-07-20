#!/usr/bin/env python3
"""Tests for scripts/check-changelog-attribution.py.

Focus: `(@user)` attribution must be recognized anywhere in a bullet's block,
not only on the `- ` marker line, so the check stays compatible with the
CHANGELOG's one-sentence-per-line prose wrapping (a long multi-sentence bullet
carries its trailing `(@houko)` on the final continuation line).

Run: python3 scripts/tests/test_check_changelog_attribution.py
"""
import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "check-changelog-attribution.py"

spec = importlib.util.spec_from_file_location("check_changelog_attribution", SCRIPT)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


def check(cond, label):
    if not cond:
        print(f"FAIL [{label}]", file=sys.stderr)
        sys.exit(1)


def main() -> None:
    bba = mod.bullet_block_has_attribution

    # Single-line bullet with attribution on the marker line.
    lines = ["- One-line bullet. (#1) (@houko)"]
    check(bba(lines, 0), "single-line attributed")

    # Multi-line bullet — attribution on the final continuation line (the
    # shape the one-sentence-per-line reformat produces). This is the
    # regression the fix targets: the marker line alone has no `(@user)`.
    lines = [
        "- First sentence.",
        "  Second sentence.",
        "  Third sentence. (#2) (@houko)",
    ]
    check(not mod.has_attribution(lines[0]), "marker line alone is unattributed")
    check(bba(lines, 0), "multi-line attributed on continuation")

    # Multi-line bullet with NO attribution anywhere must still be caught.
    lines = ["- First sentence.", "  Second sentence with no attribution."]
    check(not bba(lines, 0), "multi-line unattributed is flagged")

    # A bullet's block ends at a blank line: attribution belonging to a LATER
    # bullet must not leak backwards into an unattributed one.
    lines = [
        "- Unattributed bullet.",
        "",
        "- Later bullet. (@houko)",
    ]
    check(not bba(lines, 0), "attribution does not leak across a blank line")

    # A bullet's block ends at the next bullet marker (no blank between).
    lines = [
        "- Unattributed bullet.",
        "- Next bullet. (@houko)",
    ]
    check(not bba(lines, 0), "attribution does not leak across an adjacent bullet")

    # `# pragma: no-attribution` on a continuation line exempts the bullet.
    lines = [
        "- Historical bullet.",
        "  wrapped detail. # pragma: no-attribution",
    ]
    check(bba(lines, 0), "pragma exemption honored on continuation")

    print("OK: check-changelog-attribution multi-line bullet tests passed.")


if __name__ == "__main__":
    main()
