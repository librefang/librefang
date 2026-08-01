//! MoA progress events for UI surfacing.
//!
//! Emitted by [`super::driver::MoaDriver`] on a cache-MISS turn into a
//! kernel-owned broadcast channel. The agent loop subscribes and relays these
//! as `LoopPhase` updates so the dashboard / TUI can show advisor progress.

use serde::{Deserialize, Serialize};

/// A progress event emitted during a MoA turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MoaProgressEvent {
    /// Fan-out has started.
    FanoutStart {
        /// Total number of advisors dispatched.
        total: usize,
    },
    /// One advisor has completed.
    Progress {
        /// Advisors completed so far.
        done: usize,
        /// Total advisors.
        total: usize,
        /// Label of the advisor that just finished.
        label: String,
    },
    /// One advisor's reference output is ready.
    Reference {
        /// Zero-based index of the advisor.
        index: usize,
        /// Total number of advisors.
        count: usize,
        /// Label of the advisor.
        label: String,
    },
    /// All advisors done; the aggregator is now running.
    Aggregating {
        /// Aggregator label.
        aggregator: String,
        /// Number of reference advisors that produced output.
        ref_count: usize,
    },
}
