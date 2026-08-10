#!/usr/bin/env python3
"""Regression checks for CI, discussion backfill, and weekly report workflows."""

import os
import re
import shutil
import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
DISCUSSION = ROOT / ".github/workflows/discussion-to-issue.yml"
WEEKLY = ROOT / ".github/workflows/weekly-report.yml"
CI = ROOT / ".github/workflows/ci.yml"
TRUFFLEHOG_INSTALLER_COMMIT = "690e5c7aff8347c3885096f3962a0633d9129607"


def workflow_job(workflow: str, name: str) -> str:
    match = re.search(
        rf"^  {re.escape(name)}:\n(?P<body>(?:(?!^  [A-Za-z0-9_-]+:\n).)*)",
        workflow,
        flags=re.MULTILINE | re.DOTALL,
    )
    if match is None:
        raise AssertionError(f"workflow job not found: {name}")
    return match.group("body")


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
        report_result = subprocess.run(
            [actionlint, str(DISCUSSION), str(WEEKLY)],
            cwd=ROOT,
            text=True,
            capture_output=True,
        )
        self.assertEqual(
            report_result.returncode,
            0,
            report_result.stdout + report_result.stderr,
        )
        # ci.yml has pre-existing shellcheck findings outside this report's scope.
        # Keep actionlint's YAML/expression checks as a durable gate while the two smaller workflows retain full shellcheck coverage.
        ci_result = subprocess.run(
            [actionlint, "-shellcheck=", str(CI)],
            cwd=ROOT,
            text=True,
            capture_output=True,
        )
        self.assertEqual(ci_result.returncode, 0, ci_result.stdout + ci_result.stderr)

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

    def test_ci_routes_untrusted_values_without_script_interpolation(self):
        workflow = CI.read_text(encoding="utf-8")

        self.assertIn(r"sed -n 's|^crates/\([A-Za-z0-9_-][A-Za-z0-9_-]*\)/.*|\1|p'", workflow)
        self.assertNotIn('CRATES="${{ needs.changes.outputs.crates }}"', workflow)
        self.assertEqual(
            workflow.count("CRATES: ${{ needs.changes.outputs.crates }}"),
            4,
        )
        self.assertNotIn("origin/${{ github.base_ref }}", workflow)
        self.assertIn("BASE_REF: ${{ github.base_ref }}", workflow)
        self.assertIn('"origin/${BASE_REF}...HEAD"', workflow)

    def test_ci_long_running_jobs_have_timeouts(self):
        workflow = CI.read_text(encoding="utf-8")

        for job, minutes in {
            "quality": 30,
            "skills-wasm-sdk": 20,
            "rust-telegram-sidecar": 30,
            "security": 15,
        }.items():
            with self.subTest(job=job):
                self.assertIn(f"timeout-minutes: {minutes}", workflow_job(workflow, job))

    def test_ci_pins_trufflehog_installer_to_release_commit(self):
        workflow = CI.read_text(encoding="utf-8")

        self.assertNotIn("trufflehog/main/scripts/install.sh", workflow)
        self.assertIn(
            f"trufflehog/{TRUFFLEHOG_INSTALLER_COMMIT}/scripts/install.sh",
            workflow,
        )


if __name__ == "__main__":
    unittest.main()
