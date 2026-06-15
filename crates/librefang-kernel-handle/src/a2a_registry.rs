// ============================================================================
// 9. A2ARegistry — discovered external A2A agents (read-only directory)
// ============================================================================

pub trait A2ARegistry: Send + Sync {
    /// List discovered external A2A agents as (name, url) pairs.
    fn list_a2a_agents(&self) -> Vec<(String, String)> {
        vec![]
    }

    /// Get the URL of a discovered external A2A agent by name.
    fn get_a2a_agent_url(&self, name: &str) -> Option<String> {
        let _ = name;
        None
    }
}
