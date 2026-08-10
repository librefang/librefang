#!/usr/bin/env python3
"""Regression tests for MIME charset-aware body extraction."""

import importlib.util
import unittest
from email import message_from_bytes
from email.message import EmailMessage
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("read_email", ROOT / "scripts" / "read_email.py")
read_email = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(read_email)


class GetBodyCharsetTests(unittest.TestCase):
    def test_multipart_plain_text_uses_declared_charset(self):
        message = EmailMessage()
        message.set_content("café", charset="iso-8859-1")
        message.add_alternative("<p>fallback</p>", subtype="html", charset="utf-8")

        self.assertEqual(read_email.get_body(message), "café\n")

    def test_multipart_html_fallback_uses_declared_charset(self):
        message = EmailMessage()
        message.make_mixed()
        html = EmailMessage()
        html.set_content("<p>你好</p>", subtype="html", charset="gb2312")
        message.attach(html)

        self.assertEqual(read_email.get_body(message), "你好")

    def test_non_multipart_body_uses_declared_charset(self):
        message = EmailMessage()
        message.set_content("olá", charset="iso-8859-1")

        self.assertEqual(read_email.get_body(message), "olá\n")

    def test_unknown_charset_label_falls_back_to_utf8(self):
        message = message_from_bytes(
            b'Content-Type: text/plain; charset="x-unknown"\n'
            b"Content-Transfer-Encoding: 8bit\n\n"
            b"caf\xc3\xa9"
        )

        self.assertEqual(read_email.get_body(message), "café")


if __name__ == "__main__":
    unittest.main()
