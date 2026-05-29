//! Canonical `KernelHandleStub` for integration tests.
//!
//! Enabled by the `test-stub` feature on `librefang-kernel-handle`.
//! Add to a test crate's dev-deps:
//! ```toml
//! [dev-dependencies]
//! librefang-kernel-handle = { workspace = true, features = ["test-stub"] }
//! ```
//! Then import:
//! ```
//! use librefang_kernel_handle::test_stub::KernelHandleStub;
//! ```
//!
//! ## Design
//!
//! Only `MemoryAccess` is overridden with a real in-memory HashMap store —
//! needed by plugins that call `kv_get` / `kv_set` via `host_functions::dispatch`.
//! All other 18 role traits use their default implementations
//! (`KernelOpError::unavailable` / empty vecs / no-ops), which is sufficient
//! for most plugin-example integration tests.
//!
//! Promoted from
//! `crates/librefang-runtime/tests/support/plugin_example_harness.rs`
//! in Phase-8 C-005 so downstream test crates don't re-implement the same
//! 200-line boilerplate.

#![cfg(feature = "test-stub")]

use crate::{
    A2ARegistry, AcpFsBridge, AcpTerminalBridge, AgentControl, AgentInfo, ApiAuth, ApiAuthSnapshot,
    ApprovalGate, CatalogQuery, ChannelSender, CronControl, EventBus, GoalControl, HandsControl,
    KernelOpError, KnowledgeGraph, MemoryAccess, PromptStore, SessionWriter, TaskQueue, ToolPolicy,
    WikiAccess, WorkflowRunner,
};
use async_trait::async_trait;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Minimal `KernelHandle` stub for integration tests.
///
/// Implements every role trait. `MemoryAccess` uses an in-memory `HashMap`
/// so plugins that call `kv_get`/`kv_set` get observable side-effects.
/// All other traits use the default unavailable/empty/no-op responses.
#[derive(Default)]
pub struct KernelHandleStub {
    kv: Mutex<HashMap<String, serde_json::Value>>,
}

impl KernelHandleStub {
    /// Create a new stub wrapped in an `Arc`.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Read a value from the in-memory KV store (for test assertions).
    pub fn kv_get(&self, key: &str) -> Option<serde_json::Value> {
        self.kv.lock().unwrap().get(key).cloned()
    }

    /// Pre-seed the in-memory KV store before the plugin runs.
    pub fn kv_seed(&self, key: &str, value: serde_json::Value) {
        self.kv.lock().unwrap().insert(key.to_owned(), value);
    }
}

// ── AgentControl ─────────────────────────────────────────────────────────────

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

// ── MemoryAccess — real in-memory store ──────────────────────────────────────

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

// ── All other role traits — defaults ─────────────────────────────────────────

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
impl WorkflowRunner for KernelHandleStub {}

impl ApiAuth for KernelHandleStub {
    fn auth_snapshot(&self) -> ApiAuthSnapshot {
        ApiAuthSnapshot::default()
    }
}

impl SessionWriter for KernelHandleStub {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _session_id: librefang_types::agent::SessionId,
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
