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
    use super::*;
    use librefang_types::agent::AgentManifest;
    use librefang_types::config::DefaultModelConfig;

    // -----------------------------------------------------------------------
    // extract_description — TOML scanner without full parser dependency.
    // -----------------------------------------------------------------------

    #[test]
    fn extract_description_finds_quoted_value() {
        let toml = r#"
name = "demo"
description = "A demo agent"
"#;
        assert_eq!(extract_description(toml), "A demo agent");
    }

    #[test]
    fn extract_description_returns_empty_when_missing() {
        let toml = r#"
name = "demo"
"#;
        assert_eq!(extract_description(toml), "");
    }

    #[test]
    fn extract_description_handles_unquoted_value() {
        // The scanner trims surrounding double-quotes; an unquoted value
        // should still come through verbatim (with its surrounding whitespace
        // trimmed).
        let toml = "description = bare-value\n";
        assert_eq!(extract_description(toml), "bare-value");
    }

    #[test]
    fn extract_description_ignores_lines_with_description_substring() {
        // Only lines whose first non-whitespace token is `description` count;
        // a line like `# description of the agent` must not be picked up.
        let toml = r#"
# description of the agent
name = "demo"
description = "real one"
"#;
        assert_eq!(extract_description(toml), "real one");
    }

    #[test]
    fn extract_description_first_match_wins() {
        // Real TOML would not have two top-level `description` keys, but the
        // scanner is line-based — pin the first-match behaviour so refactors
        // don't silently flip it.
        let toml = r#"
description = "first"
description = "second"
"#;
        assert_eq!(extract_description(toml), "first");
    }

    // -----------------------------------------------------------------------
    // template_display_hint — UI hint formatter with 60-char ellipsis.
    // -----------------------------------------------------------------------

    fn make_template(description: &str) -> AgentTemplate {
        AgentTemplate {
            name: "t".to_string(),
            description: description.to_string(),
            content: String::new(),
        }
    }

    #[test]
    fn display_hint_passes_short_description_through() {
        let t = make_template("short and sweet");
        assert_eq!(template_display_hint(&t), "short and sweet");
    }

    #[test]
    fn display_hint_returns_empty_for_no_description() {
        let t = make_template("");
        assert_eq!(template_display_hint(&t), "");
    }

    #[test]
    fn display_hint_truncates_with_ellipsis_above_60_chars() {
        // 70 'a's → must be truncated to 57 chars + "..." = 60 chars total.
        let long = "a".repeat(70);
        let t = make_template(&long);
        let hint = template_display_hint(&t);
        assert_eq!(hint.chars().count(), 60);
        assert!(hint.ends_with("..."));
        assert!(hint.starts_with(&"a".repeat(57)));
    }

    #[test]
    fn display_hint_keeps_exactly_60_char_description_intact() {
        // Boundary: cutoff is `> 60`, so exactly 60 chars must NOT be
        // truncated.
        let s = "a".repeat(60);
        let t = make_template(&s);
        assert_eq!(template_display_hint(&t), s);
    }

    #[test]
    fn display_hint_counts_chars_not_bytes_for_unicode() {
        // 70 multi-byte characters: must trigger truncation by char count
        // (not by byte length) and must not panic on a non-char-boundary
        // byte slice.
        let s = "汉".repeat(70);
        let t = make_template(&s);
        let hint = template_display_hint(&t);
        assert_eq!(hint.chars().count(), 60);
        assert!(hint.ends_with("..."));
    }

    // -----------------------------------------------------------------------
    // discover_template_dirs — env-var-driven path discovery.
    //
    // These tests mutate process-global env vars; group them in a single
    // test (and serialize via a Mutex) so the cargo parallel harness can't
    // race on LIBREFANG_HOME / LIBREFANG_AGENTS_DIR.
    // -----------------------------------------------------------------------

    /// Process-wide guard for the env-mutating tests in this module: cargo
    /// runs `#[test]` fns in parallel, and `LIBREFANG_HOME`/`LIBREFANG_AGENTS_DIR`
    /// are global state. Both tests must lock the same mutex.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        // If a previous test panicked while holding the guard, the mutex is
        // poisoned but the data inside is still sound — recover and proceed.
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn discover_template_dirs_picks_up_env_override() {
        let _guard = env_lock();

        let tmp = std::env::temp_dir().join("librefang-cli-templates-test-3582");
        let _ = std::fs::create_dir_all(&tmp);

        // Snapshot + override; restore on drop.
        let prev_home = std::env::var("LIBREFANG_HOME").ok();
        let prev_agents = std::env::var("LIBREFANG_AGENTS_DIR").ok();

        // Point HOME at a definitely-empty location so the home branch
        // contributes nothing, then point AGENTS_DIR at our tmp dir.
        let empty_home = std::env::temp_dir().join("librefang-cli-templates-test-3582-empty-home");
        let _ = std::fs::create_dir_all(&empty_home);
        // SAFETY: tests in this module serialize on ENV_LOCK so no other
        // thread in this process is reading these vars concurrently.
        unsafe {
            std::env::set_var("LIBREFANG_HOME", &empty_home);
            std::env::set_var("LIBREFANG_AGENTS_DIR", &tmp);
        }

        let dirs = discover_template_dirs();
        assert!(
            dirs.contains(&tmp),
            "AGENTS_DIR override not picked up: {dirs:?}"
        );

        // Restore.
        // SAFETY: see above — still under ENV_LOCK.
        unsafe {
            match prev_home {
                Some(v) => std::env::set_var("LIBREFANG_HOME", v),
                None => std::env::remove_var("LIBREFANG_HOME"),
            }
            match prev_agents {
                Some(v) => std::env::set_var("LIBREFANG_AGENTS_DIR", v),
                None => std::env::remove_var("LIBREFANG_AGENTS_DIR"),
            }
        }

        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&empty_home);
    }

    #[test]
    fn discover_template_dirs_skips_nonexistent_env_path() {
        let _guard = env_lock();

        let bogus = std::env::temp_dir().join("librefang-cli-templates-test-3582-does-not-exist");
        let _ = std::fs::remove_dir_all(&bogus);

        let prev_home = std::env::var("LIBREFANG_HOME").ok();
        let prev_agents = std::env::var("LIBREFANG_AGENTS_DIR").ok();

        let empty_home =
            std::env::temp_dir().join("librefang-cli-templates-test-3582-empty-home-2");
        let _ = std::fs::create_dir_all(&empty_home);
        // SAFETY: ENV_LOCK serializes env mutation across this module's tests.
        unsafe {
            std::env::set_var("LIBREFANG_HOME", &empty_home);
            std::env::set_var("LIBREFANG_AGENTS_DIR", &bogus);
        }

        let dirs = discover_template_dirs();
        assert!(
            !dirs.contains(&bogus),
            "non-existent AGENTS_DIR must be filtered out: {dirs:?}"
        );

        // SAFETY: see above.
        unsafe {
            match prev_home {
                Some(v) => std::env::set_var("LIBREFANG_HOME", v),
                None => std::env::remove_var("LIBREFANG_HOME"),
            }
            match prev_agents {
                Some(v) => std::env::set_var("LIBREFANG_AGENTS_DIR", v),
                None => std::env::remove_var("LIBREFANG_AGENTS_DIR"),
            }
        }
        let _ = std::fs::remove_dir_all(&empty_home);
    }

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
