use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum KeyError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Invalid key format")]
    InvalidFormat,
    #[error("Key not found")]
    NotFound,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Ed25519KeyPair {
    public_key: String,
    // Base64-encoded 32-byte private key seed. Must be persisted alongside the
    // public key so the keypair survives daemon restarts.
    private_key: String,
}

impl Ed25519KeyPair {
    pub fn generate() -> Self {
        use rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        Ed25519KeyPair {
            public_key: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                verifying_key.as_bytes(),
            ),
            private_key: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                signing_key.to_bytes(),
            ),
        }
    }

    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    pub fn verifying_key(&self) -> Result<VerifyingKey, KeyError> {
        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &self.public_key)
                .map_err(|_| KeyError::InvalidFormat)?;

        if bytes.len() != 32 {
            return Err(KeyError::InvalidFormat);
        }

        let mut array = [0u8; 32];
        array.copy_from_slice(&bytes);
        VerifyingKey::from_bytes(&array).map_err(|_| KeyError::InvalidFormat)
    }

    pub fn signing_key(&self) -> Result<SigningKey, KeyError> {
        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &self.private_key,
        )
        .map_err(|_| KeyError::InvalidFormat)?;

        if bytes.len() != 32 {
            return Err(KeyError::InvalidFormat);
        }

        let mut array = [0u8; 32];
        array.copy_from_slice(&bytes);
        Ok(SigningKey::from_bytes(&array))
    }

    pub fn sign(&self, data: &[u8]) -> Result<Signature, KeyError> {
        let signing_key = self.signing_key()?;
        Ok(signing_key.sign(data))
    }

    pub fn verify(&self, data: &[u8], signature: &str) -> Result<bool, KeyError> {
        let verifying_key = self.verifying_key()?;
        let sig_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, signature)
                .map_err(|_| KeyError::InvalidFormat)?;

        if sig_bytes.len() != 64 {
            return Err(KeyError::InvalidFormat);
        }

        let mut array = [0u8; 64];
        array.copy_from_slice(&sig_bytes);
        let sig = Signature::from_bytes(&array);

        Ok(verifying_key.verify(data, &sig).is_ok())
    }

    pub fn fingerprint(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.public_key.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }
}

pub struct PeerKeyManager {
    key_path: PathBuf,
    keypair: Option<Ed25519KeyPair>,
}

impl PeerKeyManager {
    pub fn new(data_dir: PathBuf) -> Self {
        let key_path = data_dir.join("peer_keypair.json");
        PeerKeyManager {
            key_path,
            keypair: None,
        }
    }

    pub fn load_or_generate(&mut self) -> Result<&Ed25519KeyPair, KeyError> {
        if let Some(ref kp) = self.keypair {
            return Ok(kp);
        }

        if self.key_path.exists() {
            self.keypair = Some(serde_json::from_str(&std::fs::read_to_string(
                &self.key_path,
            )?)?);
        } else {
            let kp = Ed25519KeyPair::generate();
            let serialized = serde_json::to_string_pretty(&kp)?;
            if let Some(parent) = self.key_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&self.key_path, serialized)?;
            self.keypair = Some(kp);
        }

        self.keypair.as_ref().ok_or(KeyError::NotFound)
    }

    pub fn public_key(&self) -> Option<&str> {
        self.keypair.as_ref().map(|kp| kp.public_key())
    }

    pub fn keypair(&self) -> Option<&Ed25519KeyPair> {
        self.keypair.as_ref()
    }
}

pub fn verify_signature(pubkey: &str, data: &[u8], signature: &str) -> Result<bool, KeyError> {
    let kp = Ed25519KeyPair {
        public_key: pubkey.to_string(),
        private_key: String::new(),
    };
    kp.verify(data, signature)
}

pub fn sign_data(privkey_bytes: &[u8], data: &[u8]) -> Result<String, KeyError> {
    if privkey_bytes.len() != 32 {
        return Err(KeyError::InvalidFormat);
    }

    let mut array = [0u8; 32];
    array.copy_from_slice(privkey_bytes);
    let signing_key = SigningKey::from_bytes(&array);
    let signature = signing_key.sign(data);

    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        signature.to_bytes(),
    ))
}
