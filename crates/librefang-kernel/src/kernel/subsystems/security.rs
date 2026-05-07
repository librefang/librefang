//! Security subsystem — RBAC, device pairing, and credential vault.
//!
//! Bundles three identity-and-secrets clusters that previously sat as
//! flat fields on `LibreFangKernel`:
//!   * `auth` — RBAC + dashboard login.
//!   * `pairing` — device pairing manager.
//!   * `vault_*` — process-lifetime credential vault cache plus the
//!     mutex serialising recovery-code redemption (#3560 TOCTOU fix).

use std::sync::{Arc, Mutex, OnceLock, RwLock};

use crate::auth::AuthManager;
use crate::pairing::PairingManager;
use librefang_extensions::vault::CredentialVault;

/// Auth + pairing + vault cluster — see module docs.
pub struct SecuritySubsystem {
    /// RBAC authentication manager.
    pub(crate) auth: AuthManager,
    /// Device pairing manager.
    pub(crate) pairing: PairingManager,
    /// Serialises all recovery-code redemption attempts so the
    /// read-verify-write sequence is atomic within the process.
    /// Fixes the TOCTOU race described in issue #3560.
    pub(crate) vault_recovery_codes_mutex: Mutex<()>,
    /// Process-lifetime cache of the unlocked credential vault (#3598).
    /// `OnceLock<Arc<RwLock<…>>>` — see field-level docs at the
    /// original `LibreFangKernel.vault_cache` declaration site for
    /// rationale.
    pub(crate) vault_cache: OnceLock<Arc<RwLock<CredentialVault>>>,
}

impl SecuritySubsystem {
    pub(crate) fn new(auth: AuthManager, pairing: PairingManager) -> Self {
        Self {
            auth,
            pairing,
            vault_recovery_codes_mutex: Mutex::new(()),
            vault_cache: OnceLock::new(),
        }
    }

    /// RBAC authentication manager.
    #[inline]
    pub fn auth_ref(&self) -> &AuthManager {
        &self.auth
    }

    /// Device pairing manager.
    #[inline]
    pub fn pairing_ref(&self) -> &PairingManager {
        &self.pairing
    }
}
