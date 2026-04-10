# PLAN-MT-003: Channel Bootstrap Ownership Hardening

**Status:** Proposed
**Date:** 2026-04-09
**Related:** `ADR-MT-006-CHANNEL-BOOTSTRAP-OWNERSHIP.md`, `SPEC-MT-005-CHANNEL-BOOTSTRAP-SESSION-OWNERSHIP.md`, `PENDING-WORK.md`

---

## Goal

Close the remaining QR/session ownership gap in the channel stack without
reopening already-converged channel config/runtime isolation work.

---

## Scope

### In scope

- local owned bootstrap session records
- WeChat QR ownership
- WhatsApp gateway bootstrap ownership wrapper
- restart/reload safety for pending bootstrap sessions
- instance-targeted admin/operator bootstrap endpoints

### Out of scope

- voice session redesign
- shared user/chat/thread ownership
- broad adapter redesign
- general same-family multi-instance product expansion

---

## Phase 1: Ownership Record

1. add `ChannelBootstrapSession` type
2. add persistence helpers and atomic rewrite behavior
3. add lookup helpers by local instance identity
4. add expiry/cancel/status state transitions

Exit criteria:

- bootstrap records persist concrete `account_id`
- bootstrap records survive restart
- record lookup does not require caller-supplied raw provider handles

---

## Phase 2: WeChat First

1. replace raw WeChat QR start/status shape with instance-targeted bootstrap API
2. persist returned `qr_code` under local owned bootstrap record
3. poll status through the owned record
4. write confirmed `bot_token` only into the target instance secret/config slot
5. keep runtime startup behavior: no token means no runtime adapter start

Why first:

- local runtime already has the correct containment shape
- provider bootstrap is simple enough to wrap cleanly
- closes the cleanest QR ownership gap first

Exit criteria:

- no raw `qr_code` authority in handler surface
- restart-safe WeChat bootstrap ownership
- token persistence is instance-scoped only
- Phase 2 may use the V1 containment instance identity model
  (`channel_type + account_id`) where that is still the only safe explicit
  identity. Later phases may require explicit instance keys for families that
  need true same-family multi-instance support.

---

## Phase 3: WhatsApp Gateway Wrapper

1. replace raw WhatsApp `session_id` polling with instance-targeted bootstrap API
2. persist returned gateway `session_id` in owned bootstrap record
3. poll provider through local owned state only
4. document and reject any gateway behavior that cannot safely preserve local
   ownership assumptions

Exit criteria:

- no raw `session_id` authority in handler surface
- pending WhatsApp bootstrap has local owner binding
- conflict/restart behavior is explicit

---

## Phase 4: Reload / Recovery Hardening

1. validate bootstrap records during reload/startup
2. reject conflicting or orphaned bootstrap ownership state
3. preserve healthy active runtime when bootstrap activation fails
4. clean stale expired bootstrap records deterministically

Exit criteria:

- reload does not silently steal/bootstrap the wrong instance
- restart preserves or rejects bootstrap state deterministically

---

## Test Plan

Full test requirements are defined in
`SPEC-MT-005-CHANNEL-BOOTSTRAP-SESSION-OWNERSHIP.md` under `## Tests`.
The tests below are the core implementation checks for this plan; use the spec
as the full coverage reference, including nice-to-have and broader
adapter-specific expectations.

### Core tests

- bootstrap start requires explicit target instance
- wrong-instance lookup cannot read another instance bootstrap state
- confirmed provider session material writes only to the intended owner
- restart restores pending bootstrap ownership
- reload conflict does not replace healthy tenant runtime

### Adapter-specific tests

- WeChat QR code persisted as owned bootstrap handle
- WeChat confirmed token writes only to target instance
- WhatsApp gateway session ID persisted as owned bootstrap handle
- WhatsApp polling uses stored handle, not raw query authority

---

## Risks

### 1. Hidden dependence on same-family singleton identity

Containment:

- use explicit instance key or reject ambiguous same-family duplicates

### 2. External gateway semantics may be weaker than local ownership assumptions

Containment:

- wrap locally
- reject unsupported flows explicitly
- document unresolved gateway-side limits rather than guessing

### 3. Reload side effects can disable healthy adapters

Containment:

- validate before activation
- preserve current runtime on conflict

---

## Operator-Facing Changes

- QR flows become instance-targeted rather than daemon-global
- bootstrap status is read by local instance ownership, not raw provider handle
- operators can see whether a pending bootstrap belongs to a specific channel
  instance

---

## Definition of Done

- WeChat bootstrap ownership is locally persisted and restart-safe
- WhatsApp bootstrap state is locally wrapped and owned
- bootstrap routes no longer trust raw provider handles directly
- reload/startup handle pending bootstrap state explicitly
- docs classify remaining unresolved session families honestly
