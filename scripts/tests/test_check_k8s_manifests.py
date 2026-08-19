#!/usr/bin/env python3
"""Focused regressions for Kubernetes manifest validator diagnostics."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import sys
import unittest
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "check_k8s_manifests", ROOT / "scripts" / "check-k8s-manifests.py"
)
validator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validator)


class ValidatorErrorTests(unittest.TestCase):
    def test_too_many_arguments_returns_usage_status_two(self):
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            status = validator.main(["check", "one.yaml", "two.yaml"])

        self.assertEqual(status, 2)
        self.assertEqual(stderr.getvalue(), "usage: check [rendered.yaml]\n")

    def test_unreadable_path_returns_input_status_two(self):
        stderr = io.StringIO()
        with (
            patch.object(validator.Path, "read_text", side_effect=OSError("denied")),
            contextlib.redirect_stderr(stderr),
        ):
            status = validator.main(["check", "private.yaml"])

        self.assertEqual(status, 2)
        self.assertIn("cannot read manifest input 'private.yaml': denied", stderr.getvalue())

    def test_unreadable_stdin_returns_input_status_two(self):
        class BrokenInput:
            def read(self):
                raise OSError("closed")

        stderr = io.StringIO()
        with (
            patch.object(sys, "stdin", BrokenInput()),
            contextlib.redirect_stderr(stderr),
        ):
            status = validator.main(["check"])

        self.assertEqual(status, 2)
        self.assertIn("cannot read manifest input from stdin: closed", stderr.getvalue())

    def test_duplicate_statefulsets_report_the_exact_count(self):
        docs = [{"kind": "StatefulSet"}, {"kind": "StatefulSet"}]
        stderr = io.StringIO()
        with (
            patch.object(sys, "stdin", io.StringIO("documents")),
            patch.object(validator, "load_documents", return_value=docs),
            contextlib.redirect_stderr(stderr),
        ):
            status = validator.main(["check"])

        self.assertEqual(status, 1)
        self.assertIn("expected exactly one StatefulSet, found 2", stderr.getvalue())

    def test_explicit_zero_startup_probe_values_are_not_defaulted(self):
        failures = validator.Failures()
        container = {
            "startupProbe": {
                "httpGet": {"path": "/api/ready", "port": "http"},
                "periodSeconds": 0,
                "failureThreshold": 0,
            },
            "livenessProbe": {"httpGet": {"path": "/api/health", "port": "http"}},
            "readinessProbe": {"httpGet": {"path": "/api/ready", "port": "http"}},
        }

        validator.check_probes(container, failures)

        self.assertTrue(any("budget is only 0s" in item for item in failures.items))

    def test_invalid_yaml_returns_input_status_two(self):
        stderr = io.StringIO()
        with (
            patch.object(sys, "stdin", io.StringIO("kind: [unterminated")),
            contextlib.redirect_stderr(stderr),
        ):
            status = validator.main(["check"])

        self.assertEqual(status, 2)
        self.assertIn("error: invalid YAML manifest input:", stderr.getvalue())

    def test_missing_kind_is_rendered_without_sorting_crash(self):
        docs = [{"apiVersion": "v1"}, {"kind": "Service"}]
        stderr = io.StringIO()
        with (
            patch.object(sys, "stdin", io.StringIO("documents")),
            patch.object(validator, "load_documents", return_value=docs),
            contextlib.redirect_stderr(stderr),
        ):
            status = validator.main(["check"])

        self.assertEqual(status, 1)
        self.assertIn("rendered kinds: ['Service', None]", stderr.getvalue())

    def test_malformed_nested_structure_returns_input_status_two(self):
        stderr = io.StringIO()
        with (
            patch.object(sys, "stdin", io.StringIO("kind: StatefulSet\nspec: []\n")),
            contextlib.redirect_stderr(stderr),
        ):
            status = validator.main(["check"])

        self.assertEqual(status, 2)
        self.assertIn("error: invalid Kubernetes manifest structure:", stderr.getvalue())

    def test_non_integer_startup_probe_values_are_reported(self):
        failures = validator.Failures()
        container = {
            "startupProbe": {
                "httpGet": {"path": "/api/ready", "port": "http"},
                "periodSeconds": "10",
                "failureThreshold": None,
            },
            "livenessProbe": {"httpGet": {"path": "/api/health", "port": "http"}},
            "readinessProbe": {"httpGet": {"path": "/api/ready", "port": "http"}},
        }

        validator.check_probes(container, failures)

        self.assertTrue(
            any("must be integers" in item for item in failures.items),
            failures.items,
        )

    def test_non_mapping_probe_values_are_reported(self):
        failures = validator.Failures()
        container = {
            "startupProbe": [],
            "livenessProbe": {"httpGet": []},
            "readinessProbe": {"httpGet": {"path": "/api/ready", "port": "http"}},
        }

        validator.check_probes(container, failures)

        self.assertTrue(any("startupProbe must be a mapping" in item for item in failures.items))
        self.assertTrue(
            any("livenessProbe.httpGet must be a mapping" in item for item in failures.items)
        )


if __name__ == "__main__":
    unittest.main()
