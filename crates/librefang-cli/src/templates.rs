//! Discover and load agent templates from the agents directory.

use std::path::PathBuf;

/// A discovered agent template.
pub struct AgentTemplate {
    /// Template name (directory name).
    pub name: String,
    /// Description from the manifest.
    pub description: String,
    /// Raw TOML content.
    pub content: String,
}

/// Discover template directories. Checks:
/// 1. The repo `agents/` dir (for dev builds)
/// 2. `~/.librefang/workspaces/agents/` (installed templates)
/// 3. `LIBREFANG_AGENTS_DIR` env var
pub fn discover_template_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // Installed templates (respects LIBREFANG_HOME)
    let of_home = if let Ok(h) = std::env::var("LIBREFANG_HOME") {
        PathBuf::from(h)
    } else if let Some(home) = dirs::home_dir() {
        home.join(".librefang")
    } else {
        std::env::temp_dir().join(".librefang")
    };
    {
        let agents = of_home.join("workspaces").join("agents");
        if agents.is_dir() && !dirs.contains(&agents) {
            dirs.push(agents);
        }
    }

    // Environment override
    if let Ok(env_dir) = std::env::var("LIBREFANG_AGENTS_DIR") {
        let p = PathBuf::from(env_dir);
        if p.is_dir() && !dirs.contains(&p) {
            dirs.push(p);
        }
    }

    dirs
}

/// Load all templates from discovered directories, falling back to bundled templates.
pub fn load_all_templates() -> Vec<AgentTemplate> {
    let mut templates = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    // First: load from filesystem (user-installed or dev repo)
    for dir in discover_template_dirs() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let manifest = path.join("agent.toml");
                if !manifest.exists() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if name == "custom" || !seen_names.insert(name.clone()) {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&manifest) {
                    let description = extract_description(&content);
                    templates.push(AgentTemplate {
                        name,
                        description,
                        content,
                    });
                }
            }
        }
    }

    templates.sort_by(|a, b| a.name.cmp(&b.name));
    templates
}

/// Extract the `description` field from raw TOML without full parsing.
fn extract_description(toml_str: &str) -> String {
    for line in toml_str.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("description") {
            if let Some(rest) = rest.trim_start().strip_prefix('=') {
                let val = rest.trim().trim_matches('"');
                return val.to_string();
            }
        }
    }
    String::new()
}

/// Format a template description as a hint for cliclack select items.
pub fn template_display_hint(t: &AgentTemplate) -> String {
    if t.description.is_empty() {
        String::new()
    } else if t.description.chars().count() > 60 {
        let truncated: String = t.description.chars().take(57).collect();
        format!("{truncated}...")
    } else {
        t.description.clone()
    }
}

#[cfg(test)]
mod tests {
    use librefang_types::agent::AgentManifest;
    use librefang_types::config::DefaultModelConfig;

    /// Mirror the kernel's spawn-time + execute-time default_model overlay so
    /// we can verify a manifest with empty/"default" provider+model resolves
    /// to the configured default_model — not to any hardcoded vendor value.
    fn resolve_effective_model(
        manifest: &AgentManifest,
        default_model: &DefaultModelConfig,
    ) -> (String, String) {
        let provider_is_default =
            manifest.model.provider.is_empty() || manifest.model.provider == "default";
        let model_is_default = manifest.model.model.is_empty() || manifest.model.model == "default";
        let effective_provider = if provider_is_default {
            default_model.provider.clone()
        } else {
            manifest.model.provider.clone()
        };
        let effective_model = if model_is_default {
            default_model.model.clone()
        } else {
            manifest.model.model.clone()
        };
        (effective_provider, effective_model)
    }

    /// Bundled example template must not hardcode a provider; it should defer
    /// to the user's configured default_model (regression: openfang #967).
    #[test]
    fn example_custom_agent_template_does_not_hardcode_provider() {
        let toml_str = include_str!("../../../examples/custom-agent/agent.toml");
        let manifest: AgentManifest =
            toml::from_str(toml_str).expect("example agent.toml must parse");

        // Must not pin any specific vendor — otherwise switching default_model
        // in config.toml would have no effect on agents spawned from this template.
        assert_ne!(manifest.model.provider, "groq");
        assert_ne!(manifest.model.model, "llama-3.3-70b-versatile");

        // Must be either empty or the explicit "default" sentinel so the
        // kernel's default_model overlay applies.
        let provider_defers =
            manifest.model.provider.is_empty() || manifest.model.provider == "default";
        let model_defers = manifest.model.model.is_empty() || manifest.model.model == "default";
        assert!(
            provider_defers && model_defers,
            "example template must defer to default_model, got provider={:?} model={:?}",
            manifest.model.provider,
            manifest.model.model
        );
    }

    /// End-to-end: a manifest deferring to default_model resolves to whatever
    /// the user has configured — not to the legacy groq fallback.
    #[test]
    fn manifest_with_default_provider_resolves_to_configured_default_model() {
        let toml_str = include_str!("../../../examples/custom-agent/agent.toml");
        let manifest: AgentManifest =
            toml::from_str(toml_str).expect("example agent.toml must parse");

        // Simulate a user who switched their default to OpenAI.
        let user_default = DefaultModelConfig {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            ..Default::default()
        };

        let (provider, model) = resolve_effective_model(&manifest, &user_default);
        assert_eq!(provider, "openai");
        assert_eq!(model, "gpt-4o");
        assert_ne!(provider, "groq");
        assert_ne!(model, "llama-3.3-70b-versatile");
    }
}
