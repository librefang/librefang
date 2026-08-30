#!/usr/bin/env python3
"""Regression checks for PR status-label reconciliation."""

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/pr-status-labels.yml"

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

    def test_only_per_pr_runs_cancel_each_other(self) -> None:
        # `pull_request_target`, `check_suite` and `push` all report `github.ref` as a branch of
        # this repository rather than as a PR ref, so a ref-keyed group would put every open PR in
        # one bucket and let one PR's run cancel another's.
        self.assertIn(
            "group: ${{ github.workflow }}-${{ github.event_name }}-"
            "${{ github.event.pull_request.number || github.event.check_suite.head_sha"
            " || github.ref }}",
            self.workflow,
        )
        # The `push` / `workflow_dispatch` sweeps walk every open PR, and a push to main is exactly
        # what makes mergeability null repository-wide — cancelling them mid-merge-batch drops the
        # detection of newly conflicting PRs, with nothing left to retrigger it.
        self.assertIn(
            "cancel-in-progress: ${{ github.event_name == 'pull_request_target'"
            " || github.event_name == 'check_suite' }}",
            self.workflow,
        )

    def test_jobs_are_bounded_proportionally_to_their_work(self) -> None:
        # `conflicts` is the outlier on purpose: the main-push sweep walks every open PR and sleeps
        # 5s per retry on each one whose mergeability GitHub has not recomputed yet.
        self.assertEqual(
            job_timeouts(self.workflow),
            {
                "conflicts": 20,
                "ci-status": 5,
                "size": 5,
                "docs-only": 5,
                "review-state": 5,
            },
        )


if __name__ == "__main__":
    unittest.main()
