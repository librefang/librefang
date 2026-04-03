# ADR-008: Shared Memory Abuse Resistance and Degraded-Query Controls

**Status**: Draft
**Date**: 2026-03-15
**Authors**: Daniel Alberttis

## Context

The shared store (`shared.rvf`) is a cross-agent knowledge surface — any agent can write to it and any agent can read from it. This creates two classes of risk that do not apply to per-agent stores:
1. **Degraded query quality** — HNSW traversal can return `ResponseQuality::Degraded` or `Unreliable` results under load or after compaction
2. **Adversarial abuse** — a compromised agent or malformed embedding could poison the query space (degenerate distance distribution), cause DoS via high-frequency deep queries, or replay stale queries

ADR-003 Phase 1 provides only basic shared store functionality (`store_shared` / `recall_shared` / `query_audited`). This ADR decides whether and when to add Phase 2 abuse-resistance controls.

## Decision

**Prerequisite satisfied**: ADR-003 Phase 1 is complete as of 2026-03-20 (159 tests passing, clippy clean). This ADR may now proceed to implementation.

All items in this ADR are Phase 2 and feature-flagged (`features = ["shared-phase2"]`). Status remains Draft until Phase 2 implementation begins (see PLAN-002 T-01 through T-05).

If adopted, Phase 2 controls include:

**Degraded query handling (`query_with_envelope`)**
- Returns `ResponseQuality` (`Verified` / `Usable` / `Degraded` / `Unreliable`)
- On `Degraded`/`Unreliable`: retry once with `QualityPreference::AcceptDegraded` + `SafetyNetBudget::FULL.extended_4x()`; if still degraded, accept as-is — no further retries, no merge with per-agent results (Design A — as implemented in T-02)

**Scope-tier isolation (`MemoryScope`)** *(replaces MembershipFilter — see amendment below)*
- Five-tier hierarchy: `Global` → `Org` → `Team` → `User` → `Agent`
- Isolation by filesystem namespace: each scope tier maps to its own `.rvf` file
- `store_shared` and `recall_shared` accept an optional `scope` parameter; default is `Global` (backward-compatible)
- Multi-tenant queries fan out narrowest-to-widest and merge by score
- MembershipFilter dropped: vendor `MembershipFilter` is not wired into the `query()` path (private field on `RvfStore`, never consulted at query time); it cannot be used without forking vendor code

**Amendment (2026-03-20)**: original T-03 planned `MembershipFilter` policy bitmap. Investigation
revealed the vendor field is disconnected from query execution. The current use case is
single-user (multi-tenant is a future requirement). `MemoryScope` delivers tenant isolation without
vendor hacking and extends naturally when the multi-tenant tier is built. PLAN-002 T-03 updated.

**Adversarial detection**
- After every `query_with_envelope`, run `is_degenerate_distribution` on the distance array (uniform CV = query-poisoning signature)
- On detection: retry with wider probe; if still degenerate, return empty results + emit `MemoryEvent::AdversarialDetected`

**DoS hardening**
- `BudgetTokenBucket` per caller: 10,000 tokens/second; each recall costs `k × log2(total_vectors)` tokens
- `NegativeCache`: 3 consecutive `AdversarialDetected` events for the same query signature → blacklist for 60 seconds (immediate empty return, no HNSW touch)

**COW branch / snapshot**
- `freeze()` then `derive()` for read-only snapshots; `freeze()` then `branch()` for COW staging
- `freeze()` makes live store temporarily read-only (<1ms); COW promotion is a manual operator action

## Consequences

### If Phase 2 controls are adopted
**Positive**
- Shared store becomes safe for multi-tenant and adversarial environments
- All controls are feature-flagged — Phase 1 deployments are unaffected
- Degraded-quality fallback improves recall reliability under load
- `MemoryScope` tier hierarchy enables single-user → multi-tenant migration without architectural rework

**Negative**
- Adversarial detection, token buckets, and COW workflow add implementation complexity
- `freeze()` during branching makes shared store temporarily read-only — must be communicated to operators

**Neutral**
- `MemoryScope` is additive: single-user callers pass no scope argument (defaults to `Global`); existing behavior unchanged
- MembershipFilter removed from scope — vendor API is unwired and unsuitable without forking
- All Phase 2 calls are on `shared.rvf` (and scope-tier files) only — per-agent stores are unaffected
- Phase 1 shared store (`query_audited`) remains the deployed baseline

## Dependencies
- ADR-002 (design baseline — shared store architecture)
- ADR-003 (migration contract — must be Accepted and deployed)
- ADR-009: SONA self-learning (may interact with Phase 2 membership policy)

## Related
- ADR-003 §2.2 — original Phase 2 shared store specification (moved here)
- SPEC-001 §2.2 — all Phase 2 signatures and implementation details
