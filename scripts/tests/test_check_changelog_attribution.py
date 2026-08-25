#!/usr/bin/env python3
"""Regressions for changelog attribution and fragment validation."""

from __future__ import annotations

import argparse
import contextlib
import importlib.util
import io
import os
import unittest
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "check-changelog-attribution.py"
SPEC = importlib.util.spec_from_file_location("check_changelog_attribution", SCRIPT)
mod = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mod)


class BulletBlockTests(unittest.TestCase):
    def test_single_line_attribution(self):
        self.assertTrue(mod.bullet_block_has_attribution(["- Entry. (@houko)"], 0))

    def test_continuation_attribution(self):
        lines = ["- First.", "  Second.", "  Third. (@houko)"]
        self.assertFalse(mod.has_attribution(lines[0]))
        self.assertTrue(mod.bullet_block_has_attribution(lines, 0))

    def test_missing_attribution(self):
        self.assertFalse(
            mod.bullet_block_has_attribution(["- First.", "  No attribution."], 0)
        )

    def test_attribution_does_not_cross_blank_or_adjacent_bullet(self):
        for lines in (
            ["- Missing.", "", "- Later. (@houko)"],
            ["- Missing.", "- Adjacent. (@houko)"],
        ):
            with self.subTest(lines=lines):
                self.assertFalse(mod.bullet_block_has_attribution(lines, 0))

    def test_pragma_on_continuation(self):
        lines = ["- Historical.", "  Detail. # pragma: no-attribution"]
        self.assertTrue(mod.bullet_block_has_attribution(lines, 0))


class FragmentTests(unittest.TestCase):
    path = "changelog.d/fixed/example.md"

    def test_attributed_fragment_and_continuation_pass(self):
        self.assertEqual(mod.check_fragment(self.path, "Fix. (@houko)\n"), [])
        self.assertEqual(
            mod.check_fragment(self.path, "First.\n  Last. (@houko)\n"), []
        )

    def test_missing_attribution_has_exact_reason_and_line(self):
        violations = mod.check_fragment(self.path, "Fix.\n")
        self.assertEqual(
            violations,
            [mod.Violation(self.path, 1, "Fix.", mod.MISSING_ATTRIBUTION)],
        )

    def test_blank_line_ends_fragment_block(self):
        self.assertEqual(len(mod.check_fragment(self.path, "Fix.\n\n(@houko)\n")), 1)

    def test_empty_fragment_has_exact_reason(self):
        self.assertEqual(
            mod.check_fragment(self.path, "\n  \n"),
            [mod.Violation(self.path, 1, "", "fragment is empty")],
        )

    def test_unknown_and_missing_sections_are_rejected(self):
        for path in ("changelog.d/fix/example.md", "changelog.d/example.md"):
            with self.subTest(path=path):
                is_fragment, problem = mod.classify_fragment_path(path)
                self.assertTrue(is_fragment)
                self.assertIsNotNone(problem)
                violations = mod.check_fragment(path, "Fix. (@houko)\n")
                self.assertEqual(len(violations), 1)
                self.assertEqual(violations[0].reason, problem)

    def test_infrastructure_is_not_a_fragment(self):
        for path in (
            "changelog.d/README.md",
            "changelog.d/fixed/.gitkeep",
            "changelog.d/added/notes.txt",
        ):
            with self.subTest(path=path):
                classification = mod.classify_fragment_path(path)
                self.assertEqual(classification, (False, None))
                self.assertEqual(mod.check_fragment(path, "unattributed\n"), [])


class DiffParsingTests(unittest.TestCase):
    def test_no_newline_marker_does_not_advance_post_image_line(self):
        diff = "\n".join(
            (
                "diff --git a/CHANGELOG.md b/CHANGELOG.md",
                "@@ -2 +2,2 @@",
                "-- Replaced line without a newline.",
                "\\ No newline at end of file",
                "+- Attributed. (@houko)",
                "+- Missing attribution.",
            )
        )
        head = "\n".join(
            (
                "## [Unreleased]",
                "- Attributed. (@houko)",
                "- Missing attribution.",
                "## [2026.1.1]",
            )
        )
        with patch.object(mod, "run_git", side_effect=(diff, head)):
            violations = mod.added_lines_in_unreleased("base", "head", ROOT)

        self.assertEqual(
            violations,
            [mod.Violation("CHANGELOG.md", 3, "- Missing attribution.")],
        )

    def test_partial_diff_range_is_rejected(self):
        cases = (
            ("base", None, {}),
            (None, "head", {}),
            (None, None, {"BASE_SHA": "base"}),
            (None, None, {"HEAD_SHA": "head"}),
            ("cli-base", None, {"HEAD_SHA": "env-head"}),
            (None, "cli-head", {"BASE_SHA": "env-base"}),
            ("cli-base", None, {"BASE_SHA": "env-base", "HEAD_SHA": "env-head"}),
        )
        for base, head, environment in cases:
            with (
                self.subTest(base=base, head=head, environment=environment),
                patch.dict(os.environ, environment, clear=True),
                contextlib.redirect_stderr(io.StringIO()),
            ):
                args = argparse.Namespace(base=base, head=head)
                with self.assertRaises(SystemExit) as caught:
                    mod.resolve_diff_range(args)
                self.assertEqual(caught.exception.code, 2)

    def test_staged_no_newline_marker_does_not_advance_post_image_line(self):
        diff = "\n".join(
            (
                "diff --git a/CHANGELOG.md b/CHANGELOG.md",
                "@@ -2 +2,2 @@",
                "-- Replaced line without a newline.",
                "\\ No newline at end of file",
                "+- Attributed. (@houko)",
                "+- Missing attribution.",
            )
        )
        staged = "\n".join(
            (
                "## [Unreleased]",
                "- Attributed. (@houko)",
                "- Missing attribution.",
                "## [2026.1.1]",
            )
        )
        with patch.object(mod, "run_git", side_effect=(diff, staged)):
            violations = mod.scan_staged_added_lines(ROOT)

        self.assertEqual(
            violations,
            [mod.Violation("CHANGELOG.md", 3, "- Missing attribution.")],
        )

    def test_complete_cli_range_overrides_environment(self):
        args = argparse.Namespace(base="cli-base", head="cli-head")
        with patch.dict(os.environ, {"BASE_SHA": "env-only"}, clear=True):
            self.assertEqual(
                mod.resolve_diff_range(args),
                ("cli-base", "cli-head"),
            )

    def test_complete_environment_range_is_used(self):
        args = argparse.Namespace(base=None, head=None)
        environment = {"BASE_SHA": "env-base", "HEAD_SHA": "env-head"}
        with patch.dict(os.environ, environment, clear=True):
            self.assertEqual(
                mod.resolve_diff_range(args),
                ("env-base", "env-head"),
            )


if __name__ == "__main__":
    unittest.main()
