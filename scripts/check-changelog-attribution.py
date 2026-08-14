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

Every mode also holds `changelog.d/` fragments to the same standard. A fragment
is one file holding one bullet body, folded into `[Unreleased]` at release time
by `cargo xtask collect-fragments`, so an unattributed fragment is an
unattributed bullet with a delay on it. Two rules apply:

* The fragment's bullet block must carry `(@user)`. The block is evaluated with
  the same `bullet_block_has_attribution` predicate used for the CHANGELOG
  itself, so — exactly as in the file — the attribution may sit on any
  continuation line but may not sit past a blank line.
* The fragment must live in `changelog.d/<section>/<name>.md` with a recognised
  section. Assembly can only render a section it knows a heading for, so a typo
  such as `changelog.d/fix/` would be dropped without a word and the entry would
  vanish from the release notes — the same silent loss the fragment mechanism
  exists to prevent.

Attribution regex: `\\(@[A-Za-z0-9_][A-Za-z0-9_-]*\\)` (GitHub username
character set, at least one character — `(@)` alone is rejected).

Exit status: 0 on success, 1 if any in-scope bullet or fragment is bad,
2 on usage / git error.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path, PurePosixPath
from typing import NamedTuple, NoReturn

# GitHub usernames: 1-39 chars, [A-Za-z0-9-], cannot start with `-`. We don't
# enforce the upper bound here — the convention itself has no bound — but we
# do require at least one character and disallow a leading dash so that the
# common typo `(@-foo)` is rejected.
ATTRIBUTION_RE = re.compile(r"\(@[A-Za-z0-9_][A-Za-z0-9_-]*\)")
BULLET_RE = re.compile(r"^(\s*)-\s+\S")  # `- text` or `  - text` (nested)
# Lines ending with `# pragma: no-attribution` are explicitly exempted from the
# check. Use sparingly — only for entries added before the convention was
# enforced that cannot be retroactively attributed (e.g. author is unknown or
# the entry covers work from many people with no single owner).
NO_ATTRIBUTION_RE = re.compile(r"#\s*pragma:\s*no-attribution\s*$")
HEADER_RE = re.compile(r"^(#{1,6})\s+(.*)$")
UNRELEASED_RE = re.compile(r"^##\s+\[Unreleased\]\s*$")
RELEASE_HEADER_RE = re.compile(r"^##\s+\[[^\]]+\]")  # any `## [...]` line

CHANGELOG = "CHANGELOG.md"

# Per-PR changelog fragments. Mirrors `FRAGMENT_DIR` / `FRAGMENT_SECTIONS` in
# `xtask/src/changelog.rs`, which is the assembler: a section this set does not
# list is a section assembly has no heading for. The two lists are a
# cross-language contract — adding a section means editing both, and the xtask
# test `fragment_sections_match_the_python_validator` parses the literal below
# and fails if they drift. Keep it a single-line `frozenset({...})` so it can.
FRAGMENT_DIR = "changelog.d"
FRAGMENT_SECTIONS = frozenset({"added", "changed", "documentation", "fixed", "security"})
SECTION_CHOICES = ", ".join(sorted(FRAGMENT_SECTIONS))
# Files that live directly in `changelog.d/` and are not fragments.
FRAGMENT_ROOT_FILES = frozenset({"README.md"})

MISSING_ATTRIBUTION = "missing (@user) attribution"


class Violation(NamedTuple):
    """One in-scope problem, rendered as `path:lineno: reason: content`."""

    path: str
    lineno: int
    content: str
    reason: str = MISSING_ATTRIBUTION


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
    return ATTRIBUTION_RE.search(line) is not None or NO_ATTRIBUTION_RE.search(line) is not None


def bullet_block_has_attribution(lines: list[str], bullet_idx: int) -> bool:
    """Whether the bullet starting at `bullet_idx` (0-based) carries attribution
    on ANY of its lines.

    The repo's prose rule wraps a long bullet across multiple lines, one
    sentence per line, so the trailing `(@user)` often lands on the final
    continuation line rather than the `- ` marker line:

        - First sentence.
          Second sentence.
          Third sentence. (@houko)

    A bullet's block is its marker line plus the following continuation lines
    (indented, non-empty, not themselves a new bullet or a heading), ending at
    the first blank line, next bullet, or heading. Checking the whole block —
    rather than only the marker line — keeps the attribution rule compatible
    with the one-sentence-per-line wrapping the CHANGELOG mandates.
    """
    if has_attribution(lines[bullet_idx]):
        return True
    for j in range(bullet_idx + 1, len(lines)):
        nxt = lines[j]
        if nxt.strip() == "" or is_bullet(nxt) or HEADER_RE.match(nxt):
            break
        if has_attribution(nxt):
            return True
    return False


def report(violations: list[Violation], scope: str) -> int:
    if not violations:
        sys.stdout.write(f"OK: no changelog problems in scope '{scope}'.\n")
        return 0
    sys.stdout.write(
        f"FAIL: {len(violations)} problem(s) in scope '{scope}'. Every "
        f"[Unreleased] bullet and every {FRAGMENT_DIR}/ fragment must end with "
        f"`(@your-github-login)`, and a fragment must sit in a recognised "
        f"section directory ({SECTION_CHOICES}).\n"
    )
    for v in violations:
        # Format chosen so GitHub Actions / many editors render it as a
        # clickable link to the offending line.
        sys.stdout.write(f"{v.path}:{v.lineno}: {v.reason}: {v.content.rstrip()}\n")
    return 1


# ── changelog.d fragments ─────────────────────────────────────────────────


def classify_fragment_path(path: str) -> tuple[bool, str | None]:
    """Classify a repo-relative path under `changelog.d/`.

    Returns `(is_fragment, problem)`. `is_fragment` is False for the directory's
    own infrastructure — dotfiles such as `.gitkeep`, non-Markdown files, and
    `changelog.d/README.md` — which is never scanned. `problem` is the reason a
    path that *looks* like a fragment sits somewhere assembly will never read it.
    """
    parts = PurePosixPath(path).parts
    name = parts[-1]
    if name.startswith(".") or not name.endswith(".md"):
        return (False, None)
    if len(parts) == 2 and name in FRAGMENT_ROOT_FILES:
        return (False, None)
    if len(parts) != 3:
        return (
            True,
            f"fragment must live in {FRAGMENT_DIR}/<section>/<name>.md "
            f"(sections: {SECTION_CHOICES})",
        )
    if parts[1] not in FRAGMENT_SECTIONS:
        return (
            True,
            f"'{parts[1]}' is not a recognised {FRAGMENT_DIR} section "
            f"(expected one of: {SECTION_CHOICES}); assembly would silently "
            f"drop this entry",
        )
    return (True, None)


def check_fragment(path: str, text: str) -> list[Violation]:
    """Hold one fragment to the same standard as an `[Unreleased]` bullet.

    The fragment body IS a bullet body (just without the leading `- `), so its
    lines are handed straight to `bullet_block_has_attribution` — the same
    predicate, and therefore the same `(@user)` rule, the CHANGELOG itself is
    checked with. A blank line before the attribution ends the block and fails,
    which matches what assembly would produce: a bullet whose trailing
    `(@user)` had been orphaned into a separate paragraph.
    """
    is_fragment, problem = classify_fragment_path(path)
    if not is_fragment:
        return []
    violations: list[Violation] = []
    if problem is not None:
        violations.append(Violation(path, 1, PurePosixPath(path).name, problem))
    lines = text.splitlines()
    first = next((i for i, line in enumerate(lines) if line.strip()), None)
    if first is None:
        violations.append(Violation(path, 1, "", "fragment is empty"))
    elif not bullet_block_has_attribution(lines, first):
        violations.append(Violation(path, first + 1, lines[first]))
    return violations


def scan_worktree_fragments(root: Path) -> list[Violation]:
    """Check every fragment present in the working tree.

    Absent or empty `changelog.d/` yields nothing, so this never fails a repo
    that has no fragments pending.
    """
    base = root / FRAGMENT_DIR
    if not base.is_dir():
        return []
    violations: list[Violation] = []
    for path in sorted(p for p in base.rglob("*") if p.is_file()):
        rel = path.relative_to(root).as_posix()
        violations.extend(check_fragment(rel, path.read_text(encoding="utf-8")))
    return violations


def changed_fragment_paths(diff_args: list[str], root: Path) -> list[str]:
    """Repo-relative `changelog.d/` paths a diff adds, modifies, or renames.

    Deletions are excluded: `cargo xtask collect-fragments` removes the
    fragments it consumes, and that commit must not be failed for it.
    """
    out = run_git(
        ["diff", "--name-status", "--diff-filter=ACMR", *diff_args, "--", FRAGMENT_DIR],
        root,
    )
    paths: list[str] = []
    for line in out.splitlines():
        fields = line.split("\t")
        if len(fields) < 2:
            continue
        # A rename is `R100<TAB>old<TAB>new`; the post-image is always last.
        paths.append(fields[-1])
    return sorted(paths)


def scan_diff_fragments(base: str, head: str, root: Path) -> list[Violation]:
    """Check fragments this diff introduces, read from the post-image commit."""
    violations: list[Violation] = []
    for path in changed_fragment_paths([f"{base}..{head}"], root):
        is_fragment, _ = classify_fragment_path(path)
        if not is_fragment:
            continue
        violations.extend(check_fragment(path, run_git(["show", f"{head}:{path}"], root)))
    return violations


def scan_staged_fragments(root: Path) -> list[Violation]:
    """Check fragments this commit stages, read from the index."""
    violations: list[Violation] = []
    for path in changed_fragment_paths(["--cached"], root):
        is_fragment, _ = classify_fragment_path(path)
        if not is_fragment:
            continue
        violations.extend(check_fragment(path, run_git(["show", f":{path}"], root)))
    return violations


# ── Mode: default (diff) ──────────────────────────────────────────────────


def resolve_diff_range(args: argparse.Namespace) -> tuple[str, str]:
    """Resolve (base_ref, head_ref) for the diff scan.

    Precedence:
      1. CLI flags `--base` / `--head`
      2. env vars BASE_SHA / HEAD_SHA (set by CI)
      3. `git merge-base origin/main HEAD` and `HEAD`
    """
    def reject_incomplete_range() -> NoReturn:
        sys.stderr.write(
            "Diff range is incomplete. Pass both --base and --head, set both "
            "BASE_SHA and HEAD_SHA, or leave both unset for auto-detection.\n"
        )
        sys.exit(2)

    if bool(args.base) != bool(args.head):
        reject_incomplete_range()
    if args.base and args.head:
        return (args.base, args.head)

    base = os.environ.get("BASE_SHA")
    head = os.environ.get("HEAD_SHA")
    if bool(base) != bool(head):
        reject_incomplete_range()
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


def added_lines_in_unreleased(base: str, head: str, root: Path) -> list[Violation]:
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
    added: list[Violation] = []
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
                # Check the whole bullet block (marker + wrapped continuation
                # lines) in the post-image, so a `(@user)` on a continuation
                # line counts — the one-sentence-per-line prose rule pushes it
                # off the marker line for multi-sentence bullets.
                if is_bullet(content) and not bullet_block_has_attribution(
                    head_lines, lineno - 1
                ):
                    added.append(Violation(CHANGELOG, lineno, content))
            cur_new_lineno += 1
        elif raw.startswith("-"):
            # Removal: post-image line counter stays put.
            continue
        elif raw.startswith("\\ No newline at end of file"):
            # Diff metadata, not a post-image content line.
            continue
        else:
            # Context line under --unified=0 should not appear, but be safe.
            cur_new_lineno += 1

    return added


# ── Mode: --all-unreleased ────────────────────────────────────────────────


def scan_unreleased_section(root: Path) -> list[Violation]:
    path = root / CHANGELOG
    lines = path.read_text(encoding="utf-8").splitlines()
    rng = find_unreleased_range(lines)
    if rng is None:
        sys.stderr.write(
            "warning: no `## [Unreleased]` section found; nothing to scan.\n"
        )
        return []
    start, end = rng
    violations: list[Violation] = []
    for i in range(start + 1, end):
        line = lines[i]
        if is_bullet(line) and not bullet_block_has_attribution(lines, i):
            violations.append(Violation(CHANGELOG, i + 1, line))  # 1-based
    return violations


# ── Mode: --full ──────────────────────────────────────────────────────────


def scan_full_file(root: Path) -> list[Violation]:
    path = root / CHANGELOG
    lines = path.read_text(encoding="utf-8").splitlines()
    violations: list[Violation] = []
    in_fenced_block = False
    for i, line in enumerate(lines, start=1):
        if line.startswith("```"):
            in_fenced_block = not in_fenced_block
            continue
        if in_fenced_block:
            continue
        if is_bullet(line) and not bullet_block_has_attribution(lines, i - 1):
            violations.append(Violation(CHANGELOG, i, line))
    return violations


# ── Mode: --staged (pre-commit hook) ──────────────────────────────────────


def scan_staged_added_lines(root: Path) -> list[Violation]:
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
    added: list[Violation] = []
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
                # See added_lines_in_unreleased: attribution may sit on a
                # wrapped continuation line, so check the whole bullet block.
                if is_bullet(content) and not bullet_block_has_attribution(
                    staged_lines, lineno - 1
                ):
                    added.append(Violation(CHANGELOG, lineno, content))
            cur_new_lineno += 1
        elif raw.startswith("\\ No newline at end of file"):
            continue

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

    # Fragments are scanned in every mode: a fragment is an `[Unreleased]`
    # bullet with a delay on it, so letting one through unattributed just moves
    # the failure to release day.
    if args.full:
        return report(
            scan_full_file(root) + scan_worktree_fragments(root),
            scope=f"entire CHANGELOG.md + {FRAGMENT_DIR}/",
        )
    if args.all_unreleased:
        return report(
            scan_unreleased_section(root) + scan_worktree_fragments(root),
            scope=f"[Unreleased] section (all bullets) + {FRAGMENT_DIR}/",
        )
    if args.staged:
        return report(
            scan_staged_added_lines(root) + scan_staged_fragments(root),
            scope=f"staged additions to [Unreleased] and {FRAGMENT_DIR}/",
        )

    # Default: diff mode.
    base, head = resolve_diff_range(args)
    short_head = head[:8] if len(head) >= 8 else head
    return report(
        added_lines_in_unreleased(base, head, root)
        + scan_diff_fragments(base, head, root),
        scope=(
            f"new bullets in [Unreleased] and new {FRAGMENT_DIR}/ fragments "
            f"(diff {base[:8]}..{short_head})"
        ),
    )


if __name__ == "__main__":
    sys.exit(main())
