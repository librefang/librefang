use async_trait::async_trait;

use super::*;

// ============================================================================
// 8. HandsControl — Hand (specialized agent) lifecycle
// ============================================================================

#[async_trait]
pub trait HandsControl: Send + Sync {
    /// List available Hands and their activation status.
    async fn hand_list(&self) -> Result<Vec<serde_json::Value>, KernelOpError> {
        Err(KernelOpError::unavailable("Hands system"))
    }

    /// Install a Hand from TOML content.
    async fn hand_install(
        &self,
        toml_content: &str,
        skill_content: &str,
    ) -> Result<serde_json::Value, KernelOpError> {
        let _ = (toml_content, skill_content);
        Err(KernelOpError::unavailable("Hands system"))
    }

    /// Activate a Hand — spawns a specialized autonomous agent.
    async fn hand_activate(
        &self,
        hand_id: &str,
        config: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, KernelOpError> {
        let _ = (hand_id, config);
        Err(KernelOpError::unavailable("Hands system"))
    }

    /// Check the status and dashboard metrics of an active Hand.
    async fn hand_status(&self, hand_id: &str) -> Result<serde_json::Value, KernelOpError> {
        let _ = hand_id;
        Err(KernelOpError::unavailable("Hands system"))
    }

    /// Deactivate a running Hand and stop its agent.
    async fn hand_deactivate(&self, instance_id: &str) -> Result<(), KernelOpError> {
        let _ = instance_id;
        Err(KernelOpError::unavailable("Hands system"))
    }
}
