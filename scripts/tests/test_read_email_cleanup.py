#!/usr/bin/env python3
"""Regression tests for guaranteed IMAP session cleanup."""

import importlib.util
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
    def __init__(self):
        self.logout_calls = 0

    def login(self, _username, _password):
        return "OK", []

    def select(self, _folder, readonly=True):
        assert readonly
        return "OK", []

    def search(self, _charset, _criterion):
        return "OK", [b"1"]

    def fetch(self, _message_id, _query):
        return "OK", [(b"1 (RFC822)", b"Subject: hello\n\nbody")]

    def logout(self):
        self.logout_calls += 1
        return "BYE", []


class _LoginFailureIMAP(_FakeIMAP):
    def login(self, _username, _password):
        raise OSError("login connection failed")


class ImapCleanupTests(unittest.TestCase):
    def test_unexpected_post_login_error_still_logs_out(self):
        mail = _FakeIMAP()
        environment = {"EMAIL_USERNAME": "user", "EMAIL_PASSWORD": "secret"}

        with (
            patch.dict(os.environ, environment, clear=False),
            patch.object(sys, "argv", ["read_email.py"]),
            patch.object(read_email.imaplib, "IMAP4_SSL", return_value=mail),
            patch.object(read_email, "decode_header", side_effect=RuntimeError("decode failed")),
        ):
            with self.assertRaisesRegex(RuntimeError, "decode failed"):
                read_email.main()

        self.assertEqual(mail.logout_calls, 1)

    def test_login_failure_closes_constructed_connection(self):
        mail = _LoginFailureIMAP()
        environment = {"EMAIL_USERNAME": "user", "EMAIL_PASSWORD": "secret"}

        with (
            patch.dict(os.environ, environment, clear=False),
            patch.object(sys, "argv", ["read_email.py"]),
            patch.object(read_email.imaplib, "IMAP4_SSL", return_value=mail),
        ):
            with self.assertRaises(SystemExit) as caught:
                read_email.main()

        self.assertEqual(caught.exception.code, 1)
        self.assertEqual(mail.logout_calls, 1)


if __name__ == "__main__":
    unittest.main()
