# DESIGN: Channel Identity to Tenant Binding

**Status:** Partially Implemented
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `ADR-MT-002-API-AUTH.md`, `ROUTE-POLICY-MATRIX.md`, `COMP-HERMES-AGENT.md`

---

## Purpose

Define how channel-originated traffic enters the Qwntik multi-tenant runtime
without weakening tenant ownership rules.

This is the missing bridge between:

- messaging/session identity
- tenant account identity
- tenant-owned backend resources

---

## Problem

LibreFang already has a clear target policy for HTTP/API requests:

- concrete `X-Account-Id`
- signature verification
- tenant-owned resources scoped by `account_id`

Channel-originated traffic is harder:

- upstream platforms identify users/chats/threads, not tenants
- one integration may service many users
- one chat may contain users from different organizational scopes
- conversation scope is not the same thing as tenant scope

Without a formal binding model, channel handlers risk reintroducing the same
scope confusion the HTTP layer is being cleaned up to remove.

---

## Non-Negotiable Rules

1. No channel-originated tenant action may execute without a concrete tenant binding.
2. Platform user identity is not itself a tenant identity.
3. Session partitioning does not replace tenant authorization.
4. Ambiguous bindings fail closed.
5. Shared channels require explicit policy, not implicit heuristics.

---

## Model

Channel ingress should resolve through these layers:

1. **Platform Identity**
   Examples:
   - Telegram user ID
   - Discord user ID
   - Slack user ID
   - WhatsApp sender ID

2. **Conversation Scope**
   Examples:
   - DM
   - channel
   - thread
   - group chat

3. **Integration Binding**
   Which configured channel integration received the event

4. **Tenant Binding**
   One concrete tenant account resolved from the above context

Only after tenant binding succeeds may the request operate on tenant-owned
resources.

---

## Recommended Binding Strategies

### Strategy A: Integration-Instance Bound to One Tenant

One configured channel integration belongs to one tenant.

Use when:

- each tenant has its own bot/app/webhook
- infrastructure can be provisioned per tenant

Benefits:

- simplest policy
- easiest to audit
- least ambiguity

Cost:

- more integration objects to manage

### Strategy B: User Binding Within Shared Integration

One shared bot serves many users, but each authorized user is mapped to one tenant.

Use when:

- platform constraints make shared bots desirable
- each sender identity can be stably mapped to a tenant

Requirements:

- explicit mapping table
- no default tenant
- no fallback to "system"

Risk:

- higher operational complexity
- must handle support/admin users explicitly

### Strategy C: Thread or Channel Affinity Binding

One shared integration binds a specific room/thread/channel to a tenant.

Use when:

- the channel itself is the tenant workspace

Requirements:

- explicit room-to-tenant mapping
- rebinding controlled as an admin action

Risk:

- dangerous if multiple tenant populations share a room

---

## What Must Never Happen

- derive tenant identity from display name or free-form text
- assume one gateway process implies one tenant
- let a session continue with stale tenant context after rebinding
- allow missing binding to fall back to global/system behavior
- use conversation scope alone as authorization

---

## Recommended Storage Shape

The long-term runtime should have an explicit binding store for channel ingress:

- `integration_id`
- `platform`
- `platform_user_id` and/or `conversation_id`
- `binding_scope`
  - `integration`
  - `user`
  - `conversation`
  - `thread`
- `account_id`
- `created_at`
- `updated_at`
- `created_by`
- optional expiration/revocation metadata

This should be treated as security-sensitive configuration, not an incidental cache.

## Implementation Choice: V1 Binding Store

The first shippable implementation does not add a new database table yet.

Instead, the binding store is the existing tenant-owned channel instance
configuration:

- each configured channel instance already persists a concrete `account_id`
- adapters are started with that `account_id`
- inbound `ChannelMessage` events carry that `account_id` in `metadata`
- bridge dispatch treats that `metadata.account_id` as the required ingress binding

This is intentionally narrow:

- integration instance -> account binding is implemented now
- user/thread/room binding is left for future work
- there is no default tenant and no bridge-side inference

## Concrete Enforcement Points

Ingress binding v1 is enforced in the runtime path, not only at the route layer.

### Bridge dispatch

`crates/librefang-channels/src/bridge.rs`

- `ingress_binding_account_id()` requires a concrete `metadata.account_id`
- `dispatch_message()` rejects unbound inbound events before command handling,
  agent routing, or tenant-owned mutation
- `dispatch_with_blocks()` does the same for multimodal ingress
- journal entries now preserve inbound `metadata` so crash recovery keeps the
  tenant binding

### Router resolution

`crates/librefang-channels/src/router.rs`

- `resolve_with_context()` now treats account-bound ingress as account-scoped
- channel defaults resolve against `ChannelType:account_id`
- user defaults resolve against `account_id:platform_user_id`
- account-bound ingress does not fall back to the system default agent

### Command path

`crates/librefang-channels/src/bridge.rs`

- `/agent` now writes an account-scoped user default when the ingress event is
  tenant-bound
- command-time agent resolution now uses the bound account context rather than
  legacy global fallback

## Failure Mode

The bridge now fails closed for unbound channel ingress.

If an inbound event arrives without a concrete tenant binding:

1. no command is executed
2. no agent is resolved
3. no tenant-owned mutation runs
4. the user receives a tenant-binding error response

There is no fallback to `AccountId(None)`, `system`, or the daemon-global
default agent for tenant-bound ingress.

---

## Request Handling Contract

For each inbound channel event:

1. validate the source integration
2. derive platform identity + conversation scope
3. resolve a tenant binding
4. if resolution fails, stop before tenant-owned actions
5. pass concrete `account_id` into downstream handlers and agent context
6. ensure all created resources inherit the resolved tenant ownership

---

## Interaction With Session Scope

Session scope and tenant scope are different dimensions:

- session scope answers "which conversation history should this message join?"
- tenant scope answers "which account namespace may this action affect?"

Examples:

- two users in one shared room may need separate conversation sessions but the same tenant
- one operator may access multiple tenants, but each message must bind to exactly one tenant
- one DM session must not silently cross tenant boundaries if the operator changes context

This separation should be explicit in code and docs.

---

## Testing Requirements

At minimum, tests should cover:

- unbound platform user is rejected
- bound platform user resolves to exactly one tenant
- wrong-tenant channel access cannot read or mutate tenant-owned resources
- shared-room binding does not leak across tenants
- rebinding requires explicit privileged action
- tenant-owned artifacts created from channel traffic persist concrete `account_id`

---

## Recommendation

Prefer **integration-bound** or **explicit user-bound** tenancy.

Avoid implicit room-based or heuristic binding unless the product model makes the
room itself the tenant workspace and that policy is explicit.

For Qwntik, the safest default is:

- no tenant-owned action without a resolved binding
- no default tenant
- no `system` fallback
- no conversation-only authorization

---

## Current Implementation Decision

For the Qwntik fork, channel configuration itself is tenant-owned.

That means:

- each configured channel instance must carry a concrete `account_id`
- each tenant writes only its own config entry for a given channel type
- each tenant secret is stored under an account-scoped env var name
- route reads must filter by the requesting tenant account
- one tenant must not see whether another tenant configured the same channel type

This now solves the first ingress-binding layer for integration-instance-bound
tenancy.

It does not yet implement:

- shared integration user binding
- shared room/thread binding
- QR/session-state ownership for adapters that need explicit interactive setup
- adapter-specific route partitioning for fixed-route families such as `webhook`

---

## Storage Contract

Channel configuration is persisted in shared daemon files, but ownership is
represented explicitly inside the stored records.

### `config.toml`

Channel sections must support one-or-many entries per channel type.

Example:

```toml
[[channels.telegram]]
account_id = "acct_alpha"
bot_token_env = "TELEGRAM_BOT_TOKEN__ACCT_ALPHA"
default_agent = "support"

[[channels.telegram]]
account_id = "acct_beta"
bot_token_env = "TELEGRAM_BOT_TOKEN__ACCT_BETA"
default_agent = "ops"
```

Rules:

- `account_id` is required for tenant-owned channel entries
- upsert replaces only the matching tenant entry
- delete removes only the matching tenant entry
- other tenants’ entries must be preserved

### `secrets.env`

Secret values remain in `secrets.env`, but variable names are account-scoped.

Example:

- `TELEGRAM_BOT_TOKEN__ACCT_ALPHA`
- `SLACK_BOT_TOKEN__ACCT_BETA`

Rules:

- account suffixes must be sanitized to env-safe uppercase identifiers
- the config entry stores the env var name, not the secret value
- removing one tenant’s channel config must remove only that tenant’s secret keys

---

## Route Contract

The main channel CRUD/test surface is tenant-facing:

- `GET /api/channels`
- `GET /api/channels/{name}`
- `POST /api/channels/{name}/configure`
- `DELETE /api/channels/{name}/configure`
- `POST /api/channels/{name}/test`

Required behavior:

- concrete `X-Account-Id`
- only the caller’s channel entry is visible
- required-secret checks resolve through the caller’s configured env var names
- cross-tenant presence must be invisible

The following remain admin-only infrastructure for now:

- manual reload
- registry/introspection endpoints
- QR/session flows until session state is explicitly tenant-owned

---

## Instance Cardinality

Channel instance cardinality is adapter-specific.

Current Qwntik rule:

- config storage supports multiple entries for the same channel type across tenants
- runtime adapter registration uses tenant-qualified keys for multi-instance-safe adapters
- adapter families carry an explicit runtime multiplicity stance close to the adapter definition
- adapters with shared fixed routes or ports are not automatically safe for multiple tenant instances
- unsupported duplicate configurations are rejected during startup/reload before adapter registration or route mounting

Current example:

- `webhook` cannot safely host multiple tenant instances on the same mounted route shape without further route partitioning

Current single-instance-per-daemon families include fixed-route/shared-port adapters such as:

- `webhook`
- `voice`
- `google_chat`
- `teams`
- `line`
- `viber`
- `messenger`
- `threema`
- `pumble`
- `flock`
- `dingtalk` in webhook mode
- `feishu` / `lark` in webhook mode
- `wecom` in callback mode

Implication:

- “one entry per `(channel_type, account_id)`” is the storage baseline
- actual runtime multiplicity must still be validated per adapter family
- tenant-local configure/reload must preserve the existing live adapter set when a new conflicting config is rejected

Current limitation:

- v1 does not yet model multiple instances of the same channel type for the same tenant
- a second same-type integration for the same `account_id` is treated as an update to the existing tenant entry, not as a distinct instance

To support true same-tenant multi-instance ownership safely, the model must add
an explicit instance identity such as:

- `integration_id`
- `instance_id`
- tenant-unique integration slug/name

That identity must then flow through:

- config persistence
- secret naming
- route read/update/delete targeting
- runtime adapter registration
- ingress binding resolution

---

## Bottom Line

HTTP/API tenancy is only half the problem.
For channel-connected agent systems, the real security boundary begins at ingress.

If LibreFang formalizes channel identity to tenant binding as a first-class
design, it will avoid the most common failure mode in "shared bot" systems:
good backend ownership rules undermined by weak ingress identity semantics.
