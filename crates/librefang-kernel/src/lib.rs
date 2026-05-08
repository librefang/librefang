// `KernelApi`'s `async_trait`-expanded methods nest pinned futures deeply
// enough that the default 128-step layout pass tips over (#3566).
#![recursion_limit = "256"]

//! Core kernel for the LibreFang Agent Operating System.
//!
//! The kernel manages agent lifecycles, memory, permissions, scheduling,
//! and inter-agent communication.

pub mod agent_identity_registry;
pub mod approval;
pub mod auth;
pub mod auto_dream;
pub mod auto_reply;
pub mod backends;
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
pub mod kernel_api;
pub mod log_reload;
pub mod mcp_oauth_provider;
pub mod storage_backends;
pub use librefang_kernel_metering as metering;
pub mod orchestration;
pub mod pairing;
pub mod registry;
pub use librefang_kernel_router as router;
pub mod scheduler;
pub mod session_lifecycle;
pub mod session_policy;
pub mod session_stream_hub;
pub mod skill_workshop;
pub mod supervised_spawn;
pub mod supervisor;
pub mod trajectory;
pub mod triggers;
pub mod whatsapp_gateway;
pub mod wizard;
pub mod workflow;

pub use kernel::DeliveryTracker;
pub use kernel::LibreFangKernel;
pub use kernel::{SYSTEM_CHANNEL_AUTONOMOUS, SYSTEM_CHANNEL_CRON, SYSTEM_CHANNEL_WEBUI};
pub use kernel_api::KernelApi;

// Focused per-subsystem traits (refs #3565). Re-exported so external
// crates can bind `&dyn FooSubsystemApi` instead of dragging in the
// entire `KernelApi` surface, and so the upcoming method-body
// migration can move callers off `LibreFangKernel` inherent forwards.
pub use kernel::subsystems::{
    AgentSubsystemApi, EventSubsystemApi, GovernanceSubsystemApi, LlmSubsystemApi, McpSubsystemApi,
    MediaSubsystemApi, MemorySubsystemApi, MeshSubsystemApi, MeteringSubsystemApi,
    ProcessSubsystemApi, SecuritySubsystemApi, SkillsSubsystemApi, WorkflowSubsystemApi,
};

// ---------------------------------------------------------------------------
// Runtime re-exports (refs #3596 — API → Kernel → Runtime layering)
// ---------------------------------------------------------------------------
//
// `librefang-api` historically imported runtime types directly, which bypasses
// kernel encapsulation. The intended layering is `API → Kernel → Runtime`;
// API code should reach runtime types through the kernel boundary.
//
// These re-exports mirror the public modules of `librefang-runtime` that the
// API layer needs as read-only types or trait surfaces. They do not introduce
// new dependencies — `librefang-kernel` already depends on `librefang-runtime`.
//
// Migration is incremental (PR 1/N): adding the re-exports here unblocks API
// callers from switching `use librefang_runtime::foo` to
// `use librefang_kernel::foo` one file at a time, without breaking files that
// have not yet been migrated. A follow-up PR will delete
// `librefang-api/Cargo.toml`'s direct `librefang-runtime` dependency once the
// last in-tree `use librefang_runtime::*` is gone, letting the compiler
// enforce the boundary.
pub use librefang_runtime::a2a;
pub use librefang_runtime::agent_loop;
pub use librefang_runtime::audit;
pub use librefang_runtime::browser;
pub use librefang_runtime::catalog_sync;
pub use librefang_runtime::channel_registry;
pub use librefang_runtime::compactor;
pub use librefang_runtime::copilot_oauth;
pub use librefang_runtime::drivers;
pub use librefang_runtime::http_client;
pub use librefang_runtime::kernel_handle;
pub use librefang_runtime::llm_driver;
pub use librefang_runtime::llm_errors;
pub use librefang_runtime::mcp;
pub use librefang_runtime::mcp_oauth;
pub use librefang_runtime::mcp_server;
pub use librefang_runtime::media;
pub use librefang_runtime::model_catalog;
pub use librefang_runtime::pdf_text;
pub use librefang_runtime::plugin_manager;
pub use librefang_runtime::plugin_runtime;
pub use librefang_runtime::provider_health;
pub use librefang_runtime::registry_sync;
pub use librefang_runtime::silent_response;
pub use librefang_runtime::str_utils;
pub use librefang_runtime::tool_runner;

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
