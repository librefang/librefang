#!/usr/bin/env python3
"""Regression checks for the collector/applier review-label pipeline."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
COLLECTOR = ROOT / ".github/workflows/pr-review-collector.yml"
APPLIER = ROOT / ".github/workflows/pr-review-state-applier.yml"


class PrReviewLabelWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.collector = COLLECTOR.read_text(encoding="utf-8")
        cls.applier = APPLIER.read_text(encoding="utf-8")

    def test_collector_keeps_only_latest_review_per_pr(self) -> None:
        self.assertIn("group: pr-review-collector-${{ github.event.pull_request.number }}", self.collector)
        self.assertIn("cancel-in-progress: true", self.collector)

    def test_applier_serializes_each_pr_without_interrupting_mutations(self) -> None:
        self.assertIn("github.event.workflow_run.pull_requests[0].number", self.applier)
        self.assertIn("github.event.workflow_run.head_sha", self.applier)
        self.assertIn("cancel-in-progress: false", self.applier)

    def test_both_jobs_are_bounded(self) -> None:
        self.assertEqual(self.collector.count("timeout-minutes: 5"), 1)
        self.assertEqual(self.applier.count("timeout-minutes: 5"), 1)

    def test_removal_only_ignores_missing_labels(self) -> None:
        self.assertNotIn(".catch(() => {})", self.applier)
        self.assertIn("if (error.status !== 404) throw error", self.applier)

    def test_approval_uses_fresh_blockers(self) -> None:
        self.assertIn("const { data: freshPr }", self.applier)
        self.assertIn("freshLabels.includes('has-conflicts')", self.applier)
        self.assertIn("await removeLabel(READY)", self.applier)


if __name__ == "__main__":
    unittest.main()
