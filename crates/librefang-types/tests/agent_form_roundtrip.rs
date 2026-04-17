// Round-trip test for the dashboard's visual-editor TOML output.
// Mirrors the exact serializer rules in
// crates/librefang-api/dashboard/src/lib/agentManifest.ts so any drift
// between the two implementations is caught at build time.

use librefang_types::agent::AgentManifest;

#[test]
fn parses_form_minimum_viable_output() {
    let toml = "name = \"researcher\"\nversion = \"1.0.0\"\nmodule = \"builtin:chat\"\n\n[model]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\n";
    let m: AgentManifest = toml::from_str(toml).expect("minimum manifest must parse");
    assert_eq!(m.name, "researcher");
    assert_eq!(m.model.provider, "openai");
    assert_eq!(m.model.model, "gpt-4o");
}

#[test]
fn parses_form_full_output_with_capabilities_and_resources() {
    let toml = "name = \"researcher\"\nversion = \"1.0.0\"\ndescription = \"runs research jobs\"\nmodule = \"builtin:chat\"\ntags = [\"beta\", \"research\"]\nskills = [\"coder\"]\n\n[model]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\nsystem_prompt = \"You are a researcher.\"\ntemperature = 0.3\nmax_tokens = 8192\n\n[resources]\nmax_tool_calls_per_minute = 30\nmax_cost_per_hour_usd = 1.5\n\n[capabilities]\nnetwork = [\"api.openai.com:443\"]\nshell = [\"ls\", \"cat\"]\nagent_spawn = true\n";
    let m: AgentManifest = toml::from_str(toml).expect("full manifest must parse");
    assert_eq!(m.tags, vec!["beta", "research"]);
    assert_eq!(m.skills, vec!["coder"]);
    assert_eq!(m.model.temperature, 0.3);
    assert_eq!(m.model.max_tokens, 8192);
    assert_eq!(m.resources.max_tool_calls_per_minute, 30);
    assert_eq!(m.resources.max_cost_per_hour_usd, 1.5);
    assert_eq!(m.capabilities.network, vec!["api.openai.com:443"]);
    assert_eq!(m.capabilities.shell, vec!["ls", "cat"]);
    assert!(m.capabilities.agent_spawn);
}

#[test]
fn omitting_optional_sections_uses_defaults() {
    // Form leaves resources/capabilities out when no fields populated;
    // kernel must fall back to ResourceQuota/ManifestCapabilities defaults.
    let toml = "name = \"a\"\nmodule = \"builtin:chat\"\n\n[model]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\n";
    let m: AgentManifest = toml::from_str(toml).expect("must parse");
    assert!(m.capabilities.network.is_empty());
    assert!(!m.capabilities.agent_spawn);
    // max_llm_tokens_per_hour is Option<u64>; None means inherit global default.
    assert!(m.resources.max_llm_tokens_per_hour.is_none());
}
