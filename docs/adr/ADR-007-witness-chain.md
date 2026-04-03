# ADR-007: Memory Witness Chain and Session Audit Policy

**Status**: Accepted
**Date**: 2026-03-15
**Authors**: Daniel Alberttis

## Version History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 1.0 | 2026-03-15 | Daniel Alberttis | Initial decision record. Session witness chain policy, event table, `create_witness_chain()` confirmation, WITNESS_SEG embedding approach, wire-format conflict note (per-query vs. session-chain WITNESS_SEGs). |
| 1.1 | 2026-03-17 | ruvector-upstream sync | `RvfWitnessHeader` wire format now defined upstream in `rvf_bridge.rs`; DECISION added on canonical on-disk audit record format (RvfWitnessHeader 64B vs ProofAttestation 82B); ADR-011 Constraint 1 cross-reference added. |

## Context

ADR-003 (Memory Store Implementation) specifies that the shared store uses `query_audited`, which produces per-recall SHAKE256 witness hashes. That answers "did this query happen?" but not "what did this agent do with memory across the full session and in what order?"

The question is whether to add a session-level causal witness chain for per-agent memory operations, embedded into the `.rvf` file at session end.

`query_audited` for the shared store is baseline behavior and stays in ADR-003. This ADR covers only the additional session-level chain for per-agent stores.

## Decision

**Note**: ADR-003 v1.8 resolved this in favour of adopting session witness chains, with the full event table and embedding approach specified in §9. This ADR is the standalone decision record.

ADR-003 Phase 1 does not include session witness chains (Phase 1c addition). This ADR governs the acceptance gate for that addition. The shared store `query_audited` baseline remains in ADR-003.

If session witness chains are adopted:
- `audit.rs` maintains a session-scoped `Vec<WitnessEntry>` per active agent session
- On `SessionEnd` (or `store.close()`), `create_witness_chain(&entries)` produces chained bytes embedded as `WITNESS_SEG` in the agent `.rvf` file
- `create_witness_chain()` is confirmed present in `vendor/rvf/rvf-crypto/src/witness.rs`
- Events recorded (in session order):

| Event | `witness_type` | `action_hash` content |
|-------|:--------------:|----------------------|
| `SessionStart { agent_id, session_id, timestamp_ns }` | `0x01` | SHAKE256 of agent_id bytes |
| `VectorStored { vec_id, scope, content_hash }` | `0x02` | SHAKE256 of vec_id LE bytes |
| `VectorRecalled { query_hash, k_returned, avg_distance }` | `0x02` | SHAKE256 of query embedding bytes |
| `VectorForgotten { vec_id }` | `0x02` | SHAKE256 of vec_id LE bytes |
| `SessionEnd { turn_count, recall_count }` | `0x01` | SHAKE256 of session_id bytes |

- Additive — does not replace per-query `query_audited` on `shared.rvf`
- If `store.embed_witness_chain()` is absent from vendored `rvf-runtime`, a minor fork following the CONTENT_MAP_SEG pattern (~35 lines) is sufficient

## Consequences

### If session witness chains are adopted
**Positive**
- Enables forensic replay of full agent memory interaction sequence directly from `.rvf` at rest
- Does not require external logs
- Additive — no regression to shared store `query_audited`

**Negative**
- Additional chain construction overhead on `SessionEnd` — proportional to operations per session
- May duplicate information already captured by `AuditLog` — value vs. overhead tradeoff must be evaluated
- If `embed_witness_chain()` is absent, requires an additional minor fork of `rvf-runtime`

**Neutral**
- `query_audited` for shared store is unaffected — continues to fire for every cross-agent recall

## Dependencies
- ADR-003 (migration contract — must be Accepted first)
- ADR-004 (content durability fork) — if ADR-004 Option 2 is implemented, both ADR-004 and this ADR require changes to `vendor/rvf/rvf-runtime/src/write_path.rs`. ADR-004 should be resolved first to clarify fork scope before this ADR's fork is applied.

## Related
- ADR-003 §9 — original session witness chain specification (moved here)
- SPEC-001 §9 — `SessionAuditBuffer` struct and engine.rs wiring

---

## Amendment 1.1: ruvector-upstream Sync — 2026-03-17

### 1. `RvfWitnessHeader` wire format now defined in ruvector-upstream

**Source**: `crates/rvAgent/rvagent-core/src/rvf_bridge.rs`

`RvfWitnessHeader` is the rvAgent-side representation of the RVF witness bundle header, designed to serialize to and deserialize from a fixed 64-byte little-endian wire format. The type and its serialization methods (`to_bytes()` / `from_bytes()`) are now present in ruvector-upstream.

**Constants.**
- `WITNESS_MAGIC: u32 = 0x5257_5657` — serializes as the bytes `[57, 56, 57, 52]` in LE, spelling `"RVWW"` when read as ASCII.
- `WITNESS_HEADER_SIZE: usize = 64` — total header size in bytes.

**Wire layout (all multi-byte fields are little-endian).**

| Byte range | Field | Type | Notes |
|-----------|-------|------|-------|
| `[0..4]` | magic | `u32` | `WITNESS_MAGIC = 0x5257_5657` |
| `[4..6]` | version | `u16` | Currently `1` |
| `[6..8]` | flags | `u16` | Bitfield; see flag constants below |
| `[8..24]` | task_id | `[u8; 16]` | UUID bytes (16B) |
| `[24..32]` | policy_hash | `[u8; 8]` | SHA-256 of policy, truncated to 8 bytes |
| `[32..40]` | created_ns | `u64` | Nanoseconds since UNIX epoch |
| `[40]` | outcome | `u8` | `TaskOutcome` discriminant |
| `[41]` | governance_mode | `u8` | `GovernanceMode` discriminant |
| `[42..44]` | tool_call_count | `u16` | Number of tool calls recorded |
| `[44..48]` | total_cost_microdollars | `u32` | Aggregate cost in microdollars |
| `[48..52]` | total_latency_ms | `u32` | Aggregate wall-clock latency in ms |
| `[52..56]` | total_tokens | `u32` | Aggregate token count |
| `[56..58]` | retry_count | `u16` | Number of retries |
| `[58..60]` | section_count | `u16` | Number of TLV sections following the header |
| `[60..64]` | total_bundle_size | `u32` | Total bundle size in bytes |

**Flag constants.**
- `WIT_SIGNED: u16 = 0x0001` — bundle carries a detached ML-DSA-65 signature.
- `WIT_HAS_SPEC: u16 = 0x0002` — a spec section is present.
- `WIT_HAS_PLAN: u16 = 0x0004` — a plan section is present.
- `WIT_HAS_TRACE: u16 = 0x0008` — a trace section is present.
- `WIT_HAS_DIFF: u16 = 0x0010` — a diff section is present.
- `WIT_HAS_TEST_LOG: u16 = 0x0020` — a test log section is present.

**`TaskOutcome` enum (stored at byte `[40]`).**
- `Solved = 0` — task completed with passing tests.
- `Failed = 1` — task attempted but tests fail.
- `Skipped = 2` — task skipped (precondition not met).
- `Errored = 3` — task errored (infrastructure failure).

**`GovernanceMode` enum (stored at byte `[41]`).**
- `Restricted = 0` — read-only plus suggestions.
- `Approved = 1` — writes allowed with human confirmation gates (default).
- `Autonomous = 2` — bounded authority with automatic rollback.

**Round-trip guarantee.** `RvfWitnessHeader::to_bytes()` followed by `from_bytes()` is verified to reconstruct the original struct exactly. `from_bytes()` rejects data shorter than 64 bytes (`"data too short for witness header"`) and rejects any magic value other than `WITNESS_MAGIC` (`"invalid witness magic bytes"`).

**Relation to this ADR.** This ADR specifies that session witness chains are embedded as `WITNESS_SEG` in the `.rvf` file on `store.close()` / `SessionEnd`. The `RvfWitnessHeader` format is the canonical header for any such bundle emitted by the rvAgent layer. OpenFang's `audit.rs` and `engine.rs` session-end paths must produce headers that round-trip correctly against these constants if they are to be readable by `rvf verify-witness` or any downstream rvAgent tooling.

### 2. ⚠️ DECISION: Canonical on-disk audit record format for the OpenFang memory witness chain

**Context.** Two header formats now exist that could serve as the canonical on-disk audit record written by OpenFang's memory witness chain:

- **`RvfWitnessHeader`** (`crates/rvAgent/rvagent-core/src/rvf_bridge.rs`): 64 bytes, magic `WITNESS_MAGIC = 0x5257_5657` (`"RVWW"`), all fields listed above, round-trips correctly, available now with no additional crate dependency beyond what rvAgent already provides.
- **`ProofAttestation`** (`ruvix-types` crate): 82 bytes, referenced in ADR-011 Constraint 1. The `ruvix-types` crate dependency is heavier than `rvagent-core` and is not currently vendored in OpenFang.

These are distinct structs with different sizes, different magic bytes (if any), and potentially different field layouts. They cannot be parsed interchangeably by the same reader. ADR-011 Constraint 1 currently states that `ProofAttestation` (82B) is the audit record format. That constraint conflicts with the `RvfWitnessHeader` (64B) format that has landed in ruvector-upstream.

**Options.**

- **Option A — Use `RvfWitnessHeader` (64B, `RVWW`)**: Adopt the upstream-defined format for all new witness records written by OpenFang. ADR-011 Constraint 1 must be amended to remove the `ProofAttestation` (82B) reference and replace it with `RvfWitnessHeader` (64B). No `ruvix-types` dependency required for Phase 1.
- **Option B — Use `ProofAttestation` (82B) from `ruvix-types`**: Preserve ADR-011 Constraint 1 as written. Requires vendoring or depending on `ruvix-types`. Phase 1 implementation is blocked until that dependency is available.
- **Option C — Use `RvfWitnessHeader` for Phase 1, migrate to `ProofAttestation` for Phase 2**: Phase 1 witness records use `RvfWitnessHeader` (64B). When Phase 2 RuVix integration begins and `ruvix-types` is available, audit records are migrated to `ProofAttestation` (82B). Requires a versioned reader that handles both formats or a one-time migration of existing witness data.

**Impact on ADR-011.** Regardless of which option is chosen, ADR-011 Constraint 1 requires a corresponding amendment to resolve the conflict with the format that lands in OpenFang. This ADR does not amend ADR-011 directly — that amendment must be made separately and must reference this decision.

**Decision needed before implementing the witness chain write path in `audit.rs` or `engine.rs`.**
