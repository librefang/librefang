//! Regression test for issue #3553.
//!
//! `KernelHandle::send_channel_file_data` now takes `bytes::Bytes` instead
//! of `Vec<u8>`. The structural win is that wrapping layers (channel
//! adapters that retry, metering, fan-out) can `.clone()` the buffer for
//! free instead of copying it. This test pins that property: cloning a
//! `Bytes` returned from `Bytes::from(Vec<u8>)` must NOT allocate a fresh
//! buffer — clones share the underlying allocation.
//!
//! It also exercises the trait method through a stub implementor so any
//! future signature drift breaks compilation here, not silently inside
//! `librefang-kernel`.

use async_trait::async_trait;
use bytes::Bytes;
use librefang_kernel_handle::prelude::*;
use librefang_types::memory::{Entity, GraphMatch, GraphPattern, Relation};
use std::sync::Mutex;

struct CapturingFileKernel {
    // Store the buffer address as `usize` rather than `*const u8` so the
    // struct stays auto-`Send + Sync` without an `unsafe impl`. We only
    // compare addresses, never deref.
    last_seen_addr: Mutex<Option<usize>>,
    last_seen_len: Mutex<usize>,
}

impl CapturingFileKernel {
    fn new() -> Self {
        Self {
            last_seen_addr: Mutex::new(None),
            last_seen_len: Mutex::new(0),
        }
    }
}

#[async_trait]
impl AgentControl for CapturingFileKernel {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

impl MemoryAccess for CapturingFileKernel {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn memory_recall(
        &self,
        _key: &str,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn memory_list(&self, _peer_id: Option<&str>) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait]
impl TaskQueue for CapturingFileKernel {
    async fn task_post(
        &self,
        _title: &str,
        _description: &str,
        _assigned_to: Option<&str>,
        _created_by: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_list(&self, _status: Option<&str>) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_delete(&self, _task_id: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_retry(&self, _task_id: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_get(&self, _task_id: &str) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_update_status(&self, _task_id: &str, _new_status: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait]
impl EventBus for CapturingFileKernel {
    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait]
impl KnowledgeGraph for CapturingFileKernel {
    async fn knowledge_add_entity(&self, _entity: &Entity) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn knowledge_add_relation(&self, _relation: &Relation) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn knowledge_query(&self, _pattern: GraphPattern) -> Result<Vec<GraphMatch>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

impl CronControl for CapturingFileKernel {}
impl ApprovalGate for CapturingFileKernel {}
impl HandsControl for CapturingFileKernel {}
impl A2ARegistry for CapturingFileKernel {}

#[async_trait]
impl ChannelSender for CapturingFileKernel {
    // The method under test. Records the underlying pointer + length so
    // the test can assert the kernel observed the same allocation as the
    // caller's Bytes — i.e. the trait did not silently copy the buffer.
    async fn send_channel_file_data(
        &self,
        _channel: &str,
        _recipient: &str,
        data: Bytes,
        _filename: &str,
        _mime_type: &str,
        _thread_id: Option<&str>,
        _account_id: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        *self.last_seen_addr.lock().unwrap() = Some(data.as_ptr() as usize);
        *self.last_seen_len.lock().unwrap() = data.len();
        Ok("captured".into())
    }
}

impl PromptStore for CapturingFileKernel {}
impl WorkflowRunner for CapturingFileKernel {}
impl GoalControl for CapturingFileKernel {}
impl ToolPolicy for CapturingFileKernel {}

#[test]
fn cloning_bytes_shares_underlying_allocation() {
    // A 10 MiB payload — the size that motivated #3553 (paired with #3514).
    let payload: Vec<u8> = vec![0xAB; 10 * 1024 * 1024];
    let original = Bytes::from(payload);
    let original_addr = original.as_ptr() as usize;

    // Each clone is a refcount bump; the underlying buffer is unchanged.
    let clone_a = original.clone();
    let clone_b = original.clone();
    let clone_c = clone_b.clone();

    assert_eq!(original_addr, clone_a.as_ptr() as usize);
    assert_eq!(original_addr, clone_b.as_ptr() as usize);
    assert_eq!(original_addr, clone_c.as_ptr() as usize);
    assert_eq!(original.len(), clone_c.len());
}

#[tokio::test]
async fn send_channel_file_data_does_not_copy_buffer() {
    let kernel = CapturingFileKernel::new();
    let payload: Vec<u8> = vec![0x42; 4096];
    let original = Bytes::from(payload);
    let expected_addr = original.as_ptr() as usize;
    let expected_len = original.len();

    // Cloning at the call site simulates the metering / retry wrappers
    // the issue describes. The kernel must see the same allocation.
    kernel
        .send_channel_file_data(
            "telegram",
            "user@example",
            original.clone(),
            "test.bin",
            "application/octet-stream",
            None,
            None,
        )
        .await
        .expect("capture call should succeed");

    let seen_addr = kernel
        .last_seen_addr
        .lock()
        .unwrap()
        .expect("kernel observed Bytes");
    let seen_len = *kernel.last_seen_len.lock().unwrap();

    assert_eq!(
        seen_addr, expected_addr,
        "Bytes clone must share its allocation with the caller — \
         a different address means the trait copied the buffer"
    );
    assert_eq!(seen_len, expected_len);
}

#[test]
fn vec_to_bytes_round_trip_is_zero_copy_for_unique_bytes() {
    // The kernel impl converts back to Vec<u8> at the
    // `ChannelContent::FileData` boundary. `Vec::from(Bytes)` is
    // documented O(1) when the Bytes uniquely owns its allocation
    // (bytes 1.x via the vtable's `into_vec`). Pin that here so a
    // future bytes-crate change is caught.
    let original_vec: Vec<u8> = vec![0xCD; 8192];
    let original_addr = original_vec.as_ptr() as usize;

    let bytes = Bytes::from(original_vec);
    assert_eq!(bytes.as_ptr() as usize, original_addr);

    let round_tripped: Vec<u8> = Vec::from(bytes);
    assert_eq!(round_tripped.as_ptr() as usize, original_addr);
    assert_eq!(round_tripped.len(), 8192);
}
