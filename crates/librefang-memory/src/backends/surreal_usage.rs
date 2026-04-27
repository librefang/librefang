//! SurrealDB-backed implementation of [`UsageBackend`].
//!
//! Persists LLM usage events to the `usage_events` SurrealDB table
//! (defined in `008_usage_events.surql`).  Query methods use SurrealQL
//! aggregation functions; the more complex "check-and-record" methods are
//! implemented as non-atomic check-then-insert (SurrealDB's MVCC model
//! makes true serialisable transactions for this pattern unnecessary in
//! practice — the window is negligible and budgets are soft limits).
//!
//! All queries use parameterised bindings (`.bind()`) — no user-controlled
//! or caller-supplied strings are ever interpolated directly into SurrealQL.

use std::sync::Arc;

use chrono::Utc;
use surrealdb::{engine::any::Any, Surreal};
use uuid::Uuid;

use librefang_types::agent::{AgentId, UserId};
use librefang_types::error::{LibreFangError, LibreFangResult};

use librefang_storage::SurrealSession;

use crate::backend::UsageBackend;
use crate::usage::{ModelUsage, UsageRecord, UsageSummary};

/// Run a future on the current Tokio runtime or spin up a temporary one.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(f)),
        Err(_) => tokio::runtime::Runtime::new()
            .expect("tokio runtime")
            .block_on(f),
    }
}

// ── SurrealUsageStore ─────────────────────────────────────────────────────────

/// SurrealDB implementation of `UsageBackend`.
#[derive(Clone)]
pub struct SurrealUsageStore {
    db: Arc<Surreal<Any>>,
}

impl SurrealUsageStore {
    /// Open against an existing [`SurrealSession`].
    pub fn open(session: &SurrealSession) -> Self {
        Self {
            db: Arc::new(session.client().clone()),
        }
    }

    /// Wrap an existing connected SurrealDB instance.
    pub fn new(db: Arc<Surreal<Any>>) -> Self {
        Self { db }
    }

    async fn insert_record_async(&self, record: &UsageRecord) -> LibreFangResult<()> {
        let row = serde_json::json!({
            "agent_id": record.agent_id.0.to_string(),
            "timestamp": Utc::now().to_rfc3339(),
            "provider": record.provider,
            "model": record.model,
            "input_tokens": record.input_tokens as i64,
            "output_tokens": record.output_tokens as i64,
            "cost_usd": record.cost_usd,
            "tool_calls": record.tool_calls as i64,
            "latency_ms": record.latency_ms as i64,
            "user_id": record.user_id.as_ref().map(|u| u.to_string()),
            "channel": record.channel.clone(),
        });
        self.db
            .create::<Option<serde_json::Value>>(("usage_events", Uuid::new_v4().to_string()))
            .content(row)
            .await
            .map_err(|e| LibreFangError::Memory(format!("SurrealDB insert usage: {e}")))?;
        Ok(())
    }

    /// Execute a single-row aggregate query that returns `{ total: f64 }`.
    async fn query_f64_total(
        &self,
        query: &str,
        bindings: Vec<(&'static str, String)>,
    ) -> LibreFangResult<f64> {
        let mut q = self.db.query(query);
        for (key, val) in bindings {
            q = q.bind((key, val));
        }
        let mut res = q
            .await
            .map_err(|e| LibreFangError::Memory(format!("SurrealDB usage query: {e}")))?;
        let rows: Vec<serde_json::Value> = res
            .take(0)
            .map_err(|e| LibreFangError::Memory(format!("SurrealDB usage result: {e}")))?;
        Ok(rows
            .into_iter()
            .next()
            .and_then(|row| row.get("total")?.as_f64())
            .unwrap_or(0.0))
    }

    /// Execute a single-row aggregate query that returns `{ total: u64 }`.
    async fn query_u64_total(
        &self,
        query: &str,
        bindings: Vec<(&'static str, String)>,
    ) -> LibreFangResult<u64> {
        let mut q = self.db.query(query);
        for (key, val) in bindings {
            q = q.bind((key, val));
        }
        let mut res = q
            .await
            .map_err(|e| LibreFangError::Memory(format!("SurrealDB usage query: {e}")))?;
        let rows: Vec<serde_json::Value> = res
            .take(0)
            .map_err(|e| LibreFangError::Memory(format!("SurrealDB usage result: {e}")))?;
        Ok(rows
            .into_iter()
            .next()
            .and_then(|row| row.get("total")?.as_i64())
            .unwrap_or(0) as u64)
    }
}

// ── Helpers for time window fragments ─────────────────────────────────────────

/// ISO-8601 timestamp for `now - 1 hour`.
fn one_hour_ago() -> String {
    (Utc::now() - chrono::Duration::hours(1)).to_rfc3339()
}

/// ISO-8601 timestamp for start of current UTC day.
fn today_start() -> String {
    let now = Utc::now();
    format!(
        "{}-{:02}-{:02}T00:00:00Z",
        now.year(),
        now.month(),
        now.day()
    )
}

/// ISO-8601 timestamp for start of current UTC month.
fn month_start() -> String {
    let now = Utc::now();
    format!("{}-{:02}-01T00:00:00Z", now.year(), now.month())
}

use chrono::Datelike;

// ── UsageBackend impl ─────────────────────────────────────────────────────────

impl UsageBackend for SurrealUsageStore {
    fn record(&self, record: &UsageRecord) -> LibreFangResult<()> {
        block_on(self.insert_record_async(record))
    }

    fn query_hourly(&self, agent_id: AgentId) -> LibreFangResult<f64> {
        block_on(self.query_f64_total(
            "SELECT math::sum(cost_usd) AS total FROM usage_events \
             WHERE agent_id = $agent_id AND timestamp > $since",
            vec![
                ("agent_id", agent_id.0.to_string()),
                ("since", one_hour_ago()),
            ],
        ))
    }

    fn query_daily(&self, agent_id: AgentId) -> LibreFangResult<f64> {
        block_on(self.query_f64_total(
            "SELECT math::sum(cost_usd) AS total FROM usage_events \
             WHERE agent_id = $agent_id AND timestamp > $since",
            vec![
                ("agent_id", agent_id.0.to_string()),
                ("since", today_start()),
            ],
        ))
    }

    fn query_monthly(&self, agent_id: AgentId) -> LibreFangResult<f64> {
        block_on(self.query_f64_total(
            "SELECT math::sum(cost_usd) AS total FROM usage_events \
             WHERE agent_id = $agent_id AND timestamp > $since",
            vec![
                ("agent_id", agent_id.0.to_string()),
                ("since", month_start()),
            ],
        ))
    }

    fn query_global_hourly(&self) -> LibreFangResult<f64> {
        block_on(self.query_f64_total(
            "SELECT math::sum(cost_usd) AS total FROM usage_events WHERE timestamp > $since",
            vec![("since", one_hour_ago())],
        ))
    }

    fn query_today_cost(&self) -> LibreFangResult<f64> {
        block_on(self.query_f64_total(
            "SELECT math::sum(cost_usd) AS total FROM usage_events WHERE timestamp > $since",
            vec![("since", today_start())],
        ))
    }

    fn query_global_monthly(&self) -> LibreFangResult<f64> {
        block_on(self.query_f64_total(
            "SELECT math::sum(cost_usd) AS total FROM usage_events WHERE timestamp > $since",
            vec![("since", month_start())],
        ))
    }

    fn query_summary(&self, agent_id: Option<AgentId>) -> LibreFangResult<UsageSummary> {
        let summary = block_on(async {
            let mut res = if let Some(aid) = agent_id {
                self.db
                    .query(
                        "SELECT math::sum(input_tokens) AS total_input_tokens, \
                                math::sum(output_tokens) AS total_output_tokens, \
                                math::sum(cost_usd) AS total_cost_usd, \
                                count() AS call_count, \
                                math::sum(tool_calls) AS total_tool_calls \
                         FROM usage_events WHERE agent_id = $agent_id",
                    )
                    .bind(("agent_id", aid.0.to_string()))
                    .await
                    .map_err(|e| LibreFangError::Memory(format!("SurrealDB usage summary: {e}")))?
            } else {
                self.db
                    .query(
                        "SELECT math::sum(input_tokens) AS total_input_tokens, \
                                math::sum(output_tokens) AS total_output_tokens, \
                                math::sum(cost_usd) AS total_cost_usd, \
                                count() AS call_count, \
                                math::sum(tool_calls) AS total_tool_calls \
                         FROM usage_events",
                    )
                    .await
                    .map_err(|e| LibreFangError::Memory(format!("SurrealDB usage summary: {e}")))?
            };
            let rows: Vec<serde_json::Value> = res
                .take(0)
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB usage summary: {e}")))?;
            Ok::<_, LibreFangError>(rows)
        })?;

        let row = summary.into_iter().next().unwrap_or_default();
        Ok(UsageSummary {
            total_input_tokens: row
                .get("total_input_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as u64,
            total_output_tokens: row
                .get("total_output_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as u64,
            total_cost_usd: row
                .get("total_cost_usd")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            call_count: row.get("call_count").and_then(|v| v.as_i64()).unwrap_or(0) as u64,
            total_tool_calls: row
                .get("total_tool_calls")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as u64,
        })
    }

    fn query_by_model(&self) -> LibreFangResult<Vec<ModelUsage>> {
        let rows = block_on(async {
            let mut res = self
                .db
                .query(
                    "SELECT model, \
                            math::sum(cost_usd) AS total_cost_usd, \
                            math::sum(input_tokens) AS total_input_tokens, \
                            math::sum(output_tokens) AS total_output_tokens, \
                            count() AS call_count \
                     FROM usage_events GROUP BY model",
                )
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB model usage: {e}")))?;
            let rows: Vec<serde_json::Value> = res
                .take(0)
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB model usage: {e}")))?;
            Ok::<_, LibreFangError>(rows)
        })?;

        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let model = r.get("model")?.as_str()?.to_string();
                Some(ModelUsage {
                    model,
                    total_cost_usd: r
                        .get("total_cost_usd")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                    total_input_tokens: r
                        .get("total_input_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0) as u64,
                    total_output_tokens: r
                        .get("total_output_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0) as u64,
                    call_count: r.get("call_count").and_then(|v| v.as_i64()).unwrap_or(0) as u64,
                })
            })
            .collect())
    }

    fn check_quota_and_record(
        &self,
        record: &UsageRecord,
        max_hourly: f64,
        max_daily: f64,
        max_monthly: f64,
    ) -> LibreFangResult<()> {
        if max_hourly > 0.0 {
            let current = self.query_hourly(record.agent_id)?;
            if current + record.cost_usd >= max_hourly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded hourly cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, current, record.cost_usd, max_hourly
                )));
            }
        }
        if max_daily > 0.0 {
            let current = self.query_daily(record.agent_id)?;
            if current + record.cost_usd >= max_daily {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded daily cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, current, record.cost_usd, max_daily
                )));
            }
        }
        if max_monthly > 0.0 {
            let current = self.query_monthly(record.agent_id)?;
            if current + record.cost_usd >= max_monthly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded monthly cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, current, record.cost_usd, max_monthly
                )));
            }
        }
        self.record(record)
    }

    fn check_global_budget_and_record(
        &self,
        record: &UsageRecord,
        max_hourly: f64,
        max_daily: f64,
        max_monthly: f64,
    ) -> LibreFangResult<()> {
        if max_hourly > 0.0 {
            let current = self.query_global_hourly()?;
            if current + record.cost_usd >= max_hourly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global hourly budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    current, record.cost_usd, max_hourly
                )));
            }
        }
        if max_daily > 0.0 {
            let current = self.query_today_cost()?;
            if current + record.cost_usd >= max_daily {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global daily budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    current, record.cost_usd, max_daily
                )));
            }
        }
        if max_monthly > 0.0 {
            let current = self.query_global_monthly()?;
            if current + record.cost_usd >= max_monthly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global monthly budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    current, record.cost_usd, max_monthly
                )));
            }
        }
        self.record(record)
    }

    #[allow(clippy::too_many_arguments)]
    fn check_all_with_provider_and_record(
        &self,
        record: &UsageRecord,
        agent_max_hourly: f64,
        agent_max_daily: f64,
        agent_max_monthly: f64,
        global_max_hourly: f64,
        global_max_daily: f64,
        global_max_monthly: f64,
        provider_max_hourly: f64,
        provider_max_daily: f64,
        provider_max_monthly: f64,
        provider_max_tokens_per_hour: u64,
    ) -> LibreFangResult<()> {
        // Per-agent quota checks
        self.check_quota_and_record(record, agent_max_hourly, agent_max_daily, agent_max_monthly)?;

        // Global budget checks
        if global_max_hourly > 0.0 {
            let current = self.query_global_hourly()?;
            if current + record.cost_usd >= global_max_hourly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global hourly budget exceeded: ${:.4} / ${:.4}",
                    current, global_max_hourly
                )));
            }
        }
        if global_max_daily > 0.0 {
            let current = self.query_today_cost()?;
            if current + record.cost_usd >= global_max_daily {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global daily budget exceeded: ${:.4} / ${:.4}",
                    current, global_max_daily
                )));
            }
        }
        if global_max_monthly > 0.0 {
            let current = self.query_global_monthly()?;
            if current + record.cost_usd >= global_max_monthly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global monthly budget exceeded: ${:.4} / ${:.4}",
                    current, global_max_monthly
                )));
            }
        }

        // Per-provider budget checks
        if !record.provider.is_empty() {
            if provider_max_hourly > 0.0 {
                let current = self.query_provider_hourly(&record.provider)?;
                if current + record.cost_usd >= provider_max_hourly {
                    return Err(LibreFangError::QuotaExceeded(format!(
                        "Provider '{}' exceeded hourly cost: ${:.4} / ${:.4}",
                        record.provider, current, provider_max_hourly
                    )));
                }
            }
            if provider_max_daily > 0.0 {
                let current = self.query_provider_daily(&record.provider)?;
                if current + record.cost_usd >= provider_max_daily {
                    return Err(LibreFangError::QuotaExceeded(format!(
                        "Provider '{}' exceeded daily cost: ${:.4} / ${:.4}",
                        record.provider, current, provider_max_daily
                    )));
                }
            }
            if provider_max_monthly > 0.0 {
                let current = self.query_provider_monthly(&record.provider)?;
                if current + record.cost_usd >= provider_max_monthly {
                    return Err(LibreFangError::QuotaExceeded(format!(
                        "Provider '{}' exceeded monthly cost: ${:.4} / ${:.4}",
                        record.provider, current, provider_max_monthly
                    )));
                }
            }
            if provider_max_tokens_per_hour > 0 {
                let current = self.query_provider_tokens_hourly(&record.provider)?;
                let new_tokens = record.input_tokens + record.output_tokens;
                if current + new_tokens >= provider_max_tokens_per_hour {
                    return Err(LibreFangError::QuotaExceeded(format!(
                        "Provider '{}' exceeded hourly token budget: {} / {}",
                        record.provider, current, provider_max_tokens_per_hour
                    )));
                }
            }
        }

        self.record(record)
    }

    fn query_provider_hourly(&self, provider: &str) -> LibreFangResult<f64> {
        block_on(self.query_f64_total(
            "SELECT math::sum(cost_usd) AS total FROM usage_events \
             WHERE provider = $provider AND timestamp > $since",
            vec![
                ("provider", provider.to_string()),
                ("since", one_hour_ago()),
            ],
        ))
    }

    fn query_provider_daily(&self, provider: &str) -> LibreFangResult<f64> {
        block_on(self.query_f64_total(
            "SELECT math::sum(cost_usd) AS total FROM usage_events \
             WHERE provider = $provider AND timestamp > $since",
            vec![("provider", provider.to_string()), ("since", today_start())],
        ))
    }

    fn query_provider_monthly(&self, provider: &str) -> LibreFangResult<f64> {
        block_on(self.query_f64_total(
            "SELECT math::sum(cost_usd) AS total FROM usage_events \
             WHERE provider = $provider AND timestamp > $since",
            vec![("provider", provider.to_string()), ("since", month_start())],
        ))
    }

    fn query_provider_tokens_hourly(&self, provider: &str) -> LibreFangResult<u64> {
        block_on(self.query_u64_total(
            "SELECT math::sum(input_tokens) AS total FROM usage_events \
             WHERE provider = $provider AND timestamp > $since",
            vec![
                ("provider", provider.to_string()),
                ("since", one_hour_ago()),
            ],
        ))
    }

    fn cleanup_old(&self, days: u32) -> LibreFangResult<usize> {
        block_on(async {
            let cutoff = (Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();
            let mut res = self
                .db
                .query("DELETE FROM usage_events WHERE timestamp < $cutoff RETURN BEFORE")
                .bind(("cutoff", cutoff))
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB cleanup_old: {e}")))?;
            let deleted: Vec<serde_json::Value> = res
                .take(0)
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB cleanup_old: {e}")))?;
            Ok::<_, LibreFangError>(deleted.len())
        })
    }

    fn query_user_hourly(&self, user_id: UserId) -> LibreFangResult<f64> {
        block_on(self.query_f64_total(
            "SELECT math::sum(cost_usd) AS total FROM usage_events \
             WHERE user_id = $user_id AND timestamp > $since",
            vec![("user_id", user_id.to_string()), ("since", one_hour_ago())],
        ))
    }

    fn query_user_daily(&self, user_id: UserId) -> LibreFangResult<f64> {
        block_on(self.query_f64_total(
            "SELECT math::sum(cost_usd) AS total FROM usage_events \
             WHERE user_id = $user_id AND timestamp > $since",
            vec![("user_id", user_id.to_string()), ("since", today_start())],
        ))
    }

    fn query_user_monthly(&self, user_id: UserId) -> LibreFangResult<f64> {
        block_on(self.query_f64_total(
            "SELECT math::sum(cost_usd) AS total FROM usage_events \
             WHERE user_id = $user_id AND timestamp > $since",
            vec![("user_id", user_id.to_string()), ("since", month_start())],
        ))
    }
}
