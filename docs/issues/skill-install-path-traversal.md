# [Critical] Path traversal in `POST /api/skills/install` via `name` / `hand`

**Severity:** Critical
**Domain:** API attack surface

## Location

- Handler: `crates/librefang-api/src/routes/skills.rs:374-439`
- Request type: `crates/librefang-api/src/types.rs:502-507`

## Problem

```rust
home.join("registry").join("skills").join(&req.name)
skills_dir.join(&req.name)
home.join("workspaces").join("hands").join(hand_id)
```

No rejection of `..`, `/`, or `\` on either field. The sibling `uninstall_skill` does have the rejection (`librefang-skills/src/evolution.rs:1277`), so the hardening exists in the codebase — it just isn't called here.

## Exploit

```json
POST /api/skills/install
{ "name": "../../../etc/cron.daily/payload" }
```

- `.exists()` probe leaks arbitrary FS paths (200 vs 404 oracle).
- `copy_dir_recursive` can write outside `~/.librefang/skills/` (full filesystem write under daemon UID).

## Fix

Extract `validate_name` into a shared helper (mirror `agent_templates.rs:113-124`), call on both `req.name` and `req.hand`. Reject anything containing `..`, `/`, `\`, leading `.`, or non-`[a-z0-9_-]` after Unicode normalization.

## Tests

- Integration: `{"name":"../etc/passwd"}` → 400 with a stable error code.
- Same for `hand` field.
- Negative: confirm legitimate names like `weather-v2` still succeed.
