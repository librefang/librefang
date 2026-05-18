use super::*;
use crate::provider_health::DiscoveredModelInfo;

fn test_catalog() -> ModelCatalog {
    let home = crate::registry_sync::resolve_home_dir_for_tests();
    ModelCatalog::new(&home)
}

/// Convert plain name strings to minimal `DiscoveredModelInfo` for tests
/// that don't need to exercise capability inference.
fn names_to_info(names: &[&str]) -> Vec<DiscoveredModelInfo> {
    names
        .iter()
        .map(|n| DiscoveredModelInfo {
            name: n.to_string(),
            parameter_size: None,
            quantization_level: None,
            family: None,
            families: None,
            size: None,
            capabilities: vec![],
        })
        .collect()
}

#[test]
fn test_catalog_has_models() {
    let catalog = test_catalog();
    assert!(catalog.list_models().len() >= 30);
}

/// Mirrors the pre-refactor `catalog_sync::test_alias_catalog_parse` —
/// keeps direct coverage of `AliasesCatalogFile` deserialization, which
/// is now only consumed here in `model_catalog`.
#[test]
fn test_aliases_catalog_parse() {
    // Pure parser test — alias names and target ids are placeholders so
    // the assertions don't have to track whatever the registry's
    // canonical Sonnet / GPT ids happen to be this week.
    let toml_str = r#"
[aliases]
my-alias = "canonical-target-one"
other-alias = "canonical-target-two"
"#;
    let file: librefang_types::model_catalog::AliasesCatalogFile =
        toml::from_str(toml_str).unwrap();
    assert_eq!(file.aliases.len(), 2);
    assert_eq!(file.aliases["my-alias"], "canonical-target-one");
    assert_eq!(file.aliases["other-alias"], "canonical-target-two");
}

/// P2 regression: when registry classification is unavailable
/// (registry dir unreadable or missing), every provider must fall back
/// to is_custom=false so the dashboard does not re-enable the misleading
/// delete button on built-ins.
#[test]
fn test_is_custom_safe_fallback_on_missing_registry() {
    let tmp = tempfile::tempdir().unwrap();
    let providers_dir = tmp.path().join("providers");
    std::fs::create_dir_all(&providers_dir).unwrap();
    std::fs::write(
        providers_dir.join("acme.toml"),
        r#"[provider]
id = "acme"
display_name = "Acme"
api_key_env = "ACME_API_KEY"
base_url = "https://acme.test"
"#,
    )
    .unwrap();

    // Case 1: registry dir argument is None → classification skipped.
    let catalog = ModelCatalog::new_from_dir_with_registry(&providers_dir, None);
    assert!(
        !catalog.list_providers().iter().any(|p| p.is_custom),
        "is_custom must be false when no registry dir is supplied"
    );

    // Case 2: registry dir points to a nonexistent path → read_dir
    // fails, classification must degrade to false (not true).
    let missing_registry = tmp.path().join("nonexistent-registry");
    let catalog = ModelCatalog::new_from_dir_with_registry(&providers_dir, Some(&missing_registry));
    assert!(
        !catalog.list_providers().iter().any(|p| p.is_custom),
        "is_custom must be false when registry read_dir fails"
    );

    // Case 3: registry dir exists and does NOT contain acme.toml →
    // acme is correctly flagged custom.
    let registry_dir = tmp.path().join("registry");
    std::fs::create_dir_all(&registry_dir).unwrap();
    let catalog = ModelCatalog::new_from_dir_with_registry(&providers_dir, Some(&registry_dir));
    assert!(
        catalog
            .list_providers()
            .iter()
            .any(|p| p.id == "acme" && p.is_custom),
        "acme must be flagged custom when registry dir exists but does not list it"
    );

    // Case 4: registry dir lists acme.toml → acme is a built-in.
    std::fs::write(
        registry_dir.join("acme.toml"),
        r#"[provider]
id = "acme"
"#,
    )
    .unwrap();
    let catalog = ModelCatalog::new_from_dir_with_registry(&providers_dir, Some(&registry_dir));
    assert!(
        catalog
            .list_providers()
            .iter()
            .any(|p| p.id == "acme" && !p.is_custom),
        "acme must NOT be flagged custom when registry dir lists it"
    );
}

#[test]
fn test_catalog_has_providers() {
    let catalog = test_catalog();
    assert!(catalog.list_providers().len() >= 40);
}

#[test]
fn test_find_model_by_id() {
    let catalog = test_catalog();
    let entry = catalog.find_model("claude-sonnet-4-6").unwrap();
    assert_eq!(entry.display_name, "Claude Sonnet 4.6");
    assert_eq!(entry.provider, "anthropic");
    assert_eq!(entry.tier, ModelTier::Smart);
}

#[test]
fn test_find_model_by_alias() {
    let catalog = test_catalog();
    let entry = catalog.find_model("sonnet").unwrap();
    assert_eq!(entry.id, "claude-sonnet-4-6");
}

#[test]
fn test_find_model_case_insensitive() {
    let catalog = test_catalog();
    assert!(catalog.find_model("Claude-Sonnet-4-6").is_some());
    assert!(catalog.find_model("SONNET").is_some());
}

#[test]
fn test_find_model_not_found() {
    let catalog = test_catalog();
    assert!(catalog.find_model("nonexistent-model").is_none());
}

/// `find_model_for_provider` must filter by provider so the same model
/// id under different providers (which can differ in `context_window`)
/// resolves to the right entry. The test catalog has
/// `claude-sonnet-4-6` only under `anthropic`, so a copilot
/// lookup of the same id must miss.
#[test]
fn test_find_model_for_provider_filters_by_provider() {
    let catalog = test_catalog();
    assert!(
        catalog
            .find_model_for_provider("anthropic", "claude-sonnet-4-6")
            .is_some(),
        "anthropic catalog hit expected"
    );
    assert!(
        catalog
            .find_model_for_provider("copilot", "claude-sonnet-4-6")
            .is_none(),
        "no copilot entry for the anthropic id should exist",
    );
}

/// Empty `provider` arg disables filtering and behaves like
/// `find_model`. Useful when the agent's manifest has no provider
/// configured (e.g. fresh install before any provider key is set).
#[test]
fn test_find_model_for_provider_empty_provider_falls_back() {
    let catalog = test_catalog();
    let via_filtered = catalog
        .find_model_for_provider("", "claude-sonnet-4-6")
        .expect("empty provider should match anyway");
    let via_unfiltered = catalog
        .find_model("claude-sonnet-4-6")
        .expect("unfiltered match");
    assert_eq!(via_filtered.id, via_unfiltered.id);
}

/// Provider matching is case-insensitive — registries sometimes
/// store providers as `Anthropic` while manifests use `anthropic`.
#[test]
fn test_find_model_for_provider_case_insensitive_provider() {
    let catalog = test_catalog();
    assert!(catalog
        .find_model_for_provider("ANTHROPIC", "claude-sonnet-4-6")
        .is_some(),);
}

/// Alias resolution is also provider-scoped: `"sonnet"` must resolve
/// to the anthropic entry under `provider="anthropic"`, but a query
/// against an unrelated provider with the same alias must miss.
#[test]
fn test_find_model_for_provider_alias_is_scoped() {
    let catalog = test_catalog();
    let r = catalog
        .find_model_for_provider("anthropic", "sonnet")
        .expect("alias under anthropic");
    assert_eq!(r.id, "claude-sonnet-4-6");
    assert!(
        catalog
            .find_model_for_provider("openai", "sonnet")
            .is_none(),
        "alias must not leak across providers",
    );
}

#[test]
fn test_resolve_alias() {
    let catalog = test_catalog();
    assert_eq!(catalog.resolve_alias("sonnet"), Some("claude-sonnet-4-6"));
    assert_eq!(
        catalog.resolve_alias("haiku"),
        Some("claude-haiku-4-5-20251001")
    );
    assert!(catalog.resolve_alias("nonexistent").is_none());
}

#[test]
fn test_models_by_provider() {
    let catalog = test_catalog();
    let anthropic = catalog.models_by_provider("anthropic");
    assert!(!anthropic.is_empty());
    assert!(anthropic.iter().all(|m| m.provider == "anthropic"));
}

#[test]
fn test_models_by_tier() {
    let catalog = test_catalog();
    let frontier = catalog.models_by_tier(ModelTier::Frontier);
    assert!(frontier.len() >= 3); // At least opus, gpt-4.1, gemini-2.5-pro
    assert!(frontier.iter().all(|m| m.tier == ModelTier::Frontier));
}

#[test]
fn test_pricing_lookup() {
    let catalog = test_catalog();
    let (input, output) = catalog.pricing("claude-sonnet-4-6").unwrap();
    assert!((input - 3.0).abs() < 0.001);
    assert!((output - 15.0).abs() < 0.001);
}

#[test]
fn test_pricing_via_alias() {
    let catalog = test_catalog();
    let (input, output) = catalog.pricing("sonnet").unwrap();
    assert!((input - 3.0).abs() < 0.001);
    assert!((output - 15.0).abs() < 0.001);
}

#[test]
fn test_pricing_not_found() {
    let catalog = test_catalog();
    assert!(catalog.pricing("nonexistent").is_none());
}

#[test]
fn test_detect_auth_local_providers() {
    let mut catalog = test_catalog();
    catalog.detect_auth();
    // Local providers should be NotRequired
    let ollama = catalog.get_provider("ollama").unwrap();
    assert_eq!(ollama.auth_status, AuthStatus::NotRequired);
    let vllm = catalog.get_provider("vllm").unwrap();
    assert_eq!(vllm.auth_status, AuthStatus::NotRequired);
}

/// Module-scope mutex for tests that mutate process env vars.
///
/// `cargo test` runs tests in parallel by default, so any two tests
/// touching the same env var must share this lock — otherwise they race
/// on process-global state. Each test declaring its own `static` was the
/// earlier bug: two disjoint mutexes = no mutual exclusion.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Regression: a CLI login must NOT auto-configure the corresponding API
/// provider. `anthropic` / `openai` / `gemini` / `qwen` only light up
/// when the user sets their own API key. CLI logins surface via their
/// dedicated provider entries (`claude-code`, `codex-cli`, etc.).
///
/// This test runs with no provider API-key env vars set, so every
/// API provider should report `Missing`. We only assert on the four
/// providers that previously borrowed CLI credentials — the others
/// are naturally Missing.
#[test]
fn detect_auth_does_not_promote_api_providers_from_cli_login() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let preserved: Vec<(&str, Option<String>)> = [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "QWEN_API_KEY",
        "DASHSCOPE_API_KEY",
    ]
    .iter()
    .map(|k| (*k, std::env::var(k).ok()))
    .collect();
    for (k, _) in &preserved {
        // SAFETY: single-threaded section guarded by ENV_LOCK.
        unsafe { std::env::remove_var(k) };
    }

    let mut catalog = test_catalog();
    catalog.detect_auth();

    for id in ["anthropic", "openai", "gemini", "qwen"] {
        let p = catalog.get_provider(id).unwrap();
        assert_eq!(
            p.auth_status,
            AuthStatus::Missing,
            "{id} must be Missing when no API key is set, regardless of CLI login"
        );
    }

    for (k, v) in preserved {
        // SAFETY: single-threaded section guarded by ENV_LOCK.
        unsafe {
            if let Some(val) = v {
                std::env::set_var(k, val);
            } else {
                std::env::remove_var(k);
            }
        }
    }
}

/// `GOOGLE_API_KEY` remains a recognised alias for `GEMINI_API_KEY`
/// (officially documented by Google AI Studio as equivalent). Setting
/// it should promote Gemini to AutoDetected — this is a real API key
/// the user typed, not a CLI-credential borrow.
#[test]
fn google_api_key_alias_still_recognised_for_gemini() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let prev_gemini = std::env::var("GEMINI_API_KEY").ok();
    let prev_google = std::env::var("GOOGLE_API_KEY").ok();
    // SAFETY: single-threaded section guarded by ENV_LOCK.
    unsafe {
        std::env::remove_var("GEMINI_API_KEY");
        std::env::set_var("GOOGLE_API_KEY", "test-alias-key");
    }

    let mut catalog = test_catalog();
    catalog.detect_auth();
    let gemini = catalog.get_provider("gemini").unwrap();
    assert_eq!(gemini.auth_status, AuthStatus::AutoDetected);

    // SAFETY: single-threaded section guarded by ENV_LOCK.
    unsafe {
        if let Some(v) = prev_gemini {
            std::env::set_var("GEMINI_API_KEY", v);
        } else {
            std::env::remove_var("GEMINI_API_KEY");
        }
        if let Some(v) = prev_google {
            std::env::set_var("GOOGLE_API_KEY", v);
        } else {
            std::env::remove_var("GOOGLE_API_KEY");
        }
    }
}

/// Regression for #4803: pressing "remove key" on a CLI provider
/// (claude-code, codex-cli, gemini-cli, qwen-code) calls
/// `suppress_provider` + `detect_auth`. Pre-fix `detect_auth` ignored
/// suppression for CLI providers and re-detected them as Configured
/// whenever the CLI binary was on PATH, so the provider never left
/// the configured grid. The fix routes suppression through the CLI
/// branch.
#[test]
fn detect_auth_respects_suppression_for_cli_providers() {
    let mut catalog = test_catalog();
    // Whatever the host machine reports for these CLIs is fine — the
    // assertion is that suppression dominates the auto-detection.
    catalog.suppress_provider("claude-code");
    catalog.suppress_provider("codex-cli");
    catalog.detect_auth();

    let claude = catalog.get_provider("claude-code").unwrap();
    assert_eq!(
        claude.auth_status,
        AuthStatus::Missing,
        "suppressed CLI provider must be Missing regardless of binary presence"
    );
    let codex = catalog.get_provider("codex-cli").unwrap();
    assert_eq!(codex.auth_status, AuthStatus::Missing);

    // A non-suppressed CLI provider is unaffected — proves we did not
    // break the auto-detect path for the unsuppressed case.
    let gemini_cli = catalog.get_provider("gemini-cli").unwrap();
    assert!(matches!(
        gemini_cli.auth_status,
        AuthStatus::Configured | AuthStatus::CliNotInstalled
    ));
}

/// Regression for #4803: pressing "remove key" on a local HTTP provider
/// (ollama, vllm, lmstudio, lemonade) similarly suppressed it but
/// `detect_auth` set it back to NotRequired on the next call, so the
/// provider never left the configured grid.
#[test]
fn detect_auth_respects_suppression_for_local_providers() {
    let mut catalog = test_catalog();
    catalog.suppress_provider("ollama");
    catalog.detect_auth();

    let ollama = catalog.get_provider("ollama").unwrap();
    assert_eq!(
        ollama.auth_status,
        AuthStatus::Missing,
        "suppressed local provider must be Missing instead of NotRequired"
    );

    // Un-suppressing restores the local default. This mirrors the
    // `set_provider_url` re-enable path in the API layer.
    catalog.unsuppress_provider("ollama");
    catalog.detect_auth();
    let ollama = catalog.get_provider("ollama").unwrap();
    assert_eq!(ollama.auth_status, AuthStatus::NotRequired);
}

/// `is_suppressed` reflects `suppress_provider` / `unsuppress_provider`
/// without going through `detect_auth`. This is the accessor the
/// `probe_and_update_local_provider` gate (#4803 follow-up) reads to
/// decide whether a probe write should be skipped, so a regression in
/// the lookup primitive would silently re-introduce the bug where
/// user-triggered Test on a suppressed provider re-flipped the catalog.
#[test]
fn is_suppressed_reflects_set_membership() {
    let mut catalog = test_catalog();
    assert!(!catalog.is_suppressed("ollama"));
    catalog.suppress_provider("ollama");
    assert!(catalog.is_suppressed("ollama"));
    catalog.unsuppress_provider("ollama");
    assert!(!catalog.is_suppressed("ollama"));
    // Unknown id is just not-in-set, not a panic.
    assert!(!catalog.is_suppressed("__no_such_provider__"));
}

/// Regression for the #4803 follow-up — the periodic probe loop must
/// not re-promote a suppressed local provider. Pre-fix the filter in
/// `probe_all_local_providers_once` only checked `is_local_provider`
/// plus non-empty `base_url`, so an ollama row that the user had
/// hidden via "remove key" would still be polled every ~60 s and
/// have its `auth_status` overwritten with `NotRequired` /
/// `LocalOffline` via `set_provider_auth_status` (which bypasses
/// `detect_auth`). The fix routes the filter through
/// `local_provider_probe_targets`, which excludes suppressed
/// providers up front.
#[test]
fn local_provider_probe_targets_excludes_suppressed_providers() {
    let mut catalog = test_catalog();
    let baseline = catalog.local_provider_probe_targets();
    assert!(
        baseline.iter().any(|(id, _)| id == "ollama"),
        "ollama must be a probe target by default — sanity-check the seed catalog: {baseline:?}"
    );

    catalog.suppress_provider("ollama");
    let filtered = catalog.local_provider_probe_targets();
    assert!(
        !filtered.iter().any(|(id, _)| id == "ollama"),
        "suppressed local provider must be excluded from probe targets: {filtered:?}"
    );

    // Other local providers (e.g. vllm, lmstudio) stay in the list —
    // suppression is per-provider, not a global kill switch.
    for (id, _) in &baseline {
        if id != "ollama" {
            assert!(
                filtered.iter().any(|(fid, _)| fid == id),
                "non-suppressed local provider {id} must survive the filter"
            );
        }
    }

    catalog.unsuppress_provider("ollama");
    let restored = catalog.local_provider_probe_targets();
    assert!(
        restored.iter().any(|(id, _)| id == "ollama"),
        "un-suppressing must restore ollama as a probe target"
    );
}

#[test]
fn test_available_models_includes_local() {
    let mut catalog = test_catalog();
    catalog.detect_auth();
    let available = catalog.available_models();
    // Local providers (ollama, vllm, lmstudio) should always be available
    assert!(available.iter().any(|m| m.provider == "ollama"));
}

#[test]
fn test_provider_model_counts() {
    let catalog = test_catalog();
    let anthropic = catalog.get_provider("anthropic").unwrap();
    assert!(anthropic.model_count > 0);
    let groq = catalog.get_provider("groq").unwrap();
    assert!(groq.model_count > 0);
}

#[test]
fn test_list_aliases() {
    let catalog = test_catalog();
    let aliases = catalog.list_aliases();
    assert!(aliases.len() >= 20);
    assert_eq!(aliases.get("sonnet").unwrap(), "claude-sonnet-4-6");
    // New aliases
    assert_eq!(aliases.get("grok").unwrap(), "grok-4-0709");
}

#[test]
fn test_find_grok_by_alias() {
    let catalog = test_catalog();
    let entry = catalog.find_model("grok").unwrap();
    assert_eq!(entry.id, "grok-4-0709");
    assert_eq!(entry.provider, "xai");
}

#[test]
fn test_add_alias() {
    let mut catalog = test_catalog();
    assert!(catalog.add_alias("my-sonnet", "claude-sonnet-4-6"));
    assert_eq!(
        catalog.resolve_alias("my-sonnet").unwrap(),
        "claude-sonnet-4-6"
    );
    // Duplicate should return false
    assert!(!catalog.add_alias("my-sonnet", "gpt-4o"));
    // Alias is case-insensitive
    assert!(!catalog.add_alias("MY-SONNET", "gpt-4o"));
}

#[test]
fn test_remove_alias() {
    let mut catalog = test_catalog();
    catalog.add_alias("temp-alias", "gpt-4o");
    assert!(catalog.remove_alias("temp-alias"));
    assert!(catalog.resolve_alias("temp-alias").is_none());
    // Removing non-existent alias returns false
    assert!(!catalog.remove_alias("no-such-alias"));
    // Case-insensitive removal
    catalog.add_alias("upper-alias", "gpt-4o");
    assert!(catalog.remove_alias("UPPER-ALIAS"));
}

#[test]
fn test_new_providers_in_catalog() {
    let catalog = test_catalog();
    assert!(catalog.get_provider("perplexity").is_some());
    assert!(catalog.get_provider("cohere").is_some());
    assert!(catalog.get_provider("cerebras").is_some());
    assert!(catalog.get_provider("sambanova").is_some());
    assert!(catalog.get_provider("huggingface").is_some());
    assert!(catalog.get_provider("xai").is_some());
    assert!(catalog.get_provider("replicate").is_some());
}

#[test]
fn test_xai_models() {
    let catalog = test_catalog();
    let xai = catalog.models_by_provider("xai");
    assert!(!xai.is_empty());
    assert!(xai.iter().any(|m| m.id == "grok-4-0709"));
    assert!(xai.iter().any(|m| m.id == "grok-4-fast-reasoning"));
    assert!(xai.iter().any(|m| m.id == "grok-4-fast-non-reasoning"));
    assert!(xai.iter().any(|m| m.id == "grok-4-1-fast-reasoning"));
    assert!(xai.iter().any(|m| m.id == "grok-4-1-fast-non-reasoning"));
}

#[test]
fn test_perplexity_models() {
    let catalog = test_catalog();
    let pp = catalog.models_by_provider("perplexity");
    assert!(!pp.is_empty());
}

#[test]
fn test_cohere_models() {
    let catalog = test_catalog();
    let co = catalog.models_by_provider("cohere");
    assert!(!co.is_empty());
}

#[test]
fn test_default_creates_valid_catalog() {
    let catalog = test_catalog();
    assert!(!catalog.list_models().is_empty());
    assert!(!catalog.list_providers().is_empty());
}

#[test]
fn test_merge_adds_new_models() {
    let mut catalog = test_catalog();
    let before = catalog.models_by_provider("ollama").len();
    catalog.merge_discovered_models("ollama", &names_to_info(&["codestral:latest", "qwen2:7b"]));
    let after = catalog.models_by_provider("ollama").len();
    assert_eq!(after, before + 2);
    // Verify the new models are Local tier with zero cost
    let qwen = catalog.find_model("qwen2:7b").unwrap();
    assert_eq!(qwen.tier, ModelTier::Local);
    assert!((qwen.input_cost_per_m).abs() < f64::EPSILON);
}

#[test]
fn test_merge_skips_existing() {
    let mut catalog = test_catalog();
    // Pick an existing builtin Ollama model ID dynamically so this test
    // stays green regardless of which models the registry ships.
    let existing_id = catalog
        .models_by_provider("ollama")
        .into_iter()
        .next()
        .expect("ollama must have at least one builtin model")
        .id
        .clone();
    let before = catalog.list_models().len();
    catalog.merge_discovered_models("ollama", &names_to_info(&[existing_id.as_str()]));
    let after = catalog.list_models().len();
    assert_eq!(after, before); // no new model added
}

#[test]
fn test_merge_updates_model_count() {
    let mut catalog = test_catalog();
    let before_count = catalog.get_provider("ollama").unwrap().model_count;
    catalog.merge_discovered_models("ollama", &names_to_info(&["new-model:latest"]));
    let after_count = catalog.get_provider("ollama").unwrap().model_count;
    assert_eq!(after_count, before_count + 1);
}

#[test]
fn test_merge_infers_capabilities_from_ollama_metadata() {
    let mut catalog = test_catalog();

    let models = vec![
        // Vision model: families includes "clip"
        DiscoveredModelInfo {
            name: "llava:latest".to_string(),
            families: Some(vec!["llama".to_string(), "clip".to_string()]),
            family: Some("llama".to_string()),
            parameter_size: None,
            quantization_level: None,
            size: None,
            capabilities: vec![],
        },
        // Embedding model: name contains "embed"
        DiscoveredModelInfo {
            name: "nomic-embed-text:latest".to_string(),
            families: None,
            family: None,
            parameter_size: None,
            quantization_level: None,
            size: None,
            capabilities: vec![],
        },
        // Thinking model: name contains "deepseek-r1"
        DiscoveredModelInfo {
            name: "deepseek-r1:8b".to_string(),
            families: None,
            family: None,
            parameter_size: None,
            quantization_level: None,
            size: None,
            capabilities: vec![],
        },
        // Plain chat model
        DiscoveredModelInfo {
            name: "llama3.2:latest".to_string(),
            families: Some(vec!["llama".to_string()]),
            family: Some("llama".to_string()),
            parameter_size: None,
            quantization_level: None,
            size: None,
            capabilities: vec![],
        },
    ];
    catalog.merge_discovered_models("ollama", &models);

    let llava = catalog.find_model("llava:latest").unwrap();
    assert!(
        llava.supports_vision,
        "llava should have vision via clip family"
    );
    assert!(llava.supports_tools);

    let embed = catalog.find_model("nomic-embed-text:latest").unwrap();
    assert!(!embed.supports_vision);
    assert!(
        !embed.supports_tools,
        "embedding model should not have tools"
    );
    assert!(!embed.supports_thinking);

    let r1 = catalog.find_model("deepseek-r1:8b").unwrap();
    assert!(r1.supports_thinking, "deepseek-r1 should have thinking");
    assert!(!r1.supports_vision);

    let llama = catalog.find_model("llama3.2:latest").unwrap();
    assert!(!llama.supports_vision);
    assert!(llama.supports_tools);
    assert!(!llama.supports_thinking);
}

/// Regression #4034: explicit `thinking`/`vision` capabilities from Ollama ≥0.7 must propagate for HF-imported models with opaque names.
#[test]
fn test_merge_honours_explicit_thinking_and_vision_capabilities() {
    let mut catalog = test_catalog();
    let models = vec![DiscoveredModelInfo {
        name: "Gemma-4-26B-A4B-it-GGUF:latest".to_string(),
        families: Some(vec!["gemma".to_string()]),
        family: Some("gemma".to_string()),
        parameter_size: None,
        quantization_level: None,
        size: None,
        capabilities: vec![
            "completion".to_string(),
            "vision".to_string(),
            "thinking".to_string(),
            "tools".to_string(),
        ],
    }];
    catalog.merge_discovered_models("ollama", &models);

    let entry = catalog
        .find_model("Gemma-4-26B-A4B-it-GGUF:latest")
        .expect("HF-imported model must be added");
    assert!(
        entry.supports_vision,
        "explicit `vision` capability must propagate"
    );
    assert!(
        entry.supports_thinking,
        "explicit `thinking` capability must propagate (pre-fix this was dropped)"
    );
    assert!(entry.supports_tools);
}

/// Regression #4034 part 2: a re-probe with explicit capabilities must upgrade an existing Local-tier entry in place (handles Ollama <0.7 → ≥0.7 upgrades).
#[test]
fn test_merge_upgrades_existing_local_entry_capabilities() {
    let mut catalog = test_catalog();

    // First probe: no explicit capabilities, plain chat model.
    catalog.merge_discovered_models(
        "ollama",
        &[DiscoveredModelInfo {
            name: "Gemma-4-26B-A4B-it-GGUF:latest".to_string(),
            families: None,
            family: None,
            parameter_size: None,
            quantization_level: None,
            size: None,
            capabilities: vec![],
        }],
    );
    let pre = catalog
        .find_model("Gemma-4-26B-A4B-it-GGUF:latest")
        .unwrap();
    assert!(!pre.supports_vision);
    assert!(!pre.supports_thinking);

    // Second probe: now carries explicit capabilities.
    catalog.merge_discovered_models(
        "ollama",
        &[DiscoveredModelInfo {
            name: "Gemma-4-26B-A4B-it-GGUF:latest".to_string(),
            families: None,
            family: None,
            parameter_size: None,
            quantization_level: None,
            size: None,
            capabilities: vec![
                "vision".to_string(),
                "thinking".to_string(),
                "tools".to_string(),
            ],
        }],
    );
    let post = catalog
        .find_model("Gemma-4-26B-A4B-it-GGUF:latest")
        .unwrap();
    assert!(
        post.supports_vision,
        "second probe must upgrade vision flag"
    );
    assert!(
        post.supports_thinking,
        "second probe must upgrade thinking flag"
    );
    assert!(post.supports_tools);
}

/// Capability upgrades are monotonic — a transient probe with empty capabilities must not downgrade previously-detected vision/thinking flags.
#[test]
fn test_merge_never_downgrades_capabilities() {
    let mut catalog = test_catalog();
    catalog.merge_discovered_models(
        "ollama",
        &[DiscoveredModelInfo {
            name: "vlm-model:latest".to_string(),
            families: None,
            family: None,
            parameter_size: None,
            quantization_level: None,
            size: None,
            capabilities: vec!["vision".to_string(), "thinking".to_string()],
        }],
    );
    // Re-probe with empty capabilities — must NOT clear the previously
    // detected `vision`/`thinking` flags.
    catalog.merge_discovered_models(
        "ollama",
        &[DiscoveredModelInfo {
            name: "vlm-model:latest".to_string(),
            families: None,
            family: None,
            parameter_size: None,
            quantization_level: None,
            size: None,
            capabilities: vec![],
        }],
    );
    let entry = catalog.find_model("vlm-model:latest").unwrap();
    assert!(entry.supports_vision, "must not downgrade vision");
    assert!(entry.supports_thinking, "must not downgrade thinking");
}

#[test]
fn test_custom_model_keeps_assigned_provider() {
    let mut catalog = test_catalog();
    let added = catalog.add_custom_model(ModelCatalogEntry {
        id: "custom-qwen-model".to_string(),
        display_name: "Custom Qwen Model".to_string(),
        provider: "qwen".to_string(),
        tier: ModelTier::Custom,
        context_window: 128_000,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.0,
        output_cost_per_m: 0.0,
        supports_tools: true,
        supports_vision: false,
        supports_streaming: true,
        supports_thinking: false,
        aliases: vec!["custom-qwen".to_string()],
        ..Default::default()
    });

    assert!(added);
    let model = catalog.find_model("custom-qwen-model").unwrap();
    assert_eq!(model.provider, "qwen");

    let aliased = catalog.find_model("custom-qwen").unwrap();
    assert_eq!(aliased.provider, "qwen");
}

#[test]
fn test_custom_models_with_same_id_keep_distinct_providers() {
    let mut catalog = test_catalog();

    assert!(catalog.add_custom_model(ModelCatalogEntry {
        id: "shared-custom-id".to_string(),
        display_name: "Shared Custom ID".to_string(),
        provider: "qwen".to_string(),
        tier: ModelTier::Custom,
        context_window: 64_000,
        max_output_tokens: 4_096,
        input_cost_per_m: 0.0,
        output_cost_per_m: 0.0,
        supports_tools: true,
        supports_vision: false,
        supports_streaming: true,
        supports_thinking: false,
        aliases: Vec::new(),
        ..Default::default()
    }));

    assert!(catalog.add_custom_model(ModelCatalogEntry {
        id: "shared-custom-id".to_string(),
        display_name: "Shared Custom ID".to_string(),
        provider: "minimax".to_string(),
        tier: ModelTier::Custom,
        context_window: 64_000,
        max_output_tokens: 4_096,
        input_cost_per_m: 0.0,
        output_cost_per_m: 0.0,
        supports_tools: true,
        supports_vision: false,
        supports_streaming: true,
        supports_thinking: false,
        aliases: Vec::new(),
        ..Default::default()
    }));

    let qwen_count = catalog
        .models_by_provider("qwen")
        .iter()
        .filter(|m| m.id == "shared-custom-id")
        .count();
    let minimax_count = catalog
        .models_by_provider("minimax")
        .iter()
        .filter(|m| m.id == "shared-custom-id")
        .count();

    assert_eq!(qwen_count, 1);
    assert_eq!(minimax_count, 1);
}

#[test]
fn test_find_model_prefers_custom_over_builtin() {
    // Regression test for #983: when a custom model shares the same ID as a
    // builtin model but specifies a different provider, find_model must
    // return the custom entry so the correct provider is used for routing.
    let mut catalog = test_catalog();

    // Pick a known builtin xai model and verify it exists
    let builtin = catalog.find_model("grok-4-fast-reasoning").unwrap();
    assert_eq!(builtin.provider, "xai");

    // Add a custom model with the same ID but a different provider
    assert!(catalog.add_custom_model(ModelCatalogEntry {
        id: "grok-4-fast-reasoning".to_string(),
        display_name: "Grok 4 Fast via OpenRouter".to_string(),
        provider: "openrouter".to_string(),
        tier: ModelTier::Custom,
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.0,
        output_cost_per_m: 0.0,
        supports_tools: true,
        supports_vision: false,
        supports_streaming: true,
        supports_thinking: false,
        aliases: Vec::new(),
        ..Default::default()
    }));

    // find_model should now return the custom entry, not the builtin
    let found = catalog.find_model("grok-4-fast-reasoning").unwrap();
    assert_eq!(found.provider, "openrouter");
    assert_eq!(found.tier, ModelTier::Custom);
}

#[test]
fn test_chinese_providers_in_catalog() {
    let catalog = test_catalog();
    assert!(catalog.get_provider("qwen").is_some());
    assert!(catalog.get_provider("minimax").is_some());
    assert!(catalog.get_provider("zhipu").is_some());
    assert!(catalog.get_provider("zhipu_coding").is_some());
    assert!(catalog.get_provider("moonshot").is_some());
    assert!(catalog.get_provider("qianfan").is_some());
    assert!(catalog.get_provider("bedrock").is_some());
    assert!(catalog.get_provider("zai").is_some());
    assert!(catalog.get_provider("zai_coding").is_some());
    assert!(catalog.get_provider("kimi_coding").is_some());
    assert!(catalog.get_provider("alibaba-coding-plan").is_some());
}

#[test]
fn test_zai_models() {
    let catalog = test_catalog();
    // Z.AI chat models
    let glm5 = catalog.find_model("zai/glm-5-20250605").unwrap();
    assert_eq!(glm5.provider, "zai");
    assert_eq!(glm5.tier, ModelTier::Frontier);
    let glm47 = catalog.find_model("zai/glm-4.7").unwrap();
    assert_eq!(glm47.provider, "zai");
    assert_eq!(glm47.tier, ModelTier::Smart);
    // Z.AI coding models
    let coding5 = catalog.find_model("glm-5-coding").unwrap();
    assert_eq!(coding5.provider, "zai_coding");
    assert_eq!(coding5.tier, ModelTier::Frontier);
    let coding47 = catalog.find_model("glm-4.7-coding").unwrap();
    assert_eq!(coding47.provider, "zai_coding");
    // Aliases
    assert!(catalog.find_model("zai-glm-5").is_some());
    assert!(catalog.find_model("glm-5-code").is_some());
    assert!(catalog.find_model("glm-coding").is_some());
}

#[test]
fn test_kimi2_models() {
    let catalog = test_catalog();
    // Kimi K2 and K2.5 models — use provider-scoped lookup because
    // byteplus_coding also exposes kimi-k2.5 and the unscoped find_model
    // does not guarantee a particular provider when IDs collide.
    let k2 = catalog
        .find_model_for_provider("moonshot", "kimi-k2")
        .unwrap();
    assert_eq!(k2.provider, "moonshot");
    assert_eq!(k2.tier, ModelTier::Frontier);
    let k25 = catalog
        .find_model_for_provider("moonshot", "kimi-k2.5")
        .unwrap();
    assert_eq!(k25.provider, "moonshot");
    assert_eq!(k25.tier, ModelTier::Frontier);
    // Alias resolution
    assert!(catalog.find_model("kimi-k2.5-0711").is_some());
}

#[test]
fn test_chinese_model_aliases() {
    let catalog = test_catalog();
    assert!(catalog.find_model("kimi").is_some());
    assert!(catalog.find_model("glm").is_some());
    assert!(catalog.find_model("codegeex").is_some());
    assert!(catalog.find_model("ernie").is_some());
    assert!(catalog.find_model("minimax").is_some());
    // MiniMax M2.7 — by exact ID, alias, and case-insensitive
    let m27 = catalog.find_model("MiniMax-M2.7").unwrap();
    assert!(
        m27.provider == "minimax" || m27.provider == "minimax-cn",
        "unexpected provider: {}",
        m27.provider
    );
    assert_eq!(m27.tier, ModelTier::Frontier);
    assert!(catalog.find_model("minimax-m2.7").is_some());
    // Default "minimax" alias resolves to a minimax-family model
    let default = catalog.find_model("minimax").unwrap();
    assert!(
        default.provider == "minimax" || default.provider == "minimax-cn",
        "unexpected provider: {}",
        default.provider
    );
    // MiniMax M2.7 Highspeed — by exact ID and aliases
    let hs = catalog.find_model("MiniMax-M2.7-highspeed").unwrap();
    assert!(
        hs.provider == "minimax" || hs.provider == "minimax-cn",
        "unexpected provider: {}",
        hs.provider
    );
    assert!(catalog.find_model("minimax-m2.7-highspeed").is_some());
}

#[test]
fn test_bedrock_models() {
    let catalog = test_catalog();
    let bedrock = catalog.models_by_provider("bedrock");
    assert!(!bedrock.is_empty());
}

#[test]
fn test_set_provider_url() {
    let mut catalog = test_catalog();
    let old_url = catalog.get_provider("ollama").unwrap().base_url.clone();
    assert_eq!(old_url, "http://127.0.0.1:11434/v1");

    let updated = catalog.set_provider_url("ollama", "http://192.168.1.100:11434/v1");
    assert!(updated);
    assert_eq!(
        catalog.get_provider("ollama").unwrap().base_url,
        "http://192.168.1.100:11434/v1"
    );
}

#[test]
fn test_set_provider_url_unknown() {
    let mut catalog = test_catalog();
    let initial_count = catalog.list_providers().len();
    let updated = catalog.set_provider_url("my-custom-llm", "http://localhost:9999");
    // Unknown providers are now auto-registered as custom entries
    assert!(updated);
    assert_eq!(catalog.list_providers().len(), initial_count + 1);
    assert_eq!(
        catalog.get_provider("my-custom-llm").unwrap().base_url,
        "http://localhost:9999"
    );
}

#[test]
fn test_apply_url_overrides() {
    let mut catalog = test_catalog();
    let mut overrides = BTreeMap::new();
    overrides.insert("ollama".to_string(), "http://10.0.0.5:11434/v1".to_string());
    overrides.insert("vllm".to_string(), "http://10.0.0.6:8000/v1".to_string());
    overrides.insert("nonexistent".to_string(), "http://nowhere".to_string());

    catalog.apply_url_overrides(&overrides);

    assert_eq!(
        catalog.get_provider("ollama").unwrap().base_url,
        "http://10.0.0.5:11434/v1"
    );
    assert_eq!(
        catalog.get_provider("vllm").unwrap().base_url,
        "http://10.0.0.6:8000/v1"
    );
    // lmstudio should be unchanged
    assert_eq!(
        catalog.get_provider("lmstudio").unwrap().base_url,
        "http://127.0.0.1:1234/v1"
    );
}

/// Build a synthetic catalog with regions defined inline for deterministic testing.
fn region_test_catalog() -> ModelCatalog {
    let provider_a = r#"
[provider]
id = "test-provider"
display_name = "Test Provider"
base_url = "https://api.test.com/v1"
api_key_env = "TEST_API_KEY"

[provider.regions.us]
base_url = "https://us.api.test.com/v1"

[provider.regions.cn]
base_url = "https://cn.api.test.com/v1"
api_key_env = "TEST_CN_API_KEY"

[[models]]
id = "test-model"
display_name = "Test Model"
tier = "smart"
context_window = 32768
max_output_tokens = 4096
input_cost_per_m = 1.0
output_cost_per_m = 3.0
supports_tools = true
supports_vision = false
supports_streaming = true
"#;
    let provider_b = r#"
[provider]
id = "test-provider-nokey"
display_name = "Test Provider No Key"
base_url = "https://api.nokey.com/v1"
api_key_env = "NOKEY_API_KEY"

[provider.regions.eu]
base_url = "https://eu.api.nokey.com/v1"

[[models]]
id = "nokey-model"
display_name = "NoKey Model"
tier = "fast"
context_window = 8192
max_output_tokens = 2048
input_cost_per_m = 0.5
output_cost_per_m = 1.5
supports_tools = false
supports_vision = false
supports_streaming = false
"#;
    let sources = vec![
        (provider_a.to_string(), false),
        (provider_b.to_string(), false),
    ];
    ModelCatalog::from_sources(&sources, None)
}

#[test]
fn test_resolve_region_urls() {
    let catalog = region_test_catalog();

    // Known provider + known region -> URL resolved
    let mut sel = BTreeMap::new();
    sel.insert("test-provider".to_string(), "us".to_string());
    let urls = catalog.resolve_region_urls(&sel);
    assert_eq!(
        urls.get("test-provider").unwrap(),
        "https://us.api.test.com/v1"
    );

    // Known provider + another known region
    sel.clear();
    sel.insert("test-provider".to_string(), "cn".to_string());
    let urls = catalog.resolve_region_urls(&sel);
    assert_eq!(
        urls.get("test-provider").unwrap(),
        "https://cn.api.test.com/v1"
    );

    // Known provider + unknown region -> empty
    sel.clear();
    sel.insert("test-provider".to_string(), "jp".to_string());
    let urls = catalog.resolve_region_urls(&sel);
    assert!(urls.is_empty());
}

#[test]
fn test_resolve_region_api_keys() {
    let catalog = region_test_catalog();

    // Region with api_key_env -> returned
    let mut sel = BTreeMap::new();
    sel.insert("test-provider".to_string(), "cn".to_string());
    let keys = catalog.resolve_region_api_keys(&sel);
    assert_eq!(
        keys.get("test-provider").map(|s| s.as_str()),
        Some("TEST_CN_API_KEY")
    );

    // Region without api_key_env -> excluded
    sel.clear();
    sel.insert("test-provider".to_string(), "us".to_string());
    let keys = catalog.resolve_region_api_keys(&sel);
    assert!(!keys.contains_key("test-provider"));

    // Provider whose region has no api_key_env -> excluded
    sel.clear();
    sel.insert("test-provider-nokey".to_string(), "eu".to_string());
    let keys = catalog.resolve_region_api_keys(&sel);
    assert!(!keys.contains_key("test-provider-nokey"));
}

#[test]
fn test_resolve_region_unknown_provider() {
    let catalog = region_test_catalog();
    let mut sel = BTreeMap::new();
    sel.insert("nonexistent".to_string(), "us".to_string());
    let urls = catalog.resolve_region_urls(&sel);
    assert!(urls.is_empty());
    let keys = catalog.resolve_region_api_keys(&sel);
    assert!(keys.is_empty());
}

#[test]
fn test_codex_models_under_openai() {
    // Codex models are now merged under the "openai" provider
    let catalog = test_catalog();
    let models = catalog.models_by_provider("openai");
    assert!(models.iter().any(|m| m.id == "codex/gpt-4.1"));
    assert!(models.iter().any(|m| m.id == "codex/o4-mini"));
}

#[test]
fn test_codex_aliases() {
    let catalog = test_catalog();
    let entry = catalog.find_model("codex").unwrap();
    assert_eq!(entry.id, "codex/gpt-4.1");
}

#[test]
fn test_claude_code_provider() {
    let catalog = test_catalog();
    let cc = catalog.get_provider("claude-code").unwrap();
    assert_eq!(cc.display_name, "Claude Code");
    assert!(!cc.key_required);
}

#[test]
fn test_claude_code_models() {
    let catalog = test_catalog();
    let models = catalog.models_by_provider("claude-code");
    assert_eq!(models.len(), 3);
    assert!(models.iter().any(|m| m.id == "claude-code/opus"));
    assert!(models.iter().any(|m| m.id == "claude-code/sonnet"));
    assert!(models.iter().any(|m| m.id == "claude-code/haiku"));
}

#[test]
fn test_claude_code_aliases() {
    let catalog = test_catalog();
    let entry = catalog.find_model("claude-code").unwrap();
    assert_eq!(entry.id, "claude-code/sonnet");
}

#[test]
fn test_load_catalog_file_with_provider() {
    let toml_content = r#"
[provider]
id = "test-provider"
display_name = "Test Provider"
api_key_env = "TEST_API_KEY"
base_url = "https://api.test.example.com"
key_required = true

[[models]]
id = "test-model-1"
display_name = "Test Model 1"
provider = "test-provider"
tier = "smart"
context_window = 128000
max_output_tokens = 8192
input_cost_per_m = 1.0
output_cost_per_m = 3.0
supports_tools = true
supports_vision = false
supports_streaming = true
aliases = ["tm1"]
"#;
    let file: ModelCatalogFile = toml::from_str(toml_content).unwrap();
    let mut catalog = test_catalog();
    let initial_models = catalog.list_models().len();
    let initial_providers = catalog.list_providers().len();

    let added = catalog.merge_catalog_file(file);
    assert_eq!(added, 1);
    assert_eq!(catalog.list_models().len(), initial_models + 1);
    assert_eq!(catalog.list_providers().len(), initial_providers + 1);

    // Verify the model was added
    let model = catalog.find_model("test-model-1").unwrap();
    assert_eq!(model.provider, "test-provider");
    assert_eq!(model.tier, ModelTier::Smart);

    // Verify the provider was added
    let provider = catalog.get_provider("test-provider").unwrap();
    assert_eq!(provider.display_name, "Test Provider");
    assert_eq!(provider.base_url, "https://api.test.example.com");
    assert_eq!(provider.model_count, 1);

    // Verify alias was registered
    let aliased = catalog.find_model("tm1").unwrap();
    assert_eq!(aliased.id, "test-model-1");
}

#[test]
fn test_merge_catalog_keeps_existing_api_key_env_when_incoming_empty() {
    let mut catalog = test_catalog();
    let original_env = catalog
        .get_provider("deepseek")
        .expect("deepseek provider should exist in test catalog")
        .api_key_env
        .clone();
    assert!(!original_env.is_empty());

    let toml_content = r#"
[provider]
id = "deepseek"
display_name = "DeepSeek"
api_key_env = ""
base_url = "https://api.deepseek.com/v1"
key_required = true
"#;
    let file: ModelCatalogFile = toml::from_str(toml_content).unwrap();
    let added = catalog.merge_catalog_file(file);
    assert_eq!(added, 0);

    let merged = catalog
        .get_provider("deepseek")
        .expect("deepseek provider should still exist");
    assert_eq!(merged.api_key_env, original_env);
}

#[test]
fn test_load_catalog_file_without_provider() {
    let toml_content = r#"
[[models]]
id = "test-standalone-model"
display_name = "Standalone Model"
provider = "anthropic"
tier = "fast"
context_window = 32000
max_output_tokens = 4096
input_cost_per_m = 0.5
output_cost_per_m = 1.0
supports_tools = true
supports_vision = false
supports_streaming = true
aliases = []
"#;
    let file: ModelCatalogFile = toml::from_str(toml_content).unwrap();
    assert!(file.provider.is_none());

    let mut catalog = test_catalog();
    let added = catalog.merge_catalog_file(file);
    assert_eq!(added, 1);

    let model = catalog.find_model("test-standalone-model").unwrap();
    assert_eq!(model.provider, "anthropic");
}

#[test]
fn test_merge_catalog_skips_duplicate_models() {
    let toml_content = r#"
[[models]]
id = "claude-sonnet-4-6"
display_name = "Claude Sonnet 4.6"
provider = "anthropic"
tier = "smart"
context_window = 200000
max_output_tokens = 64000
input_cost_per_m = 3.0
output_cost_per_m = 15.0
supports_tools = true
supports_vision = true
supports_streaming = true
aliases = []
"#;
    let file: ModelCatalogFile = toml::from_str(toml_content).unwrap();
    let mut catalog = test_catalog();
    let initial_models = catalog.list_models().len();

    let added = catalog.merge_catalog_file(file);
    assert_eq!(added, 0); // Already exists
    assert_eq!(catalog.list_models().len(), initial_models);
}

#[test]
fn test_load_cached_catalog_from_dir() {
    let dir = tempfile::tempdir().unwrap();
    let toml_content = r#"
[provider]
id = "cached-provider"
display_name = "Cached Provider"
api_key_env = "CACHED_API_KEY"
base_url = "https://api.cached.example.com"
key_required = true

[[models]]
id = "cached-model-1"
display_name = "Cached Model 1"
provider = "cached-provider"
tier = "balanced"
context_window = 64000
max_output_tokens = 4096
input_cost_per_m = 0.5
output_cost_per_m = 1.5
supports_tools = true
supports_vision = false
supports_streaming = true
aliases = []
"#;
    std::fs::write(dir.path().join("cached.toml"), toml_content).unwrap();

    let mut catalog = test_catalog();
    let added = catalog.load_cached_catalog(dir.path()).unwrap();
    assert_eq!(added, 1);

    let model = catalog.find_model("cached-model-1").unwrap();
    assert_eq!(model.provider, "cached-provider");

    let provider = catalog.get_provider("cached-provider").unwrap();
    assert_eq!(provider.base_url, "https://api.cached.example.com");
}

#[test]
fn test_builtin_toml_files_parse() {
    // Verify all TOML catalog files in catalog/providers/ are valid
    let catalog_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("catalog")
        .join("providers");
    if catalog_dir.is_dir() {
        let mut total_models = 0;
        let mut total_providers = 0;
        for entry in std::fs::read_dir(&catalog_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                let data = std::fs::read_to_string(&path).unwrap();
                let file: ModelCatalogFile = toml::from_str(&data).unwrap_or_else(|e| {
                    panic!("Failed to parse {}: {e}", path.display());
                });
                if file.provider.is_some() {
                    total_providers += 1;
                }
                total_models += file.models.len();
            }
        }
        // We expect at least 25 providers and 100 models
        assert!(
            total_providers >= 25,
            "Expected at least 25 providers, got {total_providers}"
        );
        assert!(
            total_models >= 100,
            "Expected at least 100 models, got {total_models}"
        );
    }
}

#[test]
fn test_parse_remote_catalog_without_provider_on_models() {
    // Remote model-catalog repo omits `provider` on each [[models]] entry
    // because it's already in the [provider] section.
    let toml_content = r#"
[provider]
id = "test-remote"
display_name = "Test Remote"
api_key_env = "TEST_REMOTE_KEY"
base_url = "https://api.test-remote.example.com"
key_required = true

[[models]]
id = "test-remote-model-1"
display_name = "Test Remote Model 1"
tier = "frontier"
context_window = 200000
max_output_tokens = 128000
input_cost_per_m = 5.0
output_cost_per_m = 25.0
supports_tools = true
supports_vision = true
supports_streaming = true
aliases = ["trm1"]
"#;
    let file: ModelCatalogFile =
        toml::from_str(toml_content).expect("should parse without provider on models");
    assert_eq!(file.models.len(), 1);
    assert!(file.models[0].provider.is_empty());

    let mut catalog = test_catalog();
    let added = catalog.merge_catalog_file(file);
    assert_eq!(added, 1);

    let model = catalog.find_model("test-remote-model-1").unwrap();
    assert_eq!(model.provider, "test-remote");
}

#[test]
fn test_media_capabilities_parsed_from_toml() {
    let toml_content = r#"
[provider]
id = "testprov"
display_name = "Test Provider"
api_key_env = "TEST_KEY"
base_url = "https://api.test.com/v1"
key_required = true
media_capabilities = ["image_generation", "text_to_speech"]

[[models]]
id = "test-model"
display_name = "Test Model"
tier = "smart"
context_window = 128000
max_output_tokens = 4096
input_cost_per_m = 1.0
output_cost_per_m = 2.0
supports_tools = true
supports_vision = false
supports_streaming = true
"#;
    let catalog = ModelCatalog::from_sources(&[(toml_content.to_string(), false)], None);
    let providers = catalog.list_providers();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].id, "testprov");
    assert_eq!(providers[0].media_capabilities.len(), 2);
    assert!(providers[0]
        .media_capabilities
        .contains(&"image_generation".to_string()));
    assert!(providers[0]
        .media_capabilities
        .contains(&"text_to_speech".to_string()));
}

#[test]
fn test_media_capabilities_defaults_to_empty() {
    let toml_content = r#"
[provider]
id = "noprov"
display_name = "No Media"
api_key_env = "NM_KEY"
base_url = "https://api.nomedia.com"
key_required = true

[[models]]
id = "nm-1"
display_name = "NM 1"
tier = "fast"
context_window = 8000
max_output_tokens = 2000
input_cost_per_m = 0.5
output_cost_per_m = 1.0
supports_tools = false
supports_vision = false
supports_streaming = true
"#;
    let catalog = ModelCatalog::from_sources(&[(toml_content.to_string(), false)], None);
    let providers = catalog.list_providers();
    assert_eq!(providers.len(), 1);
    assert!(providers[0].media_capabilities.is_empty());
}

#[test]
fn test_alibaba_coding_plan_provider() {
    let catalog = test_catalog();
    let provider = catalog
        .get_provider("alibaba-coding-plan")
        .expect("alibaba-coding-plan provider should be registered");
    assert_eq!(provider.display_name, "Alibaba Coding Plan (Intl)");
    assert_eq!(provider.api_key_env, "ALIBABA_CODING_PLAN_API_KEY");
    assert_eq!(
        provider.base_url,
        "https://coding-intl.dashscope.aliyuncs.com/v1"
    );
    assert!(provider.key_required);
}

#[test]
fn test_alibaba_coding_plan_has_models() {
    // Smoke check only — the exact model set is owned by the upstream
    // librefang-registry repo and changes over time. Specific model
    // coverage is asserted by name in the sibling tests below.
    let catalog = test_catalog();
    let models = catalog.models_by_provider("alibaba-coding-plan");
    assert!(
        !models.is_empty(),
        "alibaba-coding-plan should expose at least one model"
    );
}

#[test]
fn test_alibaba_coding_plan_zero_cost() {
    let catalog = test_catalog();
    let qwen36plus = catalog
        .find_model("alibaba-coding-plan/qwen3.6-plus")
        .expect("qwen3.6-plus model should be registered");
    assert_eq!(qwen36plus.input_cost_per_m, 0.0);
    assert_eq!(qwen36plus.output_cost_per_m, 0.0);
}

#[test]
fn test_alibaba_coding_plan_vision_models() {
    let catalog = test_catalog();
    let qwen36plus = catalog
        .find_model("alibaba-coding-plan/qwen3.6-plus")
        .expect("qwen3.6-plus model should be registered");
    assert!(qwen36plus.supports_vision);
    assert_eq!(qwen36plus.tier, ModelTier::Smart);
    assert_eq!(qwen36plus.context_window, 1_000_000);
}

#[test]
fn test_alibaba_coding_plan_coder_models() {
    let catalog = test_catalog();
    let coder_plus = catalog
        .find_model("alibaba-coding-plan/qwen3-coder-plus")
        .expect("qwen3-coder-plus model should be registered");
    assert_eq!(coder_plus.tier, ModelTier::Smart);
    assert_eq!(coder_plus.context_window, 1_000_000);

    let coder_next = catalog
        .find_model("alibaba-coding-plan/qwen3-coder-next")
        .expect("qwen3-coder-next model should be registered");
    assert_eq!(coder_next.tier, ModelTier::Frontier);
    assert_eq!(coder_next.context_window, 262_144);
}

#[test]
fn test_alibaba_coding_plan_all_models_support_tools() {
    let catalog = test_catalog();
    let models = catalog.models_by_provider("alibaba-coding-plan");
    for model in models {
        assert!(
            model.supports_tools,
            "Model {} should support tools",
            model.id
        );
        assert!(
            model.supports_streaming,
            "Model {} should support streaming",
            model.id
        );
    }
}

/// Refs #4745. With no override, effective capabilities equal the catalog
/// entry's declared values byte-for-byte.
#[test]
fn effective_capabilities_no_override_returns_catalog_values() {
    let catalog = test_catalog();
    let entry = catalog.find_model("claude-sonnet-4-6").unwrap().clone();
    let eff = catalog.effective_capabilities(&entry);
    assert_eq!(eff.supports_tools, entry.supports_tools);
    assert_eq!(eff.supports_vision, entry.supports_vision);
    assert_eq!(eff.supports_streaming, entry.supports_streaming);
    assert_eq!(eff.supports_thinking, entry.supports_thinking);
}

/// Refs #4745. A user override of `supports_tools = Some(false)` flips the
/// effective value off even when the catalog declares the model as
/// tool-capable. Other capabilities stay at the catalog default since
/// their override fields are `None`.
#[test]
fn effective_capabilities_override_can_force_off() {
    let mut catalog = test_catalog();
    let entry = catalog.find_model("claude-sonnet-4-6").unwrap().clone();
    // sanity — the test is meaningful only when the catalog says tools=true.
    assert!(entry.supports_tools);
    let key = format!("{}:{}", entry.provider, entry.id);
    catalog.set_overrides(
        key,
        ModelOverrides {
            supports_tools: Some(false),
            ..Default::default()
        },
    );
    let eff = catalog.effective_capabilities(&entry);
    assert!(!eff.supports_tools, "override should force tools off");
    assert_eq!(eff.supports_vision, entry.supports_vision);
    assert_eq!(eff.supports_streaming, entry.supports_streaming);
    assert_eq!(eff.supports_thinking, entry.supports_thinking);
}

/// Refs #4745. A user override can also force a capability ON when the
/// catalog declares it as unsupported — this is the headline use case
/// (the issue: provider's `capabilities` field is wrong/missing).
#[test]
fn effective_capabilities_override_can_force_on() {
    let mut catalog = test_catalog();
    // Pick any model where supports_thinking is false in the catalog so
    // the override flip is observable. Using a custom-added entry keeps
    // the test resilient to upstream catalog churn.
    catalog.add_custom_model(ModelCatalogEntry {
        id: "test-no-thinking".to_string(),
        display_name: "Test Model".to_string(),
        provider: "test-provider".to_string(),
        tier: ModelTier::Custom,
        context_window: 8_192,
        max_output_tokens: 2_048,
        input_cost_per_m: 0.0,
        output_cost_per_m: 0.0,
        supports_tools: false,
        supports_vision: false,
        supports_streaming: false,
        supports_thinking: false,
        ..Default::default()
    });
    let entry = catalog.find_model("test-no-thinking").unwrap().clone();
    let key = format!("{}:{}", entry.provider, entry.id);
    catalog.set_overrides(
        key,
        ModelOverrides {
            supports_thinking: Some(true),
            supports_vision: Some(true),
            ..Default::default()
        },
    );
    let eff = catalog.effective_capabilities(&entry);
    assert!(eff.supports_thinking);
    assert!(eff.supports_vision);
    assert!(!eff.supports_tools);
    assert!(!eff.supports_streaming);
}

/// Refs #4745. `effective_capabilities_for` resolves by id-or-alias and
/// applies overrides keyed by `provider:id`.
#[test]
fn effective_capabilities_for_resolves_by_alias() {
    let mut catalog = test_catalog();
    let entry = catalog.find_model("sonnet").unwrap().clone();
    let key = format!("{}:{}", entry.provider, entry.id);
    catalog.set_overrides(
        key,
        ModelOverrides {
            supports_vision: Some(false),
            ..Default::default()
        },
    );
    let eff = catalog
        .effective_capabilities_for("sonnet")
        .expect("alias should resolve");
    assert!(!eff.supports_vision);
}

// ---------------------------------------------------------------------------
// #5137: malformed user config files must be skipped per-file (and logged),
// not silently revert the whole catalog / clobber existing state.
// ---------------------------------------------------------------------------

/// A syntactically broken provider TOML must not take down the sibling
/// valid providers in the same directory. Before #5137 the parse used
/// `if let Ok(...)` and a broken file just vanished with no log; the valid
/// providers still loaded, but a single malformed registry file could
/// previously mask which file was at fault. This pins the per-file skip
/// behaviour so a future refactor can't regress to all-or-nothing.
#[test]
fn malformed_provider_toml_is_skipped_valid_ones_still_load() {
    let tmp = tempfile::tempdir().unwrap();
    let providers_dir = tmp.path().join("providers");
    std::fs::create_dir_all(&providers_dir).unwrap();

    std::fs::write(
        providers_dir.join("good.toml"),
        r#"[provider]
id = "goodprov"
display_name = "Good Provider"
api_key_env = "GOOD_API_KEY"
base_url = "https://good.test"
"#,
    )
    .unwrap();
    // Deliberately broken TOML (unterminated string / bad table).
    std::fs::write(providers_dir.join("broken.toml"), "[provider\nid = \"oops").unwrap();

    let catalog = ModelCatalog::new_from_dir_with_registry(&providers_dir, None);
    assert!(
        catalog.get_provider("goodprov").is_some(),
        "valid provider must still load even though a sibling file is malformed"
    );
}

/// `load_suppressed` on a malformed JSON file must NOT wipe the in-memory
/// suppressed set. Before #5137 the nested `if let Ok(...)` meant a parse
/// error left the set untouched only by accident; a refactor that moved the
/// assignment out would have silently un-suppressed every provider. This
/// pins the contract explicitly.
#[test]
fn load_suppressed_keeps_existing_set_on_parse_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("suppressed.json");
    std::fs::write(&path, "{ this is not json }").unwrap();

    let mut catalog = test_catalog();
    catalog.suppress_provider("openai");
    assert!(catalog.is_suppressed("openai"));

    catalog.load_suppressed(&path);

    assert!(
        catalog.is_suppressed("openai"),
        "a malformed suppressed.json must not silently un-suppress providers (#5137)"
    );
}

/// Same contract for `load_overrides`: a malformed overrides.json must not
/// silently drop the operator's per-model tuning.
#[test]
fn load_overrides_keeps_existing_on_parse_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("overrides.json");
    std::fs::write(&path, "not-valid-json").unwrap();

    let mut catalog = test_catalog();
    let entry = catalog.find_model("sonnet").unwrap().clone();
    let key = format!("{}:{}", entry.provider, entry.id);
    catalog.set_overrides(
        key.clone(),
        ModelOverrides {
            supports_vision: Some(false),
            ..Default::default()
        },
    );
    assert!(catalog.get_overrides(&key).is_some());

    catalog.load_overrides(&path);

    assert!(
        catalog.get_overrides(&key).is_some(),
        "a malformed overrides.json must not silently drop existing overrides (#5137)"
    );
}
