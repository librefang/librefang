mod common;

use librefang_types::{
    agent::{AgentManifest, ModelConfig, ModelMode},
    config::KernelConfig,
    model_catalog::ProviderInfo,
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
    kernel.model_catalog_update(|cat| {
        let providers = cat.list_providers().to_vec();
        let mut new_providers = providers;
        new_providers.push(custom_provider.clone());
        *cat = librefang_runtime::model_catalog::ModelCatalog::from_entries(vec![], new_providers);
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
    // SAFETY: single-threaded integration test binary; no other thread races.
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
#[test]
fn keyless_local_provider_allows_routing() {
    let (kernel, tmp) = common::boot_kernel();
    let home = tmp.path();

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
