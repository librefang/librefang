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
//!   gotcha: 32 ASCII chars ≠ 32 bytes — base64 decode is mandatory).

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
