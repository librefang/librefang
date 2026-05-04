#!/usr/bin/env python3
"""Validate that new CHANGELOG.md bullets carry a `(@username)` attribution.

The repo convention (1,800+ existing entries) is to suffix each bullet with
the GitHub login of the contributor in parentheses, e.g.

    - Add Polish language (pl) (#3937) (@leszek3737)

This script enforces that convention on **new** bullets being added to the
`[Unreleased]` section. It deliberately does NOT retroactively flag historical
entries — many predate any attribution convention and the project has decided
not to backfill (issue #3400). The validator therefore has three modes:

* default (`diff`):           scan only the lines this PR adds to the
                              `[Unreleased]` section. Used by CI.
* `--all-unreleased`:         scan every bullet currently inside the
                              `[Unreleased]` section. Useful for one-off
                              audits before cutting a release.
* `--full`:                   scan every bullet in the file. Reports every
                              historical violation. Pure inventory tool —
                              not wired into CI.

Attribution regex: `\\(@[A-Za-z0-9_][A-Za-z0-9_-]*\\)` (GitHub username
character set, at least one character — `(@)` alone is rejected).

Exit status: 0 on success, 1 if any in-scope bullet is missing attribution,
2 on usage / git error.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

# GitHub usernames: 1-39 chars, [A-Za-z0-9-], cannot start with `-`. We don't
# enforce the upper bound here — the convention itself has no bound — but we
# do require at least one character and disallow a leading dash so that the
# common typo `(@-foo)` is rejected.
ATTRIBUTION_RE = re.compile(r"\(@[A-Za-z0-9_][A-Za-z0-9_-]*\)")
BULLET_RE = re.compile(r"^(\s*)-\s+\S")  # `- text` or `  - text` (nested)
HEADER_RE = re.compile(r"^(#{1,6})\s+(.*)$")
UNRELEASED_RE = re.compile(r"^##\s+\[Unreleased\]\s*$")
RELEASE_HEADER_RE = re.compile(r"^##\s+\[[^\]]+\]")  # any `## [...]` line

CHANGELOG = "CHANGELOG.md"


def run_git(args: list[str], cwd: Path) -> str:
    """Run a git command, returning stdout. Aborts the script on non-zero."""
    proc = subprocess.run(
        ["git", *args],
        cwd=str(cwd),
        check=False,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        sys.stderr.write(
            f"git {' '.join(args)} failed (exit {proc.returncode}):\n{proc.stderr}"
        )
        sys.exit(2)
    return proc.stdout


def repo_root() -> Path:
    """Locate the repo root via `git rev-parse --show-toplevel`."""
    proc = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        check=False,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        sys.stderr.write("Not inside a git repository.\n")
        sys.exit(2)
    return Path(proc.stdout.strip())


def find_unreleased_range(lines: list[str]) -> tuple[int, int] | None:
    """Return (start_line_idx_inclusive, end_line_idx_exclusive) of the
    `## [Unreleased]` section, or None if absent.

    Indices are 0-based into `lines`. The start index points at the `##
    [Unreleased]` heading itself; end is the line index of the next `## [...]`
    heading (so iterating `lines[start:end]` covers the section content).
    """
    start: int | None = None
    for i, line in enumerate(lines):
        if UNRELEASED_RE.match(line):
            start = i
            break
    if start is None:
        return None
    end = len(lines)
    for j in range(start + 1, len(lines)):
        if RELEASE_HEADER_RE.match(lines[j]):
            end = j
            break
    return (start, end)


def is_bullet(line: str) -> bool:
    return BULLET_RE.match(line) is not None


def has_attribution(line: str) -> bool:
    return ATTRIBUTION_RE.search(line) is not None


def report(violations: list[tuple[int, str]], scope: str) -> int:
    if not violations:
        sys.stdout.write(f"OK: no missing attribution in scope '{scope}'.\n")
        return 0
    sys.stdout.write(
        f"FAIL: {len(violations)} bullet(s) in scope '{scope}' missing "
        f"`(@username)` attribution. Add `(@your-github-login)` at the end.\n"
    )
    for lineno, content in violations:
        # Format chosen so GitHub Actions / many editors render it as a
        # clickable link to the offending line.
        sys.stdout.write(
            f"{CHANGELOG}:{lineno}: missing (@user) attribution: {content.rstrip()}\n"
        )
    return 1


# ── Mode: default (diff) ──────────────────────────────────────────────────


def resolve_diff_range(args: argparse.Namespace) -> tuple[str, str]:
    """Resolve (base_ref, head_ref) for the diff scan.

    Precedence:
      1. CLI flags `--base` / `--head`
      2. env vars BASE_SHA / HEAD_SHA (set by CI)
      3. `git merge-base origin/main HEAD` and `HEAD`
    """
    base = args.base or os.environ.get("BASE_SHA")
    head = args.head or os.environ.get("HEAD_SHA")
    if base and head:
        return (base, head)
    # Fallback: derive from local refs.
    root = repo_root()
    try:
        merge_base = run_git(["merge-base", "origin/main", "HEAD"], root).strip()
    except SystemExit:
        sys.stderr.write(
            "Could not determine diff base. Pass --base/--head or set "
            "BASE_SHA/HEAD_SHA, or ensure `origin/main` is fetched.\n"
        )
        sys.exit(2)
    return (merge_base or "HEAD~1", "HEAD")


def added_lines_in_unreleased(
    base: str, head: str, root: Path
) -> list[tuple[int, str]]:
    """Return list of (post-image line number, line content) for every
    `+`-prefixed line the diff adds inside the `[Unreleased]` section.

    We compute the post-image line numbers from the unified-diff hunk
    headers so that error messages point at the line as it appears in the
    branch's CHANGELOG.md.
    """
    diff = run_git(
        [
            "diff",
            "--unified=0",
            "--no-color",
            f"{base}..{head}",
            "--",
            CHANGELOG,
        ],
        root,
    )
    if not diff.strip():
        return []

    # Read the post-image (HEAD) version of the file to compute the
    # `[Unreleased]` line range.
    head_blob = run_git(["show", f"{head}:{CHANGELOG}"], root)
    head_lines = head_blob.splitlines()
    rng = find_unreleased_range(head_lines)
    if rng is None:
        # No `[Unreleased]` section in the post-image — nothing to validate.
        return []
    unreleased_start, unreleased_end = rng

    hunk_re = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")
    added: list[tuple[int, str]] = []
    cur_new_lineno: int | None = None

    for raw in diff.splitlines():
        m = hunk_re.match(raw)
        if m:
            cur_new_lineno = int(m.group(1))
            continue
        if cur_new_lineno is None:
            continue
        if raw.startswith("+++") or raw.startswith("---"):
            continue
        if raw.startswith("+"):
            content = raw[1:]
            lineno = cur_new_lineno  # 1-based
            # Filter to bullets inside [Unreleased]. `lineno` is 1-based;
            # unreleased_start/unreleased_end are 0-based indices into
            # head_lines, so the inclusive range is
            # (unreleased_start+1) .. unreleased_end (exclusive of the next
            # release heading).
            if (unreleased_start + 1) <= lineno <= unreleased_end:
                if is_bullet(content) and not has_attribution(content):
                    added.append((lineno, content))
            cur_new_lineno += 1
        elif raw.startswith("-"):
            # Removal: post-image line counter stays put.
            continue
        else:
            # Context line under --unified=0 should not appear, but be safe.
            cur_new_lineno += 1

    return added


# ── Mode: --all-unreleased ────────────────────────────────────────────────


def scan_unreleased_section(root: Path) -> list[tuple[int, str]]:
    path = root / CHANGELOG
    lines = path.read_text(encoding="utf-8").splitlines()
    rng = find_unreleased_range(lines)
    if rng is None:
        sys.stderr.write(
            "warning: no `## [Unreleased]` section found; nothing to scan.\n"
        )
        return []
    start, end = rng
    violations: list[tuple[int, str]] = []
    for i in range(start + 1, end):
        line = lines[i]
        if is_bullet(line) and not has_attribution(line):
            violations.append((i + 1, line))  # 1-based line number
    return violations


# ── Mode: --full ──────────────────────────────────────────────────────────


def scan_full_file(root: Path) -> list[tuple[int, str]]:
    path = root / CHANGELOG
    lines = path.read_text(encoding="utf-8").splitlines()
    violations: list[tuple[int, str]] = []
    in_fenced_block = False
    for i, line in enumerate(lines, start=1):
        if line.startswith("```"):
            in_fenced_block = not in_fenced_block
            continue
        if in_fenced_block:
            continue
        if is_bullet(line) and not has_attribution(line):
            violations.append((i, line))
    return violations


# ── Mode: --staged (pre-commit hook) ──────────────────────────────────────


def scan_staged_added_lines(root: Path) -> list[tuple[int, str]]:
    """Diff the index against HEAD for CHANGELOG.md and return bullets the
    commit adds inside `[Unreleased]` that lack attribution. Used by the
    pre-commit hook so contributors hear about it before pushing.
    """
    diff = run_git(
        [
            "diff",
            "--cached",
            "--unified=0",
            "--no-color",
            "--",
            CHANGELOG,
        ],
        root,
    )
    if not diff.strip():
        return []

    # Post-image is the staged content. Read it via `git show :CHANGELOG.md`.
    staged_blob = run_git(["show", f":{CHANGELOG}"], root)
    staged_lines = staged_blob.splitlines()
    rng = find_unreleased_range(staged_lines)
    if rng is None:
        return []
    unreleased_start, unreleased_end = rng

    hunk_re = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")
    added: list[tuple[int, str]] = []
    cur_new_lineno: int | None = None
    for raw in diff.splitlines():
        m = hunk_re.match(raw)
        if m:
            cur_new_lineno = int(m.group(1))
            continue
        if cur_new_lineno is None:
            continue
        if raw.startswith("+++") or raw.startswith("---"):
            continue
        if raw.startswith("+"):
            content = raw[1:]
            lineno = cur_new_lineno
            if (unreleased_start + 1) <= lineno <= unreleased_end:
                if is_bullet(content) and not has_attribution(content):
                    added.append((lineno, content))
            cur_new_lineno += 1

    return added


# ── Entry point ───────────────────────────────────────────────────────────


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Enforce `(@username)` attribution on CHANGELOG.md bullets. "
            "Default mode validates only what the current PR adds to the "
            "[Unreleased] section."
        )
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--all-unreleased",
        action="store_true",
        help="Scan every bullet currently inside the [Unreleased] section.",
    )
    mode.add_argument(
        "--full",
        action="store_true",
        help="Scan every bullet in the file (inventory mode).",
    )
    mode.add_argument(
        "--staged",
        action="store_true",
        help="Scan staged additions to [Unreleased] (pre-commit hook mode).",
    )
    parser.add_argument(
        "--base",
        help="Diff base ref (default: $BASE_SHA or `git merge-base origin/main HEAD`).",
    )
    parser.add_argument(
        "--head",
        help="Diff head ref (default: $HEAD_SHA or HEAD).",
    )
    args = parser.parse_args()

    root = repo_root()
    if not (root / CHANGELOG).exists():
        sys.stderr.write(f"{CHANGELOG} not found at repo root.\n")
        return 2

    if args.full:
        return report(scan_full_file(root), scope="entire CHANGELOG.md")
    if args.all_unreleased:
        return report(
            scan_unreleased_section(root),
            scope="[Unreleased] section (all bullets)",
        )
    if args.staged:
        return report(
            scan_staged_added_lines(root),
            scope="staged additions to [Unreleased]",
        )

    # Default: diff mode.
    base, head = resolve_diff_range(args)
    return report(
        added_lines_in_unreleased(base, head, root),
        scope=f"new bullets in [Unreleased] (diff {base[:8]}..{head[:8] if len(head) >= 8 else head})",
    )


if __name__ == "__main__":
    sys.exit(main())
