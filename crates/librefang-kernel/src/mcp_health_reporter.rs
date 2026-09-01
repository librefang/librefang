//! Kernel implementation of the runtime's MCP transport-health seam (#7963).
//!
//! # The seam
//!
//! The agent runtime is where a wedged MCP transport is actually observed — a tool call times out, or comes back with a closed transport — but the health monitor that owns reconnect lives in `librefang-extensions`, which the runtime must not depend on (`librefang-runtime` -> `librefang-extensions` would be a dependency inversion, and the runtime is deliberately below the extension layer).
//!
//! So the trait is declared in the runtime ([`McpTransportHealthReporter`](librefang_runtime::mcp::McpTransportHealthReporter)) and implemented here, in the kernel, which already depends on both — the same trait-injection pattern `McpOAuthProvider` uses.
//! Every `McpConnection` the kernel builds is handed one of these via `McpConnection::with_health_reporter`, so the runtime's dispatch path can report an outcome without naming the health monitor's type.
//!
//! # Why it holds `Arc<HealthMonitor>` and not the kernel
//!
//! The reporter is stored inside a connection which is itself stored on the kernel, so holding a strong `Arc<LibreFangKernel>` here would be a reference cycle that leaks the whole kernel.
//! `HealthMonitor` points back at nothing, so sharing it directly is both cycle-free and narrower: the reporter cannot reach anything but health state.

use std::sync::Arc;

use librefang_extensions::health::HealthMonitor;
use librefang_runtime::mcp::McpTransportHealthReporter;

/// Routes the runtime's tool-call health reports into the kernel's [`HealthMonitor`].
pub struct KernelMcpHealthReporter {
    health: Arc<HealthMonitor>,
}

impl KernelMcpHealthReporter {
    /// Wrap a shared health monitor.
    pub fn new(health: Arc<HealthMonitor>) -> Self {
        Self { health }
    }

    /// Build the shared trait object the runtime's `McpConnection` stores.
    pub fn shared(health: Arc<HealthMonitor>) -> Arc<dyn McpTransportHealthReporter> {
        Arc::new(Self::new(health))
    }
}

impl McpTransportHealthReporter for KernelMcpHealthReporter {
    fn report_transport_failure(&self, server: &str, error: &str) {
        // `report_transport_failure` returns whether the server crossed the consecutive-failure threshold and is now reconnect-eligible.
        // The health loop re-checks that itself, so the return value is used only to pick a log level: an operator wants to see the moment a server is handed to auto-reconnect, without a WARN for every absorbed blip.
        let eligible = self
            .health
            .report_transport_failure(server, error.to_string());
        if eligible {
            tracing::warn!(
                server,
                error,
                "MCP transport failure threshold reached — server marked errored and queued for auto-reconnect"
            );
        } else {
            tracing::debug!(
                server,
                error,
                "MCP transport failure recorded (below the reconnect threshold)"
            );
        }
    }

    fn report_call_ok(&self, server: &str, tool_count: usize) {
        self.health.report_ok(server, tool_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_extensions::health::{HealthMonitorConfig, TRANSPORT_FAILURES_BEFORE_RECONNECT};
    use librefang_types::mcp::McpStatus;

    fn monitor() -> Arc<HealthMonitor> {
        let m = Arc::new(HealthMonitor::new(HealthMonitorConfig::default()));
        m.register("srv");
        m.report_ok("srv", 5);
        m
    }

    #[test]
    fn transport_failures_reach_the_health_monitor_and_arm_reconnect() {
        let health = monitor();
        let reporter = KernelMcpHealthReporter::shared(Arc::clone(&health));

        for _ in 0..(TRANSPORT_FAILURES_BEFORE_RECONNECT - 1) {
            reporter.report_transport_failure("srv", "MCP tool call timed out after 60s");
            assert!(
                !health.should_reconnect("srv"),
                "below the threshold the server must not be torn down"
            );
        }

        reporter.report_transport_failure("srv", "Transport closed");
        assert!(health.should_reconnect("srv"));
        let entry = health.get_health("srv").unwrap();
        assert!(matches!(entry.status, McpStatus::Error(_)));
        assert_eq!(
            entry.consecutive_failures,
            TRANSPORT_FAILURES_BEFORE_RECONNECT
        );
    }

    #[test]
    fn a_successful_call_resets_the_failure_run() {
        let health = monitor();
        let reporter = KernelMcpHealthReporter::shared(Arc::clone(&health));

        reporter.report_transport_failure("srv", "timed out");
        reporter.report_call_ok("srv", 5);

        let entry = health.get_health("srv").unwrap();
        assert_eq!(entry.consecutive_failures, 0);
        assert_eq!(entry.status, McpStatus::Ready);
        assert!(entry.last_ok.is_some());
        assert!(!health.should_reconnect("srv"));
    }
}
