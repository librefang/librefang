# ADR-011: RuVix Interface Contract

**Status**: Draft
**Date**: 2026-03-15
**Authors**: Daniel Alberttis

## Version History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 0.1 | 2026-03-15 | Daniel Alberttis | Initial binding interface contract. Documents Daxiom's position in the RuVix architecture stack, Phase A status, and three forward-compatibility constraints governing proof chain format, coherence score dimensions, and inter-agent communication. |
| 0.2 | 2026-03-15 | Daniel Alberttis | Sherlock correction: Constraint 2 rewritten with actual `CoherenceMeta` struct (5 fields: `coherence_score`, `mutation_epoch`, `proof_attestation_hash`, `last_access_ns`, `access_count`). Removed false claim that `SchedulerScore` consumes `CoherenceMeta` — it uses `deadline_urgency + novelty_boost - risk_penalty`. Fixed primitives list from crate names (Region/Queue/Cap/Proof/Sched/VecGraph) to actual primitives (Task/Capability/Region/Queue/Timer/Proof). AQE correction: `WitTypeId` is `u32` not `u64`; updated source file reference to `src/rvf.rs`. AQE source scan: `ranking.rs` and `MemoryFragment.score` are SPEC-001 deliverables (not yet in codebase); `SharedAuditBridge` does not yet exist (current `AuditLog` uses SHA-256); `rvf-crypto::WitnessEntry` is 73 bytes (not 82) — Constraint 1 rewritten to use correct `ProofAttestation` (82-byte) format from `ruvix-types`. |
| 0.3 | 2026-03-17 | ruvector-upstream sync | Concrete implementations of `WitTypeId`, `GovernanceMode`, `PolicyCheck`, and `RvfWitnessHeader` have landed in `rvagent-core`. Constraints 1 and 3 updated with precise source citations. Open decision surfaced for Constraint 1 (`RvfWitnessHeader` 64B vs `ProofAttestation` 82B). Phase A status note: `rvAgent` 8-crate structure landed 2026-03-14 to 2026-03-16 as reference implementation of ADR-106 Layer 1/2/3. |

---

## Context

Daxiom is a fork of OpenFang v0.4.0 that replaces the `SemanticStore` vector backend with `rvf-runtime`, a userspace Rust library from `ruvector-upstream/crates/rvf/rvf-runtime/`. The `rvf-runtime` crate is one layer of a broader system: the RuVix Cognition Kernel, specified in `ruvector-upstream/docs/adr/ADR-087-ruvix-cognition-kernel.md`.

ADR-087 defines a four-layer architecture. Daxiom currently operates in the **RVF Component Space** layer and does not call RuVix syscalls. As the project evolves into Phase 2 and beyond, Daxiom's memory store, agent coordination, and audit chain will need to be mountable into a RuVix kernel without a rewrite. The decisions made today in Phase 1 either enable or foreclose that path.

This ADR is a binding interface contract. It does three things: places Daxiom explicitly in the RuVix stack, records the factual Phase A completion state, and states three constraints that Phase 2+ design must not violate. It does not require any changes to Phase 1 (SPEC-001 / PLAN-001 are already compliant with all three constraints).

### Daxiom's position in the RuVix stack

ADR-087 defines four layers:

```
AGENT CONTROL PLANE       ← Daxiom agents, Claude Code
RVF COMPONENT SPACE       ← Daxiom SemanticStore, rvf-runtime  (Daxiom Phase 1 lives here)
RUVIX COGNITION KERNEL    ← Linux-hosted nucleus (Phase A: complete), AArch64 (Phase B: in progress)
HARDWARE / HYPERVISOR     ← Linux (Phase A), AArch64 bare metal (Phase B), Cognitum (future)
```

Daxiom Phase 1 operates entirely in the RVF Component Space layer. `rvf-runtime` is a userspace Rust library — it provides `RvfStore`, `HnswIndex`, and the `ingest_batch` / `query` / `delete` surface. It does not issue RuVix syscalls. This boundary is intentional and permanent for Phase 1.

The relevant code path: `crates/openfang-memory/src/store.rs` → `RvfStore` (from `rvf-runtime`) → HNSW index on disk as `.rvf` files. No kernel involvement at any point in this path.

### RuVix Phase A status

ADR-087 divides kernel development into phases. Phase A (Linux-hosted nucleus) is complete as of the ADR-087 record date. The nine crates and their test counts are:

| Crate | Tests |
|-------|-------|
| `ruvix-types` | 63 |
| `ruvix-region` | 51 |
| `ruvix-queue` | 47 |
| `ruvix-cap` | 54 |
| `ruvix-proof` | 73 |
| `ruvix-sched` | 39 |
| `ruvix-boot` | 59 |
| `ruvix-vecgraph` | 55 |
| `ruvix-nucleus` | 319 |
| **Total** | **760** |

Phase B (bare metal AArch64, targeting QEMU virt and Raspberry Pi 4/5) is in progress. Cognitum hardware integration follows Phase B. No special hardware is required for Phase A — it runs on standard Linux. Daxiom does not depend on Phase B or Cognitum.

The six kernel primitives defined in Phase A are: Task, Capability, Region, Queue, Timer, Proof. Of these, three kernel crates are directly relevant to this contract: `ruvix-proof` (attestation format), `ruvix-vecgraph` (coherence metadata), and `ruvix-queue` (IPC).

### Why this ADR exists now

ADR-003 defines what to build for Phase 1. ADR-009 and ADR-010 define Phase 2 capabilities (SONA integration and LLM intelligence). Neither of those ADRs is constrained by kernel compatibility today, but both will introduce design choices that are difficult to reverse. The three constraints below are intended to prevent Phase 2 designs from accumulating kernel-incompatible technical debt. Recording them now, while Phase 1 is still being built, costs nothing — Phase 1 is already compliant. Waiting until Phase 2 design begins would require retrofit analysis.

---

## Decision

Three binding constraints govern all Phase 2+ design. SPEC-001 is designed to produce output that satisfies all three constraints; no changes to SPEC-001's design are required. The artifacts described (e.g., `ranking.rs`, `MemoryFragment.score`, `SharedAuditBridge`) are SPEC-001 Phase 1 deliverables — they do not yet exist in the pre-Daxiom codebase.

### Constraint 1 — Proof chain compatibility

SPEC-001 Phase 1 will create `SharedSemanticStore` with a `query_audited` method that appends witness entries to a new `SharedAuditBridge` module (does not yet exist in the pre-Daxiom codebase — the current `openfang-runtime/src/audit.rs` is a separate SHA-256 agent audit log). The `SharedAuditBridge` must produce `ProofAttestation` entries in the 82-byte format defined by `ruvix-types`.

The 82-byte layout is serialized by `ruvix-proof`'s `WitnessLog::serialize_attestation()`:

```
[0..32]   proof_term_hash      ([u8; 32])
[32..64]  environment_hash     ([u8; 32])
[64..72]  verification_timestamp_ns (u64 LE)
[72..76]  verifier_version     (u32 LE)
[76..80]  reduction_steps      (u32 LE)
[80..82]  cache_hit_rate_bps   (u16 LE)
          ─────────────────────────────
          Total: 82 bytes  (ATTESTATION_SIZE in ruvix-types)
```

The constant is defined in `ruvector-upstream/crates/ruvix/crates/types/src/lib.rs`:

```
ATTESTATION_SIZE = 82
```

`ruvix-proof`'s `WitnessLog` (in `ruvector-upstream/crates/ruvix/crates/proof/src/attestation.rs`) appends `ProofAttestation` structs via `WitnessLog::append(attestation, tier)`. When a Daxiom `.rvf` store is eventually mounted into a RuVix kernel via `rvf_mount`, the kernel's `WitnessLog` must be able to reconstruct the Daxiom-produced attestation records. If those records do not follow the 82-byte `ProofAttestation` layout, the audit chain breaks at mount time.

Note: `rvf-crypto`'s `WitnessEntry` (in `vendor/rvf/rvf-crypto/src/witness.rs`) is a different 73-byte format used for RVF segment witness chains. It is not the format consumed by `ruvix-proof`'s `WitnessLog`. `rvf-crypto`'s witness module uses SHAKE256 for hash chain linking between its 73-byte entries. `ruvix-proof`'s `WitnessLog` is a circular buffer of `ProofAttestation` structs — it does not do hash chain linking; each entry is appended independently via `append(attestation, tier)`. The entry format for `SharedAuditBridge` must be `ProofAttestation` (82-byte), not `WitnessEntry` (73-byte).

**Binding rule**: SPEC-001's `SharedAuditBridge` must produce `ProofAttestation` records in the 82-byte layout from `ruvix-types`. No Phase 2+ change may alter this byte layout. Any Phase 2 audit enhancement (e.g., richer metadata in witness records) must be additive within the payload fields defined by `ruvix-types`, not in new fields outside the 82-byte structure. Any bypass of `query_audited` that avoids appending witness entries is prohibited — the audit chain must be unbroken for every shared recall.

### Constraint 2 — Coherence scoring alignment

SPEC-001 Phase 1 will create `ranking.rs` and add a `score` field to `MemoryFragment` (neither exists in the pre-Daxiom codebase — the current `MemoryFragment` struct has `confidence: f32` and ranking is done inline in `semantic.rs`). The `score` will be a blended value from three inputs: cosine similarity (from the HNSW query result's `distance` field), `access_count` (from the `.access.db` side-store), and `last_accessed` timestamp (from the `.access.db` side-store). Two of these three inputs have direct equivalents in the kernel's `CoherenceMeta` type and must remain in the score.

`ruvix-vecgraph`'s `CoherenceTracker` (in `ruvector-upstream/crates/ruvix/crates/vecgraph/src/lib.rs`) tracks per-vector kernel metadata using `CoherenceMeta`, defined in `ruvix-types`:

- `coherence_score: u16` — 0-10000 representing 0.0–1.0 structural graph consistency (not cosine similarity)
- `mutation_epoch: u64` — incremented on each proof-gated write
- `proof_attestation_hash: [u8; 32]` — hash of the authorizing Proof token
- `last_access_ns: u64` — nanoseconds since boot (kernel clock)
- `access_count: u32`

The kernel-tracked fields `access_count` and `last_access_ns` map directly to Daxiom's `access_count` and `last_accessed` scoring inputs. Cosine similarity has no direct kernel equivalent — `coherence_score` measures structural graph consistency, not vector distance. When Daxiom's store is promoted to a kernel object via `rvf_mount`, the kernel will populate these fields; Daxiom's score computation must remain derivable from kernel-managed metadata.

If Phase 2 introduces a `MemoryFragment.score` computation that uses additional inputs — e.g., a SONA-specific relevance dimension, a recency decay curve, or a per-agent affinity weight — those inputs have no kernel-managed equivalent and cannot be maintained after promotion.

**Binding rule**: Phase 2 SONA integration (ADR-009) must express coherence improvements as adjustments to the kernel-mappable inputs — not by adding new scoring dimensions to `MemoryFragment.score`. For example, SONA may improve the quality of the similarity estimate (by using a better embedding or a re-ranking step) or may adjust the `access_count` weighting coefficient, but the final score must remain derivable from `{cosine_similarity, access_count, last_accessed}`, where `access_count` maps to `CoherenceMeta.access_count` and `last_accessed` maps to `CoherenceMeta.last_access_ns`. Any Phase 2 scoring change must be accompanied by a mapping table identifying which `CoherenceMeta` field each scoring input corresponds to.

### Constraint 3 — Queue-first inter-agent communication

All Phase 2+ inter-agent communication in Daxiom must use typed Queue semantics. The following communication patterns are prohibited across agent boundaries:

- Shared mutable state accessed directly by multiple agents (e.g., `Arc<Mutex<T>>` accessed from agent task bodies without a Queue intermediary)
- Direct method calls from one agent's execution context into another agent's internal state
- Untyped byte channels (`Vec<u8>` or raw `Bytes` messages without a schema-bearing wrapper type)

`ruvix-queue` defines the `queue_send` / `queue_recv` syscall pair as the sole IPC primitive in the RuVix kernel. When Daxiom agents are eventually hosted as kernel Tasks, every inter-agent communication must go through a Queue. The `WitTypeId` type (`pub struct WitTypeId(pub u32)`, in `ruvector-upstream/crates/ruvix/crates/types/src/rvf.rs`) allows Queue messages to carry a WIT (WASM Interface Types) schema ID that the kernel validates at `queue_recv` time. Messages without a `WitTypeId` cannot be kernel-validated.

Phase 2 designs that communicate via ad-hoc channels or shared state are not wrong at the Rust level — they will compile and run. But they represent a communication topology that has no kernel-level equivalent. Rewriting that topology at kernel-mount time is not additive work; it is a structural rewrite of the agent communication model.

**Binding rule**: Phase 2 agent-to-agent message passing must be modeled as typed message structs implementing a stable interface, passed through a channel type that can be mechanically mapped to `queue_send` / `queue_recv` at kernel-mount time (e.g., `tokio::sync::mpsc` with a typed message enum is acceptable; `Arc<Mutex<SharedState>>` accessed directly from two agent contexts is not). Phase 2 message types should carry a type identifier compatible with `WitTypeId` — a `u32` schema ID (`pub struct WitTypeId(pub u32)`) that can be assigned from a WIT definition — even if kernel validation is not active in Phase 2.

---

## What This ADR Does Not Do

- It does not specify how or when Daxiom's store will be mounted into a RuVix kernel. That is a future ADR.
- It does not require any changes to SPEC-001 or PLAN-001. SPEC-001's design is already aligned with all three constraints — no rework required.
- It does not require Daxiom to call any RuVix syscalls at any phase.
- It does not block tasks T-01 through T-14 in PLAN-001.
- It does not supersede ADR-009 or ADR-010. Those ADRs remain authoritative for Phase 2 capabilities. This ADR adds forward-compatibility constraints on their implementation.

---

## Consequences

### Positive

- SPEC-001 is designed to satisfy all three constraints. No design changes to SPEC-001 are needed; the constraints confirm that the chosen design is already kernel-compatible. The artifacts (`ranking.rs`, `MemoryFragment.score`, `SharedAuditBridge`) are Phase 1 build targets, not pre-existing code.
- Daxiom's `.rvf` files are written by `rvf-runtime` in the format that `rvf_mount` expects. No format changes are needed between Phase 1 and a future kernel mount.
- Proof-gated writes in SPEC-001 (`query_audited` + `SharedAuditBridge`) mirror the RuVix attestation model exactly. Phase 2 audit work builds on a compatible foundation rather than replacing an incompatible one.
- SPEC-001's `ranking.rs` scoring inputs `access_count` and `last_accessed` are designed to map directly to `CoherenceMeta.access_count` and `CoherenceMeta.last_access_ns`. Phase 2 SONA scoring changes can be validated against kernel field constraints using the existing `ruvix-vecgraph` test suite without cross-system reconciliation.

### Negative

- Phase 2 SONA integration (ADR-009) is constrained in its scoring model. Any SONA-derived relevance signal that cannot be expressed as an adjustment to the kernel-mappable inputs `{cosine_similarity, access_count, last_accessed}` must be discarded or carried as out-of-band metadata that does not influence `MemoryFragment.score`.
- Phase 2 shared memory access patterns must be Queue-based. This is additional design work at the start of Phase 2 compared to a simpler `Arc<Mutex<T>>` design. The payoff is zero rewrite cost at kernel-mount time.
- Any caching or read-path shortcut that bypasses `query_audited` for shared store recalls is prohibited. The audit chain must be complete. If Phase 2 introduces a cache layer in front of the shared store, cache hits must still append witness entries via `SharedAuditBridge`.
- The `ProofAttestation` 82-byte layout (from `ruvix-types::ATTESTATION_SIZE`) is frozen for `SharedAuditBridge` witness entries. If a future security audit recommends changes to the attestation format, the change requires explicit coordination with `ruvix-types` and cannot be made unilaterally in the Daxiom codebase.

### Neutral

- `ruvix-proof`, `ruvix-vecgraph`, `ruvix-queue`, and `ruvix-types` are read-only references from Daxiom's perspective. This ADR does not require any changes to those crates.
- The constraints apply to Phase 2+ design. No Phase 1 code is changed by accepting this ADR.

---

## Related

- **ADR-002** — dual-layer architecture (`{agent_id}.rvf` + `shared.rvf`); established the `rvf-runtime` dependency and the `SharedAuditBridge` audit path that Constraint 1 fixes
- **ADR-003** — memory store implementation contract; the `ranking.rs` scoring model (Constraint 2) and `query_audited` audit path (Constraint 1) are specified there
- **ADR-009** — memory intelligence / SONA integration (Phase 2); Phase 2 coherence scoring must comply with Constraint 2
- **ADR-010** — LLM intelligence layer (Phase 2); Phase 2 agent communication must comply with Constraint 3
- **[ruvector-upstream/docs/adr/ADR-087-ruvix-cognition-kernel.md]** — RuVix specification: four-layer architecture, six kernel primitives, Phase A/B breakdown, 760-test completion status
- **[ruvector-upstream/crates/ruvix/crates/types/src/lib.rs]** — `ATTESTATION_SIZE = 82`, `WitTypeId`, `CoherenceMeta`, and all kernel interface types shared across crates
- **[ruvector-upstream/crates/ruvix/crates/proof/src/attestation.rs]** — `WitnessLog`, `AttestationBuilder`, `serialize_attestation()` producing the 82-byte `ProofAttestation` layout; `lib.rs` re-exports these
- **[ruvector-upstream/crates/ruvix/crates/vecgraph/src/lib.rs]** — `CoherenceTracker`, `CoherenceMeta` (fields: `coherence_score: u16`, `mutation_epoch: u64`, `proof_attestation_hash: [u8; 32]`, `last_access_ns: u64`, `access_count: u32`)
- **[ruvector-upstream/crates/ruvix/crates/sched/src/lib.rs]** — `SchedulerScore` (`score = deadline_urgency + novelty_boost - risk_penalty`; independent of `CoherenceMeta`)
- **[ruvector-upstream/crates/ruvix/crates/queue/src/lib.rs]** — `queue_send` / `queue_recv`, the sole IPC primitive in the kernel
- **[docs/specs/SPEC-001-memory-store.md]** — Phase 1 implementation contract; already compliant with all three constraints
- **[docs/plans/PLAN-001-memory-store-phase1.md]** — Phase 1 execution plan; T-01 through T-14 are unblocked by this ADR

---

## Amendment 0.3: ruvector-upstream Sync — 2026-03-17

This amendment tightens Constraints 1 and 3 against concrete implementations that have landed in `ruvector-upstream` since version 0.2. It surfaces one open decision that must be resolved before Phase 2 design work begins. No existing constraint text is altered; this section is additive.

Source files examined:

- `crates/rvAgent/rvagent-core/src/rvf_bridge.rs` (primary)
- `crates/rvAgent/rvagent-wasm/src/rvf.rs` (secondary)

---

### Amendment 0.3.1 — Constraint 3: `WitTypeId` now has a concrete implementation

Constraint 3 previously cited `WitTypeId` as `pub struct WitTypeId(pub u32)` in `ruvector-upstream/crates/ruvix/crates/types/src/rvf.rs`. That reference remains valid. However, a second live implementation of the same type now exists in the rvAgent layer: `rvagent-core/src/rvf_bridge.rs`, lines 150–175.

The concrete definition in `rvf_bridge.rs`:

```rust
/// WIT (WASM Interface Types) type identifier for message schema validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct WitTypeId(pub u32);

impl WitTypeId {
    /// No schema (raw bytes).
    pub const NONE: Self = Self(0);

    pub const fn new(id: u32) -> Self { Self(id) }

    pub const fn is_none(&self) -> bool { self.0 == 0 }
}

impl Default for WitTypeId {
    fn default() -> Self { Self::NONE }
}
```

`WitTypeId::NONE = Self(0)` is the sentinel value meaning "no schema / raw bytes." Any message carrying `WitTypeId::NONE` cannot be kernel-validated at `queue_recv` time; it is explicitly a no-schema signal.

In `rvagent-wasm/src/rvf.rs`, the `agi_tags` module (lines 57–65) defines concrete `u16` tag constants for AGI message types used in the RVF container format:

```rust
pub mod agi_tags {
    pub const TOOL_REGISTRY:    u16 = 0x0105;
    pub const AGENT_PROMPTS:    u16 = 0x0106;
    pub const SKILL_LIBRARY:    u16 = 0x0109;
    pub const ORCHESTRATOR:     u16 = 0x0108;
    pub const MIDDLEWARE_CONFIG: u16 = 0x010A;
    pub const MCP_TOOLS:        u16 = 0x010B;
    pub const CAPABILITY_SET:   u16 = 0x010C;
}
```

These `u16` tag values are the typed schemas that Constraint 3 refers to. They are carried in the RVF segment header's `tag` field alongside each segment's `SegmentType`. In the context of `WitTypeId`, these values would be widened to `u32` (the upper 16 bits would be zero in the `0x01xx` range).

**Updated binding rule for Constraint 3 (additive)**: Any Phase 2 inter-agent message passing in OpenFang-AI must use `WitTypeId` sourced from `rvagent-core/src/rvf_bridge.rs` (or the kernel-equivalent in `ruvix-types`). Messages must not carry `WitTypeId::NONE` unless they are genuinely untyped bulk payloads with no schema validation requirement — and such messages are prohibited across agent trust boundaries. When defining new OpenFang-AI message schemas, either (a) reuse an `agi_tags` constant cast to `u32`, if the message type maps to a standard AGI container segment, or (b) define an `openfang_tags` module following the same `u16` constant pattern (`0x02xx` range is unallocated by `agi_tags`) and widen to `u32` for `WitTypeId`. The previous rule requiring type-compatible channel design (e.g., `tokio::sync::mpsc` with typed enum) remains in force.

---

### Amendment 0.3.2 — Constraint 3: `GovernanceMode` and `PolicyCheck` now concrete

`GovernanceMode` and `PolicyCheck` are now defined in `rvagent-core/src/rvf_bridge.rs` (lines 210–268). The concrete definitions:

```rust
#[repr(u8)]
pub enum GovernanceMode {
    Restricted = 0,  // Read-only plus suggestions
    Approved   = 1,  // Writes allowed with human confirmation gates
    Autonomous = 2,  // Bounded authority with automatic rollback
}

impl Default for GovernanceMode {
    fn default() -> Self { Self::Approved }
}
```

```rust
#[repr(u8)]
pub enum PolicyCheck {
    Allowed   = 0,  // Tool call allowed by policy
    Denied    = 1,  // Tool call denied by policy
    Confirmed = 2,  // Tool call required human confirmation
}

impl Default for PolicyCheck {
    fn default() -> Self { Self::Allowed }
}
```

Both implement `TryFrom<u8>` and are carried in `RvfWitnessHeader` (see Amendment 0.3.3 below).

`RvfBridgeConfig` (lines 504–537) sets the default governance mode:

```rust
pub struct RvfBridgeConfig {
    pub enabled: bool,           // default: false
    pub package_dir: Option<String>,
    pub verify_signatures: bool, // default: true
    pub rvf_witness: bool,       // default: false
    pub governance_mode: GovernanceMode, // default: GovernanceMode::Approved
}
```

`RvfBridgeConfig.enabled = false` by default. OpenFang-AI Phase 1 does not enable rvf-compat features, which is correct — Phase 1 operates in the RVF Component Space layer and the bridge is opt-in.

`RvfBridgeConfig.governance_mode` defaults to `GovernanceMode::Approved`. For OpenFang-AI Phase 1 (RVF Component Space), `Approved` is the correct governance mode: writes are allowed but require human confirmation gates. This is consistent with the `query_audited` + `SharedAuditBridge` model in Constraint 1.

**Binding rule**: Any OpenFang-AI component that interacts with `RvfBridgeConfig` (either directly or by constructing an equivalent config struct) must use `GovernanceMode::Approved` as the default for Phase 1. Promotion to `GovernanceMode::Autonomous` requires explicit justification in a future ADR. `GovernanceMode::Restricted` (read-only) may be used for read-only query paths where write confirmation gates are not applicable. No OpenFang-AI component may set `governance_mode` to `Autonomous` without a recorded decision.

---

### Amendment 0.3.3 — Constraint 1: `RvfWitnessHeader` vs `ProofAttestation` — OPEN DECISION

**This is the most significant part of this amendment. A decision is required before Phase 2 design begins.**

Constraint 1 currently requires `SharedAuditBridge` to produce `ProofAttestation` records in the 82-byte format from `ruvix-types`. A new concrete type, `RvfWitnessHeader`, is now available in `rvagent-core/src/rvf_bridge.rs` (lines 304–396). The two formats serve different purposes but overlap in scope for Phase 1 audit use.

#### `RvfWitnessHeader` — what it is

`RvfWitnessHeader` is the rvAgent-layer audit record for a single task execution. It is defined at lines 308–338 in `rvf_bridge.rs`:

```rust
pub struct RvfWitnessHeader {
    pub version: u16,
    pub flags: u16,
    pub task_id: [u8; 16],        // UUID bytes
    pub policy_hash: [u8; 8],     // SHA-256 of policy, truncated to 8B
    pub created_ns: u64,          // nanoseconds since UNIX epoch
    pub outcome: TaskOutcome,
    pub governance_mode: GovernanceMode,
    pub tool_call_count: u16,
    pub total_cost_microdollars: u32,
    pub total_latency_ms: u32,
    pub total_tokens: u32,
    pub retry_count: u16,
    pub section_count: u16,
    pub total_bundle_size: u32,
}
```

Wire format constants:

```
WITNESS_MAGIC       = 0x5257_5657  ("RVWW" in little-endian)
WITNESS_HEADER_SIZE = 64 bytes
```

The 64-byte wire layout (from `to_bytes()`, lines 342–360):

```
[0..4]    WITNESS_MAGIC (0x5257_5657 LE) — "RVWW"
[4..6]    version (u16 LE)
[6..8]    flags (u16 LE)
[8..24]   task_id ([u8; 16])
[24..32]  policy_hash ([u8; 8])
[32..40]  created_ns (u64 LE)
[40]      outcome (u8)
[41]      governance_mode (u8)
[42..44]  tool_call_count (u16 LE)
[44..48]  total_cost_microdollars (u32 LE)
[48..52]  total_latency_ms (u32 LE)
[52..56]  total_tokens (u32 LE)
[56..58]  retry_count (u16 LE)
[58..60]  section_count (u16 LE)
[60..64]  total_bundle_size (u32 LE)
          ──────────────────────────────
          Total: 64 bytes
```

`from_bytes()` is the inverse and performs magic-byte validation. A round-trip test exists in `rvf_bridge.rs` at line 760.

Flags defined in `rvf_bridge.rs` (lines 276–281):

```
WIT_SIGNED       = 0x0001
WIT_HAS_SPEC     = 0x0002
WIT_HAS_PLAN     = 0x0004
WIT_HAS_TRACE    = 0x0008
WIT_HAS_DIFF     = 0x0010
WIT_HAS_TEST_LOG = 0x0020
```

#### The tension with Constraint 1

`ProofAttestation` (82 bytes, from `ruvix-types`) is a proof-kernel record. Its fields are: `proof_term_hash [u8; 32]`, `environment_hash [u8; 32]`, `verification_timestamp_ns u64`, `verifier_version u32`, `reduction_steps u32`, `cache_hit_rate_bps u16`. It is consumed by `ruvix-proof`'s `WitnessLog` at the kernel layer (Phase 2 and above on the ADR-087 stack).

`RvfWitnessHeader` (64 bytes, magic `RVWW`) is an rvAgent-layer audit record. Its fields describe task execution outcomes, governance mode, cost, latency, and token counts. It lives in the RVF Component Space layer (Phase 1 and above on the ADR-087 stack). It is available now, with a complete round-trip implementation and test coverage.

The two records are not wire-compatible and are not competing replacements. They serve different layers:

| Format | Size | Layer (ADR-087) | Scope | Availability |
|--------|------|-----------------|-------|--------------|
| `RvfWitnessHeader` | 64B | RVF Component Space (Layer 1) | rvAgent task execution | Available now (`rvf_bridge.rs`) |
| `ProofAttestation` | 82B | RuVix Cognition Kernel (Layer 3) | Kernel proof chain | Available at Phase 2 kernel integration |

#### Three options for Constraint 1

**Option A — Revise Constraint 1 to use `RvfWitnessHeader` (64B, `RVWW`) as the canonical on-disk audit format.**

`SharedAuditBridge` produces `RvfWitnessHeader` records. The 82-byte `ProofAttestation` reference in Constraint 1 is removed. When Phase 2 kernel integration begins, a translation step wraps or annotates `RvfWitnessHeader` records into `ProofAttestation` form.

Consequence: Constraint 1 is fully satisfiable today using code that already exists. The translation burden is deferred to Phase 2.

**Option B — Keep `ProofAttestation` (82B) as the canonical format; `RvfWitnessHeader` (64B) is an rvAgent-layer record that wraps alongside it.**

`SharedAuditBridge` produces `ProofAttestation` records (82B) as currently specified. Each audit log entry may additionally include an `RvfWitnessHeader` (64B) as an envelope. The two records serve different scopes and co-exist without conflict.

Consequence: `SharedAuditBridge` must produce both records. `ProofAttestation` construction requires `ruvix-proof` integration, which is not yet available in the Phase 1 dependency tree.

**Option C — Phase 1 uses `RvfWitnessHeader` (64B); Phase 2 migrates to `ProofAttestation` (82B) when kernel integration begins.**

Constraint 1 is updated to a phased form: Phase 1 `SharedAuditBridge` produces `RvfWitnessHeader` records. Phase 2 kernel integration adds a `ProofAttestation` wrapper when `rvf_mount` is invoked. The 82-byte format remains the required kernel-layer format; it is not required at Phase 1.

Consequence: The constraint is true at the layer where each format lives. No `ruvix-proof` dependency is required in Phase 1. Phase 2 migration is explicitly scoped rather than implied.

#### Recommendation to surface (not a decision)

Option C is likely the correct resolution. `RvfWitnessHeader` lives in the RVF Component Space layer (the layer Daxiom occupies in Phase 1). `ProofAttestation` lives at the RuVix Cognition Kernel layer (the layer Daxiom will integrate with in Phase 2). Under the ADR-087 layering model, it would be architecturally inconsistent to require a Phase 1 component to produce records native to a layer it does not occupy. Option C defers the `ProofAttestation` requirement to the layer transition — which is the only moment when it becomes structurally necessary.

**This decision must be made explicitly before Phase 2 design begins.** The person making the decision should also verify that ADR-007 (being updated separately) reaches the same conclusion about the `RvfWitnessHeader` / `ProofAttestation` relationship, because ADR-007 and ADR-011 must be consistent on this point.

Constraint 1 is not amended here. It remains as written in version 0.2, pending the decision above. This amendment adds the `RvfWitnessHeader` type definition and wire layout to the record so that the decision can be made with full information.

---

### Amendment 0.3.4 — Phase A status update

The following ruvector-upstream changes are relevant to Phase A status and to OpenFang-AI's position in the ADR-087 stack:

**`rvAgent` crate structure landed 2026-03-14 to 2026-03-16.** The `rvAgent` framework (8 sub-crates: `rvagent-core`, `rvagent-wasm`, and others) is now the reference implementation of the ADR-106 Layer 1/2/3 architecture. ADR-106 specifies the shared-types adapter pattern between the rvAgent runtime and the RVF/RuVix type system. `rvf_bridge.rs` is explicitly described as "ADR-106 Layer 1 shared wire types adapter for rvAgent" (line 1 of that file). The module comment references ADR-106's shared-types architecture.

This means that the types OpenFang-AI uses from `rvagent-core` — `WitTypeId`, `GovernanceMode`, `PolicyCheck`, `RvfWitnessHeader`, `RvfBridgeConfig` — are now ADR-106-compliant Layer 1 types with a stable wire format. They are not provisional.

**`RvfBridgeConfig.enabled = false` by default** (confirmed in `rvf_bridge.rs`, line 530, and verified by test at line 848). OpenFang-AI Phase 1 correctly does not enable rvf-compat features. This is consistent with the Phase 1 boundary: Daxiom operates in the RVF Component Space layer without invoking the full rvAgent bridge stack.

**`rvf_witness = false` by default** (line 518). Wire-format witness bundle production (`RvfWitnessHeader` serialization to disk) is not enabled by default. If OpenFang-AI Phase 1 wants to use `RvfWitnessHeader` for `SharedAuditBridge` (see open decision in Amendment 0.3.3), it must set `rvf_witness = true` in its `RvfBridgeConfig`. This is an opt-in, not a default behavior.

No changes to the Phase A crate test count table in the main body of this ADR are required. That table records the state as of the ADR-087 record date and is historical. The rvAgent additions are ADR-106 deliverables at the Layer 1/2 level, above the kernel primitives counted in that table.
