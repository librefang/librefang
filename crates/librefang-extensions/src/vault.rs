//! Credential Vault — AES-256-GCM encrypted secret storage.
//!
//! Stores secrets in `~/.librefang/vault.enc`, with the master key sourced from
//! the OS keyring (Windows Credential Manager / macOS Keychain / Linux Secret Service)
//! or the `LIBREFANG_VAULT_KEY` env var for headless/CI environments.

use crate::{ExtensionError, ExtensionResult};
use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
// Sha256 is used only in non-test keyring code (v1 XOR migration + predictable fallback).
// Sha512 is imported locally in mix_fingerprint_sources so it compiles in test builds too.
#[cfg(not(test))]
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::OnceLock;
use tracing::{debug, info, warn};
// `error!` is used only in non-test keyring code paths.
#[cfg(not(test))]
use tracing::error;
use zeroize::Zeroizing;

/// Env var fallback for vault key.
const VAULT_KEY_ENV: &str = "LIBREFANG_VAULT_KEY";

/// Reserved vault key for the startup-validated sentinel (#3651).
///
/// The sentinel is a known plaintext written under this key when a vault is
/// first created (or backfilled on the first open of a legacy vault). Every
/// subsequent unlock reads the sentinel and verifies its plaintext matches
/// `SENTINEL_VALUE`; any mismatch means the unlocking key does not match the
/// key the vault was originally encrypted with, and the boot path refuses to
/// start so encrypted credentials are not silently lost.
///
/// Reserved namespace: callers MUST NOT write to this key directly. The
/// `__sentinel__` prefix matches the pattern used by other reserved internal
/// keys (TOTP recovery codes use `_recovery_codes`, totp_confirmed, …).
pub const SENTINEL_KEY: &str = "__sentinel__";

/// Versioned sentinel plaintext (#3651). The `v1` suffix lets future format
/// migrations detect the sentinel scheme — bumping the version in this
/// constant without touching the verification logic would intentionally fail
/// the comparison and force a migration code path.
pub const SENTINEL_VALUE: &str = "librefang-vault-sentinel-v1";

/// Env var that, when set to `1` or `true` (case-insensitive), forces the
/// vault to skip the OS keyring entirely and use the AES-256-GCM-wrapped
/// file fallback at `<data_local_dir>/librefang/.keyring` (mode 0600).
///
/// Useful on macOS where the Keychain ACL is bound to the per-binary code
/// signature: every fresh `cargo build` invalidates the ACL and triggers
/// another "allow" prompt on daemon restart. The file fallback provides
/// equivalent at-rest security in our threat model (the wrap key is
/// derived from a per-machine fingerprint via Argon2id).
const VAULT_NO_KEYRING_ENV: &str = "LIBREFANG_VAULT_NO_KEYRING";

/// Process-global override for `use_os_keyring`. Set once via
/// [`CredentialVault::init_with_config`] (called from
/// `LibreFangKernel::boot_with_config`) and read by every vault
/// operation thereafter. `None` (unset) means "fall back to the platform
/// default". The env var `LIBREFANG_VAULT_NO_KEYRING` always wins
/// regardless of this value.
static CONFIG_USE_OS_KEYRING: OnceLock<bool> = OnceLock::new();

/// The compiled-in default for `use_os_keyring` when neither the env var
/// nor the config has set it.
///
/// macOS Keychain ACL is per-binary signature; a fresh `cargo build`
/// invalidates the ACL → prompt fatigue on every daemon restart. The
/// file-based fallback at `<data_local_dir>/librefang/.keyring` (0600,
/// AES-256-GCM wrapped with an Argon2id-derived machine-fingerprint key)
/// provides equivalent at-rest security in our threat model. Linux and
/// Windows keyrings have stable ACLs (D-Bus / Credential Manager don't
/// pin the calling binary) so they remain opt-out by default.
pub const fn default_use_os_keyring_for_platform() -> bool {
    !cfg!(target_os = "macos")
}

/// Resolve "should we use the OS keyring right now?".
///
/// Priority: env var > process-global config > platform default.
fn should_use_os_keyring() -> bool {
    if let Ok(v) = std::env::var(VAULT_NO_KEYRING_ENV) {
        let v = v.trim();
        if v == "1" || v.eq_ignore_ascii_case("true") {
            return false;
        }
    }
    if let Some(&v) = CONFIG_USE_OS_KEYRING.get() {
        return v;
    }
    default_use_os_keyring_for_platform()
}

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
        if !super::should_use_os_keyring() {
            tracing::debug!("OS keyring disabled by config / env var — using file-based store");
            return false;
        }
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
    /// unavailable / disabled by config, and the caller should try the
    /// file-based store.
    pub fn try_load() -> Option<String> {
        if !super::should_use_os_keyring() {
            return None;
        }
        try_load_force()
    }

    /// Same as [`try_load`] but bypasses the `use_os_keyring` config gate.
    ///
    /// Used exactly once per host to migrate an existing keyring entry
    /// (e.g. a macOS Keychain item from a previous LibreFang version)
    /// into the file-based store after the user has flipped the new
    /// macOS default. May trigger the OS prompt one final time.
    pub fn try_load_force() -> Option<String> {
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

    pub fn try_load_force() -> Option<String> {
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

    /// Set the process-global "use OS keyring" flag.
    ///
    /// First call wins — subsequent calls are silently ignored (the
    /// underlying `OnceLock` is one-shot). The
    /// `LIBREFANG_VAULT_NO_KEYRING` env var always overrides this value
    /// at read time.
    ///
    /// Call this exactly once, very early, from the daemon's startup
    /// path (currently `LibreFangKernel::boot_with_config`). Standalone
    /// CLI subcommands that bypass kernel boot (`librefang vault …`)
    /// don't need to call it — `should_use_os_keyring()` falls back to
    /// the platform default when this is unset.
    pub fn set_use_os_keyring(use_keyring: bool) {
        let _ = CONFIG_USE_OS_KEYRING.set(use_keyring);
    }

    /// Apply a [`librefang_types::config::VaultConfig`] to the
    /// process-global vault state. Convenience wrapper around
    /// [`Self::set_use_os_keyring`] that resolves `None` to the
    /// platform default.
    pub fn init_with_config(use_os_keyring: Option<bool>) {
        Self::set_use_os_keyring(
            use_os_keyring.unwrap_or_else(default_use_os_keyring_for_platform),
        );
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

        // Create empty vault file with the startup sentinel pre-written
        // (#3651). Doing it here means every fresh vault is born with the
        // sentinel — only legacy vaults from before this commit will need
        // backfill at unlock time.
        self.entries.clear();
        self.entries.insert(
            SENTINEL_KEY.to_string(),
            Zeroizing::new(SENTINEL_VALUE.to_string()),
        );
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

    /// Iterate over every (key, value) pair, including reserved internal
    /// keys. Used by the `librefang vault rotate-key` workflow which has to
    /// re-encrypt the sentinel along with every user-facing entry.
    pub fn iter_all_entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Re-encrypt the entire vault under a new master key (#3651 rotate-key).
    ///
    /// Preconditions:
    /// - The vault is currently unlocked with the OLD master key.
    /// - The OLD vault file exists at `self.path`.
    ///
    /// The implementation writes the re-encrypted vault to `<path>.new`
    /// first (atomic temp file), `fsync`s it, then renames over the
    /// original. The sentinel is deliberately re-written so the post-
    /// rotation vault still verifies on next boot under the new key.
    ///
    /// Caller is responsible for:
    /// - Validating that `new_master_key` is a 32-byte base64 value
    ///   (use [`crate::vault::decode_master_key`] via the public CLI
    ///   wrapper). Garbage-in, garbage-out otherwise.
    /// - Persisting the new key (env var, OS keyring, file fallback) so
    ///   the daemon can read it on next boot.
    pub fn rewrap_with_new_key(
        &mut self,
        new_master_key: Zeroizing<[u8; 32]>,
    ) -> ExtensionResult<()> {
        if !self.unlocked {
            return Err(ExtensionError::VaultLocked);
        }
        // Make sure the sentinel is present so the post-rotation vault
        // verifies on next boot. Idempotent — a sentinel-bearing vault
        // gets the same insert.
        self.entries.insert(
            SENTINEL_KEY.to_string(),
            Zeroizing::new(SENTINEL_VALUE.to_string()),
        );
        // `save()` already does an atomic write via `<path>.tmp` + rename,
        // mode 0600. Reusing it keeps the rotate-key path on the same
        // proven IO code path — there is no separate "rotate" disk format.
        self.save(&new_master_key)?;
        // The cached_key now reflects the new master key for any
        // subsequent ops on this vault instance (e.g. an immediate verify).
        self.cached_key = Some(new_master_key);
        Ok(())
    }

    /// Store a secret in the vault.
    ///
    /// Rejects writes to the reserved [`SENTINEL_KEY`] (#3651) so external
    /// callers cannot corrupt the startup-validation contract by overwriting
    /// the sentinel with arbitrary plaintext.
    ///
    /// Lazy-initialises a never-materialised vault on first write: when the
    /// caller holds an unopened handle (vault.enc does not exist on disk),
    /// the first `set()` call runs `init()` so the credential lands in a
    /// real, persisted vault instead of being silently dropped on the
    /// floor. This is the contract `kernel::vault_handle()`'s doc-comment
    /// has always promised; before this, `set()` returned `VaultLocked` on
    /// a fresh handle and `install_integration` swallowed the error while
    /// reporting `Ready` (refs #4788, #4791).
    pub fn set(&mut self, key: String, value: Zeroizing<String>) -> ExtensionResult<()> {
        // Reject the reserved sentinel slot BEFORE any side-effecting work
        // (#3651). Doing it first means a rejected write on a fresh handle
        // does not materialise vault.enc as a side effect of lazy-init —
        // a no-op call must remain a no-op on disk.
        if key == SENTINEL_KEY {
            return Err(ExtensionError::Vault(format!(
                "Refusing to write reserved key {SENTINEL_KEY}; this slot is owned by \
                 the vault startup-validation sentinel (#3651)."
            )));
        }
        if !self.unlocked && !self.path.exists() {
            self.init()?;
        }
        if !self.unlocked {
            return Err(ExtensionError::VaultLocked);
        }
        self.entries.insert(key, value);
        let master_key = self.resolve_master_key()?;
        self.save(&master_key)
    }

    /// Remove a secret from the vault.
    ///
    /// Rejects deletes of the reserved [`SENTINEL_KEY`] (#3651): removing the
    /// sentinel would silently regress to the pre-#3651 behaviour where a
    /// wrong-key boot is undetectable.
    pub fn remove(&mut self, key: &str) -> ExtensionResult<bool> {
        if !self.unlocked {
            return Err(ExtensionError::VaultLocked);
        }
        if key == SENTINEL_KEY {
            return Err(ExtensionError::Vault(format!(
                "Refusing to remove reserved key {SENTINEL_KEY}; this slot is owned by \
                 the vault startup-validation sentinel (#3651)."
            )));
        }
        let removed = self.entries.remove(key).is_some();
        if removed {
            let master_key = self.resolve_master_key()?;
            self.save(&master_key)?;
        }
        Ok(removed)
    }

    /// List all keys in the vault (not values).
    ///
    /// Reserved internal keys (e.g. the #3651 [`SENTINEL_KEY`]) are filtered
    /// out — callers should never see them via `list_keys` or be tempted to
    /// remove them. Use [`Self::list_keys_including_internal`] when the
    /// caller genuinely needs every entry (e.g. the rotate-key migration).
    pub fn list_keys(&self) -> Vec<&str> {
        self.entries
            .keys()
            .filter(|k| k.as_str() != SENTINEL_KEY)
            .map(|k| k.as_str())
            .collect()
    }

    /// List every key in the vault, including reserved internal keys.
    ///
    /// Used by the `librefang vault rotate-key` workflow which must
    /// re-encrypt the sentinel along with every user-facing entry so the
    /// post-rotation vault still verifies on next boot.
    pub fn list_keys_including_internal(&self) -> Vec<&str> {
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

    /// Verify (or backfill) the startup sentinel introduced in #3651.
    ///
    /// The vault MUST already be unlocked. Behavior by sentinel state:
    ///
    /// - **Absent** (legacy vault written before #3651, or fresh vault that
    ///   has not yet been initialized) → write [`SENTINEL_VALUE`] under
    ///   [`SENTINEL_KEY`]. One-time backfill; subsequent boots will see it
    ///   and take the verify branch.
    /// - **Present and matches [`SENTINEL_VALUE`]** → return `Ok(())`.
    /// - **Present but does not match** → return
    ///   [`ExtensionError::VaultKeyMismatch`]. This is structurally
    ///   impossible if the unlocking key is correct: AES-GCM authenticates
    ///   every entry, so a wrong key would have already failed at `unlock()`.
    ///   This branch only fires if a future format change writes a new
    ///   sentinel value (e.g. `v2`) — it forces an explicit migration
    ///   instead of a silent boot.
    ///
    /// This method must be called from the daemon boot path **after** every
    /// successful `unlock()` so a fresh-install / corrupt-vault distinction
    /// is enforced before any credential is read or written.
    pub fn verify_or_install_sentinel(&mut self) -> ExtensionResult<()> {
        if !self.unlocked {
            return Err(ExtensionError::VaultLocked);
        }
        match self.entries.get(SENTINEL_KEY) {
            Some(stored) if stored.as_str() == SENTINEL_VALUE => {
                debug!("Vault sentinel verified ({SENTINEL_VALUE})");
                Ok(())
            }
            Some(other) => {
                // Sentinel present but plaintext differs from the expected
                // value. This is NOT a wrong-key case (AES-GCM would have
                // failed at unlock); it means the sentinel scheme has been
                // upgraded out from under this binary. Refuse to boot rather
                // than mix v1 and v2 sentinels in the same vault file.
                let sample = other.as_str().chars().take(40).collect::<String>();
                Err(ExtensionError::VaultKeyMismatch {
                    hint: format!(
                        "vault sentinel value differs from expected ({SENTINEL_VALUE}); \
                         saw {sample:?}. The vault may have been written by a newer \
                         LibreFang release. Upgrade the daemon binary or restore the \
                         vault file from backup."
                    ),
                })
            }
            None => {
                // Legacy / fresh vault: write the sentinel so future boots
                // take the verify branch. Idempotent — re-running on a
                // post-backfill vault is a no-op via the matched branch.
                info!(
                    "Vault sentinel missing — backfilling {SENTINEL_VALUE} \
                     (one-time migration for vaults predating #3651)"
                );
                let master_key = self.resolve_master_key()?;
                self.entries.insert(
                    SENTINEL_KEY.to_string(),
                    Zeroizing::new(SENTINEL_VALUE.to_string()),
                );
                self.save(&master_key)
            }
        }
    }

    /// Initialize a vault with an explicit master key (for testing / programmatic use).
    pub fn init_with_key(&mut self, master_key: Zeroizing<[u8; 32]>) -> ExtensionResult<()> {
        if self.path.exists() {
            return Err(ExtensionError::Vault(
                "Vault already exists. Delete it first to re-initialize.".to_string(),
            ));
        }
        // Mirror `init()`: pre-write the #3651 sentinel so the boot-path
        // verify branch sees a valid sentinel without needing a separate
        // backfill pass.
        self.entries.clear();
        self.entries.insert(
            SENTINEL_KEY.to_string(),
            Zeroizing::new(SENTINEL_VALUE.to_string()),
        );
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

    /// Number of user-visible entries (excludes reserved internal keys
    /// such as the #3651 [`SENTINEL_KEY`] so a freshly-init'd vault still
    /// reports as empty to callers).
    pub fn len(&self) -> usize {
        self.entries
            .keys()
            .filter(|k| k.as_str() != SENTINEL_KEY)
            .count()
    }

    /// Whether the vault is empty (ignoring reserved internal keys).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    // ── Internal ─────────────────────────────────────────────────────────

    /// Resolve the master key from cache, keyring, or env var.
    fn resolve_master_key(&self) -> ExtensionResult<Zeroizing<[u8; 32]>> {
        // Use cached key if available (avoids env var race in parallel tests)
        if let Some(ref cached) = self.cached_key {
            return Ok(cached.clone());
        }

        // Env var wins over the keyring — matches `init()`'s precedence
        // (line ~279) so an explicit `LIBREFANG_VAULT_KEY` survives across
        // re-opens of the same vault. Without this, `init()` would honor
        // the env key but a subsequent `unlock()` on a fresh instance
        // would silently switch to whatever the (process-shared) keyring
        // file currently holds — which races between parallel tests
        // (#TOTP flake) and surprises CI/headless deployments that set
        // the env var as the source of truth.
        if let Ok(key_b64) = std::env::var(VAULT_KEY_ENV) {
            let key_b64 = Zeroizing::new(key_b64);
            return decode_master_key(&key_b64);
        }

        if let Ok(key_b64) = load_keyring_key() {
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
///
/// Public since #3651 so the `librefang vault rotate-key` CLI subcommand
/// can validate `LIBREFANG_VAULT_KEY_OLD` / `LIBREFANG_VAULT_KEY_NEW` (or
/// stdin-supplied values) using the exact same parser that production
/// `unlock()` uses — guarantees that "the CLI accepted my new key" implies
/// the daemon will accept it on next boot.
pub fn decode_master_key(key_b64: &str) -> ExtensionResult<Zeroizing<[u8; 32]>> {
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

/// On-disk format for the file-based keyring fallback.
///
/// Version history:
///   2 = AES-256-GCM wrapped, fingerprint derived from raw `random_id` (pre-#4159)
///   3 = AES-256-GCM wrapped, fingerprint derived from SHA-512(domain || random_id || os_material)
///
/// Reads accept both 2 and 3; writes always emit 3.  Version 2 files are
/// auto-migrated to version 3 on the first daemon restart after upgrade.
#[cfg(not(test))]
#[derive(Serialize, Deserialize)]
struct KeyringFile {
    /// Format version (2 = legacy raw-id fingerprint, 3 = mixed fingerprint).
    version: u8,
    /// Argon2id salt (base64).
    salt: String,
    /// AES-256-GCM nonce (base64).
    nonce: String,
    /// Encrypted master key (base64).
    ciphertext: String,
}

/// Atomically write `content` to `path` with mode 0600 on Unix.
/// Used by both `store_keyring_key` and the v2→v3 migration inside
/// `load_keyring_key` to avoid duplicating the atomic-rename logic.
#[cfg(not(test))]
fn write_keyring_file(path: &std::path::Path, content: &str) -> Result<(), String> {
    let tmp_path = path.with_extension(format!("keyring.tmp.{}", std::process::id()));
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
        std::fs::rename(&tmp_path, path)
    })();

    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!("write: {e}"));
    }

    // Enforce 0600 if destination pre-existed with looser perms.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            if perms.mode() & 0o777 != 0o600 {
                perms.set_mode(0o600);
                let _ = std::fs::set_permissions(path, perms);
            }
        }
    }
    Ok(())
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
        // keyring — see issue #3178. `os_keyring::try_store` itself
        // short-circuits to `false` when `use_os_keyring` is disabled
        // (config / env var / macOS platform default).
        if os_keyring::try_store(key_b64) {
            debug!("Stored master key in OS keyring");
            return Ok(());
        }

        if should_use_os_keyring() {
            // Keyring was requested but the backend refused / was missing
            // (e.g. headless Linux without a Secret Service daemon). Warn
            // loudly — at-rest security drops to the machine-fingerprint
            // wrap key. When the user has explicitly opted out (config /
            // env var / macOS default) the file store is the intended
            // path and no warning is emitted.
            warn!(
                "OS keyring unavailable — falling back to file-based key storage. \
                 This is less secure than a real OS keyring."
            );
        }
        store_keyring_key_to_file(key_b64)
    }
    #[cfg(test)]
    {
        let _ = key_b64;
        Err("Keyring not available in tests".to_string())
    }
}

/// Write the master key to the AES-256-GCM-wrapped file fallback only,
/// never touching the OS keyring. Used by:
///   - `store_keyring_key` after the OS keyring is unavailable / disabled.
///   - The macOS migration path in `load_keyring_key` to mirror an
///     existing Keychain entry into the file fallback so subsequent
///     loads never prompt again.
#[cfg(not(test))]
fn store_keyring_key_to_file(key_b64: &str) -> Result<(), String> {
    let keyring_path = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("librefang")
        .join(".keyring");
    std::fs::create_dir_all(keyring_path.parent().unwrap()).map_err(|e| format!("mkdir: {e}"))?;

    debug!(
        "Writing master key to file-based store at {:?}",
        keyring_path
    );

    // Derive a wrapping key from the machine fingerprint via Argon2id
    let machine_id = machine_fingerprint();
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);

    let wrapping_key = derive_wrapping_key(&machine_id, &salt).map_err(|e| format!("kdf: {e}"))?;

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
        version: 3,
        salt: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, salt),
        nonce: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, nonce_bytes),
        ciphertext: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &ciphertext),
    };
    let content = serde_json::to_string_pretty(&keyring_file).map_err(|e| format!("json: {e}"))?;

    write_keyring_file(&keyring_path, &content)
}

/// Load the master key, preferring the OS keyring and falling back to the
/// file-based AES-256-GCM wrapped store. Symmetric with `store_keyring_key`.
///
/// # Version migration
///
/// v2 keyrings (pre-#4159) derived the wrap key from `random_id` directly
/// (i.e. `Argon2id(random_id, salt)` with no SHA-512 mixing).  v3 keyrings
/// use `Argon2id(SHA-512(domain || random_id || os_material)[..32], salt)`.
///
/// We retain the v2 read path for one release cycle to allow auto-migration
/// on first daemon restart post-upgrade.  Plan to remove the v2 branch after
/// release N+2 (tracked in #4159 follow-up).
///
/// On a successful v2 decrypt the file is atomically re-wrapped as v3 so
/// subsequent loads take the fast v3 path.
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
            // Migration path: the user has just opted out of the OS
            // keyring (e.g. macOS default flipped to false) but their
            // master key still lives in the OS keyring from a previous
            // run. Read it ONE final time (this may trigger one last OS
            // prompt on macOS), then mirror it into the file fallback so
            // subsequent loads never touch the OS keyring again.
            //
            // `try_load_force` bypasses the `use_os_keyring` short-circuit
            // that just returned `None` above. If `use_os_keyring` is in
            // fact enabled, `try_load` already attempted this and failed,
            // so `try_load_force` will return `None` immediately and we
            // fall through to the missing-file error.
            if !should_use_os_keyring() {
                if let Some(s) = os_keyring::try_load_force() {
                    info!(
                        "Migrated master key from OS keyring to file-based store at {:?}; \
                         OS keyring will not be consulted again on this host. \
                         You may delete the now-unused entry manually \
                         (`security delete-generic-password -s librefang-vault -a master-key`).",
                        keyring_path
                    );
                    if let Err(e) = store_keyring_key_to_file(&s) {
                        warn!(
                            "Loaded master key from OS keyring but failed to mirror to file: {e}; \
                             OS keyring will be re-read on next start"
                        );
                    }
                    return Ok(Zeroizing::new(s));
                }
            }
            return Err("Keyring file not found".to_string());
        }

        let content = std::fs::read_to_string(&keyring_path).map_err(|e| format!("read: {e}"))?;

        // Try v2 or v3 (JSON with AES-256-GCM wrapped key)
        if let Ok(keyring_file) = serde_json::from_str::<KeyringFile>(content.trim()) {
            match keyring_file.version {
                3 => {
                    // v3: fingerprint = SHA-512(domain || random_id || os_material)[..32]
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
                    let plaintext = cipher
                        .decrypt(nonce, ciphertext.as_slice())
                        .map_err(|e| format!("decrypt: {e}"))?;
                    let key_str = String::from_utf8(plaintext).map_err(|e| format!("utf8: {e}"))?;
                    return Ok(Zeroizing::new(key_str));
                }
                2 => {
                    // v2: pre-#4159 legacy. Two sub-paths could have produced
                    // the wrap key:
                    //   (a) raw random_id from .machine-id  (`legacy_v2_fingerprint`)
                    //   (b) predictable hash on hosts where .machine-id could
                    //       not be persisted (`legacy_v2_predictable_fingerprint`).
                    // Try both — if (a) fails (read-only-FS install with no
                    // .machine-id) we still recover the vault via (b) instead
                    // of regressing to "unrecoverable" on upgrade.
                    warn!(
                        "Detected v2 keyring file (pre-#4159 format) — \
                         migrating to v3 (mixed fingerprint) on first load"
                    );

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

                    // Build the candidate list. `legacy_v2_fingerprint` may
                    // legitimately fail (no .machine-id on a read-only FS);
                    // fall through to the predictable fingerprint in that
                    // case. The predictable derivation reproduces the exact
                    // pre-#4159 SHA-256(os-secret || user || host || tag).
                    let raw = legacy_v2_fingerprint().ok();
                    let predictable = legacy_v2_predictable_fingerprint();
                    let mut candidates: Vec<&[u8]> = Vec::with_capacity(2);
                    if let Some(ref bytes) = raw {
                        candidates.push(bytes.as_slice());
                    }
                    candidates.push(predictable.as_slice());

                    let key_str =
                        try_decrypt_v2(&salt, &nonce_bytes, &ciphertext, &candidates).map_err(
                            |e| {
                                format!(
                                    "v2 keyring decrypt failed across all legacy derivations: {e}; \
                                     vault is unrecoverable — restore from backup or set LIBREFANG_VAULT_KEY"
                                )
                            },
                        )?;

                    // Re-wrap with v3 fingerprint and atomically replace the file.
                    if let Err(e) = store_keyring_key(&key_str) {
                        warn!("Failed to migrate keyring from v2 to v3 format: {e}");
                    } else {
                        info!(
                            "Successfully migrated keyring file from v2 to v3 (mixed fingerprint)"
                        );
                    }

                    return Ok(Zeroizing::new(key_str));
                }
                v => {
                    return Err(format!("Unsupported keyring file version: {v}"));
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
            warn!("Failed to migrate legacy keyring to v3 format: {e}");
        } else {
            info!("Successfully migrated keyring file to AES-256-GCM wrapped format (v3)");
        }

        Ok(Zeroizing::new(key_str))
    }
    #[cfg(test)]
    {
        Err("Keyring not available in tests".to_string())
    }
}

/// Return the v2 (pre-#4159) fingerprint for a given `.machine-id` file.
///
/// v2 keyrings used the raw random_id bytes directly as the Argon2id input —
/// no SHA-512 mixing, no OS material.  This helper reads the persisted
/// random_id and returns it verbatim so the v2 decrypt path in
/// `load_keyring_key` can reconstruct the exact wrap key that was used when
/// the file was originally written.
///
/// Returns `Err` if the `.machine-id` file is missing or has the wrong length
/// (in that case the vault is unrecoverable without a manual restore, same as
/// the production path for a corrupt machine-id file).
#[cfg(not(test))]
fn legacy_v2_fingerprint() -> Result<Vec<u8>, String> {
    let fingerprint_path = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("librefang")
        .join(".machine-id");
    let bytes = std::fs::read(&fingerprint_path)
        .map_err(|e| format!("legacy_v2_fingerprint: cannot read .machine-id: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!(
            "legacy_v2_fingerprint: .machine-id has unexpected length ({} bytes, expected 32); \
             vault is unrecoverable — restore the file from backup or set LIBREFANG_VAULT_KEY",
            bytes.len()
        ));
    }
    // v2 used random_id directly (no mixing).
    Ok(bytes)
}

/// Reproduce the pre-#4159 `os_machine_secrets()` first-non-empty result.
///
/// The pre-#4159 predictable fingerprint hashed exactly **one** OS source — the
/// first readable per platform, in the order Linux machine-id files / macOS
/// IOPlatformUUID / Windows MachineGuid. This helper exists solely so the
/// v2 → v3 migration can reconstruct a vault whose wrap key was derived
/// without a persisted `.machine-id` (read-only filesystem path). Never used
/// for new stores. See `legacy_v2_predictable_fingerprint` for the framing.
#[cfg(not(test))]
fn legacy_v2_first_os_secret() -> Option<Vec<u8>> {
    #[cfg(target_os = "linux")]
    {
        for path in &["/etc/machine-id", "/var/lib/dbus/machine-id"] {
            if let Ok(s) = std::fs::read_to_string(path) {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.as_bytes().to_vec());
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
                                return Some(v.as_bytes().to_vec());
                            }
                        }
                    }
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
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
                            return Some(v.as_bytes().to_vec());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Reproduce the pre-#4159 `predictable_machine_fingerprint()` derivation
/// **byte-for-byte** so that v2 keyrings written via the read-only-FS fallback
/// path (i.e. when no `.machine-id` could be persisted) can still be decrypted
/// and re-wrapped to v3.
///
/// Without this helper, the v2 → v3 migration only handles vaults whose wrap
/// key was derived from a persisted random_id; vaults derived from
/// `predictable_machine_fingerprint()` (containers with ephemeral rootfs)
/// would be unrecoverable on upgrade — a regression vs pre-#4159 behaviour.
/// Never used for new stores; only invoked by the v2 branch of
/// `load_keyring_key` after `legacy_v2_fingerprint()` fails.
#[cfg(not(test))]
fn legacy_v2_predictable_fingerprint() -> Vec<u8> {
    use sha2::Digest;
    let mut hasher = Sha256::new();
    if let Some(secret) = legacy_v2_first_os_secret() {
        hasher.update(b"os-secret:");
        hasher.update(&secret);
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

/// Pure-function v2 keyring decryption: try each candidate fingerprint in
/// order and return the first plaintext that decrypts cleanly.
///
/// Extracted from `load_keyring_key` so the dispatch logic can be exercised
/// directly in tests instead of structurally re-implemented. The function has
/// no filesystem side effects; callers inject the fingerprints they want
/// tried (raw random_id first, predictable fallback second) and receive
/// back the master key plaintext on success.
///
/// Returns `Err` only if every candidate fails.
fn try_decrypt_v2(
    salt: &[u8],
    nonce_bytes: &[u8],
    ciphertext: &[u8],
    candidates: &[&[u8]],
) -> Result<String, String> {
    if candidates.is_empty() {
        return Err("try_decrypt_v2: no candidate fingerprints supplied".to_string());
    }
    let mut last_err = String::from("try_decrypt_v2: all candidates exhausted");
    for fp in candidates {
        let wrapping_key = match derive_wrapping_key(fp, salt) {
            Ok(k) => k,
            Err(e) => {
                last_err = format!("kdf: {e}");
                continue;
            }
        };
        let cipher = match Aes256Gcm::new_from_slice(wrapping_key.as_ref()) {
            Ok(c) => c,
            Err(e) => {
                last_err = format!("cipher init: {e}");
                continue;
            }
        };
        let nonce = Nonce::from_slice(nonce_bytes);
        match cipher.decrypt(nonce, ciphertext) {
            Ok(plaintext) => {
                return String::from_utf8(plaintext).map_err(|e| format!("utf8: {e}"));
            }
            Err(e) => {
                last_err = format!("decrypt: {e}");
                continue;
            }
        }
    }
    Err(last_err)
}

/// Return a stable, unpredictable 32-byte machine secret used as the input
/// to the Argon2id wrapping-key derivation for the file-based keyring fallback.
///
/// # Security design
///
/// The fingerprint is built from two independent entropy sources combined with
/// SHA-512 via `mix_fingerprint_sources`, so compromise of either source alone
/// is insufficient to recover the wrapping key:
///
/// 1. **Persisted random id** — a 32-byte value generated by `OsRng` on first
///    call and stored in `~/.local/share/librefang/.machine-id` (mode 0600).
///    Provides 256 bits of install-unique entropy.
///
/// 2. **OS machine-id material** — collected by `collect_os_machine_id_material`:
///    - Linux: `/etc/machine-id` and/or `/var/lib/dbus/machine-id`.
///      `/proc/sys/kernel/random/boot_id` is used ONLY as a fallback when
///      neither persistent source is readable (e.g. minimal containers).
///    - macOS: `IOPlatformUUID` obtained via `ioreg -rd1 -c IOPlatformExpertDevice`.
///    - Windows: `MachineGuid` from `reg query HKLM\SOFTWARE\Microsoft\Cryptography`.
///
/// If no OS material can be read, only the persisted random id is used.
/// If the persisted random id cannot be created or read, we fall back to the
/// predictable username+hostname derivation via `predictable_machine_fingerprint`.
#[cfg(not(test))]
fn machine_fingerprint() -> Vec<u8> {
    let fingerprint_path = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("librefang")
        .join(".machine-id");

    // Collect OS machine-id material (may be empty if none are readable).
    let os_material = collect_os_machine_id_material();

    // Try to read an existing random machine-id.
    match std::fs::read(&fingerprint_path) {
        Ok(bytes) if bytes.len() == 32 => {
            // Mix: SHA-512(random_id || os_material), then take first 32 bytes.
            // This binds the wrapping key to BOTH the random id AND the OS
            // machine identity.  Losing either alone does not break security.
            return mix_fingerprint_sources(&bytes, &os_material);
        }
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
            mix_fingerprint_sources(&random_id, &os_material)
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
                    return mix_fingerprint_sources(&bytes, &os_material);
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

/// Collect all available OS-provided machine identity material into a single
/// byte buffer.
///
/// Each source is emitted with a fixed ASCII label and a 4-byte LE length
/// prefix before its bytes.  Labelled framing means:
/// - Adding or removing a source doesn't shift the byte offsets of others
///   (each is self-describing via its label).
/// - Cross-source collisions are impossible ("AB"+"C" ≠ "A"+"BC").
/// - Future source additions are safe — the fingerprint changes only for
///   sources whose availability changes.
///
/// Source priority on Linux (highest → lowest):
///   1. `/etc/machine-id`          (systemd, stable across reboots)
///   2. `/var/lib/dbus/machine-id` (older D-Bus installations)
///   3. `/proc/sys/kernel/random/boot_id` — ONLY appended when neither of
///      the above was readable.  boot_id resets on reboot, so it is used
///      purely as a degradation mode for containers/VMs without a machine-id.
///      Including it alongside a real machine-id would cause the fingerprint
///      to rotate on every reboot, breaking vault access.
///
/// macOS: Platform UUID via `ioreg -rd1 -c IOPlatformExpertDevice`.
///        Stable across reboots, unique per physical machine.
///
/// Windows: MachineGuid via `reg query HKLM\SOFTWARE\Microsoft\Cryptography`.
///          If the reg query fails, emit nothing — fingerprint relies on the
///          persisted random_id alone (same posture as Linux without machine-id).
/// Pick the first candidate that we can resolve. Absolute paths must exist on
/// disk; bare names are accepted (PATH lookup will be used). Falls back to the
/// last candidate as a last resort. Required because launchd / Windows service
/// contexts may run with a minimal PATH that excludes `/usr/sbin` or
/// `C:\Windows\System32`, so a bare `Command::new("ioreg")` / `Command::new("reg")`
/// silently returns ENOENT (see #5025).
///
/// On Linux the lib build has no production callers (both call sites are
/// gated to `target_os = "macos"` / `target_os = "windows"`), so the
/// `--deny=warnings` CI lane flags this as dead code. The unit tests
/// (cfg(test)) cover all platforms, but the lib-only metadata build that
/// `cargo llvm-cov` performs first does not include them.
#[allow(dead_code)]
fn resolve_command(candidates: &[&'static str]) -> &'static str {
    for &p in candidates {
        let is_absolute = p.starts_with('/') || p.contains(":\\");
        if is_absolute {
            if std::path::Path::new(p).exists() {
                return p;
            }
            // Doesn't exist — fall through to next candidate.
            continue;
        }
        // Bare name: trust PATH; return immediately.
        return p;
    }
    candidates.last().copied().unwrap_or("")
}

#[cfg(not(test))]
fn collect_os_machine_id_material() -> Vec<u8> {
    let mut out = Vec::new();

    // Emit one tagged, length-prefixed source.
    // `label` must be a stable ASCII string unique to this source.
    let mut emit = |label: &[u8], bytes: &[u8]| {
        // tag: label bytes + NUL terminator
        out.extend_from_slice(label);
        out.push(0x00);
        // 4-byte LE length of the payload
        let len = u32::try_from(bytes.len()).unwrap_or(0).to_le_bytes();
        out.extend_from_slice(&len);
        out.extend_from_slice(bytes);
    };

    // Helper: read a file and trim trailing whitespace (machine-id files end with '\n').
    let read_trimmed = |path: &str| -> Option<Vec<u8>> {
        let bytes = std::fs::read(path).ok()?;
        let trimmed = bytes.trim_ascii_end();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_vec())
        }
    };

    // ----- Linux sources -----
    // Priority: persistent machine-id first; boot_id only as a fallback for
    // containers/VMs that have no persistent machine-id.
    let mut has_persistent_linux_id = false;

    if let Some(b) = read_trimmed("/etc/machine-id") {
        emit(b"etc-machine-id", &b);
        has_persistent_linux_id = true;
    }
    if let Some(b) = read_trimmed("/var/lib/dbus/machine-id") {
        emit(b"dbus-machine-id", &b);
        has_persistent_linux_id = true;
    }

    // boot_id is appended ONLY when no persistent machine-id was available.
    // On a host with a real machine-id, including boot_id would cause the
    // fingerprint to rotate on every reboot, breaking vault access.
    if !has_persistent_linux_id {
        if let Some(b) = read_trimmed("/proc/sys/kernel/random/boot_id") {
            emit(b"linux-boot-id", &b);
        }
    }

    // ----- macOS source -----
    // `ioreg -rd1 -c IOPlatformExpertDevice` prints lines like:
    //   "IOPlatformUUID" = "XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX"
    // This UUID is stable across reboots and unique per physical machine.
    // We shell out rather than reading /private/var/db/SystemPolicyConfiguration/SystemPolicy
    // (root-only, binary blob, content unstable across macOS releases).
    //
    // `ioreg` lives at `/usr/sbin/ioreg`, which launchd's default minimal PATH
    // (`/usr/local/bin:/usr/bin:/bin`) does NOT include. A bare
    // `Command::new("ioreg")` therefore silently ENOENTs in the daemon
    // context, producing a different fingerprint than the one that wrote the
    // keyring and leaving the vault locked at startup (#5025). Resolve to an
    // absolute path first; only fall back to PATH lookup if no candidate
    // exists on disk.
    #[cfg(target_os = "macos")]
    {
        let ioreg = resolve_command(&["/usr/sbin/ioreg", "/sbin/ioreg", "ioreg"]);
        match std::process::Command::new(ioreg)
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut found = false;
                for line in stdout.lines() {
                    if line.contains("IOPlatformUUID") {
                        // Line format: `  "IOPlatformUUID" = "GUID-HERE"`
                        // Split on '"' and take the 4th token (index 3) as the UUID value.
                        let parts: Vec<&str> = line.splitn(4, '"').collect();
                        if let Some(raw) = parts.get(3) {
                            let uuid = raw.trim_end_matches('"').trim();
                            if !uuid.is_empty() {
                                emit(b"macos-platform-uuid", uuid.as_bytes());
                                found = true;
                            }
                        }
                        break;
                    }
                }
                if !found {
                    warn!(
                        binary = ioreg,
                        "ioreg did not return an IOPlatformUUID — macOS fingerprint will rely on random_id alone; \
                         this will produce a DIFFERENT wrap key from the one that wrote the keyring and the vault \
                         will appear locked at startup (#5025)"
                    );
                }
            }
            Err(e) => {
                warn!(
                    binary = ioreg,
                    error = %e,
                    "failed to spawn ioreg — macOS fingerprint will rely on random_id alone; \
                     this will produce a DIFFERENT wrap key from the one that wrote the keyring and the vault \
                     will appear locked at startup. Ensure /usr/sbin/ioreg exists or that /usr/sbin is on PATH (#5025)"
                );
            }
        }
    }

    // ----- Windows source -----
    // MachineGuid is stored in HKLM\SOFTWARE\Microsoft\Cryptography.
    // Shell out to `reg query` — no external crate needed, and vault load
    // happens only once per daemon start so the overhead is acceptable.
    // Output format: "    MachineGuid    REG_SZ    <guid>"
    //
    // `reg.exe` lives at `C:\Windows\System32\reg.exe`. A service-account
    // context with a stripped PATH would hit the same class of bug as the
    // macOS launchd case (#5025), so we resolve to an absolute path first.
    #[cfg(target_os = "windows")]
    {
        let reg_bin = resolve_command(&[r"C:\Windows\System32\reg.exe", "reg"]);
        match std::process::Command::new(reg_bin)
            .args([
                "query",
                r"HKLM\SOFTWARE\Microsoft\Cryptography",
                "/v",
                "MachineGuid",
            ])
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut found = false;
                for line in stdout.lines() {
                    if line.contains("MachineGuid") && line.contains("REG_SZ") {
                        // The GUID is the last whitespace-delimited token.
                        if let Some(guid) = line.split_whitespace().last() {
                            if !guid.is_empty() {
                                emit(b"windows-machine-guid", guid.as_bytes());
                                found = true;
                            }
                        }
                        break;
                    }
                }
                if !found {
                    warn!(
                        binary = reg_bin,
                        "reg query did not return a MachineGuid — Windows fingerprint will rely on random_id alone; \
                         this will produce a DIFFERENT wrap key from the one that wrote the keyring and the vault \
                         will appear locked at startup (#5025)"
                    );
                }
            }
            Err(e) => {
                warn!(
                    binary = reg_bin,
                    error = %e,
                    "failed to spawn reg.exe — Windows fingerprint will rely on random_id alone; \
                     this will produce a DIFFERENT wrap key from the one that wrote the keyring and the vault \
                     will appear locked at startup. Ensure C:\\Windows\\System32\\reg.exe exists or is on PATH (#5025)"
                );
            }
        }
    }

    out
}

/// Mix a 32-byte random machine-id with OS-provided material using SHA-512,
/// returning the first 32 bytes.
///
/// The result is at least as strong as either input alone:
/// - If `random_id` is high-entropy, the output is high-entropy regardless of
///   `os_material`.
/// - If `random_id` is somehow recoverable (e.g. backup), `os_material` still
///   provides a second factor that must match the live machine.
fn mix_fingerprint_sources(random_id: &[u8], os_material: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha512};
    let mut hasher = Sha512::new();
    // Domain separator so this context can't be confused with other SHA-512 uses.
    hasher.update(b"librefang-machine-fingerprint-v2\x00");
    // Length-prefix each input to prevent cross-field collisions.
    let len = (random_id.len() as u32).to_le_bytes();
    hasher.update(len);
    hasher.update(random_id);
    let len = (os_material.len() as u32).to_le_bytes();
    hasher.update(len);
    hasher.update(os_material);
    // SHA-512 gives 64 bytes; take the first 32 as the fingerprint.
    hasher.finalize()[..32].to_vec()
}

/// Predictable fallback derivation used when no random machine-id can be
/// persisted.  Incorporates all available OS machine-id material so even the
/// fallback path gains entropy from stable hardware/OS identifiers.
///
/// On Linux hosts with systemd (containers included), `/etc/machine-id`
/// provides a stable UUID that is distinct per container instance, dramatically
/// reducing the predictability compared to username+hostname alone.
///
/// Operators on hosts where none of the OS sources are readable should set
/// `LIBREFANG_VAULT_KEY` for guaranteed security.
#[cfg(not(test))]
fn predictable_machine_fingerprint() -> Vec<u8> {
    let mut hasher = Sha256::new();
    // Domain separator.
    hasher.update(b"librefang-vault-fallback-v2\x00");
    if let Ok(user) = std::env::var("USERNAME").or_else(|_| std::env::var("USER")) {
        hasher.update(user.as_bytes());
    }
    if let Ok(host) = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")) {
        hasher.update(host.as_bytes());
    }
    // Incorporate all available OS machine-id material.  Each source that is
    // present adds entropy.  Missing sources are silently skipped.
    let os_material = collect_os_machine_id_material();
    hasher.update(&os_material);
    hasher.update(b"librefang-vault-v1");
    hasher.finalize().to_vec()
}

/// Derive a 256-bit wrapping key from a machine fingerprint + salt using Argon2id.
///
/// Parameters are pinned to OWASP-recommended values (m=19456 KiB, t=2, p=1)
/// matching the rest of the codebase (see `password_hash.rs`). Using
/// `Argon2::default()` is intentionally avoided here because the defaults
/// silently changed across argon2 crate minor versions.
fn derive_wrapping_key(fingerprint: &[u8], salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, String> {
    let params =
        Params::new(19_456, 2, 1, None).map_err(|e| format!("Argon2 params error: {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut derived = Zeroizing::new([0u8; 32]);
    argon2
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

    // -----------------------------------------------------------------------
    // Tests for the strengthened keyring-fallback KDF (#3606)
    // -----------------------------------------------------------------------

    /// The fingerprint mixer is deterministic: same inputs always produce the
    /// same 32-byte output.
    #[test]
    fn mix_fingerprint_sources_is_deterministic() {
        let random_id = [0xABu8; 32];
        let os_material = b"test-machine-id-12345678".to_vec();

        let f1 = mix_fingerprint_sources(&random_id, &os_material);
        let f2 = mix_fingerprint_sources(&random_id, &os_material);
        assert_eq!(f1, f2, "fingerprint mixer must be deterministic");
        assert_eq!(f1.len(), 32);
    }

    /// Different random_ids produce different fingerprints even with the same
    /// OS material — so each install has a unique wrapping key.
    #[test]
    fn mix_fingerprint_sources_different_random_ids_produce_different_outputs() {
        let os_material = b"same-os-material".to_vec();
        let id_a = [0x11u8; 32];
        let id_b = [0x22u8; 32];

        let fa = mix_fingerprint_sources(&id_a, &os_material);
        let fb = mix_fingerprint_sources(&id_b, &os_material);
        assert_ne!(
            fa, fb,
            "different random_ids must produce different fingerprints"
        );
    }

    /// Different OS materials produce different fingerprints even with the same
    /// random_id — so the OS identity is a meaningful second factor.
    #[test]
    fn mix_fingerprint_sources_different_os_material_produces_different_outputs() {
        let random_id = [0x42u8; 32];
        let mat_a = b"machine-id-aaa".to_vec();
        let mat_b = b"machine-id-bbb".to_vec();

        let fa = mix_fingerprint_sources(&random_id, &mat_a);
        let fb = mix_fingerprint_sources(&random_id, &mat_b);
        assert_ne!(
            fa, fb,
            "different OS materials must produce different fingerprints"
        );
    }

    /// Empty OS material is handled gracefully — the function still returns a
    /// 32-byte value derived from the random_id alone.
    #[test]
    fn mix_fingerprint_sources_handles_empty_os_material() {
        let random_id = [0x99u8; 32];
        let result = mix_fingerprint_sources(&random_id, &[]);
        assert_eq!(result.len(), 32);
    }

    /// Two different Argon2id salts must produce different wrapping keys for
    /// the same fingerprint — ensures per-store salt diversity is effective.
    #[test]
    fn derive_wrapping_key_different_salts_produce_different_keys() {
        let fingerprint = [0x55u8; 32];
        let salt_a = [0x01u8; 16];
        let salt_b = [0x02u8; 16];

        let ka = derive_wrapping_key(&fingerprint, &salt_a).unwrap();
        let kb = derive_wrapping_key(&fingerprint, &salt_b).unwrap();
        assert_ne!(
            ka.as_ref(),
            kb.as_ref(),
            "different salts must produce different wrapping keys"
        );
    }

    /// Argon2id KDF is deterministic: same fingerprint + salt always yields
    /// the same wrapping key across calls.
    #[test]
    fn derive_wrapping_key_is_deterministic() {
        let fingerprint = [0x77u8; 32];
        let salt = [0xAAu8; 16];

        let k1 = derive_wrapping_key(&fingerprint, &salt).unwrap();
        let k2 = derive_wrapping_key(&fingerprint, &salt).unwrap();
        assert_eq!(
            k1.as_ref(),
            k2.as_ref(),
            "Argon2id KDF must be deterministic"
        );
    }

    /// Migration path: a vault key encrypted with fingerprint A can be
    /// re-wrapped with fingerprint B and subsequently decrypted correctly.
    /// This mirrors the automatic migration that happens on first post-upgrade
    /// load when the machine-id file gains new OS material.
    #[test]
    fn wrapping_key_migration_old_to_new_fingerprint() {
        // Simulate "old" fingerprint (random_id only, no OS material mixed in).
        let old_fingerprint = [0x11u8; 32];
        let old_salt = [0x01u8; 16];
        let old_wrapping_key = derive_wrapping_key(&old_fingerprint, &old_salt).unwrap();

        // Encrypt a fake master key with the old wrapping key.
        let fake_master_key = b"fake-master-key-b64-encoded-aaaa";
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let cipher = Aes256Gcm::new_from_slice(old_wrapping_key.as_ref()).unwrap();
        let ciphertext = cipher.encrypt(nonce, fake_master_key.as_ref()).unwrap();

        // Decrypt with old key — must succeed.
        let decrypted = cipher.decrypt(nonce, ciphertext.as_slice()).unwrap();
        assert_eq!(decrypted, fake_master_key);

        // "Migration": re-derive with new fingerprint (random_id + OS material).
        let new_fingerprint = mix_fingerprint_sources(&old_fingerprint, b"etc-machine-id-abc123");
        let new_salt = [0x02u8; 16];
        let new_wrapping_key = derive_wrapping_key(&new_fingerprint, &new_salt).unwrap();

        // Re-encrypt with new wrapping key.
        let mut new_nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut new_nonce_bytes);
        let new_nonce = Nonce::from_slice(&new_nonce_bytes);
        let new_cipher = Aes256Gcm::new_from_slice(new_wrapping_key.as_ref()).unwrap();
        let new_ciphertext = new_cipher.encrypt(new_nonce, decrypted.as_slice()).unwrap();

        // Decrypt with new key — must recover original plaintext.
        let recovered = new_cipher
            .decrypt(new_nonce, new_ciphertext.as_slice())
            .unwrap();
        assert_eq!(
            recovered, fake_master_key,
            "migrated vault must decrypt correctly with new wrapping key"
        );

        // Old key must NOT decrypt new ciphertext.
        let bad_result = cipher.decrypt(new_nonce, new_ciphertext.as_slice());
        assert!(
            bad_result.is_err(),
            "old wrapping key must not decrypt new ciphertext"
        );
    }

    // -----------------------------------------------------------------------
    // v2 → v3 keyring migration tests (#4159 follow-up)
    // -----------------------------------------------------------------------

    /// Build a v2 KeyringFile JSON string using the pre-#4159 derivation:
    /// fingerprint = raw random_id (no SHA-512 mixing).
    /// Returns (json_string, random_id, master_key_plaintext).
    fn make_v2_keyring(random_id: &[u8; 32], master_key: &str) -> String {
        // v2 fingerprint = raw random_id
        let v2_fingerprint = random_id.to_vec();

        let mut salt_bytes = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt_bytes);
        let wrapping_key = derive_wrapping_key(&v2_fingerprint, &salt_bytes).unwrap();

        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let cipher = Aes256Gcm::new_from_slice(wrapping_key.as_ref()).unwrap();
        let ciphertext = cipher.encrypt(nonce, master_key.as_bytes()).unwrap();

        // Serialize in the same format as the old store_keyring_key (version=2).
        serde_json::json!({
            "version": 2u8,
            "salt": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, salt_bytes),
            "nonce": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, nonce_bytes),
            "ciphertext": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &ciphertext),
        })
        .to_string()
    }

    /// The legacy_v2_fingerprint function returns the raw random_id bytes.
    /// We test it indirectly via make_v2_keyring + derive_wrapping_key.
    /// Here we verify that the v2 derivation is exactly: Argon2id(random_id, salt).
    #[test]
    fn v2_fingerprint_is_raw_random_id() {
        let random_id = [0xAAu8; 32];
        // v2 fingerprint = raw bytes, no mixing
        let v2_fp: Vec<u8> = random_id.to_vec();
        let salt = [0x01u8; SALT_LEN];

        let k1 = derive_wrapping_key(&v2_fp, &salt).unwrap();
        let k2 = derive_wrapping_key(&random_id, &salt).unwrap();
        // They must be identical — v2 used random_id directly.
        assert_eq!(k1.as_ref(), k2.as_ref());

        // And must differ from the v3 mixed fingerprint.
        let v3_fp = mix_fingerprint_sources(&random_id, b"some-os-material");
        let k3 = derive_wrapping_key(&v3_fp, &salt).unwrap();
        assert_ne!(
            k1.as_ref(),
            k3.as_ref(),
            "v2 and v3 fingerprints must differ"
        );
    }

    /// Simulate the full v2→v3 migration path:
    ///
    /// 1. Build a v2 `.keyring` file on disk with a known random_id and master key.
    /// 2. Verify the v2 derive_wrapping_key path decrypts correctly.
    /// 3. Re-wrap with v3 fingerprint (mix_fingerprint_sources).
    /// 4. Verify the v3 ciphertext decrypts correctly with the mixed fingerprint.
    /// 5. Verify the old v2 key does NOT decrypt the v3 ciphertext (isolation).
    /// 6. Verify idempotence: calling the v3 derive again with same inputs succeeds.
    #[test]
    fn v2_to_v3_migration_full_path() {
        let random_id = [0x42u8; 32];
        let master_key = "MASTER_KEY_B64_PLACEHOLDER_FOR_TEST";
        let os_material = b"etc-machine-id\x00\x10\x00\x00\x00fake-machine-id-1234";

        // --- Step 1: synthesize a v2 keyring JSON (pre-#4159 format) ---
        let v2_json = make_v2_keyring(&random_id, master_key);
        let v2_file: serde_json::Value = serde_json::from_str(&v2_json).unwrap();
        assert_eq!(v2_file["version"], 2, "synthesized file must be version 2");

        // --- Step 2: v2 decrypt path ---
        // Reconstruct exactly as load_keyring_key's v2 branch does.
        let salt = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            v2_file["salt"].as_str().unwrap(),
        )
        .unwrap();
        let nonce_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            v2_file["nonce"].as_str().unwrap(),
        )
        .unwrap();
        let ciphertext = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            v2_file["ciphertext"].as_str().unwrap(),
        )
        .unwrap();

        let v2_fingerprint: Vec<u8> = random_id.to_vec(); // legacy_v2_fingerprint() behavior
        let v2_wrapping_key = derive_wrapping_key(&v2_fingerprint, &salt).unwrap();
        let v2_cipher = Aes256Gcm::new_from_slice(v2_wrapping_key.as_ref()).unwrap();
        let v2_nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext = v2_cipher.decrypt(v2_nonce, ciphertext.as_slice()).unwrap();
        let decrypted_key = String::from_utf8(plaintext).unwrap();
        assert_eq!(
            decrypted_key, master_key,
            "(a) v2 decrypt must recover the original master key"
        );

        // --- Step 3: re-wrap with v3 fingerprint ---
        let v3_fingerprint = mix_fingerprint_sources(&random_id, os_material);
        let mut new_salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut new_salt);
        let v3_wrapping_key = derive_wrapping_key(&v3_fingerprint, &new_salt).unwrap();
        let mut new_nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut new_nonce_bytes);
        let v3_nonce = Nonce::from_slice(&new_nonce_bytes);
        let v3_cipher = Aes256Gcm::new_from_slice(v3_wrapping_key.as_ref()).unwrap();
        let v3_ciphertext = v3_cipher
            .encrypt(v3_nonce, decrypted_key.as_bytes())
            .unwrap();

        // Serialize as a v3 keyring file.
        let v3_json = serde_json::json!({
            "version": 3u8,
            "salt": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, new_salt),
            "nonce": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, new_nonce_bytes),
            "ciphertext": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &v3_ciphertext),
        })
        .to_string();
        let v3_file: serde_json::Value = serde_json::from_str(&v3_json).unwrap();
        assert_eq!(
            v3_file["version"], 3,
            "(b) migrated file on disk must be version 3"
        );
        assert_ne!(
            v3_file["ciphertext"], v2_file["ciphertext"],
            "(b) v3 ciphertext must differ from v2 (different wrapping key)"
        );

        // --- Step 4: v3 decrypt succeeds ---
        let v3_salt_dec = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            v3_file["salt"].as_str().unwrap(),
        )
        .unwrap();
        let v3_nonce_dec = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            v3_file["nonce"].as_str().unwrap(),
        )
        .unwrap();
        let v3_ct_dec = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            v3_file["ciphertext"].as_str().unwrap(),
        )
        .unwrap();
        let v3_wk2 = derive_wrapping_key(&v3_fingerprint, &v3_salt_dec).unwrap();
        let v3_c2 = Aes256Gcm::new_from_slice(v3_wk2.as_ref()).unwrap();
        let recovered = v3_c2
            .decrypt(Nonce::from_slice(&v3_nonce_dec), v3_ct_dec.as_slice())
            .unwrap();
        assert_eq!(
            String::from_utf8(recovered).unwrap(),
            master_key,
            "(c) v3 re-load (idempotent) must recover the original master key"
        );

        // --- Step 5: v2 key must NOT decrypt v3 ciphertext ---
        let bad = v2_cipher.decrypt(Nonce::from_slice(&v3_nonce_dec), v3_ct_dec.as_slice());
        assert!(
            bad.is_err(),
            "(isolation) v2 wrapping key must not decrypt v3 ciphertext"
        );
    }

    /// `try_decrypt_v2` must accept the FIRST candidate that decrypts cleanly
    /// and stop. This is the happy path used by load_keyring_key when the
    /// persisted random_id is intact.
    #[test]
    fn try_decrypt_v2_accepts_first_matching_candidate() {
        let fp_correct = [0x42u8; 32];
        let fp_wrong_a = [0x11u8; 32];
        let fp_wrong_b = [0x22u8; 32];
        let mut salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);

        let wk = derive_wrapping_key(&fp_correct, &salt).unwrap();
        let cipher = Aes256Gcm::new_from_slice(wk.as_ref()).unwrap();
        let plaintext = b"super-secret-master-key";
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
            .unwrap();

        // Correct fingerprint first → success.
        let recovered = try_decrypt_v2(
            &salt,
            &nonce_bytes,
            &ciphertext,
            &[&fp_correct, &fp_wrong_a, &fp_wrong_b],
        )
        .unwrap();
        assert_eq!(recovered.as_bytes(), plaintext);
    }

    /// Critical regression: when the FIRST candidate fails (raw random_id
    /// missing — the read-only-FS scenario), `try_decrypt_v2` must continue
    /// to the predictable-fingerprint candidate and recover the vault. This
    /// is the path that pre-#4159 v2 vaults written without a persisted
    /// .machine-id rely on for upgrade.
    #[test]
    fn try_decrypt_v2_falls_through_to_predictable_candidate() {
        let fp_raw_random_id = [0xAAu8; 32]; // would-be candidate (a)
        let fp_predictable = [0xBBu8; 32]; // candidate (b) — actually used for write
        let mut salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);

        // Vault was written with the predictable fingerprint (read-only FS path).
        let wk = derive_wrapping_key(&fp_predictable, &salt).unwrap();
        let cipher = Aes256Gcm::new_from_slice(wk.as_ref()).unwrap();
        let plaintext = b"vault-key-from-readonly-fs-host";
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
            .unwrap();

        // Caller injects raw random_id first, predictable second — same
        // ordering as the production v2 branch in load_keyring_key.
        let recovered = try_decrypt_v2(
            &salt,
            &nonce_bytes,
            &ciphertext,
            &[&fp_raw_random_id, &fp_predictable],
        )
        .unwrap();
        assert_eq!(
            recovered.as_bytes(),
            plaintext,
            "fallback to predictable fingerprint must recover the vault"
        );
    }

    /// All candidates wrong → hard error. Makes sure we don't return Ok with
    /// some garbage plaintext when every fingerprint failed.
    #[test]
    fn try_decrypt_v2_returns_error_when_all_candidates_fail() {
        let fp_correct = [0x42u8; 32];
        let mut salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);

        let wk = derive_wrapping_key(&fp_correct, &salt).unwrap();
        let cipher = Aes256Gcm::new_from_slice(wk.as_ref()).unwrap();
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), b"plaintext".as_ref())
            .unwrap();

        let bad_a = [0x01u8; 32];
        let bad_b = [0x02u8; 32];
        let result = try_decrypt_v2(&salt, &nonce_bytes, &ciphertext, &[&bad_a, &bad_b]);
        assert!(
            result.is_err(),
            "all-wrong candidates must produce an error, got: {result:?}"
        );
    }

    /// Empty candidate list is a programmer error and must surface as an
    /// explicit error rather than silently returning Ok("").
    #[test]
    fn try_decrypt_v2_rejects_empty_candidate_list() {
        let mut salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        let nonce = [0u8; NONCE_LEN];
        let ct = vec![0u8; 32];
        let result = try_decrypt_v2(&salt, &nonce, &ct, &[]);
        assert!(result.is_err());
    }

    /// Setting `LIBREFANG_VAULT_NO_KEYRING=1` must force
    /// `should_use_os_keyring()` to false on every platform, regardless
    /// of the compiled-in default or the process-global config flag.
    /// This is the macOS prompt-fatigue escape hatch.
    #[test]
    fn vault_no_keyring_env_var_forces_file_fallback() {
        // Serialize against any other env-mutating tests in this crate.
        // `set_var` is `unsafe` since Rust 1.80 because cross-thread env
        // mutation can race with reads in libc; the codebase uses the
        // same `unsafe { ... }` pattern in other test modules.
        for v in ["1", "true", "TRUE", "True"] {
            unsafe { std::env::set_var(VAULT_NO_KEYRING_ENV, v) };
            assert!(
                !should_use_os_keyring(),
                "{VAULT_NO_KEYRING_ENV}={v} must disable the OS keyring"
            );
        }

        // Empty / unset / unrelated values must NOT force-disable.
        unsafe { std::env::set_var(VAULT_NO_KEYRING_ENV, "0") };
        let with_zero = should_use_os_keyring();
        unsafe { std::env::set_var(VAULT_NO_KEYRING_ENV, "") };
        let with_empty = should_use_os_keyring();
        unsafe { std::env::remove_var(VAULT_NO_KEYRING_ENV) };
        let unset = should_use_os_keyring();

        // These three must agree — none should force-disable. They can
        // independently be true OR false depending on platform default
        // and any prior `set_use_os_keyring` call in the same process,
        // but they must be equal.
        assert_eq!(with_zero, with_empty);
        assert_eq!(with_empty, unset);
    }

    /// macOS gets `false` by default; everywhere else gets `true`.
    /// Documents the platform policy.
    #[test]
    fn platform_default_skips_keyring_only_on_macos() {
        let expected = !cfg!(target_os = "macos");
        assert_eq!(default_use_os_keyring_for_platform(), expected);
    }

    // -----------------------------------------------------------------------
    // Sentinel tests (#3651)
    // -----------------------------------------------------------------------

    /// A freshly-init'd vault has the sentinel pre-written and verifies
    /// cleanly without any backfill.
    #[test]
    fn sentinel_present_after_init_with_key() {
        let (_dir, mut vault) = test_vault();
        let key = random_key();
        vault.init_with_key(key).unwrap();

        // Sentinel present in the underlying map …
        assert_eq!(
            vault.entries.get(SENTINEL_KEY).map(|v| v.as_str()),
            Some(SENTINEL_VALUE),
            "init must write the sentinel"
        );
        // … but hidden from list_keys / len so users still see "empty".
        assert!(vault.list_keys().is_empty());
        assert_eq!(vault.len(), 0);
        assert!(vault.is_empty());
        // verify_or_install_sentinel is a no-op on the present-and-correct path.
        vault.verify_or_install_sentinel().unwrap();
    }

    /// A "legacy" vault (one written before #3651, simulated by deleting the
    /// sentinel from the entries map and re-saving) gets the sentinel
    /// backfilled on first verify call.
    #[test]
    fn sentinel_backfilled_on_legacy_vault() {
        let (dir, mut vault) = test_vault();
        let key = random_key();
        vault.init_with_key(key.clone()).unwrap();

        // Simulate a pre-#3651 vault: pop the sentinel and re-save with the
        // same key. We can't go through `remove()` (which would reject the
        // reserved key) so we reach in directly — exactly what production
        // legacy vaults look like on disk.
        vault.entries.remove(SENTINEL_KEY);
        let mk = vault.resolve_master_key().unwrap();
        vault.save(&mk).unwrap();
        drop(vault);

        // Re-open as a fresh instance and run the verify-or-install path.
        let mut vault2 = CredentialVault::new(dir.path().join("vault.enc"));
        vault2.unlock_with_key(key.clone()).unwrap();
        assert!(
            !vault2.entries.contains_key(SENTINEL_KEY),
            "precondition: simulated legacy vault must have no sentinel"
        );
        vault2.verify_or_install_sentinel().unwrap();
        assert_eq!(
            vault2.entries.get(SENTINEL_KEY).map(|v| v.as_str()),
            Some(SENTINEL_VALUE),
            "backfill must persist the sentinel in memory"
        );

        // Re-open *again* — sentinel must now be on disk.
        drop(vault2);
        let mut vault3 = CredentialVault::new(dir.path().join("vault.enc"));
        vault3.unlock_with_key(key).unwrap();
        assert_eq!(
            vault3.entries.get(SENTINEL_KEY).map(|v| v.as_str()),
            Some(SENTINEL_VALUE),
            "backfill must persist the sentinel to disk"
        );
    }

    /// A sentinel whose plaintext disagrees with `SENTINEL_VALUE` triggers
    /// `VaultKeyMismatch`. This is the "future scheme version" branch — a
    /// wrong-key boot would already have failed at AES-GCM decrypt time.
    #[test]
    fn sentinel_mismatch_returns_vault_key_mismatch() {
        let (_dir, mut vault) = test_vault();
        let key = random_key();
        vault.init_with_key(key).unwrap();

        // Tamper with the sentinel plaintext to simulate a future v2 sentinel.
        vault.entries.insert(
            SENTINEL_KEY.to_string(),
            Zeroizing::new("librefang-vault-sentinel-v999".to_string()),
        );

        let err = vault.verify_or_install_sentinel().unwrap_err();
        match err {
            ExtensionError::VaultKeyMismatch { hint } => {
                assert!(
                    hint.contains("sentinel"),
                    "hint should mention the sentinel, got: {hint}"
                );
            }
            other => panic!("expected VaultKeyMismatch, got {other:?}"),
        }
    }

    /// External callers must not be able to overwrite the sentinel via the
    /// public `set` API — that would silently regress the boot-validation
    /// contract.
    #[test]
    fn set_rejects_reserved_sentinel_key() {
        let (_dir, mut vault) = test_vault();
        let key = random_key();
        vault.init_with_key(key).unwrap();

        let err = vault
            .set(
                SENTINEL_KEY.to_string(),
                Zeroizing::new("attacker-controlled".to_string()),
            )
            .unwrap_err();
        assert!(
            matches!(err, ExtensionError::Vault(ref m) if m.contains("Refusing to write reserved key")),
            "set must reject SENTINEL_KEY, got: {err:?}"
        );
    }

    /// External callers must not be able to remove the sentinel via the
    /// public `remove` API.
    #[test]
    fn remove_rejects_reserved_sentinel_key() {
        let (_dir, mut vault) = test_vault();
        let key = random_key();
        vault.init_with_key(key).unwrap();

        let err = vault.remove(SENTINEL_KEY).unwrap_err();
        assert!(
            matches!(err, ExtensionError::Vault(ref m) if m.contains("Refusing to remove reserved key")),
            "remove must reject SENTINEL_KEY, got: {err:?}"
        );
        // Sentinel must still be present after the rejected remove.
        assert!(vault.entries.contains_key(SENTINEL_KEY));
    }

    /// `rewrap_with_new_key` re-encrypts every entry including the sentinel
    /// so the rotated vault verifies on next boot under the new key.
    #[test]
    fn rewrap_with_new_key_preserves_entries_and_sentinel() {
        let (dir, mut vault) = test_vault();
        let key_old = random_key();
        let key_new = random_key();
        assert_ne!(key_old.as_ref(), key_new.as_ref());

        vault.init_with_key(key_old.clone()).unwrap();
        vault
            .set(
                "API_KEY".to_string(),
                Zeroizing::new("secret-1".to_string()),
            )
            .unwrap();
        vault
            .set("DB_URL".to_string(), Zeroizing::new("secret-2".to_string()))
            .unwrap();

        // Rotate to the new key.
        vault.rewrap_with_new_key(key_new.clone()).unwrap();
        drop(vault);

        // New key must work and recover every entry.
        let mut vault2 = CredentialVault::new(dir.path().join("vault.enc"));
        vault2.unlock_with_key(key_new).unwrap();
        assert_eq!(vault2.get("API_KEY").unwrap().as_str(), "secret-1");
        assert_eq!(vault2.get("DB_URL").unwrap().as_str(), "secret-2");
        // Sentinel survived the rotation.
        vault2.verify_or_install_sentinel().unwrap();

        // Old key must NO LONGER work — the file has been re-encrypted.
        let mut vault3 = CredentialVault::new(dir.path().join("vault.enc"));
        assert!(
            vault3.unlock_with_key(key_old).is_err(),
            "rotated vault must reject the old master key"
        );
    }

    /// `iter_all_entries` exposes the sentinel so the rotate-key migration
    /// can reason about the full set of slots; `list_keys` does not.
    #[test]
    fn iter_all_entries_includes_sentinel_but_list_keys_does_not() {
        let (_dir, mut vault) = test_vault();
        vault.init_with_key(random_key()).unwrap();
        vault
            .set("USER_KEY".to_string(), Zeroizing::new("v".to_string()))
            .unwrap();

        let all: Vec<&str> = vault.iter_all_entries().map(|(k, _)| k).collect();
        assert!(all.contains(&SENTINEL_KEY));
        assert!(all.contains(&"USER_KEY"));

        let user_visible = vault.list_keys();
        assert!(!user_visible.contains(&SENTINEL_KEY));
        assert!(user_visible.contains(&"USER_KEY"));
    }

    /// `set()` on a never-materialised vault (vault.enc absent on disk,
    /// `unlocked == false`) must lazy-init via the env-driven master key
    /// path, persist the entry, and leave the vault readable through the
    /// same instance. This pins the contract `kernel::vault_handle()`'s
    /// doc-comment promised but the previous implementation didn't honour
    /// — `install_integration_writes_through_cached_vault_handle` failed
    /// because the silent `VaultLocked` return swallowed by
    /// `installer::install_integration` reported `Ready` while never
    /// touching disk (refs #4788, #4791).
    #[test]
    #[serial_test::serial]
    fn set_lazy_inits_unopened_vault_on_first_write() {
        // Fixed test key — base64 of 32 zero bytes — keeps the test
        // hermetic (no random keyring writes, no Argon2id race).
        const TEST_VAULT_KEY_B64: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

        // SAFETY: serialised via `#[serial_test::serial]`; no other thread
        // in this crate's test suite reads these vars concurrently.
        unsafe {
            std::env::set_var(VAULT_KEY_ENV, TEST_VAULT_KEY_B64);
            std::env::set_var(VAULT_NO_KEYRING_ENV, "1");
        }

        let (_dir, mut vault) = test_vault();
        // Precondition: file does not exist yet, vault is locked.
        assert!(!vault.exists(), "fresh test vault must not exist on disk");
        assert!(!vault.is_unlocked(), "fresh handle must be locked");

        let path = vault.path.clone();
        vault
            .set(
                "LAZY_INIT_TOKEN".to_string(),
                Zeroizing::new("hello-lazy".to_string()),
            )
            .expect("set() must lazy-init when path is absent and proceed");

        // After lazy-init: file materialised, vault unlocked, value
        // readable through this handle.
        assert!(vault.exists(), "set() should have materialised vault.enc");
        assert!(vault.is_unlocked(), "set() should leave vault unlocked");
        let got = vault.get("LAZY_INIT_TOKEN").expect("value must round-trip");
        assert_eq!(got.as_str(), "hello-lazy");

        // Round-trip through a fresh process-shaped instance: drop the
        // unlocked vault, re-open the same path, and unlock via the env
        // master key. This pins that lazy-init *actually used* the
        // env-driven key — the previous in-instance get could pass even
        // if init() had silently fallen back to a random key (cached_key
        // would still hot-read the just-written entry within the same
        // instance, masking the divergence).
        drop(vault);
        let mut reopened = CredentialVault::new(path);
        reopened
            .unlock()
            .expect("env master key must unlock the lazy-initialised vault");
        let persisted = reopened
            .get("LAZY_INIT_TOKEN")
            .expect("token must survive a re-open with the env key");
        assert_eq!(persisted.as_str(), "hello-lazy");

        // Cleanup so neighbouring tests see an unset env.
        unsafe {
            std::env::remove_var(VAULT_KEY_ENV);
            std::env::remove_var(VAULT_NO_KEYRING_ENV);
        }
    }

    /// Calling `set()` with the reserved [`SENTINEL_KEY`] on a fresh
    /// handle must reject the write WITHOUT materialising vault.enc on
    /// disk. A rejected write is a no-op, including its filesystem
    /// footprint — otherwise an attacker / misuser triggering the
    /// rejection path can still mint a stray vault file in the home dir.
    #[test]
    #[serial_test::serial]
    fn set_rejects_sentinel_key_before_lazy_init_side_effect() {
        const TEST_VAULT_KEY_B64: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

        unsafe {
            std::env::set_var(VAULT_KEY_ENV, TEST_VAULT_KEY_B64);
            std::env::set_var(VAULT_NO_KEYRING_ENV, "1");
        }

        let (_dir, mut vault) = test_vault();
        assert!(!vault.exists());

        let err = vault
            .set(
                SENTINEL_KEY.to_string(),
                Zeroizing::new("attacker-controlled".to_string()),
            )
            .expect_err("writing the sentinel must fail");
        assert!(
            matches!(err, ExtensionError::Vault(_)),
            "expected Vault(_) refusal, got: {err:?}"
        );
        assert!(
            !vault.exists(),
            "rejected sentinel write must not materialise vault.enc"
        );
        assert!(
            !vault.is_unlocked(),
            "rejected sentinel write must leave the handle locked"
        );

        unsafe {
            std::env::remove_var(VAULT_KEY_ENV);
            std::env::remove_var(VAULT_NO_KEYRING_ENV);
        }
    }

    /// `set()` must NOT lazy-init when `vault.enc` already exists but the
    /// handle hasn't been unlocked — that is a real "wrong key / not yet
    /// unlocked" state and silently re-init'ing would either fail
    /// (`init()` rejects existing files) or worse, mask a misconfigured
    /// boot. The pre-existing `VaultLocked` error path must stay.
    #[test]
    #[serial_test::serial]
    fn set_does_not_lazy_init_when_vault_file_already_present() {
        const TEST_VAULT_KEY_B64: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

        unsafe {
            std::env::set_var(VAULT_KEY_ENV, TEST_VAULT_KEY_B64);
            std::env::set_var(VAULT_NO_KEYRING_ENV, "1");
        }

        // Materialise a vault, then drop the unlocked handle and build a
        // fresh locked one over the same path.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let mut bootstrap = CredentialVault::new(path.clone());
        bootstrap.init().expect("bootstrap init must succeed");
        drop(bootstrap);

        let mut locked = CredentialVault::new(path);
        assert!(locked.exists(), "precondition: vault.enc must exist");
        assert!(!locked.is_unlocked(), "precondition: handle must be locked");

        let err = locked
            .set("X".to_string(), Zeroizing::new("y".to_string()))
            .expect_err("set() on an existing-but-locked vault must error");
        assert!(
            matches!(err, ExtensionError::VaultLocked),
            "expected VaultLocked, got: {err:?}"
        );

        unsafe {
            std::env::remove_var(VAULT_KEY_ENV);
            std::env::remove_var(VAULT_NO_KEYRING_ENV);
        }
    }

    // ── resolve_command ─────────────────────────────────────────────────
    //
    // Regression suite for the launchd `ioreg` ENOENT bug (#5025). The
    // helper must:
    //   1. Pick an existing absolute path when one is present.
    //   2. Skip absolute paths that do not exist.
    //   3. Accept a bare command name as an implicit "trust PATH" fallback.

    #[cfg(unix)]
    #[test]
    fn resolve_command_picks_first_existing_absolute() {
        // /bin/sh exists on every supported Unix host (macOS, Linux). If a
        // CI environment ever pares this down we'll learn about it via this
        // test, which is exactly the visibility #5025 wished it had.
        let pick = resolve_command(&[
            "/this/path/definitely/does/not/exist",
            "/bin/sh",
            "fallback-bare-name",
        ]);
        assert_eq!(pick, "/bin/sh");
    }

    #[cfg(unix)]
    #[test]
    fn resolve_command_skips_missing_absolutes() {
        // Both absolutes are bogus — the bare-name fallback wins.
        let pick = resolve_command(&["/nope/one", "/nope/two", "fallback-bare-name"]);
        assert_eq!(pick, "fallback-bare-name");
    }

    #[cfg(unix)]
    #[test]
    fn resolve_command_returns_last_when_all_absolutes_missing_and_no_bare() {
        // Edge case: all candidates are absolute and none exist. The helper
        // returns the last candidate so the caller still passes a non-empty
        // command to `Command::new` and gets a proper Err back (which the
        // updated `collect_os_machine_id_material` logs explicitly).
        let pick = resolve_command(&["/nope/one", "/nope/two"]);
        assert_eq!(pick, "/nope/two");
    }

    #[cfg(windows)]
    #[test]
    fn resolve_command_skips_missing_windows_absolutes() {
        // On Windows the absolute-path discriminator is the drive prefix
        // `X:\`. cmd.exe is virtually always present in System32.
        let pick = resolve_command(&[
            r"C:\nope\does\not\exist.exe",
            r"C:\Windows\System32\cmd.exe",
            "fallback-bare-name",
        ]);
        assert_eq!(pick, r"C:\Windows\System32\cmd.exe");
    }

    #[test]
    fn resolve_command_accepts_bare_name_without_filesystem_check() {
        // The bare name is returned immediately — even if no such command
        // exists on PATH, the caller's `Command::new` will surface the
        // ENOENT (and the updated collector logs that path explicitly).
        let pick = resolve_command(&["definitely-not-a-real-command-1234567890"]);
        assert_eq!(pick, "definitely-not-a-real-command-1234567890");
    }
}
