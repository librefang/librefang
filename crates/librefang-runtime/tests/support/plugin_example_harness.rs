//! Shared test harness for Phase-6 plugin example integration tests.
//!
//! Provides:
//! - `KernelHandleStub` — minimal `KernelHandle` implementation with an
//!   in-memory KV store for the `MemoryAccess` role trait. All other role
//!   traits use the defaults from `librefang-kernel-handle` (most return
//!   `KernelOpError::unavailable`).
//! - `wasm_bytes(dir)` — reads `examples/plugins/<dir>/pre-built/plugin.wasm`
//!   relative to the workspace root, panicking cleanly on missing files so
//!   test failure messages point at the pre-built slot.

use async_trait::async_trait;
use librefang_kernel_handle::{
    A2ARegistry, AcpFsBridge, AcpTerminalBridge, AgentControl, AgentInfo, ApiAuth, ApiAuthSnapshot,
    ApprovalGate, CatalogQuery, ChannelSender, CronControl, EventBus, GoalControl, HandsControl,
    KernelOpError, KnowledgeGraph, MemoryAccess, PromptStore, SessionWriter, TaskQueue, ToolPolicy,
    WikiAccess, WorkflowRunner,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

// ---------------------------------------------------------------------------
// KernelHandleStub
// ---------------------------------------------------------------------------

/// Minimal KernelHandle stub for plugin example integration tests.
///
/// Only `MemoryAccess` is overridden with a real in-memory store — needed
/// by the `js-kv-counter` example. All other role traits delegate to the
/// trait defaults (unavailable / empty / no-op), which is sufficient for
/// `c-noop`, `rust-fs-cat`, `python-hello-time`, and `go-env-greet` since
/// those plugins either make no host calls to the kernel or go through
/// `std::fs` / `std::env` directly.
#[derive(Default)]
pub struct KernelHandleStub {
    kv: Mutex<HashMap<String, serde_json::Value>>,
}

impl KernelHandleStub {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Seed the in-memory KV store before the plugin runs.
    pub fn seed(&self, key: &str, value: serde_json::Value) {
        self.kv.lock().unwrap().insert(key.to_owned(), value);
    }

    /// Inspect the KV store after the plugin ran.
    pub fn kv_get(&self, key: &str) -> Option<serde_json::Value> {
        self.kv.lock().unwrap().get(key).cloned()
    }
}

// ── AgentControl ────────────────────────────────────────────────────────────

#[async_trait]
impl AgentControl for KernelHandleStub {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), KernelOpError> {
        Err(KernelOpError::unavailable("stub"))
    }
    async fn send_to_agent(
        &self,
        _agent_id: &str,
        _message: &str,
    ) -> Result<String, KernelOpError> {
        Err(KernelOpError::unavailable("stub"))
    }
    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }
    fn kill_agent(&self, _agent_id: &str) -> Result<(), KernelOpError> {
        Err(KernelOpError::unavailable("stub"))
    }
    fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

// ── MemoryAccess ─────────────────────────────────────────────────────────────

impl MemoryAccess for KernelHandleStub {
    fn memory_store(
        &self,
        key: &str,
        value: serde_json::Value,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<(), KernelOpError> {
        self.kv.lock().unwrap().insert(key.to_owned(), value);
        Ok(())
    }

    fn memory_recall(
        &self,
        key: &str,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, KernelOpError> {
        Ok(self.kv.lock().unwrap().get(key).cloned())
    }

    fn memory_list(
        &self,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<Vec<String>, KernelOpError> {
        Ok(self.kv.lock().unwrap().keys().cloned().collect())
    }
}

// ── All other role traits — defaults (no-op / unavailable) ──────────────────

impl WikiAccess for KernelHandleStub {}
impl CronControl for KernelHandleStub {}
impl ApprovalGate for KernelHandleStub {}
impl HandsControl for KernelHandleStub {}
impl A2ARegistry for KernelHandleStub {}
impl ChannelSender for KernelHandleStub {}
impl PromptStore for KernelHandleStub {}
impl GoalControl for KernelHandleStub {}
impl ToolPolicy for KernelHandleStub {}
impl CatalogQuery for KernelHandleStub {}
impl AcpFsBridge for KernelHandleStub {}
impl AcpTerminalBridge for KernelHandleStub {}

impl ApiAuth for KernelHandleStub {
    fn auth_snapshot(&self) -> ApiAuthSnapshot {
        ApiAuthSnapshot::default()
    }
}

impl SessionWriter for KernelHandleStub {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}

#[async_trait]
impl TaskQueue for KernelHandleStub {
    async fn task_post(
        &self,
        _title: &str,
        _description: &str,
        _assigned_to: Option<&str>,
        _created_by: Option<&str>,
    ) -> Result<String, KernelOpError> {
        Err(KernelOpError::unavailable("stub"))
    }
    async fn task_claim(
        &self,
        _agent_id: &str,
    ) -> Result<Option<serde_json::Value>, KernelOpError> {
        Ok(None)
    }
    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), KernelOpError> {
        Err(KernelOpError::unavailable("stub"))
    }
    async fn task_list(
        &self,
        _status: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, KernelOpError> {
        Ok(vec![])
    }
    async fn task_delete(&self, _task_id: &str) -> Result<bool, KernelOpError> {
        Ok(false)
    }
    async fn task_retry(&self, _task_id: &str) -> Result<bool, KernelOpError> {
        Ok(false)
    }
    async fn task_get(&self, _task_id: &str) -> Result<Option<serde_json::Value>, KernelOpError> {
        Ok(None)
    }
    async fn task_update_status(
        &self,
        _task_id: &str,
        _new_status: &str,
    ) -> Result<bool, KernelOpError> {
        Ok(false)
    }
}

#[async_trait]
impl EventBus for KernelHandleStub {
    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), KernelOpError> {
        Ok(())
    }
}

#[async_trait]
impl KnowledgeGraph for KernelHandleStub {
    async fn knowledge_add_entity(
        &self,
        _entity: &librefang_types::memory::Entity,
    ) -> Result<String, KernelOpError> {
        Err(KernelOpError::unavailable("stub"))
    }
    async fn knowledge_add_relation(
        &self,
        _relation: &librefang_types::memory::Relation,
    ) -> Result<String, KernelOpError> {
        Err(KernelOpError::unavailable("stub"))
    }
    async fn knowledge_query(
        &self,
        _pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, KernelOpError> {
        Ok(vec![])
    }
}

// ---------------------------------------------------------------------------
// Workspace helper
// ---------------------------------------------------------------------------

/// Locate the workspace root by walking up from CARGO_MANIFEST_DIR until
/// a directory containing Cargo.toml with `[workspace]` is found, or
/// until the Cargo.toml at the crate level itself is the root. In CI
/// the env var CARGO_MANIFEST_DIR reliably points at `crates/librefang-runtime/`.
pub fn workspace_root() -> std::path::PathBuf {
    // The test binary's CARGO_MANIFEST_DIR is always `crates/<crate>/`.
    // Walking up two levels gives the workspace root.
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned()
}

/// Read `examples/plugins/<dir>/pre-built/plugin.wasm` from the workspace root.
pub fn wasm_bytes(example_dir: &str) -> Vec<u8> {
    let path = workspace_root()
        .join("examples/plugins")
        .join(example_dir)
        .join("pre-built/plugin.wasm");
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "Cannot read pre-built wasm for example '{}' at {}: {e}\n\
             Run: cargo xtask plugins-rebuild {example_dir}",
            example_dir,
            path.display()
        )
    })
}
