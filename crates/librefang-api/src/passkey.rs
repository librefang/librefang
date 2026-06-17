//! Passkey (WebAuthn/FIDO2) ceremony engine (#5981).
//!
//! Wraps `webauthn-rs` and owns the short-lived, in-memory challenge state
//! for the two WebAuthn ceremonies. The HTTP handlers live in
//! [`crate::routes::passkey`]; persistence of the registered credentials
//! lives in [`librefang_memory::passkey_store`]. This module is the seam
//! between them — it speaks `webauthn-rs` types and never touches SQLite.
//!
//! ## Ceremony state
//!
//! Both ceremonies are two requests (`*-options` then `*-verify`). The
//! server-side challenge state (`PasskeyRegistration` / `PasskeyAuthentication`)
//! returned by the *start* call must be replayed into the *finish* call. We
//! correlate the two halves with an opaque random `ceremony_id` handed to the
//! browser and echoed back on verify — state is kept in a short-TTL in-memory
//! map, never persisted (a half-finished ceremony is worthless after a
//! restart). Expired entries are pruned opportunistically on each start.
//!
//! ## Identity binding
//!
//! Passkeys bind to the **same principal** the password login produces. The
//! WebAuthn user handle is a stable UUIDv5 derived from the principal name so
//! the same operator always maps to the same handle across registrations.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use webauthn_rs::prelude::*;

use crate::password_hash::generate_session_token;

/// How long a started ceremony stays valid before it is pruned. Matches the
/// WebAuthn default authenticator timeout with a little slack for user
/// interaction (biometric prompt, security-key tap).
const CEREMONY_TTL_SECS: u64 = 300;

/// Fixed namespace for deriving the stable per-principal WebAuthn user handle.
/// Random-but-constant; only its stability matters.
const USER_HANDLE_NAMESPACE: Uuid = Uuid::from_u128(0x6c69_6272_6566_616e_6770_6173_736b_6579);

/// Errors surfaced by the engine. The route layer maps these onto HTTP
/// responses.
#[derive(Debug)]
pub enum PasskeyError {
    /// The ceremony id was unknown or already consumed/expired.
    UnknownCeremony,
    /// The stored credential blob failed to deserialize (corrupt row).
    CorruptCredential(serde_json::Error),
    /// `webauthn-rs` rejected the ceremony (bad challenge, origin mismatch,
    /// signature failure, sign-count regression, …).
    Webauthn(WebauthnError),
}

impl std::fmt::Display for PasskeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PasskeyError::UnknownCeremony => {
                write!(f, "passkey ceremony expired or unknown")
            }
            PasskeyError::CorruptCredential(e) => {
                write!(f, "stored passkey credential is corrupt: {e}")
            }
            PasskeyError::Webauthn(e) => write!(f, "webauthn error: {e}"),
        }
    }
}

impl std::error::Error for PasskeyError {}

impl From<WebauthnError> for PasskeyError {
    fn from(e: WebauthnError) -> Self {
        PasskeyError::Webauthn(e)
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Base64url-encode a credential id for use as the storage primary key and
/// the revoke-endpoint handle.
pub fn encode_credential_id(id: &CredentialID) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id.as_ref())
}

struct RegistrationCeremony {
    state: PasskeyRegistration,
    /// Principal the finish call must match (defense in depth: the ceremony
    /// id is only handed to the authenticated caller, but we re-check).
    user_name: String,
    expires_at: u64,
}

struct AuthenticationCeremony {
    state: PasskeyAuthentication,
    expires_at: u64,
}

/// The passkey ceremony engine. Constructed once at boot from the configured
/// RP-ID / origin when `passkey_enabled` is true; absent otherwise (the route
/// layer then answers `503`).
pub struct PasskeyEngine {
    webauthn: Webauthn,
    /// The principal passkeys authenticate as — the resolved dashboard user.
    /// Carried so the public authentication-options endpoint knows whose
    /// credentials to offer without a username being typed.
    principal: String,
    reg_states: Mutex<HashMap<String, RegistrationCeremony>>,
    auth_states: Mutex<HashMap<String, AuthenticationCeremony>>,
}

impl PasskeyEngine {
    /// Build the engine from the configured RP-ID and origin.
    ///
    /// `rp_id` must be the effective registrable domain of `rp_origin`
    /// (no scheme, no port). When `rp_id` is empty it is derived from the
    /// origin host; when `rp_origin` is empty it defaults to
    /// `http://<rp_id>` for local development.
    pub fn new(
        rp_id: &str,
        rp_origin: &str,
        principal: &str,
    ) -> Result<Self, PasskeyEngineBuildError> {
        let (rp_id, rp_origin) = resolve_rp(rp_id, rp_origin)?;
        let origin_url = Url::parse(&rp_origin).map_err(PasskeyEngineBuildError::Origin)?;
        let webauthn = WebauthnBuilder::new(&rp_id, &origin_url)
            .map_err(PasskeyEngineBuildError::Webauthn)?
            .rp_name("LibreFang")
            .build()
            .map_err(PasskeyEngineBuildError::Webauthn)?;
        Ok(Self {
            webauthn,
            principal: principal.to_string(),
            reg_states: Mutex::new(HashMap::new()),
            auth_states: Mutex::new(HashMap::new()),
        })
    }

    /// The principal passkeys authenticate as.
    pub fn principal(&self) -> &str {
        &self.principal
    }

    fn user_handle(name: &str) -> Uuid {
        Uuid::new_v5(&USER_HANDLE_NAMESPACE, name.as_bytes())
    }

    /// Start a registration ceremony for `user_name`. `existing` is the list
    /// of already-registered passkeys so the authenticator excludes them
    /// (no accidental double-enrollment of one device).
    pub fn start_registration(
        &self,
        user_name: &str,
        existing: &[Passkey],
    ) -> Result<(String, CreationChallengeResponse), PasskeyError> {
        let exclude: Vec<CredentialID> = existing.iter().map(|p| p.cred_id().clone()).collect();
        let (ccr, state) = self.webauthn.start_passkey_registration(
            Self::user_handle(user_name),
            user_name,
            user_name,
            Some(exclude),
        )?;
        let ceremony_id = generate_session_token().token;
        let mut states = self.reg_states.lock().expect("reg_states poisoned");
        prune_expired(&mut states, |c| c.expires_at);
        states.insert(
            ceremony_id.clone(),
            RegistrationCeremony {
                state,
                user_name: user_name.to_string(),
                expires_at: now_unix() + CEREMONY_TTL_SECS,
            },
        );
        Ok((ceremony_id, ccr))
    }

    /// Finish a registration ceremony, returning the freshly minted
    /// [`Passkey`] to be persisted. Verifies the ceremony belongs to
    /// `user_name`.
    pub fn finish_registration(
        &self,
        ceremony_id: &str,
        user_name: &str,
        reg: &RegisterPublicKeyCredential,
    ) -> Result<Passkey, PasskeyError> {
        let ceremony = {
            let mut states = self.reg_states.lock().expect("reg_states poisoned");
            states.remove(ceremony_id)
        }
        .filter(|c| c.expires_at > now_unix() && c.user_name == user_name)
        .ok_or(PasskeyError::UnknownCeremony)?;
        let passkey = self
            .webauthn
            .finish_passkey_registration(reg, &ceremony.state)?;
        Ok(passkey)
    }

    /// Start an authentication ceremony against `passkeys` (the principal's
    /// full credential list). Returns the ceremony id and the request
    /// challenge to hand the browser.
    pub fn start_authentication(
        &self,
        passkeys: &[Passkey],
    ) -> Result<(String, RequestChallengeResponse), PasskeyError> {
        let (rcr, state) = self.webauthn.start_passkey_authentication(passkeys)?;
        let ceremony_id = generate_session_token().token;
        let mut states = self.auth_states.lock().expect("auth_states poisoned");
        prune_expired(&mut states, |c| c.expires_at);
        states.insert(
            ceremony_id.clone(),
            AuthenticationCeremony {
                state,
                expires_at: now_unix() + CEREMONY_TTL_SECS,
            },
        );
        Ok((ceremony_id, rcr))
    }

    /// Finish an authentication ceremony, returning the
    /// [`AuthenticationResult`] (which carries the asserted credential id and
    /// the `needs_update` / counter-update signal). The route layer persists
    /// any sign-count change and mints the session.
    pub fn finish_authentication(
        &self,
        ceremony_id: &str,
        cred: &PublicKeyCredential,
    ) -> Result<AuthenticationResult, PasskeyError> {
        let ceremony = {
            let mut states = self.auth_states.lock().expect("auth_states poisoned");
            states.remove(ceremony_id)
        }
        .filter(|c| c.expires_at > now_unix())
        .ok_or(PasskeyError::UnknownCeremony)?;
        let result = self
            .webauthn
            .finish_passkey_authentication(cred, &ceremony.state)?;
        Ok(result)
    }
}

/// Remove expired ceremonies in place. Generic over the ceremony type so both
/// maps share one implementation.
fn prune_expired<C>(map: &mut HashMap<String, C>, expiry: impl Fn(&C) -> u64) {
    let now = now_unix();
    map.retain(|_, c| expiry(c) > now);
}

/// Errors building the engine at boot. Distinct from per-request
/// [`PasskeyError`] because these are fatal-at-startup misconfigurations.
#[derive(Debug)]
pub enum PasskeyEngineBuildError {
    /// Neither an RP-ID nor a usable origin could be determined.
    MissingRpId,
    /// The configured origin is not a valid URL.
    Origin(url::ParseError),
    /// `webauthn-rs` rejected the RP-ID/origin pairing.
    Webauthn(WebauthnError),
}

impl std::fmt::Display for PasskeyEngineBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PasskeyEngineBuildError::MissingRpId => write!(
                f,
                "passkey_rp_id is empty and could not be derived from passkey_rp_origin; \
                 set one of them in config.toml"
            ),
            PasskeyEngineBuildError::Origin(e) => write!(f, "invalid passkey_rp_origin: {e}"),
            PasskeyEngineBuildError::Webauthn(e) => {
                write!(f, "invalid passkey RP configuration: {e}")
            }
        }
    }
}

impl std::error::Error for PasskeyEngineBuildError {}

/// Resolve the effective `(rp_id, rp_origin)` pair from possibly-empty config.
///
/// - Both set → used verbatim.
/// - Only `rp_id` set → origin defaults to `http://<rp_id>` (local dev).
/// - Only `rp_origin` set → rp_id derived from the origin host.
/// - Neither set → [`PasskeyEngineBuildError::MissingRpId`].
fn resolve_rp(rp_id: &str, rp_origin: &str) -> Result<(String, String), PasskeyEngineBuildError> {
    let rp_id = rp_id.trim();
    let rp_origin = rp_origin.trim();
    match (rp_id.is_empty(), rp_origin.is_empty()) {
        (false, false) => Ok((rp_id.to_string(), rp_origin.to_string())),
        (false, true) => Ok((rp_id.to_string(), format!("http://{rp_id}"))),
        (true, false) => {
            let parsed = Url::parse(rp_origin).map_err(PasskeyEngineBuildError::Origin)?;
            let host = parsed
                .host_str()
                .ok_or(PasskeyEngineBuildError::MissingRpId)?
                .to_string();
            Ok((host, rp_origin.to_string()))
        }
        (true, true) => Err(PasskeyEngineBuildError::MissingRpId),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_rp_both_set_used_verbatim() {
        let (id, origin) = resolve_rp("example.com", "https://app.example.com").unwrap();
        assert_eq!(id, "example.com");
        assert_eq!(origin, "https://app.example.com");
    }

    #[test]
    fn resolve_rp_id_only_defaults_to_http_origin() {
        let (id, origin) = resolve_rp("localhost", "").unwrap();
        assert_eq!(id, "localhost");
        assert_eq!(origin, "http://localhost");
    }

    #[test]
    fn resolve_rp_origin_only_derives_host() {
        let (id, origin) = resolve_rp("", "https://dash.example.com:8443").unwrap();
        assert_eq!(id, "dash.example.com");
        assert_eq!(origin, "https://dash.example.com:8443");
    }

    #[test]
    fn resolve_rp_neither_is_error() {
        assert!(matches!(
            resolve_rp("", ""),
            Err(PasskeyEngineBuildError::MissingRpId)
        ));
    }

    #[test]
    fn user_handle_is_stable_and_distinct() {
        assert_eq!(
            PasskeyEngine::user_handle("admin"),
            PasskeyEngine::user_handle("admin")
        );
        assert_ne!(
            PasskeyEngine::user_handle("admin"),
            PasskeyEngine::user_handle("other")
        );
    }

    #[test]
    fn engine_builds_from_localhost() {
        let engine = PasskeyEngine::new("localhost", "http://localhost:4545", "admin").unwrap();
        assert_eq!(engine.principal(), "admin");
    }

    #[test]
    fn start_registration_returns_distinct_ceremony_ids() {
        let engine = PasskeyEngine::new("localhost", "http://localhost", "admin").unwrap();
        let (id1, _) = engine.start_registration("admin", &[]).unwrap();
        let (id2, _) = engine.start_registration("admin", &[]).unwrap();
        assert_ne!(id1, id2);
        assert_eq!(engine.reg_states.lock().unwrap().len(), 2);
    }

    #[test]
    fn finish_registration_rejects_unknown_ceremony() {
        let engine = PasskeyEngine::new("localhost", "http://localhost", "admin").unwrap();
        let reg: RegisterPublicKeyCredential =
            serde_json::from_str(SAMPLE_REG).expect("sample reg parses");
        let err = engine
            .finish_registration("does-not-exist", "admin", &reg)
            .unwrap_err();
        assert!(matches!(err, PasskeyError::UnknownCeremony));
    }

    #[test]
    fn finish_registration_rejects_principal_mismatch() {
        let engine = PasskeyEngine::new("localhost", "http://localhost", "admin").unwrap();
        let (id, _) = engine.start_registration("admin", &[]).unwrap();
        let reg: RegisterPublicKeyCredential =
            serde_json::from_str(SAMPLE_REG).expect("sample reg parses");
        // Right ceremony id, wrong principal → treated as unknown.
        let err = engine
            .finish_registration(&id, "intruder", &reg)
            .unwrap_err();
        assert!(matches!(err, PasskeyError::UnknownCeremony));
        // The ceremony was consumed by the attempt.
        assert_eq!(engine.reg_states.lock().unwrap().len(), 0);
    }

    // A syntactically valid (but cryptographically bogus) registration
    // payload — enough to exercise the ceremony-lookup branches without a
    // real authenticator. The crypto verification is never reached on these
    // paths because the ceremony lookup fails first.
    const SAMPLE_REG: &str = r#"{
        "id": "AAAA",
        "rawId": "AAAA",
        "response": {
            "attestationObject": "AAAA",
            "clientDataJSON": "AAAA"
        },
        "type": "public-key"
    }"#;
}
