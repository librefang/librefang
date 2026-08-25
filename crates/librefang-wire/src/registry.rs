//! Peer registry — tracks connected peers and their agents.
//!
//! The [`PeerRegistry`] is a thread-safe, concurrent data structure that
//! records all known remote peers, their connection state, and the agents
//! they advertise.

use crate::message::RemoteAgentInfo;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// A tracked remote agent, enriched with the owning peer's identity.
#[derive(Debug, Clone)]
pub struct RemoteAgent {
    /// The remote peer that hosts this agent.
    pub peer_node_id: String,
    /// Agent details from the wire protocol.
    pub info: RemoteAgentInfo,
}

/// Connection state of a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    /// Handshake completed, fully connected.
    Connected,
    /// Connection lost but not removed yet (eligible for reconnect).
    Disconnected,
}

/// An entry representing a single known peer.
#[derive(Debug, Clone)]
pub struct PeerEntry {
    /// Unique node ID of the peer.
    pub node_id: String,
    /// Human-readable node name.
    pub node_name: String,
    /// Socket address of the peer.
    pub address: SocketAddr,
    /// Agents advertised by this peer.
    pub agents: Vec<RemoteAgentInfo>,
    /// Connection state.
    pub state: PeerState,
    /// When the peer first connected.
    pub connected_at: DateTime<Utc>,
    /// Protocol version negotiated during handshake.
    pub protocol_version: u32,
}

/// Thread-safe registry of all known peers.
#[derive(Debug, Clone)]
pub struct PeerRegistry {
    peers: Arc<RwLock<HashMap<String, PeerEntry>>>,
}

impl PeerRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn read_peers(&self) -> RwLockReadGuard<'_, HashMap<String, PeerEntry>> {
        self.peers.read().unwrap_or_else(|poisoned| {
            tracing::warn!("peer registry read lock poisoned; recovering inner state");
            self.peers.clear_poison();
            poisoned.into_inner()
        })
    }

    fn write_peers(&self) -> RwLockWriteGuard<'_, HashMap<String, PeerEntry>> {
        self.peers.write().unwrap_or_else(|poisoned| {
            tracing::warn!("peer registry write lock poisoned; recovering inner state");
            self.peers.clear_poison();
            poisoned.into_inner()
        })
    }

    /// Register or update a peer after a successful handshake.
    pub fn add_peer(&self, entry: PeerEntry) {
        let mut peers = self.write_peers();
        peers.insert(entry.node_id.clone(), entry);
    }

    /// Remove a peer entirely.
    pub fn remove_peer(&self, node_id: &str) -> Option<PeerEntry> {
        let mut peers = self.write_peers();
        peers.remove(node_id)
    }

    /// Mark a peer as disconnected (but keep its entry for possible reconnect).
    pub fn mark_disconnected(&self, node_id: &str) {
        let mut peers = self.write_peers();
        if let Some(entry) = peers.get_mut(node_id) {
            entry.state = PeerState::Disconnected;
        }
    }

    /// Mark a peer as connected again.
    pub fn mark_connected(&self, node_id: &str) {
        let mut peers = self.write_peers();
        if let Some(entry) = peers.get_mut(node_id) {
            entry.state = PeerState::Connected;
        }
    }

    /// Get a snapshot of a specific peer.
    pub fn get_peer(&self, node_id: &str) -> Option<PeerEntry> {
        let peers = self.read_peers();
        peers.get(node_id).cloned()
    }

    /// Get all connected peers.
    pub fn connected_peers(&self) -> Vec<PeerEntry> {
        let peers = self.read_peers();
        peers
            .values()
            .filter(|p| p.state == PeerState::Connected)
            .cloned()
            .collect()
    }

    /// Get all peers (connected + disconnected).
    pub fn all_peers(&self) -> Vec<PeerEntry> {
        let peers = self.read_peers();
        peers.values().cloned().collect()
    }

    /// Update the agent list for a peer (e.g., after an AgentSpawned notification).
    pub fn update_agents(&self, node_id: &str, agents: Vec<RemoteAgentInfo>) {
        let mut peers = self.write_peers();
        if let Some(entry) = peers.get_mut(node_id) {
            entry.agents = agents;
        }
    }

    /// Add a single agent to a peer's advertised list.
    pub fn add_agent(&self, node_id: &str, agent: RemoteAgentInfo) {
        let mut peers = self.write_peers();
        if let Some(entry) = peers.get_mut(node_id) {
            // Replace if agent with same ID already exists, otherwise push
            if let Some(existing) = entry.agents.iter_mut().find(|a| a.id == agent.id) {
                *existing = agent;
            } else {
                entry.agents.push(agent);
            }
        }
    }

    /// Remove an agent from a peer's advertised list.
    pub fn remove_agent(&self, node_id: &str, agent_id: &str) {
        let mut peers = self.write_peers();
        if let Some(entry) = peers.get_mut(node_id) {
            entry.agents.retain(|a| a.id != agent_id);
        }
    }

    /// Find all remote agents matching a query (searches name, tags, description).
    pub fn find_agents(&self, query: &str) -> Vec<RemoteAgent> {
        let query_lower = query.to_lowercase();
        let peers = self.read_peers();
        let mut results = Vec::new();

        for peer in peers.values() {
            if peer.state != PeerState::Connected {
                continue;
            }
            for agent in &peer.agents {
                let matches = agent.name.to_lowercase().contains(&query_lower)
                    || agent.description.to_lowercase().contains(&query_lower)
                    || agent
                        .tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query_lower));
                if matches {
                    results.push(RemoteAgent {
                        peer_node_id: peer.node_id.clone(),
                        info: agent.clone(),
                    });
                }
            }
        }

        results
    }

    /// Get all remote agents across all connected peers.
    pub fn all_remote_agents(&self) -> Vec<RemoteAgent> {
        let peers = self.read_peers();
        let mut results = Vec::new();

        for peer in peers.values() {
            if peer.state != PeerState::Connected {
                continue;
            }
            for agent in &peer.agents {
                results.push(RemoteAgent {
                    peer_node_id: peer.node_id.clone(),
                    info: agent.clone(),
                });
            }
        }

        results
    }

    /// Number of connected peers.
    pub fn connected_count(&self) -> usize {
        let peers = self.read_peers();
        peers
            .values()
            .filter(|p| p.state == PeerState::Connected)
            .count()
    }

    /// Total number of peers (including disconnected).
    pub fn total_count(&self) -> usize {
        let peers = self.read_peers();
        peers.len()
    }
}

impl Default for PeerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_peer(node_id: &str, agents: Vec<RemoteAgentInfo>) -> PeerEntry {
        PeerEntry {
            node_id: node_id.to_string(),
            node_name: format!("{node_id}-name"),
            address: "127.0.0.1:9000".parse().unwrap(),
            agents,
            state: PeerState::Connected,
            connected_at: Utc::now(),
            protocol_version: 1,
        }
    }

    fn make_agent(id: &str, name: &str, tags: &[&str]) -> RemoteAgentInfo {
        RemoteAgentInfo {
            id: id.to_string(),
            name: name.to_string(),
            description: format!("{name} agent"),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            tools: vec![],
            state: "running".to_string(),
        }
    }

    #[test]
    fn test_add_and_get_peer() {
        let registry = PeerRegistry::new();
        let peer = make_peer("node-1", vec![make_agent("a1", "coder", &["code"])]);
        registry.add_peer(peer);

        let retrieved = registry.get_peer("node-1").unwrap();
        assert_eq!(retrieved.node_id, "node-1");
        assert_eq!(retrieved.agents.len(), 1);
        assert_eq!(retrieved.agents[0].name, "coder");
    }

    #[test]
    fn test_remove_peer() {
        let registry = PeerRegistry::new();
        registry.add_peer(make_peer("node-1", vec![]));
        assert_eq!(registry.total_count(), 1);

        let removed = registry.remove_peer("node-1");
        assert!(removed.is_some());
        assert_eq!(registry.total_count(), 0);
    }

    #[test]
    fn test_disconnect_reconnect() {
        let registry = PeerRegistry::new();
        registry.add_peer(make_peer("node-1", vec![]));
        assert_eq!(registry.connected_count(), 1);

        registry.mark_disconnected("node-1");
        assert_eq!(registry.connected_count(), 0);
        assert_eq!(registry.total_count(), 1);

        registry.mark_connected("node-1");
        assert_eq!(registry.connected_count(), 1);
    }

    #[test]
    fn test_find_agents_by_name() {
        let registry = PeerRegistry::new();
        registry.add_peer(make_peer(
            "node-1",
            vec![
                make_agent("a1", "coder", &["code"]),
                make_agent("a2", "researcher", &["research"]),
            ],
        ));
        registry.add_peer(make_peer(
            "node-2",
            vec![make_agent("a3", "code-reviewer", &["code", "review"])],
        ));

        let results = registry.find_agents("code");
        assert_eq!(results.len(), 2); // "coder" and "code-reviewer"
    }

    #[test]
    fn test_find_agents_by_tag() {
        let registry = PeerRegistry::new();
        registry.add_peer(make_peer(
            "node-1",
            vec![make_agent("a1", "helper", &["security", "audit"])],
        ));

        let results = registry.find_agents("security");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].info.name, "helper");
        assert_eq!(results[0].peer_node_id, "node-1");
    }

    #[test]
    fn test_find_agents_skips_disconnected() {
        let registry = PeerRegistry::new();
        registry.add_peer(make_peer(
            "node-1",
            vec![make_agent("a1", "coder", &["code"])],
        ));
        registry.mark_disconnected("node-1");

        let results = registry.find_agents("coder");
        assert!(results.is_empty());
    }

    #[test]
    fn test_add_remove_agent() {
        let registry = PeerRegistry::new();
        registry.add_peer(make_peer("node-1", vec![]));

        registry.add_agent("node-1", make_agent("a1", "coder", &[]));
        assert_eq!(registry.get_peer("node-1").unwrap().agents.len(), 1);

        registry.remove_agent("node-1", "a1");
        assert_eq!(registry.get_peer("node-1").unwrap().agents.len(), 0);
    }

    #[test]
    fn test_all_remote_agents() {
        let registry = PeerRegistry::new();
        registry.add_peer(make_peer("node-1", vec![make_agent("a1", "coder", &[])]));
        registry.add_peer(make_peer(
            "node-2",
            vec![
                make_agent("a2", "writer", &[]),
                make_agent("a3", "tester", &[]),
            ],
        ));

        let all = registry.all_remote_agents();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_recovers_registry_state_after_lock_poisoning() {
        let registry = PeerRegistry::new();
        registry.add_peer(make_peer("node-1", vec![]));

        let poisoner = registry.clone();
        let joined = std::thread::spawn(move || {
            let _guard = poisoner.peers.write().unwrap();
            panic!("poison peer registry for recovery test");
        })
        .join();
        assert!(joined.is_err());
        assert!(registry.peers.is_poisoned());

        // The first read recovers the preserved state and clears the poison
        // flag so later reads and writes use the ordinary lock path.
        assert_eq!(registry.total_count(), 1);
        assert!(!registry.peers.is_poisoned());

        let poisoner = registry.clone();
        let joined = std::thread::spawn(move || {
            let _guard = poisoner.peers.write().unwrap();
            panic!("poison peer registry before write recovery");
        })
        .join();
        assert!(joined.is_err());
        assert!(registry.peers.is_poisoned());

        registry.add_peer(make_peer("node-2", vec![]));
        assert_eq!(registry.total_count(), 2);
        assert!(!registry.peers.is_poisoned());
    }
}
