# Gremlin Audit 2026-04-09

Scope: tenant-context propagation, tenant isolation for async/background execution, and tenant-facing raw internal error leakage.

Status:
- `fixed`
- `in_progress`
- `open`
- `review`

## Summary

The remaining gremlins are now narrower:

1. Tenant-facing routes and bridge commands still return raw backend/internal error strings in scattered places.
2. Explicit legacy/global CLI or bridge paths still need clear verification and labeling, but they are now constrained to unowned/global objects instead of silently touching tenant-owned state.
3. The newest bridge hardening is now backed by targeted `librefang-api` test execution, but broader API verification still competes with a dirty worktree and long shared test-binary link times.

The core event path is materially improved after:
- `Event.account_id`
- `publish_event_scoped`
- `publish_event_for_agent`
- scoped trigger filtering

The main remaining execution-layer risk is no longer broad tenant drift. It is residual raw-error cleanup plus keeping explicit global/manual paths boxed into unowned/global behavior only.

## Scope Boundaries

This audit is intentionally limited to:

- tenant-context propagation
- tenant isolation for async/background execution
- tenant-facing raw internal error leakage

This audit does not track the separate QR/session ownership workstream. That work is covered by:

- [ADR-MT-006-CHANNEL-BOOTSTRAP-OWNERSHIP.md](./ADR-MT-006-CHANNEL-BOOTSTRAP-OWNERSHIP.md)
- [SPEC-MT-005-CHANNEL-BOOTSTRAP-SESSION-OWNERSHIP.md](./SPEC-MT-005-CHANNEL-BOOTSTRAP-SESSION-OWNERSHIP.md)
- [PLAN-MT-003-CHANNEL-BOOTSTRAP-HARDENING.md](./PLAN-MT-003-CHANNEL-BOOTSTRAP-HARDENING.md)

## Dirty Worktree Caveat

The current branch still has broad in-progress modifications outside this gremlin class. This document is an audit of the execution/sanitization issues above, not a claim that the full worktree is already packaged as one clean reconciliation slice.

## Source Of Truth Vs Noise

Likely in scope for this gremlin audit:

- `crates/librefang-api/src/channel_bridge.rs`
- `crates/librefang-api/src/routes/workflows.rs`
- `crates/librefang-api/src/routes/system.rs`
- `crates/librefang-api/src/routes/skills.rs`
- `crates/librefang-api/src/routes/agents.rs`
- `crates/librefang-api/src/routes/network.rs`
- `crates/librefang-kernel/src/triggers.rs`
- `crates/librefang-runtime/src/tool_runner.rs`
- `crates/librefang-types/src/event.rs`

Tracked separately as a different workstream:

- `crates/librefang-api/src/channel_bootstrap.rs`
- `crates/librefang-api/src/routes/channels.rs`
- `packages/whatsapp-gateway/index.js`
- `packages/whatsapp-gateway/index.test.js`

## Exact Scope Reviewed

Files and surfaces reviewed in this audit pass:

- `crates/librefang-kernel/src/triggers.rs`
- `crates/librefang-kernel/src/kernel.rs`
- `crates/librefang-runtime/src/kernel_handle.rs`
- `crates/librefang-runtime/src/tool_runner.rs`
- `crates/librefang-api/src/routes/system.rs`
- `crates/librefang-api/src/routes/workflows.rs`
- `crates/librefang-api/src/routes/agents.rs`
- `crates/librefang-api/src/routes/network.rs`
- `crates/librefang-api/src/routes/skills.rs`
- `crates/librefang-api/src/channel_bridge.rs`
- `crates/librefang-channels/src/bridge.rs`
- `crates/librefang-types/src/event.rs`

Async/background paths explicitly checked:

- trigger evaluation and event dispatch
- runtime tool `event_publish`
- manual schedule run
- cron background `SystemEvent`
- webhook wake
- webhook agent
- bridge/manual schedule execution

Not fully exhausted in this audit pass:

- every remaining `tokio::spawn` site outside the paths above
- all channel adapters beyond the bridge/helper layer
- all tenant-facing raw-error clusters in `agents.rs`, `skills.rs`, and `system.rs`

## Policy Invariants

Execution-layer tenant invariants after this hardening:

- tenant-owned triggers must only fire for events with the same `account_id`
- global/unscoped triggers must only fire for global/unscoped events
- tenant A events must never wake tenant B triggers
- async producers must either publish an explicitly scoped event or intentionally publish a global one
- admin-only endpoints are privileged by endpoint class, not by resource ownership, unless the surface is intentionally tenant-scoped

Admin-only policy clarified in this audit:

- approvals and bindings remain daemon-global admin-only infrastructure
- webhook wake and webhook agent are admin-gated but intentionally execute within the validated admin account's tenant scope

## Evidence Matrix

| Surface | Intended Policy | Current Implementation | Test / Evidence | Status | Residual Risk |
|---|---|---|---|---|---|
| Trigger evaluation | tenant-to-tenant exact match, global-to-global only | symmetric match in `triggers.rs` | `test_unscoped_event_does_not_wake_tenant_trigger`, `test_unscoped_event_still_wakes_global_trigger`, `test_publish_event_only_triggers_same_tenant` | verified | broader trigger suite still useful |
| Tool-driven event publish | preserve caller agent tenant | `publish_event_for_agent` in runtime/kernel handle | `test_event_publish_passes_caller_agent_context` | verified | broader integration coverage still useful |
| Manual schedule `SystemEvent` | carry job tenant scope | `with_account_id(job.account_id.clone())` | `run_schedule_system_event_does_not_trigger_other_tenants` | verified | broader schedule suite still useful |
| Cron background `SystemEvent` | carry job tenant scope | kernel cron path stamps `job.account_id` | code inspection + adjacent tenant tests | review | dedicated cron background regression test still recommended |
| Webhook wake | admin-gated, tenant-scoped publish | `require_admin_owner` + `publish_event_scoped` | `test_webhook_wake_keeps_event_scoped_to_request_account` | verified | none beyond broader suite |
| Webhook agent | admin-gated, same-tenant target only | `require_admin_owner` + scoped lookup | positive/negative webhook-agent tests | in_progress | test run needed after latest fixture fix |
| Approvals admin view | endpoint-class admin-only, global visibility | reverted to `require_admin` + global registry lookup | `test_system_approvals_admin_sees_global_agent_names` | verified | broader admin suite still useful |
| Bindings admin view | endpoint-class admin-only, global visibility | reverted to `require_admin` + global registry lookup | route policy + code review | review | dedicated binding regression test recommended |

## Regressions Introduced During Hardening

Observed and corrected in this audit cycle:

- `require_admin_owner()` was temporarily applied to daemon-global approvals and bindings flows
- that changed admin-only endpoint behavior from endpoint-class privilege to owner-scoped privilege
- approvals were corrected and covered by regression test
- bindings were corrected in code and should receive a focused regression test

## Test Commands Run

Commands confirmed passing during this audit cycle:

```bash
cargo test -p librefang-kernel test_unscoped_event_does_not_wake_tenant_trigger -- --nocapture
cargo test -p librefang-kernel test_unscoped_event_still_wakes_global_trigger -- --nocapture
cargo test -p librefang-kernel test_publish_event_only_triggers_same_tenant -- --nocapture
cargo test -p librefang-api --test account_tests test_system_approvals_admin_sees_global_agent_names -- --nocapture
```

Commands still pending / should be rerun after current edits:

```bash
cargo test -p librefang-api test_webhook_agent_can_target_same_tenant_by_name -- --nocapture
cargo test -p librefang-api test_webhook_agent_cannot_target_other_tenant_by_name -- --nocapture
```

## Known Fragile Areas

- trigger evaluation semantics when new event producers are added
- bridge/manual schedule execution paths
- webhook agent tenant-owner semantics
- bindings/admin-only route classification drift
- tenant-facing raw-error sanitization clusters in `agents.rs`, `skills.rs`, and `system.rs`

## Residual Audit Gaps

Still not fully closed:

- dedicated regression coverage for global/admin bridge manual workflow execution
- dedicated regression coverage for bindings remaining daemon-global admin-only
- broader async producer audit outside the reviewed paths above
- full tenant-facing raw-error sweep in `system.rs`, `skills.rs`, and `agents.rs`

## Fix Matrix

| Priority | File | Handler / Path | Issue | Desired Behavior | Test | Status |
|---|---|---|---|---|---|---|
| P0 | `crates/librefang-api/src/routes/workflows.rs` | `run_schedule` | tenant-facing schedule run leaked backend workflow errors | generic route-safe error body | `run_schedule_sanitizes_internal_errors` | fixed |
| P0 | `crates/librefang-api/src/routes/workflows.rs` | `dry_run_workflow` | tenant-facing dry-run returned raw backend text | generic `Workflow not found` | `dry_run_workflow_sanitizes_internal_errors` | fixed |
| P0 | `crates/librefang-api/src/routes/system.rs` | `webhook_wake` | tenant-facing webhook publish leaked backend errors | generic publish failure | webhook wake sanitization test | fixed |
| P0 | `crates/librefang-api/src/routes/system.rs` | `webhook_agent` | tenant-facing webhook execution leaked backend errors and name lookup crossed tenants | generic execution failure, tenant-scoped lookup | webhook tenant tests | fixed |
| P0 | `crates/librefang-api/src/channel_bridge.rs` | scoped `/schedule run` `SystemEvent` | scoped bridge schedule could broadcast unscoped event | publish scoped event with `account_id` | `test_manage_schedule_text_scoped_system_event_stays_tenant_scoped` | fixed |
| P0 | `crates/librefang-api/src/channel_bridge.rs` | scoped `/schedule run` `AgentTurn` | tenant-facing bridge command leaked raw run error | generic `Failed to run job.` | `test_manage_schedule_text_scoped_agent_turn_sanitizes_internal_errors` | in_progress |
| P0 | `crates/librefang-api/src/channel_bridge.rs` | scoped `/schedule run` `Workflow` | tenant-facing bridge command leaked raw workflow errors | generic `Failed to run workflow.` / `Workflow not found.` | `test_manage_schedule_text_scoped_workflow_sanitizes_internal_errors` | in_progress |
| P0 | `crates/librefang-api/src/routes/skills.rs` | `hand_send_message` | tenant-facing hand message leaked backend error | generic `Message delivery failed` | `hand_send_message_sanitizes_internal_errors` | fixed |
| P0 | `crates/librefang-api/src/routes/skills.rs` | `hand_get_session` | tenant-facing hand session leaked backend load error | generic `Failed to load session` | `hand_get_session_error_response_is_generic` | in_progress |
| P1 | `crates/librefang-api/src/routes/agents.rs` | `send_message` | tenant-facing send path leaked backend delivery errors | generic translated delivery failure | focused route test | fixed |
| P1 | `crates/librefang-api/src/routes/agents.rs` | `inject_message` | tenant-facing inject path returned raw internal errors | generic `Message injection failed` / `Agent not found` | `inject_message_sanitizes_internal_errors` | fixed |
| P1 | `crates/librefang-api/src/routes/agents.rs` | `import_session` | tenant-facing import leaked raw serde parse details | generic `Invalid export format` | `import_session_sanitizes_invalid_export_errors` | fixed |
| P1 | `crates/librefang-api/src/routes/agents.rs` | `push_message` | tenant-facing push path leaked backend delivery errors | generic translated delivery failure | `push_message_sanitizes_backend_errors` | in_progress |
| P1 | `crates/librefang-api/src/routes/agents.rs` | `spawn_agent` | tenant-facing spawn path embedded raw backend text | generic tenant-safe error body | `spawn_agent_sanitizes_backend_errors` | in_progress |
| P1 | `crates/librefang-api/src/routes/agents.rs` | `bulk_create_agents` | tenant-facing bulk create leaked raw backend errors per item | generic per-item tenant-safe errors | `bulk_create_agents_sanitizes_backend_errors` | in_progress |
| P1 | `crates/librefang-api/src/routes/network.rs` | `comms_send` | tenant-facing comms send leaked backend delivery errors | generic `Message delivery failed` | `comms_send_sanitizes_internal_errors` | fixed |
| P1 | `crates/librefang-api/src/routes/network.rs` | `comms_task` | tenant-facing comms task leaked backend task-store errors | generic `Failed to post task` | `comms_task_sanitizes_internal_errors` | fixed |
| P1 | `crates/librefang-api/src/routes/skills.rs` | `get_hand` | tenant-facing missing-hand path echoed detail | generic `Hand not found` | `get_hand_sanitizes_missing_hand_errors` | in_progress |
| P1 | `crates/librefang-api/src/routes/skills.rs` | `activate_hand` | tenant-facing activation failures echoed detail | generic `Failed to activate hand` | `activate_hand_sanitizes_missing_hand_errors` | in_progress |
| P1 | `crates/librefang-api/src/routes/skills.rs` | `remove_integration` | user-visible not-found path leaked uninstall detail | generic `Integration not found` | `remove_integration_sanitizes_not_found_errors` | fixed |
| P1 | `crates/librefang-kernel/src/triggers.rs` + related | event dispatch + trigger evaluation | tenant A events could trigger tenant B triggers | scoped events fail closed on tenant mismatch | kernel tenant event tests | fixed |
| P1 | `crates/librefang-runtime/src/tool_runner.rs` + related | `event_publish` tool | tool-driven publish could drop caller tenant context | caller-scoped event publish helper | `test_event_publish_passes_caller_agent_context` | fixed |
| P1 | `crates/librefang-api/src/routes/workflows.rs` + `crates/librefang-kernel/src/kernel.rs` | manual schedule / cron `SystemEvent` | system events could publish without tenant scope | carry job `account_id` into event | workflow/kernel tenant tests | fixed |
| P1 | `crates/librefang-kernel/src/kernel.rs` | caller-bound `cron_create` / `cron_list` / `cron_cancel` | unowned caller agents still had legacy cron behavior | require tenant-owned caller and account-scoped cron access | `test_cron_create_rejects_unowned_caller_agent`, `test_cron_list_rejects_unowned_caller_agent`, `test_cron_cancel_stays_tenant_scoped` | fixed |
| P1 | `crates/librefang-api/src/channel_bridge.rs` | `None`-scoped `/schedule` commands | CLI/global path could still drift toward tenant-owned jobs | `None` scope sees unowned/global jobs only and refuses tenant-owned agents | `test_none_scoped_schedule_commands_ignore_tenant_owned_jobs`, `test_none_scoped_add_refuses_tenant_owned_agents` | fixed |
| P1 | `crates/librefang-api/src/channel_bridge.rs` | `None`-scoped `/trigger` commands | CLI/global path could still drift toward tenant-owned triggers | `None` scope sees unowned/global triggers only and refuses tenant-owned agents | `test_none_scoped_trigger_commands_ignore_tenant_owned_triggers`, `test_none_scoped_add_refuses_tenant_owned_agents` | fixed |
| P1 | `crates/librefang-api/src/channel_bridge.rs` | unscoped bridge helper fallback | direct `manage_schedule_text` / trigger helper still had all-tenant semantics if called directly | unscoped helpers delegate to hardened `*_scoped(None, ...)` paths | covered by same `None`-scope bridge tests | fixed |
| P2 | `crates/librefang-api/src/routes/system.rs` | multiple handlers | remaining raw `e.to_string()` cluster | classify tenant/admin, sanitize tenant-facing | audit-first | open |
| P2 | `crates/librefang-api/src/routes/skills.rs` | install/config/admin flows | broad raw error surface | classify tenant-facing vs admin-only | audit-first | open |
| P2 | `crates/librefang-api/src/routes/agents.rs` | multiple translator-backed responses | broad `t.t_args(... e.to_string())` cluster | remove raw backend strings from tenant-visible responses | audit-first | open |
| P2 | `crates/librefang-api/src/channel_bridge.rs` | scoped create/remove flows | bridge create/remove still echo raw errors | generic user-safe responses + logging | focused bridge tests | open |
| P2 | `crates/librefang-api/src/channel_bridge.rs` | explicit global/manual workflow run path | explicit global path still needs intentional policy labeling and full verification | document global-only intent or split more explicitly | execution review | review |

## Async / Execution Notes

Improved:
- scoped event envelope now carries `account_id`
- trigger evaluation filters on scoped tenant
- runtime tool-driven `event_publish` now preserves caller tenant
- manual schedule and cron `SystemEvent` paths now carry `job.account_id`
- webhook wake publishes scoped events
- webhook agent lookup is tenant-scoped

Remaining review:
- explicit global/manual bridge workflow execution still needs intentional labeling and full verification
- the reachable `None`-scoped schedule and trigger command paths are now limited to unowned/global objects only
- caller-bound runtime/kernel cron access now requires a tenant-owned caller

## Verification

Verification status:

- kernel/runtime tenant-context checks were the highest-signal commands for this audit
- broader branch verification is still affected by unrelated dirty-worktree state
- treat the commands below as targeted audit verification, not whole-branch health proof

Known high-signal commands:

```bash
CARGO_HOME=/tmp/librefang-cargo-home CARGO_TARGET_DIR=/tmp/librefang-target-kernel cargo test -p librefang-kernel test_publish_event_only_triggers_same_tenant -- --exact --nocapture
CARGO_HOME=/tmp/librefang-cargo-home CARGO_TARGET_DIR=/tmp/librefang-target-runtime cargo test -p librefang-runtime test_event_publish_passes_caller_agent_context -- --exact --nocapture
CARGO_HOME=/tmp/librefang-cargo-home CARGO_TARGET_DIR=/tmp/librefang-target-kernel cargo test -p librefang-kernel test_cron_create_rejects_unowned_caller_agent -- --exact --nocapture
CARGO_HOME=/tmp/librefang-cargo-home CARGO_TARGET_DIR=/tmp/librefang-target-kernel cargo test -p librefang-kernel test_cron_list_rejects_unowned_caller_agent -- --exact --nocapture
CARGO_HOME=/tmp/librefang-cargo-home CARGO_TARGET_DIR=/tmp/librefang-target-kernel cargo test -p librefang-kernel test_cron_cancel_stays_tenant_scoped -- --exact --nocapture
CARGO_HOME=/tmp/librefang-cargo-home CARGO_TARGET_DIR=/tmp/librefang-target-api cargo check -p librefang-api --tests --offline
```

Current note:
- targeted `librefang-api` bridge tests now pass for the hardened `None`-scoped global/manual bridge behavior
- broader API verification is still narrower than a full clean branch proof because this work is landing in a dirty worktree with unrelated concurrent changes

## Next Order

1. Finish `skills.rs` hand/session sanitization and verify compile.
2. Integrate `agents.rs` tenant-facing sanitization cluster and verify compile.
3. Finish `channel_bridge.rs` scoped create/remove/run sanitization.
4. Classify `system.rs` raw-error surfaces into tenant-facing vs admin-only.
5. Review the explicit global bridge manual execution path and decide whether it is intentional admin/global behavior.
