#!/usr/bin/env python3
"""Regression checks for inactive-issue reconciliation."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/issue-inactive.yml"


class IssueInactiveWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_api_failures_are_visible_and_fatal(self) -> None:
        self.assertIn("set -euo pipefail", self.workflow)
        self.assertIn("gh api --paginate --slurp", self.workflow)
        self.assertNotIn("' 2>/dev/null | jq", self.workflow)

    def test_repository_context_is_routed_through_environment(self) -> None:
        self.assertIn("REPO: ${{ github.repository }}", self.workflow)
        self.assertNotIn('REPO="${{ github.repository }}"', self.workflow)

    def test_scheduled_reconciliation_is_bounded_and_serial(self) -> None:
        self.assertIn("group: issue-inactive", self.workflow)
        self.assertIn("cancel-in-progress: false", self.workflow)
        self.assertIn("timeout-minutes: 15", self.workflow)


if __name__ == "__main__":
    unittest.main()
