//! [`kernel_handle::EventBus`] — broadcast plumbing on top of
//! [`LibreFangKernel::publish_event`]. Wraps the caller's payload in a
//! `{type, data}` envelope before handing it to the inherent broadcast.

use async_trait::async_trait;
use librefang_runtime::kernel_handle;
use librefang_types::agent::AgentId;
use librefang_types::event::{Event, EventPayload, EventTarget};

use super::super::LibreFangKernel;

#[async_trait]
impl kernel_handle::EventBus for LibreFangKernel {
    async fn publish_event(
        &self,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), kernel_handle::KernelOpError> {
        let system_agent = AgentId::new();
        // `?` lifts via `From<serde_json::Error>` on KernelOpError.
        let payload_bytes =
            serde_json::to_vec(&serde_json::json!({"type": event_type, "data": payload}))?;
        let event = Event::new(
            system_agent,
            EventTarget::Broadcast,
            EventPayload::Custom(payload_bytes),
        );
        LibreFangKernel::publish_event(self, event).await;
        Ok(())
    }
}
