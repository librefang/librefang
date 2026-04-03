# ADR-018: Memory Export Authorization and Exfiltration Resistance

**Status**: Accepted
**Date**: 2026-03-21
**Authors**: Daniel Alberttis

---

## Context

The `Memory` trait (`openfang-types/src/memory.rs`) defines two maintenance methods:

```rust
async fn export(&self, format: ExportFormat) -> Result<Vec<u8>>;
async fn import(&self, data: &[u8], format: ExportFormat) -> Result<ImportReport>;
```

Both are currently stubs on `MemorySubstrate` — `export()` returns `Ok(vec![])` and `import()` returns an error string. This was intentional during Phase 1: no production caller exists, no API route is registered.

Before either method is implemented, the authorization model must be decided. The threat surface is significant:

**Threat 1 — Insider exfiltration**: An employee or operator with system access calls `export()` to dump all agent memory (conversations, business context, customer data, internal processes) before offboarding.

**Threat 2 — Compromised session exfiltration**: An attacker who has gained access to a user's session (stolen token, XSS, SSRF) calls `export()` to silently bulk-extract memory without triggering per-query rate limits.

**Threat 3 — Exfiltration via repeated recall**: An attacker who cannot reach the export endpoint uses repeated `recall()` calls with maximum `limit` to page through the entire memory store. No single call is anomalous; the pattern across calls is.

**Threat 4 — Data poisoning via import**: An attacker with write access calls `import()` with a crafted payload to inject false memories, override prior context, or cause embedding collisions in the RVF store.

**Comparison with OpenClaw**: OpenClaw uses a single-trusted-operator model with no per-user authorization. Its own threat model rates "Data Theft via web_fetch" as **HIGH** residual risk and "Session Data Extraction" as **medium** — no mitigations beyond filesystem permissions. OpenFang must not inherit these gaps.

**Existing primitives in OpenFang**:
- ADR-008 `BudgetTokenBucket` — per-caller rate limiting on recall
- ADR-008 `NegativeCache` — blacklist on repeated adversarial patterns
- ADR-008 `WitnessLog` — persistent audit trail per routing decision (ADR-010)
- ADR-008 `MemoryScope` — five-tier hierarchy isolating agent, user, team, org, global stores
- ADR-003 `query_with_envelope` — returns `ResponseQuality` on every shared recall

---

## Decision

### 1. `export()` is permanently capability-gated

`MemoryStore::export()` MUST NOT be callable without an explicit `memory:export` capability granted to the calling identity. The capability is:

- **Not granted by default** to any agent, user, or API token
- **Granted only by an administrator** via explicit config or token scope
- **Scoped to a specific agent_id** — a token with `memory:export` on agent A cannot export agent B
- **Absent from the default `KernelConfig`** — opt-in, not opt-out

Until a capability check is wired into the call path, `export()` MUST continue to return `Err` (the current stub behavior is correct).

### 2. Export scope is hard-bounded to the caller's own agent

Even when the capability is granted, `export()` MUST:

- Only serialize fragments where `agent_id == caller_agent_id`
- **Exclude all shared-store fragments** regardless of the caller's read permissions on the shared store
- **Exclude fragments from other agents** even if the caller can `recall_shared` from them

Shared-store data belongs to the organization, not the individual agent. Exporting it requires a separate `memory:export-shared` capability that is distinct and separately audited.

### 3. Every export attempt is audit-logged

Whether the call succeeds or is rejected, the following MUST be written to the audit log synchronously before returning:

```
ExportAuditEvent {
    caller_agent_id: AgentId,
    caller_identity: String,       // token ID or session key
    timestamp_ms: u64,
    granted: bool,                 // was the capability present?
    fragment_count: Option<u64>,   // only if granted
    format: ExportFormat,
    scope: ExportScope,            // "agent-only" | "shared" (future)
}
```

The audit log uses the same `WitnessLog` pattern from ADR-010 (dim=N RvfStore + content-map segment for durability).

### 4. Bulk recall anomaly detection (Threat 3)

The `recall()` path MUST track a rolling window per caller of `(call_count, total_fragments_returned)` over a configurable window (default: 60 seconds). When `total_fragments_returned` crosses a threshold (default: 500 fragments in 60 seconds):

- Rate-limit further `recall()` calls for that caller (return empty + `ResponseQuality::Unreliable`)
- Emit `MemoryEvent::ExfiltrationSuspected` to the event bus
- Write an audit entry with the rolling window stats

This catches paging-style exfiltration that bypasses the export endpoint entirely. The threshold is configurable in `KernelConfig` under `[memory.exfiltration_guard]`.

### 5. `import()` is blocked pending a data-poisoning ADR

`MemoryStore::import()` MUST remain a hard error until a dedicated poisoning-resistance ADR is written and accepted. The risks are more severe than export:

- Crafted embeddings can corrupt the HNSW graph in the RVF store
- Injected fragments with high confidence scores can override genuine memories in recall ranking
- Large payloads can exhaust storage or cause SQLite write amplification

The stub error message MUST be updated to: `"Memory import is disabled pending ADR on data poisoning resistance (see ADR-018)"`.

### 6. No export API route until authorization is implemented

No HTTP route (`POST /api/agents/{id}/memory/export`) may be registered in `server.rs` until items 1–3 of this ADR are implemented and all acceptance criteria pass. The route registration is the deployment gate.

---

## Consequences

### Positive
- Memory stays inside the trust boundary by default — no accidental data leakage via an unguarded API
- Audit trail makes insider exfiltration detectable after the fact, even if not preventable
- Bulk recall anomaly detection catches the more common paging attack vector
- Export scope isolation (agent-only, no shared-store bleed) matches the principle of least privilege
- `import()` hard-block prevents a class of attacks that OpenClaw explicitly flagged as high-risk

### Negative
- No agent portability / backup feature until the authorization layer is built
- Legitimate operator backup workflows require implementing the capability system first
- Bulk recall threshold may generate false positives for high-traffic legitimate agents (threshold must be tunable)

### Neutral
- The `MemoryStore` trait signature does not change — only the implementation behavior changes
- OpenClaw migration (`openfang-migrate/src/openclaw.rs`) is unaffected — it operates at the filesystem level, not the `MemoryStore` trait

---

## Alternatives Considered

**A — Implement export with no auth, document as "operator responsibility"**
Rejected. OpenClaw took this approach; their own threat model rates exfiltration as HIGH residual risk. Passing responsibility to the operator is not acceptable for a system that handles business-critical agent memory.

**B — Implement export scoped to full memory including shared store**
Rejected. Shared-store data is cross-agent organizational knowledge. An individual agent exporting it would bypass the scope isolation guarantees in ADR-008.

**C — Use filesystem-level export (copy SQLite + RVF files)**
Rejected as the primary mechanism. Filesystem copies bypass all audit logging and capability checks. May be offered as an out-of-band disaster-recovery option by administrators with direct host access, but not via the API.

**D — Block import permanently**
Considered. Kept as "blocked pending a dedicated ADR" rather than permanently banned — a well-designed import with schema validation, rate limiting, and PII stripping has legitimate uses (agent cloning, disaster recovery). The architecture should not foreclose it, but the threat model must be addressed first.

---

## Implementation Gate

This ADR is **accepted but not yet implemented**. No code changes are required now. Implementation begins when a PLAN is written that references this ADR. The following must be true before the implementation plan closes:

- [ ] `memory:export` capability type defined in `openfang-types`
- [ ] Capability check wired into `MemorySubstrate::export()` — rejects without it
- [ ] `ExportAuditEvent` written on every call (success and rejection)
- [ ] Export scope bounded to caller's `agent_id`, shared-store fragments excluded
- [ ] `import()` stub error updated to reference this ADR
- [ ] Bulk recall anomaly detector implemented under `[memory.exfiltration_guard]` config key
- [ ] No HTTP export route registered until all above pass
- [ ] `cargo test --workspace` exit zero, clippy exit zero

---

## Related ADRs

- **ADR-003** — Memory store implementation (defines `MemorySubstrate`)
- **ADR-008** — Shared store abuse resistance (`BudgetTokenBucket`, `NegativeCache`, `MemoryScope`)
- **ADR-010** — `WitnessLog` audit pattern (audit log implementation reference)
- **ADR-011** — RuVix interface contract (typed message boundaries for cross-agent data)
