#!/usr/bin/env python3
"""Regression checks for discussion backfill and weekly report workflows."""

import os
import shutil
import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
DISCUSSION = ROOT / ".github/workflows/discussion-to-issue.yml"
WEEKLY = ROOT / ".github/workflows/weekly-report.yml"
CI = ROOT / ".github/workflows/ci.yml"


class WorkflowReportHardeningTests(unittest.TestCase):
    def test_discussion_backfill_is_bounded_serial_and_reports_failures(self):
        workflow = DISCUSSION.read_text(encoding="utf-8")

        self.assertIn("group: discussion-backfill", workflow)
        self.assertIn("cancel-in-progress: false", workflow)
        self.assertGreaterEqual(workflow.count("timeout-minutes:"), 3)
        self.assertNotIn('|| echo "  (skipped or failed)"', workflow)
        self.assertIn('echo "$num" >> "$failures_file"', workflow)
        self.assertIn('if [ -s "$failures_file" ]', workflow)
        self.assertIn("startsWith(github.event.comment.body, '/to-issue')", workflow)
        self.assertIn('command=${COMMENT_BODY%%[[:space:]]*}', workflow)
        self.assertIn('if [ "$command" = "/to-issue" ]', workflow)
        self.assertIn("if: steps.command.outputs.valid == 'true'", workflow)

    def test_weekly_report_fails_closed_and_uses_repository_context(self):
        workflow = WEEKLY.read_text(encoding="utf-8")

        self.assertNotIn("uses: actions/checkout", workflow)
        self.assertEqual(workflow.count("set -euo pipefail"), 3)
        self.assertIn("REPO: ${{ github.repository }}", workflow)
        self.assertNotIn('REPO="librefang/librefang"', workflow)
        self.assertNotIn("github.com/librefang/librefang", workflow)
        self.assertNotIn("NEW_CONTRIBUTORS=", workflow)
        self.assertIn("curl --fail-with-body --silent --show-error", workflow)
        self.assertIn('WEEK="${SINCE%%T*}"', workflow)

    def test_workflows_pass_actionlint(self):
        actionlint = os.environ.get("ACTIONLINT") or shutil.which("actionlint")
        self.assertIsNotNone(actionlint, "actionlint is required for this regression test")
        result = subprocess.run(
            [actionlint, str(DISCUSSION), str(WEEKLY)],
            cwd=ROOT,
            text=True,
            capture_output=True,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_ci_runs_this_regression_suite_with_pinned_actionlint(self):
        workflow = CI.read_text(encoding="utf-8")

        self.assertIn(
            "go install github.com/rhysd/actionlint/cmd/actionlint@v1.7.7",
            workflow,
        )
        self.assertIn(
            "python3 scripts/tests/test_workflow_report_hardening.py",
            workflow,
        )


if __name__ == "__main__":
    unittest.main()
