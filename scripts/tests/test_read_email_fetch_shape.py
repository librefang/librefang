#!/usr/bin/env python3
"""Regression tests for validating IMAP FETCH response structure."""

import contextlib
import importlib.util
import io
import os
import sys
import unittest
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("read_email", ROOT / "scripts" / "read_email.py")
read_email = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(read_email)


class _FakeIMAP:
    def __init__(self, fetch_data):
        self.fetch_data = fetch_data

    def login(self, _username, _password):
        return "OK", []

    def select(self, _folder, readonly=True):
        assert readonly
        return "OK", []

    def search(self, _charset, _criterion):
        return "OK", [b"1"]

    def fetch(self, _message_id, _query):
        return "OK", self.fetch_data

    def logout(self):
        return "BYE", []


class FetchShapeTests(unittest.TestCase):
    def test_valid_fetch_tuple_returns_message_bytes(self):
        data = [(b"1 (RFC822 {7})", b"message"), b")"]

        self.assertEqual(read_email.fetched_message_bytes(data), b"message")

    def test_malformed_fetch_responses_fail_with_clear_error(self):
        malformed_responses = [
            [],
            [None],
            [b"flags-only"],
            [(b"metadata-only",)],
            [(b"metadata", None)],
            [(b"metadata", "not bytes")],
        ]

        for fetch_data in malformed_responses:
            with self.subTest(fetch_data=fetch_data):
                mail = _FakeIMAP(fetch_data)
                stderr = io.StringIO()
                stdout = io.StringIO()
                environment = {"EMAIL_USERNAME": "user", "EMAIL_PASSWORD": "secret"}
                with (
                    patch.dict(os.environ, environment, clear=False),
                    patch.object(sys, "argv", ["read_email.py"]),
                    patch.object(read_email.imaplib, "IMAP4_SSL", return_value=mail),
                    patch.object(read_email.email, "message_from_bytes") as parse_message,
                    contextlib.redirect_stdout(stdout),
                    contextlib.redirect_stderr(stderr),
                ):
                    with self.assertRaises(SystemExit) as caught:
                        read_email.main()

                self.assertEqual(caught.exception.code, 1)
                self.assertIn("ERROR: fetch failed: malformed IMAP FETCH response", stderr.getvalue())
                parse_message.assert_not_called()


if __name__ == "__main__":
    unittest.main()
