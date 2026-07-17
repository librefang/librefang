# Audit trail: per-user attribution and filtering

This document is the canonical reference for how the tamper-evident audit trail (`librefang-runtime-audit`) attributes events to a LibreFang user, which event classes are inherently userless, and how operators filter the trail by user.
It complements [`access-log-fields.md`](./access-log-fields.md), which covers the *HTTP access log* (`request_logging` middleware) — a separate, non-tamper-evident stream.

## The attribution fields

Every [`AuditEntry`](../../crates/librefang-runtime-audit/src/lib.rs) carries two optional attribution fields (SQLite schema v22, folded into the per-entry hash only when present so pre-v22 rows keep verifying):

| Field      | Type              | Meaning                                                                                          |
|------------|-------------------|--------------------------------------------------------------------------------------------------|
| `user_id`  | `Option<UserId>`  | The LibreFang user that initiated the action. `None` for daemon-internal / agent-internal events. |
| `channel`  | `Option<String>`  | Where the action entered the system: `"api"`, `"telegram"`, `"slack"`, `"discord"`, `"cron"`, … `None` when there is no meaningful origin channel. |

`record_with_context(agent_id, action, detail, outcome, user_id, channel)` is the write API that commits both fields; the older `record(...)` convenience wrapper passes `None` for both and is reserved for genuinely userless events (see below).

## How identity reaches the audit write

Per-user RBAC (#3054) resolves the caller's identity in the API auth middleware and stores an [`AuthenticatedApiUser`](../../crates/librefang-api/src/middleware.rs) (name, role, `user_id`) in the request extensions.
Any handler for a human-initiated action reads it back and threads it through to the audit write:

```rust
pub async fn some_handler(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    // …
) -> impl IntoResponse {
    let user_id = api_user.as_ref().map(|u| u.0.user_id);
    state.kernel.audit().record_with_context(
        "system",
        AuditAction::ConfigChange,
        detail,
        "ok",
        user_id,
        Some("api".to_string()),
    );
}
```

`api_user` is `Option<Extension<…>>` rather than a required extractor so the handler still functions on loopback / `LIBREFANG_ALLOW_NO_AUTH=1` deployments where no identity is resolved — those requests record `user_id = None` with `channel = Some("api")`, which is the documented "authenticated-surface but anonymous caller" shape.

## Attributed event classes (human-initiated)

These events result from an operator action that carries a resolved identity, and record `user_id` when one is present:

- **Authentication / authorization** — `UserLogin`, `PermissionDenied` (auth middleware, and the in-handler admin gates for `/api/audit/*`, `/api/memory/*`, `/api/budget/*`).
- **Tool invocation** — `ToolInvoke` from `POST /api/tools/{name}/invoke` (direct REST bridge, bypasses the agent loop).
- **Configuration mutation** — `ConfigChange` from backup create / restore, MCP server add / update / taint-patch / delete, config set / reload.
- **User & RBAC management** — `ConfigChange` from user create / update / delete / bulk-import, API-key rotation, per-user policy edits, and per-user budget set / clear (all routed through `persist_users`).
- **Skill evolution** — `skill_evolve:*` (`AgentMessage`) from the dashboard-driven `/api/skills/.../evolve/*` endpoints.
- **Budget enforcement** — `BudgetExceeded` recorded during agent execution carries the initiating user's id and channel when the run originated from a human message (threaded as `attribution_user_id` / `attribution_channel`).

## Inherently userless event classes

Some events have no human initiator by construction.
They are labeled with `user_id = None`, and the `channel` field (or the `agent_id` / detail) identifies the machine origin.
Do **not** try to back-fill a user onto these — there is no correct answer, and inventing one corrupts the forensic record.

| Class                       | Examples (`AuditAction`)                     | Labeling                                                                                  |
|-----------------------------|----------------------------------------------|-------------------------------------------------------------------------------------------|
| **Daemon-internal**         | `DreamConsolidation`, `RetentionTrim`, boot / reload bookkeeping | `user_id = None`, `channel = None`; `agent_id` is the affected agent or `"system"`.       |
| **Scheduled (cron)**        | agent runs fired by a cron job               | `user_id = None`, `channel = Some("cron")` — a cron tick has no interactive user.         |
| **Agent-to-agent**          | `AgentMessage` / `ToolInvoke` from `agent_send` and autonomous-loop turns | `user_id = None`; the acting `agent_id` is the accountable principal, not a user.         |
| **Agent lifecycle (kernel)**| `AgentSpawn`, `AgentKill`                     | Recorded kernel-side with `user_id = None` today; the initiating API caller is not yet threaded through the `KernelHandle` spawn/kill surface (tracked as a follow-up). |
| **Channel-inbound (unmapped)** | `AgentMessage` from a channel with no user mapping | `user_id = None`, `channel = Some("<platform>")`; the platform sender lives in the detail / sender fields, not in `user_id`, which is a *LibreFang* user id. |
| **Manifest signature (kernel)** | `AuthAttempt` on an Ed25519 manifest-signature-verification failure (`POST /api/agents`) | `user_id = None` today: the event is recorded inside the `spawn_agent` idempotency closure (also reached by the bulk-create path), whose actor is frequently automation rather than a single human. Threading the API caller through that shared 3-function path is a follow-up. |

## Filtering by user

`GET /api/audit/query` (admin-gated) accepts a `user` parameter that matches either the stringified `UserId` UUID **or** the raw user name (re-derived via `UserId::from_name`), plus the existing `action` / `agent` / `channel` / `from` / `to` / `limit` filters:

```
GET /api/audit/query?user=Alice
GET /api/audit/query?user=6f1c…-uuid&action=ConfigChange&from=2026-01-01T00:00:00Z
```

Rows with `user_id = None` (the userless classes above) never match a `?user=` filter — to review machine-origin activity, filter by `?channel=cron` or `?action=DreamConsolidation` instead.
`GET /api/audit/export` accepts the same filter set and streams JSON or CSV.

Filtering is an in-memory pass over the bounded, hash-verified in-memory window (`AuditLog::recent`); it never rebuilds SQL from user input, so the injection surface is zero.
