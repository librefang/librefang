#!/usr/bin/env python3
"""Regression checks for umbrella auto-close mutation serialization."""

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "close-umbrella-on-last-pr.yml"


class CloseUmbrellaWorkflowTests(unittest.TestCase):
    def test_mutating_runs_are_serialized_without_cancellation(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        match = re.search(
            r"(?m)^concurrency:\n"
            r"  group: (?P<group>[^\n]+)\n"
            r"  cancel-in-progress: (?P<cancel>[^\n]+)$",
            text,
        )

        self.assertIsNotNone(match, "workflow must declare top-level concurrency")
        assert match is not None
        self.assertEqual(match.group("group"), "umbrella-autoclose")
        self.assertEqual(match.group("cancel"), "false")
        self.assertLess(
            match.start(),
            text.index("\njobs:\n"),
            "concurrency must serialize complete workflow runs, not one step",
        )


if __name__ == "__main__":
    unittest.main()
