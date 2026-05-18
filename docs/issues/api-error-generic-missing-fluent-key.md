# [Critical] 41 endpoints return `"api-error-generic"` literal instead of the real error — Fluent key never defined

**Severity:** Critical
**Domain:** Error handling

## Location

41 call sites (`rg -c 'api-error-generic'` → `agents.rs:31`, `tools_sessions.rs:10`), including:

- `crates/librefang-api/src/routes/agents.rs:3066, 3105, 3156, 3209, 3319, 3399, 3465, 3585, 3631, 3675, 3764, 3843, 4032, 4336, 4479, 4839, 5266`
- `crates/librefang-api/src/routes/tools_sessions.rs` (10 sites)

Every site has the shape:

```rust
t.t_args("api-error-generic", &[("error", &e.to_string())])
```

## Problem

The Fluent key `api-error-generic` is **never defined**. `grep -rn 'api-error-generic\s*='` against `crates/librefang-types/locales/{en,ja,zh-CN,de,fr,es}/errors.ftl` returns 0 hits. The fallback at `crates/librefang-types/src/i18n.rs:163-164` returns `key.to_string()` on missing key, so the `{$error}` interpolation never runs.

**Result:** every one of those 41 HTTP 500 responses returns `{"error": "api-error-generic"}` to the client. The actual `e.to_string()` is silently dropped — not interpolated, not logged at most sites. Operators have no diagnostic information on a class of 5xxs that includes the bugs reported in "agent-not-found-returns-500", "rusqlite-errors-leak", and the `agent_clone` 500-on-`AgentAlreadyExists` case folded into the same item.

## Fix

**Stopgap (one line per locale):** add to every `errors.ftl`:

```fluent
api-error-generic = { $error }
```

**Long-term:** route all 500s through a typed helper that logs the full error chain server-side and emits a stable scrubbed message client-side. The `MemoryRouteError` pattern at `crates/librefang-api/src/routes/memory.rs:198-215` is the correct shape.

## Tests

- Reflection test: every `t_args` key used in route handlers exists in every `errors.ftl`.
- Integration: force an internal error (e.g. corrupt session DB) and assert the response body contains the actual error context, not the literal `"api-error-generic"`.
