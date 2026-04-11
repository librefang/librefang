#!/usr/bin/env python3
"""Sync Claude skills into Codex at both global and repo scope.

Global scope:
- mirrors `~/.claude/skills/*` into `~/.codex/skills/*` as relative symlinks

Repo scope:
- mirrors `.claude/skills/*` into `.agents/skills/*` as relative symlinks
- regenerates the explicit Codex `[[skills.config]]` block in `.agents/config.toml`

Existing real directories win over generated symlinks. Only generated symlinks
that point to the expected source tree are managed or removed.
"""

from __future__ import annotations

import os
from pathlib import Path
import sys

START_MARKER = "# BEGIN AUTO-GENERATED SHARED SKILLS"
END_MARKER = "# END AUTO-GENERATED SHARED SKILLS"
GLOBAL_EXCLUDE = {".system"}


def iter_skill_entries(path: Path) -> list[Path]:
    if not path.exists():
        return []
    return sorted(
        entry for entry in path.iterdir() if entry.is_dir() or entry.is_symlink()
    )


def sync_skill_dirs(
    source_dir: Path,
    target_dir: Path,
    exclude: set[str] | None = None,
) -> tuple[list[str], list[str]]:
    created: list[str] = []
    removed: list[str] = []
    exclude = exclude or set()
    target_dir.mkdir(parents=True, exist_ok=True)

    source_skills = {
        entry.name: entry
        for entry in iter_skill_entries(source_dir)
        if entry.name not in exclude
    }

    for name, source_path in source_skills.items():
        target_path = target_dir / name
        if target_path.exists() or target_path.is_symlink():
            continue
        rel = os.path.relpath(source_path, start=target_path.parent)
        target_path.symlink_to(rel, target_is_directory=True)
        created.append(name)

    for target_path in iter_skill_entries(target_dir):
        if target_path.name in exclude or not target_path.is_symlink():
            continue
        source_path = source_dir / target_path.name
        resolved = target_path.resolve(strict=False)
        if resolved == source_path and not source_path.exists():
            target_path.unlink()
            removed.append(target_path.name)

    return created, removed


def render_repo_skills_config_block(target_dir: Path) -> str:
    blocks: list[str] = []
    for entry in iter_skill_entries(target_dir):
        blocks.append(
            "\n".join(
                [
                    "[[skills.config]]",
                    f'path = ".agents/skills/{entry.name}"',
                    "enabled = true",
                ]
            )
        )
    rendered = "\n\n".join(blocks)
    return f"{START_MARKER}\n{rendered}\n{END_MARKER}"


def update_agents_config(config_path: Path, target_dir: Path) -> None:
    text = config_path.read_text()
    start = text.find(START_MARKER)
    end = text.find(END_MARKER)
    if start == -1 or end == -1 or end < start:
        raise RuntimeError(
            f"Could not find managed skill markers in {config_path}. "
            f"Expected {START_MARKER!r} and {END_MARKER!r}."
        )
    end += len(END_MARKER)
    replacement = render_repo_skills_config_block(target_dir)
    config_path.write_text(text[:start] + replacement + text[end:])


def print_changes(label: str, created: list[str], removed: list[str]) -> None:
    print(f"{label}: added {len(created)}, removed {len(removed)}")
    if created:
        for name in created:
            print(f"  + {name}")
    if removed:
        for name in removed:
            print(f"  - {name}")


def main() -> int:
    repo_root = Path(__file__).resolve().parent.parent

    home_claude = Path.home() / ".claude" / "skills"
    home_codex = Path.home() / ".codex" / "skills"
    repo_claude = repo_root / ".claude" / "skills"
    repo_codex = repo_root / ".agents" / "skills"
    repo_codex_config = repo_root / ".agents" / "config.toml"

    if home_claude.exists():
        created, removed = sync_skill_dirs(home_claude, home_codex, exclude=GLOBAL_EXCLUDE)
        print_changes("Global sync", created, removed)
    else:
        print(f"Global sync skipped: source not found at {home_claude}")

    if not repo_claude.exists():
        print(f"Repo sync skipped: source not found at {repo_claude}", file=sys.stderr)
        return 1
    if not repo_codex_config.exists():
        print(f"Repo sync skipped: config not found at {repo_codex_config}", file=sys.stderr)
        return 1

    created, removed = sync_skill_dirs(repo_claude, repo_codex)
    update_agents_config(repo_codex_config, repo_codex)
    print_changes("Repo sync", created, removed)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
