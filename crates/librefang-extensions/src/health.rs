//! MCP server health monitor — tracks status with auto-reconnect.
//!
//! Background tokio task pings MCP connections, auto-reconnects with
//! exponential backoff (5s -> 10s -> 20s -> ... -> 5min max, 10 attempts max).
//!
//! # Two kinds of failure (#7963)
//!
//! Until #7963 the only code that could move a server into [`McpStatus::Error`] was the connect / reconnect path, so a server that completed its handshake and *later* wedged at the transport level was never reconnect-eligible: `/api/mcp/health` reported it healthy and the 60s health loop woke up and did nothing, indefinitely.
//! The runtime now reports transport-level tool-call failures here through [`HealthMonitor::report_transport_failure`], and a successful tool call through [`HealthMonitor::report_ok`], so a wedge discovered while the server is in use reaches the same auto-reconnect machinery a failed connect does.
//!
//! The two failure kinds are deliberately not equivalent, which is what [`McpFailureKind`] records:
//!
//! - A **connect** failure is proof the transport is dead — there is no connection to salvage, so it is reconnect-eligible on the first report.
//!   Gating it behind a threshold would be a regression: nothing else increments the counter for a server that never connected, so the health loop would never retry it at all.
//! - A **transport** failure observed mid-flight is only *evidence*.
//!   One tool-call timeout can just as easily be a genuinely slow tool, so it takes [`TRANSPORT_FAILURES_BEFORE_RECONNECT`] consecutive ones — with no intervening success — before the server is torn down and rebuilt.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use librefang_types::mcp::McpStatus;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

/// Consecutive transport-level failures on an already-connected MCP server before auto-reconnect is allowed to tear it down and rebuild it (#7963).
///
/// Three, because the two things this number trades off pull in opposite directions and three is the smallest value that satisfies both:
///
/// - **Not 1.** A single failure is indistinguishable from a slow tool that overran `timeout_secs`, and a reconnect drops the server's process — and with it any in-server session state — so paying that cost on one timeout is churn, not recovery.
/// - **Not 10.** Every failure is a real tool call an agent made and lost.
///   The wedge in #7963 burned 19 tool calls over ~2 days; a threshold that needs many failures reintroduces the same "broken for a long time" symptom in slower motion.
///
/// A genuinely wedged transport fails *every* call, so three consecutive failures arrive as fast as the agent retries and recovery is effectively immediate, while a transient blip is absorbed by the next success resetting the counter.
pub const TRANSPORT_FAILURES_BEFORE_RECONNECT: u32 = 3;

/// Where a failure report came from — the input to the reconnect-eligibility threshold (#7963).
/// See the module docs for why the two are not equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpFailureKind {
    /// The connect / reconnect handshake itself failed.
    /// Reconnect-eligible immediately: there is no live connection to preserve.
    Connect,
    /// An established connection failed at the transport level while in use — a tool-call timeout, a closed transport, a broken stdio pipe.
    /// Needs [`TRANSPORT_FAILURES_BEFORE_RECONNECT`] consecutive reports.
    Transport,
}

/// Health status for a single MCP server.
#[derive(Debug, Clone, Serialize)]
pub struct McpHealth {
    /// MCP server id (matches `McpServerConfigEntry.name`).
    pub id: String,
    /// Current status.
    pub status: McpStatus,
    /// Number of tools available from this MCP server.
    pub tool_count: usize,
    /// Last successful health check.
    pub last_ok: Option<DateTime<Utc>>,
    /// Last error message.
    pub last_error: Option<String>,
    /// Consecutive failures.
    pub consecutive_failures: u32,
    /// Whether auto-reconnect is in progress.
    pub reconnecting: bool,
    /// Reconnect attempt count.
    pub reconnect_attempts: u32,
    /// Uptime since last successful connect.
    pub connected_since: Option<DateTime<Utc>>,
    /// Which kind of failure produced the current `Error` status (#7963).
    ///
    /// Internal reconnect-eligibility state, not part of the `/api/mcp/health` payload — hence `#[serde(skip)]`, which also keeps the response schema (and its OpenAPI baseline) unchanged.
    #[serde(skip)]
    pub last_failure_kind: Option<McpFailureKind>,
}

impl McpHealth {
    /// Create a new health record.
    pub fn new(id: String) -> Self {
        Self {
            id,
            status: McpStatus::Available,
            tool_count: 0,
            last_ok: None,
            last_error: None,
            consecutive_failures: 0,
            reconnecting: false,
            reconnect_attempts: 0,
            connected_since: None,
            last_failure_kind: None,
        }
    }

    /// Mark as healthy.
    pub fn mark_ok(&mut self, tool_count: usize) {
        self.status = McpStatus::Ready;
        self.tool_count = tool_count;
        self.last_ok = Some(Utc::now());
        self.last_error = None;
        self.consecutive_failures = 0;
        self.reconnecting = false;
        self.reconnect_attempts = 0;
        self.last_failure_kind = None;
        if self.connected_since.is_none() {
            self.connected_since = Some(Utc::now());
        }
    }

    /// Mark as failed by the connect / reconnect path.
    ///
    /// Kept as the name every pre-#7963 call site already uses; it is [`mark_failure`](Self::mark_failure) with [`McpFailureKind::Connect`].
    pub fn mark_error(&mut self, error: String) {
        self.mark_failure(error, McpFailureKind::Connect);
    }

    /// Mark as failed, recording which kind of failure it was (#7963).
    pub fn mark_failure(&mut self, error: String, kind: McpFailureKind) {
        self.status = McpStatus::Error(error.clone());
        self.last_error = Some(error);
        self.consecutive_failures += 1;
        self.reconnecting = false;
        self.connected_since = None;
        self.last_failure_kind = Some(kind);
    }

    /// Consecutive failures this server must accumulate before auto-reconnect may act on its `Error` status (#7963).
    /// See the module docs.
    pub fn failures_required_for_reconnect(&self) -> u32 {
        match self.last_failure_kind {
            Some(McpFailureKind::Transport) => TRANSPORT_FAILURES_BEFORE_RECONNECT,
            // A connect failure, or an `Error` set before failure kinds existed, is a proven-dead transport: eligible on the first report, exactly as it was before #7963.
            Some(McpFailureKind::Connect) | None => 1,
        }
    }

    /// Mark as reconnecting.
    pub fn mark_reconnecting(&mut self) {
        self.reconnecting = true;
        self.reconnect_attempts += 1;
    }
}

/// Health monitor configuration.
#[derive(Debug, Clone)]
pub struct HealthMonitorConfig {
    /// Whether auto-reconnect is enabled.
    pub auto_reconnect: bool,
    /// Maximum reconnect attempts before giving up.
    pub max_reconnect_attempts: u32,
    /// Maximum backoff duration in seconds.
    pub max_backoff_secs: u64,
    /// Base check interval in seconds.
    pub check_interval_secs: u64,
}

impl Default for HealthMonitorConfig {
    fn default() -> Self {
        Self {
            auto_reconnect: true,
            max_reconnect_attempts: 10,
            max_backoff_secs: 300,
            check_interval_secs: 60,
        }
    }
}

/// The MCP health monitor — stores health state for all configured MCP servers.
pub struct HealthMonitor {
    /// Health records keyed by MCP server id.
    health: Arc<DashMap<String, McpHealth>>,
    /// Configuration.
    config: HealthMonitorConfig,
}

impl HealthMonitor {
    /// Create a new health monitor.
    pub fn new(config: HealthMonitorConfig) -> Self {
        Self {
            health: Arc::new(DashMap::new()),
            config,
        }
    }

    /// Register an MCP server for monitoring.
    pub fn register(&self, id: &str) {
        self.health
            .entry(id.to_string())
            .or_insert_with(|| McpHealth::new(id.to_string()));
    }

    /// Unregister an MCP server.
    pub fn unregister(&self, id: &str) {
        self.health.remove(id);
    }

    /// Report a successful health check.
    pub fn report_ok(&self, id: &str, tool_count: usize) {
        if let Some(mut entry) = self.health.get_mut(id) {
            entry.mark_ok(tool_count);
        }
    }

    /// Report a health check failure from the connect / reconnect path.
    pub fn report_error(&self, id: &str, error: String) {
        if let Some(mut entry) = self.health.get_mut(id) {
            entry.mark_error(error);
        }
    }

    /// Report a transport-level failure observed on an already-connected server — a tool-call timeout, a closed transport, a dead stdio pipe (#7963).
    ///
    /// This is the report the runtime's tool-call dispatch path makes, and it is the only thing that lets auto-reconnect notice a server that wedged *after* a successful handshake.
    /// Application errors (bad arguments, file not found — anything the server answered with a well-formed JSON-RPC error) must never come through here: the transport is working and reconnecting it would drop a healthy server.
    ///
    /// Returns whether the server is now reconnect-eligible, which callers use for log severity; the health loop re-checks [`should_reconnect`](Self::should_reconnect) itself.
    pub fn report_transport_failure(&self, id: &str, error: String) -> bool {
        if let Some(mut entry) = self.health.get_mut(id) {
            entry.mark_failure(error, McpFailureKind::Transport);
            entry.consecutive_failures >= entry.failures_required_for_reconnect()
        } else {
            false
        }
    }

    /// Get health for a specific MCP server.
    pub fn get_health(&self, id: &str) -> Option<McpHealth> {
        self.health.get(id).map(|e| e.clone())
    }

    /// Get health for all MCP servers.
    pub fn all_health(&self) -> Vec<McpHealth> {
        self.health.iter().map(|e| e.value().clone()).collect()
    }

    /// Calculate exponential backoff duration for a given attempt.
    pub fn backoff_duration(&self, attempt: u32) -> Duration {
        let base_secs = 5u64;
        let backoff = base_secs.saturating_mul(1u64 << attempt.min(10));
        Duration::from_secs(backoff.min(self.config.max_backoff_secs))
    }

    /// Check if an MCP server should be reconnected.
    ///
    /// Gated on `consecutive_failures` as well as the `Error` status (#7963):
    /// a server that wedged mid-flight must fail
    /// [`TRANSPORT_FAILURES_BEFORE_RECONNECT`] consecutive tool calls before it
    /// is torn down, so a single transient timeout cannot cause reconnect
    /// churn. A failed connect stays eligible on its first report — see
    /// [`McpHealth::failures_required_for_reconnect`].
    pub fn should_reconnect(&self, id: &str) -> bool {
        if !self.config.auto_reconnect {
            return false;
        }
        if let Some(entry) = self.health.get(id) {
            matches!(entry.status, McpStatus::Error(_))
                && entry.consecutive_failures >= entry.failures_required_for_reconnect()
                && entry.reconnect_attempts < self.config.max_reconnect_attempts
        } else {
            false
        }
    }

    /// Mark an MCP server as reconnecting.
    pub fn mark_reconnecting(&self, id: &str) {
        if let Some(mut entry) = self.health.get_mut(id) {
            entry.mark_reconnecting();
        }
    }

    /// Get a reference to the health DashMap (for background task).
    pub fn health_map(&self) -> Arc<DashMap<String, McpHealth>> {
        self.health.clone()
    }

    /// Get the config.
    pub fn config(&self) -> &HealthMonitorConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_monitor_register_report() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        monitor.register("github");

        let h = monitor.get_health("github").unwrap();
        assert_eq!(h.status, McpStatus::Available);
        assert_eq!(h.tool_count, 0);

        monitor.report_ok("github", 12);
        let h = monitor.get_health("github").unwrap();
        assert_eq!(h.status, McpStatus::Ready);
        assert_eq!(h.tool_count, 12);
        assert!(h.last_ok.is_some());
        assert!(h.connected_since.is_some());
    }

    #[test]
    fn health_monitor_error_tracking() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        monitor.register("slack");

        monitor.report_error("slack", "Connection refused".to_string());
        let h = monitor.get_health("slack").unwrap();
        assert!(matches!(h.status, McpStatus::Error(_)));
        assert_eq!(h.consecutive_failures, 1);

        monitor.report_error("slack", "Timeout".to_string());
        let h = monitor.get_health("slack").unwrap();
        assert_eq!(h.consecutive_failures, 2);

        // Recovery
        monitor.report_ok("slack", 5);
        let h = monitor.get_health("slack").unwrap();
        assert_eq!(h.consecutive_failures, 0);
        assert_eq!(h.status, McpStatus::Ready);
    }

    #[test]
    fn backoff_exponential() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        assert_eq!(monitor.backoff_duration(0), Duration::from_secs(5));
        assert_eq!(monitor.backoff_duration(1), Duration::from_secs(10));
        assert_eq!(monitor.backoff_duration(2), Duration::from_secs(20));
        assert_eq!(monitor.backoff_duration(3), Duration::from_secs(40));
        // Capped at 300s
        assert_eq!(monitor.backoff_duration(10), Duration::from_secs(300));
        assert_eq!(monitor.backoff_duration(20), Duration::from_secs(300));
    }

    #[test]
    fn should_reconnect_logic() {
        let monitor = HealthMonitor::new(HealthMonitorConfig {
            auto_reconnect: true,
            max_reconnect_attempts: 3,
            ..Default::default()
        });
        monitor.register("test");

        // Available — no reconnect needed
        assert!(!monitor.should_reconnect("test"));

        // Error — should reconnect
        monitor.report_error("test", "fail".to_string());
        assert!(monitor.should_reconnect("test"));

        // Exhaust attempts
        for _ in 0..3 {
            monitor.mark_reconnecting("test");
        }
        assert!(!monitor.should_reconnect("test"));
    }

    #[test]
    fn reconnect_failure_clears_in_progress_state_without_resetting_attempts() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        monitor.register("test");
        monitor.report_error("test", "initial failure".to_string());
        monitor.mark_reconnecting("test");

        monitor.report_error("test", "reconnect failed".to_string());

        let health = monitor.get_health("test").unwrap();
        assert!(!health.reconnecting);
        assert_eq!(health.reconnect_attempts, 1);
        assert_eq!(health.consecutive_failures, 2);
        assert!(matches!(health.status, McpStatus::Error(_)));
    }

    #[test]
    fn health_unregister() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        monitor.register("github");
        assert!(monitor.get_health("github").is_some());
        monitor.unregister("github");
        assert!(monitor.get_health("github").is_none());
    }

    #[test]
    fn all_health() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        monitor.register("a");
        monitor.register("b");
        monitor.register("c");
        let all = monitor.all_health();
        assert_eq!(all.len(), 3);
    }

    // ----------------------------------------------------------------------- #7963 — transport failures reported from the tool-call path -----------------------------------------------------------------------

    #[test]
    fn transport_failures_become_reconnect_eligible_at_the_threshold() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        monitor.register("wedged");
        // The server handshook successfully, so it starts out Ready — the pre-#7963 state in which nothing could ever flip it to Error.
        monitor.report_ok("wedged", 7);
        assert!(!monitor.should_reconnect("wedged"));

        for attempt in 1..TRANSPORT_FAILURES_BEFORE_RECONNECT {
            let eligible =
                monitor.report_transport_failure("wedged", "tool call timed out".to_string());
            assert!(
                !eligible,
                "failure {attempt} of {TRANSPORT_FAILURES_BEFORE_RECONNECT} must not be eligible yet"
            );
            assert!(
                !monitor.should_reconnect("wedged"),
                "one transient transport failure must not trigger a reconnect"
            );
        }

        let eligible = monitor.report_transport_failure("wedged", "Transport closed".to_string());
        assert!(eligible, "the threshold report must report eligibility");
        assert!(monitor.should_reconnect("wedged"));

        let health = monitor.get_health("wedged").unwrap();
        assert!(matches!(health.status, McpStatus::Error(_)));
        assert_eq!(
            health.consecutive_failures,
            TRANSPORT_FAILURES_BEFORE_RECONNECT
        );
        assert_eq!(health.last_failure_kind, Some(McpFailureKind::Transport));
    }

    #[test]
    fn single_transport_failure_is_not_reconnect_eligible() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        monitor.register("blip");
        monitor.report_ok("blip", 3);

        assert!(!monitor.report_transport_failure("blip", "timed out".to_string()));
        assert!(
            !monitor.should_reconnect("blip"),
            "a single transport failure is indistinguishable from a slow tool"
        );
    }

    #[test]
    fn a_success_between_transport_failures_resets_the_threshold() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        monitor.register("flaky");
        monitor.report_ok("flaky", 2);

        // Fail up to one short of the threshold, then succeed, then fail again: the count must restart, so the server stays connected.
        for _ in 1..TRANSPORT_FAILURES_BEFORE_RECONNECT {
            monitor.report_transport_failure("flaky", "timed out".to_string());
        }
        monitor.report_ok("flaky", 2);
        let health = monitor.get_health("flaky").unwrap();
        assert_eq!(health.consecutive_failures, 0);
        assert_eq!(health.last_failure_kind, None);
        assert_eq!(health.status, McpStatus::Ready);

        monitor.report_transport_failure("flaky", "timed out".to_string());
        assert!(!monitor.should_reconnect("flaky"));
    }

    #[test]
    fn connect_failure_is_still_eligible_on_the_first_report() {
        // Regression guard for the threshold: nothing else increments the counter for a server that never connected, so gating a connect failure behind the transport threshold would mean the health loop never retried it at all.
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        monitor.register("never-connected");

        monitor.report_error("never-connected", "spawn failed".to_string());

        let health = monitor.get_health("never-connected").unwrap();
        assert_eq!(health.consecutive_failures, 1);
        assert_eq!(health.last_failure_kind, Some(McpFailureKind::Connect));
        assert!(monitor.should_reconnect("never-connected"));
    }

    #[test]
    fn a_failed_reconnect_after_a_transport_wedge_stays_eligible() {
        // Once the transport threshold hands the server to the reconnect path, subsequent connect failures must keep it eligible (up to max_reconnect_attempts) rather than re-arming the threshold.
        let monitor = HealthMonitor::new(HealthMonitorConfig {
            auto_reconnect: true,
            max_reconnect_attempts: 3,
            ..Default::default()
        });
        monitor.register("wedged");
        monitor.report_ok("wedged", 1);
        for _ in 0..TRANSPORT_FAILURES_BEFORE_RECONNECT {
            monitor.report_transport_failure("wedged", "Transport closed".to_string());
        }
        assert!(monitor.should_reconnect("wedged"));

        monitor.mark_reconnecting("wedged");
        monitor.report_error("wedged", "reconnect failed".to_string());
        assert!(monitor.should_reconnect("wedged"));

        // …and the attempt cap still terminates the loop.
        monitor.mark_reconnecting("wedged");
        monitor.mark_reconnecting("wedged");
        assert!(!monitor.should_reconnect("wedged"));
    }

    #[test]
    fn transport_failure_on_an_unregistered_server_is_a_noop() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        assert!(!monitor.report_transport_failure("ghost", "timed out".to_string()));
        assert!(monitor.get_health("ghost").is_none());
    }

    #[test]
    fn auto_reconnect_disabled() {
        let monitor = HealthMonitor::new(HealthMonitorConfig {
            auto_reconnect: false,
            ..Default::default()
        });
        monitor.register("test");
        monitor.report_error("test", "fail".to_string());
        assert!(!monitor.should_reconnect("test"));
    }
}
