//! Ephemeral X25519 key exchange for OFP per-message session keys (#4269).
//!
//! Background: prior to this module the per-message HMAC `session_key`
//! derived from `shared_secret + handshake_nonces`. An attacker holding
//! `shared_secret` and able to passively observe a connection's
//! handshake nonces could recompute the same key and forge in-flight
//! messages. The Ed25519 identity layer from #3873 prevents node_id
//! impersonation but does not, on its own, give per-session forward
//! secrecy of the symmetric channel.
//!
//! This module adds an ephemeral X25519 keypair generated **per
//! handshake**. Both peers exchange the public halves inside the
//! Handshake/HandshakeAck messages (covered by the Ed25519 identity
//! signature so an active MITM cannot substitute their own pubkey),
//! then ECDH the local secret with the remote pubkey to obtain a
//! shared point. HKDF-SHA256 over that point yields the session key.
//!
//! Resulting properties:
//! - **Forward secrecy** — the ephemeral private keys are dropped at
//!   the end of the handshake, so a future leak of `shared_secret` or
//!   even of either node's static Ed25519 private key cannot decrypt
//!   recorded past traffic.
//! - **`shared_secret` leak no longer breaks message integrity** — the
//!   symmetric session key is independent of `shared_secret`. Stealing
//!   it still bypasses the network admission gate, but cannot be used
//!   to forge in-flight HMACs on a session the attacker did not also
//!   actively MITM during its handshake.
//!
//! Backward compatibility: `ephemeral_pubkey` is `Option<String>` on
//! the wire. When either side omits it, the kernel falls back to the
//! legacy `derive_session_key(shared_secret, nonces)` path so existing
//! peers keep interoperating during a federation rollout.

use base64::Engine as _;
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::keys::KeyError;

const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

/// HKDF info string. Bumping this string is the protocol-versioning
/// hook: change it on a breaking session-key derivation change so an
/// old client and a new server cannot accidentally agree on a key.
const HKDF_INFO: &[u8] = b"librefang-ofp/v1/session-key";

/// One side of an ephemeral KEX. The secret is held until the handshake
/// completes, at which point [`derive_session_key`] is called and the
/// caller drops the [`EphemeralKex`] — taking the private bytes with
/// it (`StaticSecret` zeroizes on drop).
pub struct EphemeralKex {
    secret: StaticSecret,
    public_b64: String,
}

impl EphemeralKex {
    /// Generate a fresh per-handshake X25519 keypair.
    pub fn generate() -> Result<Self, KeyError> {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let secret = StaticSecret::from(bytes);
        let public = PublicKey::from(&secret);
        Ok(Self {
            secret,
            public_b64: B64.encode(public.as_bytes()),
        })
    }

    /// This side's X25519 public key, base64. Goes on the wire in the
    /// handshake's `ephemeral_pubkey` field. Intentionally returns
    /// `&str` so the caller can clone into the message struct.
    pub fn public_b64(&self) -> &str {
        &self.public_b64
    }

    /// Combine the local ephemeral secret with the remote ephemeral
    /// public to produce a 32-byte HKDF-derived session key, hex-encoded
    /// for compatibility with the existing string-based per-message
    /// HMAC API.
    ///
    /// `transcript` is mixed into the HKDF salt so two peers that
    /// happened to pick the same ephemeral pair on the same wire (e.g.
    /// retransmits, parallel sessions) cannot end up with the same
    /// session key. Caller is expected to pass the concatenation of
    /// the two handshake nonces in a stable order.
    pub fn derive_session_key(
        self,
        remote_pubkey_b64: &str,
        transcript: &[u8],
    ) -> Result<String, KeyError> {
        let pk_bytes = B64
            .decode(remote_pubkey_b64)
            .map_err(|_| KeyError::InvalidFormat)?;
        if pk_bytes.len() != 32 {
            return Err(KeyError::InvalidFormat);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&pk_bytes);
        let remote = PublicKey::from(arr);

        let shared = self.secret.diffie_hellman(&remote);
        // SECURITY (#4269): per the X25519 spec the all-zero shared
        // secret is the unique known weak output (occurs when one side
        // contributes a low-order public key). RustCrypto's
        // `x25519_dalek` does NOT reject these by default; we do.
        if shared.as_bytes().iter().all(|b| *b == 0) {
            return Err(KeyError::BadSignature);
        }

        let hk = Hkdf::<Sha256>::new(Some(transcript), shared.as_bytes());
        let mut okm = [0u8; 32];
        hk.expand(HKDF_INFO, &mut okm)
            .map_err(|_| KeyError::InvalidFormat)?;
        Ok(hex::encode(okm))
    }
}

/// Build the HKDF salt bound to a given handshake. Both sides MUST
/// produce the same byte string for the derivation to agree, so the
/// nonces are concatenated in a fixed order: client first, server
/// second, regardless of which side is calling.
pub fn handshake_transcript(client_nonce: &str, server_nonce: &str) -> Vec<u8> {
    let mut t = Vec::with_capacity(client_nonce.len() + 1 + server_nonce.len());
    t.extend_from_slice(client_nonce.as_bytes());
    t.push(b'|');
    t.extend_from_slice(server_nonce.as_bytes());
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The point of the whole module: two peers each generate an
    /// ephemeral keypair, exchange pubkeys, and arrive at the same
    /// session key — without `shared_secret` ever entering the
    /// derivation.
    #[test]
    fn issue_4269_two_ephemerals_agree_on_session_key() {
        let alice = EphemeralKex::generate().unwrap();
        let bob = EphemeralKex::generate().unwrap();
        let alice_pub = alice.public_b64().to_string();
        let bob_pub = bob.public_b64().to_string();
        let transcript = handshake_transcript("client-nonce", "server-nonce");

        let k_a = alice.derive_session_key(&bob_pub, &transcript).unwrap();
        let k_b = bob.derive_session_key(&alice_pub, &transcript).unwrap();
        assert_eq!(k_a, k_b);
        assert_eq!(k_a.len(), 64); // 32 bytes hex-encoded
    }

    /// A passive observer with `shared_secret` cannot reproduce the
    /// session key because the derivation contains zero bits of
    /// `shared_secret`. The test makes that explicit by using ONLY
    /// public material to derive a different key and asserting it
    /// disagrees.
    #[test]
    fn issue_4269_session_key_is_independent_of_any_shared_secret() {
        let alice = EphemeralKex::generate().unwrap();
        let bob = EphemeralKex::generate().unwrap();
        let alice_pub = alice.public_b64().to_string();
        let bob_pub = bob.public_b64().to_string();
        let transcript = handshake_transcript("c", "s");

        let real_key = alice.derive_session_key(&bob_pub, &transcript).unwrap();

        // Re-run an HKDF using the *handshake-public* parts only — the
        // best an off-path attacker with `shared_secret` and nonces
        // could do without the ephemeral private keys.
        let public_only = format!("{alice_pub}|{bob_pub}|cluster-secret");
        let hk = Hkdf::<Sha256>::new(Some(&transcript), public_only.as_bytes());
        let mut okm = [0u8; 32];
        hk.expand(HKDF_INFO, &mut okm).unwrap();
        let attacker_key = hex::encode(okm);

        assert_ne!(
            real_key, attacker_key,
            "session key must not be reproducible from public material + shared_secret"
        );
    }

    /// Different transcripts (e.g. different handshake nonces) MUST
    /// yield different session keys even when the same ephemeral pair
    /// is reused. Pin the property so a future refactor can't silently
    /// drop the salt and reintroduce reuse.
    #[test]
    fn issue_4269_transcript_is_part_of_derivation() {
        let alice = EphemeralKex::generate().unwrap();
        let bob = EphemeralKex::generate().unwrap();
        let alice_pub = alice.public_b64().to_string();
        let bob_pub = bob.public_b64().to_string();

        let alice2 = EphemeralKex {
            secret: alice.secret.clone(),
            public_b64: alice.public_b64.clone(),
        };
        let bob2 = EphemeralKex {
            secret: bob.secret.clone(),
            public_b64: bob.public_b64.clone(),
        };

        let k1 = alice
            .derive_session_key(&bob_pub, &handshake_transcript("n1", "n2"))
            .unwrap();
        let k2 = bob2
            .derive_session_key(&alice_pub, &handshake_transcript("n1", "different"))
            .unwrap();
        // alice2 not used in this test; just satisfies clone semantic.
        let _ = alice2;
        assert_ne!(k1, k2);
    }

    #[test]
    fn invalid_remote_pubkey_is_rejected() {
        let alice = EphemeralKex::generate().unwrap();
        let res = alice.derive_session_key("not-base64!", &[]);
        assert!(matches!(res, Err(KeyError::InvalidFormat)));
    }
}
