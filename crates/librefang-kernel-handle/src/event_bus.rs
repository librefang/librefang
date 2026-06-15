use async_trait::async_trait;

use super::*;

// ============================================================================
// 4. EventBus — fire-and-forget custom events for proactive triggers
// ============================================================================

#[async_trait]
pub trait EventBus: Send + Sync {
    /// Publish a custom event that can trigger proactive agents.
    async fn publish_event(
        &self,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), KernelOpError>;
}
