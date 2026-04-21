//! Agent runtime and execution environment.
//!
//! Manages the agent execution loop, LLM driver abstraction,
//! tool execution, and WASM sandboxing for untrusted skill/plugin code.

/// Default User-Agent header sent with all outgoing HTTP requests.
/// Some LLM providers (e.g. Moonshot, Qwen) reject requests without one.
pub const USER_AGENT: &str = concat!("librefang/", env!("CARGO_PKG_VERSION"));

pub mod a2a;
pub mod agent_loop;
pub mod apply_patch;
pub mod audit;
pub mod auth_cooldown;
pub mod browser;
pub mod catalog_sync;
pub mod channel_registry;
pub mod checkpoint_manager;
pub use librefang_runtime_oauth::chatgpt_oauth;
pub mod command_lane;
pub mod compactor;
pub mod context_budget;
pub mod context_engine;
pub mod context_overflow;
pub use librefang_runtime_oauth::copilot_oauth;
pub mod docker_sandbox;
pub use librefang_llm_drivers::drivers;
pub mod embedding;
pub mod graceful_shutdown;
pub mod hooks;
pub use librefang_http as http_client;
pub use librefang_runtime_wasm::host_functions;
pub mod image_gen;
pub use librefang_kernel_handle as kernel_handle;
pub mod link_understanding;
pub use librefang_llm_driver as llm_driver;
pub use librefang_llm_driver::llm_errors;
pub mod loop_guard;
pub use librefang_runtime_mcp as mcp;
pub mod mcp_migrate;
pub use librefang_runtime_mcp::mcp_oauth;
pub mod mcp_server;
pub mod media;
pub mod media_understanding;
pub mod model_catalog;
pub mod pii_filter;
pub mod plugin_manager;
pub mod plugin_runtime;
pub mod proactive_memory;
pub mod process_manager;
pub mod prompt_builder;
pub mod provider_health;
pub mod python_runtime;
pub mod registry_sync;
pub mod reply_directives;
pub mod retry;
pub mod routing;
pub use librefang_runtime_wasm::sandbox;
pub mod session_repair;
pub mod silent_response;
pub use silent_response::{is_silent_response, SilentReason};
pub mod shell_bleed;
pub mod str_utils;
pub mod subprocess_sandbox;
pub mod tool_policy;
pub mod tool_runner;
pub mod trace_store;
pub mod tts;
pub mod web_cache;
pub mod web_content;
pub mod web_fetch;
pub mod web_search;
pub mod workspace_context;
pub mod workspace_sandbox;
