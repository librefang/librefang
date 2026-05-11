//! Per-trait `kernel_handle::*` impls for [`LibreFangKernel`].
//!
//! Each sub-module hosts exactly one `impl kernel_handle::SomeTrait for
//! LibreFangKernel`. Splitting these out of `kernel::mod` keeps the
//! ~14 800-line god-impl from doubling as a trait-impl dumping ground —
//! and lets each role surface (agent control, memory access, …) be
//! audited and edited in isolation.
//!
//! The submodules are descendants of `kernel`, so they retain access to
//! `LibreFangKernel`'s private fields and inherent methods without any
//! visibility surgery (Rust scopes private items to the module of
//! declaration *and its descendants*).

mod a2a_registry;
mod acp_fs_bridge;
mod acp_terminal_bridge;
mod agent_control;
mod api_auth;
mod approval_gate;
mod catalog_query;
mod channel_sender;
mod cron_control;
mod event_bus;
mod goal_control;
mod hands_control;
mod knowledge_graph;
mod memory_access;
mod prompt_store;
mod session_writer;
mod task_queue;
mod tool_policy;
mod wiki_access;
mod workflow_runner;
