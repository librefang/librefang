use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TrustError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Peer not found: {0}")]
    NotFound(String),
    #[error("Peer already trusted: {0}")]
    AlreadyTrusted(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TrustMode {
    #[default]
    Legacy,
    Secure,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrustedPeer {
    pub node_id: String,
    pub node_name: Option<String>,
    pub mode: TrustMode,
    pub public_key: Option<String>,
    pub first_seen: DateTime<Utc>,
    pub last_verified: DateTime<Utc>,
    pub fingerprint: Option<String>,
    pub address: Option<String>,
}

impl TrustedPeer {
    pub fn new_legacy(node_id: String, address: Option<String>) -> Self {
        let now = Utc::now();
        TrustedPeer {
            node_id,
            node_name: None,
            mode: TrustMode::Legacy,
            public_key: None,
            first_seen: now,
            last_verified: now,
            fingerprint: None,
            address,
        }
    }

    pub fn new_secure(
        node_id: String,
        public_key: String,
        node_name: Option<String>,
        address: Option<String>,
    ) -> Self {
        use sha2::{Digest, Sha256};

        let now = Utc::now();
        let mut hasher = Sha256::new();
        hasher.update(public_key.as_bytes());
        let fingerprint = hex::encode(hasher.finalize());

        TrustedPeer {
            node_id,
            node_name,
            mode: TrustMode::Secure,
            public_key: Some(public_key),
            first_seen: now,
            last_verified: now,
            fingerprint: Some(fingerprint),
            address,
        }
    }

    pub fn is_secure(&self) -> bool {
        self.mode == TrustMode::Secure && self.public_key.is_some()
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct TrustedPeersStore {
    peers: std::collections::HashMap<String, TrustedPeer>,
}

pub struct TrustedPeers {
    store_path: PathBuf,
    store: TrustedPeersStore,
}

impl TrustedPeers {
    pub fn new(data_dir: PathBuf) -> Self {
        let store_path = data_dir.join("trusted_peers.json");
        TrustedPeers {
            store_path,
            store: TrustedPeersStore::default(),
        }
    }

    pub fn load(&mut self) -> Result<(), TrustError> {
        if self.store_path.exists() {
            let content = std::fs::read_to_string(&self.store_path)?;
            self.store = serde_json::from_str(&content)?;
        }
        Ok(())
    }

    pub fn save(&self) -> Result<(), TrustError> {
        if let Some(parent) = self.store_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(&self.store)?;
        // DATA-INTEGRITY: write atomically. A plain truncate-in-place write
        // can leave a half-written trusted_peers.json if the process crashes
        // mid-write; the next `load()` then fails to deserialize and peer
        // bind refuses to start (fail-closed), bricking OFP startup. Write to
        // a sibling `.tmp` first, tighten perms on it, then rename into place
        // — rename is atomic on the same filesystem, so a reader/loader ever
        // sees either the old complete file or the new complete file.
        let tmp_path = self.store_path.with_extension("json.tmp");
        std::fs::write(&tmp_path, content)?;
        // SECURITY (#3873): Tighten file perms to 0600 on Unix. The store
        // contains every pubkey/fingerprint we trust — leakage doesn't
        // forge signatures (pubkeys are public) but is reconnaissance
        // value, exposing which nodes this daemon federates with.
        // Mirrors the policy `keys.rs::PeerKeyManager` applies to
        // `peer_keypair.json`. Applied to the temp file before rename so the
        // final file is never briefly world-readable. Best-effort;
        // failure here is non-fatal.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&tmp_path) {
                let mut perms = meta.permissions();
                perms.set_mode(0o600);
                let _ = std::fs::set_permissions(&tmp_path, perms);
            }
        }
        std::fs::rename(&tmp_path, &self.store_path)?;
        Ok(())
    }

    pub fn get(&self, node_id: &str) -> Option<&TrustedPeer> {
        self.store.peers.get(node_id)
    }

    pub fn get_mut(&mut self, node_id: &str) -> Option<&mut TrustedPeer> {
        self.store.peers.get_mut(node_id)
    }

    pub fn add(&mut self, peer: TrustedPeer) -> Result<(), TrustError> {
        let node_id = peer.node_id.clone();
        if self.store.peers.contains_key(&node_id) {
            return Err(TrustError::AlreadyTrusted(node_id));
        }
        self.store.peers.insert(node_id, peer);
        self.save()?;
        Ok(())
    }

    pub fn update(&mut self, node_id: &str, peer: TrustedPeer) -> Result<(), TrustError> {
        if !self.store.peers.contains_key(node_id) {
            return Err(TrustError::NotFound(node_id.to_string()));
        }
        self.store.peers.insert(node_id.to_string(), peer);
        self.save()?;
        Ok(())
    }

    pub fn remove(&mut self, node_id: &str) -> Result<(), TrustError> {
        if self.store.peers.remove(node_id).is_none() {
            return Err(TrustError::NotFound(node_id.to_string()));
        }
        self.save()?;
        Ok(())
    }

    pub fn list(&self) -> Vec<&TrustedPeer> {
        self.store.peers.values().collect()
    }

    pub fn list_secure(&self) -> Vec<&TrustedPeer> {
        self.store
            .peers
            .values()
            .filter(|p| p.is_secure())
            .collect()
    }

    pub fn list_legacy(&self) -> Vec<&TrustedPeer> {
        self.store
            .peers
            .values()
            .filter(|p| p.mode == TrustMode::Legacy)
            .collect()
    }

    pub fn find_by_public_key(&self, public_key: &str) -> Option<&TrustedPeer> {
        self.store
            .peers
            .values()
            .find(|p| p.public_key.as_deref() == Some(public_key))
    }

    pub fn trust_peer(
        &mut self,
        node_id: String,
        public_key: String,
        node_name: Option<String>,
        address: Option<String>,
    ) -> Result<(), TrustError> {
        let peer = TrustedPeer::new_secure(node_id, public_key, node_name, address);
        if let Some(existing) = self.store.peers.get_mut(&peer.node_id) {
            existing.mode = TrustMode::Secure;
            existing.public_key = Some(peer.public_key.unwrap_or_default());
            existing.fingerprint = peer.fingerprint;
            existing.last_verified = Utc::now();
            return self.save();
        }
        self.add(peer)
    }

    pub fn downgrade_to_legacy(&mut self, node_id: &str) -> Result<(), TrustError> {
        if let Some(peer) = self.store.peers.get_mut(node_id) {
            peer.mode = TrustMode::Legacy;
            peer.public_key = None;
            peer.fingerprint = None;
            return self.save();
        }
        Err(TrustError::NotFound(node_id.to_string()))
    }

    pub fn verify_connection(&mut self, node_id: &str) -> Result<(), TrustError> {
        if let Some(peer) = self.store.peers.get_mut(node_id) {
            peer.last_verified = Utc::now();
            return self.save();
        }
        Err(TrustError::NotFound(node_id.to_string()))
    }

    pub fn pending_connections(&self) -> Vec<&TrustedPeer> {
        let five_minutes_ago = Utc::now() - chrono::Duration::minutes(5);
        self.store
            .peers
            .values()
            .filter(|p| p.mode == TrustMode::Legacy && p.last_verified < five_minutes_ago)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir().join(format!(
            "librefang-trusted-peers-roundtrip-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let mut peers = TrustedPeers::new(dir.clone());
        peers
            .trust_peer(
                "node-a".to_string(),
                "pubkey-a".to_string(),
                Some("Node A".to_string()),
                Some("127.0.0.1:1234".to_string()),
            )
            .unwrap();

        let mut reloaded = TrustedPeers::new(dir.clone());
        reloaded.load().unwrap();
        let peer = reloaded.get("node-a").expect("peer persisted");
        assert!(peer.is_secure());
        assert_eq!(peer.public_key.as_deref(), Some("pubkey-a"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_does_not_leave_a_torn_file() {
        // A good store already exists on disk. If `save()` truncated in place
        // and then failed mid-write, the store would be corrupt and the next
        // `load()` would error. The atomic temp+rename guarantees the store
        // path always deserializes; verify a fresh loader reads a complete
        // file after a save, and that no leftover `.json.tmp` remains.
        let dir = std::env::temp_dir().join(format!(
            "librefang-trusted-peers-atomic-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let mut peers = TrustedPeers::new(dir.clone());
        peers
            .add(TrustedPeer::new_legacy(
                "node-legacy".to_string(),
                Some("10.0.0.1:9999".to_string()),
            ))
            .unwrap();
        // A second save overwrites the existing good file via rename.
        peers
            .add(TrustedPeer::new_legacy("node-legacy-2".to_string(), None))
            .unwrap();

        let store_path = dir.join("trusted_peers.json");
        let tmp_path = store_path.with_extension("json.tmp");
        assert!(!tmp_path.exists(), "temp file must be renamed away");

        let content = std::fs::read_to_string(&store_path).unwrap();
        // The on-disk file is always a fully serializable store.
        serde_json::from_str::<TrustedPeersStore>(&content).expect("store deserializes");

        let mut reloaded = TrustedPeers::new(dir.clone());
        reloaded.load().unwrap();
        assert!(reloaded.get("node-legacy").is_some());
        assert!(reloaded.get("node-legacy-2").is_some());

        std::fs::remove_dir_all(&dir).ok();
    }
}
