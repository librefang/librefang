#!/usr/bin/env python3
"""Regression tests for safe IMAP SEARCH sender criteria."""

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


class _SearchRecorder:
    def __init__(self):
        self.search_calls = []

    def select(self, _folder, readonly=True):
        assert readonly
        return "OK", []

    def search(self, *args):
        self.search_calls.append(args)
        return "OK", [b""]


class SearchSenderTests(unittest.TestCase):
    def test_imap_line_controls_are_rejected(self):
        for value in ("before\rafter", "before\nafter", "before\x00after"):
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    read_email.quote_imap_search_value(value)

    def test_quote_and_backslash_are_escaped_in_quoted_string(self):
        mail = _SearchRecorder()

        result = read_email.search_folder(mail, "INBOX", 'alice" \\ example@example.com')

        self.assertEqual(result, [])
        self.assertEqual(
            mail.search_calls,
            [(None, 'FROM "alice\\" \\\\ example@example.com"')],
        )

    def test_line_break_is_rejected_before_imap_connection(self):
        environment = {"EMAIL_USERNAME": "user", "EMAIL_PASSWORD": "secret"}
        stderr = io.StringIO()

        with (
            patch.dict(os.environ, environment, clear=False),
            patch.object(sys, "argv", ["read_email.py", "trusted@example.com\r\nALL"]),
            patch.object(read_email.imaplib, "IMAP4_SSL") as connect,
            contextlib.redirect_stderr(stderr),
        ):
            with self.assertRaises(SystemExit) as caught:
                read_email.main()

        self.assertEqual(caught.exception.code, 1)
        self.assertIn("ERROR: invalid sender search value", stderr.getvalue())
        connect.assert_not_called()


if __name__ == "__main__":
    unittest.main()
