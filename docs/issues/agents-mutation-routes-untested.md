# [Critical] ~30 `/api/agents/*` mutation routes have zero semantic test coverage

**Severity:** Critical
**Domain:** Test coverage

## Location

- Route declarations: `crates/librefang-api/src/routes/agents.rs:1-192` (46 routes)
- Mock router currently used: `crates/librefang-api/tests/api_integration_test.rs:60-171` (32 of 42 tests)
- Full router only used by 6 tests via `start_full_router` at `:173-215`

## Problem

The repo has 153 unique route paths declared and 387 `(path, method)` pairs. ~30 mutation routes on agents (bulk ops; lifecycle suspend/resume/mode; files endpoints with **path-traversal risk**; capabilities tools/skills/mcp_servers; clone/reload/push) have **only a registration check in `dead_route_audit_test.rs`** — no test asserts behavior:

- Does the route actually mutate state?
- Does it return correct status codes (4xx vs 5xx)?
- Are authz checks applied?
- Are inputs validated?

Compounding factors:
- 32 of 42 integration tests use `start_test_server` (mock router), which **bypasses auth, rate-limit, and idempotency layers**.
- Only **7 `INTERNAL_SERVER_ERROR` assertions** exist in the entire suite — 5xx error paths are essentially untested.
- The route → handler map for `agents.rs` is the largest in the codebase, making this the highest-leverage gap.

The CLAUDE.md mandate is "every new route gets an integration test against `TestServer`" — this gap predates the rule and was not retro-applied.

## Fix

1. **New code:** PRs touching `agents.rs` MUST add at least one `#[tokio::test]` round-trip (write → read → verify side effect) against `start_full_router` (not the mock).
2. **Backfill:** systematic backfill of the ~30 untested mutation routes. Recommend dispatching one per PR with a checklist tracked in a tracking issue (umbrella).
3. **Lint:** auto-derive the route → test coverage matrix from `dead_route_audit_test.rs` and emit a CI warning when an unmapped route appears.

## Highest-priority subroutes (path-traversal risk)

- `POST /api/agents/{id}/files/...` — workspace file write paths
- `POST /api/agents/{id}/clone` — fix for the `AgentAlreadyExists → 409` change (see "agent-not-found-returns-500") also requires test scaffold
- `POST /api/agents/{id}/capabilities/skills` — skill install reachable from here (overlaps with "skill-install-path-traversal" exploit chain)

## Tests

- One `#[tokio::test]` per route in `agents.rs` lifecycle/files/capabilities clusters.
- Add a coverage assertion alongside `dead_route_audit_test.rs` that fails CI when an annotated route has no entry in a `routes_under_test.rs` set.
