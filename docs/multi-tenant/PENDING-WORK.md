# PENDING-WORK: Multi-Tenant Phase 1 Follow-Up

**Date:** 2026-04-07  
**Scope:** Remaining work after Phase 1 Rounds 1-5  
**Baseline:** Rounds 1-5 landed for types, extractors, shared guards, `agents.rs`, `config.rs`, registry, persistence, and channel-bridge spawn wiring.

## Done

- [x] Round 1: `AccountId` type, `AgentEntry.account_id`, 19 unit tests
- [x] Round 2: CLI `AgentEntry` constructor fixes in `crates/librefang-cli/src/tui/event.rs`
- [x] Round 3: API account extractor, HMAC-SHA256 verification, `check_account`, shared route helpers, middleware/shared tests
- [x] Round 4a: `crates/librefang-api/src/routes/agents.rs` scoped (`50` handlers now take `account: AccountId`)
- [x] Round 4c: `crates/librefang-api/src/routes/config.rs` scoped (`15` handlers)
- [x] Registry: `list_by_account()`, `get_scoped()`, `set_account_id()` with TDD coverage
- [x] Persistence: `account_id` persisted in save/load paths with round-trip tests
- [x] Channel bridge: `spawn_agent_by_name` now takes `account_id`
- [x] Codex review fixes: atomic spawn ownership, `list_by_account` wired into handler, infallible `finalize_spawned_agent`

## Pending Checklist

### 1. CRITICAL: Gate multi-tenant mode on config, not HMAC-secret presence

- [ ] Make account enforcement depend on `multi_tenant.enabled`, not only `account_sig_secret`
- [ ] Reject missing `X-Account-Id` in multi-tenant mode so requests cannot fall through as `AccountId(None)`
- [ ] Keep HMAC verification as authentication of `X-Account-Id`, not as the feature flag for tenant isolation
- [ ] Update tests to assert the secure behavior

Files to change:
- `crates/librefang-api/src/server.rs`
- `crates/librefang-api/src/middleware.rs`
- `crates/librefang-types/src/config/types.rs`
- `crates/librefang-kernel/src/config.rs`
- `crates/librefang-api/tests/account_tests.rs` (new)

Notes:
- Current behavior in [`server.rs`](/Users/danielalberttis/Desktop/Projects/librefang/crates/librefang-api/src/server.rs) enables `require_account_id` only when `account_sig_secret` is configured.
- Current `check_account()` semantics in [`shared.rs`](/Users/danielalberttis/Desktop/Projects/librefang/crates/librefang-api/src/routes/shared.rs) still allow `AccountId(None)` to see everything, so missing-header requests are a hard isolation break when multi-tenant mode is intended.

### 2. CRITICAL: Scope the remaining route modules

- [ ] Scope `budget.rs`
- [ ] Scope `memory.rs`
- [ ] Scope `system.rs`
- [ ] Scope `workflows.rs`
- [ ] Scope `prompts.rs`
- [ ] Ensure both read and mutation paths are tenant-filtered
- [ ] Use `account: AccountId` consistently on handlers and tenant-aware kernel/query calls underneath

Files to change:
- `crates/librefang-api/src/routes/budget.rs`
- `crates/librefang-api/src/routes/memory.rs`
- `crates/librefang-api/src/routes/system.rs`
- `crates/librefang-api/src/routes/workflows.rs`
- `crates/librefang-api/src/routes/prompts.rs`
- `crates/librefang-api/src/routes/shared.rs`
- Any downstream kernel/runtime entry points reached by those handlers

Notes:
- This is the largest remaining correctness gap. `/agents` and `/config` are scoped, but cross-tenant access is still possible through these unscoped modules.

### 3. HIGH: Remove `/api/uploads/*` account-enforcement bypass

- [ ] Stop exempting `/api/uploads/*` from `require_account_id`
- [ ] Add tenant ownership metadata for uploaded files, or make upload serving agent/account scoped
- [ ] Verify generated media uploads and agent uploads cannot be fetched cross-tenant

Files to change:
- `crates/librefang-api/src/middleware.rs`
- `crates/librefang-api/src/routes/agents.rs`
- `crates/librefang-api/src/routes/media.rs`
- `crates/librefang-api/tests/account_tests.rs` (new)

Notes:
- Auth may already be required, but account enforcement is still bypassed for uploads. That is not sufficient in a multi-tenant deployment.

### 4. HIGH: Replace tests that currently bless the insecure bypass

- [ ] Rewrite middleware tests so missing `X-Account-Id` fails when multi-tenant mode is enabled
- [ ] Add regression tests proving no-header requests do not gain global visibility
- [ ] Remove or invert assertions that encode legacy full-access behavior under multi-tenant mode

Files to change:
- `crates/librefang-api/src/middleware.rs`
- `crates/librefang-api/src/routes/shared.rs`
- `crates/librefang-api/tests/account_tests.rs` (new)

Notes:
- Keep legacy behavior only for explicit single-tenant mode. Do not let tests normalize the insecure path in tenant mode.

### 5. HIGH: Round 4b is still open for channels

- [ ] Scope all `11` handlers in `channels.rs`
- [ ] Ensure channel reads, mutations, QR/session flows, and reload paths do not cross tenant boundaries
- [ ] Confirm channel-driven agent spawning and channel metadata reads stay within account scope

Files to change:
- `crates/librefang-api/src/routes/channels.rs`
- `crates/librefang-api/src/channel_bridge.rs`
- `crates/librefang-api/src/routes/shared.rs`
- `crates/librefang-api/tests/account_tests.rs` (new)

### 6. MEDIUM: Add replay protection to HMAC

- [ ] Bind signatures to more than raw `account_id`
- [ ] Add timestamp and nonce, or timestamp plus method/path binding
- [ ] Reject stale and replayed signatures
- [ ] Document the header contract and update examples/tests

Files to change:
- `crates/librefang-api/src/middleware.rs`
- `crates/librefang-types/src/config/types.rs`
- `docs/multi-tenant/ADR-MT-002-API-AUTH.md`
- `crates/librefang-api/tests/account_tests.rs` (new)

Suggested minimum:
- `X-Account-Id`
- `X-Account-Timestamp`
- `X-Account-Sig`
- Signature input should include at least account ID, request path, method, and timestamp

### 7. MEDIUM: Restrict global telemetry endpoints for scoped tenants

- [ ] Review `/api/status`
- [ ] Review `/api/health/detail`
- [ ] Decide whether scoped tenants get redacted data, tenant-local data, or denial
- [ ] Add tests so scoped requests cannot read cluster-global telemetry

Files to change:
- `crates/librefang-api/src/routes/config.rs`
- `crates/librefang-api/tests/account_tests.rs` (new)

### 8. LOW: Replace `get() + check_account()` with `get_scoped()`

- [ ] Switch individual GET-style handlers to `registry.get_scoped()` where possible
- [ ] Keep `check_account()` only for paths where a scoped lookup is not the right abstraction
- [ ] Reduce duplicated ownership checks in route code

Files to change:
- `crates/librefang-api/src/routes/agents.rs`
- `crates/librefang-api/src/routes/config.rs`
- Remaining route modules as they are scoped
- `crates/librefang-kernel/src/registry.rs` if helper APIs need minor follow-up

Notes:
- This is mostly cleanup and defense-in-depth. The higher-priority issue is to finish scoping all unscoped modules first.

### 9. HIGH: Create the missing integration test file for Round 5

- [ ] Create `crates/librefang-api/tests/account_tests.rs`
- [ ] Cover secure multi-tenant mode end-to-end, not just unit/middleware helpers
- [ ] Include cross-tenant read/write denial tests across agents, config, uploads, channels, and remaining scoped modules as they land

Files to change:
- `crates/librefang-api/tests/account_tests.rs`
- Potential shared test helpers under `crates/librefang-testing/` if needed

## Recommended Priority Order

- [ ] P0: Fix feature gating so multi-tenant mode is driven by config and missing `X-Account-Id` cannot degrade to `AccountId(None)`
- [ ] P0: Scope the unprotected route modules: `budget`, `memory`, `system`, `workflows`, `prompts`
- [ ] P1: Scope `channels.rs`
- [ ] P1: Close the `/api/uploads/*` leak
- [ ] P1: Replace insecure test expectations and add the missing integration suite
- [ ] P2: Restrict global telemetry endpoints
- [ ] P2: Add HMAC replay protection
- [ ] P3: Refactor remaining handlers to use `get_scoped()`

## Verification

Run after each logical chunk:

```bash
cargo fmt --all
cargo clippy -p librefang-api --all-targets -- -D warnings
cargo test -p librefang-api
```

Run before declaring Phase 1 complete:

```bash
cargo build --workspace --lib
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Suggested focused checks during implementation:

```bash
cargo test -p librefang-api account
cargo test -p librefang-api middleware
cargo test -p librefang-api routes::shared
```
