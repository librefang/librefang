//! [`kernel_handle::A2ARegistry`] — listing of trusted external A2A peers.
//! Returns the canonical trust-list key (not `card.url`) so callers get a
//! URL the gate at `/api/a2a/send` will accept (#3786).

use librefang_runtime::a2a::AgentCard;
use librefang_runtime::kernel_handle;
use std::sync::{Mutex, MutexGuard};

use super::super::LibreFangKernel;

fn lock_a2a_agents(
    agents: &Mutex<Vec<(String, AgentCard)>>,
) -> MutexGuard<'_, Vec<(String, AgentCard)>> {
    agents.lock().unwrap_or_else(|poisoned| {
        tracing::warn!("A2A registry lock poisoned; recovering trusted agent state");
        agents.clear_poison();
        poisoned.into_inner()
    })
}

impl kernel_handle::A2ARegistry for LibreFangKernel {
    fn list_a2a_agents(&self) -> Vec<(String, String)> {
        let agents = lock_a2a_agents(&self.mesh.a2a_external_agents);
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
        let agents = lock_a2a_agents(&self.mesh.a2a_external_agents);
        let name_lower = name.to_lowercase();
        // See list_a2a_agents — return the trust-list key, not card.url,
        // so callers get a URL that the gate will accept.
        agents
            .iter()
            .find(|(_, card)| card.name.to_lowercase() == name_lower)
            .map(|(key, _)| key.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_runtime::a2a::AgentCapabilities;

    fn card(name: &str, url: &str) -> AgentCard {
        AgentCard {
            name: name.to_string(),
            description: "test agent".to_string(),
            url: url.to_string(),
            version: "1.0".to_string(),
            capabilities: AgentCapabilities::default(),
            skills: Vec::new(),
            default_input_modes: Vec::new(),
            default_output_modes: Vec::new(),
        }
    }

    #[test]
    fn poisoned_a2a_registry_lock_recovers_trusted_agents() {
        let agents = Mutex::new(vec![(
            "https://trusted.example".to_string(),
            card("Trusted", "https://trusted.example/a2a"),
        )]);
        let poison = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let mut agents = agents.lock().unwrap();
                    agents.push((
                        "https://backup.example".to_string(),
                        card("Backup", "https://backup.example/a2a"),
                    ));
                    panic!("poison A2A registry lock");
                })
                .join()
        });

        assert!(poison.is_err());
        assert!(agents.is_poisoned());
        let mut recovered = lock_a2a_agents(&agents);
        assert_eq!(recovered.len(), 2);
        assert_eq!(recovered[0].1.name, "Trusted");
        assert!(!agents.is_poisoned());
        recovered.retain(|(_, card)| card.name != "Backup");
        drop(recovered);
        assert_eq!(lock_a2a_agents(&agents).len(), 1);
        assert!(!agents.is_poisoned());
    }
}
