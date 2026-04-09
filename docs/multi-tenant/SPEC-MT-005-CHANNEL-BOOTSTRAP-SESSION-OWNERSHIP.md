# SPEC-MT-005: Channel Bootstrap and Session Ownership

**Status:** Proposed
**Date:** 2026-04-09
**Related:** `ADR-MT-006-CHANNEL-BOOTSTRAP-OWNERSHIP.md`, `TENANT-INVARIANTS.md`, `DESIGN-CHANNEL-TENANT-BINDING.md`

---

## Purpose

Define the narrowest correct implementation model for QR/pairing/session
ownership in the Qwntik multi-tenant channel stack.

This spec covers the bootstrap state that exists before a QR-capable adapter is
fully connected and running as a tenant-owned integration.

---

## Current Code Reality

### Already converged

- configured channel instances persist concrete `account_id`
- runtime adapter startup passes `account_id` into adapters
- inbound `ChannelMessage.metadata.account_id` is required for tenant-bound
  ingress dispatch

### Not yet converged

- WhatsApp QR start/status routes use raw external `session_id`
- WeChat QR start/status routes use raw external `qr_code`
- no local bootstrap record persists ownership for those handles
- restart/reload behavior for pending bootstrap sessions is not modeled

---

## Security Goal

A QR/session bootstrap flow must be owned exactly like a tenant-owned integration
instance:

- one configured channel instance
- one concrete `account_id`
- one persisted lifecycle record

No caller should be able to drive bootstrap lifecycle using only a provider
handle that is not first resolved through local owned state.

---

## Data Model

Add a persisted bootstrap/session record:

### `ChannelBootstrapSession`

- `bootstrap_id: String`
- `channel_type: String`
- `instance_key: String`
- `account_id: String`
- `bootstrap_kind: BootstrapKind`
- `provider_handle: Option<String>`
- `provider_qr_payload: Option<String>`
- `provider_qr_url: Option<String>`
- `provider_pairing_code: Option<String>`
- `status: BootstrapStatus`
- `created_at: DateTime<Utc>`
- `updated_at: DateTime<Utc>`
- `expires_at: Option<DateTime<Utc>>`
- `created_by: String`
- `last_error: Option<String>`

### `BootstrapKind`

- `QrLogin`
- `PairingCode`
- `SessionReauth`

### `BootstrapStatus`

- `Pending`
- `Confirmed`
- `Expired`
- `Cancelled`
- `Failed`

---

## Instance Identity

Bootstrap ownership must resolve through one explicit configured channel
instance.

### V1 containment

Where a family still only supports one instance per `(channel_type, account_id)`,
the instance key may be derived from that tuple.

### Preferred end state

Use an explicit integration/instance key that survives:

- start
- poll
- confirm
- cancel
- restart
- reload

The bootstrap model should not depend on raw provider handles as the primary
key.

---

## Persistence

The first implementation may remain file-backed to stay narrow.

Requirements:

- records must persist under daemon-managed local state
- records must survive restart
- records must be rewritten atomically
- lookups must key by local instance ownership first
- provider handle lookups must be secondary only

Recommended first shape:

- one bootstrap state file under daemon home
- records stored by `bootstrap_id`
- helper indices for `(channel_type, instance_key)` and `provider_handle`

---

## Route Model

Replace daemon-global QR helpers with instance-targeted bootstrap endpoints.

### Current

- `POST /api/channels/whatsapp/qr/start`
- `GET /api/channels/whatsapp/qr/status?session_id=...`
- `POST /api/channels/wechat/qr/start`
- `GET /api/channels/wechat/qr/status?qr_code=...`

### Target

- `POST /api/channels/{family}/{instance_key}/bootstrap/start`
- `GET /api/channels/{family}/{instance_key}/bootstrap/status`
- `POST /api/channels/{family}/{instance_key}/bootstrap/cancel`

These remain admin/operator endpoints in v1, but they must target one concrete
owned instance and must never rely on caller-supplied raw provider handles.

---

## Handler Contract

### Start

1. require admin/operator access
2. resolve one configured target instance
3. require target instance to carry concrete `account_id`
4. reject if conflicting pending bootstrap exists for that instance
5. start provider bootstrap
6. persist `ChannelBootstrapSession`
7. return local bootstrap view keyed by instance

### Status

1. require admin/operator access
2. resolve target instance
3. load owned bootstrap record for that instance
4. poll provider using stored provider handle
5. update local record
6. if confirmed, persist token/session material to the owning instance only

### Cancel

1. require admin/operator access
2. resolve target instance
3. load owned bootstrap record
4. cancel provider flow if supported
5. mark local record cancelled

---

## Runtime Contract

Runtime adapter startup must not perform anonymous QR login as a side effect.

### Required behavior

- startup only consumes persisted owned token/session material
- pending bootstrap state is not equivalent to active runtime state
- reload must not steal or overwrite another instance's pending bootstrap
- startup must reject invalid ownership state rather than guessing

The current WeChat bridge containment already follows this model by refusing to
start without a persisted token. That behavior should be preserved.

---

## Adapter-Specific Rules

### WeChat

Supported in this spec.

Flow:

1. `bootstrap/start` requests QR code from iLink
2. returned `qr_code` is stored as `provider_handle`
3. `bootstrap/status` polls iLink using stored handle
4. confirmed `bot_token` is written only to the target instance's configured
   secret slot
5. runtime startup later consumes that owned token

### WhatsApp Web gateway mode

Supported only through a local ownership wrapper.

Flow:

1. `bootstrap/start` requests gateway login start
2. returned gateway `session_id` is stored as `provider_handle`
3. `bootstrap/status` polls using stored handle
4. any durable session/token material must be persisted only to the target
   instance

Limit:

If the external gateway itself cannot provide safe per-session partitioning, the
local ownership wrapper is containment, not full end-state isolation.

### Voice

Out of scope.

Voice already uses a fixed shared route and singleton session namespace, and is
correctly classified as single-instance-per-daemon. This spec does not attempt
to tenantize that session model.

---

## Conflict Rules

1. only one pending bootstrap session per local instance
2. reload/startup must reject contradictory bootstrap state
3. stale bootstrap state must not silently attach to another instance
4. provider handles must not be reusable across different instance owners

---

## Restart / Reload Rules

### Restart

- pending bootstrap records reload as pending
- operator can continue status polling by local instance
- expired/invalid records are surfaced as expired/failed, not silently dropped

### Reload

- active runtime instances remain unchanged if bootstrap validation fails
- a new conflicting bootstrap record must not disable a currently healthy tenant
  runtime

---

## Tests

### Required

1. bootstrap start requires explicit target instance
2. target instance must carry concrete `account_id`
3. status for instance A never returns instance B bootstrap state
4. confirmed WeChat token writes only to the targeted instance secret/config
5. restart preserves bootstrap ownership state
6. conflicting bootstrap sessions are rejected explicitly
7. cancelling one instance bootstrap does not affect another instance
8. no route accepts raw external provider handles as sole ownership authority

### Nice-to-have

1. bootstrap expiry cleanup
2. reload conflict rollback
3. provider error propagation to operator-visible state

---

## Non-Goals

- shared user/chat/thread binding
- general multi-instance identity redesign for all channel families
- external gateway architecture rewrite
- converting admin/operator bootstrap flows into tenant-self-service product APIs

---

## Acceptance Criteria

1. QR/pairing/session bootstrap state is persisted locally with concrete
   instance and tenant ownership
2. runtime startup no longer depends on anonymous QR-side effects
3. raw provider handles are not the only authority for bootstrap lifecycle
4. restart/reload preserve ownership or fail closed
5. unsupported shared-session families remain explicitly constrained

