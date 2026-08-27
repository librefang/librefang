//! Usage tracking store — records LLM usage events for cost monitoring.

use chrono::{NaiveDate, Utc};
use librefang_types::agent::{AgentId, SessionId, UserId};
use librefang_types::error::{LibreFangError, LibreFangResult};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Date-range filtering (#7891)
// ---------------------------------------------------------------------------

/// An inclusive calendar-date filter over `usage_events.timestamp`.
///
/// Both bounds are optional, and a `DateRange` with neither bound set is the
/// identity filter: it contributes no SQL predicate at all, so every query
/// that takes one keeps its pre-#7891 result set byte for byte.
/// That property is what makes the new query parameters backward compatible
/// rather than merely defaulted.
///
/// # Why the predicate compares the raw column instead of `date(timestamp)`
///
/// The obvious spelling is `date(timestamp) BETWEEN ?start AND ?end`, but
/// wrapping the column in a function makes the predicate non-sargable: SQLite
/// cannot use `idx_usage_timestamp` and falls back to a full table scan of
/// `usage_events`, which is the one table in the substrate that grows without
/// bound in normal operation.
/// Comparing the raw text column against a bare `YYYY-MM-DD` prefix keeps the
/// index in play, because the stored values are RFC 3339 strings whose
/// lexicographic order matches their chronological order.
///
/// The upper bound is therefore stored half-open — `end_date + 1 day`,
/// compared with `<` — so that every instant on `end_date` is included while
/// `2026-09-01T00:00:00+00:00` is not.
/// A closed `<=` bound against the bare date would have excluded all but the
/// midnight instant of the final day, which is the classic off-by-one that
/// silently drops a day of spend from a monthly report.
///
/// # UTC assumption
///
/// The only writer of this column is `UsageStore::insert_record`, which
/// stores `Utc::now().to_rfc3339()`, so every row carries a `+00:00` offset
/// and the lexicographic comparison is exact.
/// Dates supplied by a caller are likewise interpreted as UTC calendar days.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DateRange {
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
}

/// Why a [`DateRange`] could not be constructed.
///
/// Carries enough detail for the API layer to render an actionable 400 rather
/// than the caller silently receiving an empty result set for a range they
/// typed wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateRangeError {
    /// A bound did not parse as `YYYY-MM-DD`. Holds the parameter name and the rejected value.
    Malformed {
        /// Query-parameter name the bad value came from (`start_date` / `end_date`).
        field: &'static str,
        /// The value as the caller supplied it.
        value: String,
    },
    /// `end_date` is chronologically before `start_date`.
    Inverted {
        /// The supplied start bound.
        start: NaiveDate,
        /// The supplied end bound.
        end: NaiveDate,
    },
}

impl std::fmt::Display for DateRangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed { field, value } => write!(
                f,
                "`{field}` must be a calendar date in YYYY-MM-DD form (got {value:?})"
            ),
            Self::Inverted { start, end } => write!(
                f,
                "`end_date` ({end}) is before `start_date` ({start}); supply a range whose end is on or after its start"
            ),
        }
    }
}

impl std::error::Error for DateRangeError {}

impl DateRange {
    /// The identity filter — matches every row.
    pub const UNBOUNDED: Self = Self {
        start: None,
        end: None,
    };

    /// Build a range from two already-parsed bounds, rejecting an inverted range.
    pub fn new(start: Option<NaiveDate>, end: Option<NaiveDate>) -> Result<Self, DateRangeError> {
        if let (Some(s), Some(e)) = (start, end) {
            if e < s {
                return Err(DateRangeError::Inverted { start: s, end: e });
            }
        }
        Ok(Self { start, end })
    }

    /// Parse two optional `YYYY-MM-DD` strings into a range.
    ///
    /// An empty string is treated as an absent bound so that a client
    /// rendering `?start_date=&end_date=` from a form with cleared inputs gets
    /// the unfiltered response rather than a 400.
    pub fn parse(start: Option<&str>, end: Option<&str>) -> Result<Self, DateRangeError> {
        fn one(
            field: &'static str,
            raw: Option<&str>,
        ) -> Result<Option<NaiveDate>, DateRangeError> {
            match raw.map(str::trim) {
                None | Some("") => Ok(None),
                Some(v) => NaiveDate::parse_from_str(v, "%Y-%m-%d")
                    .map(Some)
                    .map_err(|_| DateRangeError::Malformed {
                        field,
                        value: v.to_string(),
                    }),
            }
        }
        Self::new(one("start_date", start)?, one("end_date", end)?)
    }

    /// Whether this range constrains anything.
    pub fn is_unbounded(&self) -> bool {
        self.start.is_none() && self.end.is_none()
    }

    /// The inclusive start bound, if set.
    pub fn start(&self) -> Option<NaiveDate> {
        self.start
    }

    /// The inclusive end bound, if set.
    pub fn end(&self) -> Option<NaiveDate> {
        self.end
    }

    /// SQL predicates to AND into a `WHERE` clause, plus their bind values in
    /// positional order.
    ///
    /// The fragment is empty for an unbounded range, so callers that splice it
    /// into a `WHERE 1=1` base emit exactly the pre-filter statement.
    fn sql_and_binds(&self) -> (String, Vec<String>) {
        let mut sql = String::new();
        let mut binds = Vec::new();
        if let Some(start) = self.start {
            sql.push_str(" AND timestamp >= ?");
            binds.push(start.format("%Y-%m-%d").to_string());
        }
        // Half-open upper bound: see the type-level note on the off-by-one.
        //
        // `succ_opt` is `None` only for `NaiveDate::MAX`, where "the day after
        // the end bound" does not exist. That bound already includes every
        // representable timestamp, so the predicate is simply omitted —
        // emitting `NaiveDate::MAX` as text instead would compare a `+`-prefixed
        // string against the stored digits and match nothing at all.
        if let Some(next) = self.end.and_then(|end| end.succ_opt()) {
            sql.push_str(" AND timestamp < ?");
            binds.push(next.format("%Y-%m-%d").to_string());
        }
        (sql, binds)
    }
}

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
    /// #7714: the agent this call's cost rolls up to, when it differs from
    /// the agent that made it.
    ///
    /// A worker spawned by another agent spends on its spawner's behalf, so
    /// the spawner needs that cost on its own budget line rather than
    /// scattered across throwaway children it cannot enumerate.
    /// Call sites set it to `entry.parent.unwrap_or(agent_id)`, so a
    /// top-level agent bills to itself.
    ///
    /// This is deliberately a *separate* column from [`Self::agent_id`]
    /// rather than a rewrite of it. `agent_id` stays the quota subject: the
    /// pre-call `check_quota` and the post-call `check_all_and_record` both
    /// evaluate the executing agent against that agent's own
    /// `manifest.resources`, so the two checks keep asking the same question
    /// about the same agent. Re-pointing `agent_id` at the parent would have
    /// made the pre-call check read the child's spend and the post-call check
    /// read the parent's, both against the child's ceiling — the attribution
    /// dimension and the enforcement dimension have to stay independent.
    ///
    /// `None` on pre-v49 records, which the read path treats as "bills to
    /// `agent_id`".
    #[serde(default)]
    pub billed_agent_id: Option<AgentId>,
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
            billed_agent_id: None,
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

/// One `usage_events` row, flattened for bulk export (#7891).
///
/// Carries every persisted column rather than the observability subset in
/// [`AgentEventRow`]: an archival export is the last copy of the data once
/// retention prunes the table, so dropping attribution columns here would make
/// the archive unable to answer the per-agent / per-user questions the live
/// endpoints can.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageExportRow {
    /// RFC 3339 timestamp of the call, as stored.
    pub timestamp: String,
    /// Agent that executed the call.
    pub agent_id: String,
    /// Agent whose budget the call bills to, when it differs from `agent_id` (#7714).
    pub billed_agent_id: Option<String>,
    /// Provider id.
    pub provider: String,
    /// Model id.
    pub model: String,
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens produced.
    pub output_tokens: u64,
    /// Estimated cost in USD.
    pub cost_usd: f64,
    /// Tool calls made during the interaction.
    pub tool_calls: u64,
    /// Round-trip latency in milliseconds.
    pub latency_ms: u64,
    /// LibreFang user the call is attributed to, when known.
    pub user_id: Option<String>,
    /// Originating channel, when known.
    pub channel: Option<String>,
    /// Session the call belonged to, when known.
    pub session_id: Option<String>,
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

fn validate_usage_record(record: &UsageRecord) -> LibreFangResult<()> {
    if !record.cost_usd.is_finite() || record.cost_usd < 0.0 {
        return Err(LibreFangError::memory_msg(
            "usage record cost_usd must be finite and non-negative",
        ));
    }
    Ok(())
}

fn validate_cost_limits(limits: &[f64]) -> LibreFangResult<()> {
    if limits
        .iter()
        .any(|limit| !limit.is_finite() || *limit < 0.0)
    {
        return Err(LibreFangError::memory_msg(
            "usage cost limits must be finite and non-negative",
        ));
    }
    Ok(())
}

fn cost_limit_exceeded(current: f64, incoming: f64, limit: f64) -> bool {
    let total = current + incoming;
    let tolerance = total.abs().max(limit.abs()).max(1.0) * 1e-12;
    total - limit > tolerance
}

/// Usage store backed by SQLite.
#[derive(Clone)]
pub struct UsageStore {
    pool: Pool<SqliteConnectionManager>,
}

impl UsageStore {
    /// Create a new usage store wrapping the given connection.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }

    /// Record a usage event.
    pub fn record(&self, record: &UsageRecord) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        Self::insert_record(&conn, record)
    }

    /// Insert a usage record into the database (helper used by both `record`
    /// and the atomic `check_quota_and_record`).
    fn insert_record(conn: &Connection, record: &UsageRecord) -> LibreFangResult<()> {
        validate_usage_record(record)?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        // RBAC M5 + session attribution: persist user_id/channel/session_id
        // alongside the existing columns. Schema v23 added user_id/channel,
        // v30 added session_id — all are NULL-able so missing attribution
        // round-trips as NULL.
        conn.execute(
            "INSERT INTO usage_events (id, agent_id, timestamp, model, provider, input_tokens, output_tokens, cost_usd, tool_calls, latency_ms, user_id, channel, session_id, billed_agent_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
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
                record.billed_agent_id.map(|a| a.0.to_string()),
            ],
        )
        .map_err(LibreFangError::memory)?;
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
        validate_cost_limits(&[max_hourly, max_daily, max_monthly])?;
        let mut conn = self.pool.get().map_err(LibreFangError::memory)?;

        // IMMEDIATE transaction acquires a reserved lock up-front, ensuring no
        // other writer can interleave between our SELECT and INSERT.  The RAII
        // guard auto-rolls back on drop if we return early (error or quota
        // exceeded), so every error path is safe.
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(LibreFangError::memory)?;

        let agent_str = record.agent_id.0.to_string();

        // Check hourly quota
        if max_hourly > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE agent_id = ?1 AND datetime(timestamp) > datetime('now', '-1 hour')",
                    rusqlite::params![&agent_str],
                    |row| row.get(0),
                )
                .map_err(LibreFangError::memory)?;
            if cost_limit_exceeded(cost, record.cost_usd, max_hourly) {
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
                     WHERE agent_id = ?1 AND datetime(timestamp) > datetime('now', 'start of day')",
                    rusqlite::params![&agent_str],
                    |row| row.get(0),
                )
                .map_err(LibreFangError::memory)?;
            if cost_limit_exceeded(cost, record.cost_usd, max_daily) {
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
                     WHERE agent_id = ?1 AND datetime(timestamp) > datetime('now', 'start of month')",
                    rusqlite::params![&agent_str],
                    |row| row.get(0),
                )
                .map_err(LibreFangError::memory)?;
            if cost_limit_exceeded(cost, record.cost_usd, max_monthly) {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded monthly cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, cost, record.cost_usd, max_monthly
                )));
            }
        }

        // All checks passed — insert the record within the same transaction
        Self::insert_record(&tx, record)?;

        tx.commit().map_err(LibreFangError::memory)?;
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
        validate_cost_limits(&[max_hourly, max_daily, max_monthly])?;
        let mut conn = self.pool.get().map_err(LibreFangError::memory)?;

        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(LibreFangError::memory)?;

        // Check global hourly budget
        if max_hourly > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE datetime(timestamp) > datetime('now', '-1 hour')",
                    [],
                    |row| row.get(0),
                )
                .map_err(LibreFangError::memory)?;
            if cost_limit_exceeded(cost, record.cost_usd, max_hourly) {
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
                     WHERE datetime(timestamp) > datetime('now', 'start of day')",
                    [],
                    |row| row.get(0),
                )
                .map_err(LibreFangError::memory)?;
            if cost_limit_exceeded(cost, record.cost_usd, max_daily) {
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
                     WHERE datetime(timestamp) > datetime('now', 'start of month')",
                    [],
                    |row| row.get(0),
                )
                .map_err(LibreFangError::memory)?;
            if cost_limit_exceeded(cost, record.cost_usd, max_monthly) {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global monthly budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    cost, record.cost_usd, max_monthly
                )));
            }
        }

        // All checks passed — insert the record
        Self::insert_record(&tx, record)?;

        tx.commit().map_err(LibreFangError::memory)?;
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
        validate_cost_limits(&[
            agent_max_hourly,
            agent_max_daily,
            agent_max_monthly,
            global_max_hourly,
            global_max_daily,
            global_max_monthly,
        ])?;
        let mut conn = self.pool.get().map_err(LibreFangError::memory)?;

        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(LibreFangError::memory)?;

        let agent_str = record.agent_id.0.to_string();

        // ── Per-agent quota checks ──────────────────────────────────
        if agent_max_hourly > 0.0 {
            let cost: f64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                     WHERE agent_id = ?1 AND datetime(timestamp) > datetime('now', '-1 hour')",
                    rusqlite::params![&agent_str],
                    |row| row.get(0),
                )
                .map_err(LibreFangError::memory)?;
            if cost_limit_exceeded(cost, record.cost_usd, agent_max_hourly) {
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
                     WHERE agent_id = ?1 AND datetime(timestamp) > datetime('now', 'start of day')",
                    rusqlite::params![&agent_str],
                    |row| row.get(0),
                )
                .map_err(LibreFangError::memory)?;
            if cost_limit_exceeded(cost, record.cost_usd, agent_max_daily) {
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
                     WHERE agent_id = ?1 AND datetime(timestamp) > datetime('now', 'start of month')",
                    rusqlite::params![&agent_str],
                    |row| row.get(0),
                )
                .map_err(LibreFangError::memory)?;
            if cost_limit_exceeded(cost, record.cost_usd, agent_max_monthly) {
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
                     WHERE datetime(timestamp) > datetime('now', '-1 hour')",
                    [],
                    |row| row.get(0),
                )
                .map_err(LibreFangError::memory)?;
            if cost_limit_exceeded(cost, record.cost_usd, global_max_hourly) {
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
                     WHERE datetime(timestamp) > datetime('now', 'start of day')",
                    [],
                    |row| row.get(0),
                )
                .map_err(LibreFangError::memory)?;
            if cost_limit_exceeded(cost, record.cost_usd, global_max_daily) {
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
                     WHERE datetime(timestamp) > datetime('now', 'start of month')",
                    [],
                    |row| row.get(0),
                )
                .map_err(LibreFangError::memory)?;
            if cost_limit_exceeded(cost, record.cost_usd, global_max_monthly) {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global monthly budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    cost, record.cost_usd, global_max_monthly
                )));
            }
        }

        // All checks passed — insert the record
        Self::insert_record(&tx, record)?;

        tx.commit().map_err(LibreFangError::memory)?;
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
        validate_cost_limits(&[
            agent_max_hourly,
            agent_max_daily,
            agent_max_monthly,
            global_max_hourly,
            global_max_daily,
            global_max_monthly,
            provider_max_hourly,
            provider_max_daily,
            provider_max_monthly,
        ])?;
        let mut conn = self.pool.get().map_err(LibreFangError::memory)?;

        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(LibreFangError::memory)?;

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
        // `where_clause` selects the rows for the window (e.g. `datetime(timestamp) > datetime(...)`).
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
                .map_err(LibreFangError::memory)?;
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
            let costs = window_costs("datetime(timestamp) > datetime('now', '-1 hour')")?;
            if agent_max_hourly > 0.0
                && cost_limit_exceeded(costs.agent, record.cost_usd, agent_max_hourly)
            {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded hourly cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, costs.agent, record.cost_usd, agent_max_hourly
                )));
            }
            if global_max_hourly > 0.0
                && cost_limit_exceeded(costs.global, record.cost_usd, global_max_hourly)
            {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global hourly budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    costs.global, record.cost_usd, global_max_hourly
                )));
            }
            if has_provider
                && provider_max_hourly > 0.0
                && cost_limit_exceeded(costs.provider, record.cost_usd, provider_max_hourly)
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
            let costs = window_costs("datetime(timestamp) > datetime('now', 'start of day')")?;
            if agent_max_daily > 0.0
                && cost_limit_exceeded(costs.agent, record.cost_usd, agent_max_daily)
            {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded daily cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, costs.agent, record.cost_usd, agent_max_daily
                )));
            }
            if global_max_daily > 0.0
                && cost_limit_exceeded(costs.global, record.cost_usd, global_max_daily)
            {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global daily budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    costs.global, record.cost_usd, global_max_daily
                )));
            }
            if has_provider
                && provider_max_daily > 0.0
                && cost_limit_exceeded(costs.provider, record.cost_usd, provider_max_daily)
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
            let costs = window_costs("datetime(timestamp) > datetime('now', 'start of month')")?;
            if agent_max_monthly > 0.0
                && cost_limit_exceeded(costs.agent, record.cost_usd, agent_max_monthly)
            {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Agent {} exceeded monthly cost quota: ${:.4} + ${:.4} / ${:.4}",
                    record.agent_id, costs.agent, record.cost_usd, agent_max_monthly
                )));
            }
            if global_max_monthly > 0.0
                && cost_limit_exceeded(costs.global, record.cost_usd, global_max_monthly)
            {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Global monthly budget exceeded: ${:.4} + ${:.4} / ${:.4}",
                    costs.global, record.cost_usd, global_max_monthly
                )));
            }
            if has_provider
                && provider_max_monthly > 0.0
                && cost_limit_exceeded(costs.provider, record.cost_usd, provider_max_monthly)
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
                     WHERE provider = ?1 AND datetime(timestamp) > datetime('now', '-1 hour')",
                    rusqlite::params![&record.provider],
                    |row| row.get(0),
                )
                .map_err(LibreFangError::memory)?;
            let current = tokens.max(0) as u64;
            let incoming = record.input_tokens.saturating_add(record.output_tokens);
            if current.saturating_add(incoming) > provider_max_tokens_per_hour {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Provider '{}' exceeded hourly token budget: {} + {} / {}",
                    record.provider, current, incoming, provider_max_tokens_per_hour
                )));
            }
        }

        // All checks passed — insert the record
        Self::insert_record(&tx, record)?;

        tx.commit().map_err(LibreFangError::memory)?;
        Ok(())
    }

    /// Query total cost in the last hour for an agent.
    pub fn query_hourly(&self, agent_id: AgentId) -> LibreFangResult<f64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE agent_id = ?1 AND datetime(timestamp) > datetime('now', '-1 hour')",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(cost)
    }

    /// Query total cost today for an agent.
    pub fn query_daily(&self, agent_id: AgentId) -> LibreFangResult<f64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE agent_id = ?1 AND datetime(timestamp) > datetime('now', 'start of day')",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(cost)
    }

    /// Query total cost in the current calendar month for an agent.
    pub fn query_monthly(&self, agent_id: AgentId) -> LibreFangResult<f64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE agent_id = ?1 AND datetime(timestamp) > datetime('now', 'start of month')",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(cost)
    }

    /// Query total cost for a specific provider in the last hour.
    pub fn query_provider_hourly(&self, provider: &str) -> LibreFangResult<f64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE provider = ?1 AND datetime(timestamp) > datetime('now', '-1 hour')",
                rusqlite::params![provider],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(cost)
    }

    /// Query total cost for a specific provider today.
    pub fn query_provider_daily(&self, provider: &str) -> LibreFangResult<f64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE provider = ?1 AND datetime(timestamp) > datetime('now', 'start of day')",
                rusqlite::params![provider],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(cost)
    }

    /// Query total cost for a specific provider in the current calendar month.
    pub fn query_provider_monthly(&self, provider: &str) -> LibreFangResult<f64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE provider = ?1 AND datetime(timestamp) > datetime('now', 'start of month')",
                rusqlite::params![provider],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(cost)
    }

    /// Query total tokens (input + output) for a specific provider in the last hour.
    pub fn query_provider_tokens_hourly(&self, provider: &str) -> LibreFangResult<u64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let tokens: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(input_tokens) + SUM(output_tokens), 0) FROM usage_events
                 WHERE provider = ?1 AND datetime(timestamp) > datetime('now', '-1 hour')",
                rusqlite::params![provider],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(tokens.max(0) as u64)
    }

    /// Distinct provider identifiers observed in `usage_events` over the
    /// current calendar month (UTC). Returned sorted ascending so the
    /// caller can rely on stable ordering when merging with the operator's
    /// `[budget.providers]` configuration map (#5650).
    ///
    /// Rows with an empty provider string are excluded — those are pre-#4807
    /// usage entries that pre-date provider attribution and would otherwise
    /// surface in the dashboard as an unnamed row the operator can't act on.
    ///
    /// Month window mirrors the longest `query_provider_*` rollup, so any
    /// provider that contributed spend within the time horizon the
    /// `[budget.providers]` table can cap is discoverable. Anything older
    /// is operationally inert — no monthly cap applies to it.
    pub fn query_distinct_providers(&self) -> LibreFangResult<Vec<String>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT provider FROM usage_events
                 WHERE provider IS NOT NULL AND provider <> ''
                   AND datetime(timestamp) > datetime('now', 'start of month')
                 ORDER BY provider ASC",
            )
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(LibreFangError::memory)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(LibreFangError::memory)?);
        }
        Ok(out)
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
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE user_id = ?1 AND datetime(timestamp) > datetime('now', '-1 hour')",
                rusqlite::params![user_id.to_string()],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(cost)
    }

    /// Total cost today (UTC calendar day, see module-level note) for a single user.
    pub fn query_user_daily(&self, user_id: UserId) -> LibreFangResult<f64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE user_id = ?1 AND datetime(timestamp) > datetime('now', 'start of day')",
                rusqlite::params![user_id.to_string()],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(cost)
    }

    /// Total cost in the current UTC calendar month (see module-level note) for a single user.
    pub fn query_user_monthly(&self, user_id: UserId) -> LibreFangResult<f64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE user_id = ?1 AND datetime(timestamp) > datetime('now', 'start of month')",
                rusqlite::params![user_id.to_string()],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(cost)
    }

    /// Per-user spend ranking, sorted by daily cost descending.
    ///
    /// Anonymous spend (rows with `user_id IS NULL`) is excluded — the
    /// ranking is meant for human attribution, not totals. `limit` caps
    /// the result set; pass `None` for "no limit".
    pub fn query_user_ranking(&self, limit: Option<u32>) -> LibreFangResult<Vec<UserSpendRanking>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        // Aggregate three time windows in a single round-trip via
        // CASE-when sums, then sort by daily desc — the interesting
        // signal for "who spent the most today". `LIMIT` is bound as a
        // parameter (rather than format!()'d into the SQL) to match the
        // rest of this module — the value is a clamped u32 so injection
        // isn't a real risk, but keeping the convention uniform avoids
        // future copy-paste from this site landing on user-controlled
        // input.
        const RANKING_SQL: &str = "SELECT user_id, \
                COALESCE(SUM(CASE WHEN datetime(timestamp) > datetime('now', '-1 hour') THEN cost_usd ELSE 0 END), 0.0) AS hourly, \
                COALESCE(SUM(CASE WHEN datetime(timestamp) > datetime('now', 'start of day') THEN cost_usd ELSE 0 END), 0.0) AS daily, \
                COALESCE(SUM(CASE WHEN datetime(timestamp) > datetime('now', 'start of month') THEN cost_usd ELSE 0 END), 0.0) AS monthly, \
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

        let mut stmt = conn.prepare(RANKING_SQL).map_err(LibreFangError::memory)?;
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
            .map_err(LibreFangError::memory)?;
        let out = rows
            .collect::<rusqlite::Result<Vec<UserSpendRanking>>>()
            .map_err(LibreFangError::memory)?;
        Ok(out)
    }

    /// Query total cost across all agents for the current hour.
    pub fn query_global_hourly(&self) -> LibreFangResult<f64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE datetime(timestamp) > datetime('now', '-1 hour')",
                [],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(cost)
    }

    /// Query total cost across all agents for the current calendar month.
    pub fn query_global_monthly(&self) -> LibreFangResult<f64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE datetime(timestamp) > datetime('now', 'start of month')",
                [],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(cost)
    }

    /// Query usage summary, optionally filtered by agent.
    pub fn query_summary(&self, agent_id: Option<AgentId>) -> LibreFangResult<UsageSummary> {
        self.query_summary_ranged(agent_id, &DateRange::UNBOUNDED)
    }

    /// Query the usage summary, optionally restricted to a calendar-date range (#7891).
    ///
    /// Passing [`DateRange::UNBOUNDED`] is exactly equivalent to [`Self::query_summary`].
    pub fn query_summary_ranged(
        &self,
        agent_id: Option<AgentId>,
        range: &DateRange,
    ) -> LibreFangResult<UsageSummary> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        let mut sql = String::from(
            "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cost_usd), 0.0), COUNT(*), COALESCE(SUM(tool_calls), 0)
             FROM usage_events WHERE 1=1",
        );
        let mut binds: Vec<String> = Vec::new();
        if let Some(aid) = agent_id {
            sql.push_str(" AND agent_id = ?");
            binds.push(aid.0.to_string());
        }
        let (range_sql, range_binds) = range.sql_and_binds();
        sql.push_str(&range_sql);
        binds.extend(range_binds);

        let summary = conn
            .query_row(&sql, rusqlite::params_from_iter(binds.iter()), |row| {
                Ok(UsageSummary {
                    total_input_tokens: row.get::<_, i64>(0)? as u64,
                    total_output_tokens: row.get::<_, i64>(1)? as u64,
                    total_cost_usd: row.get(2)?,
                    call_count: row.get::<_, i64>(3)? as u64,
                    total_tool_calls: row.get::<_, i64>(4)? as u64,
                })
            })
            .map_err(LibreFangError::memory)?;

        Ok(summary)
    }

    /// Query the usage that rolls up to one agent's budget line (#7714).
    ///
    /// "Bills to `agent_id`" means `COALESCE(billed_agent_id, agent_id) = agent_id`: a call the agent made for itself (no `billed_agent_id`, or one pointing at itself) plus every call a worker it spawned made on its behalf.
    /// This is the query that gives a spawner the budget visibility it loses when its children each spend under their own id.
    ///
    /// Distinct from [`Self::query_summary`], which answers "what did this agent execute" and stays the right question for quota enforcement.
    pub fn query_billed_summary(&self, agent_id: AgentId) -> LibreFangResult<UsageSummary> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let summary = conn
            .query_row(
                "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                        COALESCE(SUM(cost_usd), 0.0), COUNT(*), COALESCE(SUM(tool_calls), 0)
                 FROM usage_events WHERE COALESCE(billed_agent_id, agent_id) = ?1",
                rusqlite::params![agent_id.0.to_string()],
                |row| {
                    Ok(UsageSummary {
                        total_input_tokens: row.get::<_, i64>(0)? as u64,
                        total_output_tokens: row.get::<_, i64>(1)? as u64,
                        total_cost_usd: row.get(2)?,
                        call_count: row.get::<_, i64>(3)? as u64,
                        total_tool_calls: row.get::<_, i64>(4)? as u64,
                    })
                },
            )
            .map_err(LibreFangError::memory)?;
        Ok(summary)
    }

    /// Query usage grouped by model.
    pub fn query_by_model(&self) -> LibreFangResult<Vec<ModelUsage>> {
        self.query_by_model_ranged(&DateRange::UNBOUNDED)
    }

    /// Query usage grouped by model, optionally restricted to a calendar-date range (#7891).
    ///
    /// Passing [`DateRange::UNBOUNDED`] is exactly equivalent to [`Self::query_by_model`].
    pub fn query_by_model_ranged(&self, range: &DateRange) -> LibreFangResult<Vec<ModelUsage>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        let (range_sql, range_binds) = range.sql_and_binds();
        let sql = format!(
            "SELECT model, COALESCE(SUM(cost_usd), 0.0), COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0), COUNT(*)
             FROM usage_events WHERE 1=1{range_sql}
             GROUP BY model ORDER BY SUM(cost_usd) DESC"
        );
        let mut stmt = conn.prepare(&sql).map_err(LibreFangError::memory)?;

        let rows = stmt
            .query_map(rusqlite::params_from_iter(range_binds.iter()), |row| {
                Ok(ModelUsage {
                    model: row.get(0)?,
                    total_cost_usd: row.get(1)?,
                    total_input_tokens: row.get::<_, i64>(2)? as u64,
                    total_output_tokens: row.get::<_, i64>(3)? as u64,
                    call_count: row.get::<_, i64>(4)? as u64,
                })
            })
            .map_err(LibreFangError::memory)?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(LibreFangError::memory)?);
        }
        Ok(results)
    }

    /// Query model performance metrics including latency statistics.
    pub fn query_model_performance(&self) -> LibreFangResult<Vec<ModelPerformance>> {
        self.query_model_performance_ranged(&DateRange::UNBOUNDED)
    }

    /// Query model performance metrics, optionally restricted to a calendar-date range (#7891).
    ///
    /// Passing [`DateRange::UNBOUNDED`] is exactly equivalent to [`Self::query_model_performance`].
    pub fn query_model_performance_ranged(
        &self,
        range: &DateRange,
    ) -> LibreFangResult<Vec<ModelPerformance>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        let (range_sql, range_binds) = range.sql_and_binds();
        let sql = format!(
            "SELECT model,
                    COALESCE(SUM(cost_usd), 0.0),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COUNT(*),
                    COALESCE(AVG(latency_ms), 0),
                    COALESCE(MIN(latency_ms), 0),
                    COALESCE(MAX(latency_ms), 0)
             FROM usage_events WHERE 1=1{range_sql}
             GROUP BY model
             ORDER BY SUM(cost_usd) DESC"
        );
        let mut stmt = conn.prepare(&sql).map_err(LibreFangError::memory)?;

        let rows = stmt
            .query_map(rusqlite::params_from_iter(range_binds.iter()), |row| {
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
            .map_err(LibreFangError::memory)?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(LibreFangError::memory)?);
        }
        Ok(results)
    }

    /// Query daily usage breakdown for the last N days.
    ///
    /// The window is relative to *now*, not to a calendar boundary: `days = 7`
    /// means "the last 168 hours". Use [`Self::query_daily_breakdown_ranged`]
    /// for a report pinned to calendar dates.
    pub fn query_daily_breakdown(&self, days: u32) -> LibreFangResult<Vec<DailyBreakdown>> {
        self.daily_breakdown_where(
            "datetime(timestamp) > datetime('now', ?)",
            &[format!("-{days} days")],
        )
    }

    /// Query daily usage breakdown across an inclusive calendar-date range (#7891).
    ///
    /// Unlike [`Self::query_daily_breakdown`] the bounds are calendar days, which is
    /// what a monthly or quarterly report is actually asking for — "March" is
    /// `2026-03-01`..=`2026-03-31`, not "the last 31 days".
    pub fn query_daily_breakdown_ranged(
        &self,
        range: &DateRange,
    ) -> LibreFangResult<Vec<DailyBreakdown>> {
        let (range_sql, binds) = range.sql_and_binds();
        self.daily_breakdown_where(&format!("1=1{range_sql}"), &binds)
    }

    /// Shared body of the two daily-breakdown queries: same projection and
    /// grouping, different `WHERE`.
    fn daily_breakdown_where(
        &self,
        where_sql: &str,
        binds: &[String],
    ) -> LibreFangResult<Vec<DailyBreakdown>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        let sql = format!(
            "SELECT date(timestamp) as day,
                    COALESCE(SUM(cost_usd), 0.0),
                    COALESCE(SUM(input_tokens) + SUM(output_tokens), 0),
                    COUNT(*)
             FROM usage_events
             WHERE {where_sql}
             GROUP BY day
             ORDER BY day ASC"
        );
        let mut stmt = conn.prepare(&sql).map_err(LibreFangError::memory)?;

        let rows = stmt
            .query_map(rusqlite::params_from_iter(binds.iter()), |row| {
                Ok(DailyBreakdown {
                    date: row.get(0)?,
                    cost_usd: row.get(1)?,
                    tokens: row.get::<_, i64>(2)? as u64,
                    calls: row.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(LibreFangError::memory)?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(LibreFangError::memory)?);
        }
        Ok(results)
    }

    /// Query the timestamp of the earliest usage event.
    pub fn query_first_event_date(&self) -> LibreFangResult<Option<String>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let result: Option<String> = conn
            .query_row("SELECT MIN(timestamp) FROM usage_events", [], |row| {
                row.get(0)
            })
            .map_err(LibreFangError::memory)?;
        Ok(result)
    }

    /// Query today's total cost across all agents.
    pub fn query_today_cost(&self) -> LibreFangResult<f64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE datetime(timestamp) > datetime('now', 'start of day')",
                [],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(cost)
    }

    /// Query today's cost for every agent in a single SQL pass.
    ///
    /// Returns a `Vec<(AgentId, f64)>` sorted by cost descending. Using a
    /// single `GROUP BY` query instead of N per-agent `SUM` queries eliminates
    /// the N+1 pattern in `/api/budget/agents`, which was responsible for up
    /// to 1200 queries/min under typical dashboard polling. See #3684.
    pub fn query_all_agents_daily(&self) -> LibreFangResult<Vec<(AgentId, f64)>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare(
                "SELECT agent_id, SUM(cost_usd) as total_cost
                 FROM usage_events
                 WHERE datetime(timestamp) > datetime('now', 'start of day')
                 GROUP BY agent_id
                 ORDER BY total_cost DESC",
            )
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map([], |row| {
                let id_str: String = row.get(0)?;
                let cost: f64 = row.get(1)?;
                Ok((id_str, cost))
            })
            .map_err(LibreFangError::memory)?;
        let mut results = Vec::new();
        for row in rows {
            let (id_str, cost) = row.map_err(LibreFangError::memory)?;
            let agent_id = id_str.parse::<AgentId>().map_err(|e| {
                LibreFangError::memory_msg(format!(
                    "invalid agent_id '{id_str}' in usage rollup: {e}"
                ))
            })?;
            results.push((agent_id, cost));
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
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare(
                "SELECT timestamp, model, provider, input_tokens, output_tokens,
                        cost_usd, tool_calls, latency_ms
                 FROM usage_events
                 WHERE agent_id = ?1
                 ORDER BY timestamp DESC
                 LIMIT ?2",
            )
            .map_err(LibreFangError::memory)?;
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
            .map_err(LibreFangError::memory)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(LibreFangError::memory)?);
        }
        Ok(out)
    }

    /// 24h LLM-call counts grouped by channel **type**, keyed by the `usage_events.channel` value.
    /// Single grouped SQL pass (uses idx_usage_channel_time).
    ///
    /// The key is a channel *type* (`telegram`, `slack`, `api`, `cron`, …), never a per-instance sidecar name, so every sidecar of the same type shares one bucket.
    /// That column is written from `UsageRecord.channel`, which the kernel derives from `SenderContext.channel`; the bridge builds that from `channel_type_str(&ChannelMessage.channel)` and `ChannelMessage` carries only a `ChannelType`, so the sidecar instance name never reaches this table.
    /// `SenderContext.channel` additionally feeds `SessionId::for_channel(agent, channel)` and the auth `identify(&channel, …)` binding, so it cannot be re-keyed to the instance name without re-deriving every existing channel session.
    /// Per-instance traffic is available instead from the supervisor's `ChannelStatus.messages_received` / `messages_sent` counters — see `crates/librefang-api/src/routes/channels.rs::sidecar_channel_rows`.
    /// Callers must label the returned figure as per-type; presenting it per-instance is the defect #6606 documents.
    pub fn channel_type_msgs_24h_bulk(
        &self,
    ) -> LibreFangResult<std::collections::HashMap<String, u64>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let cutoff = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
        let mut stmt = conn
            .prepare(
                "SELECT channel, COUNT(*)
                 FROM usage_events
                 WHERE channel IS NOT NULL AND channel != '' AND timestamp >= ?1
                 GROUP BY channel",
            )
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map(rusqlite::params![cutoff], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(LibreFangError::memory)?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let (ch, n) = row.map_err(LibreFangError::memory)?;
            out.insert(ch, n.max(0) as u64);
        }
        Ok(out)
    }

    /// Stream every usage event in `range`, oldest first, handing each row to
    /// `visit` as it comes off the SQLite cursor (#7891).
    ///
    /// This is deliberately callback-driven rather than `-> Vec<UsageExportRow>`.
    /// The export endpoint exists for archival, so the caller-chosen range can
    /// cover a full retention window of events; materializing that into a `Vec`
    /// (and then into a response body) would hold the entire table in memory
    /// twice. Handing rows out one at a time lets the HTTP layer encode and
    /// flush each chunk while the statement is still walking the index.
    ///
    /// `visit` returns [`std::ops::ControlFlow::Break`] to stop early — the export
    /// handler uses that when the client disconnects, so an abandoned download
    /// stops reading rather than draining the whole table into a dead socket.
    pub fn for_each_event_in_range<F>(&self, range: &DateRange, mut visit: F) -> LibreFangResult<()>
    where
        F: FnMut(UsageExportRow) -> std::ops::ControlFlow<()>,
    {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let (range_sql, binds) = range.sql_and_binds();
        let sql = format!(
            "SELECT timestamp, agent_id, billed_agent_id, provider, model,
                    input_tokens, output_tokens, cost_usd, tool_calls, latency_ms,
                    user_id, channel, session_id
             FROM usage_events
             WHERE 1=1{range_sql}
             ORDER BY timestamp ASC"
        );
        let mut stmt = conn.prepare(&sql).map_err(LibreFangError::memory)?;
        let mut rows = stmt
            .query(rusqlite::params_from_iter(binds.iter()))
            .map_err(LibreFangError::memory)?;

        while let Some(row) = rows.next().map_err(LibreFangError::memory)? {
            let parsed = UsageExportRow {
                timestamp: row.get(0).map_err(LibreFangError::memory)?,
                agent_id: row.get(1).map_err(LibreFangError::memory)?,
                billed_agent_id: row.get(2).map_err(LibreFangError::memory)?,
                provider: row.get(3).map_err(LibreFangError::memory)?,
                model: row.get(4).map_err(LibreFangError::memory)?,
                input_tokens: row.get::<_, i64>(5).map_err(LibreFangError::memory)?.max(0) as u64,
                output_tokens: row.get::<_, i64>(6).map_err(LibreFangError::memory)?.max(0) as u64,
                cost_usd: row.get(7).map_err(LibreFangError::memory)?,
                tool_calls: row.get::<_, i64>(8).map_err(LibreFangError::memory)?.max(0) as u64,
                latency_ms: row.get::<_, i64>(9).map_err(LibreFangError::memory)?.max(0) as u64,
                user_id: row.get(10).map_err(LibreFangError::memory)?,
                channel: row.get(11).map_err(LibreFangError::memory)?,
                session_id: row.get(12).map_err(LibreFangError::memory)?,
            };
            if visit(parsed).is_break() {
                break;
            }
        }
        Ok(())
    }

    /// Delete usage events older than the given number of days.
    pub fn cleanup_old(&self, days: u32) -> LibreFangResult<usize> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let modifier = format!("-{days} days");
        let deleted = conn
            .execute(
                "DELETE FROM usage_events WHERE datetime(timestamp) < datetime('now', ?1)",
                [modifier],
            )
            .map_err(LibreFangError::memory)?;
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn setup() -> UsageStore {
        let manager = r2d2_sqlite::SqliteConnectionManager::memory();
        let pool = r2d2::Pool::builder().max_size(1).build(manager).unwrap();
        run_migrations(&pool.get().unwrap()).unwrap();
        UsageStore::new(pool)
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
    fn spawned_worker_spend_rolls_up_to_the_parent_budget_line() {
        // #7714: a worker spawned by another agent spends on its spawner's
        // behalf. The spawner must be able to see that cost on its own budget
        // line, which is what `query_billed_summary` answers.
        let store = setup();
        let parent = AgentId::new();
        let worker = AgentId::new();

        // The parent's own turn: no `billed_agent_id`, so it bills to itself.
        store
            .record(&UsageRecord {
                agent_id: parent,
                cost_usd: 1.0,
                input_tokens: 10,
                ..Default::default()
            })
            .unwrap();
        // The worker's turn, billed to the parent.
        store
            .record(&UsageRecord {
                agent_id: worker,
                billed_agent_id: Some(parent),
                cost_usd: 0.25,
                input_tokens: 5,
                ..Default::default()
            })
            .unwrap();

        let billed = store.query_billed_summary(parent).unwrap();
        assert_eq!(
            billed.call_count, 2,
            "the parent's budget line must include the worker's call"
        );
        assert!(
            (billed.total_cost_usd - 1.25).abs() < 1e-9,
            "expected 1.25 rolled up, got {}",
            billed.total_cost_usd
        );
        assert_eq!(billed.total_input_tokens, 15);

        // The worker bills nothing to itself — its spend belongs to the parent.
        let worker_billed = store.query_billed_summary(worker).unwrap();
        assert_eq!(
            worker_billed.call_count, 0,
            "a worker whose spend rolls up must not also carry it itself"
        );

        // Enforcement is a separate dimension: `agent_id` is untouched, so the
        // quota subject each call is checked against is still the agent that
        // actually made it. This is what keeps the pre-call and post-call
        // quota checks asking about the same agent.
        assert_eq!(
            store.query_summary(Some(worker)).unwrap().call_count,
            1,
            "the executing agent must remain the quota subject for its own call"
        );
        assert_eq!(
            store.query_summary(Some(parent)).unwrap().call_count,
            1,
            "attribution must not retroactively move the child's call onto the parent's quota"
        );
    }

    #[test]
    fn usage_record_without_billed_agent_bills_to_itself() {
        // Every pre-#7714 call site leaves `billed_agent_id` unset. Those rows
        // must keep rolling up to `agent_id`, or the migration would silently
        // drop historical spend out of every budget view.
        let store = setup();
        let agent_id = AgentId::new();
        store
            .record(&UsageRecord {
                agent_id,
                cost_usd: 0.5,
                ..Default::default()
            })
            .unwrap();

        let billed = store.query_billed_summary(agent_id).unwrap();
        assert_eq!(billed.call_count, 1);
        assert!((billed.total_cost_usd - 0.5).abs() < 1e-9);
    }

    #[test]
    fn hourly_window_excludes_records_older_than_one_hour() {
        // Regression: `usage_events.timestamp` is RFC3339 TEXT (`T`-separated,
        // offset) while `datetime('now', ...)` yields a space-separated,
        // offset-less string. A bare `timestamp > datetime('now','-1 hour')`
        // compared the two lexicographically, so once the date prefix matched
        // the `T` (0x54) > space (0x20) mismatch at index 10 meant the
        // time-of-day was never reached — the hour window degraded to a
        // same-day check and counted records well outside the hour. Wrapping
        // the column in `datetime(timestamp)` parses both sides first.
        let store = setup();
        let agent_id = AgentId::new();
        let conn = store.pool.get().unwrap();
        let insert = |ts: String, cost: f64| {
            conn.execute(
                "INSERT INTO usage_events \
                 (id, agent_id, timestamp, model, provider, input_tokens, output_tokens, cost_usd, tool_calls, latency_ms) \
                 VALUES (?1, ?2, ?3, 'm', '', 0, 0, ?4, 0, 0)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    agent_id.0.to_string(),
                    ts,
                    cost
                ],
            )
            .unwrap();
        };
        // 30 minutes old → inside the 1-hour window.
        insert(
            (Utc::now() - chrono::Duration::minutes(30)).to_rfc3339(),
            0.01,
        );
        // 90 minutes old → outside it. Before the fix this row was wrongly
        // summed into the hourly total whenever it shared the calendar day.
        insert(
            (Utc::now() - chrono::Duration::minutes(90)).to_rfc3339(),
            100.0,
        );
        drop(conn);

        let hourly = store.query_hourly(agent_id).unwrap();
        assert!(
            (hourly - 0.01).abs() < 1e-9,
            "hourly window must exclude the 90-min-old record; got {hourly}"
        );
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
    fn parameterized_day_windows_filter_and_cleanup_old_events() {
        let store = setup();
        let agent_id = AgentId::new();
        let conn = store.pool.get().unwrap();
        for (id, timestamp, cost) in [
            ("recent", Utc::now().to_rfc3339(), 1.0),
            (
                "old",
                (Utc::now() - chrono::Duration::days(10)).to_rfc3339(),
                2.0,
            ),
        ] {
            conn.execute(
                "INSERT INTO usage_events \
                 (id, agent_id, timestamp, model, provider, input_tokens, output_tokens, cost_usd, tool_calls, latency_ms) \
                 VALUES (?1, ?2, ?3, 'model', 'provider', 1, 1, ?4, 0, 0)",
                rusqlite::params![id, agent_id.0.to_string(), timestamp, cost],
            )
            .unwrap();
        }
        drop(conn);

        let breakdown = store.query_daily_breakdown(7).unwrap();
        assert_eq!(breakdown.len(), 1);
        assert!((breakdown[0].cost_usd - 1.0).abs() < f64::EPSILON);

        assert_eq!(store.cleanup_old(7).unwrap(), 1);
        assert_eq!(store.query_summary(None).unwrap().call_count, 1);
    }

    // -- #7891: date-range filtering -------------------------------------

    /// Insert an event stamped at an explicit instant.
    fn insert_at(store: &UsageStore, agent_id: AgentId, timestamp: &str, cost: f64, model: &str) {
        let conn = store.pool.get().unwrap();
        conn.execute(
            "INSERT INTO usage_events \
             (id, agent_id, timestamp, model, provider, input_tokens, output_tokens, cost_usd, tool_calls, latency_ms) \
             VALUES (?1, ?2, ?3, ?4, 'prov', 10, 20, ?5, 1, 5)",
            rusqlite::params![
                uuid::Uuid::new_v4().to_string(),
                agent_id.0.to_string(),
                timestamp,
                model,
                cost
            ],
        )
        .unwrap();
    }

    fn seed_range_fixture(store: &UsageStore) -> AgentId {
        let agent = AgentId::new();
        insert_at(store, agent, "2026-01-15T10:00:00+00:00", 1.0, "gpt-a");
        insert_at(store, agent, "2026-01-31T23:59:59+00:00", 2.0, "gpt-a");
        insert_at(store, agent, "2026-02-01T00:00:00+00:00", 4.0, "claude-b");
        agent
    }

    #[test]
    fn date_range_parse_rejects_malformed_and_inverted_input() {
        assert!(matches!(
            DateRange::parse(Some("01-15-2026"), None),
            Err(DateRangeError::Malformed {
                field: "start_date",
                ..
            })
        ));
        assert!(matches!(
            DateRange::parse(None, Some("2026-13-01")),
            Err(DateRangeError::Malformed {
                field: "end_date",
                ..
            })
        ));
        assert!(matches!(
            DateRange::parse(Some("2026-03-01"), Some("2026-01-01")),
            Err(DateRangeError::Inverted { .. })
        ));
        // Equal bounds are a legitimate single-day report, not an inversion.
        assert!(DateRange::parse(Some("2026-01-01"), Some("2026-01-01")).is_ok());
    }

    #[test]
    fn date_range_treats_blank_bounds_as_absent() {
        let r = DateRange::parse(Some(""), Some("  ")).unwrap();
        assert!(r.is_unbounded());
    }

    #[test]
    fn unbounded_range_reproduces_the_unfiltered_queries() {
        let store = setup();
        seed_range_fixture(&store);

        assert_eq!(
            store.query_summary(None).unwrap().call_count,
            store
                .query_summary_ranged(None, &DateRange::UNBOUNDED)
                .unwrap()
                .call_count
        );
        assert_eq!(
            store.query_by_model().unwrap().len(),
            store
                .query_by_model_ranged(&DateRange::UNBOUNDED)
                .unwrap()
                .len()
        );
        assert_eq!(
            store.query_model_performance().unwrap().len(),
            store
                .query_model_performance_ranged(&DateRange::UNBOUNDED)
                .unwrap()
                .len()
        );
    }

    #[test]
    fn ranged_summary_filters_to_the_requested_calendar_month() {
        let store = setup();
        seed_range_fixture(&store);

        let jan = DateRange::parse(Some("2026-01-01"), Some("2026-01-31")).unwrap();
        let s = store.query_summary_ranged(None, &jan).unwrap();
        assert_eq!(s.call_count, 2);
        assert!((s.total_cost_usd - 3.0).abs() < f64::EPSILON);
    }

    /// The upper bound must cover the whole final day, not just its midnight
    /// instant — a `<=` comparison against the bare date would drop the
    /// 23:59:59 event and quietly understate a monthly total.
    #[test]
    fn ranged_end_bound_includes_the_entire_final_day() {
        let store = setup();
        seed_range_fixture(&store);

        let last_day = DateRange::parse(Some("2026-01-31"), Some("2026-01-31")).unwrap();
        let s = store.query_summary_ranged(None, &last_day).unwrap();
        assert_eq!(s.call_count, 1);
        assert!((s.total_cost_usd - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn ranged_daily_breakdown_uses_calendar_bounds() {
        let store = setup();
        seed_range_fixture(&store);

        let jan = DateRange::parse(Some("2026-01-01"), Some("2026-01-31")).unwrap();
        let days = store.query_daily_breakdown_ranged(&jan).unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].date, "2026-01-15");
        assert_eq!(days[1].date, "2026-01-31");
    }

    #[test]
    fn for_each_event_in_range_streams_in_ascending_order_and_can_stop_early() {
        let store = setup();
        seed_range_fixture(&store);

        let mut seen = Vec::new();
        store
            .for_each_event_in_range(&DateRange::UNBOUNDED, |row| {
                seen.push(row.timestamp.clone());
                std::ops::ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!(seen.len(), 3);
        assert!(
            seen[0] < seen[1] && seen[1] < seen[2],
            "ascending: {seen:?}"
        );

        // Break stops the cursor rather than draining the table.
        let mut count = 0;
        store
            .for_each_event_in_range(&DateRange::UNBOUNDED, |_| {
                count += 1;
                std::ops::ControlFlow::Break(())
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn for_each_event_in_range_honours_the_filter() {
        let store = setup();
        seed_range_fixture(&store);

        let feb = DateRange::parse(Some("2026-02-01"), Some("2026-02-28")).unwrap();
        let mut models = Vec::new();
        store
            .for_each_event_in_range(&feb, |row| {
                models.push(row.model.clone());
                std::ops::ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!(models, vec!["claude-b"]);
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
    fn exact_cost_limits_are_allowed_by_every_atomic_entry_point() {
        let record = |agent_id, provider: &str| UsageRecord {
            agent_id,
            provider: provider.to_string(),
            cost_usd: 1.0,
            input_tokens: 4,
            output_tokens: 6,
            ..Default::default()
        };

        let store = setup();
        store
            .check_quota_and_record(&record(AgentId::new(), ""), 1.0, 1.0, 1.0)
            .unwrap();

        let store = setup();
        store
            .check_global_budget_and_record(&record(AgentId::new(), ""), 1.0, 1.0, 1.0)
            .unwrap();

        let store = setup();
        store
            .check_all_and_record(&record(AgentId::new(), ""), 1.0, 1.0, 1.0, 1.0, 1.0, 1.0)
            .unwrap();

        let store = setup();
        store
            .check_all_with_provider_and_record(
                &record(AgentId::new(), "openai"),
                1.0,
                1.0,
                1.0,
                1.0,
                1.0,
                1.0,
                1.0,
                1.0,
                1.0,
                10,
            )
            .unwrap();
    }

    #[test]
    fn decimal_rounding_does_not_reject_an_exact_cost_limit() {
        let store = setup();
        let agent_id = AgentId::new();
        let first = UsageRecord {
            agent_id,
            cost_usd: 0.1,
            ..Default::default()
        };
        store.record(&first).unwrap();

        let second = UsageRecord {
            agent_id,
            cost_usd: 0.2,
            ..Default::default()
        };
        store
            .check_quota_and_record(&second, 0.3, 0.3, 0.3)
            .unwrap();

        assert_eq!(store.query_summary(None).unwrap().call_count, 2);
    }

    #[test]
    fn invalid_costs_and_limits_are_rejected() {
        let store = setup();
        for cost_usd in [-1.0, f64::NAN, f64::INFINITY] {
            let record = UsageRecord {
                agent_id: AgentId::new(),
                cost_usd,
                ..Default::default()
            };
            assert!(store.record(&record).is_err());
        }

        let record = UsageRecord {
            agent_id: AgentId::new(),
            cost_usd: 0.1,
            ..Default::default()
        };
        assert!(store
            .check_quota_and_record(&record, f64::NAN, 1.0, 1.0)
            .is_err());
        assert!(store
            .check_global_budget_and_record(&record, -1.0, 1.0, 1.0)
            .is_err());
        assert_eq!(store.query_summary(None).unwrap().call_count, 0);
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
            0.01,  // global hourly (exceeded: 0.008 + 0.005 > 0.01)
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

    #[test]
    fn test_user_ranking_surfaces_row_decode_errors() {
        let store = setup();
        let conn = store.pool.get().unwrap();
        conn.execute(
            "INSERT INTO usage_events (id, agent_id, timestamp, model, provider, input_tokens, output_tokens, cost_usd, tool_calls, latency_ms, user_id) \
             VALUES ('bad-ranking-row', 'agent', datetime('now'), 'model', 'provider', 0, 0, 1.0, 0, 0, X'80')",
            [],
        )
        .unwrap();
        drop(conn);

        assert!(
            store.query_user_ranking(Some(10)).is_err(),
            "malformed ranking rows must fail the query instead of disappearing"
        );
    }

    #[test]
    fn query_all_agents_daily_surfaces_invalid_agent_ids() {
        let store = setup();
        let conn = store.pool.get().unwrap();
        conn.execute(
            "INSERT INTO usage_events (id, agent_id, timestamp, model, provider, input_tokens, output_tokens, cost_usd, tool_calls, latency_ms) \
             VALUES ('bad-agent-row', 'not-an-agent-id', datetime('now'), 'model', 'provider', 0, 0, 1.0, 0, 0)",
            [],
        )
        .unwrap();
        drop(conn);

        assert!(
            store.query_all_agents_daily().is_err(),
            "invalid agent IDs must not disappear from budget rollups"
        );
    }
}
