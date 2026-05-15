//! Metering engine — tracks LLM cost and enforces spending quotas.

use librefang_llm_driver::exhaustion::{
    ExhaustionReason, ProviderExhaustionStore, DEFAULT_LONG_BACKOFF,
};
use librefang_memory::usage::{ModelUsage, UsageRecord, UsageStore, UsageSummary};
use librefang_types::agent::{AgentId, ResourceQuota, UserId};
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::model_catalog::ModelCatalogEntry;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const DEFAULT_INPUT_COST_PER_M: f64 = 1.0;
const DEFAULT_OUTPUT_COST_PER_M: f64 = 3.0;

/// In-flight USD cost reservation ledger (#3616).
///
/// `check_global_budget` reads spent cost from the SQLite store, which only
/// reflects calls that have already settled. When N triggers fire
/// concurrently they all observe the same pre-call total, all pass the
/// gate, and all commit — producing several-x overshoots of
/// `max_hourly_usd` / `max_daily_usd`.
///
/// This ledger holds an *estimated* cost for every in-flight LLM call so
/// the budget gate can include pending spend in its decision. Reserve
/// before the network call, settle (or release) after.
#[derive(Debug, Default)]
struct CostReservationLedger {
    /// Total reserved USD across all in-flight calls.
    reserved_usd: Mutex<f64>,
}

impl CostReservationLedger {
    fn current(&self) -> f64 {
        *self.reserved_usd.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn add(&self, usd: f64) {
        if usd <= 0.0 {
            return;
        }
        let mut g = self.reserved_usd.lock().unwrap_or_else(|e| e.into_inner());
        *g += usd;
    }

    /// Subtract a previously-reserved amount. Clamped at 0 to defend
    /// against floating-point drift or double-release bugs.
    fn release(&self, usd: f64) {
        if usd <= 0.0 {
            return;
        }
        let mut g = self.reserved_usd.lock().unwrap_or_else(|e| e.into_inner());
        *g = (*g - usd).max(0.0);
    }
}

/// Token returned by [`MeteringEngine::reserve_global_budget`]; on drop /
/// settle it releases the matching reservation. The kernel calls
/// [`MeteringReservation::settle`] once the actual usage record is in
/// hand so the in-memory ledger doesn't double-count alongside the
/// settled SQLite row.
#[derive(Debug)]
#[must_use = "a budget reservation must be settled or released"]
pub struct MeteringReservation {
    ledger: Arc<CostReservationLedger>,
    estimated_usd: f64,
    settled: bool,
}

impl MeteringReservation {
    /// Settle the reservation. Call once the LLM response is recorded.
    pub fn settle(mut self) {
        self.ledger.release(self.estimated_usd);
        self.settled = true;
    }

    /// Release without settling (call failed before any cost was incurred).
    pub fn release(mut self) {
        self.ledger.release(self.estimated_usd);
        self.settled = true;
    }

    /// Estimated USD cost held by this reservation.
    pub fn estimated_usd(&self) -> f64 {
        self.estimated_usd
    }
}

impl Drop for MeteringReservation {
    fn drop(&mut self) {
        if !self.settled {
            // Defensive: a panic between reserve and settle still releases the slot.
            self.ledger.release(self.estimated_usd);
        }
    }
}

/// The metering engine tracks usage cost and enforces quota limits.
pub struct MeteringEngine {
    /// Persistent usage store (SQLite-backed).
    store: Arc<UsageStore>,
    /// In-memory ledger of pre-charged but not-yet-settled USD cost (#3616).
    pending: Arc<CostReservationLedger>,
    /// Optional shared provider-exhaustion store (#4807). When set,
    /// per-provider budget breaches (operator caps in `[budget.providers]`)
    /// flag the provider as exhausted so the fallback chain skips it
    /// instead of attempting the call and getting a fresh quota error.
    exhaustion: Option<ProviderExhaustionStore>,
}

impl MeteringEngine {
    /// Create a new metering engine with the given usage store.
    pub fn new(store: Arc<UsageStore>) -> Self {
        Self {
            store,
            pending: Arc::new(CostReservationLedger::default()),
            exhaustion: None,
        }
    }

    /// Attach a shared provider-exhaustion store (#4807). When set, the
    /// metering engine marks a provider as `BudgetExceeded` whenever its
    /// operator-set per-provider budget gate trips, so the LLM fallback
    /// chain skips that slot for [`DEFAULT_LONG_BACKOFF`] without first
    /// dispatching a request that the gate would only deny again.
    ///
    /// The store is cheap-clone — pass the same instance the
    /// `FallbackChain` uses so both layers observe a coherent view.
    pub fn with_exhaustion_store(mut self, store: ProviderExhaustionStore) -> Self {
        self.exhaustion = Some(store);
        self
    }

    /// Return a clone of the attached exhaustion store, when one is wired.
    /// Used by callers that need to seed the same store into other layers
    /// (e.g. an `AuxClient` built after the metering engine).
    pub fn exhaustion_store(&self) -> Option<ProviderExhaustionStore> {
        self.exhaustion.clone()
    }

    /// Mark a provider as budget-exhausted on the attached store, if any.
    /// No-op when no exhaustion store is wired (legacy callers). Centralised
    /// here so every "budget refused this provider" site uses the same
    /// reason / backoff combo.
    fn flag_provider_budget_exhausted(&self, provider: &str) {
        if provider.is_empty() {
            return;
        }
        if let Some(store) = &self.exhaustion {
            tracing::info!(
                target: "metering",
                event = "provider_budget_exhausted",
                provider = %provider,
                "operator budget cap reached; flagging provider in exhaustion store"
            );
            store.mark_exhausted(
                provider,
                ExhaustionReason::BudgetExceeded,
                Some(Instant::now() + DEFAULT_LONG_BACKOFF),
            );
        }
    }

    /// Reserve an estimated USD cost against the global budget *before*
    /// dispatching an LLM call (#3616).
    ///
    /// Returns a [`MeteringReservation`] holding the reserved amount.
    /// Callers must call `settle` (after recording the actual usage) or
    /// `release` (on dispatch failure) — `Drop` releases as a safety net.
    ///
    /// Atomicity: this only synchronises in-process callers. Two
    /// processes (or a process + an out-of-band SQL writer) can still
    /// race; matching the SQLite atomicity of `check_all_and_record` is
    /// the responsibility of the post-call settle path.
    pub fn reserve_global_budget(
        &self,
        budget: &librefang_types::config::BudgetConfig,
        estimated_usd: f64,
    ) -> LibreFangResult<MeteringReservation> {
        let pending = self.pending.current();

        // Use ">" not ">=" so a fresh kernel with a single call exactly
        // at the limit isn't rejected before it's ever recorded.
        if budget.max_hourly_usd > 0.0 {
            let spent = self.store.query_global_hourly()?;
            let projected = spent + pending + estimated_usd.max(0.0);
            if projected > budget.max_hourly_usd {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global hourly budget would be exceeded: \
                     spent ${:.4} + pending ${:.4} + this call ${:.4} > limit ${:.4}",
                    spent, pending, estimated_usd, budget.max_hourly_usd
                )));
            }
        }
        if budget.max_daily_usd > 0.0 {
            let spent = self.store.query_today_cost()?;
            let projected = spent + pending + estimated_usd.max(0.0);
            if projected > budget.max_daily_usd {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global daily budget would be exceeded: \
                     spent ${:.4} + pending ${:.4} + this call ${:.4} > limit ${:.4}",
                    spent, pending, estimated_usd, budget.max_daily_usd
                )));
            }
        }
        if budget.max_monthly_usd > 0.0 {
            let spent = self.store.query_global_monthly()?;
            let projected = spent + pending + estimated_usd.max(0.0);
            if projected > budget.max_monthly_usd {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global monthly budget would be exceeded: \
                     spent ${:.4} + pending ${:.4} + this call ${:.4} > limit ${:.4}",
                    spent, pending, estimated_usd, budget.max_monthly_usd
                )));
            }
        }

        self.pending.add(estimated_usd.max(0.0));
        Ok(MeteringReservation {
            ledger: Arc::clone(&self.pending),
            estimated_usd: estimated_usd.max(0.0),
            settled: false,
        })
    }

    /// Currently-pending (reserved-but-not-settled) USD across all callers.
    /// Exposed for diagnostics and tests.
    pub fn pending_reserved_usd(&self) -> f64 {
        self.pending.current()
    }

    /// Record a usage event (persists to SQLite).
    pub fn record(&self, record: &UsageRecord) -> LibreFangResult<()> {
        self.store.record(record)
    }

    /// Check if an agent is within its spending quotas (hourly, daily, monthly).
    /// Returns Ok(()) if under all quotas, or QuotaExceeded error if over any.
    pub fn check_quota(&self, agent_id: AgentId, quota: &ResourceQuota) -> LibreFangResult<()> {
        // Hourly check
        if quota.max_cost_per_hour_usd > 0.0 {
            let hourly_cost = self.store.query_hourly(agent_id)?;
            if hourly_cost >= quota.max_cost_per_hour_usd {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded hourly cost quota: ${:.4} / ${:.4}",
                    agent_id, hourly_cost, quota.max_cost_per_hour_usd
                )));
            }
        }

        // Daily check
        if quota.max_cost_per_day_usd > 0.0 {
            let daily_cost = self.store.query_daily(agent_id)?;
            if daily_cost >= quota.max_cost_per_day_usd {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded daily cost quota: ${:.4} / ${:.4}",
                    agent_id, daily_cost, quota.max_cost_per_day_usd
                )));
            }
        }

        // Monthly check
        if quota.max_cost_per_month_usd > 0.0 {
            let monthly_cost = self.store.query_monthly(agent_id)?;
            if monthly_cost >= quota.max_cost_per_month_usd {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded monthly cost quota: ${:.4} / ${:.4}",
                    agent_id, monthly_cost, quota.max_cost_per_month_usd
                )));
            }
        }

        Ok(())
    }

    /// Check global budget limits (across all agents).
    ///
    /// Includes any in-flight pending cost held by
    /// [`Self::reserve_global_budget`] so concurrent trigger fires don't
    /// all see the same pre-call total and collectively overshoot the
    /// configured cap (#3616).
    ///
    /// Uses `>=` (reject at limit) rather than `>` (reject past limit).
    /// [`reserve_global_budget`] uses `>` so a single call that exactly
    /// reaches the cap is still allowed through; this post-call check
    /// uses `>=` so once the limit is fully consumed no further calls
    /// are dispatched. The asymmetry is intentional.
    pub fn check_global_budget(
        &self,
        budget: &librefang_types::config::BudgetConfig,
    ) -> LibreFangResult<()> {
        let pending = self.pending.current();
        if budget.max_hourly_usd > 0.0 {
            let cost = self.store.query_global_hourly()? + pending;
            if cost >= budget.max_hourly_usd {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global hourly budget exceeded: ${:.4} / ${:.4}",
                    cost, budget.max_hourly_usd
                )));
            }
        }

        if budget.max_daily_usd > 0.0 {
            let cost = self.store.query_today_cost()? + pending;
            if cost >= budget.max_daily_usd {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global daily budget exceeded: ${:.4} / ${:.4}",
                    cost, budget.max_daily_usd
                )));
            }
        }

        if budget.max_monthly_usd > 0.0 {
            let cost = self.store.query_global_monthly()? + pending;
            if cost >= budget.max_monthly_usd {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global monthly budget exceeded: ${:.4} / ${:.4}",
                    cost, budget.max_monthly_usd
                )));
            }
        }

        Ok(())
    }

    /// Get budget status — current spend vs limits for all time windows.
    pub fn budget_status(&self, budget: &librefang_types::config::BudgetConfig) -> BudgetStatus {
        let hourly = self.store.query_global_hourly().unwrap_or(0.0);
        let daily = self.store.query_today_cost().unwrap_or(0.0);
        let monthly = self.store.query_global_monthly().unwrap_or(0.0);

        BudgetStatus {
            hourly_spend: hourly,
            hourly_limit: budget.max_hourly_usd,
            hourly_pct: if budget.max_hourly_usd > 0.0 {
                hourly / budget.max_hourly_usd
            } else {
                0.0
            },
            daily_spend: daily,
            daily_limit: budget.max_daily_usd,
            daily_pct: if budget.max_daily_usd > 0.0 {
                daily / budget.max_daily_usd
            } else {
                0.0
            },
            monthly_spend: monthly,
            monthly_limit: budget.max_monthly_usd,
            monthly_pct: if budget.max_monthly_usd > 0.0 {
                monthly / budget.max_monthly_usd
            } else {
                0.0
            },
            alert_threshold: budget.alert_threshold,
            default_max_llm_tokens_per_hour: budget.default_max_llm_tokens_per_hour,
        }
    }

    /// Get a usage summary, optionally filtered by agent.
    pub fn get_summary(&self, agent_id: Option<AgentId>) -> LibreFangResult<UsageSummary> {
        self.store.query_summary(agent_id)
    }

    /// Get usage grouped by model.
    pub fn get_by_model(&self) -> LibreFangResult<Vec<ModelUsage>> {
        self.store.query_by_model()
    }

    /// Estimate the cost of an LLM call based on model and token counts.
    ///
    /// Pricing table (approximate, per million tokens):
    ///
    /// | Model Family          | Input $/M | Output $/M |
    /// |-----------------------|-----------|------------|
    /// | claude-haiku          |     0.80  |      4.00  |
    /// | claude-sonnet-4-6     |     3.00  |     15.00  |
    /// | claude-opus-4-6       |     5.00  |     25.00  |
    /// | claude-opus (legacy)  |    15.00  |     75.00  |
    /// | gpt-5.2(-pro)         |     1.75  |     14.00  |
    /// | gpt-5(.1)             |     1.25  |     10.00  |
    /// | gpt-5-mini            |     0.25  |      2.00  |
    /// | gpt-5-nano            |     0.05  |      0.40  |
    /// | gpt-4o                |     2.50  |     10.00  |
    /// | gpt-4o-mini           |     0.15  |      0.60  |
    /// | gpt-4.1               |     2.00  |      8.00  |
    /// | gpt-4.1-mini          |     0.40  |      1.60  |
    /// | gpt-4.1-nano          |     0.10  |      0.40  |
    /// | o3-mini               |     1.10  |      4.40  |
    /// | gemini-3.1            |     2.50  |     15.00  |
    /// | gemini-3              |     0.50  |      3.00  |
    /// | gemini-2.5-flash-lite |     0.04  |      0.15  |
    /// | gemini-2.5-pro        |     1.25  |     10.00  |
    /// | gemini-2.5-flash      |     0.15  |      0.60  |
    /// | gemini-2.0-flash      |     0.10  |      0.40  |
    /// | deepseek-chat/v3      |     0.27  |      1.10  |
    /// | deepseek-reasoner/r1  |     0.55  |      2.19  |
    /// | llama-4-maverick      |     0.50  |      0.77  |
    /// | llama-4-scout         |     0.11  |      0.34  |
    /// | llama/mixtral (groq)  |     0.05  |      0.10  |
    /// | grok-4.1              |     0.20  |      0.50  |
    /// | grok-4                |     3.00  |     15.00  |
    /// | grok-3                |     3.00  |     15.00  |
    /// | qwen                  |     0.20  |      0.60  |
    /// | mistral-large         |     2.00  |      6.00  |
    /// | mistral-small         |     0.10  |      0.30  |
    /// | command-r-plus        |     2.50  |     10.00  |
    /// | alibaba-coding-plan   |subscription| (request-based quota) |
    /// | Default (unknown)     |     1.00  |      3.00  |
    ///
    /// **Subscription-based providers** (e.g., alibaba-coding-plan):
    /// These providers use request-based quotas instead of token-based billing.
    /// Models are registered with zero cost-per-token, so cost tracking in metering
    /// will show $0.00. Users should monitor usage via the provider's console.
    ///
    /// For alibaba-coding-plan specifically:
    /// - Pricing: $50/month (subscription)
    /// - Quotas: 90,000 requests/month, 45,000/week, 6,000 per 5 hours (sliding window)
    /// - Token usage: Still tracked for analytics, but cost = $0
    ///
    /// Estimate cost using default rates ($1/$3 per million tokens).
    ///
    /// Prefer [`estimate_cost_with_catalog`] which reads pricing from the
    /// model catalog.  This method exists as a fallback when no catalog is
    /// available (e.g. unit tests).
    pub fn estimate_cost(
        _model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_input_tokens: u64,
        cache_creation_input_tokens: u64,
    ) -> f64 {
        estimate_cost_from_rates(
            input_tokens,
            output_tokens,
            cache_read_input_tokens,
            cache_creation_input_tokens,
            DEFAULT_INPUT_COST_PER_M,
            DEFAULT_OUTPUT_COST_PER_M,
        )
    }

    /// Estimate cost using the model catalog as the pricing source.
    ///
    /// Falls back to the default rate ($1/$3 per million) if the model is not
    /// found in the catalog.
    pub fn estimate_cost_with_catalog(
        catalog: &librefang_runtime::model_catalog::ModelCatalog,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_input_tokens: u64,
        cache_creation_input_tokens: u64,
    ) -> f64 {
        if let Some(entry) = catalog.find_model(model) {
            let input_per_m = entry.input_cost_per_m;
            let output_per_m = entry.output_cost_per_m;

            // ChatGPT session-auth models do not expose billable catalog pricing,
            // but budgets still need a conservative non-zero estimate.
            if input_per_m == 0.0 && output_per_m == 0.0 && should_use_legacy_budget_estimate(entry)
            {
                return estimate_cost_from_rates(
                    input_tokens,
                    output_tokens,
                    cache_read_input_tokens,
                    cache_creation_input_tokens,
                    DEFAULT_INPUT_COST_PER_M,
                    DEFAULT_OUTPUT_COST_PER_M,
                );
            }

            return estimate_cost_from_rates(
                input_tokens,
                output_tokens,
                cache_read_input_tokens,
                cache_creation_input_tokens,
                input_per_m,
                output_per_m,
            );
        }

        estimate_cost_from_rates(
            input_tokens,
            output_tokens,
            cache_read_input_tokens,
            cache_creation_input_tokens,
            DEFAULT_INPUT_COST_PER_M,
            DEFAULT_OUTPUT_COST_PER_M,
        )
    }

    /// Atomically check per-agent quotas and record usage in a single SQLite
    /// transaction.  This closes the TOCTOU race between `check_quota` and
    /// `record` — no other writer can sneak in between the check and the
    /// insert.
    pub fn check_quota_and_record(
        &self,
        record: &UsageRecord,
        quota: &ResourceQuota,
    ) -> LibreFangResult<()> {
        self.store.check_quota_and_record(
            record,
            quota.max_cost_per_hour_usd,
            quota.max_cost_per_day_usd,
            quota.max_cost_per_month_usd,
        )
    }

    /// Atomically check global budget limits and record usage in a single
    /// SQLite transaction.
    pub fn check_global_budget_and_record(
        &self,
        record: &UsageRecord,
        budget: &librefang_types::config::BudgetConfig,
    ) -> LibreFangResult<()> {
        self.store.check_global_budget_and_record(
            record,
            budget.max_hourly_usd,
            budget.max_daily_usd,
            budget.max_monthly_usd,
        )
    }

    /// Atomically check both per-agent quotas and global budget limits, then
    /// record the usage event — all within a single SQLite transaction.
    ///
    /// This is the preferred method for recording usage after an LLM call,
    /// as it prevents the race condition where concurrent requests both pass
    /// the quota check before either records its usage.
    pub fn check_all_and_record(
        &self,
        record: &UsageRecord,
        quota: &ResourceQuota,
        budget: &librefang_types::config::BudgetConfig,
    ) -> LibreFangResult<()> {
        // Resolve the per-provider budget for the record's provider (if any).
        let provider_budget = if record.provider.is_empty() {
            None
        } else {
            budget.providers.get(&record.provider)
        };

        self.store.check_all_with_provider_and_record(
            record,
            quota.max_cost_per_hour_usd,
            quota.max_cost_per_day_usd,
            quota.max_cost_per_month_usd,
            budget.max_hourly_usd,
            budget.max_daily_usd,
            budget.max_monthly_usd,
            provider_budget
                .map(|p| p.max_cost_per_hour_usd)
                .unwrap_or(0.0),
            provider_budget
                .map(|p| p.max_cost_per_day_usd)
                .unwrap_or(0.0),
            provider_budget
                .map(|p| p.max_cost_per_month_usd)
                .unwrap_or(0.0),
            provider_budget.map(|p| p.max_tokens_per_hour).unwrap_or(0),
        )
    }

    /// Check a per-provider budget in isolation (non-atomic, for pre-dispatch
    /// gating or dashboards).
    ///
    /// Zero limits are treated as "unlimited" and are skipped.
    ///
    /// When the gate refuses a provider AND an exhaustion store is attached
    /// (see [`Self::with_exhaustion_store`]), the provider is also marked
    /// as `BudgetExceeded` for [`DEFAULT_LONG_BACKOFF`] so the LLM fallback
    /// chain skips it on subsequent calls without re-dispatching (#4807).
    pub fn check_provider_budget(
        &self,
        provider: &str,
        budget: &librefang_types::config::ProviderBudget,
    ) -> LibreFangResult<()> {
        if provider.is_empty() {
            return Ok(());
        }

        if budget.max_cost_per_hour_usd > 0.0 {
            let cost = self.store.query_provider_hourly(provider)?;
            if cost >= budget.max_cost_per_hour_usd {
                self.flag_provider_budget_exhausted(provider);
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Provider '{}' exceeded hourly cost budget: ${:.4} / ${:.4}",
                    provider, cost, budget.max_cost_per_hour_usd
                )));
            }
        }

        if budget.max_cost_per_day_usd > 0.0 {
            let cost = self.store.query_provider_daily(provider)?;
            if cost >= budget.max_cost_per_day_usd {
                self.flag_provider_budget_exhausted(provider);
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Provider '{}' exceeded daily cost budget: ${:.4} / ${:.4}",
                    provider, cost, budget.max_cost_per_day_usd
                )));
            }
        }

        if budget.max_cost_per_month_usd > 0.0 {
            let cost = self.store.query_provider_monthly(provider)?;
            if cost >= budget.max_cost_per_month_usd {
                self.flag_provider_budget_exhausted(provider);
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Provider '{}' exceeded monthly cost budget: ${:.4} / ${:.4}",
                    provider, cost, budget.max_cost_per_month_usd
                )));
            }
        }

        if budget.max_tokens_per_hour > 0 {
            let tokens = self.store.query_provider_tokens_hourly(provider)?;
            if tokens >= budget.max_tokens_per_hour {
                self.flag_provider_budget_exhausted(provider);
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Provider '{}' exceeded hourly token budget: {} / {}",
                    provider, tokens, budget.max_tokens_per_hour
                )));
            }
        }

        Ok(())
    }

    /// RBAC M5: check a per-user spending budget.
    ///
    /// Post-call: invoked AFTER `check_all_and_record` succeeds (the cost
    /// of the just-finished call is already in the rolled-up totals
    /// returned by `query_user_*`). Mirrors the global / per-agent /
    /// per-provider semantics — the LLM call already happened, so
    /// "exceeded" means the *next* call from this user must be denied,
    /// not that the current one is rolled back.
    ///
    /// Each window with a `0.0` limit is treated as unlimited and
    /// skipped. Returns `QuotaExceeded` when any non-zero window has
    /// already been crossed.
    pub fn check_user_budget(
        &self,
        user_id: UserId,
        budget: &librefang_types::config::UserBudgetConfig,
    ) -> LibreFangResult<()> {
        if budget.max_hourly_usd > 0.0 {
            let cost = self.store.query_user_hourly(user_id)?;
            if cost >= budget.max_hourly_usd {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "User {} exceeded hourly cost budget: ${:.4} / ${:.4}",
                    user_id, cost, budget.max_hourly_usd
                )));
            }
        }

        if budget.max_daily_usd > 0.0 {
            let cost = self.store.query_user_daily(user_id)?;
            if cost >= budget.max_daily_usd {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "User {} exceeded daily cost budget: ${:.4} / ${:.4}",
                    user_id, cost, budget.max_daily_usd
                )));
            }
        }

        if budget.max_monthly_usd > 0.0 {
            let cost = self.store.query_user_monthly(user_id)?;
            if cost >= budget.max_monthly_usd {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "User {} exceeded monthly cost budget: ${:.4} / ${:.4}",
                    user_id, cost, budget.max_monthly_usd
                )));
            }
        }

        Ok(())
    }

    /// Clean up old usage records.
    pub fn cleanup(&self, days: u32) -> LibreFangResult<usize> {
        self.store.cleanup_old(days)
    }
}

fn should_use_legacy_budget_estimate(entry: &ModelCatalogEntry) -> bool {
    entry.provider == "chatgpt"
}

fn estimate_cost_from_rates(
    input_tokens: u64,
    output_tokens: u64,
    cache_read_input_tokens: u64,
    cache_creation_input_tokens: u64,
    input_per_m: f64,
    output_per_m: f64,
) -> f64 {
    // Regular input tokens = total input minus cache tokens
    let regular_input =
        input_tokens.saturating_sub(cache_read_input_tokens + cache_creation_input_tokens);
    let regular_input_cost = (regular_input as f64 / 1_000_000.0) * input_per_m;

    // Cache-read tokens are priced at 10% of input price
    let cache_read_cost = (cache_read_input_tokens as f64 / 1_000_000.0) * input_per_m * 0.10;

    // Cache-creation tokens are priced at 125% of input price
    let cache_creation_cost =
        (cache_creation_input_tokens as f64 / 1_000_000.0) * input_per_m * 1.25;

    let output_cost = (output_tokens as f64 / 1_000_000.0) * output_per_m;
    regular_input_cost + cache_read_cost + cache_creation_cost + output_cost
}

/// Budget status snapshot — current spend vs limits for all time windows.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BudgetStatus {
    pub hourly_spend: f64,
    pub hourly_limit: f64,
    pub hourly_pct: f64,
    pub daily_spend: f64,
    pub daily_limit: f64,
    pub daily_pct: f64,
    pub monthly_spend: f64,
    pub monthly_limit: f64,
    pub monthly_pct: f64,
    pub alert_threshold: f64,
    /// Global default token limit per agent per hour (0 = use per-agent values).
    pub default_max_llm_tokens_per_hour: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_memory::MemorySubstrate;

    fn setup() -> MeteringEngine {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = Arc::new(UsageStore::new(substrate.pool()));
        MeteringEngine::new(store)
    }

    fn test_catalog() -> librefang_runtime::model_catalog::ModelCatalog {
        let home = librefang_runtime::registry_sync::resolve_home_dir_for_tests();
        librefang_runtime::model_catalog::ModelCatalog::new(&home)
    }

    #[test]
    fn test_record_and_check_quota_under() {
        let engine = setup();
        let agent_id = AgentId::new();
        let quota = ResourceQuota {
            max_cost_per_hour_usd: 1.0,
            ..Default::default()
        };

        engine
            .record(&UsageRecord {
                agent_id,
                provider: String::new(),
                model: "claude-haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.001,
                tool_calls: 0,
                latency_ms: 150,
                ..Default::default()
            })
            .unwrap();

        assert!(engine.check_quota(agent_id, &quota).is_ok());
    }

    #[test]
    fn test_check_quota_exceeded() {
        let engine = setup();
        let agent_id = AgentId::new();
        let quota = ResourceQuota {
            max_cost_per_hour_usd: 0.01,
            ..Default::default()
        };

        engine
            .record(&UsageRecord {
                agent_id,
                provider: String::new(),
                model: "claude-sonnet".to_string(),
                input_tokens: 10000,
                output_tokens: 5000,
                cost_usd: 0.05,
                tool_calls: 0,
                latency_ms: 300,
                ..Default::default()
            })
            .unwrap();

        let result = engine.check_quota(agent_id, &quota);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("exceeded hourly cost quota"));
    }

    #[test]
    fn test_check_quota_zero_limit_skipped() {
        let engine = setup();
        let agent_id = AgentId::new();
        let quota = ResourceQuota {
            max_cost_per_hour_usd: 0.0,
            ..Default::default()
        };

        // Even with high usage, a zero limit means no enforcement
        engine
            .record(&UsageRecord {
                agent_id,
                provider: String::new(),
                model: "claude-opus".to_string(),
                input_tokens: 100000,
                output_tokens: 50000,
                cost_usd: 100.0,
                tool_calls: 0,
                latency_ms: 500,
                ..Default::default()
            })
            .unwrap();

        assert!(engine.check_quota(agent_id, &quota).is_ok());
    }

    #[test]
    fn test_estimate_cost_unknown() {
        let cost = MeteringEngine::estimate_cost("my-custom-model", 1_000_000, 1_000_000, 0, 0);
        assert!((cost - 4.0).abs() < 0.01); // $1.00 + $3.00
    }

    /// Build a synthetic two-entry catalog (canonical id + alias) so the
    /// next two tests don't depend on registry state. They previously
    /// hardcoded a specific Sonnet version id against the live catalog
    /// and broke the moment the registry retired it — same anti-pattern
    /// the `_chatgpt_zero_price_*` test already moved away from.
    fn synthetic_priced_catalog() -> librefang_runtime::model_catalog::ModelCatalog {
        use librefang_types::model_catalog::{ModelCatalogEntry, ModelCatalogFile, ModelTier};
        let mut catalog = librefang_runtime::model_catalog::ModelCatalog::new_from_dir(
            &std::path::PathBuf::from("/nonexistent"),
        );
        catalog.merge_catalog_file(ModelCatalogFile {
            provider: None,
            models: vec![ModelCatalogEntry {
                id: "synthetic-priced-frontier".to_string(),
                display_name: "Synthetic Priced Frontier".to_string(),
                provider: "anthropic".to_string(),
                tier: ModelTier::Smart,
                context_window: 200_000,
                max_output_tokens: 64_000,
                input_cost_per_m: 3.0,
                output_cost_per_m: 15.0,
                supports_tools: true,
                supports_vision: true,
                supports_streaming: true,
                supports_thinking: false,
                aliases: vec!["synthetic-priced-alias".to_string()],
                ..Default::default()
            }],
        });
        catalog
    }

    #[test]
    fn test_estimate_cost_with_catalog() {
        let catalog = synthetic_priced_catalog();
        // 1M input * $3/M + 1M output * $15/M = $18.
        let cost = MeteringEngine::estimate_cost_with_catalog(
            &catalog,
            "synthetic-priced-frontier",
            1_000_000,
            1_000_000,
            0,
            0,
        );
        assert!((cost - 18.0).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_with_catalog_alias() {
        let catalog = synthetic_priced_catalog();
        // Alias should resolve to the same pricing as the canonical id.
        let cost = MeteringEngine::estimate_cost_with_catalog(
            &catalog,
            "synthetic-priced-alias",
            1_000_000,
            1_000_000,
            0,
            0,
        );
        assert!((cost - 18.0).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_with_catalog_unknown_uses_default() {
        let catalog = test_catalog();
        // Unknown model falls back to $1/$3
        let cost = MeteringEngine::estimate_cost_with_catalog(
            &catalog,
            "totally-unknown-model",
            1_000_000,
            1_000_000,
            0,
            0,
        );
        assert!((cost - 4.0).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_with_catalog_chatgpt_zero_price_uses_legacy_budget_rate() {
        // Build a synthetic catalog with a zero-priced chatgpt model so the test
        // is independent of registry state (the live registry may carry real prices).
        use librefang_types::model_catalog::{ModelCatalogEntry, ModelCatalogFile, ModelTier};
        let mut catalog = librefang_runtime::model_catalog::ModelCatalog::new_from_dir(
            &std::path::PathBuf::from("/nonexistent"),
        );
        catalog.merge_catalog_file(ModelCatalogFile {
            provider: None,
            models: vec![ModelCatalogEntry {
                id: "gpt-5.1-codex-mini".to_string(),
                display_name: "GPT-5.1 Codex Mini".to_string(),
                provider: "chatgpt".to_string(),
                tier: ModelTier::Balanced,
                context_window: 32_000,
                max_output_tokens: 4_096,
                input_cost_per_m: 0.0,
                output_cost_per_m: 0.0,
                supports_tools: true,
                supports_vision: false,
                supports_streaming: true,
                supports_thinking: false,
                aliases: vec![],
                ..Default::default()
            }],
        });
        let cost = MeteringEngine::estimate_cost_with_catalog(
            &catalog,
            "gpt-5.1-codex-mini",
            1_000_000,
            1_000_000,
            0,
            0,
        );
        // Zero-priced chatgpt model falls back to legacy rates ($1/$3 per million).
        assert!((cost - 4.0).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_with_catalog_local_zero_price_stays_zero() {
        let catalog = test_catalog();
        // Use a local model that always has zero cost; pick dynamically so this
        // stays green regardless of which specific models the registry ships.
        let local_id = catalog
            .list_models()
            .iter()
            .find(|m| m.tier == librefang_types::model_catalog::ModelTier::Local)
            .expect("registry must contain at least one local-tier model")
            .id
            .clone();
        let cost = MeteringEngine::estimate_cost_with_catalog(
            &catalog, &local_id, 1_000_000, 1_000_000, 0, 0,
        );
        assert!(cost.abs() < f64::EPSILON);
    }

    #[test]
    fn test_estimate_cost_cache_read_discount() {
        // estimate_cost uses default rates: $1/M input, $3/M output
        // 1M total input tokens, 500k are cache-read (10% of input price)
        // Regular input: 500k * $1/M = $0.50
        // Cache read: 500k * $1/M * 0.10 = $0.05
        // Output: 1M * $3/M = $3.00
        // Total = $3.55
        let cost = MeteringEngine::estimate_cost(
            "test-model", // estimate_cost is catalog-agnostic; id is just a label
            1_000_000,    // total input
            1_000_000,    // output
            500_000,      // cache_read
            0,            // cache_creation
        );
        assert!((cost - 3.55).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_cache_creation_surcharge() {
        // estimate_cost uses default rates: $1/M input, $3/M output
        // 1M total input tokens, 200k are cache-creation (125% of input price)
        // Regular input: 800k * $1/M = $0.80
        // Cache creation: 200k * $1/M * 1.25 = $0.25
        // Output: 1M * $3/M = $3.00
        // Total = $4.05
        let cost = MeteringEngine::estimate_cost(
            "test-model", // estimate_cost is catalog-agnostic; id is just a label
            1_000_000,    // total input
            1_000_000,    // output
            0,            // cache_read
            200_000,      // cache_creation
        );
        assert!((cost - 4.05).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_cache_mixed() {
        // estimate_cost uses default rates: $1/M input, $3/M output
        // 1M total input, 400k cache-read, 100k cache-creation, 500k regular
        // Regular input: 500k * $1/M = $0.50
        // Cache read: 400k * $1/M * 0.10 = $0.04
        // Cache creation: 100k * $1/M * 1.25 = $0.125
        // Output: 1M * $3/M = $3.00
        // Total = $3.665
        let cost = MeteringEngine::estimate_cost(
            "test-model", // estimate_cost is catalog-agnostic; id is just a label
            1_000_000,    // total input
            1_000_000,    // output
            400_000,      // cache_read
            100_000,      // cache_creation
        );
        assert!((cost - 3.665).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_zero_cache_matches_no_cache() {
        // estimate_cost uses default rates: $1/M input, $3/M output
        // With zero cache tokens, should match the original behavior
        let cost_with_cache =
            MeteringEngine::estimate_cost("test-model", 1_000_000, 1_000_000, 0, 0);
        let expected = 4.00; // $1.00 + $3.00
        assert!((cost_with_cache - expected).abs() < 0.01);
    }

    #[test]
    fn test_get_summary() {
        let engine = setup();
        let agent_id = AgentId::new();

        engine
            .record(&UsageRecord {
                agent_id,
                provider: String::new(),
                model: "haiku".to_string(),
                input_tokens: 500,
                output_tokens: 200,
                cost_usd: 0.005,
                tool_calls: 3,
                latency_ms: 100,
                ..Default::default()
            })
            .unwrap();

        let summary = engine.get_summary(Some(agent_id)).unwrap();
        assert_eq!(summary.call_count, 1);
        assert_eq!(summary.total_input_tokens, 500);
    }

    // ── Per-provider budget tests (issue #2316) ────────────────────

    fn record_for_provider(engine: &MeteringEngine, provider: &str, cost: f64, tokens: u64) {
        engine
            .record(&UsageRecord {
                agent_id: AgentId::new(),
                provider: provider.to_string(),
                model: "test-model".to_string(),
                input_tokens: tokens,
                output_tokens: 0,
                cost_usd: cost,
                tool_calls: 0,
                latency_ms: 50,
                ..Default::default()
            })
            .unwrap();
    }

    #[test]
    fn test_check_provider_budget_under_limit() {
        let engine = setup();
        record_for_provider(&engine, "moonshot", 0.50, 1_000);

        let budget = librefang_types::config::ProviderBudget {
            max_cost_per_hour_usd: 0.0,
            max_cost_per_day_usd: 2.0,
            max_cost_per_month_usd: 0.0,
            max_tokens_per_hour: 0,
        };
        assert!(engine.check_provider_budget("moonshot", &budget).is_ok());
    }

    #[test]
    fn test_check_provider_budget_over_limit() {
        let engine = setup();
        record_for_provider(&engine, "moonshot", 2.50, 1_000);

        let budget = librefang_types::config::ProviderBudget {
            max_cost_per_day_usd: 2.0,
            ..Default::default()
        };
        let err = engine
            .check_provider_budget("moonshot", &budget)
            .unwrap_err()
            .to_string();
        assert!(err.contains("moonshot"), "err: {err}");
        assert!(err.contains("daily cost budget"), "err: {err}");
    }

    #[test]
    fn test_check_provider_budget_zero_limit_skipped() {
        let engine = setup();
        record_for_provider(&engine, "litellm", 999.0, 10_000_000);

        // All zeros => unlimited, should pass despite huge usage.
        let budget = librefang_types::config::ProviderBudget::default();
        assert!(engine.check_provider_budget("litellm", &budget).is_ok());
    }

    #[test]
    fn test_check_provider_budget_separate_providers_isolated() {
        let engine = setup();
        // Burn budget on moonshot only.
        record_for_provider(&engine, "moonshot", 5.0, 1_000);

        let tight = librefang_types::config::ProviderBudget {
            max_cost_per_day_usd: 1.0,
            ..Default::default()
        };
        // moonshot is over.
        assert!(engine.check_provider_budget("moonshot", &tight).is_err());
        // litellm has no usage — must not be affected.
        assert!(engine.check_provider_budget("litellm", &tight).is_ok());
    }

    #[test]
    fn test_check_provider_budget_tokens_per_hour() {
        let engine = setup();
        record_for_provider(&engine, "moonshot", 0.01, 600_000);

        let budget = librefang_types::config::ProviderBudget {
            max_tokens_per_hour: 500_000,
            ..Default::default()
        };
        let err = engine
            .check_provider_budget("moonshot", &budget)
            .unwrap_err()
            .to_string();
        assert!(err.contains("token budget"), "err: {err}");
    }

    #[test]
    fn test_check_all_and_record_enforces_provider_budget() {
        let engine = setup();
        let agent_id = AgentId::new();

        // Pre-seed moonshot usage so any new record trips the daily cap.
        record_for_provider(&engine, "moonshot", 1.95, 0);

        let quota = ResourceQuota::default();
        let mut budget = librefang_types::config::BudgetConfig::default();
        budget.providers.insert(
            "moonshot".to_string(),
            librefang_types::config::ProviderBudget {
                max_cost_per_day_usd: 2.0,
                ..Default::default()
            },
        );

        // This record + existing spend would exceed the cap.
        let record = UsageRecord {
            agent_id,
            provider: "moonshot".to_string(),
            model: "kimi".to_string(),
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.10,
            tool_calls: 0,
            latency_ms: 10,
            ..Default::default()
        };
        let err = engine
            .check_all_and_record(&record, &quota, &budget)
            .unwrap_err()
            .to_string();
        assert!(err.contains("moonshot"), "err: {err}");

        // The atomic check must NOT insert the record on failure.
        let summary = engine.get_summary(Some(agent_id)).unwrap();
        assert_eq!(summary.call_count, 0);
    }

    #[test]
    fn test_check_all_and_record_free_provider_unaffected() {
        let engine = setup();
        let agent_id = AgentId::new();

        // Huge existing spend on moonshot should not affect litellm.
        record_for_provider(&engine, "moonshot", 100.0, 0);

        let quota = ResourceQuota::default();
        let mut budget = librefang_types::config::BudgetConfig::default();
        budget.providers.insert(
            "moonshot".to_string(),
            librefang_types::config::ProviderBudget {
                max_cost_per_day_usd: 2.0,
                ..Default::default()
            },
        );
        // litellm deliberately has no provider budget configured.

        let record = UsageRecord {
            agent_id,
            provider: "litellm".to_string(),
            model: "llama".to_string(),
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.0,
            tool_calls: 0,
            latency_ms: 10,
            ..Default::default()
        };
        assert!(engine
            .check_all_and_record(&record, &quota, &budget)
            .is_ok());
    }

    // ── RBAC M5: per-user budget enforcement ─────────────────────────

    #[test]
    fn test_check_user_budget_under_limit_passes() {
        let engine = setup();
        let alice = UserId::from_name("Alice");
        engine
            .record(&UsageRecord {
                agent_id: AgentId::new(),
                cost_usd: 0.10,
                user_id: Some(alice),
                ..Default::default()
            })
            .unwrap();
        let budget = librefang_types::config::UserBudgetConfig {
            max_hourly_usd: 1.0,
            max_daily_usd: 10.0,
            max_monthly_usd: 100.0,
            alert_threshold: 0.8,
        };
        assert!(engine.check_user_budget(alice, &budget).is_ok());
    }

    #[test]
    fn test_check_user_budget_zero_means_unlimited() {
        // 0.0 on every window MUST be treated as "no cap" — even after
        // a record larger than any plausible cap, the check passes.
        let engine = setup();
        let alice = UserId::from_name("Alice");
        engine
            .record(&UsageRecord {
                agent_id: AgentId::new(),
                cost_usd: 9_999.0,
                user_id: Some(alice),
                ..Default::default()
            })
            .unwrap();
        let budget = librefang_types::config::UserBudgetConfig::default();
        assert_eq!(budget.max_hourly_usd, 0.0);
        assert!(engine.check_user_budget(alice, &budget).is_ok());
    }

    #[test]
    fn test_check_user_budget_exceeds_hourly() {
        let engine = setup();
        let alice = UserId::from_name("Alice");
        engine
            .record(&UsageRecord {
                agent_id: AgentId::new(),
                cost_usd: 0.50,
                user_id: Some(alice),
                ..Default::default()
            })
            .unwrap();
        let budget = librefang_types::config::UserBudgetConfig {
            max_hourly_usd: 0.10,
            max_daily_usd: 0.0,
            max_monthly_usd: 0.0,
            alert_threshold: 0.8,
        };
        let err = engine.check_user_budget(alice, &budget).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("hourly"), "expected 'hourly' in '{msg}'");
        assert!(msg.contains(&alice.to_string()), "expected uid in '{msg}'");
    }

    #[test]
    fn test_check_user_budget_isolates_users() {
        // Bob's spend MUST NOT count against Alice's cap.
        let engine = setup();
        let alice = UserId::from_name("Alice");
        let bob = UserId::from_name("Bob");
        engine
            .record(&UsageRecord {
                agent_id: AgentId::new(),
                cost_usd: 5.0,
                user_id: Some(bob),
                ..Default::default()
            })
            .unwrap();
        let budget = librefang_types::config::UserBudgetConfig {
            max_hourly_usd: 1.0,
            max_daily_usd: 0.0,
            max_monthly_usd: 0.0,
            alert_threshold: 0.8,
        };
        assert!(engine.check_user_budget(alice, &budget).is_ok());
        assert!(engine.check_user_budget(bob, &budget).is_err());
    }

    /// Regression for #3616: parallel pre-call gates must see each other's
    /// in-flight USD reservations. Previously every concurrent caller saw
    /// the same pre-call total and all of them passed `check_global_budget`,
    /// committing several-x the configured cap.
    #[test]
    fn pre_call_reservations_block_concurrent_overshoot() {
        let engine = setup();
        let budget = librefang_types::config::BudgetConfig {
            max_hourly_usd: 1.0,
            ..Default::default()
        };

        let r1 = engine.reserve_global_budget(&budget, 0.6).unwrap();
        // The second caller observes r1's pending hold; 0.6 + 0.6 = 1.2 > 1.0.
        let err = engine
            .reserve_global_budget(&budget, 0.6)
            .expect_err("second concurrent reservation must reject");
        let msg = err.to_string();
        assert!(
            msg.contains("hourly budget would be exceeded"),
            "unexpected error: {msg}"
        );
        // After releasing r1, the slot opens up again.
        r1.release();
        assert_eq!(engine.pending_reserved_usd(), 0.0);
        let r2 = engine.reserve_global_budget(&budget, 0.6).unwrap();
        r2.settle();
    }

    /// Reservations must release on drop so a panic between reserve and
    /// settle doesn't permanently lock the budget down.
    #[test]
    fn reservation_releases_on_drop() {
        let engine = setup();
        let budget = librefang_types::config::BudgetConfig {
            max_hourly_usd: 1.0,
            ..Default::default()
        };
        {
            let _r = engine.reserve_global_budget(&budget, 0.5).unwrap();
            assert!((engine.pending_reserved_usd() - 0.5).abs() < 1e-9);
        }
        assert_eq!(engine.pending_reserved_usd(), 0.0);
    }

    /// Zero-budget config must not gate at all — reservations are no-ops
    /// for callers running without a configured global cap.
    #[test]
    fn zero_budget_disables_reservation_gate() {
        let engine = setup();
        let budget = librefang_types::config::BudgetConfig::default();
        // Even a huge estimate must pass when no limit is configured.
        let r = engine.reserve_global_budget(&budget, 9_999.0).unwrap();
        r.settle();
    }

    // ── #4807: per-provider budget breach flips exhaustion store ────────

    /// When a per-provider hourly cap trips, the engine must flag the
    /// provider in the attached exhaustion store. The fallback chain
    /// reads this on its next call and skips the slot without dispatch.
    #[test]
    fn provider_hourly_budget_flips_exhaustion_store() {
        let exhaustion = ProviderExhaustionStore::new();
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = Arc::new(UsageStore::new(substrate.pool()));
        let engine = MeteringEngine::new(store).with_exhaustion_store(exhaustion.clone());

        let agent_id = AgentId::new();
        let provider_budget = librefang_types::config::ProviderBudget {
            max_cost_per_hour_usd: 0.01,
            ..Default::default()
        };

        // Record cost above the cap.
        engine
            .record(&UsageRecord {
                agent_id,
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                input_tokens: 1_000,
                output_tokens: 500,
                cost_usd: 0.50,
                tool_calls: 0,
                latency_ms: 100,
                ..Default::default()
            })
            .unwrap();

        // Pre-condition: nothing marked yet.
        assert!(exhaustion.is_exhausted("openai").is_none());

        let result = engine.check_provider_budget("openai", &provider_budget);
        assert!(result.is_err(), "budget gate should refuse");

        // Post-condition: provider flagged with BudgetExceeded.
        let rec = exhaustion
            .is_exhausted("openai")
            .expect("provider should be flagged");
        assert_eq!(rec.reason, ExhaustionReason::BudgetExceeded);
        assert!(
            rec.until.is_some(),
            "budget-exceeded must carry an auto-clear time"
        );
    }

    /// Per-provider token cap trips the same path.
    #[test]
    fn provider_token_budget_flips_exhaustion_store() {
        let exhaustion = ProviderExhaustionStore::new();
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = Arc::new(UsageStore::new(substrate.pool()));
        let engine = MeteringEngine::new(store).with_exhaustion_store(exhaustion.clone());

        let agent_id = AgentId::new();
        let provider_budget = librefang_types::config::ProviderBudget {
            max_tokens_per_hour: 100,
            ..Default::default()
        };

        engine
            .record(&UsageRecord {
                agent_id,
                provider: "groq".to_string(),
                model: "llama-3-70b".to_string(),
                input_tokens: 1_000,
                output_tokens: 500,
                cost_usd: 0.0,
                tool_calls: 0,
                latency_ms: 100,
                ..Default::default()
            })
            .unwrap();

        assert!(engine
            .check_provider_budget("groq", &provider_budget)
            .is_err());
        let rec = exhaustion
            .is_exhausted("groq")
            .expect("groq should be flagged");
        assert_eq!(rec.reason, ExhaustionReason::BudgetExceeded);
    }

    /// Without an attached store the engine works exactly as before —
    /// the flag call is a no-op and existing call sites are unaffected.
    #[test]
    fn provider_budget_no_store_attached_is_legacy_compatible() {
        let engine = setup();
        let provider_budget = librefang_types::config::ProviderBudget {
            max_cost_per_hour_usd: 0.01,
            ..Default::default()
        };
        let agent_id = AgentId::new();
        engine
            .record(&UsageRecord {
                agent_id,
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                input_tokens: 1_000,
                output_tokens: 500,
                cost_usd: 0.50,
                tool_calls: 0,
                latency_ms: 100,
                ..Default::default()
            })
            .unwrap();

        // Still errors with QuotaExceeded — no panic, no store wiring needed.
        assert!(engine
            .check_provider_budget("openai", &provider_budget)
            .is_err());
    }

    /// `exhaustion_store()` accessor returns the same instance the engine
    /// was wired with, so the kernel can pass the same store down to the
    /// fallback-chain layer.
    #[test]
    fn exhaustion_store_accessor_round_trips_attached_handle() {
        let exhaustion = ProviderExhaustionStore::new();
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = Arc::new(UsageStore::new(substrate.pool()));
        let engine = MeteringEngine::new(store).with_exhaustion_store(exhaustion.clone());

        let from_engine = engine
            .exhaustion_store()
            .expect("engine should expose attached store");
        // Mark via the engine-returned handle, observe via the original —
        // both must point at the same underlying DashMap.
        from_engine.mark_exhausted(
            "openai",
            ExhaustionReason::BudgetExceeded,
            Some(Instant::now() + DEFAULT_LONG_BACKOFF),
        );
        assert!(exhaustion.is_exhausted("openai").is_some());
    }

    #[test]
    fn exhaustion_store_accessor_returns_none_when_unwired() {
        let engine = setup();
        assert!(engine.exhaustion_store().is_none());
    }
}
