//! Credential Vault — AES-256-GCM encrypted secret storage.
//!
//! Stores secrets in `~/.librefang/vault.enc`, with the master key sourced from
//! the OS keyring (Windows Credential Manager / macOS Keychain / Linux Secret Service)
//! or the `LIBREFANG_VAULT_KEY` env var for headless/CI environments.

use crate::{ExtensionError, ExtensionResult};
use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use serde::{Deserialize, Serialize};
// sha2 is used only in non-test keyring functions
#[cfg(not(test))]
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;
use tracing::{debug, info, warn};
// `error!` is used only in non-test keyring code paths.
#[cfg(not(test))]
use tracing::error;
use zeroize::Zeroizing;

/// Env var fallback for vault key.
const VAULT_KEY_ENV: &str = "LIBREFANG_VAULT_KEY";

/// Service name used by the legacy v1 XOR-obfuscated keyring file as a salt
/// in the unmasking hash. Must remain stable across targets so v1 → v2
/// migrations keep working on every platform we ever ran on. The OS-keyring
/// backend's own service/user constants live in the `os_keyring` module.
#[cfg(not(test))]
const KEYRING_SERVICE: &str = "librefang-vault";

/// OS keyring backend abstraction. The real impl is only compiled on
/// targets where the `keyring` crate has a usable backend (glibc Linux,
/// macOS, Windows). On musl Linux, Android, and other targets the crate
/// itself isn't pulled — see Cargo.toml — so we provide a stub that
/// always reports unavailability. Callers fall through to the
/// AES-256-GCM file-based store either way.
#[cfg(all(
    not(test),
    any(
        all(target_os = "linux", not(target_env = "musl")),
        target_os = "macos",
        target_os = "windows",
    )
))]
mod os_keyring {
    const SERVICE: &str = "librefang-vault";
    // Each install stores a single master key per host; `Entry` needs a
    // username field so we use a fixed sentinel.
    const USER: &str = "master-key";

    /// Returns true if the key was stored in the OS keyring; false means
    /// the backend was unavailable / refused, and the caller should fall
    /// through to the file-based store. Backend errors are logged at
    /// debug and surfaced as `false` — never propagated.
    pub fn try_store(key_b64: &str) -> bool {
        match keyring::Entry::new(SERVICE, USER) {
            Ok(entry) => match entry.set_password(key_b64) {
                Ok(()) => true,
                Err(e) => {
                    tracing::debug!(
                        "OS keyring set_password failed ({e}) — falling back to file-based store"
                    );
                    false
                }
            },
            Err(e) => {
                tracing::debug!(
                    "OS keyring entry initialisation failed ({e}) — falling back to file-based store"
                );
                false
            }
        }
    }

    /// Returns Some(key) if found; None means no entry / backend
    /// unavailable, and the caller should try the file-based store.
    pub fn try_load() -> Option<String> {
        match keyring::Entry::new(SERVICE, USER) {
            Ok(entry) => match entry.get_password() {
                Ok(s) => Some(s),
                Err(keyring::Error::NoEntry) => None,
                Err(e) => {
                    tracing::debug!(
                        "OS keyring get_password failed ({e}) — trying file-based fallback"
                    );
                    None
                }
            },
            Err(_) => None,
        }
    }
}

#[cfg(all(
    not(test),
    not(any(
        all(target_os = "linux", not(target_env = "musl")),
        target_os = "macos",
        target_os = "windows",
    ))
))]
mod os_keyring {
    pub fn try_store(_key_b64: &str) -> bool {
        false
    }

    pub fn try_load() -> Option<String> {
        None
    }
}
/// Salt length for Argon2.
const SALT_LEN: usize = 16;
/// Nonce length for AES-256-GCM.
const NONCE_LEN: usize = 12;
/// Magic bytes for vault file format versioning.
const VAULT_MAGIC: &[u8; 4] = b"OFV1";

/// AAD schema version; 0 = legacy path-only AAD, 1 = schema_version_le_bytes || path_bytes.
const VAULT_SCHEMA_VERSION: u32 = 1;

/// On-disk vault format (encrypted).
#[derive(Serialize, Deserialize)]
struct VaultFile {
    /// File-format version marker (always 1; not the AAD schema version).
    version: u8,
    /// Argon2 salt (base64).
    salt: String,
    /// AES-256-GCM nonce (base64).
    nonce: String,
    /// Encrypted data (base64).
    ciphertext: String,
    /// AAD schema version; defaults to 0 on legacy files (path-only AAD compat).
    #[serde(default)]
    schema_version: u32,
}

/// Decrypted vault entries.
#[derive(Default, Serialize, Deserialize)]
struct VaultEntries {
    secrets: HashMap<String, String>,
}

/// AES-256-GCM encrypted credential vault.
pub struct CredentialVault {
    /// Path to vault.enc file.
    path: PathBuf,
    /// Decrypted entries (zeroed on drop via manual clearing).
    entries: HashMap<String, Zeroizing<String>>,
    /// Whether the vault is unlocked.
    unlocked: bool,
    /// Cached master key (zeroed on drop) — avoids re-resolving from env/keyring.
    cached_key: Option<Zeroizing<[u8; 32]>>,
}

impl CredentialVault {
    /// Create a new vault at the given path.
    pub fn new(vault_path: PathBuf) -> Self {
        Self {
            path: vault_path,
            entries: HashMap::new(),
            unlocked: false,
            cached_key: None,
        }
    }

    /// Initialize a new vault. Generates a master key and stores it in the OS keyring.
    pub fn init(&mut self) -> ExtensionResult<()> {
        if self.path.exists() {
            return Err(ExtensionError::Vault(
                "Vault already exists. Delete it first to re-initialize.".to_string(),
            ));
        }

        // Check if a master key is already available (env var or keyring)
        let key_bytes = if let Ok(existing_b64) = std::env::var(VAULT_KEY_ENV) {
            // Use the existing key from env var
            info!("Using existing vault key from {}", VAULT_KEY_ENV);
            decode_master_key(&existing_b64)?
        } else if let Ok(existing_b64) = load_keyring_key() {
            info!("Using existing vault key from OS keyring");
            decode_master_key(&existing_b64)?
        } else {
            // Generate a random master key
            let mut kb = Zeroizing::new([0u8; 32]);
            OsRng.fill_bytes(kb.as_mut());
            let key_b64 = Zeroizing::new(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                kb.as_ref(),
            ));

            // Try to store in OS keyring
            match store_keyring_key(&key_b64) {
                Ok(()) => {
                    info!("Vault master key stored in OS keyring");
                }
                Err(e) => {
                    warn!(
                        "Could not store vault key in OS keyring: {e}. \
                         Set {VAULT_KEY_ENV} env var manually. \
                         Use `librefang vault init` interactively to retrieve the key.",
                    );
                }
            }
            kb
        };

        // Create empty vault file
        self.entries.clear();
        self.unlocked = true;
        self.save(&key_bytes)?;
        self.cached_key = Some(key_bytes);
        info!("Credential vault initialized at {:?}", self.path);
        Ok(())
    }

    /// Unlock the vault by loading and decrypting entries.
    pub fn unlock(&mut self) -> ExtensionResult<()> {
        if self.unlocked {
            return Ok(());
        }
        if !self.path.exists() {
            return Err(ExtensionError::Vault(
                "Vault not initialized. Run `librefang vault init`.".to_string(),
            ));
        }

        let master_key = self.resolve_master_key()?;
        self.load(&master_key)?;
        self.unlocked = true;
        self.cached_key = Some(master_key);
        debug!("Vault unlocked with {} entries", self.entries.len());
        Ok(())
    }

    /// Get a secret from the vault.
    pub fn get(&self, key: &str) -> Option<Zeroizing<String>> {
        self.entries.get(key).cloned()
    }

    /// Store a secret in the vault.
    pub fn set(&mut self, key: String, value: Zeroizing<String>) -> ExtensionResult<()> {
        if !self.unlocked {
            return Err(ExtensionError::VaultLocked);
        }
        self.entries.insert(key, value);
        let master_key = self.resolve_master_key()?;
        self.save(&master_key)
    }

    /// Remove a secret from the vault.
    pub fn remove(&mut self, key: &str) -> ExtensionResult<bool> {
        if !self.unlocked {
            return Err(ExtensionError::VaultLocked);
        }
        let removed = self.entries.remove(key).is_some();
        if removed {
            let master_key = self.resolve_master_key()?;
            self.save(&master_key)?;
        }
        Ok(removed)
    }

    /// List all keys in the vault (not values).
    pub fn list_keys(&self) -> Vec<&str> {
        self.entries.keys().map(|k| k.as_str()).collect()
    }

    /// Check if the vault file exists.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Check if the vault is unlocked.
    pub fn is_unlocked(&self) -> bool {
        self.unlocked
    }

    /// Initialize a vault with an explicit master key (for testing / programmatic use).
    pub fn init_with_key(&mut self, master_key: Zeroizing<[u8; 32]>) -> ExtensionResult<()> {
        if self.path.exists() {
            return Err(ExtensionError::Vault(
                "Vault already exists. Delete it first to re-initialize.".to_string(),
            ));
        }
        self.entries.clear();
        self.unlocked = true;
        self.save(&master_key)?;
        self.cached_key = Some(master_key);
        debug!(
            "Credential vault initialized at {:?} (explicit key)",
            self.path
        );
        Ok(())
    }

    /// Unlock the vault with an explicit master key (for testing / programmatic use).
    pub fn unlock_with_key(&mut self, master_key: Zeroizing<[u8; 32]>) -> ExtensionResult<()> {
        if self.unlocked {
            return Ok(());
        }
        if !self.path.exists() {
            return Err(ExtensionError::Vault(
                "Vault not initialized. Run `librefang vault init`.".to_string(),
            ));
        }
        self.load(&master_key)?;
        self.unlocked = true;
        self.cached_key = Some(master_key);
        debug!(
            "Vault unlocked with {} entries (explicit key)",
            self.entries.len()
        );
        Ok(())
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the vault is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // ── Internal ─────────────────────────────────────────────────────────

    /// Resolve the master key from cache, keyring, or env var.
    fn resolve_master_key(&self) -> ExtensionResult<Zeroizing<[u8; 32]>> {
        // Use cached key if available (avoids env var race in parallel tests)
        if let Some(ref cached) = self.cached_key {
            return Ok(cached.clone());
        }

        // Try OS keyring first
        if let Ok(key_b64) = load_keyring_key() {
            return decode_master_key(&key_b64);
        }

        // Fallback to env var
        if let Ok(key_b64) = std::env::var(VAULT_KEY_ENV) {
            let key_b64 = Zeroizing::new(key_b64);
            return decode_master_key(&key_b64);
        }

        Err(ExtensionError::VaultLocked)
    }

    /// Returns `schema_version_le_bytes || path_bytes` as AES-GCM AAD.
    fn aad_bytes(path: &std::path::Path, schema_version: u32) -> Vec<u8> {
        let path_str = path.to_string_lossy();
        let path_bytes = path_str.as_bytes();
        let mut buf = Vec::with_capacity(4 + path_bytes.len());
        buf.extend_from_slice(&schema_version.to_le_bytes());
        buf.extend_from_slice(path_bytes);
        buf
    }

    /// Save encrypted vault to disk; AAD binds ciphertext to path + schema version.
    fn save(&self, master_key: &[u8; 32]) -> ExtensionResult<()> {
        // Serialize entries to JSON
        let plain_entries: HashMap<String, String> = self
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().to_string()))
            .collect();
        let vault_data = VaultEntries {
            secrets: plain_entries,
        };
        let plaintext = Zeroizing::new(
            serde_json::to_vec(&vault_data)
                .map_err(|e| ExtensionError::Vault(format!("Serialization failed: {e}")))?,
        );

        // Generate salt and nonce
        let mut salt = [0u8; SALT_LEN];
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut nonce_bytes);

        // Derive encryption key from master key + salt using Argon2
        let derived_key = derive_key(master_key, &salt)?;

        let cipher = Aes256Gcm::new_from_slice(derived_key.as_ref())
            .map_err(|e| ExtensionError::Vault(format!("Cipher init failed: {e}")))?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let aad = Self::aad_bytes(&self.path, VAULT_SCHEMA_VERSION);
        let ciphertext = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext.as_slice(),
                    aad: &aad,
                },
            )
            .map_err(|e| ExtensionError::Vault(format!("Encryption failed: {e}")))?;

        // Write to file
        let vault_file = VaultFile {
            version: 1,
            salt: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, salt),
            nonce: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, nonce_bytes),
            ciphertext: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &ciphertext,
            ),
            schema_version: VAULT_SCHEMA_VERSION,
        };
        let content = serde_json::to_string_pretty(&vault_file)
            .map_err(|e| ExtensionError::Vault(format!("Vault file serialization failed: {e}")))?;

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Prepend OFV1 magic bytes for format detection
        let mut output = Vec::with_capacity(VAULT_MAGIC.len() + content.len());
        output.extend_from_slice(VAULT_MAGIC);
        output.extend_from_slice(content.as_bytes());

        // Atomic write to .tmp (mode 0600 on Unix) then rename over target.
        let temp_path = self.path.with_extension("tmp");
        {
            let mut opts = OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&temp_path)?;
            f.write_all(&output)?;
            f.sync_all()?;
        }
        std::fs::rename(&temp_path, &self.path)?;
        // Enforce 0600 if a pre-existing file had looser perms.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&self.path) {
                let mut perms = meta.permissions();
                if perms.mode() & 0o777 != 0o600 {
                    perms.set_mode(0o600);
                    let _ = std::fs::set_permissions(&self.path, perms);
                }
            }
        }
        Ok(())
    }

    /// Load and decrypt vault from disk.
    ///
    /// The vault file path is passed as AAD to AES-GCM decrypt; this must
    /// match the path that was active when the ciphertext was produced.
    fn load(&mut self, master_key: &[u8; 32]) -> ExtensionResult<()> {
        let raw = std::fs::read(&self.path)?;

        // Strip OFV1 magic header if present; legacy JSON files start with '{'
        let content = if raw.starts_with(VAULT_MAGIC) {
            std::str::from_utf8(&raw[VAULT_MAGIC.len()..])
                .map_err(|e| ExtensionError::Vault(format!("UTF-8 decode failed: {e}")))?
        } else if raw.first() == Some(&b'{') {
            // Legacy JSON vault (no magic header)
            std::str::from_utf8(&raw)
                .map_err(|e| ExtensionError::Vault(format!("UTF-8 decode failed: {e}")))?
        } else {
            return Err(ExtensionError::Vault(
                "Unrecognized vault file format".to_string(),
            ));
        };

        let vault_file: VaultFile = serde_json::from_str(content)
            .map_err(|e| ExtensionError::Vault(format!("Vault file parse failed: {e}")))?;

        if vault_file.version != 1 {
            return Err(ExtensionError::Vault(format!(
                "Unsupported vault version: {}",
                vault_file.version
            )));
        }

        let salt =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &vault_file.salt)
                .map_err(|e| ExtensionError::Vault(format!("Salt decode failed: {e}")))?;
        let nonce_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &vault_file.nonce,
        )
        .map_err(|e| ExtensionError::Vault(format!("Nonce decode failed: {e}")))?;
        let ciphertext = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &vault_file.ciphertext,
        )
        .map_err(|e| ExtensionError::Vault(format!("Ciphertext decode failed: {e}")))?;

        // Derive key
        let derived_key = derive_key(master_key, &salt)?;

        // schema_version=0 on disk means path-only AAD (legacy compat); save() rewrites at v1.
        let cipher = Aes256Gcm::new_from_slice(derived_key.as_ref())
            .map_err(|e| ExtensionError::Vault(format!("Cipher init failed: {e}")))?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let aad: Vec<u8> = match vault_file.schema_version {
            0 => self.path.to_string_lossy().as_bytes().to_vec(),
            v if v == VAULT_SCHEMA_VERSION => Self::aad_bytes(&self.path, v),
            other => {
                return Err(ExtensionError::Vault(format!(
                    "Unsupported vault AAD schema version: {other}"
                )));
            }
        };
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    nonce,
                    Payload {
                        msg: ciphertext.as_slice(),
                        aad: &aad,
                    },
                )
                .map_err(|e| ExtensionError::Vault(format!("Decryption failed: {e}")))?,
        );

        // Parse entries
        let vault_data: VaultEntries = serde_json::from_slice(&plaintext)
            .map_err(|e| ExtensionError::Vault(format!("Vault data parse failed: {e}")))?;

        self.entries.clear();
        for (k, v) in vault_data.secrets {
            self.entries.insert(k, Zeroizing::new(v));
        }
        Ok(())
    }
}

impl Drop for CredentialVault {
    fn drop(&mut self) {
        // Zeroizing<String> handles zeroing individual values.
        // Clear the map to ensure all entries are dropped.
        self.entries.clear();
        self.cached_key = None;
        self.unlocked = false;
    }
}

/// Derive a 256-bit key from master key + salt using Argon2id.
fn derive_key(master_key: &[u8; 32], salt: &[u8]) -> ExtensionResult<Zeroizing<[u8; 32]>> {
    let mut derived = Zeroizing::new([0u8; 32]);
    Argon2::default()
        .hash_password_into(master_key, salt, derived.as_mut())
        .map_err(|e| ExtensionError::Vault(format!("Key derivation failed: {e}")))?;
    Ok(derived)
}

/// Decode a base64 master key into raw bytes.
fn decode_master_key(key_b64: &str) -> ExtensionResult<Zeroizing<[u8; 32]>> {
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, key_b64)
        .map_err(|e| ExtensionError::Vault(format!("Key decode failed: {e}")))?;
    if bytes.len() != 32 {
        return Err(ExtensionError::Vault(format!(
            "Invalid key length: expected 32, got {}",
            bytes.len()
        )));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// On-disk format for the file-based keyring fallback (v2, AES-256-GCM wrapped).
#[cfg(not(test))]
#[derive(Serialize, Deserialize)]
struct KeyringFile {
    /// Format version (2 = AES-256-GCM wrapped).
    version: u8,
    /// Argon2id salt (base64).
    salt: String,
    /// AES-256-GCM nonce (base64).
    nonce: String,
    /// Encrypted master key (base64).
    ciphertext: String,
}

/// Store the master key in the OS keyring (libsecret on Linux, Keychain on
/// macOS, Credential Manager on Windows). Falls back to a file-based
/// AES-256-GCM wrapped store only when the OS keyring is genuinely
/// unavailable (e.g. headless Linux without a Secret Service daemon).
fn store_keyring_key(key_b64: &str) -> Result<(), String> {
    #[cfg(not(test))]
    {
        // Try the OS keyring first. The previous behaviour silently dropped
        // through to the file fallback even on hosts that had a working
        // keyring — see issue #3178.
        if os_keyring::try_store(key_b64) {
            debug!("Stored master key in OS keyring");
            return Ok(());
        }

        // File-based fallback — wraps the master key with AES-256-GCM using an
        // Argon2id-derived wrapping key from the machine fingerprint.
        let keyring_path = dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("librefang")
            .join(".keyring");
        std::fs::create_dir_all(keyring_path.parent().unwrap())
            .map_err(|e| format!("mkdir: {e}"))?;

        warn!(
            "OS keyring unavailable — falling back to file-based key storage at {:?}. \
             This is less secure than a real OS keyring.",
            keyring_path
        );

        // Derive a wrapping key from the machine fingerprint via Argon2id
        let machine_id = machine_fingerprint();
        let mut salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);

        let wrapping_key =
            derive_wrapping_key(&machine_id, &salt).map_err(|e| format!("kdf: {e}"))?;

        // Encrypt the master key with AES-256-GCM
        let cipher = Aes256Gcm::new_from_slice(wrapping_key.as_ref())
            .map_err(|e| format!("cipher init: {e}"))?;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, key_b64.as_bytes())
            .map_err(|e| format!("encrypt: {e}"))?;

        let keyring_file = KeyringFile {
            version: 2,
            salt: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, salt),
            nonce: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, nonce_bytes),
            ciphertext: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &ciphertext,
            ),
        };
        let content =
            serde_json::to_string_pretty(&keyring_file).map_err(|e| format!("json: {e}"))?;

        // Atomic write with mode 0600 on Unix; non-Unix relies on OS ACLs.
        let tmp_path = keyring_path.with_extension(format!("keyring.tmp.{}", std::process::id()));
        let result = (|| -> std::io::Result<()> {
            use std::io::Write as _;
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp_path)?;
            f.write_all(content.as_bytes())?;
            f.flush()?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp_path, &keyring_path)
        })();

        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(format!("write: {e}"));
        }

        // Enforce 0600 if destination pre-existed with looser perms.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&keyring_path) {
                let mut perms = meta.permissions();
                if perms.mode() & 0o777 != 0o600 {
                    perms.set_mode(0o600);
                    let _ = std::fs::set_permissions(&keyring_path, perms);
                }
            }
        }
        Ok(())
    }
    #[cfg(test)]
    {
        let _ = key_b64;
        Err("Keyring not available in tests".to_string())
    }
}

/// Load the master key, preferring the OS keyring and falling back to the
/// file-based AES-256-GCM wrapped store. Symmetric with `store_keyring_key`.
fn load_keyring_key() -> Result<Zeroizing<String>, String> {
    #[cfg(not(test))]
    {
        // OS keyring first (issue #3178). `try_load` returns None for both
        // "no entry" (normal on a host that previously stored to the file
        // fallback) and "backend unavailable" — both cases drop through
        // silently to the file path below.
        if let Some(s) = os_keyring::try_load() {
            debug!("Loaded master key from OS keyring");
            return Ok(Zeroizing::new(s));
        }

        let keyring_path = dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("librefang")
            .join(".keyring");
        if !keyring_path.exists() {
            return Err("Keyring file not found".to_string());
        }

        let content = std::fs::read_to_string(&keyring_path).map_err(|e| format!("read: {e}"))?;

        // Try v2 (JSON with AES-256-GCM wrapped key)
        if let Ok(keyring_file) = serde_json::from_str::<KeyringFile>(content.trim()) {
            if keyring_file.version != 2 {
                return Err(format!(
                    "Unsupported keyring file version: {}",
                    keyring_file.version
                ));
            }

            let salt = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                &keyring_file.salt,
            )
            .map_err(|e| format!("salt decode: {e}"))?;
            let nonce_bytes = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                &keyring_file.nonce,
            )
            .map_err(|e| format!("nonce decode: {e}"))?;
            let ciphertext = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                &keyring_file.ciphertext,
            )
            .map_err(|e| format!("ciphertext decode: {e}"))?;

            let machine_id = machine_fingerprint();
            let wrapping_key =
                derive_wrapping_key(&machine_id, &salt).map_err(|e| format!("kdf: {e}"))?;

            let cipher = Aes256Gcm::new_from_slice(wrapping_key.as_ref())
                .map_err(|e| format!("cipher init: {e}"))?;
            let nonce = Nonce::from_slice(&nonce_bytes);
            match cipher.decrypt(nonce, ciphertext.as_slice()) {
                Ok(plaintext) => {
                    let key_str = String::from_utf8(plaintext).map_err(|e| format!("utf8: {e}"))?;
                    return Ok(Zeroizing::new(key_str));
                }
                Err(_) => {
                    // #3634 backwards-compat: a vault written before the
                    // os-machine-id mix-in landed used the legacy username+
                    // hostname derivation. Try it once; on success we re-wrap
                    // with the new derivation so the upgrade is one-shot.
                    debug!(
                        "v2 keyring decrypt failed with current fingerprint; \
                         retrying with legacy username+hostname derivation \
                         for one-shot migration"
                    );
                    let legacy_id = legacy_predictable_fingerprint();
                    let legacy_key = derive_wrapping_key(&legacy_id, &salt)
                        .map_err(|e| format!("kdf legacy: {e}"))?;
                    let legacy_cipher = Aes256Gcm::new_from_slice(legacy_key.as_ref())
                        .map_err(|e| format!("cipher init legacy: {e}"))?;
                    let plaintext = legacy_cipher
                        .decrypt(nonce, ciphertext.as_slice())
                        .map_err(|e| format!("decrypt: {e}"))?;
                    let key_str = String::from_utf8(plaintext).map_err(|e| format!("utf8: {e}"))?;
                    // Re-wrap with the new fingerprint to migrate this vault
                    // off the recoverable derivation. Best-effort: a write
                    // failure leaves the vault on legacy derivation, which is
                    // no worse than before.
                    if let Err(e) = store_keyring_key(&key_str) {
                        warn!(
                            "Decrypted v2 keyring with legacy derivation but \
                             could not re-wrap with new derivation: {e}"
                        );
                    } else {
                        info!(
                            "Migrated v2 keyring wrap-key from legacy \
                             username+hostname to OS-machine-id derivation (#3634)"
                        );
                    }
                    return Ok(Zeroizing::new(key_str));
                }
            }
        }

        // Legacy v1 fallback: XOR-obfuscated format (base64-encoded XOR blob).
        // Migrate by re-storing with the new format after successful load.
        warn!(
            "Detected legacy XOR-obfuscated keyring file — migrating to AES-256-GCM wrapped format"
        );
        let obfuscated =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, content.trim())
                .map_err(|e| format!("legacy decode: {e}"))?;

        let machine_id = machine_fingerprint();
        let mut hasher = Sha256::new();
        hasher.update(&machine_id);
        hasher.update(KEYRING_SERVICE.as_bytes());
        let mask: Vec<u8> = hasher.finalize().to_vec();

        let key_bytes: Vec<u8> = obfuscated
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ mask[i % mask.len()])
            .collect();
        let key_str = String::from_utf8(key_bytes).map_err(|e| format!("legacy utf8: {e}"))?;

        // Re-store with proper encryption to auto-migrate
        if let Err(e) = store_keyring_key(&key_str) {
            warn!("Failed to migrate legacy keyring to v2 format: {e}");
        } else {
            info!("Successfully migrated keyring file to AES-256-GCM wrapped format");
        }

        Ok(Zeroizing::new(key_str))
    }
    #[cfg(test)]
    {
        Err("Keyring not available in tests".to_string())
    }
}

/// Return a stable, unpredictable 32-byte machine secret used as the input
/// to the Argon2id wrapping-key derivation for the file-based keyring fallback.
///
/// # Security design
/// The old implementation derived this value from `username + hostname`, which
/// is predictable to any local user and therefore provided no meaningful
/// protection against a local attacker reading the `.keyring` file.
///
/// This version stores a randomly-generated 32-byte value in a 0600-permissioned
/// file on first call. Subsequent calls read the same value so the wrapping key
/// is stable across restarts while still being unguessable.
///
/// If the file cannot be created (e.g. a read-only filesystem), we fall back to
/// the predictable username+hostname derivation and emit a warning — the same
/// degraded security as before, but only as a last resort.
#[cfg(not(test))]
fn machine_fingerprint() -> Vec<u8> {
    let fingerprint_path = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("librefang")
        .join(".machine-id");

    // Try to read an existing random machine-id.
    match std::fs::read(&fingerprint_path) {
        Ok(bytes) if bytes.len() == 32 => return bytes,
        Ok(bytes) => {
            // Length mismatch — most likely a partial-write crash from an
            // earlier non-atomic save, an external corruption, or a manual
            // truncate.  DO NOT regenerate: the random id used to wrap the
            // existing vault entries is gone either way, but overwriting
            // the file makes any chance of recovery (restore from backup)
            // also impossible.  Fall back to the predictable derivation;
            // operators get a hard error decrypting the existing vault and
            // can restore the file from backup.
            error!(
                "machine-id file at {:?} has unexpected length ({} bytes, expected 32). \
                 NOT regenerating to preserve any chance of restoring from backup. \
                 The vault wrapping key cannot be reconstructed; restore the file or \
                 set LIBREFANG_VAULT_KEY to recover.",
                fingerprint_path,
                bytes.len()
            );
            return predictable_machine_fingerprint();
        }
        Err(_) => {
            // File does not exist (or unreadable) — fall through to create.
        }
    }

    // Generate a fresh random 32-byte value.
    let mut random_id = [0u8; 32];
    OsRng.fill_bytes(&mut random_id);

    if let Some(parent) = fingerprint_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Atomic create:
    //   1. open <path>.tmp with O_EXCL | mode 0600 (Unix)
    //   2. write_all + flush + sync_all
    //   3. rename(tmp, final) — atomic on POSIX
    // Locking the perms at `open` time closes the TOCTOU window where a
    // separate `set_permissions` call would briefly expose the secret at
    // umask defaults.  `O_EXCL` makes the first-run race deterministic:
    // if two daemons start concurrently, only one wins the create; the
    // loser falls back to reading the winner's file.
    let tmp_path =
        fingerprint_path.with_extension(format!("machine-id.tmp.{}", std::process::id()));
    let persisted = (|| -> std::io::Result<()> {
        use std::io::Write as _;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp_path)?;
        f.write_all(&random_id)?;
        f.flush()?;
        f.sync_all()?;
        drop(f);
        // rename is atomic; if another daemon got there first the source
        // tmp still exists from this caller's PID, so cleanup on failure
        // below removes it.
        std::fs::rename(&tmp_path, &fingerprint_path)
    })();

    match persisted {
        Ok(()) => {
            warn!(
                "OS keyring unavailable — generated random machine-id for keyring fallback at {:?}. \
                 This file must not be deleted; losing it makes the vault unrecoverable.",
                fingerprint_path
            );
            random_id.to_vec()
        }
        Err(e) => {
            // Clean up any abandoned tmp.
            let _ = std::fs::remove_file(&tmp_path);
            // Either the destination already exists (lost the create_new
            // race against another daemon), or persist genuinely failed.
            // Re-read once: if the contents are 32 bytes, use them — this
            // is the lost-race path.
            if let Ok(bytes) = std::fs::read(&fingerprint_path) {
                if bytes.len() == 32 {
                    debug!(
                        "Lost machine-id create_new race; using the file written by the winning process at {:?}",
                        fingerprint_path
                    );
                    return bytes;
                }
            }
            warn!(
                "Could not persist machine-id for keyring fallback ({e}): \
                 falling back to predictable username+hostname derivation. \
                 Set LIBREFANG_VAULT_KEY for a secure alternative."
            );
            predictable_machine_fingerprint()
        }
    }
}

/// Predictable fallback derivation used when no random machine-id can be
/// persisted. #3634: the original derivation used only `SHA256(username ||
/// hostname)`, which a local attacker recovers offline by reading `whoami`
/// and `hostname`. We now mix in an OS-provided machine-unique identifier
/// (`/etc/machine-id` on Linux, ioreg / WMI on macOS / Windows) so the
/// fingerprint cannot be reconstructed from public-facing fields alone.
/// On systems where none of the OS sources are readable we still fall back
/// to username+hostname and emit a warning — that path is no worse than the
/// pre-#3634 baseline, but operators should set `LIBREFANG_VAULT_KEY`.
#[cfg(not(test))]
fn predictable_machine_fingerprint() -> Vec<u8> {
    use sha2::Digest;
    let mut hasher = Sha256::new();
    let mut had_os_secret = false;
    for secret in os_machine_secrets() {
        if !secret.is_empty() {
            hasher.update(b"os-secret:");
            hasher.update(&secret);
            had_os_secret = true;
        }
    }
    if !had_os_secret {
        warn!(
            "No OS machine-id source available — keyring fallback wrap-key \
             will be derived from hostname+username only. Set \
             LIBREFANG_VAULT_KEY for a secure alternative."
        );
    }
    if let Ok(user) = std::env::var("USERNAME").or_else(|_| std::env::var("USER")) {
        hasher.update(b"user:");
        hasher.update(user.as_bytes());
    }
    if let Ok(host) = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")) {
        hasher.update(b"host:");
        hasher.update(host.as_bytes());
    }
    hasher.update(b"librefang-vault-v2");
    hasher.finalize().to_vec()
}

/// Pre-#3634 derivation: SHA256(user || host || tag). Kept ONLY so that vaults
/// written before the OS-machine-id mix-in can still be decrypted on first
/// load and then re-wrapped with the new derivation. Never used for new
/// stores.
#[cfg(not(test))]
fn legacy_predictable_fingerprint() -> Vec<u8> {
    use sha2::Digest;
    let mut hasher = Sha256::new();
    if let Ok(user) = std::env::var("USERNAME").or_else(|_| std::env::var("USER")) {
        hasher.update(user.as_bytes());
    }
    if let Ok(host) = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")) {
        hasher.update(host.as_bytes());
    }
    hasher.update(b"librefang-vault-v1");
    hasher.finalize().to_vec()
}

/// Collect OS-provided machine-unique identifiers. Returns the first non-empty
/// readable source per platform; multiple are tried so a missing file on one
/// distro/kernel doesn't fail closed. None of these are secrets at the OS
/// level, but they are not derivable from `whoami`/`hostname` and require
/// local-read access to the host.
#[cfg(not(test))]
fn os_machine_secrets() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    #[cfg(target_os = "linux")]
    {
        for path in &["/etc/machine-id", "/var/lib/dbus/machine-id"] {
            if let Ok(s) = std::fs::read_to_string(path) {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.as_bytes().to_vec());
                    break;
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if let Some(idx) = line.find("IOPlatformUUID") {
                        if let Some(eq) = line[idx..].find('=') {
                            let v = line[idx + eq + 1..].trim().trim_matches('"');
                            if !v.is_empty() {
                                out.push(v.as_bytes().to_vec());
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        // MachineGuid lives under HKLM\SOFTWARE\Microsoft\Cryptography. Read
        // via `reg query` to avoid pulling in a winreg dependency just for
        // the fallback path.
        if let Ok(output) = std::process::Command::new("reg")
            .args([
                "query",
                "HKLM\\SOFTWARE\\Microsoft\\Cryptography",
                "/v",
                "MachineGuid",
            ])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if let Some(idx) = line.find("REG_SZ") {
                        let v = line[idx + "REG_SZ".len()..].trim();
                        if !v.is_empty() {
                            out.push(v.as_bytes().to_vec());
                            break;
                        }
                    }
                }
            }
        }
    }
    out
}

/// Derive a 256-bit wrapping key from a machine fingerprint + salt using Argon2id.
#[cfg(not(test))]
fn derive_wrapping_key(fingerprint: &[u8], salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, String> {
    let mut derived = Zeroizing::new([0u8; 32]);
    Argon2::default()
        .hash_password_into(fingerprint, salt, derived.as_mut())
        .map_err(|e| format!("Argon2 key derivation failed: {e}"))?;
    Ok(derived)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_vault() -> (tempfile::TempDir, CredentialVault) {
        let dir = tempfile::tempdir().unwrap();
        let vault_path = dir.path().join("vault.enc");
        let vault = CredentialVault::new(vault_path);
        (dir, vault)
    }

    /// Generate a random 32-byte master key for tests.
    fn random_key() -> Zeroizing<[u8; 32]> {
        let mut kb = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(kb.as_mut());
        kb
    }

    #[test]
    fn vault_init_and_roundtrip() {
        let (dir, mut vault) = test_vault();
        let key = random_key();

        // Init creates vault file
        vault.init_with_key(key.clone()).unwrap();
        assert!(vault.exists());
        assert!(vault.is_unlocked());
        assert!(vault.is_empty());

        // Store a secret
        vault
            .set(
                "GITHUB_TOKEN".to_string(),
                Zeroizing::new("ghp_test123".to_string()),
            )
            .unwrap();
        assert_eq!(vault.len(), 1);

        // Read it back
        let val = vault.get("GITHUB_TOKEN").unwrap();
        assert_eq!(val.as_str(), "ghp_test123");

        // New vault instance, unlock with same key
        let mut vault2 = CredentialVault::new(dir.path().join("vault.enc"));
        vault2.unlock_with_key(key).unwrap();
        let val2 = vault2.get("GITHUB_TOKEN").unwrap();
        assert_eq!(val2.as_str(), "ghp_test123");

        // Remove
        assert!(vault2.remove("GITHUB_TOKEN").unwrap());
        assert!(vault2.get("GITHUB_TOKEN").is_none());
    }

    #[test]
    fn vault_list_keys() {
        let (_dir, mut vault) = test_vault();
        let key = random_key();

        vault.init_with_key(key).unwrap();
        vault
            .set("A".to_string(), Zeroizing::new("1".to_string()))
            .unwrap();
        vault
            .set("B".to_string(), Zeroizing::new("2".to_string()))
            .unwrap();

        let mut keys = vault.list_keys();
        keys.sort();
        assert_eq!(keys, vec!["A", "B"]);
    }

    #[test]
    fn vault_wrong_key_fails() {
        let (dir, mut vault) = test_vault();
        let good_key = random_key();

        vault.init_with_key(good_key).unwrap();
        vault
            .set("SECRET".to_string(), Zeroizing::new("value".to_string()))
            .unwrap();

        // Wrong key — should fail to decrypt
        let bad_key = random_key();
        let mut vault2 = CredentialVault::new(dir.path().join("vault.enc"));
        assert!(vault2.unlock_with_key(bad_key).is_err());
    }

    #[test]
    fn derive_key_deterministic() {
        let master = [42u8; 32];
        let salt = [1u8; 16];
        let k1 = derive_key(&master, &salt).unwrap();
        let k2 = derive_key(&master, &salt).unwrap();
        assert_eq!(k1.as_ref(), k2.as_ref());
    }

    #[test]
    fn vault_file_has_magic_header() {
        let (_dir, mut vault) = test_vault();
        let key = random_key();
        vault.init_with_key(key).unwrap();

        let raw = std::fs::read(&vault.path).unwrap();
        assert_eq!(&raw[..4], b"OFV1");
    }

    #[test]
    fn vault_legacy_json_compat() {
        let (dir, mut vault) = test_vault();
        let key = random_key();
        vault.init_with_key(key.clone()).unwrap();
        vault
            .set("KEY".to_string(), Zeroizing::new("val".to_string()))
            .unwrap();

        // Strip the OFV1 magic header to simulate a legacy vault file
        let raw = std::fs::read(&vault.path).unwrap();
        assert_eq!(&raw[..4], b"OFV1");
        std::fs::write(&vault.path, &raw[4..]).unwrap();

        // Should still load (legacy compat) — the path (AAD) is unchanged so
        // the GCM tag remains valid even without the magic prefix.
        let mut vault2 = CredentialVault::new(dir.path().join("vault.enc"));
        vault2.unlock_with_key(key).unwrap();
        assert_eq!(vault2.get("KEY").unwrap().as_str(), "val");
    }

    #[test]
    fn vault_rejects_bad_magic() {
        let (dir, mut vault) = test_vault();
        let key = random_key();
        vault.init_with_key(key.clone()).unwrap();

        // Overwrite with unrecognized binary data
        std::fs::write(&vault.path, b"BAAD not json").unwrap();

        let mut vault2 = CredentialVault::new(dir.path().join("vault.enc"));
        let result = vault2.unlock_with_key(key);
        assert!(result.is_err());
        let msg = format!("{:?}", result.unwrap_err());
        assert!(msg.contains("Unrecognized vault file format"));
    }

    /// Regression test for #3788: a file with schema_version=0 (legacy
    /// path-only AAD) must still decrypt with the path-only AAD path so
    /// pre-#3788 vaults keep working after upgrade.
    #[test]
    fn vault_legacy_schema_version_zero_decrypts_with_path_only_aad() {
        use aes_gcm::aead::{Aead, KeyInit, Payload};
        use aes_gcm::Aes256Gcm;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let key = random_key();

        // Hand-roll a legacy v0 ciphertext blob using the path as the only
        // AAD (the pre-#3788 layout) so we exercise the compat decode
        // branch without depending on git history.
        let plain = serde_json::to_vec(&VaultEntries {
            secrets: {
                let mut m = HashMap::new();
                m.insert("LEGACY_KEY".to_string(), "legacy_value".to_string());
                m
            },
        })
        .unwrap();

        let mut salt = [0u8; SALT_LEN];
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut nonce_bytes);
        let derived = derive_key(&key, &salt).unwrap();
        let cipher = Aes256Gcm::new_from_slice(derived.as_ref()).unwrap();
        let path_only_aad = path.to_string_lossy();
        let ct = cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: plain.as_slice(),
                    aad: path_only_aad.as_bytes(),
                },
            )
            .unwrap();

        let legacy_file = VaultFile {
            version: 1,
            salt: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, salt),
            nonce: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, nonce_bytes),
            ciphertext: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &ct),
            schema_version: 0, // ← legacy
        };
        let json = serde_json::to_string(&legacy_file).unwrap();
        let mut out = Vec::with_capacity(VAULT_MAGIC.len() + json.len());
        out.extend_from_slice(VAULT_MAGIC);
        out.extend_from_slice(json.as_bytes());
        std::fs::write(&path, &out).unwrap();

        let mut vault = CredentialVault::new(path);
        vault.unlock_with_key(key).unwrap();
        assert_eq!(vault.get("LEGACY_KEY").unwrap().as_str(), "legacy_value");
    }

    /// Regression test for #3788: a file written at schema_version=1 must
    /// fail to decrypt if an attacker downgrades the field to 0 because
    /// the AAD recorded at encryption time included the version bytes.
    #[test]
    fn vault_rejects_schema_version_downgrade() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let key = random_key();

        let mut vault = CredentialVault::new(path.clone());
        vault.init_with_key(key.clone()).unwrap();
        vault
            .set("K".to_string(), Zeroizing::new("v".to_string()))
            .unwrap();
        drop(vault);

        // Tamper with schema_version on disk: 1 → 0
        let raw = std::fs::read(&path).unwrap();
        let json_start = VAULT_MAGIC.len();
        let mut file: VaultFile = serde_json::from_slice(&raw[json_start..]).unwrap();
        assert_eq!(file.schema_version, VAULT_SCHEMA_VERSION);
        file.schema_version = 0;
        let mut tampered = Vec::with_capacity(raw.len());
        tampered.extend_from_slice(VAULT_MAGIC);
        tampered.extend_from_slice(serde_json::to_string_pretty(&file).unwrap().as_bytes());
        std::fs::write(&path, &tampered).unwrap();

        let mut vault2 = CredentialVault::new(path);
        let res = vault2.unlock_with_key(key);
        assert!(
            res.is_err(),
            "schema_version downgrade must fail authentication"
        );
    }

    /// Regression test for #3724: vault.enc must be created with mode 0600
    /// on Unix so the encrypted blob is never world-readable.
    #[cfg(unix)]
    #[test]
    fn vault_file_is_chmod_0600() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, mut vault) = test_vault();
        let key = random_key();
        vault.init_with_key(key).unwrap();
        let mode = std::fs::metadata(&vault.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "vault.enc must be 0600, got {mode:o}");
    }

    /// Regression test for #3788: copying a vault file to a different path
    /// must fail decryption because the AES-GCM AAD (vault path) no longer
    /// matches the path embedded at encryption time.
    #[test]
    fn vault_path_binding_rejects_file_swap() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();

        let path_a = dir_a.path().join("vault.enc");
        let path_b = dir_b.path().join("vault.enc");

        let key = random_key();

        // Create and populate vault at path_a
        let mut vault_a = CredentialVault::new(path_a.clone());
        vault_a.init_with_key(key.clone()).unwrap();
        vault_a
            .set(
                "TOKEN".to_string(),
                Zeroizing::new("secret_value".to_string()),
            )
            .unwrap();

        // Copy the raw vault bytes to path_b (simulating a file-swap attack)
        std::fs::copy(&path_a, &path_b).unwrap();

        // Opening path_b with the same key and the *same* path as path_a would
        // succeed (same AAD). Opening it as path_b must fail because path_b was
        // not the path used during encryption.
        let mut vault_b = CredentialVault::new(path_b);
        let result = vault_b.unlock_with_key(key);
        assert!(
            result.is_err(),
            "Decryption of a vault file at a swapped path must fail (AAD mismatch)"
        );
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("Decryption failed"),
            "Expected 'Decryption failed' error, got: {msg}"
        );
    }
}
