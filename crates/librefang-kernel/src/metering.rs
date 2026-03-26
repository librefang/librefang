//! Metering engine — tracks LLM cost and enforces spending quotas.

use librefang_memory::usage::{ModelUsage, UsageRecord, UsageStore, UsageSummary};
use librefang_types::agent::{AgentId, ResourceQuota};
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::model_catalog::ModelCatalogEntry;
use std::sync::Arc;

const DEFAULT_INPUT_COST_PER_M: f64 = 1.0;
const DEFAULT_OUTPUT_COST_PER_M: f64 = 3.0;

/// The metering engine tracks usage cost and enforces quota limits.
pub struct MeteringEngine {
    /// Persistent usage store (SQLite-backed).
    store: Arc<UsageStore>,
}

impl MeteringEngine {
    /// Create a new metering engine with the given usage store.
    pub fn new(store: Arc<UsageStore>) -> Self {
        Self { store }
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
    pub fn check_global_budget(
        &self,
        budget: &librefang_types::config::BudgetConfig,
    ) -> LibreFangResult<()> {
        if budget.max_hourly_usd > 0.0 {
            let cost = self.store.query_global_hourly()?;
            if cost >= budget.max_hourly_usd {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global hourly budget exceeded: ${:.4} / ${:.4}",
                    cost, budget.max_hourly_usd
                )));
            }
        }

        if budget.max_daily_usd > 0.0 {
            let cost = self.store.query_today_cost()?;
            if cost >= budget.max_daily_usd {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global daily budget exceeded: ${:.4} / ${:.4}",
                    cost, budget.max_daily_usd
                )));
            }
        }

        if budget.max_monthly_usd > 0.0 {
            let cost = self.store.query_global_monthly()?;
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
    /// | Default (unknown)     |     1.00  |      3.00  |
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
        self.store.check_all_and_record(
            record,
            quota.max_cost_per_hour_usd,
            quota.max_cost_per_day_usd,
            quota.max_cost_per_month_usd,
            budget.max_hourly_usd,
            budget.max_daily_usd,
            budget.max_monthly_usd,
        )
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
        let store = Arc::new(UsageStore::new(substrate.usage_conn()));
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
                model: "claude-haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.001,
                tool_calls: 0,
                latency_ms: 150,
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
                model: "claude-sonnet".to_string(),
                input_tokens: 10000,
                output_tokens: 5000,
                cost_usd: 0.05,
                tool_calls: 0,
                latency_ms: 300,
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
                model: "claude-opus".to_string(),
                input_tokens: 100000,
                output_tokens: 50000,
                cost_usd: 100.0,
                tool_calls: 0,
                latency_ms: 500,
            })
            .unwrap();

        assert!(engine.check_quota(agent_id, &quota).is_ok());
    }

    #[test]
    fn test_estimate_cost_unknown() {
        let cost = MeteringEngine::estimate_cost("my-custom-model", 1_000_000, 1_000_000, 0, 0);
        assert!((cost - 4.0).abs() < 0.01); // $1.00 + $3.00
    }

    #[test]
    fn test_estimate_cost_with_catalog() {
        let catalog = test_catalog();
        // Sonnet: $3/M input, $15/M output
        let cost = MeteringEngine::estimate_cost_with_catalog(
            &catalog,
            "claude-sonnet-4-20250514",
            1_000_000,
            1_000_000,
            0,
            0,
        );
        assert!((cost - 18.0).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_with_catalog_alias() {
        let catalog = test_catalog();
        // "sonnet" alias should resolve to same pricing
        let cost = MeteringEngine::estimate_cost_with_catalog(
            &catalog, "sonnet", 1_000_000, 1_000_000, 0, 0,
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
                aliases: vec![],
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
        let cost = MeteringEngine::estimate_cost_with_catalog(
            &catalog, "llama3.2", 1_000_000, 1_000_000, 0, 0,
        );
        assert!(cost.abs() < f64::EPSILON);
    }

    #[test]
    fn test_estimate_cost_cache_read_discount() {
        // 1M total input tokens, 500k are cache-read (10% of input price)
        // Sonnet: $3/M input, $15/M output
        // Regular input: 500k * $3/M = $1.50
        // Cache read: 500k * $3/M * 0.10 = $0.15
        // Output: 1M * $15/M = $15.00
        // Total = $16.65
        let cost = MeteringEngine::estimate_cost(
            "claude-sonnet-4-20250514",
            1_000_000, // total input
            1_000_000, // output
            500_000,   // cache_read
            0,         // cache_creation
        );
        assert!((cost - 16.65).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_cache_creation_surcharge() {
        // 1M total input tokens, 200k are cache-creation (125% of input price)
        // Sonnet: $3/M input, $15/M output
        // Regular input: 800k * $3/M = $2.40
        // Cache creation: 200k * $3/M * 1.25 = $0.75
        // Output: 1M * $15/M = $15.00
        // Total = $18.15
        let cost = MeteringEngine::estimate_cost(
            "claude-sonnet-4-20250514",
            1_000_000, // total input
            1_000_000, // output
            0,         // cache_read
            200_000,   // cache_creation
        );
        assert!((cost - 18.15).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_cache_mixed() {
        // 1M total input, 400k cache-read, 100k cache-creation, 500k regular
        // Sonnet: $3/M input, $15/M output
        // Regular input: 500k * $3/M = $1.50
        // Cache read: 400k * $3/M * 0.10 = $0.12
        // Cache creation: 100k * $3/M * 1.25 = $0.375
        // Output: 1M * $15/M = $15.00
        // Total = $16.995
        let cost = MeteringEngine::estimate_cost(
            "claude-sonnet-4-20250514",
            1_000_000, // total input
            1_000_000, // output
            400_000,   // cache_read
            100_000,   // cache_creation
        );
        assert!((cost - 16.995).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_zero_cache_matches_no_cache() {
        // With zero cache tokens, should match the original behavior
        let cost_with_cache = MeteringEngine::estimate_cost("gpt-4o", 1_000_000, 1_000_000, 0, 0);
        let expected = 12.50; // $2.50 + $10.00
        assert!((cost_with_cache - expected).abs() < 0.01);
    }

    #[test]
    fn test_get_summary() {
        let engine = setup();
        let agent_id = AgentId::new();

        engine
            .record(&UsageRecord {
                agent_id,
                model: "haiku".to_string(),
                input_tokens: 500,
                output_tokens: 200,
                cost_usd: 0.005,
                tool_calls: 3,
                latency_ms: 100,
            })
            .unwrap();

        let summary = engine.get_summary(Some(agent_id)).unwrap();
        assert_eq!(summary.call_count, 1);
        assert_eq!(summary.total_input_tokens, 500);
    }
}
