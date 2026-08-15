#!/usr/bin/env python3
"""Regression checks for issue-label state reconciliation."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/issue-labels.yml"


class IssueLabelsWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_same_issue_events_are_serial(self) -> None:
        self.assertIn("group: issue-labels-${{ github.event.issue.number", self.workflow)
        self.assertIn("|| 'backfill'", self.workflow)
        self.assertIn("cancel-in-progress: false", self.workflow)

    def test_response_state_uses_environment_and_propagates_failures(self) -> None:
        for binding in (
            "ISSUE_NUMBER: ${{ github.event.issue.number }}",
            "EVENT_NAME: ${{ github.event_name }}",
            "COMMENT_ASSOC: ${{ github.event.comment.author_association }}",
        ):
            self.assertIn(binding, self.workflow)
        self.assertNotIn('ISSUE="${{ github.event.issue.number }}"', self.workflow)
        self.assertNotIn("2>/dev/null || true", self.workflow)
        self.assertIn("labels_json=$(gh issue view", self.workflow)

    def test_reopened_issues_clear_stale_response_state(self) -> None:
        self.assertIn("github.event.action == 'reopened'", self.workflow)
        self.assertIn('if [ "$EVENT_NAME" = "issues" ]', self.workflow)

    def test_backfill_paginates_every_open_issue(self) -> None:
        self.assertIn("gh api --paginate --slurp", self.workflow)
        self.assertIn("select(.pull_request == null)", self.workflow)
        self.assertNotIn("--limit 1000", self.workflow)

    def test_jobs_have_proportional_timeouts(self) -> None:
        self.assertEqual(self.workflow.count("timeout-minutes: 5"), 2)
        self.assertEqual(self.workflow.count("timeout-minutes: 15"), 1)


if __name__ == "__main__":
    unittest.main()
