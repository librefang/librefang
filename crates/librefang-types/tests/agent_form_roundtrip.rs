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
fn parses_form_with_advanced_sections() {
    // Mirror the kind of TOML the form's serializer emits when every advanced
    // section is filled in. Catches drift between the dashboard's output and
    // the kernel's deserializer (e.g. renamed fields, changed enum variants).
    let toml = r#"name = "agent"
priority = "High"
session_mode = "new"
web_search_augmentation = "always"
schedule = { periodic = { cron = "0 9 * * *" } }
exec_policy = "allowlist"

[model]
provider = "openai"
model = "gpt-4o"
api_key_env = "OPENAI_API_KEY"

[resources]
max_cost_per_month_usd = 50

[capabilities]
memory_read = ["user/*"]
memory_write = ["user/*"]
agent_message = ["*"]
ofp_connect = ["peer-*"]

[thinking]
budget_tokens = 5000
stream_thinking = true

[autonomous]
max_iterations = 100
heartbeat_channel = "telegram"

[routing]
simple_model = "claude-haiku"
medium_model = "claude-sonnet"
complex_model = "claude-opus"
simple_threshold = 100
complex_threshold = 500

[[fallback_models]]
provider = "anthropic"
model = "claude-3-5-sonnet"

[[context_injection]]
name = "policy"
content = "Be polite."
position = "before_user"
"#;
    let m: AgentManifest = toml::from_str(toml).expect("advanced manifest must parse");
    assert_eq!(m.priority, librefang_types::agent::Priority::High);
    assert_eq!(m.session_mode, librefang_types::agent::SessionMode::New);
    let fallbacks = m
        .fallback_models
        .as_deref()
        .expect("fallback_models should be Some");
    assert_eq!(fallbacks.len(), 1);
    assert_eq!(fallbacks[0].model, "claude-3-5-sonnet");
    assert_eq!(m.context_injection.len(), 1);
    assert_eq!(m.context_injection[0].name, "policy");
    assert!(m.thinking.is_some());
    assert_eq!(m.thinking.as_ref().unwrap().budget_tokens, 5000);
    assert!(m.autonomous.is_some());
    assert_eq!(m.autonomous.as_ref().unwrap().max_iterations, 100);
    assert!(m.routing.is_some());
    assert_eq!(m.routing.as_ref().unwrap().simple_model, "claude-haiku");
    assert_eq!(m.capabilities.memory_read, vec!["user/*"]);
}

#[test]
fn parses_form_response_format_json_schema() {
    // The form's JSON-schema serializer emits the schema as an inline TOML
    // table; verify the kernel reads it back as ResponseFormat::JsonSchema.
    let toml = r#"name = "a"
response_format = { type = "json_schema", name = "user", schema = { type = "object" }, strict = true }

[model]
provider = "openai"
model = "gpt-4o"
"#;
    let m: AgentManifest = toml::from_str(toml).expect("response_format must parse");
    let rf = m.response_format.expect("response_format set");
    match rf {
        librefang_types::config::ResponseFormat::JsonSchema { name, strict, .. } => {
            assert_eq!(name, "user");
            assert_eq!(strict, Some(true));
        }
        _ => panic!("expected JsonSchema variant"),
    }
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
