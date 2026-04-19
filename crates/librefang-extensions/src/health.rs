//! MCP server health monitor — tracks status with auto-reconnect.
//!
//! Background tokio task pings MCP connections, auto-reconnects with
//! exponential backoff (5s -> 10s -> 20s -> ... -> 5min max, 10 attempts max).

use crate::McpStatus;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

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
        if self.connected_since.is_none() {
            self.connected_since = Some(Utc::now());
        }
    }

    /// Mark as failed.
    pub fn mark_error(&mut self, error: String) {
        self.status = McpStatus::Error(error.clone());
        self.last_error = Some(error);
        self.consecutive_failures += 1;
        self.connected_since = None;
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

    /// Report a health check failure.
    pub fn report_error(&self, id: &str, error: String) {
        if let Some(mut entry) = self.health.get_mut(id) {
            entry.mark_error(error);
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
    pub fn should_reconnect(&self, id: &str) -> bool {
        if !self.config.auto_reconnect {
            return false;
        }
        if let Some(entry) = self.health.get(id) {
            matches!(entry.status, McpStatus::Error(_))
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
