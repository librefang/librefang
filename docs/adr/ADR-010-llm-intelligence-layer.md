# ADR-010: LLM Intelligence Layer — ruvllm Routing and Adaptive Learning

**Status**: Draft
**Phase**: 2
**Date**: 2026-03-14
**Authors**: Daniel Alberttis

## Version History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 0.1 | 2026-03-14 | Daniel Alberttis | Initial draft. ruvllm as a routing intelligence layer over existing LLM drivers. 7-factor complexity scoring for model-tier selection. SONA adaptive learning on routing decisions. MoE expert routing and local inference deferred. |
| 0.2 | 2026-03-14 | Daniel Alberttis | Documented compaction side-effect: `compact_session()` uses `resolve_driver()` so all LLM compaction calls route through `RuvLLMRouter` when enabled. Compaction will score <0.3 → Haiku. Added `compaction_model_override` field recommendation to `RoutingConfig`. Added compaction test to Phase 1. |
| 0.3 | 2026-03-14 | Daniel Alberttis | Updated file tree: side-store files changed from JSON (`.access.json`) to SQLite (`.access.db`) per ADR-003 v0.7 decision. |
| 0.4 | 2026-03-15 | Daniel Alberttis | S4 clarification: `compaction_model_override` field downgraded from recommended-to-add to optional/low-priority. Source-verified: native OpenFang compacts using the agent manifest model verbatim; ruvllm complexity scoring structurally routes compaction to Haiku (score <0.3) regardless of manifest, which already improves on native behaviour. Override field not required for Phase 1. |
| 0.5 | 2026-03-17 | ruvector-upstream sync | ruvllm MoE buffer reuse landed (commit 20c620b5). mcp-brain-server trainer.rs now available. Common Crawl adapter as forward reference for domain expertise scoring. ADR-011 Phase 2 gate reminder added. ⚠️ DECISION on Phase 2 performance target re-validation. |

---

> ⛔ **PHASE 2 GATE — READ BEFORE IMPLEMENTING**
>
> ADR-011 (RuVix Interface Contract) imposes one binding constraint on all Phase 2 ruvllm and SONA routing work. **Do not begin Phase 2 implementation until you have read ADR-011 in full.**
>
> **Constraint 3 (inter-agent comms):** Any mechanism where routing decisions, SONA learning signals, or MoE gate state are shared between agents must use typed Queue semantics. The `RuvLLMRouter` may not expose a shared mutable handle that agents grab directly — routing calls must go through typed message boundaries. This applies to the SONA three-tier adaptive learning path: training signals from one agent's routing decisions must not be fed to another agent's learner via direct memory sharing.
>
> Reference: `docs/adr/ADR-011-ruvix-interface-contract.md`

---

## Context

OpenFang's current LLM routing is coarse. `ModelRouter` in `openfang-runtime` selects between model tiers based on `TaskComplexity` — a single scoring signal. The manifest's `model` field overrides it entirely. There is no per-request analysis of what the request actually needs, no cost/latency awareness, no feedback loop from outcomes back to routing decisions.

`ruvllm` (v2.5.2 — Rust crate + npm package) is Ruv's LLM intelligence layer. It provides three capabilities directly relevant to OpenFang:

1. **7-factor complexity routing** — scores each request across token count, reasoning depth, domain expertise, code complexity, planning complexity, security sensitivity, and performance criticality, then routes to the appropriate model tier (Haiku/Sonnet/Opus or equivalent). Uses `ComplexityAnalyzer` with configurable weights per factor.

2. **SONA three-tier adaptive learning** — routing decisions themselves become training data. Instant (<1ms MicroLoRA), background (~100ms EWC++), and deep (scheduled) adaptation improve routing accuracy over time without manual tuning.

3. **MoE memory-aware expert routing** — for Mixture of Experts models, `MemoryAwareRouter` adds a cache residency bonus to gate network logits, achieving 70% cache hit rate vs 34% baseline with <1% accuracy loss. In scope when local inference is added (a future ADR).

This ADR covers Phase 1: `ruvllm` routing intelligence layered over OpenFang's existing remote API drivers. Local inference (`CandleBackend`, GGUF loading) is **not in scope** — that warrants a separate ADR covering model selection, hardware requirements, and on-device deployment. Phase 2 adds SONA learning on routing decisions.

---

## Decision

### 1. What Changes — and What Doesn't

OpenFang's existing drivers (`AnthropicDriver`, `GeminiDriver`, `OpenAIDriver`) are unchanged. They continue to make the actual API calls. `ruvllm` is inserted as a routing layer **above** the drivers, not replacing them.

```
Message from agent runtime
        ↓
RuvLLMRouter (new — ADR-010)
  ComplexityAnalyzer → scores 7 factors → selects model tier
  WitnessLog → records routing decision + outcome
  SonaIntegration → learns from outcome (Phase 2)
        ↓
Existing drivers (unchanged)
  AnthropicDriver / GeminiDriver / OpenAIDriver
        ↓
LLM API response
```

The `LlmDriver` trait in `openfang-runtime` is unchanged. `RuvLLMRouter` implements it. The kernel swaps `AnthropicDriver` for `RuvLLMRouter` when ruvllm routing is enabled in `KernelConfig`.

**Compaction side-effect** — `kernel.compact_agent_session()` calls `resolve_driver(&manifest)` to obtain an `Arc<dyn LlmDriver>` and passes it directly into `compact_session(driver, model, session, config)`. There is no separate compaction driver. When `RuvLLMRouter` is the resolved driver, **all LLM compaction summarization calls also route through ruvllm's complexity scoring**.

Natively (without ruvllm), OpenFang uses the agent's manifest model verbatim for compaction — an Opus agent compacts at Opus rates. With `RuvLLMRouter` enabled, compaction summarization (bounded input, no reasoning depth, no code) will consistently score below 0.3 and route to Haiku regardless of manifest, which is an improvement over native behaviour. This is the intended behaviour and must be tested explicitly (see §5 Phase 1 tests).

**`compaction_model_override` — optional, low priority.** A `compaction_model_override: Option<String>` field in `RoutingConfig` would allow operators to hard-pin compaction to a specific model as an additional safety net. However, given that the complexity scoring structurally guarantees Haiku for compaction tasks (the task characteristics — bounded input, no code, no reasoning chain — are invariant), this field is not required for correct behaviour. It may be added as a comfort feature if operators want an explicit config knob, but it is not a prerequisite for Phase 1 and should not block implementation.

---

### 2. ComplexityAnalyzer — 7-Factor Model Routing

`ModelRouter` in ruvllm replaces OpenFang's existing `ModelRouter` for the routing decision. It scores 7 factors:

| Factor | Weight | What it measures | Low → High |
|--------|:------:|-----------------|-----------|
| `token_estimate` | 0.20 | Estimated input + output token count | 0–500 → 500–2000 → >2000 |
| `reasoning_depth` | 0.25 | Multi-step logic, deductive chains | Simple reply → complex proof |
| `domain_expertise` | 0.20 | Specialised knowledge required | General → deep domain |
| `code_complexity` | 0.15 | Algorithm difficulty, architecture | Boilerplate → distributed systems |
| `planning_complexity` | 0.10 | Multi-step execution plans | Single action → week-long plan |
| `security_sensitivity` | 0.05 | Safety-critical output | Informational → agent capability grant |
| `performance_criticality` | 0.05 | Latency constraint | Background task → real-time channel |

**Routing thresholds:**

| Score range | Model tier | Typical OpenFang usage |
|-------------|-----------|------------------------|
| < 0.3 | Haiku (fast, cheap) | Tool call responses, simple KV lookups, status replies |
| 0.3–0.6 | Sonnet (balanced) | Standard agent tasks, code generation, memory synthesis |
| > 0.6 | Opus (capable, costly) | Complex reasoning, security decisions, architecture planning |

The existing manifest `model` field acts as a ceiling, not an override: if a manifest specifies `claude-haiku-4-5` the router never escalates above Haiku even if complexity scores Sonnet. If the manifest specifies the full model family (e.g., `anthropic/claude`) the router selects the tier freely.

**Complexity estimation** runs synchronously before the API call. It does not call the LLM — it analyses the message payload, system prompt length, tool list size, and prior conversation length. Estimated latency: < 1ms.

---

### 3. RuvLLMRouter Implementation

```rust
// crates/openfang-runtime/src/drivers/ruvllm_router.rs

pub struct RuvLLMRouter {
    inner:        Arc<dyn LlmDriver>,   // Wrapped existing driver (e.g. AnthropicDriver)
    engine:       RuvLLMEngine,         // ruvllm session/witness/sona management
    router:       ModelRouter,          // 7-factor complexity analyzer
    config:       RoutingConfig,        // Thresholds, model names per tier
}

pub struct RoutingConfig {
    pub haiku_model:  String,   // e.g. "claude-haiku-4-5-20251001"
    pub sonnet_model: String,   // e.g. "claude-sonnet-4-6"
    pub opus_model:   String,   // e.g. "claude-opus-4-6"
    pub complexity_weights: ComplexityWeights,  // Override default factor weights
    pub manifest_ceiling: bool, // Respect manifest model as ceiling (default: true)
}
```

`RuvLLMRouter::complete()` flow:
1. `ComplexityAnalyzer::analyze(&request)` → `ComplexityScore { score, factors }`
2. Select model from `RoutingConfig` based on score and manifest ceiling
3. Rewrite `request.model` to the selected model name
4. Delegate to `self.inner.complete(request)` — the actual API call
5. Record routing decision + latency + outcome to `WitnessLog`
6. (Phase 2) Feed outcome to `SonaIntegration::post_query()`

The `WitnessLog` is HNSW-indexed (backed by an `RvfStore` at `~/.openfang/ruvllm.rvf`). This means routing decisions are semantically searchable — debugging why an agent got Haiku when it needed Opus is a vector query, not a log scan.

---

### 4. Configuration

New section in `KernelConfig` / `config.toml`:

```toml
[ruvllm]
enabled = true

[ruvllm.routing]
haiku_model  = "claude-haiku-4-5-20251001"
sonnet_model = "claude-sonnet-4-6"
opus_model   = "claude-opus-4-6"
manifest_ceiling = true        # Respect manifest model as upper bound

[ruvllm.routing.weights]
token_estimate       = 0.20
reasoning_depth      = 0.25
domain_expertise     = 0.20
code_complexity      = 0.15
planning_complexity  = 0.10
security_sensitivity = 0.05
performance_criticality = 0.05
```

When `ruvllm.enabled = false` (the default), the kernel uses the existing `ModelRouter` and driver selection unchanged — zero regression risk.

---

### 5. Witness Log and Audit

`ruvllm` maintains a `WitnessLog` per engine instance — an HNSW-indexed `RvfStore` at `~/.openfang/ruvllm.rvf`. Every routing decision is recorded:

```rust
pub struct WitnessEntry {
    pub request_id:       Uuid,
    pub agent_id:         AgentId,
    pub complexity_score: f32,
    pub selected_model:   String,
    pub rationale:        ComplexityFactors,   // All 7 factor scores
    pub latency_ms:       f64,
    pub input_tokens:     u32,
    pub output_tokens:    u32,
    pub cost_usd:         f64,
}
```

`engine.search_witness(&query_embedding, k)` returns semantically similar past routing decisions — useful for auditing cost outliers, debugging unexpected model selection, and (Phase 2) as training data for SONA.

The `ruvllm.rvf` file is separate from the memory layer files:

```
~/.openfang/
├── shared.rvf              ← ADR-003
├── shared.access.db        ← ADR-003 (AgentSideStore — SQLite side-store)
├── sona.rvf                ← ADR-009
├── ruvllm.rvf              ← ADR-010 (witness log)
└── agents/
    ├── {agent_id}.rvf
    └── {agent_id}.access.db  ← ADR-003 (AgentSideStore)
```

---

### 6. Phase 2 — SONA Adaptive Learning on Routing

Phase 2 wires `SonaIntegration` inside `RuvLLMEngine` to learn from routing outcomes. The three-tier learning loop:

**Instant (<1ms)**: After each response, `SonaLlm::instant_adapt(&query_emb, &response_emb, quality_score)` updates a rank-4 `MicroLoRA` applied to the complexity embedding. This nudges the score function toward better calibration without a training round.

**Background (~100ms)**: After accumulating 10+ samples, `SonaLlm::maybe_background()` runs EWC++ consolidation — elastic weight constraints prevent the adapter from catastrophically forgetting prior calibrations while learning new routing patterns.

**Deep (scheduled)**: `SonaLlm::deep_optimize(&samples)` recomputes on the full trajectory buffer. Triggered manually or on a schedule.

What SONA learns about routing: which complexity factor weights produce accurate tier selection for this specific agent's workload. An agent that primarily does code generation will see its `code_complexity` weight drift upward; an agent that does conversational tasks will see `token_estimate` and `reasoning_depth` dominate. The adaptation is per-engine-instance, not global.

Phase 2 requires `features = ["ruvllm-sona"]` on `openfang-runtime`. The `ruvector-dag` crate (`DagSonaEngine`, `MicroLoRA`, `EWC++`) is vendored at this phase — it is a deeper dependency than `rvf-adapter-sona` and has its own `rust-version` and BLAS requirements.

---

### 7. MoE Expert Routing — Deferred to Phase 2 (Local Inference)

`MemoryAwareRouter` provides cache-aware expert selection for MoE models. It adds a cache residency bonus (0.15 recommended) to the gate network logits, achieving 70% cache hit rate vs 34% baseline with <1% accuracy loss. This is only relevant when OpenFang is running inference locally (GGUF model via `CandleBackend`). Remote API calls do not expose expert routing.

Local inference is deferred to a future ADR. At that point `MemoryAwareRouter` slots in as a single component within `CandleBackend`'s forward pass — it does not change `RuvLLMRouter`'s interface.

---

### 8. Crate Vendoring

```
crates/ruvllm/ (from ruvector-upstream)
```

`ruvllm` is a large Rust crate with optional features. OpenFang vendors only the needed feature set:

```toml
# In vendor/ruvllm/Cargo.toml (stripped from upstream)
[features]
default = ["routing-metrics"]    # Complexity analyzer + witness log
sona    = ["dep:ruvector-sona"]  # Phase 2: adaptive learning
```

Features explicitly excluded: `candle` (local inference), `metal`, `cuda`, `inference-metal`, `coreml`, `hybrid-ane`, `parallel` — these are for the local `CandleBackend` and are not needed for the routing-only Phase 1. This significantly reduces compile time and removes BLAS/GPU build requirements.

**`rust-version` check**: Verify `vendor/ruvllm/Cargo.toml` before compiling. If it declares `rust-version` above the workspace pin, resolve using the same two options from ADR-003 Phase 0 (bump workspace or patch vendored).

---

### 9. Implementation Order

> **Prerequisite**: ADR-003 Phase 1 complete and `cargo test -p openfang-memory` passing.

#### Phase 1 — Routing layer

1. Vendor `ruvllm` with `default = ["routing-metrics"]` features only.
2. Add `[ruvllm]` section to `KernelConfig` (disabled by default).
3. Implement `RuvLLMRouter` in `crates/openfang-runtime/src/drivers/ruvllm_router.rs`.
4. Wire into kernel driver selection: when `ruvllm.enabled = true`, wrap the primary driver.
5. Tests:
   - `test_routing_simple_request_selects_haiku`
   - `test_routing_complex_code_selects_opus`
   - `test_routing_manifest_ceiling_respected`
   - `test_witness_log_records_decision`
   - `test_routing_disabled_passthrough`
   - `test_compaction_routes_to_haiku` — verifies `compact_session` with `RuvLLMRouter` selects Haiku tier regardless of manifest model

Acceptance criteria: `cargo test -p openfang-runtime` passes with and without `ruvllm.enabled`. `ruvllm.rvf` is created at first request when enabled. Routing decisions are visible via `engine.search_witness()`.

#### Phase 2 — SONA adaptive learning

1. Add `sona` feature to vendored `ruvllm` Cargo.toml.
2. Vendor `ruvector-dag` (for `DagSonaEngine`, `MicroLoRA`, `EWC++`) into `vendor/ruvector-dag/`.
3. Add `features = ["ruvllm-sona"]` gate to `openfang-runtime`.
4. Wire `post_query()` callback into `RuvLLMRouter::complete()`.
5. Tests: `test_sona_instant_adapt_latency_under_1ms`, `test_sona_background_consolidation`, `test_routing_improves_after_100_samples`.

---

## Consequences

### Positive

- Model-tier routing becomes data-driven per request, not per manifest — the right model for the task, not the most expensive one by default
- 7-factor complexity analysis is more accurate than the current single-signal `TaskComplexity` for distinguishing Haiku-appropriate from Opus-appropriate work
- `WitnessLog` makes routing decisions semantically searchable — cost attribution and routing audits become a vector query instead of grep
- Zero regression path — `ruvllm.enabled = false` (default) leaves the existing driver layer completely untouched
- Phase 2 SONA learning adapts factor weights per agent workload profile without any manual tuning

### Negative

- `ruvllm.rvf` adds a fourth RVF file to manage
- Complexity estimation adds ~1ms synchronous overhead per request before the API call — negligible for most workloads but relevant for high-frequency tool calls
- When `manifest_ceiling = true`, manifests that hardcode a model name bypass the router's tier selection entirely — operators need to update manifests to use family names rather than specific model IDs to get full routing benefit

### Neutral

- Remote API call semantics are unchanged — the router rewrites `request.model` but the driver makes the same HTTP call
- Local inference (`CandleBackend`, GGUF) is a separate future ADR; this ADR does not change the hardware or deployment requirements for OpenFang
- `ModelRouter` in `openfang-runtime` is retained but bypassed when `ruvllm.enabled = true`; it can be removed in a later cleanup pass

---

## References

- `ADR-002-memory-engine-integration.md` — current driver layer description
- `ADR-009-memory-intelligence.md` — SONA framework this ADR's Phase 2 extends
- `crates/ruvllm/src/lib.rs` in ruvector-upstream — `RuvLLMEngine`, `RuvLLMConfig` (lines 694–994)
- `crates/ruvllm/src/claude_flow/model_router.rs` — `ComplexityAnalyzer`, `ModelRouter`, 7-factor weights
- `crates/ruvllm/src/moe/mod.rs` — `MemoryAwareRouter`, `ExpertAffinity` (Phase 2, local inference)
- `crates/ruvllm/src/optimization/mod.rs` — `SonaLlm`, `SonaIntegration` (Phase 2)
- `crates/openfang-runtime/src/llm_driver.rs` in openfang-ai — `LlmDriver` trait being implemented

---

## Amendment 0.6: §8 Crate Vendoring Superseded — 2026-03-21

**ADR-014 (Vendor Code Ownership) accepted and executed.** §8 "Crate Vendoring" is superseded.

**What changed**: `vendor/ruvllm/` was a placeholder skeleton with no types (documented T-10 blocker). Rather than vendoring the full upstream `ruvllm` crate (which would require vendoring `ruvector-core` as a massive additional dep), OpenFang-AI implemented `crates/openfang-routing/` as a first-party owned crate per ADR-014.

**New implementation location**: `crates/openfang-routing/` (not `vendor/ruvllm/`)

**What is implemented** (20 tests passing, 2026-03-21):
- `ModelTier { Haiku, Sonnet, Opus }` — `#[serde(rename_all = "lowercase")]`, `PartialOrd`
- `TaskType { General, Compaction, CodeGeneration, SecurityAudit }`
- `ComplexityFactors` — 7 normalised [0,1] fields
- `ComplexityWeights` — ADR-010 §2 defaults (token=0.20, reasoning=0.25, domain=0.20, code=0.15, planning=0.10, security=0.05, perf=0.05 — sums to 1.0)
- `ComplexityScore { overall, factors, tier, confidence }`
- `ComplexityAnalyzer::analyze(prompt, task_type, context_tokens)` + `score_factors(factors)` (deterministic, < 1ms)
- `ModelRouter::route_by_score(score, ceiling)` — ceiling downgrades only, never upgrades
- `RoutingConfig` — serde-deserializable, `enabled: false` default

**openfang-runtime wiring** (from PLAN-003 Round 7, Codex): `ruvllm = ["dep:openfang-routing"]` is live in `crates/openfang-runtime/Cargo.toml`.

**Remaining T-10 work**: `RuvLLMRouter` struct in `ruvllm_router.rs` (wraps `Arc<dyn LlmDriver>` + `ComplexityAnalyzer`) and `resolve_driver()` kernel wiring — all types imported from `openfang_routing::`, none redefined.

**`vendor/` no longer exists.** PLAN-003 Rounds 1–7 complete.

---

## Amendment 0.5: ruvector-upstream Sync — 2026-03-17

### 1. ruvllm MoE routing buffer reuse optimization landed

Commit `20c620b5` (2026-03-16, "perf(ruvllm): optimize MoE routing with buffer reuse and optional metrics") made two changes to `crates/ruvllm/src/moe/router.rs` that affect the `MemoryAwareRouter` documented in ADR-010 §7.

**P0 — Pre-allocated result buffer (`result_buffer` field on `MemoryAwareRouter`):**

A `result_buffer: Vec<ExpertId>` field (capacity pre-allocated to `top_k` at construction time) was added to `MemoryAwareRouter`. The `select_top_k_buffered()` method was changed from:

```rust
self.index_buffer.iter().take(k).map(|(id, _)| *id).collect()
```

to filling `result_buffer` directly and returning it via `std::mem::take`:

```rust
self.result_buffer.extend(self.index_buffer.iter().take(k).map(|(id, _)| *id));
std::mem::take(&mut self.result_buffer)
```

The `select_top_2_unrolled()` fast path (`fn select_top_2_unrolled(&mut self)`) was similarly changed from `vec![best.0, second.0]` to pushing into and taking from `result_buffer`. Expected allocation savings: 1–2µs per routing call. This is relevant to the ADR-010 §7 claim that `MemoryAwareRouter` achieves 70% cache hit rate with <1% accuracy loss — the buffer reuse affects latency, not selection accuracy, so the accuracy claim is unaffected.

**P1 — Optional `routing-metrics` feature flag:**

`Instant::now()` and all cache-hit/miss tracking inside `route()` are now conditional on `#[cfg(feature = "routing-metrics")]`. The feature is enabled by default. Production builds that disable it avoid the `Instant::now()` syscall overhead (~0.04–0.08µs per call). The commit message documents this as a `Cargo.toml` feature addition to `crates/ruvllm/Cargo.toml`.

**Implication for ADR-010 Phase 2 vendoring:** When `ruvllm` is vendored with `default = ["routing-metrics"]` (as planned in §8), the metrics feature is active and `MemoryAwareRouter` behaves as documented. If a stripped vendor build disables `routing-metrics`, the witness log's cache hit/miss counts will not be populated — `WitnessEntry` latency tracking will still work (that is in `RuvLLMRouter`, not the MoE layer), but MoE-specific cache residency metrics will be absent. This is only relevant if local inference (and thus `MemoryAwareRouter`) is enabled in a future ADR.

**The 70% cache hit rate and <1% accuracy loss claims (ADR-010 §7)** are unchanged. The buffer reuse optimization is a latency improvement to the hot path, not an algorithmic change to expert selection. The `cache_bonus: 0.15` default in `RouterConfig` that achieves the 70% hit rate target was not modified.

### 2. mcp-brain-server trainer now available

`crates/mcp-brain-server/src/trainer.rs` (landed 2026-03-16, 1015 lines) defines the `BrainTrainer` struct and `TrainerConfig` that operate the brain server's SONA learning pipeline. This is directly relevant to ADR-010 §6 (Phase 2 SONA adaptive learning on routing), because the brain server trainer is the mechanism through which routing decisions can be fed back as training data.

Key types in the trainer (first 200 lines):

- **`BrainTrainer`** — holds a `TrainerConfig` and `reqwest::Client`. Entry point: `run_training_cycle() -> TrainingCycleReport` (async).
- **`TrainerConfig`** — `min_confidence: f64` (default 0.70), `max_per_cycle: usize` (default 100), `duplicate_threshold: f64` (cosine similarity, default 0.95), `active_domains: Vec<DiscoveryDomain>`, `trigger_sona: bool` (default true), `submit_lora: bool` (default true), `api_delay_ms: u64` (default 1000).
- **`TrainingCycleReport`** — records `sona_cycles_triggered: usize`, `knowledge_velocity_before: f64`, `knowledge_velocity_after: f64`, and `discoveries_ingested: usize`. These are the metrics that would surface as SONA training signals for routing quality.
- **`DiscoveryDomain`** enum: `SpaceScience`, `EarthScience`, `AcademicResearch`, `EconomicsFinance`, `MedicalGenomics`, `MaterialsPhysics`.
- **`Discovery`** struct: carries a `witness_hash: Option<String>` for provenance (witness chain attestation).

The trainer's `trigger_sona: bool` and `submit_lora: bool` flags in `TrainerConfig` correspond directly to the two-phase SONA learning ADR-010 §6 specifies: `trigger_sona` maps to the instant/background adaptation tiers; `submit_lora` maps to LoRA delta submission. The trainer's `duplicate_threshold: 0.95` cosine similarity gate prevents the same routing pattern from being ingested multiple times — relevant to the Phase 2 concern about SONA overfitting on repeated workloads.

**For ADR-010 Phase 2**: `BrainTrainer` is the server-side counterpart to the client-side `SonaIntegration::post_query()` call in `RuvLLMRouter`. When routing decisions are logged to the `WitnessLog` (`ruvllm.rvf`), they can be packaged as `Discovery` records and submitted to the brain trainer's `/v1/memories` + `/v1/train` cycle. The trainer's `min_confidence: 0.70` threshold aligns with the quality filter described in ADR-009 §1.2 (`QUALITY_THRESHOLD: 0.7`).

### 3. Common Crawl adapter — forward reference for domain expertise scoring

`crates/mcp-brain-server/src/web_ingest.rs` and `web_memory.rs` implement the Common Crawl adapter that populates the brain server's knowledge base with domain coverage data. This is speculative context for ADR-010, not a committed design change.

The `domain_expertise` factor in ADR-010's 7-factor complexity scoring (weight 0.20, §2) currently relies on the `ComplexityAnalyzer`'s static heuristics. The brain server's Common Crawl index could, in Phase 2+, become a runtime input to domain expertise scoring: a query touching a domain well-represented in the brain server's crawl data would score lower on `domain_expertise` (the model already has broad coverage) than a query touching a rare or specialised domain (where the model would need to reason more carefully). This is a forward reference only — it requires the `domain_expertise` factor weight computation to be made dynamic, which is not in scope for Phase 1 or Phase 2 as currently specified. It is noted here as an integration opportunity for a future amendment.

### 4. ADR-011 Phase 2 gate reminder

Constraint 3 (typed Queue semantics for inter-agent routing signal sharing) from ADR-011 applies to all SONA routing Phase 2 work. Specifically: the `SonaIntegration::post_query()` callback in `RuvLLMRouter::complete()` (ADR-010 §6, step 6) must not share training signals between agents via direct memory access. If multiple agents share a `RuvLLMRouter` instance (or if a shared `SonaLlm` learner is introduced), all cross-agent signal paths must use typed Queue message boundaries with `ruvix-types::WitTypeId` schemas. The `BrainTrainer`'s HTTP-based submission (`POST /v1/memories`) is a valid Queue boundary for inter-agent signal sharing — each agent submits independently; the brain server aggregates.

### 5. ⚠️ DECISION: Re-validate Phase 2 performance targets against updated MemoryAwareRouter

ADR-010 §7 states: `MemoryAwareRouter` achieves 70% cache hit rate vs. 34% baseline with <1% accuracy loss. These figures originate from the ruvllm `RouterConfig` documentation (`cache_bonus: 0.15` default, `crates/ruvllm/src/moe/router.rs` lines 214–215).

Commit `20c620b5` changed the performance characteristics of `MemoryAwareRouter` in two ways:

1. **Latency improved** by 1–2µs per routing call (buffer reuse, eliminated `collect()` allocation).
2. **Metrics tracking is now conditional** on `#[cfg(feature = "routing-metrics")]` — the 26 MoE router tests all pass, but any benchmarks that measured hit rate by observing `MoeMetrics` struct data need to confirm they are compiled with `routing-metrics` enabled, or the hit/miss counters will read zero.

**Question**: Should ADR-010's Phase 2 performance targets (70% cache hit rate, <1% accuracy loss) be re-validated against the updated implementation before Phase 2 begins?

The accuracy loss claim is unlikely to be affected — buffer reuse does not change the selection algorithm. The cache hit rate claim could be affected only if the `routing-metrics` feature flag is inadvertently disabled during Phase 2 benchmarking (producing false zero hit counts). The latency improvement is a net positive.

**Recommended resolution**: The existing targets (70% cache hit rate, <1% accuracy loss) remain valid as algorithmic claims. Before Phase 2 benchmarking, confirm that the vendored `ruvllm` includes `routing-metrics` in its default feature set. No formal re-validation of the targets is required unless benchmarks during Phase 2 produce results that diverge from the stated figures — at which point the measurement methodology (feature flag state) should be checked first.

**Decision: existing targets (70% cache hit rate, <1% accuracy loss) remain valid as algorithmic claims.** Before Phase 2 benchmarking, confirm `routing-metrics` is in the vendored `ruvllm` default feature set. No re-validation required unless Phase 2 benchmarks diverge — in which case, check feature flag state first. Resolved 2026-03-20.
