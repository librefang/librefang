//! Usage tracking store — records LLM usage events for cost monitoring.

use chrono::Utc;
use librefang_types::agent::{AgentId, SessionId, UserId};
use librefang_types::error::{LibreFangError, LibreFangResult};
use rusqlite::{Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// A single usage event recording an LLM call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageRecord {
    /// Which agent made the call.
    pub agent_id: AgentId,
    /// Provider id (e.g. "openai", "moonshot", "litellm", "ollama"). Empty
    /// string means the caller did not track a provider — in that case the
    /// per-provider budget check is skipped.
    #[serde(default)]
    pub provider: String,
    /// Model used.
    pub model: String,
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens consumed.
    pub output_tokens: u64,
    /// Estimated cost in USD.
    pub cost_usd: f64,
    /// Number of tool calls in this interaction.
    pub tool_calls: u32,
    /// Latency in milliseconds.
    pub latency_ms: u64,
    /// RBAC M5: LibreFang user that triggered the call (resolved from the
    /// API caller, channel binding, or sender context). `None` for
    /// kernel-internal events (cron / boot tasks) and pre-M5 records that
    /// pre-date this column.
    #[serde(default)]
    pub user_id: Option<UserId>,
    /// RBAC M5: Channel the call originated from (e.g. "telegram",
    /// "discord", "api", "cron", "cli"). `None` for unattributed calls.
    #[serde(default)]
    pub channel: Option<String>,
    /// Session this LLM call belonged to, when available. `None` for
    /// session-less paths (ephemeral side-questions, background review)
    /// and for pre-v30 records that pre-date this column.
    #[serde(default)]
    pub session_id: Option<SessionId>,
}

impl UsageRecord {
    /// Convenience constructor for tests and call sites that do not yet
    /// attribute usage to a user / channel. Keeps the new optional fields
    /// out of every existing struct literal in the kernel.
    ///
    /// Eight positional args is over the clippy default of seven, but the
    /// shape mirrors the metering record schema 1:1 and grouping into a
    /// builder would push call-site noise into ~20 internal kernel paths
    /// that touch this constructor without gaining type safety. Suppression
    /// is local to this fn.
    #[allow(clippy::too_many_arguments)]
    pub fn anonymous(
        agent_id: AgentId,
        provider: impl Into<String>,
        model: impl Into<String>,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        tool_calls: u32,
        latency_ms: u64,
    ) -> Self {
        Self {
            agent_id,
            provider: provider.into(),
            model: model.into(),
            input_tokens,
            output_tokens,
            cost_usd,
            tool_calls,
            latency_ms,
            user_id: None,
            channel: None,
            session_id: None,
        }
    }
}

/// Summary of usage over a period.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageSummary {
    /// Total input tokens.
    pub total_input_tokens: u64,
    /// Total output tokens.
    pub total_output_tokens: u64,
    /// Total estimated cost in USD.
    pub total_cost_usd: f64,
    /// Total number of calls.
    pub call_count: u64,
    /// Total tool calls.
    pub total_tool_calls: u64,
}

/// One row of a per-agent recent-events feed. Mirrors the columns on
/// `usage_events` that operators care about when looking at a single
/// agent's tail (model / latency / tokens / cost).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEventRow {
    pub timestamp: String,
    pub model: String,
    pub provider: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub tool_calls: u64,
    pub latency_ms: u64,
}

/// Usage grouped by model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUsage {
    /// Model name.
    pub model: String,
    /// Total cost for this model.
    pub total_cost_usd: f64,
    /// Total input tokens.
    pub total_input_tokens: u64,
    /// Total output tokens.
    pub total_output_tokens: u64,
    /// Number of calls.
    pub call_count: u64,
}

/// Model performance metrics including latency statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPerformance {
    /// Model name.
    pub model: String,
    /// Total cost for this model.
    pub total_cost_usd: f64,
    /// Total input tokens.
    pub total_input_tokens: u64,
    /// Total output tokens.
    pub total_output_tokens: u64,
    /// Number of calls.
    pub call_count: u64,
    /// Average latency in milliseconds.
    pub avg_latency_ms: f64,
    /// Minimum latency in milliseconds.
    pub min_latency_ms: u64,
    /// Maximum latency in milliseconds.
    pub max_latency_ms: u64,
    /// Cost per call in USD.
    pub cost_per_call: f64,
    /// Average latency per call in milliseconds.
    pub avg_latency_per_call: f64,
}

/// Per-user spend ranking row (RBAC M5).
///
/// `user_id` is the stringified [`librefang_types::agent::UserId`] —
/// callers re-parse it via `FromStr` if they need the typed form. Three
/// time windows are precomputed by the SQL so the dashboard doesn't have
/// to issue four queries per row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSpendRanking {
    pub user_id: String,
    pub hourly_cost_usd: f64,
    pub daily_cost_usd: f64,
    pub monthly_cost_usd: f64,
    pub call_count: u64,
}

/// Daily usage breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyBreakdown {
    /// Date string (YYYY-MM-DD).
    pub date: String,
    /// Total cost for this day.
    pub cost_usd: f64,
    /// Total tokens (input + output).
    pub tokens: u64,
    /// Number of API calls.
    pub calls: u64,
}

/// Usage store backed by SQLite.
#[derive(Clone)]
pub struct UsageStore {
    conn: Arc<Mutex<Connection>>,
}

impl UsageStore {
    /// Create a new usage store wrapping the given connection.
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Record a usage event.
    pub fn record(&self, record: &UsageRecord) -> LibreFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        Self::insert_record(&conn, record)
    }

    /// Insert a usage record into the database (helper used by both `record`
    /// and the atomic `check_quota_and_record`).
    fn insert_record(conn: &Connection, record: &UsageRecord) -> LibreFangResult<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        // RBAC M5 + session attribution: persist user_id/channel/session_id
        // alongside the existing columns. Schema v23 added user_id/channel,
        // v30 added session_id — all are NULL-able so missing attribution
        // round-trips as NULL.
        conn.execute(
            "INSERT INTO usage_events (id, agent_id, timestamp, model, provider, input_tokens, output_tokens, cost_usd, tool_calls, latency_ms, user_id, channel, session_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                id,
                record.agent_id.0.to_string(),
                now,
                record.model,
                record.provider,
                record.input_tokens as i64,
                record.output_tokens as i64,
                record.cost_usd,
                record.tool_calls as i64,
                record.latency_ms as i64,
                record.user_id.map(|u| u.to_string()),
                record.channel.as_deref(),
                record.session_id.map(|s| s.0.to_string()),
            ],
        )
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Atomically check per-agent quotas and record usage in a single SQLite
    /// transaction.  This prevents the TOCTOU race where two concurrent
    /// requests both pass the quota check before either records its usage.
    ///
    /// Returns `Ok(())` if the record was inserted within quota, or
    /// `QuotaExceeded` if inserting would breach any of the supplied limits
    /// (in which case nothing is written).
    pub fn check_quota_and_record(
        &self,
        record: &UsageRecord,
        max_hourly: f64,
        max_daily: f64,
        max_monthly: f64,
    ) -> LibreFangResult<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        // IMMEDIATE transaction acquires a reserved lock up-front, ensuring no
        // other writer can interleave between our SELECT and INSERT.  The RAII
        // guard auto-rolls back on drop if we return early (error or quota
        // exceeded), so every error path is safe.
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let agent_str = record.agent_id.0.to_string();

        // Check hourly quota
        if max_hourly > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE agent_id = ?1 AND timestamp > datetime('now', '-1 hour')",
                    rusqlite::params![&agent_str],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            if cost + record.cost_usd >= max_hourly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded hourly cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, cost, record.cost_usd, max_hourly
                )));
            }
        }

        // Check daily quota
        if max_daily > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE agent_id = ?1 AND timestamp > datetime('now', 'start of day')",
                    rusqlite::params![&agent_str],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            if cost + record.cost_usd >= max_daily {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded daily cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, cost, record.cost_usd, max_daily
                )));
            }
        }

        // Check monthly quota
        if max_monthly > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE agent_id = ?1 AND timestamp > datetime('now', 'start of month')",
                    rusqlite::params![&agent_str],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            if cost + record.cost_usd >= max_monthly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded monthly cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, cost, record.cost_usd, max_monthly
                )));
            }
        }

        // All checks passed — insert the record within the same transaction
        Self::insert_record(&tx, record)?;

        tx.commit()
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Atomically check global budget limits and record usage in a single
    /// SQLite transaction.  Similar to `check_quota_and_record` but checks
    /// aggregate spend across *all* agents.
    pub fn check_global_budget_and_record(
        &self,
        record: &UsageRecord,
        max_hourly: f64,
        max_daily: f64,
        max_monthly: f64,
    ) -> LibreFangResult<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        // Check global hourly budget
        if max_hourly > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE timestamp > datetime('now', '-1 hour')",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            if cost + record.cost_usd >= max_hourly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global hourly budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    cost, record.cost_usd, max_hourly
                )));
            }
        }

        // Check global daily budget
        if max_daily > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE timestamp > datetime('now', 'start of day')",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            if cost + record.cost_usd >= max_daily {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global daily budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    cost, record.cost_usd, max_daily
                )));
            }
        }

        // Check global monthly budget
        if max_monthly > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE timestamp > datetime('now', 'start of month')",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            if cost + record.cost_usd >= max_monthly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global monthly budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    cost, record.cost_usd, max_monthly
                )));
            }
        }

        // All checks passed — insert the record
        Self::insert_record(&tx, record)?;

        tx.commit()
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Atomically check both per-agent quotas and global budget limits, then
    /// record the usage event — all within a single SQLite transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn check_all_and_record(
        &self,
        record: &UsageRecord,
        agent_max_hourly: f64,
        agent_max_daily: f64,
        agent_max_monthly: f64,
        global_max_hourly: f64,
        global_max_daily: f64,
        global_max_monthly: f64,
    ) -> LibreFangResult<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let agent_str = record.agent_id.0.to_string();

        // ── Per-agent quota checks ──────────────────────────────────
        if agent_max_hourly > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE agent_id = ?1 AND timestamp > datetime('now', '-1 hour')",
                    rusqlite::params![&agent_str],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            if cost + record.cost_usd >= agent_max_hourly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded hourly cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, cost, record.cost_usd, agent_max_hourly
                )));
            }
        }

        if agent_max_daily > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE agent_id = ?1 AND timestamp > datetime('now', 'start of day')",
                    rusqlite::params![&agent_str],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            if cost + record.cost_usd >= agent_max_daily {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded daily cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, cost, record.cost_usd, agent_max_daily
                )));
            }
        }

        if agent_max_monthly > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE agent_id = ?1 AND timestamp > datetime('now', 'start of month')",
                    rusqlite::params![&agent_str],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            if cost + record.cost_usd >= agent_max_monthly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded monthly cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, cost, record.cost_usd, agent_max_monthly
                )));
            }
        }

        // ── Global budget checks ────────────────────────────────────
        if global_max_hourly > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE timestamp > datetime('now', '-1 hour')",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            if cost + record.cost_usd >= global_max_hourly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global hourly budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    cost, record.cost_usd, global_max_hourly
                )));
            }
        }

        if global_max_daily > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE timestamp > datetime('now', 'start of day')",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            if cost + record.cost_usd >= global_max_daily {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global daily budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    cost, record.cost_usd, global_max_daily
                )));
            }
        }

        if global_max_monthly > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE timestamp > datetime('now', 'start of month')",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            if cost + record.cost_usd >= global_max_monthly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global monthly budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    cost, record.cost_usd, global_max_monthly
                )));
            }
        }

        // All checks passed — insert the record
        Self::insert_record(&tx, record)?;

        tx.commit()
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Atomically check per-agent quotas, global budget, AND the per-provider
    /// budget for the record's provider, then record the event — all within a
    /// single SQLite transaction.
    ///
    /// `provider_*` limits apply only if `record.provider` is non-empty and
    /// the corresponding limit is > 0. Pass zero for "unlimited".
    #[allow(clippy::too_many_arguments)]
    pub fn check_all_with_provider_and_record(
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
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let agent_str = record.agent_id.0.to_string();
        let has_provider = !record.provider.is_empty();

        // Each window collapses what was previously up to 3 separate `SUM(...)`
        // queries (agent / global / provider) into one row of conditional
        // sums (#3382). The full hot path drops from up to 10 round-trips per
        // LLM call to 4 (3 cost windows + 1 token window) when every limit is
        // configured, while preserving identical semantics.
        struct WindowCosts {
            agent: f64,
            global: f64,
            provider: f64,
        }

        // Helper closure: run one combined SUM query for a given time window.
        // `where_clause` selects the rows for the window (e.g. `timestamp > datetime(...)`).
        let window_costs = |where_clause: &str| -> LibreFangResult<WindowCosts> {
            let sql = format!(
                "SELECT \
                    COALESCE(SUM(CASE WHEN agent_id = ?1 THEN cost_usd ELSE 0 END), 0.0), \
                    COALESCE(SUM(cost_usd), 0.0), \
                    COALESCE(SUM(CASE WHEN provider = ?2 THEN cost_usd ELSE 0 END), 0.0) \
                 FROM usage_events WHERE {where_clause}"
            );
            let row: (f64, f64, f64) = tx
                .query_row(
                    &sql,
                    rusqlite::params![&agent_str, &record.provider],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            Ok(WindowCosts {
                agent: row.0,
                global: row.1,
                provider: row.2,
            })
        };

        let need_hourly = agent_max_hourly > 0.0
            || global_max_hourly > 0.0
            || (has_provider && provider_max_hourly > 0.0);
        if need_hourly {
            let costs = window_costs("timestamp > datetime('now', '-1 hour')")?;
            if agent_max_hourly > 0.0 && costs.agent + record.cost_usd >= agent_max_hourly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded hourly cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, costs.agent, record.cost_usd, agent_max_hourly
                )));
            }
            if global_max_hourly > 0.0 && costs.global + record.cost_usd >= global_max_hourly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global hourly budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    costs.global, record.cost_usd, global_max_hourly
                )));
            }
            if has_provider
                && provider_max_hourly > 0.0
                && costs.provider + record.cost_usd >= provider_max_hourly
            {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Provider '{}' exceeded hourly cost budget: ${:.4} + ${:.4} / ${:.4}",
                    record.provider, costs.provider, record.cost_usd, provider_max_hourly
                )));
            }
        }

        let need_daily = agent_max_daily > 0.0
            || global_max_daily > 0.0
            || (has_provider && provider_max_daily > 0.0);
        if need_daily {
            let costs = window_costs("timestamp > datetime('now', 'start of day')")?;
            if agent_max_daily > 0.0 && costs.agent + record.cost_usd >= agent_max_daily {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded daily cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, costs.agent, record.cost_usd, agent_max_daily
                )));
            }
            if global_max_daily > 0.0 && costs.global + record.cost_usd >= global_max_daily {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global daily budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    costs.global, record.cost_usd, global_max_daily
                )));
            }
            if has_provider
                && provider_max_daily > 0.0
                && costs.provider + record.cost_usd >= provider_max_daily
            {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Provider '{}' exceeded daily cost budget: ${:.4} + ${:.4} / ${:.4}",
                    record.provider, costs.provider, record.cost_usd, provider_max_daily
                )));
            }
        }

        let need_monthly = agent_max_monthly > 0.0
            || global_max_monthly > 0.0
            || (has_provider && provider_max_monthly > 0.0);
        if need_monthly {
            let costs = window_costs("timestamp > datetime('now', 'start of month')")?;
            if agent_max_monthly > 0.0 && costs.agent + record.cost_usd >= agent_max_monthly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded monthly cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, costs.agent, record.cost_usd, agent_max_monthly
                )));
            }
            if global_max_monthly > 0.0 && costs.global + record.cost_usd >= global_max_monthly {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global monthly budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    costs.global, record.cost_usd, global_max_monthly
                )));
            }
            if has_provider
                && provider_max_monthly > 0.0
                && costs.provider + record.cost_usd >= provider_max_monthly
            {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Provider '{}' exceeded monthly cost budget: ${:.4} + ${:.4} / ${:.4}",
                    record.provider, costs.provider, record.cost_usd, provider_max_monthly
                )));
            }
        }

        // Provider hourly token budget — separate aggregate (input+output tokens),
        // kept as its own query because it sums different columns.
        if has_provider && provider_max_tokens_per_hour > 0 {
            let tokens: i64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(input_tokens) + SUM(output_tokens), 0) FROM usage_events
                     WHERE provider = ?1 AND timestamp > datetime('now', '-1 hour')",
                    rusqlite::params![&record.provider],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            let current = tokens.max(0) as u64;
            let incoming = record.input_tokens.saturating_add(record.output_tokens);
            if current.saturating_add(incoming) >= provider_max_tokens_per_hour {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Provider '{}' exceeded hourly token budget: {} + {} / {}",
                    record.provider, current, incoming, provider_max_tokens_per_hour
                )));
            }
        }

        // All checks passed — insert the record
        Self::insert_record(&tx, record)?;

        tx.commit()
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Query total cost in the last hour for an agent.
    pub fn query_hourly(&self, agent_id: AgentId) -> LibreFangResult<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE agent_id = ?1 AND timestamp > datetime('now', '-1 hour')",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Query total cost today for an agent.
    pub fn query_daily(&self, agent_id: AgentId) -> LibreFangResult<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE agent_id = ?1 AND timestamp > datetime('now', 'start of day')",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Query total cost in the current calendar month for an agent.
    pub fn query_monthly(&self, agent_id: AgentId) -> LibreFangResult<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE agent_id = ?1 AND timestamp > datetime('now', 'start of month')",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Query total cost for a specific provider in the last hour.
    pub fn query_provider_hourly(&self, provider: &str) -> LibreFangResult<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE provider = ?1 AND timestamp > datetime('now', '-1 hour')",
                rusqlite::params![provider],
                |row| row.get(0),
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Query total cost for a specific provider today.
    pub fn query_provider_daily(&self, provider: &str) -> LibreFangResult<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE provider = ?1 AND timestamp > datetime('now', 'start of day')",
                rusqlite::params![provider],
                |row| row.get(0),
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Query total cost for a specific provider in the current calendar month.
    pub fn query_provider_monthly(&self, provider: &str) -> LibreFangResult<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE provider = ?1 AND timestamp > datetime('now', 'start of month')",
                rusqlite::params![provider],
                |row| row.get(0),
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Query total tokens (input + output) for a specific provider in the last hour.
    pub fn query_provider_tokens_hourly(&self, provider: &str) -> LibreFangResult<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let tokens: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(input_tokens) + SUM(output_tokens), 0) FROM usage_events
                 WHERE provider = ?1 AND timestamp > datetime('now', '-1 hour')",
                rusqlite::params![provider],
                |row| row.get(0),
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(tokens.max(0) as u64)
    }

    // ── Per-user spend rollup (RBAC M5) ─────────────────────────────────
    //
    // Pre-M5 rows have `user_id IS NULL` and never match these queries —
    // that is the right default since they pre-date attribution and would
    // otherwise be assigned to whichever user the operator looks at first.
    // The `idx_usage_user_time` index added in v23 keeps these aggregates
    // O(log n + k) regardless of total table size.
    //
    // **Time zone:** SQLite's `datetime('now', 'start of day')` returns
    // the UTC day boundary, NOT the server-local boundary. Operators in
    // non-UTC zones see "today's spend" sliced on the UTC midnight
    // (e.g. an Asia/Shanghai admin watching at 06:00 local sees the
    // window that started at 14:00 the previous evening). This matches
    // the existing global / per-agent rollups (`query_global_*`,
    // `query_agent_*`) and the server-side `usage_events.timestamp` —
    // making spend totals comparable across all the rollups in this
    // module. If a future operator wants local-day buckets, swap the
    // SQL to `datetime('now', 'localtime', 'start of day', 'utc')` in
    // every roll-up and update the `BudgetConfig` doc to match.

    /// Total cost in the last hour (UTC sliding window) for a single user.
    pub fn query_user_hourly(&self, user_id: UserId) -> LibreFangResult<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE user_id = ?1 AND timestamp > datetime('now', '-1 hour')",
                rusqlite::params![user_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Total cost today (UTC calendar day, see module-level note) for a single user.
    pub fn query_user_daily(&self, user_id: UserId) -> LibreFangResult<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE user_id = ?1 AND timestamp > datetime('now', 'start of day')",
                rusqlite::params![user_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Total cost in the current UTC calendar month (see module-level note) for a single user.
    pub fn query_user_monthly(&self, user_id: UserId) -> LibreFangResult<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE user_id = ?1 AND timestamp > datetime('now', 'start of month')",
                rusqlite::params![user_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Per-user spend ranking, sorted by daily cost descending.
    ///
    /// Anonymous spend (rows with `user_id IS NULL`) is excluded — the
    /// ranking is meant for human attribution, not totals. `limit` caps
    /// the result set; pass `None` for "no limit".
    pub fn query_user_ranking(&self, limit: Option<u32>) -> LibreFangResult<Vec<UserSpendRanking>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        // Aggregate three time windows in a single round-trip via
        // CASE-when sums, then sort by daily desc — the interesting
        // signal for "who spent the most today". `LIMIT` is bound as a
        // parameter (rather than format!()'d into the SQL) to match the
        // rest of this module — the value is a clamped u32 so injection
        // isn't a real risk, but keeping the convention uniform avoids
        // future copy-paste from this site landing on user-controlled
        // input.
        const RANKING_SQL: &str = "SELECT user_id, \
                COALESCE(SUM(CASE WHEN timestamp > datetime('now', '-1 hour') THEN cost_usd ELSE 0 END), 0.0) AS hourly, \
                COALESCE(SUM(CASE WHEN timestamp > datetime('now', 'start of day') THEN cost_usd ELSE 0 END), 0.0) AS daily, \
                COALESCE(SUM(CASE WHEN timestamp > datetime('now', 'start of month') THEN cost_usd ELSE 0 END), 0.0) AS monthly, \
                COUNT(*) AS calls \
             FROM usage_events \
             WHERE user_id IS NOT NULL \
             GROUP BY user_id \
             ORDER BY daily DESC, monthly DESC \
             LIMIT ?1";
        // SQLite treats a negative LIMIT as "no limit" — so `None` maps to
        // -1 and `Some(n)` clamps to 1000 (same hard cap the call sites use).
        let bound_limit: i64 = match limit {
            Some(n) => n.min(1000) as i64,
            None => -1,
        };

        let mut stmt = conn
            .prepare(RANKING_SQL)
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![bound_limit], |row| {
                Ok(UserSpendRanking {
                    user_id: row.get::<_, String>(0)?,
                    hourly_cost_usd: row.get(1)?,
                    daily_cost_usd: row.get(2)?,
                    monthly_cost_usd: row.get(3)?,
                    call_count: row.get::<_, i64>(4)?.max(0) as u64,
                })
            })
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let out: Vec<UserSpendRanking> = rows.filter_map(|r| r.ok()).collect();
        Ok(out)
    }

    /// Query total cost across all agents for the current hour.
    pub fn query_global_hourly(&self) -> LibreFangResult<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE timestamp > datetime('now', '-1 hour')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Query total cost across all agents for the current calendar month.
    pub fn query_global_monthly(&self) -> LibreFangResult<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE timestamp > datetime('now', 'start of month')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Query usage summary, optionally filtered by agent.
    pub fn query_summary(&self, agent_id: Option<AgentId>) -> LibreFangResult<UsageSummary> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match agent_id {
            Some(aid) => (
                "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                        COALESCE(SUM(cost_usd), 0.0), COUNT(*), COALESCE(SUM(tool_calls), 0)
                 FROM usage_events WHERE agent_id = ?1",
                vec![Box::new(aid.0.to_string())],
            ),
            None => (
                "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                        COALESCE(SUM(cost_usd), 0.0), COUNT(*), COALESCE(SUM(tool_calls), 0)
                 FROM usage_events",
                vec![],
            ),
        };

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let summary = conn
            .query_row(sql, params_refs.as_slice(), |row| {
                Ok(UsageSummary {
                    total_input_tokens: row.get::<_, i64>(0)? as u64,
                    total_output_tokens: row.get::<_, i64>(1)? as u64,
                    total_cost_usd: row.get(2)?,
                    call_count: row.get::<_, i64>(3)? as u64,
                    total_tool_calls: row.get::<_, i64>(4)? as u64,
                })
            })
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        Ok(summary)
    }

    /// Query usage grouped by model.
    pub fn query_by_model(&self) -> LibreFangResult<Vec<ModelUsage>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT model, COALESCE(SUM(cost_usd), 0.0), COALESCE(SUM(input_tokens), 0),
                        COALESCE(SUM(output_tokens), 0), COUNT(*)
                 FROM usage_events GROUP BY model ORDER BY SUM(cost_usd) DESC",
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(ModelUsage {
                    model: row.get(0)?,
                    total_cost_usd: row.get(1)?,
                    total_input_tokens: row.get::<_, i64>(2)? as u64,
                    total_output_tokens: row.get::<_, i64>(3)? as u64,
                    call_count: row.get::<_, i64>(4)? as u64,
                })
            })
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| LibreFangError::Memory(e.to_string()))?);
        }
        Ok(results)
    }

    /// Query model performance metrics including latency statistics.
    pub fn query_model_performance(&self) -> LibreFangResult<Vec<ModelPerformance>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT model, 
                        COALESCE(SUM(cost_usd), 0.0), 
                        COALESCE(SUM(input_tokens), 0), 
                        COALESCE(SUM(output_tokens), 0), 
                        COUNT(*),
                        COALESCE(AVG(latency_ms), 0),
                        COALESCE(MIN(latency_ms), 0),
                        COALESCE(MAX(latency_ms), 0)
                 FROM usage_events 
                 GROUP BY model 
                 ORDER BY SUM(cost_usd) DESC",
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let call_count: i64 = row.get(4)?;
                let total_cost_usd: f64 = row.get(1)?;
                let avg_latency_ms: f64 = row.get(5)?;

                Ok(ModelPerformance {
                    model: row.get(0)?,
                    total_cost_usd,
                    total_input_tokens: row.get::<_, i64>(2)? as u64,
                    total_output_tokens: row.get::<_, i64>(3)? as u64,
                    call_count: call_count as u64,
                    avg_latency_ms,
                    min_latency_ms: row.get::<_, i64>(6)? as u64,
                    max_latency_ms: row.get::<_, i64>(7)? as u64,
                    cost_per_call: if call_count > 0 {
                        total_cost_usd / call_count as f64
                    } else {
                        0.0
                    },
                    avg_latency_per_call: avg_latency_ms,
                })
            })
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| LibreFangError::Memory(e.to_string()))?);
        }
        Ok(results)
    }

    /// Query daily usage breakdown for the last N days.
    pub fn query_daily_breakdown(&self, days: u32) -> LibreFangResult<Vec<DailyBreakdown>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let mut stmt = conn
            .prepare(&format!(
                "SELECT date(timestamp) as day,
                            COALESCE(SUM(cost_usd), 0.0),
                            COALESCE(SUM(input_tokens) + SUM(output_tokens), 0),
                            COUNT(*)
                     FROM usage_events
                     WHERE timestamp > datetime('now', '-{days} days')
                     GROUP BY day
                     ORDER BY day ASC"
            ))
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(DailyBreakdown {
                    date: row.get(0)?,
                    cost_usd: row.get(1)?,
                    tokens: row.get::<_, i64>(2)? as u64,
                    calls: row.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| LibreFangError::Memory(e.to_string()))?);
        }
        Ok(results)
    }

    /// Query the timestamp of the earliest usage event.
    pub fn query_first_event_date(&self) -> LibreFangResult<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let result: Option<String> = conn
            .query_row("SELECT MIN(timestamp) FROM usage_events", [], |row| {
                row.get(0)
            })
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(result)
    }

    /// Query today's total cost across all agents.
    pub fn query_today_cost(&self) -> LibreFangResult<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE timestamp > datetime('now', 'start of day')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Query today's cost for every agent in a single SQL pass.
    ///
    /// Returns a `Vec<(AgentId, f64)>` sorted by cost descending. Using a
    /// single `GROUP BY` query instead of N per-agent `SUM` queries eliminates
    /// the N+1 pattern in `/api/budget/agents`, which was responsible for up
    /// to 1200 queries/min under typical dashboard polling. See #3684.
    pub fn query_all_agents_daily(&self) -> LibreFangResult<Vec<(AgentId, f64)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT agent_id, SUM(cost_usd) as total_cost
                 FROM usage_events
                 WHERE timestamp > datetime('now', 'start of day')
                 GROUP BY agent_id
                 ORDER BY total_cost DESC",
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                let id_str: String = row.get(0)?;
                let cost: f64 = row.get(1)?;
                Ok((id_str, cost))
            })
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let mut results = Vec::new();
        for row in rows {
            let (id_str, cost) = row.map_err(|e| LibreFangError::Memory(e.to_string()))?;
            if let Ok(agent_id) = id_str.parse::<AgentId>() {
                results.push((agent_id, cost));
            }
        }
        Ok(results)
    }

    /// Recent usage events for one agent — backs the dashboard's
    /// agent-detail Logs tab so it shows turn-level operational data
    /// (model / latency / tokens / cost) instead of the audit ledger,
    /// which is mostly admin lifecycle entries. Newest first.
    pub fn list_agent_events_recent(
        &self,
        agent_id: AgentId,
        limit: u32,
    ) -> LibreFangResult<Vec<AgentEventRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT timestamp, model, provider, input_tokens, output_tokens,
                        cost_usd, tool_calls, latency_ms
                 FROM usage_events
                 WHERE agent_id = ?1
                 ORDER BY timestamp DESC
                 LIMIT ?2",
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let rows = stmt
            .query_map(
                rusqlite::params![agent_id.0.to_string(), limit as i64],
                |row| {
                    Ok(AgentEventRow {
                        timestamp: row.get(0)?,
                        model: row.get(1)?,
                        provider: row.get(2)?,
                        input_tokens: row.get::<_, i64>(3)?.max(0) as u64,
                        output_tokens: row.get::<_, i64>(4)?.max(0) as u64,
                        cost_usd: row.get(5)?,
                        tool_calls: row.get::<_, i64>(6)?.max(0) as u64,
                        latency_ms: row.get::<_, i64>(7)?.max(0) as u64,
                    })
                },
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| LibreFangError::Memory(e.to_string()))?);
        }
        Ok(out)
    }

    /// Delete usage events older than the given number of days.
    pub fn cleanup_old(&self, days: u32) -> LibreFangResult<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let deleted = conn
            .execute(
                &format!(
                    "DELETE FROM usage_events WHERE timestamp < datetime('now', '-{days} days')"
                ),
                [],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn setup() -> UsageStore {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        UsageStore::new(Arc::new(Mutex::new(conn)))
    }

    #[test]
    fn test_record_and_query_summary() {
        let store = setup();
        let agent_id = AgentId::new();

        store
            .record(&UsageRecord {
                agent_id,
                provider: String::new(),
                model: "claude-haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.001,
                tool_calls: 2,
                latency_ms: 150,
                ..Default::default()
            })
            .unwrap();

        store
            .record(&UsageRecord {
                agent_id,
                provider: String::new(),
                model: "claude-sonnet".to_string(),
                input_tokens: 500,
                output_tokens: 200,
                cost_usd: 0.01,
                tool_calls: 1,
                latency_ms: 300,
                ..Default::default()
            })
            .unwrap();

        let summary = store.query_summary(Some(agent_id)).unwrap();
        assert_eq!(summary.call_count, 2);
        assert_eq!(summary.total_input_tokens, 600);
        assert_eq!(summary.total_output_tokens, 250);
        assert!((summary.total_cost_usd - 0.011).abs() < 0.0001);
        assert_eq!(summary.total_tool_calls, 3);
    }

    #[test]
    fn test_query_summary_all_agents() {
        let store = setup();
        let a1 = AgentId::new();
        let a2 = AgentId::new();

        store
            .record(&UsageRecord {
                agent_id: a1,
                provider: String::new(),
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.001,
                tool_calls: 0,
                latency_ms: 100,
                ..Default::default()
            })
            .unwrap();

        store
            .record(&UsageRecord {
                agent_id: a2,
                provider: String::new(),
                model: "sonnet".to_string(),
                input_tokens: 200,
                output_tokens: 100,
                cost_usd: 0.005,
                tool_calls: 1,
                latency_ms: 200,
                ..Default::default()
            })
            .unwrap();

        let summary = store.query_summary(None).unwrap();
        assert_eq!(summary.call_count, 2);
        assert_eq!(summary.total_input_tokens, 300);
    }

    #[test]
    fn test_query_by_model() {
        let store = setup();
        let agent_id = AgentId::new();

        for _ in 0..3 {
            store
                .record(&UsageRecord {
                    agent_id,
                    provider: String::new(),
                    model: "haiku".to_string(),
                    input_tokens: 100,
                    output_tokens: 50,
                    cost_usd: 0.001,
                    tool_calls: 0,
                    latency_ms: 100,
                    ..Default::default()
                })
                .unwrap();
        }

        store
            .record(&UsageRecord {
                agent_id,
                provider: String::new(),
                model: "sonnet".to_string(),
                input_tokens: 500,
                output_tokens: 200,
                cost_usd: 0.01,
                tool_calls: 1,
                latency_ms: 250,
                ..Default::default()
            })
            .unwrap();

        let by_model = store.query_by_model().unwrap();
        assert_eq!(by_model.len(), 2);
        // sonnet should be first (highest cost)
        assert_eq!(by_model[0].model, "sonnet");
        assert_eq!(by_model[1].model, "haiku");
        assert_eq!(by_model[1].call_count, 3);
    }

    #[test]
    fn test_query_hourly() {
        let store = setup();
        let agent_id = AgentId::new();

        store
            .record(&UsageRecord {
                agent_id,
                provider: String::new(),
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.05,
                tool_calls: 0,
                latency_ms: 150,
                ..Default::default()
            })
            .unwrap();

        let hourly = store.query_hourly(agent_id).unwrap();
        assert!((hourly - 0.05).abs() < 0.001);
    }

    #[test]
    fn test_query_daily() {
        let store = setup();
        let agent_id = AgentId::new();

        store
            .record(&UsageRecord {
                agent_id,
                provider: String::new(),
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.123,
                tool_calls: 0,
                latency_ms: 100,
                ..Default::default()
            })
            .unwrap();

        let daily = store.query_daily(agent_id).unwrap();
        assert!((daily - 0.123).abs() < 0.001);
    }

    #[test]
    fn test_cleanup_old() {
        let store = setup();
        let agent_id = AgentId::new();

        store
            .record(&UsageRecord {
                agent_id,
                provider: String::new(),
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.001,
                tool_calls: 0,
                latency_ms: 100,
                ..Default::default()
            })
            .unwrap();

        // Cleanup events older than 1 day should not remove today's events
        let deleted = store.cleanup_old(1).unwrap();
        assert_eq!(deleted, 0);

        let summary = store.query_summary(None).unwrap();
        assert_eq!(summary.call_count, 1);
    }

    #[test]
    fn test_empty_summary() {
        let store = setup();
        let summary = store.query_summary(None).unwrap();
        assert_eq!(summary.call_count, 0);
        assert_eq!(summary.total_cost_usd, 0.0);
    }

    #[test]
    fn test_query_model_performance() {
        let store = setup();
        let agent_id = AgentId::new();

        // Record usage events with different latencies
        for (latency, cost) in [(100, 0.001), (200, 0.002), (300, 0.003)] {
            store
                .record(&UsageRecord {
                    agent_id,
                    provider: String::new(),
                    model: "haiku".to_string(),
                    input_tokens: 100,
                    output_tokens: 50,
                    cost_usd: cost,
                    tool_calls: 0,
                    latency_ms: latency,
                    ..Default::default()
                })
                .unwrap();
        }

        store
            .record(&UsageRecord {
                agent_id,
                provider: String::new(),
                model: "sonnet".to_string(),
                input_tokens: 500,
                output_tokens: 200,
                cost_usd: 0.01,
                tool_calls: 1,
                latency_ms: 500,
                ..Default::default()
            })
            .unwrap();

        let performance = store.query_model_performance().unwrap();
        assert_eq!(performance.len(), 2);

        // sonnet should be first (highest cost)
        let sonnet = &performance[0];
        assert_eq!(sonnet.model, "sonnet");
        assert_eq!(sonnet.call_count, 1);
        assert!((sonnet.avg_latency_ms - 500.0).abs() < 0.1);

        let haiku = &performance[1];
        assert_eq!(haiku.model, "haiku");
        assert_eq!(haiku.call_count, 3);
        // Average of 100, 200, 300 = 200
        assert!((haiku.avg_latency_ms - 200.0).abs() < 0.1);
        assert_eq!(haiku.min_latency_ms, 100);
        assert_eq!(haiku.max_latency_ms, 300);
    }

    #[test]
    fn test_check_quota_and_record_under_limit() {
        let store = setup();
        let agent_id = AgentId::new();

        let result = store.check_quota_and_record(
            &UsageRecord {
                agent_id,
                provider: String::new(),
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.001,
                tool_calls: 0,
                latency_ms: 100,
                ..Default::default()
            },
            1.0,   // hourly
            10.0,  // daily
            100.0, // monthly
        );
        assert!(result.is_ok());

        // Verify the record was actually inserted
        let summary = store.query_summary(Some(agent_id)).unwrap();
        assert_eq!(summary.call_count, 1);
    }

    #[test]
    fn test_check_quota_and_record_exceeds_hourly() {
        let store = setup();
        let agent_id = AgentId::new();

        // First record: use up most of the budget
        store
            .record(&UsageRecord {
                agent_id,
                provider: String::new(),
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.009,
                tool_calls: 0,
                latency_ms: 100,
                ..Default::default()
            })
            .unwrap();

        // Second record: should be rejected atomically
        let result = store.check_quota_and_record(
            &UsageRecord {
                agent_id,
                provider: String::new(),
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.002,
                tool_calls: 0,
                latency_ms: 100,
                ..Default::default()
            },
            0.01, // hourly limit
            10.0,
            100.0,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("hourly cost quota"));

        // Verify the second record was NOT inserted
        let summary = store.query_summary(Some(agent_id)).unwrap();
        assert_eq!(summary.call_count, 1);
    }

    #[test]
    fn test_check_all_and_record_global_budget() {
        let store = setup();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        // Agent A uses some budget
        store
            .record(&UsageRecord {
                agent_id: agent_a,
                provider: String::new(),
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.008,
                tool_calls: 0,
                latency_ms: 100,
                ..Default::default()
            })
            .unwrap();

        // Agent B tries to record — per-agent quota is fine but global is exceeded
        let result = store.check_all_and_record(
            &UsageRecord {
                agent_id: agent_b,
                provider: String::new(),
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.005,
                tool_calls: 0,
                latency_ms: 100,
                ..Default::default()
            },
            1.0,   // agent hourly (fine)
            10.0,  // agent daily (fine)
            100.0, // agent monthly (fine)
            0.01,  // global hourly (exceeded: 0.008 + 0.005 >= 0.01)
            10.0,  // global daily
            100.0, // global monthly
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Global hourly budget exceeded"));

        // Agent B's record was NOT inserted
        let summary = store.query_summary(Some(agent_b)).unwrap();
        assert_eq!(summary.call_count, 0);
    }

    // ── RBAC M5: per-user spend rollup ──────────────────────────────────

    #[test]
    fn test_user_spend_rollup_per_window() {
        // Records carrying user_id must roll up cleanly into hourly /
        // daily / monthly totals; records WITHOUT user_id must not leak
        // into any user's spend bucket (anonymous spend stays anonymous).
        let store = setup();
        let alice = librefang_types::agent::UserId::from_name("Alice");
        let bob = librefang_types::agent::UserId::from_name("Bob");

        store
            .record(&UsageRecord {
                agent_id: AgentId::new(),
                cost_usd: 0.10,
                user_id: Some(alice),
                channel: Some("api".to_string()),
                ..Default::default()
            })
            .unwrap();
        store
            .record(&UsageRecord {
                agent_id: AgentId::new(),
                cost_usd: 0.05,
                user_id: Some(alice),
                channel: Some("telegram".to_string()),
                ..Default::default()
            })
            .unwrap();
        store
            .record(&UsageRecord {
                agent_id: AgentId::new(),
                cost_usd: 1.0,
                user_id: Some(bob),
                channel: Some("api".to_string()),
                ..Default::default()
            })
            .unwrap();
        // Anonymous spend — must NOT be attributed to anyone.
        store
            .record(&UsageRecord {
                agent_id: AgentId::new(),
                cost_usd: 999.0,
                user_id: None,
                channel: Some("cron".to_string()),
                ..Default::default()
            })
            .unwrap();

        assert!((store.query_user_hourly(alice).unwrap() - 0.15).abs() < 1e-9);
        assert!((store.query_user_daily(alice).unwrap() - 0.15).abs() < 1e-9);
        assert!((store.query_user_monthly(alice).unwrap() - 0.15).abs() < 1e-9);
        assert!((store.query_user_hourly(bob).unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_user_ranking_excludes_anonymous_and_orders_by_daily() {
        let store = setup();
        let alice = librefang_types::agent::UserId::from_name("Alice");
        let bob = librefang_types::agent::UserId::from_name("Bob");

        // Bob spends more than Alice; an anonymous spike is loudest of all
        // but must NOT appear in the ranking.
        store
            .record(&UsageRecord {
                agent_id: AgentId::new(),
                cost_usd: 5.0,
                user_id: Some(alice),
                ..Default::default()
            })
            .unwrap();
        store
            .record(&UsageRecord {
                agent_id: AgentId::new(),
                cost_usd: 12.5,
                user_id: Some(bob),
                ..Default::default()
            })
            .unwrap();
        store
            .record(&UsageRecord {
                agent_id: AgentId::new(),
                cost_usd: 9999.0,
                user_id: None,
                ..Default::default()
            })
            .unwrap();

        let ranking = store.query_user_ranking(Some(10)).unwrap();
        assert_eq!(ranking.len(), 2);
        // Bob first (higher daily), Alice second.
        assert_eq!(ranking[0].user_id, bob.to_string());
        assert_eq!(ranking[1].user_id, alice.to_string());
        assert!((ranking[0].daily_cost_usd - 12.5).abs() < 1e-9);
        assert!((ranking[1].daily_cost_usd - 5.0).abs() < 1e-9);
    }
}
