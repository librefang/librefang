# ADR-009: Memory Intelligence — SONA Self-Learning and Federation

**Status**: Draft
**Phase**: 2
**Date**: 2026-03-14
**Authors**: Daniel Alberttis

## Version History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 0.1 | 2026-03-14 | Daniel Alberttis | Initial draft. SONA three-store integration via consolidation.rs hook, and rvf-federation export/import for shared.rvf cross-agent learning. |
| 0.2 | 2026-03-15 | Daniel Alberttis | F4 fix: Context section corrected — `sona_step()` stub does not yet exist; the current `consolidation.rs` is pre-ADR SQLite-only. Added §6 Phase 0 defining the exact stub signature and call site that ADR-003 Phase 1 must land before any ADR-009 work begins. ADR-003 Phase 1 goal updated to include stub delivery as an explicit acceptance criterion. |
| 0.3 | 2026-03-15 | Daniel Alberttis | S6 fix: Expanded Phase 1 in-memory confidence limitation in Consequences from a one-liner into a full mitigation note — observable behaviour, warm-up window (~60s), three operator mitigations, and Phase 2 resolution path. |
| 0.4 | 2026-03-17 | ruvector-upstream sync | Reference implementation landed: `examples/rvf/examples/brain_training_integration.rs` demonstrates the SONA three-store + brain server federation protocol. Adds ⚠️ DECISION on direct vs. abstraction-layer federation. ADR-011 Phase 2 gate reminder added. |

---

> ⛔ **PHASE 2 GATE — READ BEFORE IMPLEMENTING**
>
> ADR-011 (RuVix Interface Contract) was written after this ADR and imposes two binding constraints on all Phase 2 SONA and federation work. **Do not begin implementation until you have read ADR-011 in full.**
>
> **Constraint 2 (coherence scoring):** `MemoryFragment.score` and any SONA-derived quality signals must remain derivable from the kernel-mappable inputs `{cosine_similarity, access_count, last_accessed}`. These map to `CoherenceMeta.access_count` and `CoherenceMeta.last_access_ns` in `ruvix-vecgraph`. SONA may improve how those inputs are weighted — it must not introduce scoring dimensions that have no kernel-managed equivalent. (`CoherenceMeta` actual fields: `coherence_score: u16`, `mutation_epoch: u64`, `proof_attestation_hash: [u8; 32]`, `last_access_ns: u64`, `access_count: u32`.)
>
> **Constraint 3 (inter-agent comms):** Any Phase 2 mechanism that allows agents to share SONA patterns or federated knowledge must use typed Queue semantics. Direct `Arc<Mutex<T>>` access across agent context boundaries is prohibited. `ruvix-types::WitTypeId` (`u32` newtype) should be used for typed message schemas.
>
> Reference: `docs/adr/ADR-011-ruvix-interface-contract.md`

---

## Context

ADR-003 Phase 1 must add a `sona_step()` hook point to the new `consolidation.rs` before any ADR-009 work begins. As of this writing that stub does not yet exist — the current `consolidation.rs` is the pre-ADR SQLite-only implementation. ADR-003 lists `rvf-adapters/sona` and `rvf-federation` as Phase 2 crates; this ADR provides the implementation contract for both. See §6 Phase 0 for the exact stub that ADR-003 Phase 1 must land.

The current `consolidation.rs` background task does one thing: applies confidence decay to memories older than 7 days. That prevents stale memories from surfacing — it is purely subtractive. It does not learn. It does not recognise which recall patterns are working. It does not allow agents to benefit from each other's experience.

Two components from `ruvector-upstream` address this directly:

**`rvf-adapter-sona`** — Three RVF-backed data structures (`TrajectoryStore`, `ExperienceReplayBuffer`, `NeuralPatternStore`) that turn the consolidation loop into a learning loop. After each recall, trajectories are recorded. High-confidence trajectories are promoted to the pattern store. The pattern store enables pre-query adaptation: future queries near known successful patterns are nudged toward them before hitting the HNSW index.

**`rvf-federation`** — Export/import protocol for `shared.rvf`. Agents can export accuracy-weighted knowledge they have accumulated and merge it into the org brain without overwriting existing entries. Byzantine-tolerant outlier removal and differential privacy are built in. This is how individual agent learning propagates to the shared store.

Together they answer the second gap from ADR-001: no self-learning. The first gap (O(n) scan) was answered by ADR-003. This ADR closes the loop.

---

## Decision

### 1. SONA Integration

#### 1.1 What rvf-adapter-sona provides

Three stores coexist in a single `~/.openfang/sona.rvf` file. The type marker is stored in RVF metadata field 4 (`FIELD_TYPE`):

| Store | Type marker | What it records |
|-------|-------------|-----------------|
| `TrajectoryStore` | `"trajectory"` | Sequence of (state_embedding, action, reward) steps. Circular buffer of the most recent `trajectory_window` steps (default 100). |
| `ExperienceReplayBuffer` | `"experience"` | Off-policy (state, action, reward, next_state) tuples. Circular buffer of `replay_capacity` entries (default 10,000). Prioritised sampling via HNSW nearest-neighbour on state embeddings. |
| `NeuralPatternStore` | `"pattern"` | Recognised patterns with confidence scores. In-memory category index (`HashMap<String, Vec<u64>>`). Ranked by confidence descending. |

**`SonaConfig`**:
```rust
pub struct SonaConfig {
    pub data_dir: PathBuf,           // ~/.openfang/ — sona.rvf lives here
    pub dimension: u16,              // Must match EmbeddingDriver output dimension
    pub replay_capacity: usize,      // Default 10_000
    pub trajectory_window: usize,    // Default 100
}
```

#### 1.2 What the consolidation hook does

The `sona_step()` stub in `consolidation.rs` is expanded to three operations, called by the background Tokio task:

**Post-recall trajectory recording** (called after every successful `SemanticStore::recall`):
```rust
trajectory_store.record_step(
    step_id,
    &query_embedding,
    "semantic-recall",   // action label
    quality_score,       // reward: blend of result count + avg confidence + latency delta
)?;
```

**Background learning cycle** (called every 60 seconds or when trajectory buffer is full):
```rust
let high_quality = trajectory_store.get_recent(100)
    .into_iter()
    .filter(|t| t.reward > QUALITY_THRESHOLD)   // Default: 0.7
    .collect::<Vec<_>>();

for t in high_quality {
    // NeuralPatternStore API: store_pattern(name, category, embedding, confidence) — name FIRST
    pattern_store.store_pattern(
        &format!("recall-{}", t.step_id),
        "semantic-recall",
        &t.state_embedding,
        t.reward,
    )?;
}
// ExperienceReplayBuffer API: push(state, action, reward, next_state) — 4 args
experience_buffer.push(&query_embedding, "recall", quality_score, &query_embedding)?;
```

**Pre-query adaptation** (Phase 2 — called before `AgentSemanticStore::recall_one`):
```rust
// NeuralPatternStore API: search_patterns(query_embedding, k) — 2 args
let similar_patterns = pattern_store.search_patterns(&query_embedding, 3)?;
if !similar_patterns.is_empty() {
    // Nudge query embedding toward centroid of similar successful patterns
    let adapted = blend_toward_centroid(&query_embedding, &similar_patterns, BLEND_ALPHA);
    // adapted embedding replaces query_embedding for this recall
}
```

Phase 1 implements post-recall recording and the background learning cycle only. Pre-query adaptation is Phase 2 (gated by `features = ["sona-adapt"]`).

#### 1.3 What SONA learns

SONA does not rewrite the HNSW index. It learns at the query layer — which shapes of query reliably produce high-confidence results, and nudges future queries of similar shape toward the same region of the embedding space. The RVF stores are separate from the agent's memory stores; they contain learning state, not memories.

Confidence updates to patterns are in-memory only in Phase 1 (`update_confidence()` updates the in-memory `PatternMeta` HashMap). Phase 2 persists confidence updates back to `sona.rvf` via a delete-reingest cycle (same mechanism as `update_embedding` in ADR-003 §1.3).

#### 1.4 File layout addition

```
~/.openfang/
├── shared.rvf
├── shared.access.db
├── sona.rvf                ← new (ADR-009)
└── agents/
    ├── {agent_id}.rvf
    └── {agent_id}.access.db
```

One `sona.rvf` per deployment. All agents share the same pattern store — patterns learned from agent A's recalls benefit agent B. The trajectory and experience stores are agent-scoped in the in-memory window; the pattern store is global.

---

### 2. Federation Integration

#### 2.1 What rvf-federation provides

Federation operates on learned data that agents have accumulated and allows it to be merged into `shared.rvf` in a privacy-preserving way.

**`FederatedAggregator`** — three strategies:

| Strategy | Weight basis | Use case |
|----------|-------------|----------|
| `FedAvg` | `trajectory_count` (number of learning examples) | Default — more evidence = more weight |
| `FedProx { mu }` | `trajectory_count` + proximal regularisation | Prevents drift when contributors are heterogeneous |
| `WeightedAverage` | explicit `quality_weight` scalar (0.0–1.0) | When accuracy metrics are available externally |

Byzantine outlier removal is applied before aggregation: contributions whose L2 norm deviates more than 2σ from the median are excluded. The `AggregateWeights` result carries `byzantine_filtered: bool` and `outliers_removed: u32`.

**`DiffPrivacyEngine`** — Gaussian or Laplace noise added to all numerical payloads:
- Beta posterior parameters (alpha, beta)
- LoRA weight vectors
- Policy kernel knobs
- Cost curve values

Parameters: `epsilon` (privacy loss), `delta` (failure probability), `mechanism` (Gaussian recommended for LoRA weights, Laplace for scalars).

#### 2.2 Export/import flow

**Export** (agent → federation layer):
```rust
let export = ExportBuilder::new(agent_pseudonym, domain_id)
    .add_priors(agent.trajectory_priors())
    .add_weights(agent.lora_deltas())
    .build(&mut dp_engine)?;
// export.privacy_proof — proof of DP noise application
// export.redaction_log — PII scanner attestation
```

**Import** (federation layer → shared.rvf):
```rust
let merger = ImportMerger::new();
merger.validate(&export)?;
// merge_priors takes &mut RvfStore (the shared store), not a sub-field
merger.merge_priors(
    &mut shared_store,
    &export.priors,
    export.manifest.version,
)?;
merger.merge_weights(
    &mut shared_store,
    &export.weights[0],
    local_accuracy,
    export.manifest.quality_weight,
)?;
```

Version-aware dampening: if `export.manifest.version` matches local, full weight is applied. Older versions are mixed with a uniform prior before merging, reducing their influence proportionally.

#### 2.3 What federation changes in openfang-memory

A new `FederationManager` struct in `engine.rs` (behind `features = ["federation"]`):

```rust
pub struct FederationManager {
    aggregator: FederatedAggregator,
    dp_engine: DiffPrivacyEngine,
    policy: FederationPolicy,
}

impl SemanticStore {
    // Export this agent's learned knowledge for federation
    pub fn federation_export(&self, agent_id: &AgentId) -> Result<FederatedExport>;

    // Import federated knowledge into shared.rvf
    pub fn federation_import(&mut self, export: FederatedExport) -> Result<ImportResult>;
}
```

Federation is triggered explicitly — it is not automatic. A kernel capability (`FederationContribute`) gates which agents may call `federation_export`. The import step runs in the consolidation background task on a configurable interval (default: every 6 hours, or when triggered by `kernel.run_federation_round()`).

---

### 3. Crates to Vendor

Two additional crates added to `vendor/rvf/`:

| Crate | Version | Purpose |
|-------|---------|---------|
| `rvf-adapters/sona` | 0.1.0 | `SonaConfig`, `TrajectoryStore`, `ExperienceReplayBuffer`, `NeuralPatternStore` |
| `rvf-federation` | 0.1.0 | `ExportBuilder`, `ImportMerger`, `FederatedAggregator`, `DiffPrivacyEngine`, `FederationPolicy` |

The deeper `ruvector-dag` SONA engine (`DagSonaEngine`, `MicroLoRA`, `EWC++`) is **not vendored in Phase 1**. The `rvf-adapter-sona` crate is the thin RVF-backed adapter — it uses RVF stores for trajectory/experience/pattern data but does not implement the full gradient adaptation loop. `MicroLoRA` and `EWC++` are Phase 2.

**Updated vendor layout**:
```
vendor/rvf/
├── rvf-runtime/          ← ADR-003
├── rvf-types/            ← ADR-003
├── rvf-crypto/           ← ADR-003
├── rvf-adapters/
│   └── sona/             ← ADR-009
└── rvf-federation/       ← ADR-009
```

---

### 4. New Feature Flags in openfang-memory

```toml
# crates/openfang-memory/Cargo.toml
[features]
default = []
shared-phase2 = []         # ADR-003 Phase 2
sona = ["dep:rvf-adapter-sona"]
sona-adapt = ["sona"]      # Phase 2 pre-query adaptation (MicroLoRA)
federation = ["dep:rvf-federation"]
```

All new capabilities are feature-gated. Phase 1 of this ADR ships `sona` (recording + background learning). `sona-adapt` and `federation` are Phase 2.

---

### 5. Module File Changes

No new source files are required. Changes are additive within existing files:

- **`engine.rs`** — `SemanticStore` gains a `sona: Option<SonaStores>` field (Some when `sona` feature enabled). New method `federation_export` / `federation_import` (behind `federation` feature).
- **`consolidation.rs`** — `sona_step()` stub is implemented. Background task wired to call trajectory recording post-recall and background learning on timer.
- **`lib.rs`** — Re-exports `FederatedExport`, `ImportResult` (behind `federation` feature).

No changes to `store.rs`, `agent_meta.rs`, `shared_meta.rs`, `ranking.rs`, or `audit.rs`.

---

### 6. Implementation Order

#### Phase 0 — sona_step() stub (delivered as part of ADR-003 Phase 1)

> **Owner**: ADR-003 implementer. This step must be complete and committed before any ADR-009 work begins.

The new `consolidation.rs` (written as part of ADR-003 Phase 1) must expose a `sona_step()` method on `ConsolidationEngine`. In Phase 1 it is a no-op — its only purpose is to provide the call-site that ADR-009 Phase 1 will wire into:

```rust
impl ConsolidationEngine {
    /// SONA learning hook — called by SemanticStore after every successful recall.
    /// No-op until ADR-009 Phase 1 is implemented (feature = "sona").
    /// Signature is fixed: do not change without updating ADR-009 §1.2.
    #[cfg(not(feature = "sona"))]
    pub fn sona_step(&self, _step_id: u64, _embedding: &[f32], _quality: f32) {}
}
```

Call site in `engine.rs` (also ADR-003 Phase 1):

```rust
// After every successful SemanticStore::recall_one:
self.consolidation.sona_step(step_id, &query_embedding, quality_score);
```

Acceptance: `grep -r "sona_step" crates/openfang-memory/src/` returns matches in both `consolidation.rs` and `engine.rs` before ADR-009 Phase 1 starts.

---

> **Prerequisite for all phases below**: ADR-003 Phase 1 complete, `cargo test -p openfang-memory` passing, and `sona_step()` stub confirmed present (Phase 0 above).

#### Phase 1 — SONA recording and background learning

1. Vendor `rvf-adapters/sona` into `vendor/rvf/rvf-adapters/sona/`.
2. Add `sona` feature flag to `crates/openfang-memory/Cargo.toml`.
3. Add `SonaStores` wrapper struct in `engine.rs` holding `TrajectoryStore`, `ExperienceReplayBuffer`, `NeuralPatternStore`.
4. Wire `sona_step()` in `consolidation.rs`:
   - Post-recall: `trajectory_store.record_step(step_id, embedding, "semantic-recall", quality)`
   - Background (every 60s): promote high-confidence trajectories to `pattern_store`
5. Tests: `test_sona_trajectory_recording`, `test_sona_background_promotion`, `test_sona_pattern_search`.

Acceptance criteria: `cargo test -p openfang-memory --features sona` passes. `sona.rvf` is created at first recall with `sona` feature enabled. Patterns accumulate in `sona.rvf` after 60 seconds of recall activity.

#### Phase 2 — Pre-query adaptation and federation

1. Vendor `rvf-federation` into `vendor/rvf/rvf-federation/`.
2. Implement pre-query adaptation in `engine.rs` behind `sona-adapt`.
3. Implement `FederationManager`, `federation_export`, `federation_import` behind `federation`.
4. Wire federation round trigger into kernel capability system (`FederationContribute`).
5. Tests: `test_sona_prequery_adaptation`, `test_federation_export_import`, `test_federation_byzantine_filter`, `test_federation_diff_privacy_applied`.

---

## Consequences

### Positive

- `consolidation.rs` becomes a learning loop, not just decay — the store improves with use
- Pattern-based pre-query adaptation (Phase 2) improves recall precision for known query shapes without touching the HNSW index
- Federation lets the org brain (`shared.rvf`) benefit from agent specialisation without centralised training
- Byzantine outlier removal and differential privacy make federation safe to enable in multi-tenant deployments
- All new capabilities are feature-flagged — Phase 1 ships sona recording with zero impact on builds that don't enable it

### Negative

- `sona.rvf` adds a third RVF file to manage alongside `shared.rvf` and `{agent_id}.rvf` — more disk surface, more file handles
- **Phase 1 pattern confidence is in-memory only — not persisted to `sona.rvf`.**
  `update_confidence()` updates an in-memory `HashMap<String, PatternMeta>` only. On restart, all pattern entries reload from `sona.rvf` but every confidence score resets to its original ingest value. The learning from the previous session is structurally present (the pattern embeddings exist) but the trust weighting is gone.

  **What operators will observe:** After a restart, the first ~60 seconds of recall activity re-promotes high-quality trajectories back into the pattern store and rebuilds confidence. During this warm-up window, recall quality falls back to unweighted HNSW results — correct, but not SONA-adapted.

  **Mitigations (Phase 1):**
  - Avoid restarting during active high-volume recall windows (e.g. batch jobs, scheduled recall sweeps).
  - If restarts are frequent, disable the `sona` feature flag until Phase 2 — the store functions correctly without it.
  - The trajectory and experience stores *are* persisted to `sona.rvf`, so the raw learning data survives; only the derived confidence weights are lost.

  **Phase 2 resolution:** `update_confidence()` will persist via a delete-reingest cycle on `sona.rvf` (same mechanism as `update_embedding` in ADR-003 §1.3). After that, restarts are fully warm.
- Federation round timing (default 6 hours) means the shared store may lag agent learning by up to 6 hours; not a real-time propagation mechanism

### Neutral

- SONA does not retrain the HNSW index — it learns at the query layer, which is lighter but also means it cannot correct structural mistakes in stored embeddings
- Federation requires `FederationContribute` capability — disabled by default; opt-in per agent

---

## References

- `ADR-003-memory-store-implementation.md` — phase structure and vendoring convention this ADR extends
- `crates/rvf/rvf-adapters/sona/src/` in ruvector-upstream — `SonaConfig`, `TrajectoryStore`, `ExperienceReplayBuffer`, `NeuralPatternStore`
- `crates/rvf/rvf-federation/src/` in ruvector-upstream — `ExportBuilder`, `ImportMerger`, `FederatedAggregator`, `DiffPrivacyEngine`
- `crates/ruvector-dag/src/sona/engine.rs` in ruvector-upstream — `DagSonaEngine`, `MicroLoRA`, `EWC++` (Phase 2 reference, not vendored in Phase 1)

---

## Amendment 0.4: ruvector-upstream Sync — 2026-03-17

### 1. Reference implementation now available

`examples/rvf/examples/brain_training_integration.rs` (landed 2026-03-16, commit `48954004` area) provides the first concrete end-to-end demonstration of the SONA data-to-brain pipeline that ADR-009 specifies. It does not use the `rvf-adapter-sona` / `rvf-federation` crates directly — it implements the pattern at the application level using raw HTTP calls to `pi.ruv.io`. This distinction matters (see §4 DECISION below).

#### Brain server API surface demonstrated

The example exposes four HTTP endpoints against `$BRAIN_URL` (default `https://pi.ruv.io`), all authenticated via `Authorization: Bearer $PI`:

| Method | Path | Body / Params | Purpose |
|--------|------|---------------|---------|
| `POST` | `/v1/memories` | JSON: `title`, `content`, `category`, `tags` | Ingest a single discovery as a brain memory |
| `POST` | `/v1/train` | `{}` (empty JSON object) | Trigger a SONA training cycle |
| `GET` | `/v1/sona/stats` | — | Retrieve current SONA learning metrics |
| `GET` | `/v1/explore` | — | Retrieve meta-learning / exploration stats |
| `GET` | `/v1/temporal` | — | Retrieve temporal delta tracking stats |

The `BrainClient` struct (lines 414–512 of the file) encapsulates these calls. Methods: `share_memory(&TrainingExperience)`, `train()`, `sona_stats()`, `explore()`, `temporal()`. All use `curl` subprocesses with `--max-time 10` (memories) or `--max-time 15` (train). Authentication is read from the `PI` environment variable; the URL from `BRAIN_URL`.

#### TrainingExperience — the federation payload format

The `TrainingExperience` struct (lines 87–97) is the unit of data transferred to the brain server:

```rust
struct TrainingExperience {
    domain:   String,   // e.g. "exoplanet", "seismology", "climate"
    state:    String,   // Structured string encoding the input observation
    action:   String,   // What the model decided (e.g. "flagged_anomaly:...")
    reward:   f64,      // Normalised quality score [0, 1]
    category: String,   // Taxonomy label (always "pattern" in this example)
    title:    String,   // Human-readable summary
    content:  String,   // Full narrative of the discovery
    tags:     Vec<String>,
}
```

Only `title`, `content`, `category`, and `tags` are sent in the `/v1/memories` POST body. The `state`, `action`, and `reward` fields are used locally to rank and filter experiences before sharing — they are **not** transmitted to the brain server in this implementation. This is a notable divergence from the ADR-009 specification (see divergences below).

#### How trajectories are written

The example does not use `TrajectoryStore` directly. Instead, it constructs `TrainingExperience` records locally by running a graph-cut anomaly detector over each dataset. The `reward` field is computed per-experience (normalised anomaly score, clamped to `[0, 1]`). High-reward experiences (the top-10 by score per dataset, lines 169–191) are selected for sharing. The trajectory recording described in ADR-009 §1.2 (`trajectory_store.record_step(...)`) is not present in this example; the pattern of "run detector, score, share high scorers" is the application-level analogue.

#### How high-confidence trajectories are promoted to NeuralPatternStore

Promotion to `NeuralPatternStore` is not directly implemented in this example. Promotion is delegated entirely to the brain server: after calling `POST /v1/memories` for each experience, the example calls `POST /v1/train` once (lines 617–621), which triggers the SONA training cycle on the server side. The brain server is responsible for updating its own pattern store — the client has no visibility into which memories were promoted.

#### Federation export format and privacy controls

The federation export in this example is implicit: each `POST /v1/memories` body contains only human-readable `title`, `content`, `category`, and `tags`. There is no `DiffPrivacyEngine` noise application, no Byzantine filter, and no `ExportBuilder` / `ImportMerger` call sequence as specified in ADR-009 §2.2. Privacy control is limited to:

1. Obfuscating the API key in console output (`brain.api_key[..4]...brain.api_key[len-4..]`, line 578).
2. Checking `brain.is_configured()` — if `PI` is unset, the example runs in dry-run mode and no data is transmitted (lines 551–574).

There is no DP noise, no `redaction_log`, and no `privacy_proof` attestation in this implementation.

#### Divergences from ADR-009 specification

| ADR-009 spec | Reference implementation | Impact |
|---|---|---|
| `TrajectoryStore.record_step()` called post-recall | Not used; experiences constructed locally from detector output | The three-store pipeline is not exercised; `sona.rvf` is not created |
| `ExperienceReplayBuffer` wired into consolidation | Not present | Off-policy replay is absent |
| `NeuralPatternStore` promotion on background cycle | Delegated to brain server via `/v1/train` | Promotion logic is opaque; `pattern_store.store_pattern()` is not called |
| `DiffPrivacyEngine` applied on export | Not applied | Raw content is sent to brain server; no DP attestation |
| `ExportBuilder` / `ImportMerger` used for federation | Not used; raw HTTP POST to `/v1/memories` | No `privacy_proof`, no `redaction_log`, no Byzantine filtering |
| Reward / state / action fields in federation payload | Not transmitted to brain server | Brain server sees only human-readable text, not structured RL tuples |

These divergences indicate that `brain_training_integration.rs` implements the **federation intent** (share discoveries with a central brain, trigger learning) but does not implement the **federation protocol** specified in ADR-009. The reference implementation is architecturally simpler and trades DP/Byzantine guarantees for operational simplicity.

### 2. rvf-adapter-sona and rvf-federation crate status

`brain_training_integration.rs` demonstrates the federation protocol at the **example / application layer** only. It calls the brain server's HTTP endpoints directly and does not exercise `rvf-adapter-sona` or `rvf-federation` internally. The production crates remain Phase 2 deliverables per §6 of this ADR. They now have a reference implementation to validate the end-to-end intent against, but the crate-level API surface (`ExportBuilder`, `ImportMerger`, `DiffPrivacyEngine`, `TrajectoryStore`, `NeuralPatternStore`) has not changed — it is still the contract documented in ADR-009 §1.1 and §2.1.

Before Phase 2 begins, the `rvf-adapter-sona` and `rvf-federation` implementations must be reconciled against this example's observed behaviour — particularly the gap in DP noise application and the fact that the brain server expects human-readable memory payloads, not raw RL tuples.

### 3. ADR-011 Phase 2 gate reminder

Constraint 2 (coherence scoring) and Constraint 3 (inter-agent comms via typed Queue) from ADR-011 apply to all SONA Phase 2 work. The reference implementation in `brain_training_integration.rs` does not go through typed Queue boundaries — it calls HTTP endpoints directly from within a single-process example. This is acceptable for an example but is not the permitted pattern for Phase 2 production code. Any OpenFang agent that shares SONA patterns or federated knowledge must do so through typed Queue semantics, not direct HTTP calls embedded in agent logic.

Specifically: the `BrainClient` struct and its direct `curl` subprocess invocations must not be ported into `openfang-memory` or `openfang-runtime` as-is. A Queue message boundary (`FederationContribute` capability, as described in §2.3 of this ADR) must gate all federation calls.

### 4. ⚠️ DECISION: Direct brain server API surface vs. rvf-federation abstraction layer

The reference implementation calls `pi.ruv.io` directly:

```
agent code
    → POST https://pi.ruv.io/v1/memories   (per-experience)
    → POST https://pi.ruv.io/v1/train
```

ADR-009 §2.1–2.3 specifies a different topology:

```
agent code
    → ExportBuilder / federation_export()
    → rvf-federation crate (DiffPrivacyEngine, Byzantine filter)
    → ImportMerger / federation_import() into shared.rvf
    (brain server is an optional downstream recipient, not the primary store)
```

Two options for OpenFang-AI:

**Option A — Direct brain server API** (as demonstrated in the reference implementation): OpenFang agents call `/v1/memories` and `/v1/train` on `pi.ruv.io` directly, bypassing the `rvf-federation` abstraction. Simpler to implement; matches the example exactly. Creates a **hard runtime dependency on `pi.ruv.io`** for all SONA federation. Offline operation (air-gapped deployments, network partitions) is not possible without fallback logic. DP noise and Byzantine filtering are delegated to whatever the brain server does internally — not auditable by OpenFang.

**Option B — rvf-federation abstraction layer** (as originally specified in ADR-009): `rvf-federation` crate remains the federation boundary. The brain server is an optional downstream sink that `rvf-federation` may push to, but `shared.rvf` remains the authoritative local store. Offline operation is fully supported. DP attestation and Byzantine filtering are local and auditable. More implementation work; requires the `rvf-federation` crate to be fully vendored and the API surface reconciled with the brain server's memory format.

**Decision: Option B — rvf-federation abstraction layer.**

Rationale: OpenFang-AI targets self-hosted deployments where `pi.ruv.io` availability cannot be guaranteed. `shared.rvf` must remain the authoritative local store. DP attestation and Byzantine filtering must be local and auditable. The brain server (`pi.ruv.io`) is an optional downstream sink, not a required runtime dependency. This is a hard gate for PLAN-002 T-09 — federation implementation must use `ExportBuilder` / `ImportMerger` / `DiffPrivacyEngine`, not raw HTTP calls to `pi.ruv.io`.
