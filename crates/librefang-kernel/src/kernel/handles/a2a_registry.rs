//! [`kernel_handle::A2ARegistry`] — listing of trusted external A2A peers.
//! Returns the canonical trust-list key (not `card.url`) so callers get a
//! URL the gate at `/api/a2a/send` will accept (#3786).

use librefang_runtime::kernel_handle;

use super::super::LibreFangKernel;

impl kernel_handle::A2ARegistry for LibreFangKernel {
    fn list_a2a_agents(&self) -> Vec<(String, String)> {
        let agents = self
            .mesh
            .a2a_external_agents
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Return (name, key) pairs where `key` is the trust-list key
        // (first tuple element), not `card.url`. The card's self-declared
        // url is `<base>/a2a` while the trust gate at /api/a2a/send and
        // tool_a2a_send compare against the canonicalized base URL. Using
        // `card.url` here would silently mismatch the gate and break every
        // statically-seeded entry. (Bug #3786)
        agents
            .iter()
            .map(|(key, card)| (card.name.clone(), key.clone()))
            .collect()
    }

    fn get_a2a_agent_url(&self, name: &str) -> Option<String> {
        let agents = self
            .mesh
            .a2a_external_agents
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let name_lower = name.to_lowercase();
        // See list_a2a_agents — return the trust-list key, not card.url,
        // so callers get a URL that the gate will accept.
        agents
            .iter()
            .find(|(_, card)| card.name.to_lowercase() == name_lower)
            .map(|(key, _)| key.clone())
    }
}
