//! Agent types — the operator-facing projection of an [`AgentManifest`], and the
//! merge rules that keep an edit through that projection non-destructive (#7740).
//!
//! An "agent type" is a reusable agent manifest an operator authors once and spawns from.
//! Every surface that lets a human or an agent author one — `PUT /api/templates/{name}`, the dashboard editor, the TUI templates screen, and the agent-facing `agent_type_create` tool (#7722) — speaks the same small flat shape rather than the full 58-field manifest: a name, a description, a system prompt, a provider, a model, a tool list and a skill list.
//!
//! That projection is lossy in one direction and that is the entire hazard.
//! A manifest carries `[[triggers]]`, `[compaction]`, `max_history_messages`, `mcp_servers`, `tool_allowlist`, `session_mode`, `[workspaces]`, `channels`, `[exec_policy]`, `fallback_models` and forty-odd more fields that the flat shape has no room for.
//! Rebuilding a manifest from the flat shape and writing it over the operator's file therefore deletes everything the shape cannot express, silently, with a 200 response.
//!
//! So this module deliberately offers no "flat shape → manifest" conversion for an existing file.
//! It offers two operations instead:
//!
//! - [`AgentTypeSpec::apply_to`] — a patch applied *over* a manifest that was read from disk.
//!   A field the caller did not send is left exactly as it was found.
//! - [`AgentTypeSpec::into_new_manifest`] — the create path, which starts from a blank manifest because there is nothing on disk to preserve.
//!
//! `into_new_manifest` writes an **exhaustive** struct literal with no `..Default::default()` rest.
//! That is the structural guard against this bug class returning: adding field 59 to `AgentManifest` fails to compile here until someone decides whether a newly-created agent type should carry it, instead of silently widening the set of fields a save can reset.

use crate::agent::{AgentManifest, ManifestCapabilities, ModelConfig};
use serde::{Deserialize, Serialize};

/// The flat agent-type shape, as a **patch**.
///
/// Every field is `Option` so absent and empty stay distinguishable:
/// `None` means "the caller did not mention this, keep what is on disk", and `Some("")` means "the caller cleared it".
/// Collapsing those two — the `unwrap_or("You are a helpful AI agent.")` shape — is what turns a deliberately blank system prompt into canned text on the operator's disk.
///
/// `name` is accepted so the create path and the agent-facing tool can carry it in the body, but a route that already has the name in its URL path is the authority on identity and should ignore this field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTypeSpec {
    /// Agent type name. Ignored by `PUT /api/templates/{name}`, where the URL path segment is authoritative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Human-readable description shown in the catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// System prompt written to `[model] system_prompt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// LLM provider written to `[model] provider`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Model id written to `[model] model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Granted tools, written to `[capabilities] tools`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    /// Installed skill references, written to top-level `skills`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
}

impl AgentTypeSpec {
    /// Apply this patch over a manifest read from disk.
    ///
    /// Only the fields the caller actually sent are touched.
    /// Everything else — including every manifest field this shape cannot express — is left untouched, which is the whole point of routing an edit through here rather than through a constructor.
    pub fn apply_to(&self, manifest: &mut AgentManifest) {
        if let Some(name) = &self.name {
            manifest.name = name.clone();
        }
        if let Some(description) = &self.description {
            manifest.description = description.clone();
        }
        if let Some(system_prompt) = &self.system_prompt {
            manifest.model.system_prompt = system_prompt.clone();
        }
        if let Some(provider) = &self.provider {
            manifest.model.provider = provider.clone();
        }
        if let Some(model) = &self.model {
            manifest.model.model = model.clone();
        }
        if let Some(tools) = &self.tools {
            manifest.capabilities.tools = tools.clone();
        }
        if let Some(skills) = &self.skills {
            manifest.skills = skills.clone();
        }
    }

    /// Build a brand-new agent type manifest from this spec.
    ///
    /// `name` comes from the caller (a URL path segment, or a validated tool argument) rather than from `self.name`, because identity is the one field a create route must not take on trust from the body.
    ///
    /// The literal below is exhaustive on purpose — there is no `..Default::default()` rest pattern.
    /// `AgentManifest` gains fields regularly, and a rest pattern would keep compiling while quietly deciding, on behalf of whoever added the field, that an operator-authored agent type should get its `Default`.
    /// Spelling all 58 out turns that into a compile error at the one place where the decision actually has to be made.
    /// The values are the manifest's own documented defaults; the comment on each group says why it is not derived from the spec.
    pub fn into_new_manifest(self, name: String) -> AgentManifest {
        // `ModelConfig`'s own `Default` substitutes `"default"` for provider and model and canned text for the system prompt.
        // Those are correct as the *unspecified* value — `"default"` is the sentinel the kernel resolves against `[default_model]` — but they must never override something the caller actually sent, including an empty string.
        let model_defaults = ModelConfig::default();
        let capability_defaults = ManifestCapabilities::default();

        AgentManifest {
            // From the spec.
            name,
            description: self.description.unwrap_or_default(),
            model: ModelConfig {
                provider: self.provider.unwrap_or(model_defaults.provider),
                model: self.model.unwrap_or(model_defaults.model),
                system_prompt: self.system_prompt.unwrap_or(model_defaults.system_prompt),
                // Sampling and endpoint overrides are per-deployment, not per-type; an agent type that pinned them would override the operator's global config at every spawn.
                max_tokens: model_defaults.max_tokens,
                temperature: model_defaults.temperature,
                api_key_env: model_defaults.api_key_env,
                base_url: model_defaults.base_url,
                context_window: model_defaults.context_window,
                max_output_tokens: model_defaults.max_output_tokens,
                extra_params: model_defaults.extra_params,
            },
            skills: self.skills.unwrap_or_default(),
            capabilities: ManifestCapabilities {
                tools: self.tools.unwrap_or_default(),
                // The remaining capability grants are deliberately left at their defaults rather than exposed in the flat shape: network egress, shell access and cross-agent messaging are privilege decisions that belong in the manifest an operator reviews, not in a seven-field form.
                network: capability_defaults.network,
                memory_read: capability_defaults.memory_read,
                memory_write: capability_defaults.memory_write,
                agent_spawn: capability_defaults.agent_spawn,
                agent_message: capability_defaults.agent_message,
                shell: capability_defaults.shell,
                ofp_discover: capability_defaults.ofp_discover,
                ofp_connect: capability_defaults.ofp_connect,
            },

            // Identity and provenance the flat shape does not carry.
            version: crate::VERSION.to_string(),
            author: String::new(),
            module: "builtin:chat".to_string(),
            tags: Vec::new(),
            metadata: std::collections::HashMap::new(),
            is_hand: false,

            // Everything below is out of the flat shape's reach and starts at the manifest default.
            // An operator edits these in the agent type's `agent.toml`, and a later save through the flat shape preserves them because `apply_to` never touches them.
            schedule: Default::default(),
            session_mode: Default::default(),
            fallback_models: None,
            resources: Default::default(),
            priority: Default::default(),
            profile: None,
            tools: std::collections::HashMap::new(),
            skills_disabled: false,
            mcp_servers: Vec::new(),
            channels: Vec::new(),
            mcp_disabled: false,
            routing: None,
            autonomous: None,
            pinned_model: None,
            workspace: None,
            generate_identity_files: true,
            workspaces: std::collections::HashMap::new(),
            exec_policy: None,
            tool_allowlist: Vec::new(),
            tool_blocklist: Vec::new(),
            tools_disabled: false,
            response_format: None,
            enabled: true,
            allowed_plugins: Vec::new(),
            inherit_parent_context: true,
            thinking: None,
            context_injection: Vec::new(),
            web_search_augmentation: Default::default(),
            auto_dream_enabled: false,
            auto_dream_min_hours: None,
            auto_dream_min_sessions: None,
            show_progress: true,
            auto_evolve: true,
            channel_overrides: None,
            max_history_messages: None,
            max_concurrent_invocations: None,
            assignee_wake: None,
            cache_context: false,
            tool_exec_backend: None,
            skill_workshop: Default::default(),
            proactive_memory: Default::default(),
            compaction: None,
            context_engine: None,
            rl_export: Default::default(),
            triggers: Vec::new(),
            reconcile_orphans: Default::default(),
            async_tasks: Default::default(),
        }
    }
}

/// The flat projection of a manifest, for the GET side of the round-trip.
///
/// This is the exact inverse of [`AgentTypeSpec::apply_to`]: everything it emits, a subsequent
/// `PUT` of the same document writes back unchanged.
/// Emitting a field here that `apply_to` cannot write, or omitting one that it can, is what makes a GET → edit → PUT cycle lose data, so the two functions are kept adjacent deliberately.
pub fn agent_type_spec_of(manifest: &AgentManifest) -> AgentTypeSpec {
    AgentTypeSpec {
        name: Some(manifest.name.clone()),
        description: Some(manifest.description.clone()),
        system_prompt: Some(manifest.model.system_prompt.clone()),
        provider: Some(manifest.model.provider.clone()),
        model: Some(manifest.model.model.clone()),
        tools: Some(manifest.capabilities.tools.clone()),
        skills: Some(manifest.skills.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_with_extras() -> AgentManifest {
        let mut m = AgentTypeSpec::default().into_new_manifest("researcher".to_string());
        m.max_history_messages = Some(42);
        m.tool_allowlist = vec!["file_read".to_string()];
        m.mcp_servers = vec!["github".to_string()];
        m.session_mode = crate::agent::SessionMode::New;
        m.channels = vec!["telegram".to_string()];
        m
    }

    #[test]
    fn patch_leaves_unmentioned_manifest_fields_alone() {
        let mut manifest = manifest_with_extras();
        let patch = AgentTypeSpec {
            description: Some("edited".to_string()),
            ..AgentTypeSpec::default()
        };
        patch.apply_to(&mut manifest);

        assert_eq!(manifest.description, "edited");
        assert_eq!(manifest.max_history_messages, Some(42));
        assert_eq!(manifest.tool_allowlist, vec!["file_read".to_string()]);
        assert_eq!(manifest.mcp_servers, vec!["github".to_string()]);
        assert_eq!(manifest.session_mode, crate::agent::SessionMode::New);
        assert_eq!(manifest.channels, vec!["telegram".to_string()]);
    }

    #[test]
    fn blank_strings_are_written_through_rather_than_replaced_with_canned_text() {
        let mut manifest = manifest_with_extras();
        manifest.model.system_prompt = "old".to_string();
        let patch = AgentTypeSpec {
            system_prompt: Some(String::new()),
            provider: Some(String::new()),
            model: Some(String::new()),
            ..AgentTypeSpec::default()
        };
        patch.apply_to(&mut manifest);

        assert_eq!(manifest.model.system_prompt, "");
        assert_eq!(manifest.model.provider, "");
        assert_eq!(manifest.model.model, "");
    }

    #[test]
    fn create_uses_manifest_defaults_only_for_fields_the_caller_omitted() {
        let created = AgentTypeSpec {
            system_prompt: Some(String::new()),
            ..AgentTypeSpec::default()
        }
        .into_new_manifest("blank".to_string());

        // Explicitly cleared: stays cleared.
        assert_eq!(created.model.system_prompt, "");
        // Not mentioned at all: the sentinel the kernel resolves against `[default_model]`.
        assert_eq!(created.model.provider, "default");
        assert_eq!(created.model.model, "default");
        assert_eq!(created.name, "blank");
    }

    #[test]
    fn spec_projection_round_trips_through_the_patch() {
        let original = {
            let mut m = manifest_with_extras();
            m.skills = vec!["research".to_string(), "summarize".to_string()];
            m.capabilities.tools = vec!["web_search".to_string()];
            m.model.system_prompt = "Be terse.".to_string();
            m
        };

        // What a GET hands the editor, replayed verbatim by a save that changed nothing.
        let spec = agent_type_spec_of(&original);
        let mut round_tripped = original.clone();
        spec.apply_to(&mut round_tripped);

        assert_eq!(round_tripped.skills, original.skills);
        assert_eq!(
            round_tripped.capabilities.tools,
            original.capabilities.tools
        );
        assert_eq!(
            round_tripped.model.system_prompt,
            original.model.system_prompt
        );
        assert_eq!(round_tripped.description, original.description);
        assert_eq!(round_tripped.max_history_messages, Some(42));
    }

    #[test]
    fn skills_survive_a_save_that_does_not_mention_them() {
        let mut manifest = manifest_with_extras();
        manifest.skills = vec!["research".to_string()];
        AgentTypeSpec {
            description: Some("edited".to_string()),
            ..AgentTypeSpec::default()
        }
        .apply_to(&mut manifest);

        assert_eq!(manifest.skills, vec!["research".to_string()]);
    }

    #[test]
    fn an_explicit_empty_list_still_clears() {
        let mut manifest = manifest_with_extras();
        manifest.skills = vec!["research".to_string()];
        AgentTypeSpec {
            skills: Some(Vec::new()),
            ..AgentTypeSpec::default()
        }
        .apply_to(&mut manifest);

        assert!(manifest.skills.is_empty());
    }

    #[test]
    fn unknown_keys_are_rejected_rather_than_silently_dropped() {
        // A typo'd key in a save body must not deserialize to "field absent", which under
        // patch semantics reads as "keep the old value" and loses the edit without any error.
        let err = serde_json::from_value::<AgentTypeSpec>(serde_json::json!({
            "systemPrompt": "camelCase typo",
        }))
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn a_manifest_serialized_after_a_patch_still_parses() {
        let mut manifest = manifest_with_extras();
        agent_type_spec_of(&manifest).apply_to(&mut manifest);
        let toml = toml::to_string_pretty(&manifest).expect("serialize");
        let parsed: AgentManifest = toml::from_str(&toml).expect("reparse");
        assert_eq!(parsed.max_history_messages, Some(42));
        assert_eq!(parsed.tool_allowlist, vec!["file_read".to_string()]);
    }
}
