//! Inter-agent tools: `agent_find`, `agent_send`, `agent_spawn`,
//! `agent_list`, `agent_kill`.

use super::error::{ToolError, ToolResult};
use super::{check_taint_outbound_text, require_kernel_typed, AGENT_CALL_DEPTH};
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
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("agent_id"))?;
    let message = input["message"]
        .as_str()
        .ok_or(ToolError::MissingParameter("message"))?;
    let conversation_key = input["conversation_key"].as_str();

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
    let current_depth = AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0);
    if current_depth >= max_depth {
        return Err(ToolError::PermissionDenied(format!(
            "Inter-agent call depth exceeded (max {max_depth}). \
             A->B->C chain is too deep. Use the task queue instead."
        )));
    }

    AGENT_CALL_DEPTH
        .scope(std::cell::Cell::new(current_depth + 1), async {
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
pub(super) fn build_agent_manifest_toml(
    name: &str,
    system_prompt: &str,
    tools: Vec<String>,
    shell: Vec<String>,
    network: bool,
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

    let manifest_json = serde_json::json!({
        "name": name,
        "model": {
            "system_prompt": system_prompt,
        },
        "capabilities": capabilities,
    });

    toml::to_string(&manifest_json).map_err(|e| format!("Failed to serialize to TOML: {}", e))
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
    let system_prompt = input["system_prompt"]
        .as_str()
        .ok_or(ToolError::MissingParameter("system_prompt"))?;

    let spawn_sink = TaintSink::agent_message();
    if let Some(violation) = check_taint_outbound_text(name, &spawn_sink) {
        return Err(ToolError::PermissionDenied(format!(
            "Taint violation (name): {violation}"
        )));
    }
    if let Some(violation) = check_taint_outbound_text(system_prompt, &spawn_sink) {
        return Err(ToolError::PermissionDenied(format!(
            "Taint violation (system_prompt): {violation}"
        )));
    }

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

    let manifest_toml = build_agent_manifest_toml(name, system_prompt, tools, shell, network)
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
