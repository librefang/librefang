#!/usr/bin/env python3
"""Regression checks for PR status-label reconciliation."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/pr-status-labels.yml"


class PrStatusLabelsWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_empty_external_check_set_preserves_labels(self) -> None:
        guard = "if (externalRuns.length === 0)"
        self.assertIn(guard, self.workflow)
        self.assertLess(self.workflow.index(guard), self.workflow.index("const hasFailure"))

    def test_conflict_reconciliation_is_complete_and_conservative(self) -> None:
        self.assertIn("prs = await github.paginate(github.rest.pulls.list", self.workflow)
        self.assertIn("attempt < 3", self.workflow)
        self.assertIn("if (mergeable === null)", self.workflow)
        self.assertIn("else if (mergeable === true && hasLabel)", self.workflow)
        self.assertNotIn("mergeable !== false && hasLabel", self.workflow)

    def test_label_removal_only_ignores_absence(self) -> None:
        self.assertNotIn(".catch(() => {})", self.workflow)
        self.assertEqual(
            self.workflow.count("if (error.status !== 404) throw error"),
            6,
        )

    def test_jobs_are_bounded_and_same_sha_runs_are_serial(self) -> None:
        self.assertIn("group: pr-status-labels-${{", self.workflow)
        self.assertIn("cancel-in-progress: false", self.workflow)
        self.assertEqual(self.workflow.count("timeout-minutes: 5"), 5)


if __name__ == "__main__":
    unittest.main()
