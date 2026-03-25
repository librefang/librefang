//! Usage tracking store — records LLM usage events for cost monitoring.

use chrono::Utc;
use librefang_types::agent::AgentId;
use librefang_types::error::{LibreFangError, LibreFangResult};
use rusqlite::{Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// A single usage event recording an LLM call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    /// Which agent made the call.
    pub agent_id: AgentId,
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
        conn.execute(
            "INSERT INTO usage_events (id, agent_id, timestamp, model, input_tokens, output_tokens, cost_usd, tool_calls, latency_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                id,
                record.agent_id.0.to_string(),
                now,
                record.model,
                record.input_tokens as i64,
                record.output_tokens as i64,
                record.cost_usd,
                record.tool_calls as i64,
                record.latency_ms as i64,
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
                model: "claude-haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.001,
                tool_calls: 2,
                latency_ms: 150,
            })
            .unwrap();

        store
            .record(&UsageRecord {
                agent_id,
                model: "claude-sonnet".to_string(),
                input_tokens: 500,
                output_tokens: 200,
                cost_usd: 0.01,
                tool_calls: 1,
                latency_ms: 300,
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
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.001,
                tool_calls: 0,
                latency_ms: 100,
            })
            .unwrap();

        store
            .record(&UsageRecord {
                agent_id: a2,
                model: "sonnet".to_string(),
                input_tokens: 200,
                output_tokens: 100,
                cost_usd: 0.005,
                tool_calls: 1,
                latency_ms: 200,
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
                    model: "haiku".to_string(),
                    input_tokens: 100,
                    output_tokens: 50,
                    cost_usd: 0.001,
                    tool_calls: 0,
                    latency_ms: 100,
                })
                .unwrap();
        }

        store
            .record(&UsageRecord {
                agent_id,
                model: "sonnet".to_string(),
                input_tokens: 500,
                output_tokens: 200,
                cost_usd: 0.01,
                tool_calls: 1,
                latency_ms: 250,
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
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.05,
                tool_calls: 0,
                latency_ms: 150,
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
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.123,
                tool_calls: 0,
                latency_ms: 100,
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
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.001,
                tool_calls: 0,
                latency_ms: 100,
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
                    model: "haiku".to_string(),
                    input_tokens: 100,
                    output_tokens: 50,
                    cost_usd: cost,
                    tool_calls: 0,
                    latency_ms: latency,
                })
                .unwrap();
        }

        store
            .record(&UsageRecord {
                agent_id,
                model: "sonnet".to_string(),
                input_tokens: 500,
                output_tokens: 200,
                cost_usd: 0.01,
                tool_calls: 1,
                latency_ms: 500,
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
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.001,
                tool_calls: 0,
                latency_ms: 100,
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
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.009,
                tool_calls: 0,
                latency_ms: 100,
            })
            .unwrap();

        // Second record: should be rejected atomically
        let result = store.check_quota_and_record(
            &UsageRecord {
                agent_id,
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.002,
                tool_calls: 0,
                latency_ms: 100,
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
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.008,
                tool_calls: 0,
                latency_ms: 100,
            })
            .unwrap();

        // Agent B tries to record — per-agent quota is fine but global is exceeded
        let result = store.check_all_and_record(
            &UsageRecord {
                agent_id: agent_b,
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.005,
                tool_calls: 0,
                latency_ms: 100,
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
}
