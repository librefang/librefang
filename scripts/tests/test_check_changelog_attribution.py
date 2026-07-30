#!/usr/bin/env python3
"""Tests for scripts/check-changelog-attribution.py.

Two focuses:

* `(@user)` attribution must be recognized anywhere in a bullet's block, not
  only on the `- ` marker line, so the check stays compatible with the
  CHANGELOG's one-sentence-per-line prose wrapping (a long multi-sentence bullet
  carries its trailing `(@houko)` on the final continuation line).
* A `changelog.d/` fragment is held to the same standard as an `[Unreleased]`
  bullet, and a fragment in an unrecognised section directory is rejected
  outright — assembly has no heading to render it under and would drop it
  silently.

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

    fragment_tests()

    print("OK: check-changelog-attribution bullet + fragment tests passed.")


def fragment_tests() -> None:
    check_fragment = mod.check_fragment
    classify = mod.classify_fragment_path

    # A well-formed fragment in a recognised section passes.
    ok = "changelog.d/fixed/6623-wire-max-content-chars.md"
    check(classify(ok) == (True, None), "well-formed fragment path accepted")
    check(
        check_fragment(ok, "Fix the thing. (#6623) (@houko)\n") == [],
        "attributed fragment passes",
    )

    # Multi-line body with the attribution on the final continuation line — the
    # shape the one-sentence-per-line prose rule produces.
    check(
        check_fragment(
            ok, "First sentence.\n  Second sentence.\n  Third. (#6623) (@houko)\n"
        )
        == [],
        "attribution on a fragment's continuation line counts",
    )

    # Missing attribution fails, and points at the first content line.
    missing = check_fragment(ok, "Fix the thing. (#6623)\n")
    check(len(missing) == 1, "unattributed fragment is flagged")
    check(missing[0].lineno == 1, "unattributed fragment reports line 1")
    check(
        missing[0].reason == mod.MISSING_ATTRIBUTION,
        "unattributed fragment reports the attribution reason",
    )

    # A blank line ends the bullet block, so an attribution stranded after one
    # does not count — assembly would have orphaned it into its own paragraph.
    check(
        len(check_fragment(ok, "Fix the thing.\n\n(@houko)\n")) == 1,
        "attribution past a blank line does not count",
    )

    # An empty fragment fails rather than assembling into nothing.
    empty = check_fragment(ok, "\n  \n")
    check(len(empty) == 1 and empty[0].reason == "fragment is empty", "empty fragment flagged")

    # An unrecognised section directory fails even when attribution is present:
    # assembly has no heading for it and would drop the entry silently.
    typo = "changelog.d/fix/6623-wire-max-content-chars.md"
    is_fragment, problem = classify(typo)
    check(is_fragment and problem is not None, "unrecognised section directory rejected")
    bad_section = check_fragment(typo, "Fix the thing. (#6623) (@houko)\n")
    check(len(bad_section) == 1, "unrecognised section is the only violation reported")
    check("not a recognised" in bad_section[0].reason, "unrecognised section reason is explicit")

    # A fragment dropped straight into `changelog.d/` misses its section too.
    check(
        classify("changelog.d/6623-oops.md")[0] is True
        and classify("changelog.d/6623-oops.md")[1] is not None,
        "fragment outside any section directory rejected",
    )

    # Infrastructure is not a fragment and is never scanned.
    for infra in (
        "changelog.d/README.md",
        "changelog.d/fixed/.gitkeep",
        "changelog.d/added/notes.txt",
    ):
        check(classify(infra) == (False, None), f"{infra} treated as infrastructure")
        check(check_fragment(infra, "no attribution here\n") == [], f"{infra} not scanned")


if __name__ == "__main__":
    main()
