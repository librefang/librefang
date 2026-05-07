//! Mesh subsystem — A2A registry, OFP peer wiring, channel adapters,
//! agent bindings, broadcast config, and the delivery receipt tracker.
//!
//! Bundles eight cross-process / cross-network handles that previously
//! sat as a flat cluster on `LibreFangKernel`. Inner field names are
//! kept intact so the migration is purely mechanical
//! (`self.a2a_task_store` → `self.mesh.a2a_task_store`).

use std::sync::{Arc, Mutex, OnceLock};

use dashmap::DashMap;
use librefang_channels::types::ChannelAdapter;
use librefang_runtime::a2a::{A2aTaskStore, AgentCard};
use librefang_types::config::{AgentBinding, BroadcastConfig};
use librefang_wire::{PeerNode, PeerRegistry};

use crate::kernel::DeliveryTracker;

/// Focused mesh API.
pub trait MeshSubsystemApi: Send + Sync {
    /// A2A task store.
    fn a2a_tasks(&self) -> &A2aTaskStore;
    /// Discovered external A2A agent cards.
    fn a2a_agents(&self) -> &Mutex<Vec<(String, AgentCard)>>;
    /// Bridge-registered channel adapters.
    fn channel_adapters_ref(&self) -> &DashMap<String, Arc<dyn ChannelAdapter>>;
    /// Multi-account agent binding list.
    fn bindings_ref(&self) -> &Mutex<Vec<AgentBinding>>;
    /// Broadcast configuration snapshot.
    fn broadcast_ref(&self) -> &BroadcastConfig;
    /// Delivery receipt tracker.
    fn delivery(&self) -> &DeliveryTracker;
    /// OFP peer registry (set once at startup).
    fn peer_registry_ref(&self) -> Option<&PeerRegistry>;
    /// OFP peer node (set once at startup).
    fn peer_node_ref(&self) -> Option<&Arc<PeerNode>>;
}

/// A2A + peers + channels + bindings cluster — see module docs.
pub struct MeshSubsystem {
    /// A2A task store for tracking task lifecycle.
    pub(crate) a2a_task_store: A2aTaskStore,
    /// Discovered external A2A agent cards.
    pub(crate) a2a_external_agents: Mutex<Vec<(String, AgentCard)>>,
    /// Delivery receipt tracker (bounded LRU, max 10K entries).
    pub(crate) delivery_tracker: DeliveryTracker,
    /// Agent bindings for multi-account routing (Mutex for runtime
    /// add/remove).
    pub(crate) bindings: Mutex<Vec<AgentBinding>>,
    /// Broadcast configuration.
    pub(crate) broadcast: BroadcastConfig,
    /// OFP peer registry — tracks connected peers (set once during OFP
    /// startup).
    pub(crate) peer_registry: OnceLock<PeerRegistry>,
    /// OFP peer node — the local networking node (set once during OFP
    /// startup).
    pub(crate) peer_node: OnceLock<Arc<PeerNode>>,
    /// Channel adapters registered at bridge startup (for proactive
    /// `channel_send` tool).
    pub(crate) channel_adapters: DashMap<String, Arc<dyn ChannelAdapter>>,
}

impl MeshSubsystem {
    pub(crate) fn new(
        a2a_task_store: A2aTaskStore,
        bindings: Vec<AgentBinding>,
        broadcast: BroadcastConfig,
    ) -> Self {
        Self {
            a2a_task_store,
            a2a_external_agents: Mutex::new(Vec::new()),
            delivery_tracker: DeliveryTracker::new(),
            bindings: Mutex::new(bindings),
            broadcast,
            peer_registry: OnceLock::new(),
            peer_node: OnceLock::new(),
            channel_adapters: DashMap::new(),
        }
    }
}

impl MeshSubsystemApi for MeshSubsystem {
    #[inline]
    fn a2a_tasks(&self) -> &A2aTaskStore {
        &self.a2a_task_store
    }

    #[inline]
    fn a2a_agents(&self) -> &Mutex<Vec<(String, AgentCard)>> {
        &self.a2a_external_agents
    }

    #[inline]
    fn channel_adapters_ref(&self) -> &DashMap<String, Arc<dyn ChannelAdapter>> {
        &self.channel_adapters
    }

    #[inline]
    fn bindings_ref(&self) -> &Mutex<Vec<AgentBinding>> {
        &self.bindings
    }

    #[inline]
    fn broadcast_ref(&self) -> &BroadcastConfig {
        &self.broadcast
    }

    #[inline]
    fn delivery(&self) -> &DeliveryTracker {
        &self.delivery_tracker
    }

    #[inline]
    fn peer_registry_ref(&self) -> Option<&PeerRegistry> {
        self.peer_registry.get()
    }

    #[inline]
    fn peer_node_ref(&self) -> Option<&Arc<PeerNode>> {
        self.peer_node.get()
    }
}
