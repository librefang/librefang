# Gremlin Audit 2026-04-09

Scope: tenant-context propagation, tenant isolation for async/background execution, and tenant-facing raw internal error leakage.

Status:
- `fixed`
- `in_progress`
- `open`
- `review`

## Summary

The remaining gremlins are now narrower:

1. Explicit legacy/global CLI or bridge paths still need clear verification and labeling, but they are now constrained to unowned/global objects instead of silently touching tenant-owned state.
2. Runtime-generated uploads now carry sidecar ownership metadata, and the old disk-only `/api/uploads` fallback is removed so unregistered files now fail closed. This seam is now part of the converged tenant-owned upload story, not a half-migrated exception.
3. The main remaining work is now residual compatibility-seam classification and a few admin/global sanitization decisions, not broad tenant-context drift.

The core event path is materially improved after:
- `Event.account_id`
- `publish_event_scoped`
- `publish_event_for_agent`
- scoped trigger filtering

The main remaining execution-layer risk is no longer broad tenant drift. It is
now mostly explicit admin/global catalog classification follow-up plus future
re-audit of remaining optional-scope helper families.

The recurring gremlin class behind most of this audit is now explicit:

- any API that accepts tenant scope as `Option<&str>`
- and silently degrades to global behavior when passed `None`
- or silently drops tenant context when the caller forgets to thread scope

Under the convergence policy below, that pattern is no longer just a useful
smell. It is the standard sign that a surface is still living in a documented
compatibility seam or has not yet been migrated to the strict tenant-owned
model.

Current sharp-edged examples still worth explicit labeling or follow-up review:

- `crates/librefang-api/src/channel_bridge.rs`
  - `find_agent_entry_by_name_scoped(account_id: Option<&str>, ...)`
- these are acceptable today only because reachable tenant paths pass
  `Some(account_id)` and the `None` branch is reserved for explicit CLI/global
  compatibility

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
- final admin/global catalog classification notes in `skills.rs`
- every remaining dual-mode `Option<&str>` ownership helper outside the
  hardened bridge/event/cron/workflow path

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

## Convergence Policy

This audit now follows a stricter architectural direction: stop treating
tenant/account scope as an optional convention and make it a typed boundary.

Target model:

- one compatibility extractor for public, legacy, admin, and bootstrap surfaces
- one strict extractor for tenant-owned surfaces
- tenant-owned route families use only the strict extractor
- admin/global/public route families stay explicitly separate
- tenant-owned helper APIs accept concrete tenant/account types, not `Option`
- storage and compatibility shims may keep optional-account seams temporarily,
  but only at explicit boundaries

Route-family classification target:

- tenant-owned
- admin/global
- public/auth/bootstrap

Desired end state:

- tenant-owned handlers take a concrete tenant/account type
- tenant-owned helper functions stop accepting optional account state
- cross-tenant ownership checks do not depend on caller discipline
- `AccountId(None)` or `Option<&str>` remains only in documented compatibility
  seams
- legacy global or `system`-style fallback stays out of tenant-owned domain
  logic

Migration rule:

- migrate whole route families or cohesive modules, not endless one-off
  handlers
- if a file mixes tenant-owned and admin/global behavior, split policy first or
  document the file as a bounded compatibility island

Standard per-module verification target:

- missing tenant header -> `400`
- same-owner path works
- cross-tenant access denied
- admin/global behavior unchanged where applicable

Stop condition:

- optional-account usage remains only in documented compatibility zones
- tenant-owned modules no longer accept ambiguous account context
- remaining global/manual behavior is explicit, reviewed, and intentionally
  separated from tenant-owned flows

## Evidence Matrix

| Surface | Intended Policy | Current Implementation | Test / Evidence | Status | Residual Risk |
|---|---|---|---|---|---|
| Trigger evaluation | tenant-to-tenant exact match, global-to-global only | symmetric match in `triggers.rs` | `test_unscoped_event_does_not_wake_tenant_trigger`, `test_unscoped_event_still_wakes_global_trigger`, `test_publish_event_only_triggers_same_tenant` | verified | broader trigger suite still useful |
| Tool-driven event publish | preserve caller agent tenant and reject unowned caller fallback | `publish_event_for_agent` in runtime/kernel handle now preserves caller tenant and fails closed for unowned callers | `test_event_publish_passes_caller_agent_context`, `test_publish_event_for_agent_default_fails_closed`, `test_publish_event_for_agent_rejects_unowned_caller_agent` | verified | broader integration coverage still useful |
| Manual schedule `SystemEvent` | carry job tenant scope | `with_account_id(job.account_id.clone())` | `run_schedule_system_event_does_not_trigger_other_tenants` | verified | broader schedule suite still useful |
| Cron background `SystemEvent` | carry job tenant scope | kernel cron path stamps `job.account_id` and due-job execution now has focused regression coverage | `test_cron_background_system_event_stays_tenant_scoped` | verified | broader schedule suite still useful |
| Spawn lifecycle event producer | preserve spawned agent tenant on synchronous trigger evaluation | spawn path now stamps lifecycle event with `entry.account_id` before direct trigger evaluation | `test_spawn_agent_owned_lifecycle_event_stays_tenant_scoped` | verified | broader lifecycle-event suite still useful |
| Webhook wake | admin-gated, tenant-scoped publish | `require_admin_owner` + `publish_event_scoped` | `test_webhook_wake_keeps_event_scoped_to_request_account` | verified | none beyond broader suite |
| Approval / heartbeat event producers | preserve source agent tenant when events are tenant-owned | approval and heartbeat producers now route through `publish_event(...)` instead of raw bus publish | `test_submit_tool_approval_keeps_approval_events_tenant_scoped` | verified | heartbeat-specific regression test still useful |
| Webhook agent | admin-gated, same-tenant target only | `require_admin_owner` + scoped lookup | `test_webhook_agent_can_target_same_tenant_by_name`, `test_webhook_agent_cannot_target_other_tenant_by_name` | verified | broader webhook suite still useful |
| Approvals admin view | endpoint-class admin-only, global visibility | reverted to `require_admin` + global registry lookup | `test_system_approvals_admin_sees_global_agent_names` | verified | broader admin suite still useful |
| Bindings admin view | endpoint-class admin-only, global visibility | reverted to `require_admin` + global registry lookup | `test_system_bindings_admin_sees_global_bindings` | verified | broader admin suite still useful |

## Regressions Introduced During Hardening

Observed and corrected in this audit cycle:

- `require_admin_owner()` was temporarily applied to daemon-global approvals and bindings flows
- that changed admin-only endpoint behavior from endpoint-class privilege to owner-scoped privilege
- approvals were corrected and covered by regression test
- bindings were corrected in code and are now covered by a focused regression test

## Test Commands Run

Commands confirmed passing during this audit cycle:

```bash
cargo test -p librefang-kernel test_unscoped_event_does_not_wake_tenant_trigger -- --nocapture
cargo test -p librefang-kernel test_unscoped_event_still_wakes_global_trigger -- --nocapture
cargo test -p librefang-kernel test_publish_event_only_triggers_same_tenant -- --nocapture
cargo test -p librefang-kernel --lib test_spawn_agent_owned_lifecycle_event_stays_tenant_scoped -- --nocapture
cargo test -p librefang-api --test account_tests test_system_approvals_admin_sees_global_agent_names -- --nocapture
cargo test -p librefang-api --lib test_system_bindings_admin_sees_global_bindings -- --nocapture
cargo test -p librefang-api test_webhook_agent_can_target_same_tenant_by_name -- --nocapture
cargo test -p librefang-api test_webhook_agent_cannot_target_other_tenant_by_name -- --nocapture
```

## Known Fragile Areas

- trigger evaluation semantics when new event producers are added
- cross-surface event publishing where new producers can still choose between
  explicit tenant scope and implicit/global publish paths
- bridge/manual schedule execution paths
- webhook agent tenant-owner semantics
- bindings/admin-only route classification drift
- dual-mode `Option<&str>` scoped APIs that are correct today only because
  current callers are disciplined

## Residual Audit Gaps

Still not fully closed:

- dedicated regression coverage for global/admin bridge manual workflow execution
- dedicated regression coverage for bindings remaining daemon-global admin-only
- broader async producer audit outside the reviewed paths above
- explicit review of remaining `Option<&str>` scoped APIs that still allow
  global fallback by design

## Fix Matrix

| Priority | File | Handler / Path | Issue | Desired Behavior | Test | Status |
|---|---|---|---|---|---|---|
| P0 | `crates/librefang-api/src/routes/workflows.rs` | `run_schedule` | tenant-facing schedule run leaked backend workflow errors | generic route-safe error body | `run_schedule_sanitizes_internal_errors` | fixed |
| P0 | `crates/librefang-api/src/routes/workflows.rs` | `dry_run_workflow` | tenant-facing dry-run returned raw backend text | generic `Workflow not found` | `dry_run_workflow_sanitizes_internal_errors` | fixed |
| P0 | `crates/librefang-api/src/routes/system.rs` | `webhook_wake` | tenant-facing webhook publish leaked backend errors | generic publish failure | webhook wake sanitization test | fixed |
| P0 | `crates/librefang-api/src/routes/system.rs` | `webhook_agent` | tenant-facing webhook execution leaked backend errors and name lookup crossed tenants | generic execution failure, tenant-scoped lookup | webhook tenant tests | fixed |
| P0 | `crates/librefang-api/src/channel_bridge.rs` | scoped `/schedule run` `SystemEvent` | scoped bridge schedule could broadcast unscoped event | publish scoped event with `account_id` | `test_manage_schedule_text_scoped_system_event_stays_tenant_scoped` | fixed |
| P0 | `crates/librefang-api/src/channel_bridge.rs` | scoped `/schedule run` `AgentTurn` | tenant-facing bridge command leaked raw run error | generic `Failed to run job.` | `test_manage_schedule_text_scoped_agent_turn_sanitizes_internal_errors` | fixed |
| P0 | `crates/librefang-api/src/channel_bridge.rs` | scoped `/schedule run` `Workflow` | tenant-facing bridge command leaked raw workflow errors | generic `Failed to run workflow.` / `Workflow not found.` | `test_manage_schedule_text_scoped_workflow_sanitizes_internal_errors` | fixed |
| P0 | `crates/librefang-api/src/routes/skills.rs` | `hand_send_message` | tenant-facing hand message leaked backend error | generic `Message delivery failed` | `hand_send_message_sanitizes_internal_errors` | fixed |
| P0 | `crates/librefang-api/src/routes/skills.rs` | `hand_get_session` | tenant-facing hand session leaked backend load error | generic `Failed to load session` | `hand_get_session_error_response_is_generic` | fixed |
| P1 | `crates/librefang-api/src/routes/agents.rs` | `send_message` | tenant-facing send path leaked backend delivery errors | generic translated delivery failure | focused route test | fixed |
| P1 | `crates/librefang-api/src/routes/agents.rs` | `send_message_stream` sender context | streaming message path dropped `SenderContext` and downgraded to the less-scoped execution path | streaming route reuses the same sender-context helper and sender-aware kernel entry point as non-streaming send | `test_request_sender_context_*`, `routes::agents::tests::*` compile/run | fixed |
| P1 | `crates/librefang-api/src/routes/agents.rs` + related callers | `resolve_attachments` scope | tenant-owned attachment resolution accepted optional account scope and carried an unregistered/global fallback branch | attachment resolution now requires concrete tenant scope everywhere it is used | `test_resolve_attachments_rejects_cross_tenant_uploads`, `test_resolve_attachments_rejects_unregistered_uploads_for_scoped_callers` | fixed |
| P1 | `crates/librefang-api/src/routes/agents.rs` | `inject_message` | tenant-facing inject path returned raw internal errors | generic `Message injection failed` / `Agent not found` | `inject_message_sanitizes_internal_errors` | fixed |
| P1 | `crates/librefang-api/src/routes/agents.rs` | `import_session` | tenant-facing import leaked raw serde parse details | generic `Invalid export format` | `import_session_sanitizes_invalid_export_errors` | fixed |
| P1 | `crates/librefang-api/src/routes/agents.rs` | `push_message` | tenant-facing push path leaked backend delivery errors | generic translated delivery failure | `push_message_sanitizes_backend_errors` | fixed |
| P1 | `crates/librefang-api/src/routes/agents.rs` | `spawn_agent` | tenant-facing spawn path embedded raw backend text | generic tenant-safe error body | `spawn_agent_sanitizes_backend_errors` | fixed |
| P1 | `crates/librefang-api/src/routes/agents.rs` | `bulk_create_agents` | tenant-facing bulk create leaked raw backend errors per item | generic per-item tenant-safe errors | `bulk_create_agents_sanitizes_backend_errors` | fixed |
| P1 | `crates/librefang-api/src/routes/network.rs` | `comms_send` | tenant-facing comms send leaked backend delivery errors | generic `Message delivery failed` | `comms_send_sanitizes_internal_errors` | fixed |
| P1 | `crates/librefang-api/src/routes/network.rs` | `comms_task` | tenant-facing comms task leaked backend task-store errors | generic `Failed to post task` | `comms_task_sanitizes_internal_errors` | fixed |
| P1 | `crates/librefang-api/src/routes/skills.rs` | `get_hand` | tenant-facing missing-hand path echoed detail | generic `Hand not found` | `get_hand_sanitizes_missing_hand_errors` | fixed |
| P1 | `crates/librefang-api/src/routes/skills.rs` | `activate_hand` | tenant-facing activation failures echoed detail | generic `Failed to activate hand` | `activate_hand_sanitizes_missing_hand_errors` | fixed |
| P1 | `crates/librefang-api/src/routes/skills.rs` | `remove_integration` | user-visible not-found path leaked uninstall detail | generic `Integration not found` | `remove_integration_sanitizes_not_found_errors` | fixed |
| P1 | `crates/librefang-kernel/src/triggers.rs` + related | event dispatch + trigger evaluation | tenant A events could trigger tenant B triggers | scoped events fail closed on tenant mismatch | kernel tenant event tests | fixed |
| P1 | `crates/librefang-runtime/src/tool_runner.rs` + related | `event_publish` tool | tool-driven publish could drop caller tenant context | caller-scoped event publish helper | `test_event_publish_passes_caller_agent_context` | fixed |
| P1 | `crates/librefang-api/src/routes/workflows.rs` + `crates/librefang-kernel/src/kernel.rs` | manual schedule / cron `SystemEvent` | system events could publish without tenant scope | carry job `account_id` into event | workflow/kernel tenant tests | fixed |
| P1 | `crates/librefang-kernel/src/kernel.rs` | caller-bound `cron_create` / `cron_list` / `cron_cancel` | unowned caller agents still had legacy cron behavior | require tenant-owned caller and account-scoped cron access | `test_cron_create_rejects_unowned_caller_agent`, `test_cron_list_rejects_unowned_caller_agent`, `test_cron_cancel_stays_tenant_scoped` | fixed |
| P1 | `crates/librefang-kernel/src/kernel.rs` | approval + heartbeat event producers | tenant-owned approval/health events bypassed `publish_event(...)` and lost inferred tenant scope | route producer events through the normal kernel publish pipeline | `test_submit_tool_approval_keeps_approval_events_tenant_scoped` | fixed |
| P1 | `crates/librefang-api/src/channel_bridge.rs` | `None`-scoped `/schedule` commands | CLI/global path could still drift toward tenant-owned jobs | `None` scope sees unowned/global jobs only and refuses tenant-owned agents | `test_none_scoped_schedule_commands_ignore_tenant_owned_jobs`, `test_none_scoped_add_refuses_tenant_owned_agents` | fixed |
| P1 | `crates/librefang-api/src/channel_bridge.rs` | `None`-scoped `/trigger` commands | CLI/global path could still drift toward tenant-owned triggers | `None` scope sees unowned/global triggers only and refuses tenant-owned agents | `test_none_scoped_trigger_commands_ignore_tenant_owned_triggers`, `test_none_scoped_add_refuses_tenant_owned_agents` | fixed |
| P1 | `crates/librefang-api/src/channel_bridge.rs` | unscoped bridge helper fallback | direct `manage_schedule_text` / trigger helper still had all-tenant semantics if called directly | unscoped helpers delegate to hardened `*_scoped(None, ...)` paths | covered by same `None`-scope bridge tests | fixed |
| P1 | `crates/librefang-api/src/routes/system.rs` | sessions + backup + webhook handlers | raw internal error strings leaked through mixed tenant/admin system surfaces | replace route-facing raw error text with generic responses plus server-side logging | `cargo check -p librefang-api --tests --offline` + raw-error grep | fixed |
| P0 | `crates/librefang-api/src/routes/skills.rs` | `list_hands` requirement payload | tenant-visible hand catalog leaked real environment variable values through `requirements[].current_value` | never return secret/env values in tenant-visible requirement payloads | `list_hands_does_not_expose_requirement_env_values` | fixed |
| P1 | `crates/librefang-api/src/routes/skills.rs` | hand lifecycle/settings actions | `pause_hand` / `resume_hand` / `deactivate_hand` / `update_hand_settings` echoed backend detail on failure | generic hand-action failure responses plus logging | `hand_action_failed_response_is_generic` | fixed |
| P2 | `crates/librefang-api/src/routes/skills.rs` | hand definition/catalog classification | daemon-global hand definition readiness/install metadata was still served through a tenant-visible route family without explicit classification | hand catalog is now documented as explicit global catalog behavior; tenant-owned behavior stays in activation/settings/instances | route docs + audit classification | fixed |
| P1 | `crates/librefang-runtime/src/tool_runner.rs` + `crates/librefang-api/src/routes/agents.rs` | runtime-generated upload ownership | runtime `image_generate` wrote `/api/uploads/...` files without registry metadata and depended on the route's unscoped fallback | generated uploads now write sidecar ownership/content metadata and `serve_upload` honors it when the registry is missing | `test_serve_upload_honors_sidecar_scope_without_registry_entry`, targeted runtime compile/test | fixed |
| P1 | `crates/librefang-api/src/routes/agents.rs` | `/api/uploads` legacy disk fallback | route allowed raw disk-only upload serving for files with no registry entry or sidecar ownership metadata | remove disk-only fallback so upload serving requires registry metadata or sidecar ownership metadata | `test_serve_upload_honors_sidecar_scope_without_registry_entry`, `test_serve_upload_rejects_unregistered_disk_only_fallback`, `cargo check -p librefang-api --tests --offline` | fixed |
| P1 | `crates/librefang-api/src/routes/agents.rs` | session/update/runtime control handlers | tenant-visible agent routes still surfaced backend/internal error text in scattered JSON error bodies | replace route-facing raw error text with generic responses plus server-side logging | `cargo check -p librefang-api --tests --offline` + raw-error grep | fixed |
| P2 | `crates/librefang-api/src/routes/system.rs` | registry content CRUD | admin/global registry content handlers echoed content-type, existence, and path detail in response text | generic admin-safe request/conflict/type errors and no filesystem path in success body | `registry_content_*_response_is_generic` | fixed |
| P2 | `crates/librefang-api/src/routes/skills.rs` | skill install/catalog + admin/config/install flows | MCP config update/delete and integration reconnect/reload still surfaced raw backend detail in admin responses | admin/config/install surfaces now use generic admin-safe failure bodies; remaining global catalog behavior is explicitly classified above | `admin_sanitizer_helpers_are_generic` + focused code review | fixed |
| P2 | `crates/librefang-api/src/routes/agents.rs` | residual admin/file/update surfaces | broad residual row remained after earlier hardening | file/update/control surfaces are now reduced to generic validation and route-safe errors; upload serving is now fully tenant-owned and fail-closed | source sweep + existing `patch_agent_sanitizes_backend_errors` and file-route review | fixed |
| P2 | `crates/librefang-api/src/channel_bridge.rs` | scoped create/remove flows | bridge create/remove still echo raw errors | generic user-safe responses + logging | `test_manage_schedule_text_scoped_add_sanitizes_internal_errors` + focused code review for trigger/remove paths | fixed |
| P1 | `crates/librefang-api/src/channel_bridge.rs` + `crates/librefang-kernel/src/kernel.rs` | explicit global/manual workflow list/run path | `None`-scoped workflow commands could see tenant-owned workflows and global workflow runs could resolve tenant-owned step agents | `None` scope sees unowned/global workflows only and global workflow runs resolve unowned agents only | bridge/kernel workflow scope tests | fixed |

## Async / Execution Notes

Improved:
- scoped event envelope now carries `account_id`
- trigger evaluation filters on scoped tenant
- runtime tool-driven `event_publish` now preserves caller tenant
- manual schedule and cron `SystemEvent` paths now carry `job.account_id`
- approval and heartbeat producers now go through `publish_event(...)` so source-agent tenant inference is preserved
- `publish_event_for_agent(...)` now fails closed for unowned callers instead of publishing a global custom event
- webhook wake publishes scoped events
- webhook agent lookup is tenant-scoped

Remaining review:
- the reachable `None`-scoped schedule and trigger command paths are now limited to unowned/global objects only
- the reachable `None`-scoped workflow command paths are now limited to unowned/global workflows only
- caller-bound runtime/kernel cron access now requires a tenant-owned caller
- `channel_bridge.rs` still contains intentional dual-mode helpers where `None`
  means CLI/global compatibility rather than tenant scope; those should remain
  documented and re-audited before any future reuse

## Next Audit Targets

Highest-value remaining follow-up targets:

- remaining cross-surface event publishing producers outside the already-hardened
  runtime/kernel/channel flow, especially cron background execution
- remaining `Option<&str>` scoped helper APIs that can degrade to global/manual
  behavior by design
- remaining dual-mode bridge helpers such as
  `find_agent_entry_by_name_scoped(account_id: Option<&str>, ...)`

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
CARGO_HOME=/tmp/librefang-cargo-home CARGO_TARGET_DIR=/tmp/librefang-target-kernel cargo test -p librefang-kernel --lib test_publish_event_for_agent_rejects_unowned_caller_agent -- --nocapture
CARGO_HOME=/tmp/librefang-cargo-home CARGO_TARGET_DIR=/tmp/librefang-target-kernel cargo test -p librefang-kernel --lib test_cron_background_system_event_stays_tenant_scoped -- --nocapture
CARGO_HOME=/tmp/librefang-cargo-home CARGO_TARGET_DIR=/tmp/librefang-target-api cargo check -p librefang-api --tests --offline
```

Current note:
- targeted `librefang-api` bridge tests now pass for the hardened `None`-scoped global/manual bridge behavior
- broader API verification is still narrower than a full clean branch proof because this work is landing in a dirty worktree with unrelated concurrent changes

## Remaining `Option<&str>` Re-Audit Targets

These are the remaining places where the recurring gremlin class still exists
in some form and should be treated as the next audit map. Under the
convergence policy above, each one should either:

- be migrated to a concrete tenant/account boundary
- be labeled as an explicit compatibility seam
- or be confirmed as intentional admin/global infrastructure

Already hardened in this audit:

- `crates/librefang-channels/src/bridge.rs`
  - scoped trait defaults now fail closed when `account_id.is_some()`
- `crates/librefang-runtime/src/kernel_handle.rs`
  - `publish_event_scoped(...)` and `publish_event_for_agent(...)` defaults now fail closed when tenant scope is present or caller-scoped publish is requested on non-real handles
- `crates/librefang-api/src/channel_bridge.rs`
  - `None`-scoped trigger/schedule helpers are boxed into unowned/global objects only
- `crates/librefang-api/src/channel_bridge.rs` + `crates/librefang-kernel/src/kernel.rs`
  - `None`-scoped workflow list/run is boxed into unowned/global workflows only
  - global workflow execution no longer resolves tenant-owned step agents
- `crates/librefang-kernel/src/kernel.rs`
  - caller-bound cron helpers no longer degrade to unowned/global behavior

Still worth re-auditing:

- `crates/librefang-api/src/routes/agents.rs`
  - raw `/api/uploads` disk-only fallback is now removed
  - runtime-generated uploads carry sidecar ownership metadata and upload
    serving now fails closed when ownership metadata is missing

- `crates/librefang-kernel/src/kernel.rs`
  - `publish_event(...)`
  - `publish_event_scoped(..., account_id: Option<&str>)`
  - `publish_event_for_agent(...)`
  - This is the most likely next family to re-audit because event publishing is
    cross-surface: runtime tools, webhook wake, bridge/manual schedule runs,
    cron background execution, and any future async producer can all land here.
    The main question is no longer trigger filtering; it is whether every
    producer chooses the right publish path and always carries tenant scope when
    the event is tenant-owned. The trait-level fallback gremlin here is now
    closed; the remaining review is concrete producer classification.

- `crates/librefang-api/src/channel_bridge.rs`
  - `find_agent_entry_by_name_scoped(account_id: Option<&str>, ...)`
  - `spawn_agent_by_name_owned(..., account_id: Option<&str>)`
  - These are currently intentional dual-mode helpers for tenant-owned versus
    global/legacy bridge execution. They are not a confirmed bug, but they are
    part of the same smell and should be rechecked whenever bridge policy
    expands.

- `crates/librefang-kernel/src/kernel.rs`
  - provider/account resolution helpers that accept `account_id: Option<&str>`
  - These were not part of this gremlin pass and are not currently implicated,
    but they fit the same structural pattern and should be treated as future
    audit candidates if provider routing regresses.

## Next Order

1. Add broader schedule/lifecycle regression coverage only when new producer paths appear, rather than continuing one-off spot checks.
