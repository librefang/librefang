use super::install::parse_plugin_i18n_blocks;
use super::registry::{
    is_valid_registry_pubkey_b64, registry_index_urls, registry_pubkey_cache_path,
    EMBEDDED_REGISTRY_PUBKEYS,
};
use super::*;

#[test]
fn test_plugins_dir() {
    let dir = plugins_dir();
    assert!(dir.ends_with("plugins"));
    assert!(dir.to_string_lossy().contains(".librefang"));
}

#[test]
fn test_list_plugins_no_panic() {
    // Should not panic even if plugins dir doesn't exist
    let _ = list_plugins();
}

#[test]
fn test_get_plugin_not_installed() {
    let result = get_plugin_info("nonexistent-test-plugin-xyz");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not installed"));
}

#[test]
fn test_remove_not_installed() {
    let result = remove_plugin("nonexistent-test-plugin-xyz");
    assert!(result.is_err());
}

#[test]
fn test_scaffold_and_remove() {
    let tmp = tempfile::tempdir().unwrap();
    // Override HOME to use temp dir
    let plugin_dir = tmp.path().join("test-scaffold-plugin");
    std::fs::create_dir_all(&plugin_dir).unwrap();

    // Test manifest parsing from scaffold content
    let manifest_content = r#"name = "test-scaffold"
version = "0.1.0"
description = "Test scaffold"
author = ""

[hooks]
ingest = "hooks/ingest.py"
after_turn = "hooks/after_turn.py"
"#;
    let manifest: PluginManifest = toml::from_str(manifest_content).unwrap();
    assert_eq!(manifest.name, "test-scaffold");
    assert_eq!(manifest.version, "0.1.0");
    assert_eq!(manifest.hooks.ingest.as_deref(), Some("hooks/ingest.py"));
    assert_eq!(
        manifest.hooks.after_turn.as_deref(),
        Some("hooks/after_turn.py")
    );
}

#[test]
fn test_copy_dir_recursive() {
    let tmp_src = tempfile::tempdir().unwrap();
    let tmp_dst = tempfile::tempdir().unwrap();

    // Create source structure
    std::fs::create_dir_all(tmp_src.path().join("hooks")).unwrap();
    std::fs::write(tmp_src.path().join("plugin.toml"), "name = \"test\"").unwrap();
    std::fs::write(tmp_src.path().join("hooks/ingest.py"), "# hook").unwrap();

    let dst = tmp_dst.path().join("copied");
    copy_dir_recursive(tmp_src.path(), &dst).unwrap();

    assert!(dst.join("plugin.toml").exists());
    assert!(dst.join("hooks/ingest.py").exists());
}

#[test]
fn test_dir_size() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "hello").unwrap();
    std::fs::write(tmp.path().join("b.txt"), "world!").unwrap();
    let size = dir_size(tmp.path());
    assert_eq!(size, 11); // 5 + 6
}

#[test]
fn test_check_hooks_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let plugin_dir = tmp.path().to_path_buf();
    std::fs::create_dir_all(plugin_dir.join("hooks")).unwrap();
    std::fs::write(plugin_dir.join("hooks/ingest.py"), "").unwrap();

    let manifest = PluginManifest {
        name: "test".to_string(),
        version: "0.1.0".to_string(),
        hooks: librefang_types::config::ContextEngineHooks {
            ingest: Some("hooks/ingest.py".to_string()),
            after_turn: Some("hooks/after_turn.py".to_string()), // missing
            ..Default::default()
        },
        ..Default::default()
    };

    assert!(!check_hooks_exist(&plugin_dir, &manifest));

    // Now create the missing file
    std::fs::write(plugin_dir.join("hooks/after_turn.py"), "").unwrap();
    assert!(check_hooks_exist(&plugin_dir, &manifest));

    // Path traversal: hook pointing outside plugin dir should fail
    let manifest_escape = PluginManifest {
        name: "test".to_string(),
        version: "0.1.0".to_string(),
        hooks: librefang_types::config::ContextEngineHooks {
            ingest: Some("../../etc/passwd".to_string()),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(!check_hooks_exist(&plugin_dir, &manifest_escape));
}

/// Live listing smoke test — ensures the enriched listing populates
/// `description`/`version`/`hooks` from at least one plugin's `plugin.toml`.
/// Ignored by default — requires network access to GitHub.
#[tokio::test]
#[ignore]
async fn test_list_registry_plugins_enriched() {
    // Skip disk cache so a cached name-only listing from a previous run
    // cannot mask a regression.
    // SAFETY: this test is marked #[ignore] and only runs explicitly (not
    // in parallel); no other test thread races on this env var.
    unsafe { std::env::set_var("LIBREFANG_REGISTRY_NO_CACHE", "1") };
    let entries = list_registry_plugins("librefang/librefang-registry")
        .await
        .expect("registry listing should succeed");
    assert!(!entries.is_empty(), "expected at least one plugin");
    assert!(
        entries.iter().any(|e| e.description.is_some()),
        "expected at least one plugin with a description"
    );
    assert!(
        entries.iter().any(|e| e.version.is_some()),
        "expected at least one plugin with a version"
    );
    assert!(
        entries.iter().any(|e| !e.hooks.is_empty()),
        "expected at least one plugin declaring hooks"
    );
}

/// Integration test: install from GitHub registry, run hook, then remove.
/// Ignored by default — requires network access.
#[tokio::test]
#[ignore]
async fn test_registry_install_run_remove() {
    // 1. Install echo-memory from registry
    let source = PluginSource::Registry {
        name: "echo-memory".to_string(),
        github_repo: None,
    };
    let info = install_plugin(&source)
        .await
        .expect("registry install failed");
    assert_eq!(info.manifest.name, "echo-memory");
    assert_eq!(info.manifest.version, "0.1.0");
    assert!(info.hooks_valid);

    // 2. List should include it
    let plugins = list_plugins();
    assert!(plugins.iter().any(|p| p.manifest.name == "echo-memory"));

    // 3. Run ingest hook
    let ingest_path = info.path.join("hooks/ingest.py");
    assert!(ingest_path.exists());

    let mut child = tokio::process::Command::new("python3")
        .arg(&ingest_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("python3 should be available");

    {
        use tokio::io::AsyncWriteExt;
        let stdin = child.stdin.as_mut().unwrap();
        stdin
            .write_all(br#"{"type":"ingest","agent_id":"test-001","message":"Hello world"}"#)
            .await
            .unwrap();
    }
    child.stdin.take(); // close stdin
    let out = child.wait_with_output().await.unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ingest_result"), "got: {stdout}");
    assert!(stdout.contains("echo-memory"), "got: {stdout}");

    // 4. Remove
    remove_plugin("echo-memory").expect("remove failed");
    assert!(get_plugin_info("echo-memory").is_err());
}

/// Sanity: a manifest with no `[i18n.*]` tables yields an empty map,
/// not a serialization error or panic.
#[test]
fn parse_plugin_i18n_no_block() {
    let toml_str = r#"
name = "test-plugin"
version = "0.1.0"
description = "English description"
"#;
    let value: toml::Value = toml::from_str(toml_str).unwrap();
    let i18n = parse_plugin_i18n_blocks(&value);
    assert!(i18n.is_empty());
}

/// Multiple `[i18n.<lang>]` blocks with both fields populate cleanly.
#[test]
fn parse_plugin_i18n_multi_lang() {
    let toml_str = r#"
name = "auto-summarizer"
version = "0.1.0"
description = "English description"

[i18n.zh]
name = "自动摘要"
description = "持续维护会话摘要。"

[i18n.zh-TW]
name = "自動摘要"
description = "持續維護會話摘要。"

[i18n.fr]
name = "Auto-résumé"
description = "Maintient un résumé continu."
"#;
    let value: toml::Value = toml::from_str(toml_str).unwrap();
    let i18n = parse_plugin_i18n_blocks(&value);
    assert_eq!(i18n.len(), 3);
    assert_eq!(i18n["zh"].name.as_deref(), Some("自动摘要"));
    assert_eq!(i18n["zh-TW"].name.as_deref(), Some("自動摘要"));
    assert_eq!(
        i18n["fr"].description.as_deref(),
        Some("Maintient un résumé continu.")
    );
}

/// A block that only sets `name` (no description) survives, with
/// description left as `None` so callers know to fall back.
#[test]
fn parse_plugin_i18n_partial_entry() {
    let toml_str = r#"
[i18n.de]
name = "Beispiel"
"#;
    let value: toml::Value = toml::from_str(toml_str).unwrap();
    let i18n = parse_plugin_i18n_blocks(&value);
    assert_eq!(i18n.len(), 1);
    assert_eq!(i18n["de"].name.as_deref(), Some("Beispiel"));
    assert!(i18n["de"].description.is_none());
}

/// A `[i18n.<lang>]` block that sets neither field is dropped — keeping
/// it would just take memory for no observable effect at the API
/// boundary.
#[test]
fn parse_plugin_i18n_empty_entry_dropped() {
    let toml_str = r#"
[i18n.ja]
"#;
    let value: toml::Value = toml::from_str(toml_str).unwrap();
    let i18n = parse_plugin_i18n_blocks(&value);
    assert!(i18n.is_empty(), "empty i18n.ja entry should not be kept");
}

/// Non-string `name` / `description` values (e.g. someone wrote a
/// number by mistake) are silently ignored rather than panicking.
#[test]
fn parse_plugin_i18n_non_string_values_ignored() {
    let toml_str = r#"
[i18n.es]
name = 42
description = "Spanish description"
"#;
    let value: toml::Value = toml::from_str(toml_str).unwrap();
    let i18n = parse_plugin_i18n_blocks(&value);
    assert_eq!(i18n.len(), 1);
    assert!(i18n["es"].name.is_none(), "non-string name dropped");
    assert_eq!(
        i18n["es"].description.as_deref(),
        Some("Spanish description")
    );
}

// ── #3805 — registry pubkey resolver (env > TOFU cache > worker fetch) ──

/// Round-trip: a 32-byte non-zero key encodes/decodes through the
/// validator. This is the shape the resolver, the worker keygen script,
/// and ed25519_dalek all agree on.
#[test]
fn valid_registry_pubkey_b64_accepts_real_key() {
    use base64::Engine as _;
    let real_key = [0xABu8; 32];
    let b64 = base64::engine::general_purpose::STANDARD.encode(real_key);
    assert!(
        is_valid_registry_pubkey_b64(&b64),
        "non-zero 32-byte key must validate"
    );
}

/// The validator must reject the historical all-zero placeholder, garbage
/// base64, and wrong-length keys — the three failure modes that fall back
/// to the next link of the resolver chain.
#[test]
fn valid_registry_pubkey_b64_rejects_invalid_inputs() {
    use base64::Engine as _;

    // All-zero 32-byte key (legacy placeholder) — rejected.
    let zero = base64::engine::general_purpose::STANDARD.encode([0u8; 32]);
    assert!(
        !is_valid_registry_pubkey_b64(&zero),
        "all-zero placeholder must be rejected"
    );

    // Wrong length (16 bytes) — rejected.
    let short = base64::engine::general_purpose::STANDARD.encode([0xAAu8; 16]);
    assert!(
        !is_valid_registry_pubkey_b64(&short),
        "16-byte key must be rejected"
    );

    // Wrong length (64 bytes) — rejected.
    let long = base64::engine::general_purpose::STANDARD.encode([0xAAu8; 64]);
    assert!(
        !is_valid_registry_pubkey_b64(&long),
        "64-byte key must be rejected"
    );

    // Non-base64 garbage — rejected.
    assert!(
        !is_valid_registry_pubkey_b64("not-base64!!!"),
        "garbage input must be rejected"
    );

    // Empty input — rejected.
    assert!(!is_valid_registry_pubkey_b64(""), "empty must be rejected");

    // Whitespace tolerated around a valid key.
    let real_key = [0xABu8; 32];
    let b64_padded = format!(
        "  {}  ",
        base64::engine::general_purpose::STANDARD.encode(real_key),
    );
    assert!(
        is_valid_registry_pubkey_b64(&b64_padded),
        "validator must trim whitespace"
    );
}

/// The TOFU cache path is derived from $HOME — verify the directory layout
/// matches the conventional `~/.librefang/` location used elsewhere in the
/// runtime (config, plugins, agents).
#[test]
fn registry_pubkey_cache_path_lives_under_dotlibrefang() {
    // Save and restore HOME to avoid leaking into other tests.
    let original = std::env::var("HOME").ok();
    // SAFETY: tests in this module are single-threaded for env mutation.
    unsafe {
        std::env::set_var("HOME", "/tmp/librefang-test-home-pubkey");
    }
    let path = registry_pubkey_cache_path().expect("path resolution");
    // SAFETY: restoring the prior value of HOME.
    unsafe {
        match original {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
    assert_eq!(
        path,
        std::path::PathBuf::from("/tmp/librefang-test-home-pubkey/.librefang/registry.pub"),
    );
}

/// Slot 0 of the embedded keys MUST be the active key — no expiry.
/// A maintainer who absent-mindedly sets `expires_at: Some(...)` on
/// slot 0 during a rotation edit would silently break installs the
/// moment that timestamp passed (PR re-review LOW round 4). Compile-
/// time + test-time guard so the regression is caught before ship.
#[test]
fn embedded_pubkeys_slot0_has_no_expiry() {
    let slot0 = EMBEDDED_REGISTRY_PUBKEYS
        .first()
        .expect("EMBEDDED_REGISTRY_PUBKEYS must have at least one entry");
    assert!(
        slot0.expires_at.is_none(),
        "EMBEDDED_REGISTRY_PUBKEYS[0] must have expires_at: None — slot 0 \
         is the active key and must not be marked for rotation"
    );
    assert!(
        is_valid_registry_pubkey_b64(slot0.pubkey_b64),
        "EMBEDDED_REGISTRY_PUBKEYS[0].pubkey_b64 is not a valid 32-byte Ed25519 key"
    );
}

/// Official registry defaults to the worker-signed mirror — the GitHub
/// repo has no committed `index.json`, so any other choice would lose
/// the only end-to-end Ed25519-verifiable path.
#[test]
fn registry_index_urls_official_defaults_to_worker_mirror() {
    let (idx, sig) = registry_index_urls("librefang/librefang-registry", None, None);
    assert_eq!(idx, "https://stats.librefang.ai/api/registry/index.json");
    assert_eq!(
        sig,
        "https://stats.librefang.ai/api/registry/index.json.sig"
    );
}

/// Self-hosted forks fall back to GitHub raw — keeps the existing path
/// for forks that don't yet run a signed mirror, while still allowing
/// them to opt in via the env vars.
#[test]
fn registry_index_urls_fork_falls_back_to_github_raw() {
    let (idx, sig) = registry_index_urls("acme/private-registry", None, None);
    assert_eq!(
        idx,
        "https://raw.githubusercontent.com/acme/private-registry/main/index.json"
    );
    assert_eq!(
        sig,
        "https://raw.githubusercontent.com/acme/private-registry/main/index.json.sig"
    );
}

/// Env overrides win regardless of which registry is in use — operators
/// of air-gapped / on-prem deployments must be able to redirect both
/// the official and the fork path at their own infrastructure.
#[test]
fn registry_index_urls_env_overrides_win_for_both_paths() {
    let (idx, sig) = registry_index_urls(
        "librefang/librefang-registry",
        Some("https://internal.example/index.json".into()),
        Some("https://internal.example/index.json.sig".into()),
    );
    assert_eq!(idx, "https://internal.example/index.json");
    assert_eq!(sig, "https://internal.example/index.json.sig");

    let (idx, sig) = registry_index_urls(
        "acme/private-registry",
        Some("https://internal.example/index.json".into()),
        Some("https://internal.example/index.json.sig".into()),
    );
    assert_eq!(idx, "https://internal.example/index.json");
    assert_eq!(sig, "https://internal.example/index.json.sig");
}

// ── Bug #3804 — hook script integrity check logic ────────────────────────

/// Helper: build a minimal PluginManifest with the given hook paths and
/// integrity entries so we can exercise the detection logic without
/// spinning up an HTTP server.
fn make_manifest_with_hooks(
    hooks: &[(&str, &str)],     // (field_name, script_path)
    integrity: &[(&str, &str)], // (script_path, sha256hex)
) -> PluginManifest {
    let mut m = PluginManifest {
        name: "test-plugin".to_string(),
        version: "0.1.0".to_string(),
        ..Default::default()
    };
    for &(field, path) in hooks {
        match field {
            "ingest" => m.hooks.ingest = Some(path.to_string()),
            "after_turn" => m.hooks.after_turn = Some(path.to_string()),
            "bootstrap" => m.hooks.bootstrap = Some(path.to_string()),
            "assemble" => m.hooks.assemble = Some(path.to_string()),
            "compact" => m.hooks.compact = Some(path.to_string()),
            "prepare_subagent" => m.hooks.prepare_subagent = Some(path.to_string()),
            "merge_subagent" => m.hooks.merge_subagent = Some(path.to_string()),
            _ => {}
        }
    }
    for &(path, hash) in integrity {
        m.integrity.insert(path.to_string(), hash.to_string());
    }
    m
}

/// Extracts the list of hook script paths that are declared in a manifest
/// but missing from its integrity map.  Delegates to the production
/// `manifest_missing_integrity_hooks` so the install-time check, the
/// `lint_plugin` warning, and these regression tests can never drift.
fn missing_integrity_hooks(manifest: &PluginManifest) -> Vec<String> {
    super::manifest_missing_integrity_hooks(manifest)
}

/// A plugin with no hooks declared requires no integrity entries.
#[test]
fn hook_integrity_no_hooks_no_requirement() {
    let m = make_manifest_with_hooks(&[], &[]);
    assert!(
        missing_integrity_hooks(&m).is_empty(),
        "no hooks → no integrity entries required"
    );
}

/// Every declared hook must appear in [integrity]; any missing entry is flagged.
#[test]
fn hook_integrity_missing_entries_detected() {
    let m = make_manifest_with_hooks(
        &[
            ("ingest", "hooks/ingest.py"),
            ("after_turn", "hooks/after_turn.py"),
        ],
        &[
            // after_turn is covered, but ingest is not
            ("hooks/after_turn.py", "abc123"),
        ],
    );
    let missing = missing_integrity_hooks(&m);
    assert_eq!(missing, vec!["hooks/ingest.py"]);
}

/// When all declared hooks have integrity entries, no missing entries are reported.
#[test]
fn hook_integrity_all_covered_passes() {
    let m = make_manifest_with_hooks(
        &[
            ("ingest", "hooks/ingest.py"),
            ("after_turn", "hooks/after_turn.py"),
        ],
        &[
            ("hooks/ingest.py", "deadbeef"),
            ("hooks/after_turn.py", "cafebabe"),
        ],
    );
    assert!(
        missing_integrity_hooks(&m).is_empty(),
        "all hooks covered → no missing integrity entries"
    );
}

/// All seven hook fields are checked, not just ingest/after_turn.
#[test]
fn hook_integrity_all_hook_fields_checked() {
    let all_hooks = [
        ("ingest", "hooks/ingest.py"),
        ("after_turn", "hooks/after_turn.py"),
        ("bootstrap", "hooks/bootstrap.py"),
        ("assemble", "hooks/assemble.py"),
        ("compact", "hooks/compact.py"),
        ("prepare_subagent", "hooks/prepare_subagent.py"),
        ("merge_subagent", "hooks/merge_subagent.py"),
    ];
    // Provide integrity for all but compact and merge_subagent.
    let integrity_provided = [
        ("hooks/ingest.py", "h1"),
        ("hooks/after_turn.py", "h2"),
        ("hooks/bootstrap.py", "h3"),
        ("hooks/assemble.py", "h4"),
        ("hooks/prepare_subagent.py", "h6"),
    ];
    let m = make_manifest_with_hooks(&all_hooks, &integrity_provided);
    let mut missing = missing_integrity_hooks(&m);
    missing.sort();
    assert_eq!(
        missing,
        vec!["hooks/compact.py", "hooks/merge_subagent.py"],
        "compact and merge_subagent must be flagged"
    );
}

// ── Bug #4036 — registry publish pipeline must auto-inject integrity ──

/// Helper: write a minimal plugin layout into `dir` with the requested
/// hook scripts and the requested raw `plugin.toml` body.  Returns the
/// directory path so the caller can keep ownership of the tempdir.
fn write_fake_plugin(
    dir: &Path,
    manifest_toml: &str,
    scripts: &[(&str, &[u8])],
) -> std::path::PathBuf {
    std::fs::write(dir.join("plugin.toml"), manifest_toml).unwrap();
    for (rel, body) in scripts {
        let abs = dir.join(rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&abs, body).unwrap();
    }
    dir.to_path_buf()
}

/// pack_plugin_for_publish must write [integrity] entries with the
/// correct SHA-256 of every declared hook.  Mirrors the real
/// `context-decay` regression: declares two hooks, no [integrity].
#[test]
fn pack_plugin_for_publish_auto_injects_hashes_for_context_decay_shape() {
    let tmp = tempfile::tempdir().unwrap();
    let plugin_dir = tmp.path().join("context-decay");
    std::fs::create_dir_all(&plugin_dir).unwrap();

    let manifest = r#"name = "context-decay"
version = "0.1.0"
description = "Decay older context entries"
author = "Test"

[hooks]
ingest = "hooks/ingest.py"
after_turn = "hooks/after_turn.py"
"#;
    let ingest_body = b"# ingest hook\nprint('ingest')\n";
    let after_turn_body = b"# after_turn hook\nprint('after')\n";
    let plugin_dir = write_fake_plugin(
        &plugin_dir,
        manifest,
        &[
            ("hooks/ingest.py", ingest_body),
            ("hooks/after_turn.py", after_turn_body),
        ],
    );

    // Sanity: the unsigned manifest must fail validation up-front,
    // matching the user-visible error in the bug report.
    let pre = validate_publish_ready(&plugin_dir).expect_err("unsigned must fail");
    assert!(
        pre.contains("hooks/ingest.py") && pre.contains("hooks/after_turn.py"),
        "validate_publish_ready must list every missing hook, got: {pre}"
    );

    // Run the publish packer.
    let written = pack_plugin_for_publish(&plugin_dir).expect("pack must succeed");
    assert_eq!(written.len(), 2, "both hooks must be hashed");
    assert_eq!(written["hooks/ingest.py"], sha256_hex(ingest_body));
    assert_eq!(written["hooks/after_turn.py"], sha256_hex(after_turn_body));

    // Re-read the manifest and confirm the [integrity] block is present
    // and matches the expected hashes.
    let rewritten = std::fs::read_to_string(plugin_dir.join("plugin.toml")).unwrap();
    let parsed: PluginManifest = toml::from_str(&rewritten).expect("rewritten manifest valid");
    assert_eq!(
        parsed.integrity.get("hooks/ingest.py").map(String::as_str),
        Some(sha256_hex(ingest_body).as_str())
    );
    assert_eq!(
        parsed
            .integrity
            .get("hooks/after_turn.py")
            .map(String::as_str),
        Some(sha256_hex(after_turn_body).as_str())
    );

    // The packed plugin must now satisfy the publish-readiness check.
    validate_publish_ready(&plugin_dir).expect("packed plugin must validate");

    // And the install-time loader must accept it without complaint.
    let loaded = load_plugin_manifest(&plugin_dir).expect("install-time load must accept");
    assert_eq!(loaded.name, "context-decay");
    assert_eq!(loaded.integrity.len(), 2);
}

/// pack_plugin_for_publish must replace a stale [integrity] block — not
/// duplicate it — when authors re-pack after editing a hook script.
#[test]
fn pack_plugin_for_publish_replaces_stale_integrity_block() {
    let tmp = tempfile::tempdir().unwrap();
    let plugin_dir = tmp.path().join("stale-test");
    std::fs::create_dir_all(&plugin_dir).unwrap();

    let manifest = r#"name = "stale-test"
version = "0.1.0"
description = "Replace stale integrity"
author = "Test"

[hooks]
ingest = "hooks/ingest.py"

[integrity]
"hooks/ingest.py" = "deadbeef_stale_hash_must_be_replaced"
"hooks/removed.py" = "0000_orphan_hash_must_be_dropped"
"#;
    let ingest_body = b"# fresh content\n";
    write_fake_plugin(&plugin_dir, manifest, &[("hooks/ingest.py", ingest_body)]);

    let written = pack_plugin_for_publish(&plugin_dir).expect("pack must succeed");
    assert_eq!(written.len(), 1);

    let rewritten = std::fs::read_to_string(plugin_dir.join("plugin.toml")).unwrap();
    // Only one [integrity] header — no duplicates from the stale block.
    assert_eq!(
        rewritten.matches("[integrity]").count(),
        1,
        "stale [integrity] block must be replaced, not appended:\n{rewritten}"
    );
    // Stale entry for a hook that no longer exists must be gone.
    assert!(
        !rewritten.contains("hooks/removed.py"),
        "orphan integrity entry must be dropped:\n{rewritten}"
    );

    let parsed: PluginManifest = toml::from_str(&rewritten).unwrap();
    assert_eq!(
        parsed.integrity.get("hooks/ingest.py").map(String::as_str),
        Some(sha256_hex(ingest_body).as_str())
    );
    assert!(!parsed.integrity.contains_key("hooks/removed.py"));
}

/// pack_plugin_for_publish must fail loudly when a manifest references
/// a hook script that isn't on disk — that's a packaging bug and
/// emitting the SHA-256 of empty bytes would silently mask it.
#[test]
fn pack_plugin_for_publish_rejects_missing_hook_file() {
    let tmp = tempfile::tempdir().unwrap();
    let plugin_dir = tmp.path().join("missing-hook");
    std::fs::create_dir_all(&plugin_dir).unwrap();

    let manifest = r#"name = "missing-hook"
version = "0.1.0"
description = "Hook file is not shipped"
author = "Test"

[hooks]
ingest = "hooks/ingest.py"
"#;
    // Note: deliberately do NOT write hooks/ingest.py.
    std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();

    let err = pack_plugin_for_publish(&plugin_dir).expect_err("missing hook must fail");
    assert!(
        err.contains("hooks/ingest.py"),
        "error must name the missing hook, got: {err}"
    );
}

/// A plugin that declares no hooks at all is publish-ready by definition
/// — pack returns an empty map, validate accepts.
#[test]
fn pack_plugin_for_publish_accepts_no_hooks_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    let plugin_dir = tmp.path().join("metadata-only");
    std::fs::create_dir_all(&plugin_dir).unwrap();

    let manifest = r#"name = "metadata-only"
version = "0.1.0"
description = "No hooks, just metadata"
author = "Test"
"#;
    std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();

    validate_publish_ready(&plugin_dir).expect("no-hooks plugin is publish-ready");
    let written = pack_plugin_for_publish(&plugin_dir).expect("pack must succeed");
    assert!(written.is_empty(), "no hooks → no hashes written");
}

/// validate_publish_ready must accept a partially-signed manifest only
/// when EVERY declared hook is covered.  A plugin that ships
/// `[integrity]` for some but not all hooks is still rejected — this is
/// the defense-in-depth backstop for issue #4036.
#[test]
fn validate_publish_ready_rejects_partial_integrity() {
    let tmp = tempfile::tempdir().unwrap();
    let plugin_dir = tmp.path().join("partial");
    std::fs::create_dir_all(&plugin_dir).unwrap();

    let manifest = r#"name = "partial"
version = "0.1.0"
description = "Half-signed"
author = "Test"

[hooks]
ingest = "hooks/ingest.py"
after_turn = "hooks/after_turn.py"

[integrity]
"hooks/ingest.py" = "abc"
"#;
    std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();

    let err = validate_publish_ready(&plugin_dir).expect_err("partial must fail");
    assert!(
        err.contains("hooks/after_turn.py"),
        "after_turn must be flagged as missing, got: {err}"
    );
    assert!(
        !err.contains("hooks/ingest.py"),
        "ingest is signed and must NOT be flagged, got: {err}"
    );
}

/// pack_plugin_for_publish must produce byte-identical output across
/// repeated invocations on identical inputs — a property the registry
/// archive checksum and any reproducible-build verifier depend on.
#[test]
fn pack_plugin_for_publish_is_deterministic() {
    fn pack_once(seed: &[u8]) -> String {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("det");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let manifest = r#"name = "det"
version = "0.1.0"
description = "Determinism test"
author = "Test"

[hooks]
ingest = "hooks/ingest.py"
after_turn = "hooks/after_turn.py"
bootstrap = "hooks/bootstrap.py"
"#;
        write_fake_plugin(
            &plugin_dir,
            manifest,
            &[
                ("hooks/ingest.py", seed),
                ("hooks/after_turn.py", seed),
                ("hooks/bootstrap.py", seed),
            ],
        );
        pack_plugin_for_publish(&plugin_dir).unwrap();
        std::fs::read_to_string(plugin_dir.join("plugin.toml")).unwrap()
    }

    let a = pack_once(b"identical seed");
    let b = pack_once(b"identical seed");
    assert_eq!(
        a, b,
        "pack output must be byte-identical for identical inputs"
    );
}
