//! Per-peer Ed25519 identity for the OFP wire protocol.
//!
//! Each kernel persists one Ed25519 keypair under `<data_dir>/peer_keypair.json`
//! and presents it during the handshake. Recipients pin the public key to the
//! advertised `node_id` (TOFU) so a leaked `shared_secret` can no longer be
//! used to impersonate other nodes — the attacker would also need the private
//! key file of the node they wish to spoof.
//!
//! This module ships the key primitive and on-disk format. Wiring it into the
//! handshake itself is done in `peer.rs` (see #3873).

use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

#[derive(Error, Debug)]
pub enum KeyError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Invalid key format")]
    InvalidFormat,
    #[error("OS RNG failure: {0}")]
    Rng(String),
    #[error("Signature verification failed")]
    BadSignature,
}

/// Persisted shape on disk. Both halves are base64-encoded; the file MUST be
/// kept readable only by the daemon user (caller's responsibility).
///
/// `node_id` was added in PR-3 of #3873. Files written by PR-1 are missing
/// the field — `load_or_generate` materializes a UUID and rewrites the file
/// in that case so subsequent restarts see a stable identity.
#[derive(Serialize, Deserialize)]
struct PersistedKeyPair {
    public_key: String,
    private_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    node_id: Option<String>,
}

/// An Ed25519 keypair owned by this node.
///
/// `public_key` is base64(32 bytes). `private_key_bytes` is the raw 32-byte
/// seed; never serialized via the public `Serialize` impl on this type.
#[derive(Clone)]
pub struct Ed25519KeyPair {
    public_key: String,
    private_key_bytes: [u8; 32],
}

impl std::fmt::Debug for Ed25519KeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ed25519KeyPair")
            .field("public_key", &self.public_key)
            .field("private_key_bytes", &"[redacted]")
            .finish()
    }
}

impl Ed25519KeyPair {
    /// Generate a fresh keypair using the OS CSPRNG.
    pub fn generate() -> Result<Self, KeyError> {
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        let signing = SigningKey::from_bytes(&seed);
        let verifying = signing.verifying_key();
        Ok(Self {
            public_key: B64.encode(verifying.as_bytes()),
            private_key_bytes: seed,
        })
    }

    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    pub fn verifying_key(&self) -> Result<VerifyingKey, KeyError> {
        let bytes = B64
            .decode(&self.public_key)
            .map_err(|_| KeyError::InvalidFormat)?;
        if bytes.len() != 32 {
            return Err(KeyError::InvalidFormat);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        VerifyingKey::from_bytes(&arr).map_err(|_| KeyError::InvalidFormat)
    }

    fn signing_key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.private_key_bytes)
    }

    /// Sign `data`; returns base64(64-byte signature).
    pub fn sign(&self, data: &[u8]) -> String {
        let sig: Signature = self.signing_key().sign(data);
        B64.encode(sig.to_bytes())
    }

    /// SHA-256 fingerprint of the base64 public key, hex-encoded. Stable
    /// human-comparable string for out-of-band verification.
    pub fn fingerprint(&self) -> String {
        fingerprint_of_pubkey(&self.public_key)
    }
}

/// Verify that `signature` (base64) is a valid Ed25519 signature of `data`
/// under `public_key` (base64).
pub fn verify_signature(public_key: &str, data: &[u8], signature: &str) -> Result<(), KeyError> {
    let pk_bytes = B64
        .decode(public_key)
        .map_err(|_| KeyError::InvalidFormat)?;
    if pk_bytes.len() != 32 {
        return Err(KeyError::InvalidFormat);
    }
    let mut pk_arr = [0u8; 32];
    pk_arr.copy_from_slice(&pk_bytes);
    let vk = VerifyingKey::from_bytes(&pk_arr).map_err(|_| KeyError::InvalidFormat)?;

    let sig_bytes = B64.decode(signature).map_err(|_| KeyError::InvalidFormat)?;
    if sig_bytes.len() != 64 {
        return Err(KeyError::InvalidFormat);
    }
    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(&sig_bytes);
    let sig = Signature::from_bytes(&sig_arr);

    vk.verify(data, &sig).map_err(|_| KeyError::BadSignature)
}

/// SHA-256(public_key_b64) hex — stable fingerprint for OOB verification.
pub fn fingerprint_of_pubkey(public_key: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(public_key.as_bytes());
    hex::encode(hasher.finalize())
}

/// Loads `<data_dir>/peer_keypair.json` if present, otherwise generates a
/// fresh keypair + node_id and persists them. The file stores the keypair
/// (both halves base64) **and** the node_id together so a daemon restart
/// resumes under the same OFP identity — without this, every restart
/// minted a new `Uuid::new_v4()` for the node_id and the TOFU pin map
/// from #3873 had no stable key to bind against.
pub struct PeerKeyManager {
    key_path: PathBuf,
    keypair: Option<Ed25519KeyPair>,
    node_id: Option<String>,
}

impl PeerKeyManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            key_path: data_dir.join("peer_keypair.json"),
            keypair: None,
            node_id: None,
        }
    }

    /// Load the persisted identity if present, otherwise generate a fresh
    /// one and write it to disk. After this returns successfully, both
    /// [`PeerKeyManager::keypair`] and [`PeerKeyManager::node_id`] are
    /// guaranteed to be `Some`.
    pub fn load_or_generate(&mut self) -> Result<&Ed25519KeyPair, KeyError> {
        if let Some(ref kp) = self.keypair {
            return Ok(kp);
        }

        let (kp, node_id, needs_rewrite) = if self.key_path.exists() {
            let raw = std::fs::read_to_string(&self.key_path)?;
            let persisted: PersistedKeyPair = serde_json::from_str(&raw)?;
            let priv_bytes = B64
                .decode(&persisted.private_key)
                .map_err(|_| KeyError::InvalidFormat)?;
            if priv_bytes.len() != 32 {
                return Err(KeyError::InvalidFormat);
            }
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&priv_bytes);
            // Re-derive the public key from the seed and cross-check the file.
            let derived_pub = B64.encode(SigningKey::from_bytes(&seed).verifying_key().as_bytes());
            if derived_pub != persisted.public_key {
                return Err(KeyError::InvalidFormat);
            }
            // Migrate PR-1 files (no node_id field) by minting one and
            // marking the file for rewrite.
            let (node_id, rewrite) = match persisted.node_id {
                Some(id) if !id.is_empty() => (id, false),
                _ => (uuid::Uuid::new_v4().to_string(), true),
            };
            (
                Ed25519KeyPair {
                    public_key: persisted.public_key,
                    private_key_bytes: seed,
                },
                node_id,
                rewrite,
            )
        } else {
            let kp = Ed25519KeyPair::generate()?;
            let node_id = uuid::Uuid::new_v4().to_string();
            (kp, node_id, true)
        };

        if needs_rewrite {
            if let Some(parent) = self.key_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let persisted = PersistedKeyPair {
                public_key: kp.public_key.clone(),
                private_key: B64.encode(kp.private_key_bytes),
                node_id: Some(node_id.clone()),
            };
            let serialized = serde_json::to_string_pretty(&persisted)?;
            std::fs::write(&self.key_path, serialized)?;
            // Best-effort tighten file perms on Unix (0600). Failure is
            // non-fatal — caller is responsible for data_dir mode.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = std::fs::metadata(&self.key_path) {
                    let mut perms = meta.permissions();
                    perms.set_mode(0o600);
                    let _ = std::fs::set_permissions(&self.key_path, perms);
                }
            }
        }

        self.node_id = Some(node_id);
        Ok(self.keypair.insert(kp))
    }

    pub fn keypair(&self) -> Option<&Ed25519KeyPair> {
        self.keypair.as_ref()
    }

    pub fn public_key(&self) -> Option<&str> {
        self.keypair.as_ref().map(|kp| kp.public_key())
    }

    /// Stable node_id persisted alongside the keypair. `Some` after a
    /// successful [`load_or_generate`].
    pub fn node_id(&self) -> Option<&str> {
        self.node_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_unique_keys() {
        let a = Ed25519KeyPair::generate().unwrap();
        let b = Ed25519KeyPair::generate().unwrap();
        assert_ne!(a.public_key(), b.public_key());
        assert_ne!(a.private_key_bytes, b.private_key_bytes);
    }

    #[test]
    fn sign_then_verify_roundtrip() {
        let kp = Ed25519KeyPair::generate().unwrap();
        let msg = b"OFP handshake nonce";
        let sig = kp.sign(msg);
        verify_signature(kp.public_key(), msg, &sig).expect("signature must verify");
    }

    #[test]
    fn verify_rejects_tampered_message() {
        let kp = Ed25519KeyPair::generate().unwrap();
        let sig = kp.sign(b"original");
        assert!(matches!(
            verify_signature(kp.public_key(), b"tampered", &sig),
            Err(KeyError::BadSignature)
        ));
    }

    #[test]
    fn verify_rejects_other_peers_pubkey() {
        let kp_a = Ed25519KeyPair::generate().unwrap();
        let kp_b = Ed25519KeyPair::generate().unwrap();
        let sig = kp_a.sign(b"msg");
        assert!(matches!(
            verify_signature(kp_b.public_key(), b"msg", &sig),
            Err(KeyError::BadSignature)
        ));
    }

    /// CRITICAL: persistence roundtrip must preserve the private key.
    /// The previous implementation marked `private_key_bytes` as
    /// `#[serde(skip)]`, silently dropping the private key on save and
    /// returning a zero-length key on reload — signing then panicked.
    #[test]
    fn manager_persistence_roundtrip_preserves_private_key() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr_a = PeerKeyManager::new(tmp.path().to_path_buf());
        let kp_a = mgr_a.load_or_generate().unwrap().clone();
        let sig = kp_a.sign(b"ping");

        let mut mgr_b = PeerKeyManager::new(tmp.path().to_path_buf());
        let kp_b = mgr_b.load_or_generate().unwrap();
        assert_eq!(kp_a.public_key(), kp_b.public_key());
        // The reloaded keypair must be able to produce the SAME signature
        // (Ed25519 is deterministic), proving the private key survived.
        assert_eq!(sig, kp_b.sign(b"ping"));
    }

    #[test]
    fn manager_rejects_tampered_pubkey_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr = PeerKeyManager::new(tmp.path().to_path_buf());
        let _ = mgr.load_or_generate().unwrap();
        let path = tmp.path().join("peer_keypair.json");
        let raw = std::fs::read_to_string(&path).unwrap();
        let mut persisted: PersistedKeyPair = serde_json::from_str(&raw).unwrap();
        // Swap in a different valid pubkey while keeping the original priv.
        let other = Ed25519KeyPair::generate().unwrap();
        persisted.public_key = other.public_key().to_string();
        std::fs::write(&path, serde_json::to_string(&persisted).unwrap()).unwrap();

        let mut mgr2 = PeerKeyManager::new(tmp.path().to_path_buf());
        assert!(matches!(
            mgr2.load_or_generate(),
            Err(KeyError::InvalidFormat)
        ));
    }

    #[test]
    fn fingerprint_is_stable_and_unique() {
        let kp = Ed25519KeyPair::generate().unwrap();
        assert_eq!(kp.fingerprint(), fingerprint_of_pubkey(kp.public_key()));
        let other = Ed25519KeyPair::generate().unwrap();
        assert_ne!(kp.fingerprint(), other.fingerprint());
    }

    /// PR-3: node_id MUST persist across restarts. Without this the kernel
    /// fell back to `Uuid::new_v4()` per startup and TOFU pinning had no
    /// stable key to bind against.
    #[test]
    fn manager_persists_node_id_across_restarts() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr_a = PeerKeyManager::new(tmp.path().to_path_buf());
        let _ = mgr_a.load_or_generate().unwrap();
        let id_a = mgr_a.node_id().unwrap().to_string();
        assert!(!id_a.is_empty());

        let mut mgr_b = PeerKeyManager::new(tmp.path().to_path_buf());
        let _ = mgr_b.load_or_generate().unwrap();
        assert_eq!(mgr_b.node_id(), Some(id_a.as_str()));
    }

    /// PR-3 backward compat: a PR-1-format file (no `node_id` field) must
    /// still load successfully. The manager mints a fresh node_id and
    /// rewrites the file so subsequent restarts see a stable identity.
    #[test]
    fn manager_migrates_legacy_file_without_node_id() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("peer_keypair.json");
        // Hand-craft a PR-1-shaped file (only public_key + private_key).
        let kp = Ed25519KeyPair::generate().unwrap();
        let legacy = serde_json::json!({
            "public_key": kp.public_key(),
            "private_key": B64.encode(kp.private_key_bytes),
        });
        std::fs::write(&path, serde_json::to_string_pretty(&legacy).unwrap()).unwrap();

        let mut mgr = PeerKeyManager::new(tmp.path().to_path_buf());
        let loaded = mgr.load_or_generate().unwrap();
        assert_eq!(loaded.public_key(), kp.public_key());
        let migrated_id = mgr.node_id().unwrap().to_string();
        assert!(!migrated_id.is_empty());

        // File must have been rewritten with the new field — a second mgr
        // sees the same node_id without minting a fresh one.
        let mut mgr2 = PeerKeyManager::new(tmp.path().to_path_buf());
        let _ = mgr2.load_or_generate().unwrap();
        assert_eq!(mgr2.node_id(), Some(migrated_id.as_str()));
    }
}
