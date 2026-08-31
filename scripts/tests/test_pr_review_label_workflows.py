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
        self.assertIn("github.event.pull_request.number || github.run_id", self.collector)
        self.assertIn("github.event.review.state == 'approved'", self.collector)
        self.assertIn("github.event.review.state == 'changes_requested'", self.collector)
        self.assertIn("cancel-in-progress: true", self.collector)

    def test_applier_serializes_each_pr_without_interrupting_mutations(self) -> None:
        self.assertIn("pr_number: ${{ steps.payload.outputs.pr_number }}", self.applier)
        self.assertIn("group: pr-review-applier-${{ needs.prepare.outputs.pr_number }}", self.applier)
        self.assertNotIn("github.event.workflow_run.pull_requests", self.applier)
        self.assertNotIn("github.event.workflow_run.head_sha", self.applier)
        self.assertIn("cancel-in-progress: false", self.applier)

    def test_both_jobs_are_bounded(self) -> None:
        self.assertEqual(self.collector.count("timeout-minutes: 5"), 1)
        self.assertEqual(self.applier.count("timeout-minutes: 5"), 2)

    def test_payload_is_validated_before_the_mutating_job(self) -> None:
        self.assertIn("id: payload", self.applier)
        self.assertIn("core.setOutput('pr_number', String(prNumber))", self.applier)
        self.assertIn("core.setOutput('review_state', reviewState)", self.applier)
        self.assertIn("PR_NUMBER: ${{ needs.prepare.outputs.pr_number }}", self.applier)
        self.assertIn("REVIEW_STATE: ${{ needs.prepare.outputs.review_state }}", self.applier)

    def test_removal_only_ignores_missing_labels(self) -> None:
        self.assertNotIn(".catch(() => {})", self.applier)
        self.assertIn("if (error.status !== 404) throw error", self.applier)

    def test_approval_uses_fresh_blockers(self) -> None:
        self.assertIn("const { data: freshPr }", self.applier)
        self.assertIn("freshLabels.includes('has-conflicts')", self.applier)
        self.assertIn("await removeLabel(READY)", self.applier)

    def test_applier_job_can_write_pull_request_labels(self) -> None:
        # The labels endpoint lives under `/issues/{n}/labels`, but GitHub gates it on the
        # pull_requests scope when `{n}` is a pull request.
        # #7546 moved permissions to the job level and narrowed this to `pull-requests: read`;
        # every applier run 403'd with "Resource not accessible by integration" from then on.
        apply_job = self.applier.split("\n  apply:\n", 1)[1]
        self.assertIn("issues: write", apply_job)
        self.assertIn("pull-requests: write", apply_job)
        self.assertNotIn("pull-requests: read", apply_job)

    def test_prepare_job_can_read_the_collector_artifact(self) -> None:
        # `actions/download-artifact` needs `actions: read` to reach another run's artifact.
        prepare_job = self.applier.split("\n  prepare:\n", 1)[1].split("\n  apply:\n", 1)[0]
        self.assertIn("actions: read", prepare_job)

    def test_applier_reconciles_latest_review_instead_of_payload_order(self) -> None:
        self.assertIn("github.rest.pulls.listReviews", self.applier)
        self.assertIn("const latestReview = reviews", self.applier)
        self.assertIn("Date.parse(left.submitted_at || '')", self.applier)
        self.assertIn("const reviewState = latestReview.state.toLowerCase()", self.applier)
        self.assertIn("stale payload ${payloadReviewState}", self.applier)
        self.assertNotIn("const reviewState = process.env.REVIEW_STATE", self.applier)


if __name__ == "__main__":
    unittest.main()
