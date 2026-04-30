//! Core kernel for the LibreFang Agent Operating System.
//!
//! The kernel manages agent lifecycles, memory, permissions, scheduling,
//! and inter-agent communication.

pub mod approval;
pub mod auth;
pub mod auto_dream;
pub mod auto_reply;
pub mod background;
pub mod capabilities;
pub mod config;
pub mod config_reload;
pub mod cron;
pub mod cron_delivery;
pub mod error;
pub mod event_bus;
pub mod heartbeat;
pub mod hooks;
pub mod inbox;
pub mod kernel;
pub mod log_reload;
pub mod mcp_oauth_provider;
pub use librefang_kernel_metering as metering;
pub mod orchestration;
pub mod pairing;
pub mod registry;
pub use librefang_kernel_router as router;
pub mod scheduler;
pub mod session_lifecycle;
pub mod session_policy;
pub mod session_stream_hub;
pub mod supervisor;
pub mod trajectory;
pub mod triggers;
pub mod whatsapp_gateway;
pub mod wizard;
pub mod workflow;

pub use kernel::DeliveryTracker;
pub use kernel::LibreFangKernel;

// ---------------------------------------------------------------------------
// Shared persist utility
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Global counter so concurrent persist calls never share a staging path.
static PERSIST_SEQ: AtomicU64 = AtomicU64::new(0);

/// Build a unique `.json.tmp.<pid>.<seq>.<nanos>` staging path for atomic
/// file writes (#3648). Two daemons sharing the same `home_dir`, or two
/// threads within one process, each get a distinct path.
pub(crate) fn persist_tmp_path(final_path: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    final_path.with_extension(format!(
        "json.tmp.{}.{}.{}",
        std::process::id(),
        PERSIST_SEQ.fetch_add(1, Ordering::Relaxed),
        nanos,
    ))
}
