# [High] Mock router test infrastructure overhaul — auth/rate-limit/idempotency entirely untested, and the mock silently hides unregistered routes

**Severity:** High · **Domain:** Test coverage
**Status:** Merges 1 earlier issue into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | 32/42 integration tests use `start_test_server`, skipping the auth, rate-limit, idempotency, and body-size layers | `tests/api_integration_test.rs:60-171` |
| unregistered route shadow | The `start_test_server` mock harness silently shadows missing route registrations (the same root cause behind the "agents-mutation-routes-untested" class of bugs) | `tests/api_integration_test.rs:60-171` |

## Why merged

Both are blind spots caused by the same mock harness — one is missing coverage, the other is missing alarms; one change addresses both.

## Combined fix plan

1. **New tests must use `start_full_router`** — add the rule to `tests/CLAUDE.md` and a custom lint to enforce it.
2. **Migrate the existing 32 mock tests**: most need a single-line builder swap; a few need an auth-token fixture.
3. **Rename the mock harness** to `start_handler_test_server` so its intent is clear; keep it only for handler-level unit tests that genuinely need isolation.
4. **Fail loud on unregistered routes**: the mock harness verifies that the registered handler set is a subset of routes wired into the app, and fails to start otherwise.

## Tests

- After migration, `rg 'start_test_server\b' tests/` returns only the few remaining handler-only tests.
- Wiring a handler without `app.route(...)` causes the mock harness to fail at startup.
- Regressions in the class of "api-error-generic-missing-fluent-key" (Fluent key) and "auth-callback-no-rate-limit" (rate-limit allowlist) can be caught once migration lands.
