mod common;

use librefang_runtime::kernel_handle::CatalogQuery;
use librefang_types::{
    agent::{AgentManifest, ModelConfig, ModelMode},
    config::KernelConfig,
    model_catalog::{ModelCatalogEntry, ModelTier, ProviderInfo},
};

fn flexible_manifest(provider: &str) -> AgentManifest {
    AgentManifest {
        name: "test-agent".to_string(),
        model: ModelConfig {
            mode: ModelMode::Flexible,
            provider: provider.to_string(),
            model: "test-model".to_string(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn write_profiles(home: &std::path::Path, profiles_toml: &str) {
    std::fs::write(home.join("model_profiles.toml"), profiles_toml).unwrap();
}

/// A catalog provider with a custom `api_key_env` (e.g. `UNSLOTH_API_KEY`
/// instead of the convention `UNSLOTH_STUDIO_API_KEY`) must be routable when
/// that custom env var is set. Regression for the #7781 review P1.
// `set_var` races any other thread that reads the environment, and the
// default libtest harness runs both tests in this binary on a thread pool —
// `keyless_local_provider_allows_routing` boots a kernel that reads the
// environment. `#[serial]` keeps the two off concurrent threads (#7781
// review: the "single-threaded test binary" justification below was wrong).
#[serial_test::serial]
#[test]
fn catalog_defined_env_name_allows_routing() {
    let (kernel, tmp) = common::boot_kernel();
    let home = tmp.path();

    let custom_provider = ProviderInfo {
        id: "unsloth-studio".to_string(),
        display_name: "Unsloth Studio".to_string(),
        api_key_env: "UNSLOTH_API_KEY".to_string(),
        base_url: "https://api.unsloth.example".to_string(),
        key_required: true,
        model_count: 1,
        ..Default::default()
    };
    // The catalog entry for the routed model is load-bearing, not decoration.
    // `model_resolution_declines_routing` runs *before* the credential gate this test is about, and declines a declared remote provider whose catalog does not know the routed model id — the "declared but not yet synced" case the review asked it to cover.
    // Declaring `unsloth-studio` with an empty model list therefore made `route_to_profile` return `None` for a reason with nothing to do with the assertion below, so the test could never pass; the same defect `0afe7d65e` fixed for the unit-level twin of this test in `agent_execution.rs`, which `cargo test --lib` sees and this binary does not.
    kernel.model_catalog_update(|cat| {
        let providers = cat.list_providers().to_vec();
        let mut new_providers = providers;
        new_providers.push(custom_provider.clone());
        *cat = librefang_runtime::model_catalog::ModelCatalog::from_entries(
            vec![ModelCatalogEntry {
                id: "unsloth-7b".to_string(),
                provider: "unsloth-studio".to_string(),
                ..Default::default()
            }],
            new_providers,
        );
    });

    write_profiles(
        home,
        r#"
[[profiles]]
name = "custom-route"
tags = ["code", "implement"]
provider = "unsloth-studio"
model = "unsloth-7b"
cost_tier = "cheap"
priority = 100
max_complexity = 1.0
"#,
    );

    let manifest = flexible_manifest("anthropic");
    let mut cfg = KernelConfig {
        home_dir: home.to_path_buf(),
        model_router: librefang_types::model_profile::ModelRouterConfig {
            enabled: true,
            complexity_threshold: 0.0,
            ..Default::default()
        },
        ..KernelConfig::default()
    };
    cfg.data_dir = home.join("data");

    // Set the catalog-defined env var (not the convention one).
    // SAFETY: `#[serial]` above keeps this test and the keyless one — whose
    // boot_kernel reads the environment — off concurrent threads.
    unsafe { std::env::set_var("UNSLOTH_API_KEY", "test-key") };
    let result = kernel.route_to_profile(&manifest, "implement the new feature", &cfg);
    unsafe { std::env::remove_var("UNSLOTH_API_KEY") };

    assert!(
        result.is_some(),
        "route_to_profile must accept a catalog-defined api_key_env, got None"
    );
    let profile = result.unwrap();
    assert_eq!(profile.provider, "unsloth-studio");
}

/// A local/keyless provider (e.g. ollama) must be routable without any API key.
/// Regression for the #7781 review P1.
#[serial_test::serial]
#[test]
fn keyless_local_provider_allows_routing() {
    let (kernel, tmp) = common::boot_kernel();
    let home = tmp.path();

    // "ollama" is deliberately declared, and with no model matching the profile's `codellama`.
    // Both halves are load-bearing for the reason `0afe7d65e` gave for the unit-level twin of this test: `model_resolution_declines_routing` is `!is_local && provider_declared`, so an *undeclared* provider short-circuits on `provider_declared == false` alone and the assertion below would still hold with the local-provider exemption deleted.
    // Declaring it makes `is_local` the only reason routing survives as far as the credential gate this test is about.
    kernel.model_catalog_update(|cat| {
        let providers = cat.list_providers().to_vec();
        let mut new_providers = providers;
        new_providers.push(ProviderInfo {
            id: "ollama".to_string(),
            display_name: "Ollama".to_string(),
            base_url: "http://127.0.0.1:11434".to_string(),
            key_required: false,
            ..Default::default()
        });
        *cat = librefang_runtime::model_catalog::ModelCatalog::from_entries(vec![], new_providers);
    });

    write_profiles(
        home,
        r#"
[[profiles]]
name = "local-route"
tags = ["code", "implement"]
provider = "ollama"
model = "codellama"
cost_tier = "cheap"
priority = 100
max_complexity = 1.0
"#,
    );

    let manifest = flexible_manifest("anthropic");
    let mut cfg = KernelConfig {
        home_dir: home.to_path_buf(),
        model_router: librefang_types::model_profile::ModelRouterConfig {
            enabled: true,
            complexity_threshold: 0.0,
            ..Default::default()
        },
        ..KernelConfig::default()
    };
    cfg.data_dir = home.join("data");

    // No env var set for ollama — it's keyless.
    let result = kernel.route_to_profile(&manifest, "implement the new feature", &cfg);

    assert!(
        result.is_some(),
        "route_to_profile must accept keyless local providers, got None"
    );
    let profile = result.unwrap();
    assert_eq!(profile.provider, "ollama");
}

/// `resolve_model_profile` must resolve the profile's `model` through the
/// live catalog alias table the same way `route_to_profile` resolves it.
/// Every builtin profile (`model_profiles.toml`) names an alias like
/// `"haiku"` rather than a concrete model id, and an unresolved alias
/// reaches the wire request verbatim — the spawned agent then fails auth on
/// its first turn. Regression for the #7789 review (agent.rs:234).
#[test]
fn resolve_model_profile_resolves_catalog_alias() {
    let (kernel, tmp) = common::boot_kernel();
    let home = tmp.path();

    // The alias has to arrive attached to a real catalog entry, not on its own.
    // Resolution is provider-scoped (`find_model_for_manifest`, filtered to the
    // profile's own provider), and every path through it ends at
    // `models.iter().find(|m| m.id == canonical)` — a bare `add_alias` naming an
    // id no entry carries resolves to nothing. `add_custom_model` registers the
    // entry and its `aliases` in one call, which is also how a real provider
    // file lands in the catalog.
    kernel.model_catalog_update(|cat| {
        cat.add_custom_model(ModelCatalogEntry {
            id: "claude-haiku-4-5-canonical".to_string(),
            display_name: "Claude Haiku 4.5".to_string(),
            provider: "anthropic".to_string(),
            tier: ModelTier::Custom,
            aliases: vec!["quick-alias".to_string()],
            ..Default::default()
        });
    });

    write_profiles(
        home,
        r#"
[[profiles]]
name = "quick"
tags = ["quick"]
provider = "anthropic"
model = "quick-alias"
cost_tier = "cheap"
priority = 100
max_complexity = 1.0
"#,
    );

    let profile = kernel
        .resolve_model_profile("quick")
        .expect("profile 'quick' must resolve from the catalog");
    assert_eq!(
        profile.model, "claude-haiku-4-5-canonical",
        "resolve_model_profile must resolve the profile's model through the \
         catalog alias table, not hand back the alias verbatim"
    );
}

/// `check_provider_credentials` must refuse a provider nobody configured a
/// key for, naming the env var the operator would set — the same guard
/// `route_to_profile` applies per turn, now available to gate `agent_spawn`
/// before a child is born onto an unauthenticated provider. Regression for
/// the #7789 review (agent.rs:233).
#[test]
fn check_provider_credentials_refuses_unconfigured_provider() {
    let (kernel, _tmp) = common::boot_kernel();

    // A synthetic provider id whose convention env var
    // (`CHECK_TEST_PROVIDER_API_KEY`) nothing in a real environment sets, so
    // the refusal is reached deterministically regardless of ambient
    // developer credentials.
    let err = kernel
        .check_provider_credentials("check-test-provider")
        .expect_err("an unconfigured, non-local provider must be refused");
    assert!(
        err.contains("CHECK_TEST_PROVIDER_API_KEY"),
        "refusal must name the missing env var, got: {err}"
    );
}

/// A local/keyless provider must never be refused for missing credentials —
/// mirrors `keyless_local_provider_allows_routing` above, but through the
/// new `check_provider_credentials` gate rather than `route_to_profile`.
#[test]
fn check_provider_credentials_allows_local_provider() {
    let (kernel, _tmp) = common::boot_kernel();
    assert!(
        kernel.check_provider_credentials("ollama").is_ok(),
        "a local/keyless provider must not be refused for missing credentials"
    );
}
