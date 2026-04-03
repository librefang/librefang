# ADR-019: Routing Threshold Calibration via EMA Feedback

**Status**: Accepted
**Date**: 2026-03-21
**Authors**: Daniel Alberttis

---

## Context

### Background — the full TinyDancer stack

`@ruvector/tiny-dancer` is a compiled Rust binary distributed as a platform-specific npm package (e.g. `@ruvector/tiny-dancer-darwin-arm64`). The full Rust source lives in `ruvector-upstream/crates/ruvector-tiny-dancer-core/`. It is a production-grade **FastGRNN neural routing system** — not just an EMA heuristic. Key components:

- **`FastGRNN`** (Fast Gated Recurrent Neural Network) — `input_dim=5`, `hidden_dim=8`, `output_dim=1`, low-rank factorization, quantization support. Weights saved as safetensors. Sub-millisecond inference.
- **Feature engineering** — 5 scalar features per candidate: semantic similarity, recency score, frequency score, success rate, metadata overlap. Computed via SIMD-accelerated cosine similarity (`simsimd` crate).
- **Uncertainty estimation** — conformal prediction; returns uncertainty estimate alongside confidence so callers can decide when to trigger multi-model fallback.
- **Circuit breaker** — opens after N consecutive failures; routes fall through to fallback until the breaker resets.
- **Training pipeline** — Adam optimizer + BPTT + knowledge distillation. Needs a labeled dataset of `(feature_vector, correct_routing_decision)` tuples.
- **SQLite persistence** — routing history, model snapshots, and candidate embeddings persisted via `rusqlite`.

In `agentic-qe`, the `NeuralTinyDancerRouter` wraps the rule-based `TinyDancerRouter` with this compiled binary. The rule-based layer handles cold-start; once enough outcomes are collected, the neural model takes over.

**Why OpenFang cannot plug `ruvector-tiny-dancer-core` in directly today:**

The FastGRNN model requires a *trained* weights file (safetensors). There is no pre-trained model for OpenFang's specific routing problem (7-factor complexity vector → haiku/sonnet/opus). The feature engineering in `tiny-dancer-core` is designed for a *candidate selection* problem (pick the best agent from N candidates), not a *threshold classification* problem (is this complexity score above/below threshold?). Adapting it requires:
1. Re-engineering the input features from 5 generic features to OpenFang's 7 routing factors
2. Collecting a labeled training dataset from real routing outcomes (which do not exist yet)
3. Running the training pipeline and validating the model
4. Wiring the model file path into `KernelConfig`

This is 3–4 weeks of work minimum. **ADR-019 implements the EMA calibration layer first — the immediate online feedback mechanism.** The FastGRNN upgrade path is documented in §9 below as a future ADR once the training dataset exists.

---

### The current gap

ADR-010 built the `RuvLLMRouter` — a complexity-scoring wrapper that classifies each request and routes it to a model tier (Haiku / Sonnet / Opus). Two sub-decisions of ADR-010 are now live:

- **T-11** (`ComplexityAnalyzer`) — 7-factor scoring; `haiku_threshold` and `sonnet_threshold` from `RoutingConfig` determine tier boundaries
- **T-12** (`WitnessLog`) — every routing decision is persisted to `data_dir/ruvllm/witness.rvf`
- **Routing outcome feedback** (post T-12 addendum, PLAN-002) — every completed request pushes `(factor_vector, tier, reward)` into `ExperienceReplayBuffer` at `data_dir/ruvllm/experience/sona.rvf`

**The remaining gap**: `haiku_threshold` and `sonnet_threshold` are static. They are set at startup from `KernelConfig` and never updated. If a provider's Haiku model degrades (higher error rate at a certain complexity level), the router keeps sending requests to it at the same rate. The reward signal in `ExperienceReplayBuffer` captures this drift but nothing reads it back.

**Reference implementation — `agentic-qe` (TypeScript)**:

`agentic-qe/src/routing/tiny-dancer-router.ts` implements per-tier success tracking via `getSuccessRateByModel()`, accumulating `(success, total)` counts per tier from recorded outcomes. `agentic-qe/src/routing/calibration/ema-calibrator.ts` implements EMA calibration (alpha=0.1, min 10 outcomes) originally for per-agent voting weights. `agentic-qe/src/learning/regret-tracker.ts` runs log-log regression on cumulative regret per domain to classify growth rate (sublinear = learning, linear = stagnation, superlinear = getting worse).

OpenFang's design adapts these three concepts into a single Rust component — `TierCalibrator` — with one key difference from the TypeScript version: where `agentic-qe` adjusts voting *weights* for multi-agent consensus, OpenFang's calibrator adjusts routing *thresholds* directly, because OpenFang has no multi-agent consensus layer — it has a single `RuvLLMRouter` and two configurable thresholds.

**Why the thresholds are the right lever**: the complexity score for a request is fixed (it depends only on the prompt). What changes is how that score maps to a tier. Raising `haiku_threshold` means fewer requests qualify as "simple enough for Haiku" — the tier boundary moves, not the score. This is exactly the right knob when Haiku is failing: route less to it until its success rate recovers.

---

## Decision

### 1. Introduce `TierCalibrator` in `openfang-routing`

A new struct `TierCalibrator` lives in `crates/openfang-routing/src/calibrator.rs`. It is feature-gated on `features = ["routing-adapt"]` — the same pattern used for `shared-phase2` and `sona-adapt` in `openfang-memory`.

```rust
pub struct TierCalibrator {
    config: CalibratorConfig,
    state: Arc<Mutex<CalibratorState>>,
}

pub struct CalibratorConfig {
    /// EMA smoothing factor. Default: 0.1 (matches agentic-qe EMACalibrator).
    pub alpha: f64,
    /// Minimum outcomes per tier before calibration adjusts thresholds.
    /// Below this count, neutral (no adjustment). Default: 50.
    pub min_outcomes: usize,
    /// Trigger a calibration pass every N total outcomes across all tiers.
    pub calibration_interval: usize,
    /// Maximum allowed single-step threshold adjustment (fractional). Default: 0.05 (5%).
    pub max_adjustment_fraction: f64,
    /// Absolute bounds for haiku_threshold: [floor, ceiling]. Default: [0.05, 0.40].
    pub haiku_bounds: (f64, f64),
    /// Absolute bounds for sonnet_threshold: [floor, ceiling]. Default: [0.40, 0.80].
    pub sonnet_bounds: (f64, f64),
    /// Target success rate floor. If EMA drops below this, raise threshold. Default: 0.75.
    pub success_floor: f64,
    /// Target success rate ceiling. If EMA exceeds this, lower threshold. Default: 0.95.
    pub success_ceiling: f64,
}

pub struct CalibratorState {
    /// EMA of reward per tier: "haiku" | "sonnet" | "opus".
    pub ema_per_tier: HashMap<String, f64>,
    /// Total outcomes seen per tier.
    pub outcomes_per_tier: HashMap<String, usize>,
    /// Cumulative regret per tier (for health diagnostics).
    pub cumulative_regret: HashMap<String, f64>,
    /// Total outcomes across all tiers since last calibration pass.
    pub outcomes_since_last_calibration: usize,
    /// Current live thresholds (may differ from initial RoutingConfig).
    pub haiku_threshold: f64,
    pub sonnet_threshold: f64,
    /// Timestamp of last calibration pass (Unix ms).
    pub last_calibrated_ms: u64,
}
```

### 2. The EMA update rule

On every `record_outcome(tier: &str, reward: f64)` call:

```
ema[tier] = alpha * reward + (1 - alpha) * ema[tier]
```

On first observation for a tier: `ema[tier] = reward` (no warmup bias).

Regret for a decision: `regret = 1.0 - reward` (perfect routing = 0 regret, failed routing = 1.0 regret). Cumulative regret per tier is tracked for the health dashboard (see §6).

### 3. Calibration pass — threshold adjustment rule

A calibration pass runs every `calibration_interval` total outcomes (default: 50). It is synchronous and runs under the `CalibratorState` mutex. Cost: O(1) arithmetic — no RVF read on the hot path.

For each threshold:

```
if outcomes[tier] < min_outcomes:
    skip (not enough data)

if ema[tier] < success_floor:
    # Tier is underperforming — route fewer requests to it
    # Raise the threshold: requests must score LOWER to qualify
    delta = current_threshold * max_adjustment_fraction
    new_threshold = clamp(current_threshold + delta, bounds.floor, bounds.ceiling)

elif ema[tier] > success_ceiling:
    # Tier is overperforming — it can handle more traffic
    # Lower the threshold: more requests qualify
    delta = current_threshold * max_adjustment_fraction
    new_threshold = clamp(current_threshold - delta, bounds.floor, bounds.ceiling)

else:
    # Within target band — no adjustment
```

The `haiku_threshold` is adjusted based on `ema["haiku"]`. The `sonnet_threshold` is adjusted based on `ema["sonnet"]`. Opus has no threshold to adjust (it is the ceiling tier).

**Constraint — haiku/sonnet ordering**: after any adjustment, enforce `haiku_threshold < sonnet_threshold`. If the adjustment would violate this, clamp the adjusted value to `other_threshold ± 0.05`. This prevents the degenerate state where the two thresholds cross.

### 4. `RuvLLMRouter` integration

`TierCalibrator` is held behind `Arc<RwLock<CalibratorState>>` in `RuvLLMRouter`:

```rust
calibrator: Option<TierCalibrator>,
```

In `complete()`, after the existing experience buffer push:

```rust
if let Some(cal) = &self.calibrator {
    cal.record_outcome(&tier_name, reward);
    if let Some((new_haiku, new_sonnet)) = cal.maybe_calibrate() {
        // Update live thresholds in the analyzer — no lock needed on hot path,
        // calibrator holds its own state lock and returns new values only
        // when a calibration pass actually ran.
        self.analyzer.set_thresholds(new_haiku, new_sonnet);
        self.router.set_thresholds(new_haiku, new_sonnet);
        tracing::info!(
            haiku_threshold = new_haiku,
            sonnet_threshold = new_sonnet,
            "ruvllm threshold calibration updated"
        );
    }
}
```

`maybe_calibrate()` returns `Option<(f64, f64)>` — `None` unless a calibration pass actually ran this call. This means zero overhead on the hot path for the 49 calls between calibration passes.

`ComplexityAnalyzer` and `ModelRouter` need a `set_thresholds(haiku: f64, sonnet: f64)` method — updating their internal `haiku_threshold` / `opus_threshold` fields. No reconstruction, no reallocation. This is an `&mut self` method; since `RuvLLMRouter::complete()` takes `&self`, `ComplexityAnalyzer` and `ModelRouter` must be wrapped in `Mutex<_>` when `routing-adapt` is enabled. The existing `Mutex<ExperienceReplayBuffer>` pattern is the precedent.

### 5. Persistence — `calibration.json` sidecar

Calibration state is persisted to `data_dir/ruvllm/calibration.json` (plain JSON, not RVF — the state is a small key-value struct, not a vector space). Written synchronously after every calibration pass. Loaded at startup in `RuvLLMRouter::new()` if the file exists.

Format:

```json
{
  "ema_per_tier": { "haiku": 0.87, "sonnet": 0.91, "opus": 1.0 },
  "outcomes_per_tier": { "haiku": 312, "sonnet": 88, "opus": 7 },
  "cumulative_regret": { "haiku": 40.3, "sonnet": 7.9, "opus": 0.0 },
  "haiku_threshold": 0.22,
  "sonnet_threshold": 0.61,
  "last_calibrated_ms": 1742567834000
}
```

On startup with a saved state: thresholds resume from the saved values (not from `KernelConfig`). `KernelConfig` thresholds become the initial values only on first run or after a manual reset. This matches the agentic-qe `EMACalibrator.deserialize()` pattern.

**Manual reset**: `DELETE <data_dir>/ruvllm/calibration.json` reverts to `KernelConfig` thresholds on next restart. No API route required — operator action.

### 6. Regret health diagnostics

`TierCalibrator` exposes a `health_summary() -> CalibrationHealth` method:

```rust
pub struct CalibrationHealth {
    pub per_tier: HashMap<String, TierHealth>,
}

pub struct TierHealth {
    pub tier: String,
    pub ema_success_rate: f64,
    pub outcomes_total: usize,
    pub cumulative_regret: f64,
    /// "sublinear" | "linear" | "superlinear" | "insufficient_data"
    pub regret_growth_rate: RegretGrowthRate,
    pub current_threshold: Option<f64>,
}
```

`regret_growth_rate` is computed via log-log OLS regression on the stored `(decision_count, cumulative_regret)` series, identical to `agentic-qe/src/learning/regret-tracker.ts`:

- Slope < 0.9 → `Sublinear` (calibrator is learning)
- Slope 0.9–1.1 → `Linear` (stagnation — thresholds may be stuck at bounds)
- Slope > 1.1 → `Superlinear` (getting worse — alert warranted)
- < 50 points → `InsufficientData`

This is surfaced via a new API endpoint `GET /api/ruvllm/calibration` (registered only when `routing-adapt` feature is active). No new kernel config required — the endpoint reads from `TierCalibrator::health_summary()` through the kernel handle.

### 7. Feature gate and `KernelConfig`

New feature: `routing-adapt` in `openfang-routing/Cargo.toml`.
Propagated to `openfang-runtime`: `ruvllm-adapt = ["dep:openfang-routing/routing-adapt"]`.

New config section in `KernelConfig`:

```toml
[routing.calibration]
enabled = false          # opt-in
alpha = 0.1
min_outcomes = 50
calibration_interval = 50
max_adjustment_fraction = 0.05
haiku_bounds = [0.05, 0.40]
sonnet_bounds = [0.40, 0.80]
success_floor = 0.75
success_ceiling = 0.95
```

`enabled = false` by default — calibration is opt-in. When disabled, `RuvLLMRouter` holds `calibrator: None` and behaviour is identical to pre-ADR-019.

### 8. `ExperienceReplayBuffer` relationship

`TierCalibrator` does NOT read from `ExperienceReplayBuffer`. The two systems are parallel write targets from `RuvLLMRouter::complete()`:

- `ExperienceReplayBuffer` — RVF-backed circular buffer for future offline RL training (full Q-learning tuples, persisted)
- `TierCalibrator` — in-memory EMA state with JSON sidecar for online threshold adaptation (scalar EMA per tier, lightweight)

This separation is intentional. The replay buffer accumulates data for a future training loop that may use more sophisticated algorithms (e.g. DQN, PPO). The calibrator is the immediate online feedback mechanism. They share the same `(tier, reward)` signal but serve different consumers.

---

## Consequences

### Positive
- Thresholds adapt to real-world model performance without operator intervention
- EMA is numerically stable and O(1) per outcome — no overhead on the hot path
- Calibration interval (default 50) amortizes the mutex acquisition cost
- Persistence survives restarts — no cold-start penalty after first burn-in period
- Regret health diagnostics give operators a signal that the system is actually learning
- Feature-gated: zero impact on deployments that do not opt in

### Negative
- `ComplexityAnalyzer` and `ModelRouter` require a `set_thresholds()` mutation method, which means wrapping them in `Mutex` when `routing-adapt` is active — adds a lock on the routing hot path
- The 5% max adjustment per calibration pass means slow convergence if thresholds are badly misconfigured initially (by design — prevents oscillation)
- Calibration state diverges from `KernelConfig` after first run; operators who edit `KernelConfig` thresholds post-deployment will see their changes overridden by the saved state unless they delete `calibration.json`

### Neutral
- The `ExperienceReplayBuffer` wire (post T-12 addendum) is not changed — both systems coexist
- `WitnessLog` is not involved — calibration state has its own sidecar
- `openfang-sona` is not involved — `TierCalibrator` is a routing-layer concern and lives in `openfang-routing`

---

## Alternatives Considered

**A — Use `ExperienceReplayBuffer` samples as the calibration signal**
Rejected for the online path. Reading from RVF on every calibration pass (50 requests) adds I/O on the routing hot path. The in-memory EMA in `TierCalibrator` achieves the same signal without disk reads. The replay buffer remains available for future offline RL.

**B — Port `agentic-qe` EMACalibrator directly (adjust voting weights, not thresholds)**
Rejected. OpenFang has no multi-agent consensus layer — there are no voting weights to adjust. The correct lever in a single-router system is the threshold itself.

**C — Adjust thresholds on every outcome (no calibration interval)**
Rejected. EMA already smooths individual samples, but acquiring the `Mutex<CalibratorState>` + writing `calibration.json` on every single request adds latency on the hot path. Batching to every 50 outcomes is the right trade-off.

**D — Use a separate background task for calibration**
Considered. A tokio background task polling the replay buffer periodically avoids any mutex on the hot path entirely. Deferred to a follow-up — requires additional runtime wiring. The synchronous `maybe_calibrate()` approach is simpler and sufficient for the current traffic volumes.

**E — Persist calibration state to RVF instead of JSON**
Rejected. Calibration state is a small flat struct (< 1KB), not a vector space. RVF's HNSW overhead is wasted here. JSON sidecar is readable by operators without tooling.

---

## Implementation Gate

**Status**: Accepted, not yet implemented. Implementation begins when a PLAN is written referencing this ADR.

- [ ] `TierCalibrator` struct + `record_outcome()` + `maybe_calibrate()` in `openfang-routing/src/calibrator.rs`
- [ ] `ComplexityAnalyzer::set_thresholds()` + `ModelRouter::set_thresholds()` mutation methods
- [ ] `routing-adapt` feature gate in `openfang-routing/Cargo.toml`
- [ ] `ruvllm-adapt` feature gate in `openfang-runtime/Cargo.toml`
- [ ] `calibrator: Option<TierCalibrator>` field in `RuvLLMRouter`; `record_outcome` + `maybe_calibrate` call in `complete()`
- [ ] `calibration.json` load on startup, write after every calibration pass
- [ ] `[routing.calibration]` config section in `KernelConfig` with `enabled = false` default
- [ ] `CalibrationHealth` + `TierHealth` + `RegretGrowthRate` types; `health_summary()` method
- [ ] `GET /api/ruvllm/calibration` route (feature-gated)
- [ ] Tests: EMA converges correctly after N outcomes; thresholds respect bounds; ordering constraint enforced; `min_outcomes` gate respected; `calibration.json` round-trip; regret growth rate classification (sublinear/linear/superlinear)
- [ ] `cargo test --workspace` exit zero, clippy exit zero

---

### 9. FastGRNN upgrade path (future ADR)

Once `TierCalibrator` has collected sufficient routing history (target: 500+ labeled outcomes per tier from real production traffic), a follow-on ADR should:

1. **Re-engineer the input features** — map OpenFang's 7-factor complexity vector to `FeatureVector` in `ruvector-tiny-dancer-core`:
   - `semantic_similarity` ← cosine similarity of request embedding to tier centroid embedding
   - `recency_score` ← time decay on last N requests to this tier
   - `frequency_score` ← tier utilization fraction over rolling window
   - `success_rate` ← current EMA from `TierCalibrator` (available immediately)
   - `metadata_overlap` ← ceiling_applied flag + task_type match score

2. **Generate a training dataset** — export `(feature_vector, tier_label, reward)` tuples from the accumulated `ExperienceReplayBuffer` and `WitnessLog`

3. **Train a FastGRNN model** using `ruvector-tiny-dancer-core`'s training pipeline (Adam, BPTT, knowledge distillation from `ComplexityAnalyzer` as the teacher model)

4. **Wire the model into `RuvLLMRouter`** — `TierCalibrator` becomes the fallback path when the circuit breaker opens or training data is insufficient; `FastGRNN` takes primary routing when the model is loaded

5. **Add the `ruvector-tiny-dancer-core` crate** to openfang's workspace — it is already locally available at `ruvector-upstream/crates/ruvector-tiny-dancer-core/` and depends only on workspace-compatible crates (`ndarray`, `rusqlite`, `parking_lot`, `simsimd`)

This upgrade path is additive: `TierCalibrator` is not removed, it becomes the circuit-breaker fallback for the neural router.

---

## Related ADRs

- **ADR-010** — LLM intelligence layer (`RuvLLMRouter`, `ComplexityAnalyzer`, `WitnessLog`)
- **ADR-009** — SONA self-learning (`ExperienceReplayBuffer`, `NeuralPatternStore`)
- **ADR-008** — Shared store abuse resistance (feature-gate pattern reference)
- **ADR-013** — Development workflow (mandatory 10-step gate for implementation)

## Reference Implementation

- `ruvector-upstream/crates/ruvector-tiny-dancer-core/` — full FastGRNN Rust source (local); `src/model.rs` (FastGRNN), `src/feature_engineering.rs` (5-feature extraction), `src/router.rs` (circuit breaker + inference), `src/training.rs` (Adam + BPTT + distillation), `src/uncertainty.rs` (conformal prediction)
- `agentic-qe/node_modules/@ruvector/tiny-dancer/` — compiled binary (v0.1.17), `index.d.ts` for full API surface
- `agentic-qe/src/routing/tiny-dancer-router.ts` — per-tier success rate tracking, `recordOutcome()`
- `agentic-qe/src/routing/calibration/ema-calibrator.ts` — EMA calibration (alpha=0.1, min 10 outcomes, weight floor/ceiling)
- `agentic-qe/src/learning/regret-tracker.ts` — log-log OLS regression for regret growth rate classification (sublinear threshold=0.9, superlinear threshold=1.1, min 50 points)
