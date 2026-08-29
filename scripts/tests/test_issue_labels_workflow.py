#!/usr/bin/env python3
"""Regression checks for issue-label state reconciliation."""

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/issue-labels.yml"

JOB_RE = re.compile(r"^  (?P<name>[a-z0-9-]+):$", re.MULTILINE)
TIMEOUT_RE = re.compile(r"^    timeout-minutes: (?P<minutes>\d+)$", re.MULTILINE)


def job_timeouts(workflow: str) -> dict[str, int]:
    """Map each top-level job name to its declared `timeout-minutes`.

    Counting raw occurrences of a timeout value cannot tell which job carries it, so a job that
    silently loses its cap still matches as long as some other job happens to use the same number.
    """
    jobs = list(JOB_RE.finditer(workflow))
    timeouts: dict[str, int] = {}
    for index, job in enumerate(jobs):
        end = jobs[index + 1].start() if index + 1 < len(jobs) else len(workflow)
        found = TIMEOUT_RE.search(workflow, job.end(), end)
        if found:
            timeouts[job.group("name")] = int(found.group("minutes"))
    return timeouts


class IssueLabelsWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_events_of_different_kinds_never_cancel_each_other(self) -> None:
        # The event name is part of the group key on purpose: `area` only runs on the `issues`
        # event, so an `issue_comment` run sharing a group would cancel it and drop the `area/*`
        # labels for good — the backfill cannot repair that, since it only visits issues carrying
        # no labels at all and the comment run has already added one.
        self.assertIn(
            "group: ${{ github.workflow }}-${{ github.event_name }}-"
            "${{ github.event.issue.number || 'backfill' }}",
            self.workflow,
        )
        # The backfill walks the whole unlabeled backlog and nothing retriggers a half-finished
        # one, so it is the single path that must survive a newer run.
        self.assertIn(
            "cancel-in-progress: ${{ github.event_name != 'workflow_dispatch' }}",
            self.workflow,
        )

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
        # Every job is capped, and the caps match the work: the single-issue label paths finish in
        # seconds, while the backfill walks the whole open backlog one API page at a time.
        self.assertEqual(
            job_timeouts(self.workflow),
            {"test-labeler": 5, "area": 5, "response-state": 5, "backfill": 15},
        )


if __name__ == "__main__":
    unittest.main()
