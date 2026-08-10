#!/usr/bin/env python3
"""Regression tests for complete RFC 2047 subject decoding."""

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("read_email", ROOT / "scripts" / "read_email.py")
read_email = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(read_email)


class DecodeSubjectTests(unittest.TestCase):
    def test_preserves_plain_segments_around_encoded_word(self):
        raw = "Re: =?utf-8?q?caf=C3=A9?= ready"

        self.assertEqual(read_email.decode_subject(raw), "Re: café ready")

    def test_decodes_all_adjacent_encoded_words(self):
        raw = "=?iso-8859-1?q?Ol=E1?= =?utf-8?b?5L2g5aW9?="

        self.assertEqual(read_email.decode_subject(raw), "Olá你好")

    def test_missing_subject_is_empty(self):
        self.assertEqual(read_email.decode_subject(None), "")

    def test_unknown_segment_charset_falls_back_to_utf8(self):
        raw = "prefix =?x-unknown?b?Y2Fmw6k=?= suffix"

        self.assertEqual(read_email.decode_subject(raw), "prefix café suffix")


if __name__ == "__main__":
    unittest.main()
