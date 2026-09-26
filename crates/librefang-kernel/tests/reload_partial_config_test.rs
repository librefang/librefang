//! #8459 — a config reload must not revert a field the on-disk document does not state.
//!
//! Every load funnels through `try_load_config`, which ends by deserializing the resolved document
//! into a `KernelConfig`. `KernelConfig` carries **container-level** `#[serde(default)]`, and that
//! fills each absent field from `KernelConfig::default()` — not from the field type's own `Default`.
//! So a document that parses but is *partial* does not leave the live values alone: it resets
//! everything it does not mention to the compiled default.
//!
//! The hazard is documented as already closed at `crates/librefang-kernel/src/config.rs:209`, and it
//! is, for a file that fails to parse. Strictness stops at parseability, which is what this pins.
//!
//! A partial document is not hypothetical. `persist_identity_sections`
//! (`crates/librefang-api/src/routes/users.rs:1368`) reads the existing file and rewrites only the
//! sections it manages, so on a kernel whose `config_path` does not exist yet — an in-memory boot,
//! the desktop, every integration test, and a deployment pointing `LIBREFANG_CONFIG_PATH` at a fresh
//! path — the first write produces a `config.toml` stating `users` and nothing else. A deployment
//! supplying its token through `LIBREFANG_API_KEY` (`config.rs:121`) has an `api_key` in memory that
//! no file states, which is why the assertion below is not about a path.

use librefang_testing::MockKernelBuilder;

/// The document a partial writer leaves behind when there is nothing to read first.
const PARTIAL_CONFIG: &str = "[[users]]\nname = \"Alice\"\nrole = \"user\"\n";

#[tokio::test(flavor = "multi_thread")]
async fn a_reload_keeps_the_fields_the_document_does_not_state() {
    let (kernel, tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.api_key = "a-key-no-file-states".to_string();
            c.default_model.model = "a-model-no-file-states".to_string();
        })
        .build();

    let before = kernel.config_snapshot();
    let (home_before, key_before, model_before) = (
        before.home_dir.clone(),
        before.api_key.clone(),
        before.default_model.model.clone(),
    );
    drop(before);

    // The document states `users`. It does not state a home, a key or a model.
    std::fs::write(tmp.path().join("config.toml"), PARTIAL_CONFIG)
        .expect("write the partial config in the kernel's own config path");

    kernel
        .reload_config()
        .await
        .expect("a document that parses must reload");

    let after = kernel.config_snapshot();
    assert_eq!(
        after.home_dir, home_before,
        "a reload must not invent a home directory the document does not state"
    );
    assert_eq!(
        after.api_key, key_before,
        "a reload must not invent an API key the document does not state"
    );
    assert_eq!(
        after.default_model.model, model_before,
        "a reload must not invent a default model the document does not state"
    );

    kernel.shutdown();
}

/// The same fault, reached the way it was actually found: through the API's config-write path,
/// which creates the partial document itself rather than being handed one.
///
/// Kept separate from the test above because it fails for the same reason but from a different
/// direction — if only one of them goes green after a fix, the fix is in the wrong place.
#[tokio::test(flavor = "multi_thread")]
async fn a_reload_keeps_a_key_the_writer_never_stated() {
    let (kernel, tmp) = MockKernelBuilder::new()
        .with_config(|c| c.api_key = "a-key-no-file-states".to_string())
        .build();

    let key_before = kernel.config_snapshot().api_key.clone();

    // What `persist_identity_sections` leaves behind on a kernel whose `config_path` did not exist:
    // it starts from an empty document and inserts only the sections it owns.
    std::fs::write(tmp.path().join("config.toml"), PARTIAL_CONFIG).expect("write");

    kernel.reload_config().await.expect("reload");

    assert_eq!(
        kernel.config_snapshot().api_key,
        key_before,
        "the write that created this document owns `users`; it must not be read as owning the key"
    );

    kernel.shutdown();
}

/// A document that spells a documented alias must reload instead of failing on a duplicate field.
///
/// `serde_json::to_value(base)` emits the canonical name (`api_listen`) while the document keeps
/// whatever the operator wrote (`listen_addr`), so merging the two as plain maps left both keys in
/// one object and serde rejected the result with `duplicate field api_listen`. Every reload of such
/// a config failed, while boot still worked because it passes no base and no overlay runs.
#[tokio::test(flavor = "multi_thread")]
async fn a_reload_accepts_a_documented_alias_for_a_stated_field() {
    let (kernel, tmp) = MockKernelBuilder::new()
        .with_config(|c| c.api_listen = "127.0.0.1:4545".to_string())
        .build();

    std::fs::write(
        tmp.path().join("config.toml"),
        "listen_addr = \"0.0.0.0:9999\"\n",
    )
    .expect("write the document that uses the alias");

    kernel.reload_config().await.expect(
        "the alias names the same field serde accepts at boot; the reload must not reject it",
    );

    assert_eq!(
        kernel.config_snapshot().api_listen,
        "0.0.0.0:9999",
        "the alias must land in the canonical field"
    );

    kernel.shutdown();
}

/// A stated map replaces the live one wholesale, so an entry the document omits is revoked.
///
/// `provider_api_keys` is a `BTreeMap`, and a recursive overlay keeps every entry the document does
/// not restate — an operator rotating or revoking one provider key writes the stated table without
/// that entry and the key stays in use, with the reload reporting no change.
#[tokio::test(flavor = "multi_thread")]
async fn a_reload_drops_a_map_entry_the_stated_table_omits() {
    let (kernel, tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.provider_api_keys
                .insert("openai".to_string(), "OPENAI_API_KEY".to_string());
            c.provider_api_keys
                .insert("anthropic".to_string(), "ANTHROPIC_API_KEY".to_string());
        })
        .build();

    std::fs::write(
        tmp.path().join("config.toml"),
        "[provider_api_keys]\nanthropic = \"ANTHROPIC_API_KEY\"\n",
    )
    .expect("write the document that revokes the openai entry");

    kernel.reload_config().await.expect("reload");

    let after = kernel.config_snapshot();
    assert!(
        !after.provider_api_keys.contains_key("openai"),
        "the stated table must replace the live map, not merge into it: {:?}",
        after.provider_api_keys
    );
    assert_eq!(
        after.provider_api_keys.get("anthropic").map(String::as_str),
        Some("ANTHROPIC_API_KEY"),
    );

    kernel.shutdown();
}
