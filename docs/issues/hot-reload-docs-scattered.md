# [Medium] Hot-reload semantics scattered across `config_reload.rs`, root CLAUDE.md, two `docs/architecture/*.md` files

**Severity:** Medium · **Domain:** Architecture · **Source:** `audit-06-architecture.md`

## Problem
No single ops-facing table answers "which config fields hot-reload, which require restart, which silently noop?" The per-agent concurrency cap requiring full respawn (per CLAUDE.md) is one example of many gotchas not in any one place.

## Fix
Generate the table from `build_reload_plan` (see "config-reload-coverage"). One canonical location: `docs/operations/config-reload.md`. Reference it from CLAUDE.md and the per-architecture docs.

## Tests
- After the reflection test in "config-reload-coverage" lands, the table is auto-derived; doc and code can never drift.
