#!/usr/bin/env python3
"""Regression checks for production worker deployment serialization."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/deploy-worker.yml"


class DeployWorkerWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_production_deployments_are_serial_and_not_interrupted(self) -> None:
        self.assertIn("group: deploy-workers-production", self.workflow)
        self.assertIn("cancel-in-progress: false", self.workflow)

    def test_both_jobs_have_bounded_runtime(self) -> None:
        self.assertEqual(self.workflow.count("timeout-minutes: 15"), 2)

    def test_deploy_tooling_is_exact_and_shell_is_explicit(self) -> None:
        self.assertEqual(self.workflow.count("npx -y wrangler@4.121.0 deploy"), 2)
        self.assertEqual(self.workflow.count("shell: bash"), 3)
        self.assertNotIn("wrangler@4 deploy", self.workflow)


if __name__ == "__main__":
    unittest.main()
