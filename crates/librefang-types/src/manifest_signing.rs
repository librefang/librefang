//! Ed25519-based manifest signing for supply chain integrity.
//!
//! Agent manifests are TOML files that define an agent's capabilities,
//! tools, and configuration. A compromised or tampered manifest can grant
//! an agent elevated privileges. This module allows manifests to be
//! cryptographically signed so that the kernel can verify their integrity
//! and provenance before loading.
//!
//! The signing scheme:
//! 1. Compute SHA-256 of the manifest content.
//! 2. Sign the hash with Ed25519 (via `ed25519-dalek`).
//! 3. Bundle the signature, public key, and content hash into a
//!    `SignedManifest` envelope.
//!
//! Verification recomputes the hash and checks the Ed25519 signature
//! against the embedded public key.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A signed manifest envelope containing the original manifest text,
/// its content hash, the Ed25519 signature, and the signer's public key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedManifest {
    /// The raw manifest content (typically TOML).
    pub manifest: String,
    /// Hex-encoded SHA-256 hash of `manifest`.
    pub content_hash: String,
    /// Ed25519 signature bytes over `content_hash`.
    pub signature: Vec<u8>,
    /// The signer's Ed25519 public key bytes (32 bytes).
    pub signer_public_key: Vec<u8>,
    /// Human-readable identifier for the signer (e.g. email or key ID).
    pub signer_id: String,
}

/// Computes the hex-encoded SHA-256 hash of a manifest string.
pub fn hash_manifest(manifest: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(manifest.as_bytes());
    hex::encode(hasher.finalize())
}

impl SignedManifest {
    /// Signs a manifest with the given Ed25519 signing key.
    ///
    /// Returns a `SignedManifest` envelope ready for serialisation and
    /// distribution alongside (or instead of) the raw manifest file.
    pub fn sign(
        manifest: impl Into<String>,
        signing_key: &SigningKey,
        signer_id: impl Into<String>,
    ) -> Self {
        let manifest = manifest.into();
        let content_hash = hash_manifest(&manifest);
        let signature = signing_key.sign(content_hash.as_bytes());
        let verifying_key = signing_key.verifying_key();

        Self {
            manifest,
            content_hash,
            signature: signature.to_bytes().to_vec(),
            signer_public_key: verifying_key.to_bytes().to_vec(),
            signer_id: signer_id.into(),
        }
    }

    /// Verify the envelope's **internal consistency only** — the SHA-256
    /// still matches the manifest text and the signature is valid for that
    /// hash under the bundled `signer_public_key`.
    ///
    /// ⚠️ **This is not identity verification.** An attacker can generate
    /// their own keypair, sign any manifest with it, and embed the matching
    /// public key in the envelope — the envelope will still `verify()`
    /// successfully. This method is only safe for integrity checks where
    /// the caller already obtained `signer_public_key` out-of-band from a
    /// trusted channel.
    ///
    /// For supply-chain protection use [`Self::verify_with_trusted_keys`],
    /// which requires `signer_public_key` to match one of a caller-supplied
    /// trust-anchor list before accepting the signature.
    pub fn verify(&self) -> Result<(), String> {
        // Re-compute the hash and compare.
        let recomputed = hash_manifest(&self.manifest);
        if recomputed != self.content_hash {
            return Err(format!(
                "content hash mismatch: expected {} but manifest hashes to {}",
                self.content_hash, recomputed
            ));
        }

        // Reconstruct the public key.
        let pk_bytes: [u8; 32] = self
            .signer_public_key
            .as_slice()
            .try_into()
            .map_err(|_| "invalid public key length (expected 32 bytes)".to_string())?;
        let verifying_key = VerifyingKey::from_bytes(&pk_bytes)
            .map_err(|e| format!("invalid public key: {}", e))?;

        // Reconstruct the signature.
        let sig_bytes: [u8; 64] = self
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| "invalid signature length (expected 64 bytes)".to_string())?;
        let signature = Signature::from_bytes(&sig_bytes);

        // Verify.
        verifying_key
            .verify(self.content_hash.as_bytes(), &signature)
            .map_err(|e| format!("signature verification failed: {}", e))
    }

    /// Supply-chain-safe verification: requires the envelope's
    /// `signer_public_key` to byte-equal one of `trusted_keys` before
    /// running the normal integrity + signature check.
    ///
    /// `trusted_keys` is the caller's allowlist of 32-byte Ed25519 public
    /// keys — typically sourced from `KernelConfig.trusted_manifest_signers`.
    /// An empty list is treated as "no signers are trusted" and every
    /// envelope is rejected, so a misconfigured daemon fails closed instead
    /// of silently accepting self-signed envelopes.
    pub fn verify_with_trusted_keys(&self, trusted_keys: &[[u8; 32]]) -> Result<(), String> {
        if trusted_keys.is_empty() {
            return Err("manifest signature rejected: no trusted_manifest_signers \
                 configured — add the signer's Ed25519 public key to \
                 `trusted_manifest_signers` in config.toml"
                .to_string());
        }

        // Check the bundled signer_public_key is on the allowlist before we
        // do anything else. If the attacker embedded their own public key
        // this is the step that rejects the envelope.
        let pk_bytes: [u8; 32] = self
            .signer_public_key
            .as_slice()
            .try_into()
            .map_err(|_| "invalid public key length (expected 32 bytes)".to_string())?;
        if !trusted_keys.iter().any(|k| k == &pk_bytes) {
            return Err(format!(
                "manifest signature rejected: signer {} is not in \
                 trusted_manifest_signers",
                self.signer_id
            ));
        }

        // Known-good signer — run the normal integrity / signature check.
        self.verify()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a deterministic signing key from a seed byte.
    /// Tests don't need cryptographic randomness — they need reproducibility.
    fn test_signing_key(seed: u8) -> SigningKey {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        SigningKey::from_bytes(&bytes)
    }

    #[test]
    fn test_sign_and_verify() {
        let signing_key = test_signing_key(1);
        let manifest = r#"
[agent]
name = "hello-world"
description = "A simple test agent"

[capabilities]
shell = false
network = false
"#;

        let signed = SignedManifest::sign(manifest, &signing_key, "test@librefang.ai");
        assert_eq!(signed.content_hash, hash_manifest(manifest));
        assert_eq!(signed.signer_id, "test@librefang.ai");
        assert!(signed.verify().is_ok());
    }

    #[test]
    fn test_tampered_fails() {
        let signing_key = test_signing_key(2);
        let manifest = "[agent]\nname = \"secure-agent\"\n";

        let mut signed = SignedManifest::sign(manifest, &signing_key, "signer-1");

        // Tamper with the manifest content after signing.
        signed.manifest = "[agent]\nname = \"evil-agent\"\nshell = true\n".to_string();

        let result = signed.verify();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("content hash mismatch"));
    }

    #[test]
    fn test_wrong_key_fails() {
        let signing_key = test_signing_key(3);
        let wrong_key = test_signing_key(4);

        let manifest = "[agent]\nname = \"test\"\n";
        let mut signed = SignedManifest::sign(manifest, &signing_key, "signer-a");

        // Replace the public key with a different key's public key.
        signed.signer_public_key = wrong_key.verifying_key().to_bytes().to_vec();

        let result = signed.verify();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("signature verification failed"));
    }

    /// Regression: the bare `verify()` method only checks envelope
    /// self-consistency, so an attacker-generated keypair produces a
    /// "valid" envelope. This is the vulnerability `verify_with_trusted_keys`
    /// exists to close.
    #[test]
    fn test_plain_verify_accepts_self_signed_attacker() {
        let attacker = test_signing_key(42);
        let evil = "[agent]\nname = \"evil\"\nshell = true\n";
        let signed = SignedManifest::sign(evil, &attacker, "attacker@evil");
        assert!(
            signed.verify().is_ok(),
            "plain verify() must not be relied on as supply-chain defence"
        );
    }

    #[test]
    fn test_trusted_verify_rejects_untrusted_signer() {
        let attacker = test_signing_key(42);
        let official = test_signing_key(1);
        let evil = "[agent]\nname = \"evil\"\nshell = true\n";
        let signed = SignedManifest::sign(evil, &attacker, "attacker@evil");

        let trusted: [[u8; 32]; 1] = [official.verifying_key().to_bytes()];
        let err = signed
            .verify_with_trusted_keys(&trusted)
            .expect_err("self-signed envelope must be rejected");
        assert!(
            err.contains("not in trusted_manifest_signers"),
            "err was {err}"
        );
    }

    #[test]
    fn test_trusted_verify_accepts_trusted_signer() {
        let official = test_signing_key(1);
        let manifest = "[agent]\nname = \"hello\"\n";
        let signed = SignedManifest::sign(manifest, &official, "official@librefang");

        let trusted: [[u8; 32]; 1] = [official.verifying_key().to_bytes()];
        signed
            .verify_with_trusted_keys(&trusted)
            .expect("trusted signer must pass");
    }

    #[test]
    fn test_trusted_verify_empty_allowlist_fails_closed() {
        let official = test_signing_key(1);
        let manifest = "[agent]\nname = \"hello\"\n";
        let signed = SignedManifest::sign(manifest, &official, "official@librefang");

        let err = signed
            .verify_with_trusted_keys(&[])
            .expect_err("empty trust list must fail closed");
        assert!(err.contains("no trusted_manifest_signers"), "err was {err}");
    }

    #[test]
    fn test_trusted_verify_rejects_tampered_manifest_from_trusted_signer() {
        let official = test_signing_key(1);
        let manifest = "[agent]\nname = \"hello\"\n";
        let mut signed = SignedManifest::sign(manifest, &official, "official@librefang");
        // Attacker swaps the manifest body but keeps the original signature.
        signed.manifest = "[agent]\nname = \"evil\"\nshell = true\n".to_string();

        let trusted: [[u8; 32]; 1] = [official.verifying_key().to_bytes()];
        let err = signed
            .verify_with_trusted_keys(&trusted)
            .expect_err("tampered manifest must be rejected even under trust check");
        assert!(err.contains("content hash mismatch"), "err was {err}");
    }
}
