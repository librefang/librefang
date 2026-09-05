//! Inter-agent tools: `agent_find`, `agent_send`, `agent_spawn`,
//! `agent_list`, `agent_kill`.

use super::error::{ToolError, ToolResult};
use super::{
    caller_agent_id_missing, check_taint_outbound_text, current_agent_depth, require_kernel_typed,
    with_agent_call_depth,
};
use crate::kernel_handle::prelude::*;
use librefang_types::taint::TaintSink;
use std::fmt::Write;
use std::sync::Arc;
use tracing::warn;

pub(super) fn tool_agent_find(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let query = input["query"]
        .as_str()
        .ok_or(ToolError::MissingParameter("query"))?;
    let agents = kh.find_agents(query);
    if agents.is_empty() {
        return Ok(format!("No agents found matching '{query}'."));
    }
    let result: Vec<serde_json::Value> = agents
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "name": a.name,
                "state": a.state,
                "description": a.description,
                "tags": a.tags,
                "tools": a.tools,
                "model": format!("{}:{}", a.model_provider, a.model_name),
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&result)?)
}

pub(super) async fn tool_agent_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    caller_session_id: Option<librefang_types::agent::SessionId>,
    chat_id: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("agent_id"))?;
    let message = input["message"]
        .as_str()
        .ok_or(ToolError::MissingParameter("message"))?;
    let conversation_key = input["conversation_key"].as_str();
    // Non-blocking mode (#6043): when true, register the delegation on the
    // kernel async-task tracker and return a task id immediately instead of
    // blocking this agent's loop until the callee replies (which otherwise
    // trips `tool_timeout_secs` for any long delegation).
    //
    // Non-blocking is the DEFAULT when a caller agent is known, because the blocking default made every unannotated delegation a timeout risk: the model had to predict that the callee would be slow and opt in to async in advance, and a wrong guess burned the turn on `tool_timeout_secs`.
    //
    // The default is deliberately conditional on `caller_agent_id` rather than an unconditional `true`.
    // The async path below requires a known caller so the tracker can route the completion back, and rejects the call outright without one; the blocking path has explicit `(None, _)` arms for system-initiated sends.
    // An unconditional default would therefore turn every callerless send — the kernel's own system-initiated dispatches — into an `InvalidParameter` error.
    // An explicit `"async": true` still errors without a caller, which is the pre-existing contract and is left untouched.
    let async_mode = input["async"]
        .as_bool()
        .unwrap_or_else(|| caller_agent_id.is_some());

    if let Some(caller) = caller_agent_id {
        if caller == agent_id {
            return Err(ToolError::InvalidParameter {
                name: "agent_id",
                reason: "agent_send: an agent cannot send a message to itself".to_string(),
            });
        }
    }

    let sink = TaintSink::agent_message();
    // agent_id is a UUID/name identifier, not free-form content — skip taint
    // check here. Taint validation remains on message, conversation_key, etc.
    if let Some(violation) = check_taint_outbound_text(message, &sink) {
        return Err(ToolError::PermissionDenied(format!(
            "Taint violation (message): {violation}"
        )));
    }
    if let Some(key) = conversation_key {
        if let Some(violation) = check_taint_outbound_text(key, &sink) {
            return Err(ToolError::PermissionDenied(format!(
                "Taint violation (conversation_key): {violation}"
            )));
        }
    }

    // Check + increment inter-agent call depth. Surfaced as
    // `PermissionDenied` (→ `LibreFangError::CapabilityDenied` → HTTP 403),
    // not `Upstream` (→ 5xx): this is a kernel-policy quota, not a downstream
    // crash. Lifting to 5xx would mislead caller retry logic into treating a
    // self-imposed limit as a transient infra failure.
    let max_depth = kh.max_agent_call_depth();
    let current_depth = current_agent_depth();
    if current_depth >= max_depth {
        return Err(ToolError::PermissionDenied(format!(
            "Inter-agent call depth exceeded (max {max_depth}). \
             A->B->C chain is too deep. Use the task queue instead."
        )));
    }

    // Non-blocking path (#6043). Register the delegation on the kernel's
    // async-task tracker and return a task id immediately; the callee's reply
    // is injected back into this session when it finishes (mid-turn or
    // wake-idle). The depth guard above still applies — a too-deep chain is
    // rejected before it can fire asynchronously — but the synchronous
    // `AGENT_CALL_DEPTH` scope is intentionally NOT carried across the async
    // boundary: the callee runs in a detached task and starts its own depth
    // chain, so async delegation breaks the synchronous A->B->C accumulation
    // by design. Requires a known caller so the tracker can route completion.
    if async_mode {
        let caller = caller_agent_id.ok_or(ToolError::InvalidParameter {
            name: "async",
            reason: "async agent_send requires a known caller agent context".to_string(),
        })?;
        let session_str = caller_session_id.map(|s| s.0.to_string());
        let outcome = kh
            .send_to_agent_async_tracked(
                agent_id,
                message,
                caller,
                session_str.as_deref(),
                conversation_key,
                chat_id,
            )
            .await
            .map_err(ToolError::upstream)?;
        // Branch on what the kernel actually did (#6650). The tracked and fallback outcomes used to arrive as an indistinguishable `String`, so the fallback's *response body* was rendered as `task_id` under a note telling the model to stop and wait for a completion event that would never fire — the answer it already held.
        // Surfaces that pass no session (the MCP HTTP bridge, the REST `/api/tools/{name}` bridge) hit that path on every `async: true` call.
        return Ok(match outcome {
            AsyncSendOutcome::Tracked(task_id) => serde_json::json!({
                "task_id": task_id,
                "status": "delegated",
                "note": "Delegation started asynchronously; the target's reply will be \
                         delivered to this session when it completes. Do not wait — \
                         continue or end your turn.",
            })
            .to_string(),
            // No session to deliver a completion event to, so the kernel ran the delegation inline.
            // Hand back the reply exactly as the blocking path below does — same shape, same expectations.
            AsyncSendOutcome::Inline(response) => response,
        });
    }

    // `with_agent_call_depth` boxes the callee turn's future before entering the scope.
    // The nested turn is an enormous state machine, and inlining it here stacked another full copy of it per A->B->C level — the same stack hazard `held_agent_locks::scope` boxes to avoid (refs #6659).
    // It is also the single definition of "one level deeper", shared with the kernel's workflow-step dispatch so both charge the same quota.
    with_agent_call_depth(async {
        // When we know the caller, use the cascade-aware entry so a
        // parent `/stop` propagates into the callee (issue #3044).
        // System-initiated calls (caller_agent_id = None) fall back to
        // the legacy path.
        match (caller_agent_id, conversation_key) {
            (Some(parent), Some(key)) => {
                kh.send_to_agent_as_with_key(agent_id, message, parent, key)
                    .await
            }
            (Some(parent), None) => kh.send_to_agent_as(agent_id, message, parent).await,
            (None, Some(key)) => kh.send_to_agent_with_key(agent_id, message, key).await,
            (None, None) => kh.send_to_agent(agent_id, message).await,
        }
    })
    .await
    .map_err(ToolError::upstream)
}

/// Build agent manifest TOML from parsed parameters.
///
/// `profile` is the already-resolved model-router profile the spawned agent
/// should run on, or `None` to leave the model unset so the child inherits the
/// kernel default exactly as it did before profiles existed.
///
/// A profile pins `provider` and `model` outright rather than setting
/// `mode = "flexible"`. Naming a profile at spawn time is a request for *that*
/// model — a verifier asked to run on `quick` should stay on `quick`, not be
/// handed to the per-turn router which could move it back onto an expensive
/// model on the next turn. It also means the parameter works with
/// `[model_router] enabled = false`, which is the common case for an operator
/// who wants cheap subagents without automatic routing everywhere.
///
/// `parent_override` is the spawning agent's own `[model.router_override]`,
/// copied verbatim into the child's manifest when it has one (#7789 review).
/// Checking the requested profile against the parent only constrains the first
/// hop: a parent budgeted at `cheap` could spawn a permitted child, hand it
/// `agent_spawn`, and have *that* child — born with no override, which every
/// caller reads as unconstrained — spawn on the most expensive profile in the
/// catalog, billed to the same operator. Propagating the override means a
/// child is capped at most as loosely as its parent however many hops down the
/// chain it sits. It also makes `fixed = true` bind the subtree rather than
/// only refusing the parent a profile of its own.
pub(super) fn build_agent_manifest_toml(
    name: &str,
    system_prompt: &str,
    tools: Vec<String>,
    shell: Vec<String>,
    network: bool,
    profile: Option<&librefang_types::model_profile::ModelProfile>,
    parent_override: Option<&librefang_types::model_profile::AgentRouterOverride>,
) -> Result<String, String> {
    let mut tools = tools;
    let has_shell = !shell.is_empty();

    // Auto-add shell_exec to tools if shell is specified (without duplicates)
    if has_shell && !tools.iter().any(|t| t == "shell_exec") {
        tools.push("shell_exec".to_string());
    }

    let mut capabilities = serde_json::json!({
        "tools": tools,
    });
    if network {
        capabilities["network"] = serde_json::json!(["*"]);
    }
    if has_shell {
        capabilities["shell"] = serde_json::json!(shell);
    }

    let mut model_json = serde_json::json!({
        "system_prompt": system_prompt,
    });
    if let Some(profile) = profile {
        model_json["provider"] = serde_json::json!(profile.provider);
        model_json["model"] = serde_json::json!(profile.model);
        // Only carried when the profile actually states one, so a profile
        // without an explicit window keeps the runtime's own resolution
        // (registry, persisted cache, live `/v1/models` probe) rather than
        // pinning a value the profile author never chose.
        if let Some(context_window) = profile.context_window {
            model_json["context_window"] = serde_json::json!(context_window);
        }
    }
    if let Some(parent_override) = parent_override {
        model_json["router_override"] = serde_json::to_value(parent_override)
            .map_err(|e| format!("Failed to serialize parent router_override: {e}"))?;
    }

    let manifest_json = serde_json::json!({
        "name": name,
        "model": model_json,
        "capabilities": capabilities,
    });

    toml::to_string(&manifest_json).map_err(|e| format!("Failed to serialize to TOML: {}", e))
}

/// Error for an `agent_spawn` naming a profile that is not in the catalog.
///
/// Lists what *is* available, because the caller is usually an LLM: an error
/// that only says "unknown" invites a second guess, while one that enumerates
/// the catalog lets the next attempt succeed. `available` is already ordered
/// by the catalog (#3298), so the message is stable across retries.
pub(super) fn unknown_profile_error(name: &str, available: &[String]) -> ToolError {
    if available.is_empty() {
        return ToolError::upstream_msg(format!(
            "Unknown model profile '{name}'. No model profiles are configured — \
             add them to ~/.librefang/model_profiles.toml."
        ));
    }
    ToolError::upstream_msg(format!(
        "Unknown model profile '{name}'. Available profiles: {}.",
        available.join(", ")
    ))
}

/// Refusal for an `agent_spawn { profile }` the spawning agent's own
/// `[model.router_override]` does not permit (#7789 review).
///
/// Same shape as [`unknown_profile_error`]: name what was asked for, then
/// list what *is* permitted, so the caller — usually an LLM — can retry
/// correctly instead of guessing. `permitted` is ordered by the catalog
/// (#3298), so the message is stable across retries.
pub(super) fn check_profile_against_parent(
    profile: &librefang_types::model_profile::ModelProfile,
    override_: &librefang_types::model_profile::AgentRouterOverride,
    permitted: &[String],
) -> Result<(), ToolError> {
    if override_.fixed {
        return Err(ToolError::upstream_msg(format!(
            "Model profile '{name}' cannot be used here: the spawning agent is pinned with \
             `[model.router_override] fixed = true`, which opts out of every profile — for its \
             own turns and for the agents it spawns. Spawn without a profile, or ask the \
             operator to relax the pin.",
            name = profile.name,
        )));
    }
    if !override_.permits(profile) {
        return Err(ToolError::upstream_msg(format!(
            "Model profile '{name}' is not permitted for this agent by its \
             `[model.router_override]`. Permitted profiles: {permitted}.",
            name = profile.name,
            permitted = permitted.join(", "),
        )));
    }
    Ok(())
}

/// The parent's permitted profile names, ordered by the catalog (#3298).
///
/// Filters the full catalog through the parent's override with the same
/// `AgentRouterOverride::permits` predicate the per-turn router uses, so a
/// refusal enumerates exactly what a correct retry can name.
fn permitted_profile_names(
    kh: &Arc<dyn KernelHandle>,
    override_: &librefang_types::model_profile::AgentRouterOverride,
) -> Vec<String> {
    kh.model_profile_names()
        .into_iter()
        .filter(|name| {
            kh.resolve_model_profile(name)
                .is_some_and(|p| override_.permits(&p))
        })
        .collect()
}

/// Expand a list of tool names into full `Capability` grants for the parent.
///
/// Tool names at the `execute_tool` level (e.g. `"file_read"`, `"shell_exec"`)
/// are `ToolInvoke` capabilities. But a child manifest may also request
/// resource-level capabilities (`NetConnect`, `ShellExec`, `AgentSpawn`, etc.)
/// that are *implied* by tool names. Without expanding, `validate_capability_inheritance`
/// would reject legitimate child capabilities because `ToolInvoke("web_fetch")`
/// cannot cover a child's `NetConnect("*")` — they are different enum variants.
///
/// This mirrors the `ToolProfile::implied_capabilities()` logic in agent.rs.
pub(super) fn tools_to_parent_capabilities(
    tools: &[String],
) -> Vec<librefang_types::capability::Capability> {
    use librefang_types::capability::Capability;

    let mut caps: Vec<Capability> = tools
        .iter()
        .map(|t| Capability::ToolInvoke(t.clone()))
        .collect();

    let has_net = tools.iter().any(|t| t.starts_with("web_") || t == "*");
    let has_shell = tools.iter().any(|t| t == "shell_exec" || t == "*");
    let has_agent_spawn = tools.iter().any(|t| t == "agent_spawn" || t == "*");
    let has_agent_msg = tools.iter().any(|t| t.starts_with("agent_") || t == "*");
    let has_memory = tools.iter().any(|t| t.starts_with("memory_") || t == "*");

    if has_net {
        caps.push(Capability::NetConnect("*".into()));
    }
    if has_shell {
        caps.push(Capability::ShellExec("*".into()));
    }
    if has_agent_spawn {
        caps.push(Capability::AgentSpawn);
    }
    if has_agent_msg {
        caps.push(Capability::AgentMessage("*".into()));
    }
    if has_memory {
        caps.push(Capability::MemoryRead("*".into()));
        caps.push(Capability::MemoryWrite("*".into()));
    }

    caps
}

pub(super) async fn tool_agent_spawn(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    parent_id: Option<&str>,
    parent_allowed_tools: Option<&[String]>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;

    let name = input["name"]
        .as_str()
        .ok_or(ToolError::MissingParameter("name"))?;
    let system_prompt = input["system_prompt"].as_str();

    let spawn_sink = TaintSink::agent_message();
    if let Some(violation) = check_taint_outbound_text(name, &spawn_sink) {
        return Err(ToolError::PermissionDenied(format!(
            "Taint violation (name): {violation}"
        )));
    }
    if let Some(prompt) = system_prompt {
        if let Some(violation) = check_taint_outbound_text(prompt, &spawn_sink) {
            return Err(ToolError::PermissionDenied(format!(
                "Taint violation (system_prompt): {violation}"
            )));
        }
    }

    if input["ephemeral"].as_bool().unwrap_or(false) {
        return tool_agent_spawn_ephemeral(
            input,
            kh,
            name,
            system_prompt,
            parent_id,
            parent_allowed_tools,
        )
        .await;
    }

    // Beyond this point the call builds a permanent agent, for which a system
    // prompt is the manifest's only description of what the agent is. The
    // schema no longer marks it required — an ephemeral worker spawned from an
    // agent type gets its prompt from the template — so the permanent path
    // reasserts it here, with the same error the schema used to produce.
    let system_prompt = system_prompt.ok_or(ToolError::MissingParameter("system_prompt"))?;

    let tools: Vec<String> = input["tools"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .enumerate()
                .filter_map(|(i, v)| match v.as_str() {
                    Some(s) => Some(s.to_string()),
                    None => {
                        warn!(index = i, "tools[{}]: non-string value, skipping", i);
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let network = input["network"].as_bool().unwrap_or(false);
    let shell: Vec<String> = input["shell"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .enumerate()
                .filter_map(|(i, v)| match v.as_str() {
                    Some(s) => Some(s.to_string()),
                    None => {
                        warn!(index = i, "shell[{}]: non-string value, skipping", i);
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // The spawning agent's own `[model.router_override]`, looked up once
    // (#7789 review). It does two jobs below: it gates the profile this call
    // asks for, and it is copied onto the child so the cap survives the hop.
    //
    // An `Err` is a failed *lookup*, not an absent override, and it fails the
    // spawn closed. A live agent mid-turn always resolves, so the only way
    // here is an id that names no agent — including one handed in by the REST
    // tool endpoint, where `agent_id` is caller-supplied. Treating that as
    // "unconstrained" would make an unresolvable id the cheapest way around
    // every cap in this file.
    let parent_override = match parent_id {
        Some(parent) => kh.model_router_override_for(parent).map_err(|reason| {
            ToolError::upstream_msg(format!(
                "Cannot verify the spawning agent's model-router constraints, so the spawn is \
                 refused rather than granting an unconstrained child: {reason}."
            ))
        })?,
        None => None,
    };

    // Resolve the profile name before spawning so an unknown name fails loudly
    // here instead of silently producing an agent on the wrong model.
    //
    // The request is then checked against the parent's override: the same
    // `allowed_profiles` and `cost_budget` the per-turn router applies, plus a
    // hard refusal when the parent is `fixed`. Without this, delegation is a
    // way around every per-agent constraint the profile layer introduces — an
    // agent budgeted at `cheap` could spawn a helper on the most expensive
    // profile in the catalog, billed to the same operator, with the parent's
    // cap never applied.
    let profile = match input["profile"].as_str() {
        Some(profile_name) => {
            let profile = kh
                .resolve_model_profile(profile_name)
                .ok_or_else(|| unknown_profile_error(profile_name, &kh.model_profile_names()))?;
            if let Some(override_) = parent_override.as_ref() {
                let permitted = permitted_profile_names(kh, override_);
                check_profile_against_parent(&profile, override_, &permitted)?;
            }
            Some(profile)
        }
        None => None,
    };

    let manifest_toml = build_agent_manifest_toml(
        name,
        system_prompt,
        tools,
        shell,
        network,
        profile.as_ref(),
        parent_override.as_ref(),
    )
    .map_err(ToolError::upstream_msg)?;
    // Build parent capabilities from the parent's allowed tools list.
    // This prevents a sub-agent from escalating privileges beyond what
    // its parent is permitted to use (capability inheritance enforcement).
    //
    // Tool names imply resource-level capabilities (matching implied_capabilities
    // logic in ToolProfile): e.g. "web_fetch" implies NetConnect("*"),
    // "shell_exec" implies ShellExec("*"), "agent_spawn" implies AgentSpawn.
    // Without this expansion, validate_capability_inheritance would reject
    // legitimate child capabilities because ToolInvoke("web_fetch") cannot
    // cover a child's NetConnect("*") — they are different Capability variants.
    let parent_caps: Vec<librefang_types::capability::Capability> =
        if let Some(tools) = parent_allowed_tools {
            tools_to_parent_capabilities(tools)
        } else {
            // No allowed_tools means unrestricted parent — grant ToolAll
            vec![librefang_types::capability::Capability::ToolAll]
        };

    let (id, agent_name) = kh
        .spawn_agent_checked(&manifest_toml, parent_id, &parent_caps)
        .await
        .map_err(ToolError::upstream)?;
    Ok(format!(
        "Agent spawned successfully.\n  ID: {id}\n  Name: {agent_name}"
    ))
}

/// The `ephemeral: true` branch of `agent_spawn` (refs #6699).
///
/// Runs the task inline and returns the worker's answer, rather than returning an agent id: an ephemeral worker has no id worth handing back, because by the time this returns it no longer exists.
///
/// The caller agent is not optional here. Every safety property of the ephemeral path — spend billed to a real ledger, the spawning agent's `[resources]` quota enforced, a tool set that cannot exceed the spawner's own — is expressed in terms of the parent, so a callerless surface (the MCP HTTP bridge, the REST tool endpoint) is refused rather than given an unattributed worker.
///
/// Depth is *not* re-checked here. The kernel checks it inside `spawn_ephemeral_worker` and surfaces `CapabilityDenied`, which `ToolError::upstream` preserves; adding a second check against the same counter is how one of two copies eventually stops matching the other.
async fn tool_agent_spawn_ephemeral(
    input: &serde_json::Value,
    kh: &Arc<dyn KernelHandle>,
    name: &str,
    system_prompt: Option<&str>,
    parent_id: Option<&str>,
    parent_allowed_tools: Option<&[String]>,
) -> ToolResult {
    let message = input["message"]
        .as_str()
        .ok_or(ToolError::MissingParameter("message"))?;
    if let Some(violation) = check_taint_outbound_text(message, &TaintSink::agent_message()) {
        return Err(ToolError::PermissionDenied(format!(
            "Taint violation (message): {violation}"
        )));
    }

    let parent = parent_id.ok_or_else(|| caller_agent_id_missing("agent_spawn"))?;
    let parent_id: librefang_types::agent::AgentId =
        parent.parse().map_err(|_| ToolError::InvalidParameter {
            name: "agent_spawn",
            reason: format!("calling agent id '{parent}' is not a UUID"),
        })?;

    let mut request =
        librefang_types::ephemeral::EphemeralSpawnRequest::new(parent_id, name, message);
    request.system_prompt = system_prompt.map(str::to_string);
    request.agent_type = input["agent_type"].as_str().map(str::to_string);
    if let Some(agent_type) = request.agent_type.as_deref() {
        if let Some(violation) = check_taint_outbound_text(agent_type, &TaintSink::agent_message())
        {
            return Err(ToolError::PermissionDenied(format!(
                "Taint violation (agent_type): {violation}"
            )));
        }
    }

    // An explicit tool list is honoured as-is; the kernel rejects any name the
    // parent cannot itself call. When the model asks for no particular tools,
    // fall back to the parent's own allowlist rather than to "everything":
    // a restricted parent that says nothing must not hand its worker a wider
    // set than it holds. `None` (unrestricted parent) stays `None`, which the
    // kernel reads as "whatever the parent's manifest grants".
    let requested: Option<Vec<String>> = match input["tools"].as_array() {
        Some(arr) => Some(
            arr.iter()
                .enumerate()
                .filter_map(|(i, v)| match v.as_str() {
                    Some(s) => Some(s.to_string()),
                    None => {
                        warn!(index = i, "tools[{}]: non-string value, skipping", i);
                        None
                    }
                })
                .collect(),
        ),
        None => parent_allowed_tools.map(<[String]>::to_vec),
    };
    request.tools = requested;

    // Only `provider` and `model` are read out of the caller's `model` object.
    // `EphemeralModelOverride` cannot carry `base_url` or `api_key_env` — see
    // its doc comment for why widening it would be a credential-exfiltration
    // primitive rather than a convenience.
    if let Some(model) = input["model"].as_object() {
        request.model = Some(librefang_types::ephemeral::EphemeralModelOverride {
            provider: model
                .get("provider")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            model: model
                .get("model")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        });
    } else if let Some(model) = input["model"].as_str() {
        request.model = Some(librefang_types::ephemeral::EphemeralModelOverride {
            provider: None,
            model: Some(model.to_string()),
        });
    }

    request.max_iterations = input["max_iterations"]
        .as_u64()
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX));

    let result = kh
        .spawn_ephemeral(request)
        .await
        .map_err(ToolError::upstream)?;

    Ok(format!(
        "Ephemeral worker '{}' finished in {} iteration(s).\n\n{}",
        result.name, result.iterations, result.response
    ))
}

pub(super) fn tool_agent_list(kernel: Option<&Arc<dyn KernelHandle>>) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let agents = kh.list_agents();
    if agents.is_empty() {
        return Ok("No agents currently running.".to_string());
    }
    let mut output = String::with_capacity(64 + agents.len() * 128);
    let _ = writeln!(output, "Running agents ({}):", agents.len());
    for a in &agents {
        let _ = writeln!(
            output,
            "  - {} (id: {}, state: {}, model: {}:{})",
            a.name, a.id, a.state, a.model_provider, a.model_name
        );
    }
    Ok(output)
}

pub(super) fn tool_agent_kill(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("agent_id"))?;
    // agent_id is a UUID/name identifier, not free-form content — no taint check.
    kh.kill_agent(agent_id).map_err(ToolError::upstream)?;
    Ok(format!("Agent {agent_id} killed successfully."))
}

// ---------------------------------------------------------------------------
// agent_type_create — author a reusable agent type from a conversation (#7722)
// ---------------------------------------------------------------------------

/// Create an operator-authored agent type from an agent-authored spec.
///
/// This handler is deliberately thin, for the same reason `tool_workflow_create` is: everything that decides whether the write is legal — the name rule, the refusal to shadow a live agent, the atomic claim against a concurrent create — is enforced kernel-side by the shared `agent_type_store`, which the HTTP `POST /api/templates` handler calls too.
/// Re-checking any of it here would be a second copy of a rule that has to stay identical across the two surfaces, and the second copy is the one that goes stale.
///
/// What it does own is the deserialization boundary.
/// `AgentTypeSpec` is `deny_unknown_fields`, so a key the model invented — `temperature`, `max_tokens`, `memory` — is refused by name instead of being dropped on the floor and answered with a cheerful success the model then builds on.
/// The rejection is reported as an invalid `spec` parameter, carrying serde's own message, because the offending key is exactly what the model needs to see to fix the call on its next turn.
pub(super) async fn tool_agent_type_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let name = input["name"]
        .as_str()
        .ok_or(ToolError::MissingParameter("name"))?
        .to_string();

    let spec: librefang_types::agent_type::AgentTypeSpec = serde_json::from_value(input.clone())
        .map_err(|e| ToolError::InvalidParameter {
            name: "spec",
            reason: e.to_string(),
        })?;

    let kh = require_kernel_typed(kernel)?;
    // `InvalidInput` (a name the store will not accept) and `Conflict` (the name is taken, by an agent type or by a live agent) both describe something the model can fix by calling again with a different name, so the kernel's reason is relayed against `name` rather than flattened into an opaque upstream failure.
    let summary = kh
        .create_agent_type(&name, spec)
        .await
        .map_err(|e| match e {
            librefang_types::error::LibreFangError::InvalidInput(reason)
            | librefang_types::error::LibreFangError::Conflict(reason) => {
                ToolError::InvalidParameter {
                    name: "name",
                    reason,
                }
            }
            other => ToolError::upstream(other),
        })?;

    Ok(serde_json::json!({
        "name": summary.name,
        "description": summary.description,
        "provider": summary.provider,
        "model": summary.model,
        "tools": summary.tools,
        "skills": summary.skills,
    })
    .to_string())
}
