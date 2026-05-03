//! Integration test for the `librefang vault rotate-key` workflow (#3651).
//!
//! The CLI subcommand wraps `CredentialVault::unlock_with_key`,
//! `verify_or_install_sentinel`, and `rewrap_with_new_key` — all from
//! `librefang-extensions`. This test exercises the same code path the CLI
//! drives so a regression in any of those building blocks (or in the
//! sentinel preservation contract) trips the test the CLI would have
//! caught at runtime.
//!
//! Spawning the CLI binary is intentionally avoided: `cmd_vault_rotate_key`
//! calls `std::process::exit` on every error path and reads
//! `LIBREFANG_VAULT_KEY_OLD` / `LIBREFANG_VAULT_KEY_NEW` from the global
//! process env, which makes parallel `cargo test` runs flaky. Driving the
//! library API directly is deterministic AND covers the actual rotation
//! invariants (vault re-encrypted under new key, old key rejected, sentinel
//! survives).

use librefang_extensions::vault::{CredentialVault, SENTINEL_KEY, SENTINEL_VALUE};
use zeroize::Zeroizing;

/// Build a 32-byte key whose every byte is `b`. Deterministic so test
/// failures are reproducible without `OsRng` noise.
fn key_filled(b: u8) -> Zeroizing<[u8; 32]> {
    Zeroizing::new([b; 32])
}

/// End-to-end: create vault with key A → store entries → rotate to key B →
/// reads work with B → reads fail with A → sentinel survives and verifies.
#[test]
fn rotate_key_end_to_end_replaces_master_key_and_preserves_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let vault_path = dir.path().join("vault.enc");

    let key_a = key_filled(0x11);
    let key_b = key_filled(0x22);
    assert_ne!(key_a.as_ref(), key_b.as_ref());

    // ── Phase 1: create vault under KEY A and populate it. ──────────────
    {
        let mut vault = CredentialVault::new(vault_path.clone());
        vault
            .init_with_key(key_a.clone())
            .expect("init under key A");
        vault
            .set(
                "API_KEY".to_string(),
                Zeroizing::new("groq-key-aaa".to_string()),
            )
            .expect("set API_KEY");
        vault
            .set(
                "REFRESH_TOKEN".to_string(),
                Zeroizing::new("rt-bbb".to_string()),
            )
            .expect("set REFRESH_TOKEN");
        // sanity: sentinel was pre-written by init_with_key (#3651).
        vault
            .verify_or_install_sentinel()
            .expect("sentinel present after init");
    }

    // ── Phase 2: rotate from KEY A → KEY B (matches what cmd_vault_rotate_key does). ──
    {
        let mut vault = CredentialVault::new(vault_path.clone());
        vault
            .unlock_with_key(key_a.clone())
            .expect("unlock under OLD key A");
        vault
            .verify_or_install_sentinel()
            .expect("sentinel verifies under OLD key");
        // Confirm we see exactly the two user entries (sentinel is hidden).
        let mut user_keys = vault.list_keys();
        user_keys.sort();
        assert_eq!(user_keys, vec!["API_KEY", "REFRESH_TOKEN"]);
        // Re-wrap under KEY B — atomic save + sentinel preserved.
        vault
            .rewrap_with_new_key(key_b.clone())
            .expect("rewrap with NEW key B");
    }

    // ── Phase 3: NEW key reads succeed and recover the original plaintext. ──
    {
        let mut vault = CredentialVault::new(vault_path.clone());
        vault
            .unlock_with_key(key_b.clone())
            .expect("unlock under NEW key B");
        assert_eq!(
            vault.get("API_KEY").expect("API_KEY present").as_str(),
            "groq-key-aaa"
        );
        assert_eq!(
            vault
                .get("REFRESH_TOKEN")
                .expect("REFRESH_TOKEN present")
                .as_str(),
            "rt-bbb"
        );
        // Sentinel survived rotation and verifies cleanly.
        vault
            .verify_or_install_sentinel()
            .expect("sentinel verifies under NEW key");
        // Sentinel is invisible to user-facing list_keys.
        let mut user_keys = vault.list_keys();
        user_keys.sort();
        assert_eq!(user_keys, vec!["API_KEY", "REFRESH_TOKEN"]);
    }

    // ── Phase 4: OLD key MUST now be rejected — that's the whole point. ──
    {
        let mut vault = CredentialVault::new(vault_path.clone());
        let result = vault.unlock_with_key(key_a);
        assert!(
            result.is_err(),
            "rotated vault must reject the OLD key A; got {result:?}"
        );
    }
}

/// Rotating to the SAME master key would be a no-op masking an operator
/// error. The CLI rejects it up-front; verify the underlying invariant
/// here too (the rewrap itself does succeed at the library layer because
/// it's just a re-encrypt-with-same-key, but the CLI guard prevents the
/// footgun before we get there).
#[test]
fn rewrap_with_identical_key_still_decrypts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let vault_path = dir.path().join("vault.enc");
    let key = key_filled(0x42);

    let mut vault = CredentialVault::new(vault_path.clone());
    vault.init_with_key(key.clone()).unwrap();
    vault
        .set("K".to_string(), Zeroizing::new("v".to_string()))
        .unwrap();
    // Library-level rewrap with the same key is allowed (idempotent re-encrypt
    // under fresh AES-GCM nonce/salt). The CLI layer is where the same-key
    // guard lives, intentionally — see `vault-rotate-same-key` in main.ftl.
    vault.rewrap_with_new_key(key.clone()).unwrap();

    // Re-open and confirm everything still works under the same key.
    let mut v2 = CredentialVault::new(vault_path);
    v2.unlock_with_key(key).unwrap();
    assert_eq!(v2.get("K").unwrap().as_str(), "v");
    v2.verify_or_install_sentinel().unwrap();
}

/// Rotation must preserve every reserved internal slot (only the sentinel
/// today, but future internal keys would benefit from the same guard).
/// Without `iter_all_entries` / sentinel-aware rewrap, the post-rotation
/// vault would be missing the sentinel and the boot path would refuse to
/// start under the new key.
#[test]
fn sentinel_round_trips_through_rotation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let vault_path = dir.path().join("vault.enc");
    let key_old = key_filled(0xAA);
    let key_new = key_filled(0xBB);

    let mut vault = CredentialVault::new(vault_path.clone());
    vault.init_with_key(key_old.clone()).unwrap();
    vault.rewrap_with_new_key(key_new.clone()).unwrap();
    drop(vault);

    let mut vault2 = CredentialVault::new(vault_path);
    vault2.unlock_with_key(key_new).unwrap();
    // Use iter_all_entries (which includes reserved keys) to inspect the
    // sentinel directly; verify_or_install_sentinel is also exercised.
    let sentinel_pair = vault2.iter_all_entries().find(|(k, _)| *k == SENTINEL_KEY);
    let (_, sv) = sentinel_pair.expect("sentinel must survive rotation");
    assert_eq!(sv, SENTINEL_VALUE, "sentinel value must round-trip exactly");
    vault2.verify_or_install_sentinel().unwrap();
}
