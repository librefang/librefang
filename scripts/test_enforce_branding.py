#!/usr/bin/env python3
"""
test_enforce_branding.py — Unit tests for the prose-mode logic added in M4
of the BossFang rebrand-completion roadmap.

Covers:
- Word-boundary regex (LibreFangKernel stays intact; bare LibreFang flips)
- Fenced code blocks (``` and ~~~) are skipped
- Inline code spans (`…`) are skipped
- TSX files have no fence/inline awareness — flipped unconditionally
- Multi-paragraph content with mixed prose / code / fences
- Idempotency (running twice changes nothing on the second pass)

Run from the repo root:
    python3 scripts/test_enforce_branding.py
"""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

# The script is named with a hyphen so it can't be imported by name. Load
# it directly from its file path.
_THIS_DIR = Path(__file__).resolve().parent
_SCRIPT_PATH = _THIS_DIR / "enforce-branding.py"
_SPEC = importlib.util.spec_from_file_location("enforce_branding", _SCRIPT_PATH)
assert _SPEC is not None and _SPEC.loader is not None
eb = importlib.util.module_from_spec(_SPEC)
sys.modules["enforce_branding"] = eb
_SPEC.loader.exec_module(eb)


class ProseReplacementTests(unittest.TestCase):
    def test_bare_libre_fang_flips_in_prose(self) -> None:
        self.assertEqual(
            eb.replace_prose_in_mdx("# LibreFang Docs\n"),
            "# BossFang Docs\n",
        )

    def test_layer_internal_struct_names_preserved(self) -> None:
        # Word-bounded \bLibreFang\b must not match LibreFangKernel etc.
        text = "Boots via LibreFangKernel; errors raise LibreFangError or LibreFangConfig.\n"
        self.assertEqual(eb.replace_prose_in_mdx(text), text)

    def test_inline_code_skipped(self) -> None:
        # The product-name reference inside backticks is preserved; the
        # prose reference outside is flipped.
        before = "The string `\"LibreFang Agent OS\"` shown by LibreFang.\n"
        after = "The string `\"LibreFang Agent OS\"` shown by BossFang.\n"
        self.assertEqual(eb.replace_prose_in_mdx(before), after)

    def test_fenced_code_block_skipped(self) -> None:
        # Triple-backtick fence with TOML — contents must not be touched.
        before = (
            "Configure LibreFang via TOML:\n"
            "\n"
            "```toml\n"
            'name = "LibreFang Agent OS"\n'
            "```\n"
            "\n"
            "LibreFang reads this on startup.\n"
        )
        after = (
            "Configure BossFang via TOML:\n"
            "\n"
            "```toml\n"
            'name = "LibreFang Agent OS"\n'  # untouched inside fence
            "```\n"
            "\n"
            "BossFang reads this on startup.\n"
        )
        self.assertEqual(eb.replace_prose_in_mdx(before), after)

    def test_tilde_fence_skipped(self) -> None:
        # ~~~ is also a valid Markdown fence marker.
        before = "Prose LibreFang.\n~~~\nfenced LibreFang\n~~~\nPost LibreFang.\n"
        after = "Prose BossFang.\n~~~\nfenced LibreFang\n~~~\nPost BossFang.\n"
        self.assertEqual(eb.replace_prose_in_mdx(before), after)

    def test_mixed_fence_markers_dont_close_each_other(self) -> None:
        # A ~~~ fence is not closed by a ``` line — and vice versa. This
        # protects against snippets that include the other marker.
        before = (
            "~~~\n"
            "LibreFang in tilde fence — ``` is not the close marker.\n"
            "~~~\n"
            "Prose LibreFang here.\n"
        )
        after = (
            "~~~\n"
            "LibreFang in tilde fence — ``` is not the close marker.\n"
            "~~~\n"
            "Prose BossFang here.\n"
        )
        self.assertEqual(eb.replace_prose_in_mdx(before), after)

    def test_indented_fence_recognised(self) -> None:
        # Fences inside list items are indented; the regex tolerates
        # leading whitespace.
        before = "- Item:\n   ```\n   LibreFang inside\n   ```\n   LibreFang outside\n"
        # The "outside" line is at the indent level — it's still prose,
        # so it flips. "inside" the fence stays.
        after = "- Item:\n   ```\n   LibreFang inside\n   ```\n   BossFang outside\n"
        self.assertEqual(eb.replace_prose_in_mdx(before), after)

    def test_idempotent(self) -> None:
        once = eb.replace_prose_in_mdx("LibreFang and `LibreFangKernel` examples.\n")
        twice = eb.replace_prose_in_mdx(once)
        self.assertEqual(once, twice)
        self.assertEqual(once, "BossFang and `LibreFangKernel` examples.\n")

    def test_tsx_unconditional_replacement(self) -> None:
        # No fence/inline-code awareness for TSX — straight regex replace.
        before = 'export const TITLE = "LibreFang Docs";\n'
        after = 'export const TITLE = "BossFang Docs";\n'
        self.assertEqual(eb.replace_prose_in_tsx(before), after)

    def test_tsx_preserves_struct_names(self) -> None:
        before = "import { LibreFangError } from '@librefang/sdk';\n"
        self.assertEqual(eb.replace_prose_in_tsx(before), before)


class AuditTests(unittest.TestCase):
    def test_audit_finds_prose_hit(self) -> None:
        self.assertEqual(
            eb.audit_prose_in_mdx("Header about LibreFang.\n"),
            ["LibreFang"],
        )

    def test_audit_skips_fenced_block(self) -> None:
        text = "```\nLibreFang in fence\n```\n"
        self.assertEqual(eb.audit_prose_in_mdx(text), [])

    def test_audit_skips_inline_code(self) -> None:
        text = "Inline `LibreFangKernel` only — no prose hit.\n"
        # \bLibreFang\b doesn't match inside LibreFangKernel anyway, but
        # also: inline code is skipped, so even bare LibreFang here would
        # not register.
        self.assertEqual(eb.audit_prose_in_mdx(text), [])

    def test_audit_inline_code_with_bare_libre_fang(self) -> None:
        # Bare LibreFang inside inline code is INTENTIONALLY not flagged —
        # those references document UI strings and are addressed in a
        # follow-up PR (or by changing the inline-code skipping rule
        # later).
        text = "The banner reads `LibreFang Agent OS`.\n"
        self.assertEqual(eb.audit_prose_in_mdx(text), [])


class FileLevelTests(unittest.TestCase):
    def test_enforce_color_file_does_not_touch_docs_prose(self) -> None:
        # Use a tempdir to confirm the color pass does NOT scan docs/.
        import tempfile

        with tempfile.TemporaryDirectory() as tmpdir:
            tmp = Path(tmpdir)
            mdx = tmp / "page.mdx"
            mdx.write_text("LibreFang docs\n", encoding="utf-8")

            # enforce_color_file only handles color tokens — confirms the
            # dispatch by extension wouldn't be invoked on .mdx as a color
            # file (the main loop scopes .mdx to PROSE_SCAN_DIRS).
            changed = eb.enforce_color_file(mdx)
            self.assertFalse(changed)
            self.assertEqual(mdx.read_text(encoding="utf-8"), "LibreFang docs\n")


if __name__ == "__main__":
    unittest.main()
