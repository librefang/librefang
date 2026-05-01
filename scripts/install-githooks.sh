#!/usr/bin/env bash
# One-shot installer: point this clone's git at the version-controlled hooks.
# After running this once, every future commit / push uses .githooks/* and
# inherits the rules in CLAUDE.md.
#
# Idempotent — running it again just re-confirms the setting.

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

git config core.hooksPath .githooks
chmod +x .githooks/* 2>/dev/null || true

echo "✓ core.hooksPath set to .githooks"
echo "  Active hooks:"
ls -1 .githooks | sed 's/^/    /'
