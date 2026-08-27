//! Privacy pass over an [`AgentManifest`] before it leaves the machine it was authored on (#7771).
//!
//! An agent type built on an operator's own install is an `AgentManifest` written against one machine.
//! It carries an absolute workspace path under that operator's home directory, the name of the environment variable holding their provider credentials, a self-hosted or private base URL, a command and environment allowlist that encodes their local security policy, an arbitrary metadata bag, and free text they pasted into a system prompt or a context injection.
//! Publishing that verbatim to a shared registry puts all of it into a public git history, where it stays.
//! The registry's own validator requires only `name`, `description` and `module`, so nothing on the way in catches it.
//!
//! This module is the pass that has to run first, in two deliberately separate halves:
//!
//! - [`sanitize_for_publication`] returns a publishable copy with the instance-specific fields removed or reset and the portable half untouched.
//!   It never touches the caller's manifest and never writes a file.
//! - [`scan_for_publication`] reports what it found, with a bounded preview per finding, so the operator sees what is about to be dropped and confirms.
//!
//! The detector is deliberately **not** "whatever the sanitiser removes".
//! It also scans every field the sanitiser keeps, because an operator can paste anything into a system prompt and no structural rule separates portable configuration from an internal hostname sitting inside free text.
//! Each [`Finding`] says which of the two cases it is via [`Finding::removed_by_sanitizer`]: `true` means confirming publication is enough, `false` means the operator has to edit the value by hand.
//! Keeping the halves apart is the point — the operator decides, rather than having their file quietly rewritten on their behalf.
//!
//! # Why the classification cannot silently rot
//!
//! A hardcoded field list that falls behind the struct is the failure mode for a pass like this: a new `AgentManifest` field defaults into the publishable copy and leaks on the next promotion, with nothing to notice.
//! Three mechanisms prevent that here.
//!
//! 1. [`sanitize_for_publication`] destructures `AgentManifest` with **no `..` rest pattern** and rebuilds it field by field with **no struct-update syntax**.
//!    A new field therefore fails to compile in this module — `E0027` on the pattern, `E0063` on the construction — until someone names it on both sides and so decides what happens to it.
//!    The same holds for the three types whose interiors are reduced rather than kept or dropped whole: [`ModelConfig`], [`FallbackModel`] and [`ManifestCapabilities`].
//! 2. [`CLASSIFICATION`] is the human-readable half of the same decision, and `classification_covers_every_serialized_manifest_field` asserts the table and the serialized struct agree exactly, in both directions.
//! 3. The detector's scan of retained values walks the *serialized* publishable manifest rather than a list of field names, so a newly added portable field is scanned for pasted secrets from the moment it exists.
//!
//! # Best-effort, by construction
//!
//! The value-level scanners share the conservative denylist in [`crate::taint`], the same one the outbound tool-call sink uses, plus host-path and private-endpoint shape rules defined here.
//! They will miss an obfuscated credential and they will occasionally flag a structured identifier.
//! That is acceptable for a detector whose output a human reads before deciding; it would not be acceptable for a gate that blocks on its own, and nothing here is one.

use crate::agent::{AgentManifest, FallbackModel, ManifestCapabilities, ModelConfig};
use crate::taint::{self, TaintRuleId, TaintSink};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

/// Maximum characters of a dropped or flagged value echoed back in a [`Finding::preview`].
pub const PREVIEW_MAX_CHARS: usize = 96;

/// Maximum nesting depth the retained-value walk descends before giving up.
///
/// A manifest's own shape is far shallower than this; the cap exists because `response_format`'s JSON schema is operator-supplied and arbitrarily deep, and a recursive walk over attacker-shaped input should not be able to exhaust the stack.
/// Exceeding it produces an [`PrivacyCategory::Unscannable`] finding rather than a silent truncation.
const MAX_SCAN_DEPTH: usize = 24;

/// Shortest unbroken alphanumeric run that makes a value look like an opaque credential rather than a structured identifier.
///
/// Model ids, module paths and provider slugs break into short segments at `-`, `_`, `/`, `.` or `:` — `claude-sonnet-4-20250514` and `accounts/fireworks/models/llama-v3p1-405b-instruct` both top out at eight characters.
/// Real opaque tokens carry one long unbroken run.
/// Without this gate the taint scanner's `OpaqueToken` rule fires on every long model id in the manifest, and a detector that cries wolf on `model.model` teaches the operator to click through the findings that matter.
const OPAQUE_RUN_THRESHOLD: usize = 20;

/// Why a value must not travel with a published agent type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyCategory {
    /// An absolute path, or anything else describing the host's filesystem layout.
    HostPath,
    /// The name of an environment variable that holds a credential.
    ///
    /// The name is not the secret, but it names the operator's key management and is meaningless on anyone else's machine.
    CredentialBinding,
    /// A base URL, network host allowlist entry, or peer pattern that may point at a private, self-hosted or internal endpoint.
    PrivateEndpoint,
    /// Host-specific security or execution policy — command and environment allowlists, the tool-execution backend, the context-engine wiring.
    HostPolicy,
    /// Free-form operator-authored literal text, which is where internal documentation, customer names and hostnames end up.
    OperatorText,
    /// An arbitrary operator-supplied key/value bag, or the author identity.
    OperatorMetadata,
    /// A reference to something else on this install — another agent by name, a local workflow id — that does not exist on the machine installing the type.
    LocalWiring,
    /// A flag describing this install's runtime state or provenance rather than the agent type itself.
    LocalState,
    /// A credential-shaped literal found inside a value, by the same denylist the outbound tool-call sink uses.
    SecretLiteral,
    /// Personally identifiable information — an e-mail address, phone number, card number or SSN — found inside a value.
    PersonalData,
    /// The value could not be rendered for scanning, so it is unreviewed.
    /// Treat it as suspect rather than clean.
    Unscannable,
}

impl PrivacyCategory {
    /// Stable snake_case identifier, matching the serialized form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HostPath => "host_path",
            Self::CredentialBinding => "credential_binding",
            Self::PrivateEndpoint => "private_endpoint",
            Self::HostPolicy => "host_policy",
            Self::OperatorText => "operator_text",
            Self::OperatorMetadata => "operator_metadata",
            Self::LocalWiring => "local_wiring",
            Self::LocalState => "local_state",
            Self::SecretLiteral => "secret_literal",
            Self::PersonalData => "personal_data",
            Self::Unscannable => "unscannable",
        }
    }
}

impl fmt::Display for PrivacyCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What [`sanitize_for_publication`] does with one `AgentManifest` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldClass {
    /// Travels verbatim.
    /// This is the half that makes the agent type useful to someone else, and dropping it would leave nothing worth publishing.
    Portable,
    /// Removed, or reset to the type-level default. Carries the reason.
    Stripped(PrivacyCategory),
    /// Kept, but with named inner fields stripped.
    /// Carries the reason those inner fields go.
    /// The reduction itself is exhaustive over the inner type, so a new inner field also has to be classified.
    Reduced(PrivacyCategory),
}

/// Every `AgentManifest` field and what publication does to it.
///
/// This is documentation with a test attached, not the implementation: [`sanitize_for_publication`] is the implementation, and `classification_covers_every_serialized_manifest_field` asserts this table and the struct agree in both directions, so neither can drift ahead of the other.
pub const CLASSIFICATION: &[(&str, FieldClass)] = &[
    ("name", FieldClass::Portable),
    ("version", FieldClass::Portable),
    ("description", FieldClass::Portable),
    // An operator's `author` is routinely a local username or an e-mail address.
    // Attribution for a contribution belongs to the pull request that carries it, not to a field copied off one machine.
    (
        "author",
        FieldClass::Stripped(PrivacyCategory::OperatorMetadata),
    ),
    ("module", FieldClass::Portable),
    ("schedule", FieldClass::Portable),
    ("session_mode", FieldClass::Portable),
    // `provider` / `model` / `system_prompt` / sampling knobs travel; `api_key_env`, `base_url` and the free-form `extra_params` do not.
    (
        "model",
        FieldClass::Reduced(PrivacyCategory::CredentialBinding),
    ),
    (
        "fallback_models",
        FieldClass::Reduced(PrivacyCategory::CredentialBinding),
    ),
    ("resources", FieldClass::Portable),
    ("priority", FieldClass::Portable),
    // `tools` / memory scopes / spawn and message grants travel; the `network`, `shell` and `ofp_connect` allowlists are host policy naming host targets.
    (
        "capabilities",
        FieldClass::Reduced(PrivacyCategory::HostPolicy),
    ),
    ("profile", FieldClass::Portable),
    // Per-tool `params` is an untyped operator-supplied bag — the natural place for an inline token, an internal URL or a local path.
    ("tools", FieldClass::Stripped(PrivacyCategory::OperatorText)),
    ("skills", FieldClass::Portable),
    ("skills_disabled", FieldClass::Portable),
    // Server *names*, not URLs — MCP endpoints and headers live in `KernelConfig`, never in a manifest.
    // The names are what makes the type reproducible, so they travel.
    ("mcp_servers", FieldClass::Portable),
    // `channel_type` strings ("telegram", "discord"), not bindings.
    // The tokens and chat ids live in `KernelConfig`.
    ("channels", FieldClass::Portable),
    ("mcp_disabled", FieldClass::Portable),
    (
        "metadata",
        FieldClass::Stripped(PrivacyCategory::OperatorMetadata),
    ),
    ("tags", FieldClass::Portable),
    ("routing", FieldClass::Portable),
    ("autonomous", FieldClass::Portable),
    ("pinned_model", FieldClass::Portable),
    ("workspace", FieldClass::Stripped(PrivacyCategory::HostPath)),
    ("generate_identity_files", FieldClass::Portable),
    // Both `path` and `mount` describe the host's directory layout, and the symbolic names themselves are often the operator's own vocabulary.
    (
        "workspaces",
        FieldClass::Stripped(PrivacyCategory::HostPath),
    ),
    (
        "exec_policy",
        FieldClass::Stripped(PrivacyCategory::HostPolicy),
    ),
    ("tool_allowlist", FieldClass::Portable),
    ("tool_blocklist", FieldClass::Portable),
    ("tools_disabled", FieldClass::Portable),
    ("response_format", FieldClass::Portable),
    ("enabled", FieldClass::Stripped(PrivacyCategory::LocalState)),
    ("allowed_plugins", FieldClass::Portable),
    ("inherit_parent_context", FieldClass::Portable),
    ("thinking", FieldClass::Portable),
    (
        "context_injection",
        FieldClass::Stripped(PrivacyCategory::OperatorText),
    ),
    ("is_hand", FieldClass::Stripped(PrivacyCategory::LocalState)),
    ("web_search_augmentation", FieldClass::Portable),
    ("auto_dream_enabled", FieldClass::Portable),
    ("auto_dream_min_hours", FieldClass::Portable),
    ("auto_dream_min_sessions", FieldClass::Portable),
    ("show_progress", FieldClass::Portable),
    ("auto_evolve", FieldClass::Portable),
    // Free-text `system_prompt` / `model` overrides plus `group_trigger_patterns`, which are regexes naming the operator's bot aliases and the people it answers to.
    // Dropped whole rather than reduced: the type is a large and fast-moving config struct, and an exhaustive reduction over it would couple this pass to churn it has no stake in.
    (
        "channel_overrides",
        FieldClass::Stripped(PrivacyCategory::OperatorText),
    ),
    ("max_history_messages", FieldClass::Portable),
    ("max_concurrent_invocations", FieldClass::Portable),
    ("assignee_wake", FieldClass::Portable),
    ("cache_context", FieldClass::Portable),
    // `ssh` / `daytona` name execution infrastructure that only resolves against the matching `[tool_exec.*]` subtable on this install.
    (
        "tool_exec_backend",
        FieldClass::Stripped(PrivacyCategory::HostPolicy),
    ),
    ("skill_workshop", FieldClass::Portable),
    ("proactive_memory", FieldClass::Portable),
    ("compaction", FieldClass::Portable),
    // Plugin names, local Python hook script paths and a sidecar endpoint.
    (
        "context_engine",
        FieldClass::Stripped(PrivacyCategory::HostPolicy),
    ),
    ("rl_export", FieldClass::Portable),
    // `target_agent` and `workflow_id` name records on this install, and `prompt_template` is operator free text.
    (
        "triggers",
        FieldClass::Stripped(PrivacyCategory::LocalWiring),
    ),
    ("reconcile_orphans", FieldClass::Portable),
    ("async_tasks", FieldClass::Portable),
];

/// One privacy risk in a manifest that is about to be published.
///
/// Findings are operator-facing review output.
/// They are not an error type and they are not a gate: the caller shows them and asks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// Dotted path to the value, e.g. `model.api_key_env`, `fallback_models[1].base_url`, `capabilities.agent_message[0]`.
    pub field: String,
    /// Why this value must not travel.
    pub category: PrivacyCategory,
    /// Bounded, single-line rendering of the offending value, capped at [`PREVIEW_MAX_CHARS`] characters.
    ///
    /// This may contain the very material the finding is about.
    /// Show it to the operator who owns it; do not log it, and do not attach it to the published artefact.
    pub preview: String,
    /// `true` when [`sanitize_for_publication`] already removes this value, so confirming publication is enough.
    ///
    /// `false` when the value sits inside a field the sanitiser keeps — a hostname pasted into a system prompt, an absolute path in a description — and the operator has to edit it by hand before publishing.
    pub removed_by_sanitizer: bool,
}

/// Return a publishable copy of `manifest` with the instance-specific fields removed or reset, leaving the portable half untouched.
///
/// The input is borrowed and never mutated.
/// The result is still a valid `AgentManifest`: it round-trips through TOML and keeps `name`, `description` and `module`, the three fields the registry validator requires.
///
/// Pair it with [`scan_for_publication`], which reports what this removed *and* what it had to keep, so the operator can confirm before anything is published.
/// See the module docs for why a new `AgentManifest` field cannot slip through here.
pub fn sanitize_for_publication(manifest: &AgentManifest) -> AgentManifest {
    // Exhaustive destructure — no `..`.
    // A new field fails to compile here until it is named, and `_` marks a deliberate decision to drop it.
    let AgentManifest {
        name,
        version,
        description,
        author: _,
        // Instance-specific: the principal an agent acts for names a `[[users]]` /
        // `[[groups]]` entry in *this* deployment, which means nothing in another one.
        owner: _,
        module,
        schedule,
        session_mode,
        model,
        fallback_models,
        resources,
        priority,
        capabilities,
        profile,
        tools: _,
        skills,
        skills_disabled,
        mcp_servers,
        channels,
        mcp_disabled,
        metadata: _,
        tags,
        routing,
        autonomous,
        pinned_model,
        workspace: _,
        generate_identity_files,
        workspaces: _,
        exec_policy: _,
        tool_allowlist,
        tool_blocklist,
        tools_disabled,
        response_format,
        enabled: _,
        allowed_plugins,
        inherit_parent_context,
        thinking,
        context_injection: _,
        is_hand: _,
        web_search_augmentation,
        auto_dream_enabled,
        auto_dream_min_hours,
        auto_dream_min_sessions,
        show_progress,
        auto_evolve,
        channel_overrides: _,
        max_history_messages,
        max_concurrent_invocations,
        assignee_wake,
        cache_context,
        tool_exec_backend: _,
        skill_workshop,
        proactive_memory,
        compaction,
        context_engine: _,
        rl_export,
        triggers: _,
        reconcile_orphans,
        async_tasks,
    } = manifest.clone();

    // Exhaustive construction — no `..Default::default()`.
    // A new field fails to compile here too.
    AgentManifest {
        name,
        version,
        description,
        author: String::new(),
        owner: None,
        module,
        schedule,
        session_mode,
        model: reduce_model(model),
        fallback_models: fallback_models
            .map(|chain| chain.into_iter().map(reduce_fallback_model).collect()),
        resources,
        priority,
        capabilities: reduce_capabilities(capabilities),
        profile,
        tools: HashMap::new(),
        skills,
        skills_disabled,
        mcp_servers,
        channels,
        mcp_disabled,
        metadata: HashMap::new(),
        tags,
        routing,
        autonomous,
        pinned_model,
        workspace: None,
        generate_identity_files,
        workspaces: HashMap::new(),
        exec_policy: None,
        tool_allowlist,
        tool_blocklist,
        tools_disabled,
        response_format,
        // A type published from a disabled agent should install enabled; `false` here describes this install, not the type.
        enabled: true,
        allowed_plugins,
        inherit_parent_context,
        thinking,
        context_injection: Vec::new(),
        is_hand: false,
        web_search_augmentation,
        auto_dream_enabled,
        auto_dream_min_hours,
        auto_dream_min_sessions,
        show_progress,
        auto_evolve,
        channel_overrides: None,
        max_history_messages,
        max_concurrent_invocations,
        assignee_wake,
        cache_context,
        tool_exec_backend: None,
        skill_workshop,
        proactive_memory,
        compaction,
        context_engine: None,
        rl_export,
        triggers: Vec::new(),
        reconcile_orphans,
        async_tasks,
    }
}

/// Drop the credential binding, the endpoint override and the free-form provider extensions from a model config, keeping the parts that name a publicly available model.
fn reduce_model(model: ModelConfig) -> ModelConfig {
    let ModelConfig {
        provider,
        model,
        max_tokens,
        temperature,
        system_prompt,
        api_key_env: _,
        base_url: _,
        context_window,
        max_output_tokens,
        extra_params: _,
    } = model;

    ModelConfig {
        provider,
        model,
        max_tokens,
        temperature,
        system_prompt,
        api_key_env: None,
        base_url: None,
        context_window,
        max_output_tokens,
        extra_params: BTreeMap::new(),
    }
}

/// Same reduction as [`reduce_model`], for one entry of a fallback chain.
fn reduce_fallback_model(entry: FallbackModel) -> FallbackModel {
    let FallbackModel {
        provider,
        model,
        api_key_env: _,
        base_url: _,
        extra_params: _,
    } = entry;

    FallbackModel {
        provider,
        model,
        api_key_env: None,
        base_url: None,
        extra_params: BTreeMap::new(),
    }
}

/// Drop the capability grants that enumerate host targets, keeping the tool, memory and agent-messaging grants that describe what the type does.
fn reduce_capabilities(capabilities: ManifestCapabilities) -> ManifestCapabilities {
    let ManifestCapabilities {
        network: _,
        tools,
        memory_read,
        memory_write,
        agent_spawn,
        agent_message,
        shell: _,
        ofp_discover,
        ofp_connect: _,
    } = capabilities;

    ManifestCapabilities {
        network: Vec::new(),
        tools,
        memory_read,
        memory_write,
        agent_spawn,
        agent_message,
        shell: Vec::new(),
        ofp_discover,
        ofp_connect: Vec::new(),
    }
}

/// Report every privacy risk in `manifest`, in a stable order.
///
/// Two passes, and the second is the reason this is not just a diff against [`sanitize_for_publication`]:
///
/// 1. Every instance-specific field that is actually set, with a bounded preview of what publication would drop ([`Finding::removed_by_sanitizer`] `== true`).
/// 2. Every string the publishable copy still carries, scanned for credential shapes, personal data, absolute host paths and private endpoints ([`Finding::removed_by_sanitizer`] `== false`).
///    These are the ones the operator has to fix by hand — the sanitiser cannot know which half of a system prompt is the internal hostname.
///
/// An empty result means nothing was detected, not that publication is provably safe: see the module docs on the best-effort nature of the value-level scanners.
pub fn scan_for_publication(manifest: &AgentManifest) -> Vec<Finding> {
    let mut findings = Vec::new();
    collect_stripped_findings(manifest, &mut findings);
    collect_retained_findings(manifest, &mut findings);
    findings
}

fn stripped(field: &str, category: PrivacyCategory, preview: String) -> Finding {
    Finding {
        field: field.to_string(),
        category,
        preview,
        removed_by_sanitizer: true,
    }
}

/// Pass 1 — the instance-specific fields, in `AgentManifest` declaration order.
fn collect_stripped_findings(manifest: &AgentManifest, out: &mut Vec<Finding>) {
    use PrivacyCategory as C;

    if !manifest.author.is_empty() {
        out.push(stripped(
            "author",
            C::OperatorMetadata,
            preview_text(&manifest.author),
        ));
    }
    if let Some(env) = &manifest.model.api_key_env {
        out.push(stripped(
            "model.api_key_env",
            C::CredentialBinding,
            preview_text(env),
        ));
    }
    if let Some(url) = &manifest.model.base_url {
        out.push(stripped(
            "model.base_url",
            C::PrivateEndpoint,
            preview_text(url),
        ));
    }
    if !manifest.model.extra_params.is_empty() {
        out.push(stripped(
            "model.extra_params",
            C::OperatorMetadata,
            preview_json(&manifest.model.extra_params),
        ));
    }
    for (index, entry) in manifest.fallback_models.iter().flatten().enumerate() {
        if let Some(env) = &entry.api_key_env {
            out.push(stripped(
                &format!("fallback_models[{index}].api_key_env"),
                C::CredentialBinding,
                preview_text(env),
            ));
        }
        if let Some(url) = &entry.base_url {
            out.push(stripped(
                &format!("fallback_models[{index}].base_url"),
                C::PrivateEndpoint,
                preview_text(url),
            ));
        }
        if !entry.extra_params.is_empty() {
            out.push(stripped(
                &format!("fallback_models[{index}].extra_params"),
                C::OperatorMetadata,
                preview_json(&entry.extra_params),
            ));
        }
    }
    if !manifest.capabilities.network.is_empty() {
        out.push(stripped(
            "capabilities.network",
            C::PrivateEndpoint,
            preview_list(&manifest.capabilities.network),
        ));
    }
    if !manifest.capabilities.shell.is_empty() {
        out.push(stripped(
            "capabilities.shell",
            C::HostPolicy,
            preview_list(&manifest.capabilities.shell),
        ));
    }
    if !manifest.capabilities.ofp_connect.is_empty() {
        out.push(stripped(
            "capabilities.ofp_connect",
            C::PrivateEndpoint,
            preview_list(&manifest.capabilities.ofp_connect),
        ));
    }
    if !manifest.tools.is_empty() {
        out.push(stripped(
            "tools",
            C::OperatorText,
            preview_sorted_keys(manifest.tools.keys()),
        ));
    }
    if !manifest.metadata.is_empty() {
        out.push(stripped(
            "metadata",
            C::OperatorMetadata,
            preview_sorted_keys(manifest.metadata.keys()),
        ));
    }
    if let Some(workspace) = &manifest.workspace {
        out.push(stripped(
            "workspace",
            C::HostPath,
            preview_text(&workspace.display().to_string()),
        ));
    }
    if !manifest.workspaces.is_empty() {
        // Sorted: the source is a `HashMap`, and a preview whose contents reshuffle between runs is not reviewable.
        let mut entries: Vec<String> = manifest
            .workspaces
            .iter()
            .map(|(alias, decl)| {
                let target = decl
                    .mount
                    .as_ref()
                    .or(decl.path.as_ref())
                    .map(|path| path.display().to_string())
                    .unwrap_or_default();
                format!("{alias} -> {target}")
            })
            .collect();
        entries.sort();
        out.push(stripped(
            "workspaces",
            C::HostPath,
            preview_text(&entries.join(", ")),
        ));
    }
    if let Some(policy) = &manifest.exec_policy {
        out.push(stripped("exec_policy", C::HostPolicy, preview_json(policy)));
    }
    if !manifest.enabled {
        out.push(stripped("enabled", C::LocalState, "false".to_string()));
    }
    if !manifest.context_injection.is_empty() {
        let names: Vec<&str> = manifest
            .context_injection
            .iter()
            .map(|injection| injection.name.as_str())
            .collect();
        out.push(stripped(
            "context_injection",
            C::OperatorText,
            preview_text(&format!(
                "{} entr{}: {}",
                names.len(),
                if names.len() == 1 { "y" } else { "ies" },
                names.join(", ")
            )),
        ));
    }
    if manifest.is_hand {
        out.push(stripped("is_hand", C::LocalState, "true".to_string()));
    }
    if let Some(overrides) = &manifest.channel_overrides {
        out.push(stripped(
            "channel_overrides",
            C::OperatorText,
            preview_json(overrides),
        ));
    }
    if let Some(backend) = &manifest.tool_exec_backend {
        out.push(stripped(
            "tool_exec_backend",
            C::HostPolicy,
            preview_json(backend),
        ));
    }
    if let Some(engine) = &manifest.context_engine {
        out.push(stripped(
            "context_engine",
            C::HostPolicy,
            preview_json(engine),
        ));
    }
    if !manifest.triggers.is_empty() {
        let mut targets: Vec<String> = manifest
            .triggers
            .iter()
            .filter_map(|trigger| {
                trigger
                    .target_agent
                    .clone()
                    .or_else(|| trigger.workflow_id.clone())
            })
            .filter(|target| !target.is_empty())
            .collect();
        targets.sort();
        targets.dedup();
        let suffix = if targets.is_empty() {
            String::new()
        } else {
            format!(", targets: {}", targets.join(", "))
        };
        out.push(stripped(
            "triggers",
            C::LocalWiring,
            preview_text(&format!("{} trigger(s){suffix}", manifest.triggers.len())),
        ));
    }
}

/// Pass 2 — every string the publishable copy still carries.
///
/// The walk is over the *serialized* publishable manifest rather than a list of field names, so a field added to `AgentManifest` tomorrow is scanned from the moment it exists.
/// Ordering is stable: nothing that survives [`sanitize_for_publication`] is a `HashMap`, and `serde_json`'s object representation is ordered, so the same manifest always yields the same finding sequence.
fn collect_retained_findings(manifest: &AgentManifest, out: &mut Vec<Finding>) {
    let publishable = sanitize_for_publication(manifest);
    let value = match serde_json::to_value(&publishable) {
        Ok(value) => value,
        Err(e) => {
            // Defensive rather than reachable today: `serde_json` maps a non-finite float to null instead of failing, and nothing in a manifest uses a non-string map key.
            // It stays because reporting a clean manifest when the scan could not run is the one failure mode this module must never have, and that should not become possible by accident.
            out.push(Finding {
                field: "<manifest>".to_string(),
                category: PrivacyCategory::Unscannable,
                preview: preview_text(&format!("could not be serialized for scanning: {e}")),
                removed_by_sanitizer: false,
            });
            return;
        }
    };

    let sink = TaintSink::net_fetch();
    walk_strings(&value, "", 0, &sink, out);
}

fn walk_strings(
    value: &serde_json::Value,
    path: &str,
    depth: usize,
    sink: &TaintSink,
    out: &mut Vec<Finding>,
) {
    if depth > MAX_SCAN_DEPTH {
        out.push(Finding {
            field: path.to_string(),
            category: PrivacyCategory::Unscannable,
            preview: format!("nesting deeper than {MAX_SCAN_DEPTH} levels was not scanned"),
            removed_by_sanitizer: false,
        });
        return;
    }

    match value {
        serde_json::Value::String(text) => {
            for category in scan_text(text, sink) {
                out.push(Finding {
                    field: path.to_string(),
                    category,
                    preview: preview_text(text),
                    removed_by_sanitizer: false,
                });
            }
        }
        serde_json::Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                walk_strings(item, &format!("{path}[{index}]"), depth + 1, sink, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                walk_strings(child, &child_path, depth + 1, sink, out);
            }
        }
        _ => {}
    }
}

/// Categories that fire against one string value, deduplicated and returned in a fixed order so the finding list is stable across runs.
fn scan_text(text: &str, sink: &TaintSink) -> Vec<PrivacyCategory> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut hits: HashSet<PrivacyCategory> = HashSet::new();

    // The taint denylist, whole value first.
    // `OpaqueToken` is skipped for values that break into short segments — see `OPAQUE_RUN_THRESHOLD`.
    for category in taint_categories(text, sink) {
        hits.insert(category);
    }

    // `WellKnownPrefix` and `OpaqueToken` anchor on the whole payload, so a key pasted mid-sentence into a system prompt only matches when the value is scanned token by token as well.
    // Only the credential rules are re-run per token: the PII rules already match anywhere in the text.
    if !hits.contains(&PrivacyCategory::SecretLiteral) {
        for token in text.split_whitespace().map(trim_token) {
            if token.is_empty() || token == text {
                continue;
            }
            if taint_categories(token, sink).contains(&PrivacyCategory::SecretLiteral) {
                hits.insert(PrivacyCategory::SecretLiteral);
                break;
            }
        }
    }

    // Shape rules this module owns.
    for token in text.split_whitespace().map(trim_token) {
        if token.is_empty() {
            continue;
        }
        if looks_like_absolute_host_path(token) {
            hits.insert(PrivacyCategory::HostPath);
        }
        if looks_like_private_endpoint(token) {
            hits.insert(PrivacyCategory::PrivateEndpoint);
        }
    }

    // Fixed emission order.
    [
        PrivacyCategory::SecretLiteral,
        PrivacyCategory::PersonalData,
        PrivacyCategory::HostPath,
        PrivacyCategory::PrivateEndpoint,
    ]
    .into_iter()
    .filter(|category| hits.contains(category))
    .collect()
}

fn taint_categories(text: &str, sink: &TaintSink) -> Vec<PrivacyCategory> {
    let mut skip = HashSet::new();
    if !has_long_unbroken_alnum_run(text, OPAQUE_RUN_THRESHOLD) {
        skip.insert(TaintRuleId::OpaqueToken);
    }
    taint::detect_outbound_text_violation_rules_with_skip(text, sink, &skip)
        .into_iter()
        .map(|rule| match rule {
            TaintRuleId::AuthorizationLiteral
            | TaintRuleId::KeyValueSecret
            | TaintRuleId::WellKnownPrefix
            | TaintRuleId::OpaqueToken
            | TaintRuleId::SensitiveKeyName => PrivacyCategory::SecretLiteral,
            TaintRuleId::PiiEmail
            | TaintRuleId::PiiPhone
            | TaintRuleId::PiiCreditCard
            | TaintRuleId::PiiSsn => PrivacyCategory::PersonalData,
        })
        .collect()
}

/// Whether `text` contains an unbroken run of at least `threshold` alphanumeric characters.
/// See [`OPAQUE_RUN_THRESHOLD`].
fn has_long_unbroken_alnum_run(text: &str, threshold: usize) -> bool {
    let mut run = 0usize;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            run += 1;
            if run >= threshold {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// Strip the punctuation a value picks up from surrounding prose, so `(sk-abc123…)` and `/Users/jane/notes.` are scanned as the value itself.
fn trim_token(token: &str) -> &str {
    token.trim_matches(|c: char| {
        matches!(
            c,
            '(' | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '"'
                | '\''
                | '`'
                | ','
                | ';'
                | '!'
                | '?'
                | '.'
        )
    })
}

/// Path segments that make an absolute POSIX path a *host filesystem* path rather than a URL route or a slash-separated identifier.
///
/// Restricting to known roots is a deliberate trade: `/api/v1/messages` inside a system prompt is not reported, and neither is a path under a non-standard root such as `/nvme0/agents/`.
/// A detector that flags every slash-bearing string trains the operator to skip the findings that matter, and the field where a real workspace path lives — `workspace` — is stripped outright and reported by the first pass regardless of its root.
const HOST_PATH_ROOTS: &[&str] = &[
    "applications",
    "bin",
    "boot",
    "data",
    "dev",
    "etc",
    "home",
    "library",
    "media",
    "mnt",
    "opt",
    "private",
    "proc",
    "root",
    "run",
    "sbin",
    "srv",
    "sys",
    "tmp",
    "usr",
    "users",
    "var",
    "volumes",
    "workspace",
    "workspaces",
];

/// Whether `token` looks like an absolute path into a host's filesystem.
fn looks_like_absolute_host_path(token: &str) -> bool {
    // POSIX: `/users/jane/...` — a known root plus at least one more segment.
    if let Some(rest) = token.strip_prefix('/') {
        let mut segments = rest.split('/').filter(|segment| !segment.is_empty());
        if let Some(root) = segments.next() {
            if segments.next().is_some()
                && HOST_PATH_ROOTS
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(root))
            {
                return true;
            }
        }
    }

    let bytes = token.as_bytes();
    // Windows drive path: `C:\Users\jane` or `C:/Users/jane`.
    if bytes.len() >= 4
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    // UNC share: `\\fileserver\vault`.
    if let Some(rest) = token.strip_prefix("\\\\") {
        if rest.contains('\\') {
            return true;
        }
    }

    false
}

/// Domain suffixes that only resolve inside someone's own network.
const PRIVATE_HOST_SUFFIXES: &[&str] = &[
    ".local",
    ".localhost",
    ".internal",
    ".intranet",
    ".lan",
    ".corp",
    ".home",
    ".private",
];

/// Whether `token` names a host that only exists on the operator's network.
///
/// Covers loopback and RFC 1918 / link-local literals, private domain suffixes, and — when the token carries a URL scheme — a single-label hostname such as `http://gitlab/`, which is only resolvable against the operator's own search domain.
fn looks_like_private_endpoint(token: &str) -> bool {
    let (host, had_scheme) = match split_host(token) {
        Some(parts) => parts,
        None => return false,
    };
    if host.is_empty() {
        return false;
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let lower = host.to_ascii_lowercase();

    if lower == "localhost" || lower == "::1" {
        return true;
    }
    if let Some(octets) = ipv4_octets(&lower) {
        return match octets {
            [127, ..] => true,
            [10, ..] => true,
            [169, 254, ..] => true,
            [192, 168, ..] => true,
            [172, second, ..] if (16..=31).contains(&second) => true,
            _ => false,
        };
    }
    if lower.contains('.')
        && PRIVATE_HOST_SUFFIXES
            .iter()
            .any(|suffix| lower.ends_with(suffix))
    {
        return true;
    }
    // A bare single-label host is only meaningful with an explicit scheme; without one, every ordinary word would match.
    had_scheme && !lower.contains('.')
}

/// Split the host out of a URL-ish token, reporting whether a scheme was present.
/// Returns `None` for tokens that cannot carry a host at all.
fn split_host(token: &str) -> Option<(&str, bool)> {
    let (rest, had_scheme) = match token.find("://") {
        Some(index) => (&token[index + 3..], true),
        None => (token, false),
    };
    // Trim path / query / fragment.
    let authority = rest
        .find(['/', '?', '#'])
        .map_or(rest, |index| &rest[..index]);
    // Trim userinfo.
    let authority = authority
        .rfind('@')
        .map_or(authority, |index| &authority[index + 1..]);
    if authority.is_empty() {
        return None;
    }
    // Trim the port, but leave a bracketed IPv6 literal alone.
    let host = if authority.starts_with('[') {
        authority
    } else {
        match authority.rfind(':') {
            Some(index) if authority[index + 1..].chars().all(|c| c.is_ascii_digit()) => {
                &authority[..index]
            }
            _ => authority,
        }
    };
    Some((host, had_scheme))
}

/// Parse `host` as a dotted-quad IPv4 literal.
/// Requires all four octets so that a version string like `10.0.0` is not read as an RFC 1918 address.
fn ipv4_octets(host: &str) -> Option<[u8; 4]> {
    let mut octets = [0u8; 4];
    let mut parts = host.split('.');
    for slot in &mut octets {
        let part = parts.next()?;
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(octets)
}

// ---------------------------------------------------------------------------
// Previews
// ---------------------------------------------------------------------------

fn preview_text(text: &str) -> String {
    truncate_preview(&single_line(text))
}

fn preview_json<T: Serialize>(value: &T) -> String {
    match serde_json::to_string(value) {
        Ok(rendered) => preview_text(&rendered),
        Err(_) => "<unserializable>".to_string(),
    }
}

fn preview_list(values: &[String]) -> String {
    preview_text(&values.join(", "))
}

fn preview_sorted_keys<'a>(keys: impl Iterator<Item = &'a String>) -> String {
    let mut names: Vec<&str> = keys.map(String::as_str).collect();
    names.sort_unstable();
    preview_text(&names.join(", "))
}

/// Collapse control characters and runs of whitespace so a preview occupies one line however the operator formatted the original.
fn single_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() || ch.is_control() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(ch);
    }
    out
}

/// Truncate on a character boundary at [`PREVIEW_MAX_CHARS`], marking the cut.
fn truncate_preview(text: &str) -> String {
    for (seen, (index, _)) in text.char_indices().enumerate() {
        if seen == PREVIEW_MAX_CHARS {
            return format!("{}…", &text[..index]);
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{ManifestTrigger, WorkspaceDecl, WorkspaceMode};
    use crate::config::{ContextInjection, ExecPolicy};
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    // -----------------------------------------------------------------------
    // Fixtures
    // -----------------------------------------------------------------------

    /// A manifest carrying one planted value per sensitive category, each value a distinctive sentinel so `assert!(!rendered.contains(..))` is unambiguous.
    fn leaky_manifest() -> AgentManifest {
        let mut manifest = AgentManifest {
            name: "researcher".to_string(),
            description: "Reads sources and writes briefs.".to_string(),
            module: "builtin:chat".to_string(),
            author: "jane.doe@acme-internal.example".to_string(),
            workspace: Some(PathBuf::from(
                "/Users/janedoe/.librefang/workspaces/researcher-a1b2",
            )),
            ..AgentManifest::default()
        };

        manifest.model.provider = "anthropic".to_string();
        manifest.model.model = "claude-sonnet-4-20250514".to_string();
        manifest.model.system_prompt = "You are a research assistant.".to_string();
        manifest.model.api_key_env = Some("ACME_PROD_ANTHROPIC_KEY".to_string());
        manifest.model.base_url = Some("https://llm-gateway.acme.internal/v1".to_string());
        manifest
            .model
            .extra_params
            .insert("acme_org".to_string(), serde_json::json!("org_SENTINEL_1"));

        manifest.fallback_models = Some(vec![FallbackModel {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            api_key_env: Some("ACME_FALLBACK_OPENAI_KEY".to_string()),
            base_url: Some("http://10.42.7.9:8000/v1".to_string()),
            extra_params: BTreeMap::from([(
                "deployment".to_string(),
                serde_json::json!("acme-sentinel-deployment"),
            )]),
        }]);

        manifest.capabilities.tools = vec!["web_fetch".to_string(), "file_read".to_string()];
        manifest.capabilities.network = vec!["vault.acme.internal:443".to_string()];
        manifest.capabilities.shell = vec!["/opt/acme/bin/deploy".to_string()];
        manifest.capabilities.ofp_connect = vec!["peer.acme.lan".to_string()];

        manifest.tools.insert(
            "http".to_string(),
            crate::agent::ToolConfig {
                params: HashMap::from([(
                    "bearer".to_string(),
                    serde_json::json!("SENTINEL_TOOL_PARAM"),
                )]),
            },
        );
        manifest.metadata.insert(
            "cost_centre".to_string(),
            serde_json::json!("SENTINEL_COST_CENTRE"),
        );

        manifest.workspaces.insert(
            "contracts".to_string(),
            WorkspaceDecl {
                path: None,
                mount: Some(PathBuf::from("/Volumes/acme-legal/contracts")),
                mode: WorkspaceMode::default(),
            },
        );

        manifest.exec_policy = Some(ExecPolicy {
            allowed_commands: vec!["/opt/acme/bin/deploy".to_string()],
            ..ExecPolicy::default()
        });

        manifest.context_injection = vec![ContextInjection {
            name: "runbook".to_string(),
            content: "Escalate to oncall@acme-internal.example via SENTINEL_RUNBOOK.".to_string(),
            position: Default::default(),
            condition: None,
        }];

        manifest.channel_overrides = Some(crate::config::ChannelOverrides {
            system_prompt: Some("Answer as SENTINEL_BOT_ALIAS.".to_string()),
            ..Default::default()
        });

        manifest.tool_exec_backend = Some(crate::tool_exec::BackendKind::Ssh);

        manifest.context_engine = Some(crate::config::ContextEngineTomlConfig {
            plugin: Some("acme-sentinel-recall".to_string()),
            ..Default::default()
        });

        manifest.triggers = vec![ManifestTrigger {
            pattern: serde_json::json!("task_posted"),
            prompt_template: "Handle SENTINEL_TRIGGER: {{event}}".to_string(),
            target_agent: Some("acme-sentinel-worker".to_string()),
            ..ManifestTrigger::default()
        }];

        manifest.is_hand = true;
        manifest.enabled = false;

        manifest
    }

    /// A manifest with every top-level `skip_serializing_if` field populated, so serializing it emits all 58 `AgentManifest` keys.
    /// Used only by the classification-coverage test.
    fn fully_populated_manifest() -> AgentManifest {
        let mut manifest = leaky_manifest();
        manifest.compaction = Some(crate::agent::CompactionOverrides::default());
        assert!(
            manifest.fallback_models.is_some()
                && manifest.tool_exec_backend.is_some()
                && manifest.context_engine.is_some()
                && !manifest.triggers.is_empty(),
            "fixture must populate every field carrying skip_serializing_if"
        );
        manifest
    }

    fn rendered(manifest: &AgentManifest) -> String {
        serde_json::to_string(manifest).expect("manifest serializes to JSON")
    }

    fn fields_with(findings: &[Finding], category: PrivacyCategory) -> Vec<&str> {
        findings
            .iter()
            .filter(|finding| finding.category == category)
            .map(|finding| finding.field.as_str())
            .collect()
    }

    // -----------------------------------------------------------------------
    // The classification cannot rot
    // -----------------------------------------------------------------------

    /// The compile-time half of this guarantee is in `sanitize_for_publication`: it destructures `AgentManifest` with no `..` and rebuilds it with no `..Default::default()`, so a new field is an `E0027`/`E0063` compile error there until it is classified.
    /// This test is the runtime half — it keeps the human-readable `CLASSIFICATION` table from falling behind the struct, in either direction.
    #[test]
    fn classification_covers_every_serialized_manifest_field() {
        let value =
            serde_json::to_value(fully_populated_manifest()).expect("manifest serializes to JSON");
        let serialized: BTreeSet<String> = value
            .as_object()
            .expect("manifest serializes to an object")
            .keys()
            .cloned()
            .collect();
        let classified: BTreeSet<String> = CLASSIFICATION
            .iter()
            .map(|(field, _)| (*field).to_string())
            .collect();

        assert_eq!(
            serialized, classified,
            "CLASSIFICATION and AgentManifest disagree. A field only in the manifest needs a \
             CLASSIFICATION entry (and a decision in sanitize_for_publication). A field only in \
             CLASSIFICATION was renamed or removed, or fully_populated_manifest() no longer sets \
             it and its skip_serializing_if hides it."
        );
    }

    #[test]
    fn classification_has_no_duplicate_entries() {
        let unique: BTreeSet<&str> = CLASSIFICATION.iter().map(|(field, _)| *field).collect();
        assert_eq!(
            unique.len(),
            CLASSIFICATION.len(),
            "duplicate field in CLASSIFICATION"
        );
    }

    // -----------------------------------------------------------------------
    // One test per category of sensitive field
    // -----------------------------------------------------------------------

    #[test]
    fn absolute_workspace_path_does_not_survive_promotion() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        assert_eq!(publishable.workspace, None);
        assert!(!rendered(&publishable).contains("/Users/janedoe"));
    }

    #[test]
    fn named_workspace_mounts_do_not_survive_promotion() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        assert!(publishable.workspaces.is_empty());
        let json = rendered(&publishable);
        assert!(!json.contains("/Volumes/acme-legal"));
        assert!(!json.contains("contracts"));
    }

    #[test]
    fn credential_env_bindings_do_not_survive_promotion() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        assert_eq!(publishable.model.api_key_env, None);
        let chain = publishable
            .fallback_models
            .as_ref()
            .expect("chain is preserved");
        assert_eq!(chain[0].api_key_env, None);
        let json = rendered(&publishable);
        assert!(!json.contains("ACME_PROD_ANTHROPIC_KEY"));
        assert!(!json.contains("ACME_FALLBACK_OPENAI_KEY"));
    }

    #[test]
    fn provider_base_urls_do_not_survive_promotion() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        assert_eq!(publishable.model.base_url, None);
        let chain = publishable
            .fallback_models
            .as_ref()
            .expect("chain is preserved");
        assert_eq!(chain[0].base_url, None);
        let json = rendered(&publishable);
        assert!(!json.contains("llm-gateway.acme.internal"));
        assert!(!json.contains("10.42.7.9"));
    }

    #[test]
    fn provider_extra_params_do_not_survive_promotion() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        assert!(publishable.model.extra_params.is_empty());
        let chain = publishable
            .fallback_models
            .as_ref()
            .expect("chain is preserved");
        assert!(chain[0].extra_params.is_empty());
        let json = rendered(&publishable);
        assert!(!json.contains("org_SENTINEL_1"));
        assert!(!json.contains("acme-sentinel-deployment"));
    }

    #[test]
    fn exec_policy_does_not_survive_promotion() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        assert!(publishable.exec_policy.is_none());
        assert!(!rendered(&publishable).contains("/opt/acme/bin/deploy"));
    }

    #[test]
    fn host_target_capability_allowlists_do_not_survive_promotion() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        assert!(publishable.capabilities.network.is_empty());
        assert!(publishable.capabilities.shell.is_empty());
        assert!(publishable.capabilities.ofp_connect.is_empty());
        let json = rendered(&publishable);
        assert!(!json.contains("vault.acme.internal"));
        assert!(!json.contains("peer.acme.lan"));
    }

    #[test]
    fn context_injections_do_not_survive_promotion() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        assert!(publishable.context_injection.is_empty());
        let json = rendered(&publishable);
        assert!(!json.contains("SENTINEL_RUNBOOK"));
        assert!(!json.contains("oncall@acme-internal.example"));
    }

    #[test]
    fn operator_metadata_and_author_do_not_survive_promotion() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        assert!(publishable.metadata.is_empty());
        assert!(publishable.author.is_empty());
        let json = rendered(&publishable);
        assert!(!json.contains("SENTINEL_COST_CENTRE"));
        assert!(!json.contains("jane.doe@acme-internal.example"));
    }

    #[test]
    fn per_tool_params_do_not_survive_promotion() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        assert!(publishable.tools.is_empty());
        assert!(!rendered(&publishable).contains("SENTINEL_TOOL_PARAM"));
    }

    #[test]
    fn channel_overrides_do_not_survive_promotion() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        assert!(publishable.channel_overrides.is_none());
        assert!(!rendered(&publishable).contains("SENTINEL_BOT_ALIAS"));
    }

    #[test]
    fn host_execution_and_context_engine_wiring_do_not_survive_promotion() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        assert!(publishable.tool_exec_backend.is_none());
        assert!(publishable.context_engine.is_none());
        assert!(!rendered(&publishable).contains("acme-sentinel-recall"));
    }

    #[test]
    fn local_trigger_wiring_does_not_survive_promotion() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        assert!(publishable.triggers.is_empty());
        let json = rendered(&publishable);
        assert!(!json.contains("acme-sentinel-worker"));
        assert!(!json.contains("SENTINEL_TRIGGER"));
    }

    #[test]
    fn local_state_flags_are_reset_to_type_defaults() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        assert!(!publishable.is_hand, "provenance flag must not travel");
        assert!(
            publishable.enabled,
            "a type published from a disabled agent must install enabled"
        );
    }

    #[test]
    fn no_planted_sentinel_survives_promotion() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        let json = rendered(&publishable);
        for sentinel in [
            "SENTINEL_TOOL_PARAM",
            "SENTINEL_COST_CENTRE",
            "SENTINEL_RUNBOOK",
            "SENTINEL_BOT_ALIAS",
            "SENTINEL_TRIGGER",
            "org_SENTINEL_1",
            "acme-sentinel-deployment",
            "acme-sentinel-recall",
            "acme-sentinel-worker",
            "/Users/janedoe",
            "/Volumes/acme-legal",
            "/opt/acme/bin/deploy",
            "ACME_PROD_ANTHROPIC_KEY",
            "ACME_FALLBACK_OPENAI_KEY",
            "llm-gateway.acme.internal",
            "vault.acme.internal",
            "peer.acme.lan",
            "10.42.7.9",
            "jane.doe@acme-internal.example",
        ] {
            assert!(
                !json.contains(sentinel),
                "'{sentinel}' survived promotion: {json}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // The portable half survives
    // -----------------------------------------------------------------------

    #[test]
    fn portable_fields_survive_promotion() {
        let original = leaky_manifest();
        let publishable = sanitize_for_publication(&original);

        assert_eq!(publishable.name, "researcher");
        assert_eq!(
            publishable.description, "Reads sources and writes briefs.",
            "the description is the whole point of a registry entry"
        );
        assert_eq!(publishable.module, "builtin:chat");
        assert_eq!(publishable.model.provider, "anthropic");
        assert_eq!(publishable.model.model, "claude-sonnet-4-20250514");
        assert_eq!(
            publishable.model.system_prompt,
            "You are a research assistant."
        );
        assert_eq!(
            publishable.capabilities.tools,
            vec!["web_fetch".to_string(), "file_read".to_string()]
        );
        let chain = publishable
            .fallback_models
            .as_ref()
            .expect("the fallback chain itself is portable");
        assert_eq!(chain[0].provider, "openai");
        assert_eq!(chain[0].model, "gpt-4o");
    }

    #[test]
    fn portable_collections_survive_promotion() {
        let mut manifest = leaky_manifest();
        manifest.skills = vec!["summarise".to_string()];
        manifest.mcp_servers = vec!["filesystem".to_string()];
        manifest.channels = vec!["telegram".to_string()];
        manifest.tags = vec!["research".to_string()];
        manifest.tool_allowlist = vec!["web_fetch".to_string()];
        manifest.allowed_plugins = vec!["qdrant-recall".to_string()];
        manifest.max_history_messages = Some(40);
        manifest.routing = Some(crate::agent::ModelRoutingConfig::default());

        let publishable = sanitize_for_publication(&manifest);
        assert_eq!(publishable.skills, vec!["summarise".to_string()]);
        assert_eq!(publishable.mcp_servers, vec!["filesystem".to_string()]);
        assert_eq!(publishable.channels, vec!["telegram".to_string()]);
        assert_eq!(publishable.tags, vec!["research".to_string()]);
        assert_eq!(publishable.tool_allowlist, vec!["web_fetch".to_string()]);
        assert_eq!(
            publishable.allowed_plugins,
            vec!["qdrant-recall".to_string()]
        );
        assert_eq!(publishable.max_history_messages, Some(40));
        assert!(publishable.routing.is_some());
    }

    #[test]
    fn sanitizing_does_not_mutate_the_caller_s_manifest() {
        let original = leaky_manifest();
        let before = rendered(&original);
        let _ = sanitize_for_publication(&original);
        assert_eq!(
            before,
            rendered(&original),
            "the sanitiser must never rewrite the operator's own manifest"
        );
    }

    #[test]
    fn publishable_manifest_round_trips_through_toml() {
        let publishable = sanitize_for_publication(&leaky_manifest());
        let text = toml::to_string_pretty(&publishable).expect("publishable manifest renders");
        let reparsed: AgentManifest = toml::from_str(&text).expect("publishable manifest reparses");
        assert_eq!(reparsed.name, publishable.name);
        assert_eq!(reparsed.description, publishable.description);
        assert_eq!(reparsed.module, publishable.module);
        assert_eq!(reparsed.workspace, None);
        assert!(reparsed.metadata.is_empty());
    }

    // -----------------------------------------------------------------------
    // Detector
    // -----------------------------------------------------------------------

    #[test]
    fn detector_reports_every_stripped_field_it_finds() {
        let findings = scan_for_publication(&leaky_manifest());
        let reported: BTreeSet<&str> = findings
            .iter()
            .filter(|finding| finding.removed_by_sanitizer)
            .map(|finding| finding.field.as_str())
            .collect();

        for expected in [
            "author",
            "model.api_key_env",
            "model.base_url",
            "model.extra_params",
            "fallback_models[0].api_key_env",
            "fallback_models[0].base_url",
            "fallback_models[0].extra_params",
            "capabilities.network",
            "capabilities.shell",
            "capabilities.ofp_connect",
            "tools",
            "metadata",
            "workspace",
            "workspaces",
            "exec_policy",
            "enabled",
            "context_injection",
            "is_hand",
            "channel_overrides",
            "tool_exec_backend",
            "context_engine",
            "triggers",
        ] {
            assert!(
                reported.contains(expected),
                "detector missed '{expected}': {reported:?}"
            );
        }
    }

    #[test]
    fn detector_stays_quiet_on_an_already_portable_manifest() {
        let manifest = AgentManifest {
            name: "summariser".to_string(),
            description: "Turns long documents into short ones.".to_string(),
            module: "builtin:chat".to_string(),
            tags: vec!["writing".to_string()],
            ..AgentManifest::default()
        };
        let findings = scan_for_publication(&manifest);
        assert!(findings.is_empty(), "unexpected findings: {findings:#?}");
    }

    #[test]
    fn detector_flags_a_credential_pasted_into_a_retained_system_prompt() {
        let mut manifest = AgentManifest {
            name: "leaker".to_string(),
            description: "d".to_string(),
            ..AgentManifest::default()
        };
        manifest.model.system_prompt =
            "Authenticate with sk-ant-api03-QQQQWWWWEEEERRRRTTTTYYYY before calling.".to_string();

        let findings = scan_for_publication(&manifest);
        let flagged = fields_with(&findings, PrivacyCategory::SecretLiteral);
        assert!(
            flagged.contains(&"model.system_prompt"),
            "a key pasted mid-sentence must be flagged in a field the sanitiser keeps: \
             {findings:#?}"
        );
        assert!(
            findings
                .iter()
                .filter(|finding| finding.field == "model.system_prompt")
                .all(|finding| !finding.removed_by_sanitizer),
            "a finding in a retained field must tell the operator to edit it by hand"
        );
    }

    #[test]
    fn detector_flags_a_host_path_in_a_retained_description() {
        let manifest = AgentManifest {
            name: "pathy".to_string(),
            description: "Indexes the notes under /Users/janedoe/vault every morning.".to_string(),
            ..AgentManifest::default()
        };
        let findings = scan_for_publication(&manifest);
        assert!(
            fields_with(&findings, PrivacyCategory::HostPath).contains(&"description"),
            "{findings:#?}"
        );
    }

    #[test]
    fn detector_flags_a_private_endpoint_in_a_retained_tag() {
        let manifest = AgentManifest {
            name: "endpointy".to_string(),
            description: "d".to_string(),
            tags: vec!["https://wiki.acme.internal/agents".to_string()],
            ..AgentManifest::default()
        };
        let findings = scan_for_publication(&manifest);
        assert!(
            fields_with(&findings, PrivacyCategory::PrivateEndpoint).contains(&"tags[0]"),
            "{findings:#?}"
        );
    }

    #[test]
    fn detector_flags_personal_data_in_a_retained_prompt() {
        let mut manifest = AgentManifest {
            name: "pii".to_string(),
            description: "d".to_string(),
            ..AgentManifest::default()
        };
        manifest.model.system_prompt =
            "Escalate to priya.rao@acme.example when unsure.".to_string();
        let findings = scan_for_publication(&manifest);
        assert!(
            fields_with(&findings, PrivacyCategory::PersonalData).contains(&"model.system_prompt"),
            "{findings:#?}"
        );
    }

    #[test]
    fn detector_does_not_mistake_a_model_id_for_a_credential() {
        let mut manifest = AgentManifest {
            name: "modelly".to_string(),
            description: "d".to_string(),
            ..AgentManifest::default()
        };
        // Long, hyphen- and slash-segmented identifiers are exactly what the taint scanner's opaque-token rule would otherwise flag.
        manifest.model.model = "accounts/fireworks/models/llama-v3p1-405b-instruct".to_string();
        manifest.pinned_model = Some("claude-sonnet-4-5-20250929".to_string());

        let findings = scan_for_publication(&manifest);
        assert!(
            findings.is_empty(),
            "model identifiers must not be reported as credentials: {findings:#?}"
        );
    }

    #[test]
    fn detector_output_is_stable_across_runs() {
        let manifest = leaky_manifest();
        let first = scan_for_publication(&manifest);
        let second = scan_for_publication(&manifest);
        assert_eq!(first, second, "finding order must be deterministic");
    }

    #[test]
    fn previews_are_bounded_and_single_line() {
        let mut manifest = leaky_manifest();
        manifest.author = format!("line one\nline two {}", "x".repeat(500));
        for finding in scan_for_publication(&manifest) {
            assert!(
                finding.preview.chars().count() <= PREVIEW_MAX_CHARS + 1,
                "preview exceeds the cap: {finding:?}"
            );
            assert!(
                !finding.preview.contains('\n'),
                "preview must be one line: {finding:?}"
            );
        }
    }

    #[test]
    fn nesting_past_the_scan_depth_is_reported_rather_than_passed_as_clean() {
        // `response_format`'s JSON schema is operator-supplied and arbitrarily deep.
        // The walk stops at `MAX_SCAN_DEPTH` and must say that it did.
        let mut schema = serde_json::json!({"type": "string"});
        for _ in 0..MAX_SCAN_DEPTH + 8 {
            schema = serde_json::json!({ "properties": schema });
        }
        let manifest = AgentManifest {
            name: "deep".to_string(),
            description: "d".to_string(),
            response_format: Some(crate::config::ResponseFormat::JsonSchema {
                name: "deep".to_string(),
                schema,
                strict: None,
            }),
            ..AgentManifest::default()
        };

        let findings = scan_for_publication(&manifest);
        assert!(
            findings
                .iter()
                .any(|finding| finding.category == PrivacyCategory::Unscannable),
            "an unscanned subtree must be reported: {findings:#?}"
        );
    }

    // -----------------------------------------------------------------------
    // Shape rules
    // -----------------------------------------------------------------------

    #[test]
    fn host_path_shape_rule_matches_real_paths_only() {
        for path in [
            "/Users/janedoe/vault",
            "/home/ci/agents/worker",
            "/opt/acme/bin/deploy",
            "/Volumes/acme-legal/contracts",
            "C:\\Users\\jane\\vault",
            "D:/data/agents",
            "\\\\fileserver\\vault",
        ] {
            assert!(
                looks_like_absolute_host_path(path),
                "'{path}' should read as a host path"
            );
        }
        for other in [
            "/help",
            "/",
            "relative/path/file.txt",
            "https://example.com/a/b",
            "and/or",
        ] {
            assert!(
                !looks_like_absolute_host_path(other),
                "'{other}' should not read as a host path"
            );
        }
    }

    #[test]
    fn private_endpoint_shape_rule_matches_internal_hosts_only() {
        for token in [
            "http://localhost:4545/api",
            "https://llm-gateway.acme.internal/v1",
            "http://10.42.7.9:8000/v1",
            "https://192.168.1.10",
            "http://172.20.0.5/health",
            "http://127.0.0.1:8080",
            "https://peer.acme.lan",
            "http://gitlab/projects",
            "https://user:pw@build.corp/status",
        ] {
            assert!(
                looks_like_private_endpoint(token),
                "'{token}' should read as a private endpoint"
            );
        }
        for token in [
            "https://api.anthropic.com/v1",
            "https://example.com",
            "10.0.0",
            "1.2.3",
            "0.1.0",
            "gpt-4o",
            "summarise",
            "https://172.15.0.1/",
            "https://8.8.8.8/",
        ] {
            assert!(
                !looks_like_private_endpoint(token),
                "'{token}' should not read as a private endpoint"
            );
        }
    }

    #[test]
    fn long_unbroken_run_gate_separates_tokens_from_identifiers() {
        assert!(has_long_unbroken_alnum_run(
            "QQQQWWWWEEEERRRRTTTTYYYY",
            OPAQUE_RUN_THRESHOLD
        ));
        assert!(!has_long_unbroken_alnum_run(
            "accounts/fireworks/models/llama-v3p1-405b-instruct",
            OPAQUE_RUN_THRESHOLD
        ));
        assert!(!has_long_unbroken_alnum_run(
            "claude-sonnet-4-20250514",
            OPAQUE_RUN_THRESHOLD
        ));
    }
}
