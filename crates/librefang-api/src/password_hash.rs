//! Argon2id password hashing for dashboard authentication.
//!
//! Replaces the previous plaintext password comparison with Argon2id,
//! which is resistant to GPU/ASIC attacks and rainbow tables.
//!
//! Supports transparent migration from legacy plaintext passwords:
//! - If `dashboard_pass_hash` is set (Argon2id PHC string), verify against it.
//! - If only `dashboard_pass` is set (plaintext/vault), fall back to constant-time
//!   plaintext comparison and return the Argon2id hash for transparent upgrade.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};

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

/// Result of a dashboard password verification.
pub enum VerifyResult {
    /// Password matched (Argon2id or legacy). Contains the session token.
    Ok {
        token: String,
        /// If Some, the caller should persist this Argon2id hash to upgrade
        /// from the legacy plaintext password.
        upgrade_hash: Option<String>,
    },
    /// Password did not match.
    Denied,
}

/// Verify dashboard credentials with Argon2id (preferred) or legacy plaintext fallback.
///
/// - If `pass_hash` is non-empty, verify with Argon2id only.
/// - Otherwise, fall back to constant-time plaintext comparison against `cfg_pass`.
///   On success, returns an `upgrade_hash` so the caller can transparently migrate.
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

    if !user_ok {
        return VerifyResult::Denied;
    }

    // Strategy 1: Argon2id hash is configured — use it exclusively.
    if !pass_hash.is_empty() {
        if verify_password(input_pass, pass_hash) {
            let token = derive_session_token(cfg_user, input_pass);
            return VerifyResult::Ok {
                token,
                upgrade_hash: None,
            };
        }
        return VerifyResult::Denied;
    }

    // Strategy 2: Legacy plaintext password — constant-time compare then offer upgrade.
    if cfg_pass.is_empty() {
        return VerifyResult::Denied;
    }

    let pass_ok: bool = input_pass.as_bytes().ct_eq(cfg_pass.as_bytes()).into();

    if pass_ok {
        let token = derive_session_token(cfg_user, input_pass);
        // Generate an Argon2id hash so the caller can persist it for future logins.
        let upgrade_hash = hash_password(input_pass).ok();
        VerifyResult::Ok {
            token,
            upgrade_hash,
        }
    } else {
        VerifyResult::Denied
    }
}

/// Derive a deterministic session token from credentials using HMAC-SHA256.
///
/// This keeps the same token-derivation logic as before so existing sessions
/// remain valid across the migration.
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
        // Different salts → different hashes, but both verify.
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
    fn test_derive_session_token_is_deterministic() {
        let t1 = derive_session_token("admin", "secret");
        let t2 = derive_session_token("admin", "secret");
        assert_eq!(t1, t2);
        assert!(!t1.is_empty());
    }

    #[test]
    fn test_derive_session_token_differs_for_different_creds() {
        let t1 = derive_session_token("admin", "pass1");
        let t2 = derive_session_token("admin", "pass2");
        assert_ne!(t1, t2);
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
                assert!(!token.is_empty());
            }
            VerifyResult::Denied => panic!("should have succeeded"),
        }
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
                assert!(!token.is_empty());
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
}
