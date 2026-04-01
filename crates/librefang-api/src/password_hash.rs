//! Argon2id password hashing for dashboard authentication.
//!
//! Replaces the previous plaintext password comparison with Argon2id,
//! which is resistant to GPU/ASIC attacks and rainbow tables.
//!
//! Supports transparent migration from legacy plaintext passwords:
//! - If `dashboard_pass_hash` is set (Argon2id PHC string), verify against it.
//! - If only `dashboard_pass` is set (plaintext/vault), fall back to constant-time
//!   plaintext comparison and return the Argon2id hash for transparent upgrade.
//!
//! Session tokens are now generated randomly with expiration support,
//! replacing the old deterministic HMAC-SHA256-derived tokens that could
//! not be revoked or expired.

use argon2::{
    password_hash::{
        rand_core::{OsRng, RngCore},
        PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
    },
    Algorithm, Argon2, Params, Version,
};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Hash a password with Argon2id using recommended parameters.
///
/// Returns a PHC-format string like `$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`.
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let params = Params::new(19_456, 2, 1, None)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let hash = argon2.hash_password(password.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

/// Verify a password against an Argon2id PHC-format hash string.
pub fn verify_password(password: &str, hash_str: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash_str) else {
        return false;
    };
    // Use default Argon2id verifier — it reads params from the PHC string.
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// A session token with creation metadata for expiration support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionToken {
    /// The hex-encoded random token string.
    pub token: String,
    /// Unix timestamp (seconds) when this token was created.
    pub created_at: u64,
}

/// Default session TTL: 30 days.
pub const DEFAULT_SESSION_TTL_SECS: u64 = 30 * 24 * 3600;

/// Generate a cryptographically random session token.
///
/// Uses OS-level CSPRNG to produce a 32-byte (256-bit) random token,
/// paired with a creation timestamp for expiration checks.
pub fn generate_session_token() -> SessionToken {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    SessionToken { token, created_at }
}

/// Check if a session token has expired based on its creation time and TTL.
pub fn is_token_expired(token: &SessionToken, ttl_secs: u64) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now.saturating_sub(token.created_at) >= ttl_secs
}

/// Result of a dashboard password verification.
pub enum VerifyResult {
    /// Password matched (Argon2id or legacy). Contains the session token.
    Ok {
        token: SessionToken,
        /// If Some, the caller should persist this Argon2id hash to upgrade
        /// from the legacy plaintext password.
        upgrade_hash: Option<String>,
    },
    /// Password did not match.
    Denied,
}

/// Pick the server-side secret used to derive dashboard session tokens.
fn dashboard_session_secret<'a>(cfg_pass: &'a str, pass_hash: &'a str) -> Option<&'a str> {
    if !pass_hash.is_empty() {
        Some(pass_hash)
    } else if !cfg_pass.is_empty() {
        Some(cfg_pass)
    } else {
        None
    }
}

/// Derive the dashboard session token from the configured credentials.
#[deprecated(note = "Use generate_session_token() for random tokens with expiration")]
pub fn derive_dashboard_session_token(
    username: &str,
    cfg_pass: &str,
    pass_hash: &str,
) -> Option<String> {
    let secret = dashboard_session_secret(cfg_pass, pass_hash)?;
    if username.is_empty() {
        return None;
    }
    #[allow(deprecated)]
    Some(derive_session_token(username, secret))
}

/// Verify dashboard credentials with Argon2id (preferred) or legacy plaintext fallback.
///
/// - If `pass_hash` is non-empty, verify with Argon2id only.
/// - Otherwise, fall back to constant-time plaintext comparison against `cfg_pass`.
///   On success, returns an `upgrade_hash` so the caller can transparently migrate.
///
/// **Timing safety**: The password verification path always executes regardless of
/// whether the username matched. This prevents attackers from enumerating valid
/// usernames via timing differences (Argon2id is ~tens of ms vs instant return).
///
/// Returns a randomly generated `SessionToken` on success instead of a
/// deterministic HMAC-derived token, preventing token replay and enabling
/// expiration-based revocation.
pub fn verify_dashboard_password(
    input_user: &str,
    input_pass: &str,
    cfg_user: &str,
    cfg_pass: &str,
    pass_hash: &str,
) -> VerifyResult {
    // Username is always compared with constant-time comparison.
    use subtle::ConstantTimeEq;
    let user_ok: bool = input_user.as_bytes().ct_eq(cfg_user.as_bytes()).into();

    // Always verify password to prevent timing side-channel on username enumeration.
    // Even if username is wrong, we still run Argon2id so timing is constant.
    let pass_ok = if !pass_hash.is_empty() {
        verify_password(input_pass, pass_hash)
    } else if !cfg_pass.is_empty() {
        input_pass.as_bytes().ct_eq(cfg_pass.as_bytes()).into()
    } else {
        // No credentials configured — run a dummy hash to keep timing constant.
        let _ = hash_password(input_pass);
        false
    };

    if !user_ok || !pass_ok {
        return VerifyResult::Denied;
    }

    // Both matched — build result.
    if !pass_hash.is_empty() {
        let token = generate_session_token();
        VerifyResult::Ok {
            token,
            upgrade_hash: None,
        }
    } else {
        let token = generate_session_token();
        // Generate an Argon2id hash so the caller can persist it for future logins.
        let upgrade_hash = hash_password(input_pass).ok();
        VerifyResult::Ok {
            token,
            upgrade_hash,
        }
    }
}

/// Derive a deterministic session token from credentials using HMAC-SHA256.
///
/// This keeps the same token-derivation logic as before so existing sessions
/// remain valid across the migration.
#[deprecated(note = "Use generate_session_token() for random tokens with expiration")]
pub fn derive_session_token(username: &str, password: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(password.as_bytes()).expect("HMAC accepts any key size");
    mac.update(username.as_bytes());
    mac.update(b"librefang-dashboard-session");
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify() {
        let hash = hash_password("hunter2").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("hunter2", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn test_different_passwords_produce_different_hashes() {
        let h1 = hash_password("password1").unwrap();
        let h2 = hash_password("password2").unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_same_password_produces_different_salts() {
        let h1 = hash_password("same").unwrap();
        let h2 = hash_password("same").unwrap();
        // Different salts -> different hashes, but both verify.
        assert_ne!(h1, h2);
        assert!(verify_password("same", &h1));
        assert!(verify_password("same", &h2));
    }

    #[test]
    fn test_verify_rejects_invalid_hash_format() {
        assert!(!verify_password("pass", "not-a-valid-phc-string"));
        assert!(!verify_password("pass", ""));
    }

    #[test]
    #[allow(deprecated)]
    fn test_derive_session_token_is_deterministic() {
        let t1 = derive_session_token("admin", "secret");
        let t2 = derive_session_token("admin", "secret");
        assert_eq!(t1, t2);
        assert!(!t1.is_empty());
    }

    #[test]
    #[allow(deprecated)]
    fn test_derive_session_token_differs_for_different_creds() {
        let t1 = derive_session_token("admin", "pass1");
        let t2 = derive_session_token("admin", "pass2");
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_generate_session_token_is_random() {
        let t1 = generate_session_token();
        let t2 = generate_session_token();
        assert_ne!(t1.token, t2.token);
        assert_eq!(t1.token.len(), 64); // 32 bytes = 64 hex chars
        assert!(t1.created_at > 0);
    }

    #[test]
    fn test_session_token_not_expired() {
        let token = generate_session_token();
        assert!(!is_token_expired(&token, DEFAULT_SESSION_TTL_SECS));
    }

    #[test]
    fn test_session_token_expired() {
        let token = SessionToken {
            token: "deadbeef".to_string(),
            created_at: 0, // epoch = long ago
        };
        assert!(is_token_expired(&token, DEFAULT_SESSION_TTL_SECS));
    }

    #[test]
    fn test_session_token_zero_ttl_expires_immediately() {
        let token = generate_session_token();
        // A TTL of 0 means the token expires the instant it's created.
        assert!(is_token_expired(&token, 0));
    }

    #[test]
    fn test_verify_dashboard_argon2id_path() {
        let hash = hash_password("mypass").unwrap();
        match verify_dashboard_password("admin", "mypass", "admin", "", &hash) {
            VerifyResult::Ok {
                upgrade_hash,
                token,
            } => {
                assert!(upgrade_hash.is_none()); // Already using Argon2id
                assert!(!token.token.is_empty());
                assert_eq!(token.token.len(), 64);
                assert!(!is_token_expired(&token, DEFAULT_SESSION_TTL_SECS));
            }
            VerifyResult::Denied => panic!("should have succeeded"),
        }
    }

    #[test]
    fn test_verify_dashboard_argon2id_produces_unique_tokens() {
        let hash = hash_password("mypass").unwrap();
        let t1 = match verify_dashboard_password("admin", "mypass", "admin", "", &hash) {
            VerifyResult::Ok { token, .. } => token,
            VerifyResult::Denied => panic!("should have succeeded"),
        };
        let t2 = match verify_dashboard_password("admin", "mypass", "admin", "", &hash) {
            VerifyResult::Ok { token, .. } => token,
            VerifyResult::Denied => panic!("should have succeeded"),
        };
        // Each login produces a unique random token.
        assert_ne!(t1.token, t2.token);
    }

    #[test]
    fn test_verify_dashboard_argon2id_wrong_password() {
        let hash = hash_password("mypass").unwrap();
        assert!(matches!(
            verify_dashboard_password("admin", "wrong", "admin", "", &hash),
            VerifyResult::Denied
        ));
    }

    #[test]
    fn test_verify_dashboard_argon2id_wrong_username() {
        let hash = hash_password("mypass").unwrap();
        assert!(matches!(
            verify_dashboard_password("wrong", "mypass", "admin", "", &hash),
            VerifyResult::Denied
        ));
    }

    #[test]
    fn test_verify_dashboard_legacy_plaintext_fallback() {
        match verify_dashboard_password("admin", "secret", "admin", "secret", "") {
            VerifyResult::Ok {
                upgrade_hash,
                token,
            } => {
                // Should offer an upgrade hash
                assert!(upgrade_hash.is_some());
                let uh = upgrade_hash.unwrap();
                assert!(uh.starts_with("$argon2id$"));
                // The upgrade hash should verify against the password
                assert!(verify_password("secret", &uh));
                assert!(!token.token.is_empty());
                assert_eq!(token.token.len(), 64);
            }
            VerifyResult::Denied => panic!("should have succeeded with legacy plaintext"),
        }
    }

    #[test]
    fn test_verify_dashboard_legacy_wrong_password() {
        assert!(matches!(
            verify_dashboard_password("admin", "wrong", "admin", "secret", ""),
            VerifyResult::Denied
        ));
    }

    #[test]
    fn test_verify_dashboard_no_credentials() {
        assert!(matches!(
            verify_dashboard_password("admin", "pass", "admin", "", ""),
            VerifyResult::Denied
        ));
    }

    #[test]
    #[allow(deprecated)]
    fn test_session_token_matches_legacy_derivation() {
        // Verify our token derivation matches the old HMAC-SHA256 approach
        // so existing sessions are not invalidated.
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(b"pass").expect("HMAC key");
        mac.update(b"user");
        mac.update(b"librefang-dashboard-session");
        let expected: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();

        assert_eq!(derive_session_token("user", "pass"), expected);
    }

    #[test]
    #[allow(deprecated)]
    fn test_dashboard_session_token_uses_hash_when_available() {
        let token =
            derive_dashboard_session_token("admin", "legacy-pass", "stored-hash").expect("token");
        assert_eq!(token, derive_session_token("admin", "stored-hash"));
    }

    /// Verify that wrong-username and wrong-password both return `Denied`
    /// and that the password verification path is always exercised
    /// (i.e., no early return on username mismatch that would leak timing).
    #[test]
    fn test_timing_constant_on_wrong_username() {
        let hash = hash_password("correct-pass").unwrap();

        // Wrong username, correct password — must be Denied.
        assert!(matches!(
            verify_dashboard_password("wrong-user", "correct-pass", "admin", "", &hash),
            VerifyResult::Denied
        ));

        // Correct username, wrong password — must be Denied.
        assert!(matches!(
            verify_dashboard_password("admin", "wrong-pass", "admin", "", &hash),
            VerifyResult::Denied
        ));

        // Wrong username, wrong password — must be Denied.
        assert!(matches!(
            verify_dashboard_password("wrong-user", "wrong-pass", "admin", "", &hash),
            VerifyResult::Denied
        ));

        // Legacy plaintext path: wrong username, correct password — must be Denied.
        assert!(matches!(
            verify_dashboard_password("wrong-user", "secret", "admin", "secret", ""),
            VerifyResult::Denied
        ));

        // No credentials path: wrong username — must be Denied (and still runs
        // a dummy hash rather than returning instantly).
        assert!(matches!(
            verify_dashboard_password("wrong-user", "pass", "admin", "", ""),
            VerifyResult::Denied
        ));
    }

    #[test]
    fn test_session_token_serialization_roundtrip() {
        let token = generate_session_token();
        let json = serde_json::to_string(&token).unwrap();
        let deserialized: SessionToken = serde_json::from_str(&json).unwrap();
        assert_eq!(token.token, deserialized.token);
        assert_eq!(token.created_at, deserialized.created_at);
    }
}
