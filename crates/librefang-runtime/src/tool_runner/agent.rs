//! Inter-agent tools: `agent_find`, `agent_send`, `agent_spawn`,
//! `agent_list`, `agent_kill`.

use super::{check_taint_outbound_text, require_kernel, AGENT_CALL_DEPTH};
use crate::kernel_handle::prelude::*;
use librefang_types::taint::TaintSink;
use std::sync::Arc;

pub(super) fn tool_agent_find(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
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
    serde_json::to_string_pretty(&result).map_err(|e| format!("Serialize error: {e}"))
}

pub(super) async fn tool_agent_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or("Missing 'agent_id' parameter")?;
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;

    // Self-send guard: sending a message to oneself would attempt to acquire
    // `agent_msg_locks[id]` while that lock is already held by the current
    // turn, causing an unrecoverable deadlock (issue #3613).
    if let Some(caller) = caller_agent_id {
        if caller == agent_id {
            return Err("agent_send: an agent cannot send a message to itself".to_string());
        }
    }

    // Taint check: refuse to pass obvious credential payloads across
    // the agent boundary. `tool_agent_send` is the entry point for
    // both in-process delegation *and* external A2A peers, so an LLM
    // that stuffs `OPENAI_API_KEY=sk-…` into its own tool-call
    // arguments would otherwise exfiltrate the secret to whoever is
    // on the receiving side. Uses `TaintSink::agent_message` so the
    // rejection message matches the shape documented in the taint
    // module.
    if let Some(violation) = check_taint_outbound_text(message, &TaintSink::agent_message()) {
        return Err(format!("Taint violation: {violation}"));
    }

    // Check + increment inter-agent call depth
    let max_depth = kh.max_agent_call_depth();
    let current_depth = AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0);
    if current_depth >= max_depth {
        return Err(format!(
            "Inter-agent call depth exceeded (max {}). \
             A->B->C chain is too deep. Use the task queue instead.",
            max_depth
        ));
    }

    AGENT_CALL_DEPTH
        .scope(std::cell::Cell::new(current_depth + 1), async {
            // When we know the caller, use the cascade-aware entry so a
            // parent `/stop` propagates into the callee (issue #3044).
            // System-initiated calls (caller_agent_id = None) fall back to
            // the legacy path.
            match caller_agent_id {
                Some(parent) => kh.send_to_agent_as(agent_id, message, parent).await,
                None => kh.send_to_agent(agent_id, message).await,
            }
        })
        .await
        .map_err(|e| e.to_string())
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
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;

    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let system_prompt = input["system_prompt"]
        .as_str()
        .ok_or("Missing 'system_prompt' parameter")?;

    let tools: Vec<String> = input["tools"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let network = input["network"].as_bool().unwrap_or(false);
    let shell: Vec<String> = input["shell"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let manifest_toml = build_agent_manifest_toml(name, system_prompt, tools, shell, network)?;
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
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "Agent spawned successfully.\n  ID: {id}\n  Name: {agent_name}"
    ))
}

pub(super) fn tool_agent_list(kernel: Option<&Arc<dyn KernelHandle>>) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agents = kh.list_agents();
    if agents.is_empty() {
        return Ok("No agents currently running.".to_string());
    }
    let mut output = format!("Running agents ({}):\n", agents.len());
    for a in &agents {
        output.push_str(&format!(
            "  - {} (id: {}, state: {}, model: {}:{})\n",
            a.name, a.id, a.state, a.model_provider, a.model_name
        ));
    }
    Ok(output)
}

pub(super) fn tool_agent_kill(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or("Missing 'agent_id' parameter")?;
    kh.kill_agent(agent_id).map_err(|e| e.to_string())?;
    Ok(format!("Agent {agent_id} killed successfully."))
}
