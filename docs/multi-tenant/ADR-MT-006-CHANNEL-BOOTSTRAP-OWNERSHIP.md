# ADR-MT-006: Channel Bootstrap and Session Ownership

**Status:** Proposed
**Date:** 2026-04-09
**Related:** `TENANT-INVARIANTS.md`, `ADR-MT-003-RESOURCE-ISOLATION.md`, `DESIGN-CHANNEL-TENANT-BINDING.md`, `CURRENT-CODE-AUDIT.md`

---

## Decision

QR, pairing, and session-bootstrap artifacts for channel integrations are
security-sensitive ownership state.

They must not exist only as daemon-global runtime memory, raw external provider
handles, or operator knowledge.

For the Qwntik fork, bootstrap/session state must bind to:

- one concrete configured channel instance
- one concrete `account_id`
- one explicit bootstrap lifecycle record

Normal tenant ownership for inbound channel traffic is not considered complete
until the bootstrap/session state that created that runtime is also owned and
fail-closed.

---

## Problem

The current branch already converged:

- tenant-owned channel config/secrets
- account-qualified adapter startup
- account-bound ingress routing
- multiplicity enforcement for unsafe adapter families

But QR/session-heavy adapters still have a gap before they reach that state:

- QR start/status flows are still daemon-global admin/operator endpoints
- external bootstrap handles like `session_id` or `qr_code` are not persisted as
  local owned records
- restart/reload semantics for in-progress bootstrap are not modeled
- bootstrap ownership is not expressed as part of the channel instance contract

This creates an architecture mismatch:

- the connected adapter may be tenant-owned
- the process that made it connected is still globally scoped and weakly owned

---

## Rules

1. No bootstrap flow may exist without one concrete configured target instance.
2. No bootstrap flow may exist without one concrete `account_id`.
3. Raw provider bootstrap handles are not sufficient ownership authority.
4. Poll, confirm, cancel, and resume actions must resolve through local owned
   bootstrap state, not through caller-supplied provider handles alone.
5. Runtime startup must only consume owned persisted session/token material.
6. Adapters that cannot satisfy these rules remain explicitly admin/global-only
   or single-instance-per-daemon.

---

## In Scope

- WeChat QR login ownership
- WhatsApp Web gateway bootstrap ownership wrapper
- persistence of bootstrap lifecycle state
- restart/reload behavior for pending bootstrap sessions
- instance-targeted operator/admin bootstrap routes

---

## Out of Scope

- shared user/chat/thread ownership beyond integration-instance binding
- full redesign of external WhatsApp gateway internals
- voice/session namespace redesign
- broad same-channel multi-instance product redesign across all adapters

If same-channel multi-instance identity is required for a QR-capable family, it
must be solved explicitly for that family before bootstrap ownership can be
called complete.

---

## Consequences

### Positive

- bootstrap ownership becomes auditable and restart-safe
- operator actions become explicit and instance-targeted
- one tenant's bootstrap flow cannot be confused with another tenant's
- runtime activation and bootstrap lifecycle use the same ownership model

### Negative

- adds a new persisted control-plane record type
- requires instance-targeted admin/operator APIs instead of daemon-global QR
  helpers
- may expose adapter-specific limits where true multi-instance bootstrap is not
  realistically safe

---

## Adapter Classification Consequence

### Supported with owned bootstrap records

- WeChat QR login
- WhatsApp Web gateway mode, but only through a local owned wrapper around
  gateway session handles

### Not solved by this ADR

- voice and other fixed-route/shared-session families that already require
  singleton runtime treatment

Those remain intentionally constrained until they gain explicit route/session
partitioning.

---

## Failure Model

If bootstrap ownership cannot be established:

- no tenant-owned runtime instance may be activated from that flow
- no provider handle may be treated as locally authoritative
- reload/startup must fail closed rather than guessing owner state

This fork prefers explicit rejection over bootstrap ambiguity.
