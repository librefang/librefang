//! Integration tests for the credential vault (#3696).
//!
//! Covers the encrypt → persist → reload → decrypt round-trip with an
//! explicit master key (no OS keyring, no env-var dependency on the host),
//! plus a few invariants the rest of the daemon relies on:
//!
//! - A vault initialised with key K can only be unlocked with the same key K
//!   (wrong-key path surfaces an error instead of silently corrupting state).
//! - Reserved internal keys (the #3651 sentinel) are not visible via the
//!   public `list_keys` API and cannot be overwritten via `set` / `remove`.
//! - `decode_master_key` enforces the documented 32-byte length (CLAUDE.md
//!   gotcha: 32 ASCII chars ≠ 32 bytes — base64 decode is mandatory), and
//!   surfaces a structured error for both the literal 32-ASCII-char paste
//!   (valid base64 → 24 bytes) and non-base64 input, instead of silently
//!   booting with a truncated key.

use base64::Engine as _;
use librefang_extensions::vault::{decode_master_key, CredentialVault, SENTINEL_KEY};
use librefang_extensions::ExtensionError;
use tempfile::TempDir;
use zeroize::Zeroizing;

/// Generate a deterministic 32-byte key (base64 encoded) suitable for tests.
/// Mirrors the production contract: callers MUST supply a 32-byte key (the
/// `openssl rand -base64 32` recipe yields exactly 44 chars decoding to 32
/// bytes).
fn fixture_key_b64() -> String {
    // Use the all-zeros key. Tests don't care about cryptographic strength;
    // only that the key round-trips through `decode_master_key` and that the
    // same key value reproduces the same vault contents on reopen.
    let raw = [0u8; 32];
    base64::engine::general_purpose::STANDARD.encode(raw)
}

fn fixture_vault_path(tmp: &TempDir) -> std::path::PathBuf {
    tmp.path().join("vault.enc")
}

#[test]
fn decode_master_key_rejects_wrong_byte_length() {
    // 32 ASCII chars decoded as base64 yields 24 bytes — not 32. This test
    // pins the gotcha called out in CLAUDE.md so a future caller can't paste
    // a 32-char ASCII string and silently boot with a 24-byte truncated key.
    let too_short_b64 = base64::engine::general_purpose::STANDARD.encode([0u8; 24]);
    let err = decode_master_key(&too_short_b64).expect_err("24 bytes must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("Invalid key length"),
        "expected length-error, got {msg:?}"
    );

    // The happy path: 32 raw bytes round-trip cleanly.
    let ok_b64 = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
    let key = decode_master_key(&ok_b64).expect("32 bytes must decode");
    assert_eq!(key.as_ref(), &[7u8; 32]);
}

/// The classic LIBREFANG_VAULT_KEY foot-gun (CLAUDE.md gotcha): an operator
/// pastes a literal 32-character ASCII string, assuming "32 characters = 32
/// bytes". It is not — base64 decoding is mandatory. This test feeds the
/// literal string forms (not a value constructed by re-encoding 24 bytes,
/// which `decode_master_key_rejects_wrong_byte_length` above already covers)
/// and pins that BOTH failure modes surface a structured error rather than a
/// silently mis-decoded short key.
#[test]
fn decode_master_key_rejects_literal_32_ascii_chars() {
    // Case 1: 32 chars that ARE valid base64 (all in the alphabet, length a
    // multiple of 4) but decode to 24 bytes, not 32. This is the exact value
    // an operator gets by typing 32 random-looking characters that happen to
    // be base64-legal. The length guard must reject it.
    let thirty_two_ascii = "x".repeat(32);
    assert_eq!(thirty_two_ascii.len(), 32, "fixture must be 32 ASCII chars");
    let err = decode_master_key(&thirty_two_ascii)
        .expect_err("a 32-char ASCII string must not decode to a 32-byte key");
    let msg = err.to_string();
    assert!(
        msg.contains("Invalid key length") && msg.contains("32"),
        "expected a length error mentioning the 32-byte requirement, got {msg:?}"
    );

    // Case 2: 32 chars containing characters OUTSIDE the base64 alphabet.
    // These cannot decode at all, so the base64 decode branch must fire with
    // its own structured error (not a panic, not a truncated key).
    let thirty_two_non_base64 = "!".repeat(32);
    assert_eq!(thirty_two_non_base64.len(), 32);
    let err =
        decode_master_key(&thirty_two_non_base64).expect_err("non-base64 input must be rejected");
    assert!(
        err.to_string().contains("decode"),
        "expected a base64 decode error, got {:?}",
        err.to_string()
    );

    // Sanity anchor: the documented correct recipe (`openssl rand -base64 32`
    // produces a 44-char string decoding to 32 bytes) is accepted. We mirror
    // that shape here so the contrast with the rejected 32-char form is
    // explicit in one place.
    let correct_b64 = base64::engine::general_purpose::STANDARD.encode([0xABu8; 32]);
    assert_eq!(
        correct_b64.len(),
        44,
        "openssl rand -base64 32 yields 44 chars"
    );
    let key = decode_master_key(&correct_b64).expect("44-char base64 must decode to 32 bytes");
    assert_eq!(key.as_ref(), &[0xABu8; 32]);
}

#[test]
fn vault_roundtrip_encrypt_then_decrypt_with_same_key() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture_vault_path(&tmp);
    let key = decode_master_key(&fixture_key_b64()).unwrap();

    // Phase 1: initialise, write a few entries, drop.
    {
        let mut vault = CredentialVault::new(path.clone());
        vault
            .init_with_key(Zeroizing::new(*key))
            .expect("init must succeed on a fresh path");
        assert!(vault.is_unlocked());

        vault
            .set(
                "OPENAI_API_KEY".to_string(),
                Zeroizing::new("sk-test-openai".to_string()),
            )
            .unwrap();
        vault
            .set(
                "ANTHROPIC_API_KEY".to_string(),
                Zeroizing::new("sk-ant-test".to_string()),
            )
            .unwrap();
        // vault drops here — entries are zeroed in memory; only the encrypted
        // file at `path` survives.
    }

    // Phase 2: reopen with the same key — entries must be recoverable.
    let mut vault = CredentialVault::new(path);
    vault
        .unlock_with_key(Zeroizing::new(*key))
        .expect("unlock with the same key must succeed");

    assert_eq!(
        vault.get("OPENAI_API_KEY").map(|v| v.as_str().to_string()),
        Some("sk-test-openai".to_string())
    );
    assert_eq!(
        vault
            .get("ANTHROPIC_API_KEY")
            .map(|v| v.as_str().to_string()),
        Some("sk-ant-test".to_string())
    );

    // list_keys must surface user-facing keys but hide reserved internals.
    let keys: std::collections::BTreeSet<&str> = vault.list_keys().into_iter().collect();
    assert!(keys.contains("OPENAI_API_KEY"));
    assert!(keys.contains("ANTHROPIC_API_KEY"));
    assert!(
        !keys.contains(SENTINEL_KEY),
        "list_keys must filter out the #3651 sentinel"
    );
}

#[test]
fn vault_unlock_with_wrong_key_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture_vault_path(&tmp);

    // Initialise under key A.
    let key_a = decode_master_key(&fixture_key_b64()).unwrap();
    {
        let mut vault = CredentialVault::new(path.clone());
        vault.init_with_key(Zeroizing::new(*key_a)).unwrap();
        vault
            .set(
                "K".to_string(),
                Zeroizing::new("sensitive-value".to_string()),
            )
            .unwrap();
    }

    // Try to unlock under key B (different bytes). AES-GCM authenticates the
    // ciphertext, so a wrong key MUST fail loudly rather than yield garbage
    // plaintext. This is the contract the boot path depends on (#3651).
    let key_b_b64 = base64::engine::general_purpose::STANDARD.encode([1u8; 32]);
    let key_b = decode_master_key(&key_b_b64).unwrap();

    let mut vault = CredentialVault::new(path);
    let err = vault
        .unlock_with_key(Zeroizing::new(*key_b))
        .expect_err("unlock with the wrong key must fail");
    // Either flavour of vault error is acceptable — the contract is just
    // "non-Ok"; we don't pin the variant because the underlying AES-GCM
    // failure message has historically been routed through both `Vault` and
    // `VaultKeyMismatch` depending on the format version.
    assert!(
        matches!(
            err,
            ExtensionError::Vault(_) | ExtensionError::VaultKeyMismatch { .. }
        ),
        "wrong-key unlock should surface a Vault error, got {err:?}"
    );
    assert!(
        !vault.is_unlocked(),
        "vault must not transition to unlocked after a failed key check"
    );
}

#[test]
fn vault_rejects_writes_to_reserved_sentinel_key() {
    // The #3651 sentinel is owned by the vault implementation. External
    // callers must not be able to overwrite or remove it via the public API,
    // because doing so would silently break the boot-path verify branch.
    let tmp = tempfile::tempdir().unwrap();
    let path = fixture_vault_path(&tmp);
    let key = decode_master_key(&fixture_key_b64()).unwrap();

    let mut vault = CredentialVault::new(path);
    vault.init_with_key(Zeroizing::new(*key)).unwrap();

    let set_err = vault
        .set(
            SENTINEL_KEY.to_string(),
            Zeroizing::new("attacker-payload".to_string()),
        )
        .expect_err("set on sentinel must be refused");
    assert!(
        matches!(set_err, ExtensionError::Vault(_)),
        "sentinel write must surface as Vault error, got {set_err:?}"
    );

    let remove_err = vault
        .remove(SENTINEL_KEY)
        .expect_err("remove on sentinel must be refused");
    assert!(
        matches!(remove_err, ExtensionError::Vault(_)),
        "sentinel remove must surface as Vault error, got {remove_err:?}"
    );
}
