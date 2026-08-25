#!/usr/bin/env python3
"""Fail if any GitHub Actions job lacks an explicit ``timeout-minutes``.

A job without the key inherits GitHub's 360-minute default.
That is long enough for a wedged job to hold one of the repository's few concurrent
execution slots for six hours, which is exactly what happened when two ``main`` CI
runs hung ``in_progress`` and stalled more than eighty queued runs behind them.
On ``main`` the concurrency group is keyed per-sha with ``cancel-in-progress: false``
by design, so nothing supersedes a hung run and only the timeout can end it.

This guard exists because coverage is easy to regress: every new job silently
inherits the six-hour default unless its author remembers the key.

Jobs that only call a reusable workflow (``uses:`` at job level) are skipped, because
GitHub rejects ``timeout-minutes`` on those -- the cap belongs to the called workflow.

The same pass also rejects a duplicate key anywhere in a workflow file.
``yaml.safe_load`` silently keeps the last of two identical keys, so a file that YAML accepts can still be one GitHub Actions refuses to start -- and a refused workflow reports as a red run with no jobs, which reads like a test failure rather than a malformed file.
That is not hypothetical: resolving a merge where ``main`` and a branch had each added ``timeout-minutes`` to the same job by keeping *both* lines produced exactly that, passed every YAML check, and reached ``main``.
"""

from __future__ import annotations

import sys
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover - CI always has PyYAML
    print("check-workflow-timeouts: PyYAML is required (pip install pyyaml)", file=sys.stderr)
    raise SystemExit(2)

WORKFLOW_DIR = Path(__file__).resolve().parent.parent / ".github" / "workflows"


class DuplicateKey(Exception):
    """A mapping in a workflow file declares the same key twice."""


class _StrictLoader(yaml.SafeLoader):
    """``SafeLoader`` that refuses a duplicate mapping key instead of keeping the last one."""


def _no_duplicate_keys(loader: _StrictLoader, node, deep: bool = False) -> dict:
    mapping: dict = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in mapping:
            raise DuplicateKey(f"duplicate key {key!r} on line {key_node.start_mark.line + 1}")
        mapping[key] = loader.construct_object(value_node, deep=deep)
    return mapping


_StrictLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, _no_duplicate_keys
)


def main() -> int:
    if not WORKFLOW_DIR.is_dir():
        print(f"check-workflow-timeouts: no workflow directory at {WORKFLOW_DIR}", file=sys.stderr)
        return 2

    missing: list[tuple[str, str]] = []
    skipped = 0
    checked = 0

    for path in sorted(WORKFLOW_DIR.glob("*.y*ml")):
        try:
            doc = yaml.load(path.read_text(encoding="utf-8"), Loader=_StrictLoader)
        except DuplicateKey as exc:
            print(
                f"check-workflow-timeouts: {path.name} has a {exc}."
                " GitHub Actions refuses to start a workflow with a duplicate key, and"
                " reports it as a failed run with no jobs. Keep one of the two lines.",
                file=sys.stderr,
            )
            return 1
        except yaml.YAMLError as exc:
            print(f"check-workflow-timeouts: {path.name} is not valid YAML: {exc}", file=sys.stderr)
            return 2

        if not isinstance(doc, dict):
            continue
        jobs = doc.get("jobs")
        if not isinstance(jobs, dict):
            continue

        for job_id, job in jobs.items():
            if not isinstance(job, dict):
                continue
            # A job that delegates to a reusable workflow cannot carry the key.
            if "uses" in job:
                skipped += 1
                continue
            checked += 1
            if "timeout-minutes" not in job:
                missing.append((path.name, str(job_id)))

    if missing:
        print("check-workflow-timeouts: these jobs have no explicit timeout-minutes:", file=sys.stderr)
        for workflow, job_id in missing:
            print(f"  {workflow}: {job_id}", file=sys.stderr)
        print(
            "\nAdd `timeout-minutes: <n>` at job level, sized generously from the job's"
            " observed duration.\nA false timeout on a legitimate long build is worse than"
            " the hang this guards against, so prefer a high value over a tight one.",
            file=sys.stderr,
        )
        return 1

    print(f"OK: all {checked} workflow jobs declare timeout-minutes ({skipped} reusable-workflow jobs skipped).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
