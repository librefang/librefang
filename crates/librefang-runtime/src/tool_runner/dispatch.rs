//! Tool dispatch.
//!
//! `execute_tool` is the public entry point — it runs the approval /
//! capability / taint gate, then delegates to `execute_tool_raw`, which is
//! the pure `match tool_name { ... }` dispatch table that calls into each
//! `tool_runner::<domain>` module.
//!
//! `ToolExecContext` bundles every cross-cutting handle the dispatch table
//! threads through: kernel handle, registries, sandbox configs, the active
//! workspace root, interrupts, the checkpoint manager, etc.

use super::*;
use crate::mcp;
use crate::web_search::WebToolsContext;
use librefang_skills::registry::SkillRegistry;
use librefang_types::taint::TaintSink;
use librefang_types::tool::{ToolDefinition, ToolResult};
use librefang_types::tool_compat::normalize_tool_name;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, warn};

/// Get the current inter-agent call depth from the task-local context.
/// Returns 0 if called outside an agent task.
pub fn current_agent_depth() -> u32 {
    AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0)
}

/// Runtime context for bare tool dispatch.
///
/// Used by [`execute_tool_raw`] so that tool dispatch is fully separated from
/// the approval / capability / taint gate logic in [`execute_tool`].  Build this
/// from the flat parameter list and pass it down; it can also be constructed
/// directly from a [`librefang_types::tool::DeferredToolExecution`] payload
/// during the resume path.
pub struct ToolExecContext<'a> {
    pub kernel: Option<&'a Arc<dyn KernelHandle>>,
    pub allowed_tools: Option<&'a [String]>,
    /// Full `ToolDefinition` list for the agent's granted tools (builtin +
    /// MCP + skills). When `Some`, lazy-load meta-tools (`tool_load`,
    /// `tool_search`) consult this as the source of truth so non-builtin
    /// tools remain loadable after the eager schema trim (issue #3044).
    /// `None` falls back to the builtin catalog — kept for legacy/test call
    /// sites that don't have the list on hand.
    pub available_tools: Option<&'a [ToolDefinition]>,
    pub caller_agent_id: Option<&'a str>,
    pub skill_registry: Option<&'a SkillRegistry>,
    /// Skill allowlist for the calling agent. Empty slice = all skills allowed.
    pub allowed_skills: Option<&'a [String]>,
    pub mcp_connections: Option<&'a tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    pub web_ctx: Option<&'a WebToolsContext>,
    pub browser_ctx: Option<&'a crate::browser::BrowserManager>,
    pub allowed_env_vars: Option<&'a [String]>,
    pub workspace_root: Option<&'a Path>,
    pub media_engine: Option<&'a crate::media_understanding::MediaEngine>,
    pub media_drivers: Option<&'a crate::media::MediaDriverCache>,
    pub exec_policy: Option<&'a librefang_types::config::ExecPolicy>,
    pub tts_engine: Option<&'a crate::tts::TtsEngine>,
    pub docker_config: Option<&'a librefang_types::config::DockerSandboxConfig>,
    pub process_manager: Option<&'a crate::process_manager::ProcessManager>,
    /// Background process registry — tracks fire-and-forget processes spawned by
    /// `shell_exec` with a rolling 200 KB output buffer.
    pub process_registry: Option<&'a crate::process_registry::ProcessRegistry>,
    pub sender_id: Option<&'a str>,
    pub channel: Option<&'a str>,
    /// LibreFang `SessionId` the tool call belongs to. When `Some`, the
    /// `file_read` / `file_write` builtins consult
    /// `kernel.acp_fs_client(session_id)` and route through the editor's
    /// `fs/*` reverse-RPC instead of the local filesystem (#3313).
    /// `None` for legacy / test call sites that don't have the id on
    /// hand — those keep the previous local-fs behaviour. Owned (vs.
    /// borrowed) because `SessionId` is `Copy` (16 bytes) and the
    /// upstream agent-loop callers pass it as a `Option<&str>` UUID
    /// string that we parse here.
    pub session_id: Option<librefang_types::agent::SessionId>,
    /// Artifact spill threshold from `[tool_results] spill_threshold_bytes`.
    /// Tool results larger than this are written to the artifact store.
    /// `0` means use the compiled default (16 KiB).
    pub spill_threshold_bytes: u64,
    /// Per-artifact write cap from `[tool_results] max_artifact_bytes`.
    /// Spill is skipped when the result exceeds this, falling back to
    /// truncation.  `0` means use the compiled default (64 MiB).
    pub max_artifact_bytes: u64,
    /// Optional checkpoint manager.  When `Some`, a snapshot is taken
    /// automatically before every `file_write` and `apply_patch` call.
    /// Snapshot failures are non-fatal (logged as warnings only).
    pub checkpoint_manager: Option<&'a Arc<crate::checkpoint_manager::CheckpointManager>>,
    /// Per-session interrupt handle.  Tools MAY poll `interrupt.is_cancelled()`
    /// at natural checkpoints to exit early when the user stops the session.
    /// `None` means no interrupt support was wired up for this call site (legacy
    /// paths) — tools must treat `None` the same as "not cancelled".
    pub interrupt: Option<crate::interrupt::SessionInterrupt>,
    /// Session-scoped dangerous command checker. When `Some`, the session allowlist
    /// is preserved across tool calls so previously-approved patterns are not re-blocked.
    pub dangerous_command_checker:
        Option<&'a Arc<tokio::sync::RwLock<crate::dangerous_command::DangerousCommandChecker>>>,
}

/// Execute a tool without running the approval / capability / taint gate.
///
/// This is the pure dispatch layer: it pattern-matches on `tool_name` and calls
/// the right implementation.  All pre-flight checks (capability enforcement,
/// approval gate, taint checks, truncated-args detection) live in the outer
/// [`execute_tool`] wrapper; this function only handles the match.
//
// The `#[allow(unused_variables)]` is for `--no-default-features` builds
// where the media / browser / docker-sandbox tool arms are cfg-gated out
// and the destructured `media_engine`, `media_drivers`, `browser_ctx`,
// `tts_engine`, `docker_config` bindings have no consumer. Re-flagging
// them per-feature would be 5 nested `cfg_attr` blocks; this is cleaner.
#[allow(unused_variables)]
pub async fn execute_tool_raw(
    tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    ctx: &ToolExecContext<'_>,
) -> ToolResult {
    let tool_name = normalize_tool_name(tool_name);

    // §A — notify_owner is dispatched before the result-string wrapper so it
    // can carry a structured `owner_notice` side-channel back to the agent
    // loop. The model sees only an opaque ack in `content` (so it cannot echo
    // the private summary in a public reply); the real payload travels in
    // `ToolResult.owner_notice` and is consumed by `agent_loop.rs`.
    if tool_name == "notify_owner" {
        return tool_notify_owner(tool_use_id, input);
    }

    // Lazy tool loading meta-tools (issue #3044). `tool_load` carries the
    // loaded schema via `ToolResult.loaded_tool` side-channel which the agent
    // loop reads to extend the next request's tools list. Both are dispatched
    // before the generic Result<String, String> wrapper so the side-channel
    // survives.
    if tool_name == "tool_load" {
        let mut r = tool_meta_load(input, ctx.available_tools);
        r.tool_use_id = tool_use_id.to_string();
        return r;
    }
    if tool_name == "tool_search" {
        let mut r = tool_meta_search(input, ctx.available_tools);
        r.tool_use_id = tool_use_id.to_string();
        return r;
    }

    let ToolExecContext {
        kernel,
        allowed_tools,
        available_tools: _,
        caller_agent_id,
        skill_registry,
        allowed_skills,
        mcp_connections,
        web_ctx,
        browser_ctx,
        allowed_env_vars,
        workspace_root,
        media_engine,
        media_drivers,
        exec_policy,
        tts_engine,
        docker_config,
        process_manager,
        process_registry: _,
        sender_id,
        channel,
        session_id,
        spill_threshold_bytes,
        max_artifact_bytes,
        checkpoint_manager,
        interrupt,
        dangerous_command_checker,
    } = ctx;

    let result = match tool_name {
        // Filesystem tools
        "file_read" => {
            // SECURITY: Validate the requested path stays inside the
            // agent's allowed-workspace set BEFORE handing off to ACP
            // (#3313 review). The editor would otherwise faithfully
            // serve `/etc/shadow` back to the LLM if the agent asked
            // for it — the editor sandbox is for editor users, not
            // for agents pretending to be editor users.
            let mut allowed = named_ws_prefixes(*kernel, *caller_agent_id);
            if let Some(dl) = kernel.and_then(|k| k.channel_file_download_dir()) {
                allowed.push(dl);
            }
            if let Some(violation) = check_absolute_path_inside_workspace(
                input.get("path").and_then(|v| v.as_str()),
                *workspace_root,
                &allowed,
            ) {
                return ToolResult::error(tool_use_id.to_string(), violation);
            }

            // ACP routing: when an editor is bound to this session,
            // hand the read off to the editor's `fs/read_text_file`
            // instead of touching the local fs. The editor sees its
            // in-memory buffer state (unsaved edits, virtual fs) which
            // is what the user expects when prompting from inside the
            // editor (#3313).
            if let (Some(k), Some(sid)) = (kernel, session_id) {
                if let Some(client) = k.acp_fs_client(*sid) {
                    let Some(path_str) = input.get("path").and_then(|v| v.as_str()) else {
                        return ToolResult::error(
                            tool_use_id.to_string(),
                            "Missing 'path' parameter".to_string(),
                        );
                    };
                    let path = std::path::PathBuf::from(path_str);
                    let line = input["line"].as_u64().map(|v| v as u32);
                    let limit = input["limit"].as_u64().map(|v| v as u32);
                    return match client.read_text_file(path.clone(), line, limit).await {
                        Ok(content) => {
                            // #4971: dedup repeated reads of the same buffer.
                            // Only applies when no slicing args were supplied —
                            // a partial read (`line` / `limit`) returns a
                            // window, not the full content, so hashing would
                            // be lossy.
                            let final_content = if line.is_none() && limit.is_none() {
                                maybe_dedup_file_read(*kernel, *session_id, &path, content)
                            } else {
                                content
                            };
                            ToolResult::ok(tool_use_id.to_string(), final_content)
                        }
                        Err(e) => ToolResult::error(
                            tool_use_id.to_string(),
                            format!("ACP fs/read_text_file failed: {e}"),
                        ),
                    };
                }
            }
            let extra_refs: Vec<&Path> = allowed.iter().map(|p| p.as_path()).collect();
            let raw_input_path = input.get("path").and_then(|v| v.as_str());
            let resolved_for_dedup = raw_input_path
                .and_then(|p| resolve_file_path_ext(p, *workspace_root, &extra_refs).ok());
            tool_file_read(input, *workspace_root, &extra_refs)
                .await
                .map(|content| match resolved_for_dedup {
                    Some(resolved) => {
                        maybe_dedup_file_read(*kernel, *session_id, &resolved, content)
                    }
                    None => content,
                })
        }
        "file_write" => {
            // Enforce named workspace read-only restrictions before the sandbox resolves the path.
            // Agents learn absolute workspace paths from TOOLS.md; an absolute path that falls
            // inside a read-only named workspace must be rejected here.
            if let (Some(k), Some(agent_id)) = (kernel, caller_agent_id) {
                let raw = input["path"].as_str().unwrap_or("");
                if Path::new(raw).is_absolute() {
                    let ro = k.readonly_workspace_prefixes(agent_id);
                    if ro.iter().any(|prefix| Path::new(raw).starts_with(prefix)) {
                        return ToolResult {
                            tool_use_id: tool_use_id.to_string(),
                            content: format!(
                                "Write denied: '{}' is in a read-only named workspace",
                                raw
                            ),
                            is_error: true,
                            ..Default::default()
                        };
                    }
                }
            }
            // SECURITY: workspace-jail check on absolute paths BEFORE
            // ACP routing (#3313 review). Same rationale as file_read:
            // the editor sandbox is for editor users, not agents.
            // `tool_file_write` runs the equivalent check on the
            // local-fs path; this is the missing pre-ACP guard.
            let writable = named_ws_prefixes_writable(*kernel, *caller_agent_id);
            if let Some(violation) = check_absolute_path_inside_workspace(
                input.get("path").and_then(|v| v.as_str()),
                *workspace_root,
                &writable,
            ) {
                return ToolResult::error(tool_use_id.to_string(), violation);
            }
            // ACP routing: if an editor is attached to this session,
            // route the write through `fs/write_text_file` so it goes
            // into the editor's buffer (with its own undo stack and
            // dirty-state tracking) instead of the local fs (#3313).
            if let (Some(k), Some(sid)) = (kernel, session_id) {
                if let Some(client) = k.acp_fs_client(*sid) {
                    let Some(path_str) = input.get("path").and_then(|v| v.as_str()) else {
                        return ToolResult::error(
                            tool_use_id.to_string(),
                            "Missing 'path' parameter".to_string(),
                        );
                    };
                    let Some(content) = input.get("content").and_then(|v| v.as_str()) else {
                        return ToolResult::error(
                            tool_use_id.to_string(),
                            "Missing 'content' parameter".to_string(),
                        );
                    };
                    let path = std::path::PathBuf::from(path_str);
                    return match client.write_text_file(path, content.to_string()).await {
                        Ok(()) => ToolResult::ok(
                            tool_use_id.to_string(),
                            format!("Wrote {path_str} via editor"),
                        ),
                        Err(e) => ToolResult::error(
                            tool_use_id.to_string(),
                            format!("ACP fs/write_text_file failed: {e}"),
                        ),
                    };
                }
            }
            maybe_snapshot(checkpoint_manager, *workspace_root, "pre file_write").await;
            let extra_refs: Vec<&Path> = writable.iter().map(|p| p.as_path()).collect();
            tool_file_write(input, *workspace_root, &extra_refs).await
        }
        "file_list" => {
            let mut extra = named_ws_prefixes(*kernel, *caller_agent_id);
            // #4434: see file_read above — bridge download dir is read-side allowlisted.
            if let Some(dl) = kernel.and_then(|k| k.channel_file_download_dir()) {
                extra.push(dl);
            }
            let extra_refs: Vec<&Path> = extra.iter().map(|p| p.as_path()).collect();
            tool_file_list(input, *workspace_root, &extra_refs).await
        }
        "apply_patch" => {
            // SECURITY #3662: Enforce named workspace read-only restrictions
            // before applying the patch.  Mirrors the upfront check in the
            // `file_write` arm: any absolute target path that falls inside a
            // read-only named workspace is rejected here, before the sandbox
            // resolver even runs.  The sandbox itself would also block such
            // writes (readonly workspaces are excluded from `additional_roots`),
            // but the explicit pre-check catches the violation earlier and
            // returns a clearer error message.
            if let (Some(k), Some(agent_id)) = (kernel, caller_agent_id) {
                let ro = k.readonly_workspace_prefixes(agent_id);
                if !ro.is_empty() {
                    // Parse the patch to inspect target paths before executing.
                    if let Some(patch_str) = input["patch"].as_str() {
                        if let Ok(ops) = crate::apply_patch::parse_patch(patch_str) {
                            for op in &ops {
                                let raw_paths: Vec<&str> = match op {
                                    crate::apply_patch::PatchOp::AddFile { path, .. } => {
                                        vec![path.as_str()]
                                    }
                                    crate::apply_patch::PatchOp::UpdateFile {
                                        path,
                                        move_to,
                                        ..
                                    } => {
                                        let mut v = vec![path.as_str()];
                                        if let Some(dest) = move_to {
                                            v.push(dest.as_str());
                                        }
                                        v
                                    }
                                    crate::apply_patch::PatchOp::DeleteFile { path } => {
                                        vec![path.as_str()]
                                    }
                                };
                                for raw in raw_paths {
                                    if Path::new(raw).is_absolute()
                                        && ro
                                            .iter()
                                            .any(|prefix| Path::new(raw).starts_with(prefix))
                                    {
                                        return ToolResult {
                                            tool_use_id: tool_use_id.to_string(),
                                            content: format!(
                                                "Write denied: '{}' is in a read-only named workspace",
                                                raw
                                            ),
                                            is_error: true,
                                            ..Default::default()
                                        };
                                    }
                                }
                            }
                        }
                    }
                }
            }
            maybe_snapshot(checkpoint_manager, *workspace_root, "pre apply_patch").await;
            // apply_patch needs write access — restrict to rw named workspaces only.
            let extra = named_ws_prefixes_writable(*kernel, *caller_agent_id);
            let extra_refs: Vec<&Path> = extra.iter().map(|p| p.as_path()).collect();
            // SECURITY #3662 (defense-in-depth): also propagate the *canonical*
            // read-only prefixes so `apply_patch_ext` can reject any resolved
            // path that lands inside a read-only workspace, even if a future
            // refactor of `additional_roots` accidentally widens the writable
            // set.
            let ro_prefixes = named_ws_prefixes_readonly(*kernel, *caller_agent_id);
            let ro_refs: Vec<&Path> = ro_prefixes.iter().map(|p| p.as_path()).collect();
            tool_apply_patch(input, *workspace_root, &extra_refs, &ro_refs).await
        }

        // Web tools (upgraded: multi-provider search, SSRF-protected fetch)
        "web_fetch" => match input["url"].as_str() {
            None => Err("Missing 'url' parameter".to_string()),
            Some(url) => {
                // Taint check: block URLs containing secrets/PII from being exfiltrated
                if let Some(violation) = check_taint_net_fetch(url) {
                    return ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!("Taint violation: {violation}"),
                        is_error: true,
                        ..Default::default()
                    };
                }
                let method = input["method"].as_str().unwrap_or("GET");
                let headers = input.get("headers").and_then(|v| v.as_object());
                let body = input["body"].as_str();
                // Body-side taint check: the URL scan handles query
                // strings, but POST/PUT callers can stuff credentials
                // into the request body instead.
                if let Some(body_text) = body {
                    if let Some(violation) =
                        check_taint_outbound_text(body_text, &TaintSink::net_fetch())
                    {
                        return ToolResult {
                            tool_use_id: tool_use_id.to_string(),
                            content: format!("Taint violation: {violation}"),
                            is_error: true,
                            ..Default::default()
                        };
                    }
                }
                // Header values, too — an LLM that knows the filter
                // blocks `body` might fall back to stuffing the token
                // into `Authorization:` via `headers`.
                if let Some(headers_map) = headers {
                    for (name, value) in headers_map {
                        if let Some(vs) = value.as_str() {
                            if let Some(violation) =
                                check_taint_outbound_header(name, vs, &TaintSink::net_fetch())
                            {
                                return ToolResult {
                                    tool_use_id: tool_use_id.to_string(),
                                    content: format!("Taint violation: {violation}"),
                                    is_error: true,
                                    ..Default::default()
                                };
                            }
                        }
                    }
                }
                let (threshold, max_artifact) =
                    resolve_spill_config(*spill_threshold_bytes, *max_artifact_bytes);
                if let Some(ctx) = web_ctx {
                    // #3347 5/N: also wire spill into the primary
                    // WebToolsContext::fetch path (Tavily / Brave / Jina /
                    // SSRF-protected GET).  #4651 only wired the legacy
                    // plain-HTTP fallback; large readability-converted
                    // payloads on the main path were still inlined.
                    ctx.fetch
                        .fetch_with_options(url, method, headers, body)
                        .await
                        .map(|body| {
                            spill_or_passthrough("web_fetch", body, threshold, max_artifact)
                        })
                } else {
                    tool_web_fetch_legacy(input, threshold, max_artifact).await
                }
            }
        },
        "web_fetch_to_file" => {
            // Taint scans on URL / headers / body mirror the `web_fetch`
            // arm exactly — same TaintSink::net_fetch() sink, same outbound
            // semantics. Writing to disk does not soften the outbound
            // exfiltration risk because the URL itself still leaves the
            // host (and the response is persisted, not just transient).
            let Some(url) = input["url"].as_str() else {
                return ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content: "Missing 'url' parameter".to_string(),
                    is_error: true,
                    ..Default::default()
                };
            };
            if let Some(violation) = check_taint_net_fetch(url) {
                return ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content: format!("Taint violation: {violation}"),
                    is_error: true,
                    ..Default::default()
                };
            }
            if let Some(body_text) = input["body"].as_str() {
                if let Some(violation) =
                    check_taint_outbound_text(body_text, &TaintSink::net_fetch())
                {
                    return ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!("Taint violation: {violation}"),
                        is_error: true,
                        ..Default::default()
                    };
                }
            }
            if let Some(headers_map) = input.get("headers").and_then(|v| v.as_object()) {
                for (name, value) in headers_map {
                    if let Some(vs) = value.as_str() {
                        if let Some(violation) =
                            check_taint_outbound_header(name, vs, &TaintSink::net_fetch())
                        {
                            return ToolResult {
                                tool_use_id: tool_use_id.to_string(),
                                content: format!("Taint violation: {violation}"),
                                is_error: true,
                                ..Default::default()
                            };
                        }
                    }
                }
            }

            // dest_path pre-flight checks mirror the `file_write` arm:
            // reject writes that land in a read-only named workspace, and
            // reject absolute paths that escape every allowed prefix or
            // contain `..` components.
            if let (Some(k), Some(agent_id)) = (kernel, caller_agent_id) {
                let raw = input["dest_path"].as_str().unwrap_or("");
                if Path::new(raw).is_absolute() {
                    let ro = k.readonly_workspace_prefixes(agent_id);
                    if ro.iter().any(|prefix| Path::new(raw).starts_with(prefix)) {
                        return ToolResult {
                            tool_use_id: tool_use_id.to_string(),
                            content: format!(
                                "Write denied: '{}' is in a read-only named workspace",
                                raw
                            ),
                            is_error: true,
                            ..Default::default()
                        };
                    }
                }
            }
            let writable = named_ws_prefixes_writable(*kernel, *caller_agent_id);
            if let Some(violation) = check_absolute_path_inside_workspace(
                input.get("dest_path").and_then(|v| v.as_str()),
                *workspace_root,
                &writable,
            ) {
                return ToolResult::error(tool_use_id.to_string(), violation);
            }

            let extra_refs: Vec<&Path> = writable.iter().map(|p| p.as_path()).collect();
            crate::web_fetch_to_file::tool_web_fetch_to_file(
                input,
                *web_ctx,
                *workspace_root,
                &extra_refs,
            )
            .await
        }
        "web_search" => match input["query"].as_str() {
            None => Err("Missing 'query' parameter".to_string()),
            Some(query) => {
                let max_results = input["max_results"].as_u64().unwrap_or(5) as usize;
                let (threshold, max_artifact) =
                    resolve_spill_config(*spill_threshold_bytes, *max_artifact_bytes);
                if let Some(ctx) = web_ctx {
                    ctx.search.search(query, max_results).await.map(|body| {
                        spill_or_passthrough("web_search", body, threshold, max_artifact)
                    })
                } else {
                    tool_web_search_legacy(input).await.map(|body| {
                        spill_or_passthrough("web_search", body, threshold, max_artifact)
                    })
                }
            }
        },

        // Shell tool — exec policy + metacharacter check + taint check
        "shell_exec" => {
            let Some(command) = input["command"].as_str() else {
                return ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content: "Missing 'command' parameter".to_string(),
                    is_error: true,
                    ..Default::default()
                };
            };

            // SECURITY (#3313 review): every check below runs BEFORE
            // the ACP routing branch — earlier revisions of this file
            // returned to the editor's terminal panel before validating
            // exec_policy / metacharacters / taint / dangerous patterns
            // / readonly-workspace prefixes, which let an agent
            // exfiltrate or destroy local data through the editor by
            // sending commands the LibreFang sandbox would otherwise
            // refuse. The editor's own sandbox is for editor users —
            // an agent driving the editor must satisfy LibreFang's
            // policy first.

            // FIXME(#3822): shell_exec still cannot stop a spawned
            // process from writing to read-only named workspaces (no
            // mount-namespace / sandbox-exec / chroot). We block
            // commands whose argv references a read-only prefix
            // below, but a process that calls `open()` directly with
            // a hard-coded path is out of scope for this layer.
            if let (Some(k), Some(aid)) = (kernel, caller_agent_id) {
                let ro = k.readonly_workspace_prefixes(aid);
                if !ro.is_empty() {
                    tracing::debug!(
                        agent_id = %aid,
                        readonly_prefixes = ?ro,
                        "shell_exec: argv-level readonly enforcement engaged \
                         (in-process syscalls bypass this layer — see #3822)"
                    );
                }
            }

            let is_full_exec = exec_policy
                .is_some_and(|p| p.mode == librefang_types::config::ExecSecurityMode::Full);

            // Exec policy enforcement (allowlist / deny / full)
            if let Some(policy) = exec_policy {
                if let Err(reason) =
                    crate::subprocess_sandbox::validate_command_allowlist(command, policy)
                {
                    return ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!(
                            "shell_exec blocked: {reason}. Current exec_policy.mode = '{:?}'. \
                             To allow shell commands, set exec_policy.mode = 'full' in the agent manifest or config.toml.",
                            policy.mode
                        ),
                        is_error: true,
                        ..Default::default()
                    };
                }
            }

            // SECURITY: Check for shell metacharacters in non-full modes.
            // Full mode explicitly trusts the agent — skip metacharacter checks.
            if !is_full_exec {
                if let Some(reason) =
                    crate::subprocess_sandbox::contains_shell_metacharacters(command)
                {
                    return ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!(
                            "shell_exec blocked: command contains {reason}. \
                             Shell metacharacters are not allowed in allowlist mode."
                        ),
                        is_error: true,
                        ..Default::default()
                    };
                }
            }

            // Skip heuristic taint patterns for Full exec policy (e.g. hand agents that need curl)
            if !is_full_exec {
                if let Some(violation) = check_taint_shell_exec(command) {
                    return ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!("Taint violation: {violation}"),
                        is_error: true,
                        ..Default::default()
                    };
                }
            }

            // Dangerous command detection gate.
            //
            // Runs in Manual mode for all exec policies (including Full) because
            // even explicitly-trusted agents should not silently execute commands
            // like `rm -rf /` or fork bombs.
            //
            // In Manual mode a Dangerous result causes an immediate block with a
            // descriptive error. The agent can route approval via the existing
            // `submit_tool_approval` path by catching the error message and
            // re-submitting after the user has explicitly allowed the pattern.
            {
                use crate::dangerous_command::{
                    ApprovalMode, CheckResult, DangerousCommandChecker,
                };
                let check_result = if let Some(checker_arc) = dangerous_command_checker {
                    checker_arc.read().await.check(command)
                } else {
                    DangerousCommandChecker::new(ApprovalMode::Manual).check(command)
                };
                if let CheckResult::Dangerous { description } = check_result {
                    warn!(
                        command = crate::str_utils::safe_truncate_str(command, 120),
                        description, "Dangerous command detected — blocking execution"
                    );
                    return ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!(
                            "shell_exec blocked: dangerous command detected ({description}). \
                             The command matches a known-dangerous pattern and has been blocked \
                             for safety. If you need to run this command, request explicit user \
                             approval first."
                        ),
                        is_error: true,
                        ..Default::default()
                    };
                }
            }

            // SECURITY (fix #3822, improved by #4903): enforce named workspace
            // read-only restrictions for shell_exec using argument-role awareness.
            //
            // The original implementation blocked *any* mention of an RO path in
            // the command, which caused false-positives for read commands such as
            // `cat /vaults-ro/x/foo.md`. The new approach uses
            // `classify_shell_exec_ro_safety` to distinguish reads (allowed) from
            // writes (blocked). Unrecognised verbs still fall back to deny so the
            // security posture is not weakened. See the module-level comment above
            // `classify_shell_exec_ro_safety` for the full design rationale.
            if let (Some(k), Some(agent_id)) = (kernel, caller_agent_id) {
                let ro_prefixes = k.readonly_workspace_prefixes(agent_id);
                if !ro_prefixes.is_empty() {
                    // Build the full command string that includes any explicit `args`
                    // entries. We append them to the base command so the classifier
                    // can tokenise everything together.
                    let mut full_command = command.to_string();
                    if let Some(args_arr) = input.get("args").and_then(|a| a.as_array()) {
                        for v in args_arr {
                            if let Some(s) = v.as_str() {
                                full_command.push(' ');
                                full_command.push_str(s);
                            }
                        }
                    }
                    for ro_prefix in &ro_prefixes {
                        let prefix_str = ro_prefix.to_string_lossy();
                        // Only run the classifier if the RO prefix actually appears in
                        // the command (quick short-circuit to avoid allocations).
                        if !full_command.contains(prefix_str.as_ref()) {
                            continue;
                        }
                        // Path-boundary check: make sure it's not a shared-prefix
                        // false-positive (e.g. /data vs /data2).
                        //
                        // We must check ALL occurrences, not just the first one.
                        // A command like `cat /vaults-roxxx/dummy; rm /vaults-ro/x/foo`
                        // has its first match at `/vaults-roxxx` (boundary fails),
                        // so using `.find()` alone would skip the second real match
                        // and let the `rm` through (B1).
                        let at_boundary = {
                            let ps = prefix_str.as_ref();
                            full_command.match_indices(ps).any(|(idx, _)| {
                                let after = &full_command[idx + ps.len()..];
                                after.is_empty()
                                    || after.starts_with('/')
                                    || after.starts_with('"')
                                    || after.starts_with('\'')
                                    || after.starts_with(' ')
                            })
                        };
                        if !at_boundary {
                            continue;
                        }
                        if let RoSafety::Block(reason) =
                            classify_shell_exec_ro_safety(&full_command, prefix_str.as_ref())
                        {
                            return ToolResult {
                                tool_use_id: tool_use_id.to_string(),
                                content: reason,
                                is_error: true,
                                ..Default::default()
                            };
                        }
                    }
                }
            }

            // ACP routing: when an editor is bound to this session and
            // declares `terminal` capability, host the command's PTY in
            // the editor's terminal panel (#3313). All LibreFang-side
            // policy checks above must pass first — see the SECURITY
            // comment at the top of this arm.
            //
            // We also pass `cwd = Some(workspace_root)` (when
            // available) so the editor terminal lands inside the
            // agent's declared workspace, mirroring the local-exec
            // path. Earlier revisions passed `None`, which let the
            // editor pick its session cwd — fine for project-scoped
            // editors, but invalid relative paths once the agent's
            // own workspace differs from the editor's project root
            // (e.g. a daemon-attached agent in `~/.librefang/agents/X`).
            if let (Some(k), Some(sid)) = (kernel, session_id) {
                if let Some(client) = k.acp_terminal_client(*sid) {
                    if client.capabilities() {
                        let cwd_for_acp = workspace_root.map(|p| p.to_path_buf());
                        // Pick a platform-appropriate command interpreter.
                        // ACP's trust model is same-user, same-host, so
                        // the editor's host platform matches the
                        // daemon's; `cfg!(windows)` gates correctly.
                        // Hardcoding `sh -c` would fail on Windows
                        // editors that don't ship a POSIX shell on PATH.
                        let (shell, shell_arg) = if cfg!(windows) {
                            ("cmd", "/C")
                        } else {
                            ("sh", "-c")
                        };
                        let result = client
                            .run_command(
                                shell.to_string(),
                                vec![shell_arg.to_string(), command.to_string()],
                                Vec::new(),
                                cwd_for_acp,
                                Some(64 * 1024),
                            )
                            .await;
                        return match result {
                            Ok(r) => {
                                let suffix = if r.truncated {
                                    "\n[output truncated]"
                                } else {
                                    ""
                                };
                                let exit_summary = match (r.exit_code, r.signal) {
                                    (Some(0), _) => String::new(),
                                    (Some(code), _) => format!("\n[exit code: {code}]"),
                                    (None, Some(sig)) => format!("\n[signal: {sig}]"),
                                    (None, None) => "\n[exit: unknown]".to_string(),
                                };
                                let is_err = r.exit_code.unwrap_or(1) != 0;
                                ToolResult {
                                    tool_use_id: tool_use_id.to_string(),
                                    content: format!("{}{suffix}{exit_summary}", r.output),
                                    is_error: is_err,
                                    ..Default::default()
                                }
                            }
                            Err(e) => ToolResult::error(
                                tool_use_id.to_string(),
                                format!("ACP terminal/* failed: {e}"),
                            ),
                        };
                    }
                }
            }

            let effective_allowed_env_vars = allowed_env_vars.or_else(|| {
                exec_policy.and_then(|policy| {
                    if policy.allowed_env_vars.is_empty() {
                        None
                    } else {
                        Some(policy.allowed_env_vars.as_slice())
                    }
                })
            });
            tool_shell_exec(
                input,
                effective_allowed_env_vars.unwrap_or(&[]),
                *workspace_root,
                *exec_policy,
                interrupt.clone(),
            )
            .await
        }

        // Inter-agent tools (require kernel handle)
        "agent_send" => tool_agent_send(input, *kernel, *caller_agent_id).await,
        "agent_spawn" => tool_agent_spawn(input, *kernel, *caller_agent_id, *allowed_tools).await,
        "agent_list" => tool_agent_list(*kernel),
        "agent_kill" => tool_agent_kill(input, *kernel),

        // Shared memory tools (peer-scoped when sender_id is present).
        // #5139: the per-user `UserMemoryAccess` ACL is enforced inside each
        // tool fn (`enforce_memory_acl`) using the attributed sender +
        // channel, mirroring the proactive-retrieval gate.
        "memory_store" => tool_memory_store(input, *kernel, *sender_id, *channel),
        "memory_recall" => tool_memory_recall(input, *kernel, *sender_id, *channel),
        "memory_list" => tool_memory_list(*kernel, *sender_id, *channel),

        // Memory wiki tools (issue #3329) — same #5139 per-user ACL gate.
        "wiki_get" => tool_wiki_get(input, *kernel, *sender_id, *channel),
        "wiki_search" => tool_wiki_search(input, *kernel, *sender_id, *channel),
        "wiki_write" => tool_wiki_write(input, *kernel, *caller_agent_id, *sender_id, *channel),

        // Collaboration tools
        "agent_find" => tool_agent_find(input, *kernel),
        "task_post" => tool_task_post(input, *kernel, *caller_agent_id).await,
        "task_claim" => tool_task_claim(*kernel, *caller_agent_id).await,
        "task_complete" => tool_task_complete(input, *kernel, *caller_agent_id).await,
        "task_list" => tool_task_list(input, *kernel).await,
        "task_status" => tool_task_status(input, *kernel).await,
        "event_publish" => tool_event_publish(input, *kernel).await,

        // Scheduling tools (delegate to CronScheduler via kernel handle)
        "schedule_create" => {
            tool_schedule_create(input, *kernel, *caller_agent_id, *sender_id).await
        }
        "schedule_list" => tool_schedule_list(*kernel, *caller_agent_id).await,
        "schedule_delete" => tool_schedule_delete(input, *kernel).await,

        // Knowledge graph tools
        "knowledge_add_entity" => tool_knowledge_add_entity(input, *kernel).await,
        "knowledge_add_relation" => tool_knowledge_add_relation(input, *kernel).await,
        "knowledge_query" => tool_knowledge_query(input, *kernel).await,

        // Image analysis tool
        "image_analyze" => {
            // #4981: media read tools must see into the channel-bridge
            // download dir (e.g. `/tmp/librefang_uploads/<uuid>.jpg`) the
            // same way `file_read` does — the kernel itself delivers those
            // paths to the agent in inbound channel messages, so refusing
            // to open them is internally contradictory.
            let mut extra = named_ws_prefixes(*kernel, *caller_agent_id);
            if let Some(dl) = kernel.and_then(|k| k.channel_file_download_dir()) {
                extra.push(dl);
            }
            let extra_refs: Vec<&Path> = extra.iter().map(|p| p.as_path()).collect();
            tool_image_analyze(input, *workspace_root, &extra_refs).await
        }

        // Media understanding tools
        #[cfg(feature = "media")]
        "media_describe" => {
            // #4981: see image_analyze above — staging dir is read-side allowlisted.
            let mut extra = named_ws_prefixes(*kernel, *caller_agent_id);
            if let Some(dl) = kernel.and_then(|k| k.channel_file_download_dir()) {
                extra.push(dl);
            }
            let extra_refs: Vec<&Path> = extra.iter().map(|p| p.as_path()).collect();
            tool_media_describe(input, *media_engine, *workspace_root, &extra_refs).await
        }
        #[cfg(feature = "media")]
        "media_transcribe" => {
            // #4981: see image_analyze above — staging dir is read-side allowlisted.
            // This is the primary path: Telegram voice messages land at
            // `<staging>/<uuid>.oga` and the agent calls media_transcribe on
            // exactly that path.
            let mut extra = named_ws_prefixes(*kernel, *caller_agent_id);
            if let Some(dl) = kernel.and_then(|k| k.channel_file_download_dir()) {
                extra.push(dl);
            }
            let extra_refs: Vec<&Path> = extra.iter().map(|p| p.as_path()).collect();
            tool_media_transcribe(input, *media_engine, *workspace_root, &extra_refs).await
        }

        // Media generation tools (MediaDriver-based)
        #[cfg(feature = "media")]
        "image_generate" => {
            let upload_dir = kernel
                .map(|k| k.effective_upload_dir())
                .unwrap_or_else(|| std::env::temp_dir().join("librefang_uploads"));
            tool_image_generate(input, *media_drivers, *workspace_root, &upload_dir).await
        }
        #[cfg(feature = "media")]
        "video_generate" => tool_video_generate(input, *media_drivers).await,
        #[cfg(feature = "media")]
        "video_status" => tool_video_status(input, *media_drivers).await,
        #[cfg(feature = "media")]
        "music_generate" => tool_music_generate(input, *media_drivers, *workspace_root).await,

        // TTS/STT tools
        #[cfg(feature = "media")]
        "text_to_speech" => {
            tool_text_to_speech(input, *media_drivers, *tts_engine, *workspace_root).await
        }
        #[cfg(feature = "media")]
        "speech_to_text" => {
            // #4981: see image_analyze above — staging dir is read-side allowlisted.
            let mut extra = named_ws_prefixes(*kernel, *caller_agent_id);
            if let Some(dl) = kernel.and_then(|k| k.channel_file_download_dir()) {
                extra.push(dl);
            }
            let extra_refs: Vec<&Path> = extra.iter().map(|p| p.as_path()).collect();
            tool_speech_to_text(input, *media_engine, *workspace_root, &extra_refs).await
        }

        // Docker sandbox tool
        #[cfg(feature = "docker-sandbox")]
        "docker_exec" => {
            tool_docker_exec(input, *docker_config, *workspace_root, *caller_agent_id).await
        }

        // Location tool
        "location_get" => tool_location_get().await,

        // System time tool
        "system_time" => Ok(tool_system_time()),

        // Skill file read tool
        "skill_read_file" => tool_skill_read_file(input, *skill_registry, *allowed_skills).await,

        // Skill evolution tools
        "skill_evolve_create" => {
            tool_skill_evolve_create(input, *skill_registry, *caller_agent_id).await
        }
        "skill_evolve_update" => {
            tool_skill_evolve_update(input, *skill_registry, *caller_agent_id).await
        }
        "skill_evolve_patch" => {
            tool_skill_evolve_patch(input, *skill_registry, *caller_agent_id).await
        }
        "skill_evolve_delete" => tool_skill_evolve_delete(input, *skill_registry).await,
        "skill_evolve_rollback" => {
            tool_skill_evolve_rollback(input, *skill_registry, *caller_agent_id).await
        }
        "skill_evolve_write_file" => tool_skill_evolve_write_file(input, *skill_registry).await,
        "skill_evolve_remove_file" => tool_skill_evolve_remove_file(input, *skill_registry).await,

        // Cron scheduling tools
        "cron_create" => tool_cron_create(input, *kernel, *caller_agent_id, *sender_id).await,
        "cron_list" => tool_cron_list(*kernel, *caller_agent_id).await,
        "cron_cancel" => tool_cron_cancel(input, *kernel, *caller_agent_id).await,

        // Channel send tool (proactive outbound messaging)
        "channel_send" => {
            let extra = named_ws_prefixes(*kernel, *caller_agent_id);
            let extra_refs: Vec<&Path> = extra.iter().map(|p| p.as_path()).collect();
            tool_channel_send(
                input,
                *kernel,
                *workspace_root,
                *sender_id,
                *caller_agent_id,
                &extra_refs,
            )
            .await
        }

        // Persistent process tools
        "process_start" => tool_process_start(input, *process_manager, *caller_agent_id).await,
        "process_poll" => tool_process_poll(input, *process_manager).await,
        "process_write" => tool_process_write(input, *process_manager).await,
        "process_kill" => tool_process_kill(input, *process_manager).await,
        "process_list" => tool_process_list(*process_manager, *caller_agent_id).await,

        // Hand tools (curated autonomous capability packages)
        "hand_list" => tool_hand_list(*kernel).await,
        "hand_activate" => tool_hand_activate(input, *kernel).await,
        "hand_status" => tool_hand_status(input, *kernel).await,
        "hand_deactivate" => tool_hand_deactivate(input, *kernel).await,

        // A2A outbound tools (cross-instance agent communication)
        "a2a_discover" => tool_a2a_discover(input).await,
        "a2a_send" => tool_a2a_send(input, *kernel).await,

        // Goal tracking tool
        "goal_update" => tool_goal_update(input, *kernel),

        // Workflow tools
        "workflow_run" => tool_workflow_run(input, *kernel).await,
        "workflow_list" => tool_workflow_list(*kernel).await,
        "workflow_describe" => tool_workflow_describe(input, *kernel).await,
        "workflow_status" => tool_workflow_status(input, *kernel).await,
        "workflow_start" => {
            tool_workflow_start(input, *kernel, *caller_agent_id, *session_id).await
        }
        "workflow_cancel" => tool_workflow_cancel(input, *kernel).await,

        // Browser automation tools
        #[cfg(feature = "browser")]
        "browser_navigate" => {
            let Some(url) = input["url"].as_str() else {
                return ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content: "Missing 'url' parameter".to_string(),
                    is_error: true,
                    ..Default::default()
                };
            };
            if let Some(violation) = check_taint_net_fetch(url) {
                return ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content: format!("Taint violation: {violation}"),
                    is_error: true,
                    ..Default::default()
                };
            }
            match browser_ctx {
                Some(mgr) => {
                    let aid = caller_agent_id.unwrap_or("default");
                    crate::browser_tools::tool_browser_navigate(input, mgr, aid).await
                }
                None => Err(
                    "Browser tools not available. Ensure Chrome/Chromium is installed.".to_string(),
                ),
            }
        }
        #[cfg(feature = "browser")]
        "browser_click" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser_tools::tool_browser_click(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        #[cfg(feature = "browser")]
        "browser_type" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser_tools::tool_browser_type(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        #[cfg(feature = "browser")]
        "browser_screenshot" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                let upload_dir = kernel
                    .map(|k| k.effective_upload_dir())
                    .unwrap_or_else(|| std::env::temp_dir().join("librefang_uploads"));
                crate::browser_tools::tool_browser_screenshot(input, mgr, aid, &upload_dir).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        #[cfg(feature = "browser")]
        "browser_read_page" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser_tools::tool_browser_read_page(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        #[cfg(feature = "browser")]
        "browser_close" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser_tools::tool_browser_close(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        #[cfg(feature = "browser")]
        "browser_scroll" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser_tools::tool_browser_scroll(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        #[cfg(feature = "browser")]
        "browser_wait" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser_tools::tool_browser_wait(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        #[cfg(feature = "browser")]
        "browser_run_js" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser_tools::tool_browser_run_js(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        #[cfg(feature = "browser")]
        "browser_back" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser_tools::tool_browser_back(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },

        // Artifact retrieval tool — recovers content spilled to disk by the
        // artifact store when a tool result exceeded `spill_threshold_bytes`.
        "read_artifact" => {
            let artifact_dir = crate::artifact_store::default_artifact_storage_dir();
            tool_read_artifact(input, &artifact_dir).await
        }

        // Canvas / A2UI tool
        "canvas_present" => tool_canvas_present(input, *workspace_root).await,

        other => {
            // Fallback 1: MCP tools (mcp_{server}_{tool} prefix)
            if mcp::is_mcp_tool(other) {
                // SECURITY: Verify MCP tool is in the agent's allowed_tools list.
                if let Some(allowed) = allowed_tools {
                    if !allowed
                        .iter()
                        .any(|pattern| librefang_types::capability::glob_matches(pattern, other))
                    {
                        warn!(tool = other, "MCP tool not in agent's allowed_tools list");
                        return ToolResult {
                            tool_use_id: tool_use_id.to_string(),
                            content: format!(
                                "Permission denied: MCP tool '{other}' is not in the agent's allowed tools list"
                            ),
                            is_error: true,
                            ..Default::default()
                        };
                    }
                }
                if let Some(mcp_conns) = mcp_connections {
                    let mut conns = mcp_conns.lock().await;
                    let server_name =
                        mcp::resolve_mcp_server_from_known(other, conns.iter().map(|c| c.name()))
                            .map(str::to_string);
                    if let Some(server_name) = server_name {
                        if let Some(conn) =
                            conns.iter_mut().find(|c| c.name() == server_name.as_str())
                        {
                            debug!(
                                tool = other,
                                server = server_name,
                                "Dispatching to MCP server"
                            );
                            match conn.call_tool(other, input).await {
                                Ok(content) => Ok(content),
                                Err(e) => Err(format!("MCP tool call failed: {e}")),
                            }
                        } else {
                            Err(format!("MCP server '{server_name}' not connected"))
                        }
                    } else {
                        Err(format!("Invalid MCP tool name: {other}"))
                    }
                } else {
                    Err(format!("MCP not available for tool: {other}"))
                }
            }
            // Fallback 2: Skill registry tool providers
            else if let Some(registry) = skill_registry {
                if let Some(skill) = registry.find_tool_provider(other) {
                    debug!(tool = other, skill = %skill.manifest.skill.name, "Dispatching to skill");
                    let skill_dir = skill.path.clone();
                    let env_policy = kernel.and_then(|k| k.skill_env_passthrough_policy());
                    match librefang_skills::loader::execute_skill_tool(
                        &skill.manifest,
                        &skill.path,
                        other,
                        input,
                        env_policy.as_ref(),
                    )
                    .await
                    {
                        Ok(skill_result) => {
                            let content = serde_json::to_string(&skill_result.output)
                                .unwrap_or_else(|_| skill_result.output.to_string());
                            if skill_result.is_error {
                                Err(content)
                            } else {
                                // Fire-and-forget usage increment on success.
                                tokio::task::spawn_blocking(move || {
                                    if let Err(e) =
                                        librefang_skills::evolution::record_skill_usage(&skill_dir)
                                    {
                                        tracing::debug!(error = %e, dir = %skill_dir.display(), "record_skill_usage failed");
                                    }
                                });
                                Ok(content)
                            }
                        }
                        Err(e) => Err(format!("Skill execution failed: {e}")),
                    }
                } else {
                    Err(format!("Unknown tool: {other}"))
                }
            } else {
                Err(format!("Unknown tool: {other}"))
            }
        }
    };

    match result {
        Ok(content) => ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content,
            is_error: false,
            ..Default::default()
        },
        Err(err) => ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: format!("Error: {err}"),
            is_error: true,
            ..Default::default()
        },
    }
}

/// Execute a tool by name with the given input, returning a ToolResult.
///
/// The optional `kernel` handle enables inter-agent tools. If `None`,
/// agent tools will return an error indicating the kernel is not available.
///
/// `allowed_tools` enforces capability-based security: if provided, only
/// tools in the list may execute. This prevents an LLM from hallucinating
/// tool names outside the agent's capability grants.
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool(
    tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    allowed_tools: Option<&[String]>,
    caller_agent_id: Option<&str>,
    skill_registry: Option<&SkillRegistry>,
    allowed_skills: Option<&[String]>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    allowed_env_vars: Option<&[String]>,
    workspace_root: Option<&Path>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    media_drivers: Option<&crate::media::MediaDriverCache>,
    exec_policy: Option<&librefang_types::config::ExecPolicy>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&librefang_types::config::DockerSandboxConfig>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    process_registry: Option<&crate::process_registry::ProcessRegistry>,
    sender_id: Option<&str>,
    channel: Option<&str>,
    checkpoint_manager: Option<&Arc<crate::checkpoint_manager::CheckpointManager>>,
    interrupt: Option<crate::interrupt::SessionInterrupt>,
    session_id: Option<&str>,
    dangerous_command_checker: Option<
        &Arc<tokio::sync::RwLock<crate::dangerous_command::DangerousCommandChecker>>,
    >,
    available_tools: Option<&[ToolDefinition]>,
) -> ToolResult {
    // Normalize the tool name through compat mappings so LLM-hallucinated aliases
    // (e.g. "fs-write" → "file_write") resolve to the canonical LibreFang name.
    let tool_name = normalize_tool_name(tool_name);

    // Capability enforcement: reject tools not in the allowed list.
    // Entries support wildcard patterns (e.g. "file_*" matches "file_read").
    if let Some(allowed) = allowed_tools {
        if !allowed
            .iter()
            .any(|pattern| librefang_types::capability::glob_matches(pattern, tool_name))
        {
            warn!(tool_name, "Capability denied: tool not in allowed list");
            return ToolResult {
                tool_use_id: tool_use_id.to_string(),
                content: format!(
                    "Permission denied: agent does not have capability to use tool '{tool_name}'"
                ),
                is_error: true,
                ..Default::default()
            };
        }
    }

    let shell_exec_full_mode = tool_name == "shell_exec"
        && exec_policy.is_some_and(|p| p.mode == librefang_types::config::ExecSecurityMode::Full);

    // Parse the session id once. Invalid UUIDs (legacy non-uuid session
    // ids, channel-derived synthetic ids) leave this `None` so the ACP
    // routing in `file_read` / `file_write` falls through to the
    // local-fs path — same effect as not having the field at all.
    //
    // Computed up here (rather than at the `ToolExecContext`
    // construction site below) so the deferred-approval branch can
    // persist the SessionId into `DeferredToolExecution.session_id` —
    // the field threads through v36's `deferred_payload` BLOB so a
    // post-restart `Allow once` rebuilds the same routing context and
    // resumes against the editor's `acp_fs_client` /
    // `acp_terminal_client` instead of silently falling back to local
    // fs / shell (#3313 review, H1).
    let parsed_session_id = session_id
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(librefang_types::agent::SessionId);

    // Approval gate: check if this tool requires human approval before execution.
    // Uses sender/channel context for per-sender trust and channel-specific policies.
    if let Some(kh) = kernel {
        if kh.is_tool_denied_with_context(tool_name, sender_id, channel) {
            warn!(tool_name, channel, "Execution denied by channel policy");
            return ToolResult {
                tool_use_id: tool_use_id.to_string(),
                content: format!(
                    "Execution denied: '{tool_name}' is blocked by the active channel policy."
                ),
                is_error: true,
                ..Default::default()
            };
        }

        // Per-user RBAC gate (RBAC M3, issue #3054 Phase 2). Layered on
        // top of the existing channel deny: an explicit `Deny` here
        // hard-blocks the call; `NeedsApproval` flips the call into
        // approval-required mode regardless of the global require list;
        // `Allow` defers to the existing approval logic.
        let user_gate = kh.resolve_user_tool_decision(tool_name, sender_id, channel);
        let force_approval = match &user_gate {
            librefang_types::user_policy::UserToolGate::Allow => false,
            librefang_types::user_policy::UserToolGate::Deny { reason } => {
                warn!(tool_name, channel, %reason, "Execution denied by per-user policy");
                return ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content: format!("Execution denied: {reason}"),
                    is_error: true,
                    ..Default::default()
                };
            }
            librefang_types::user_policy::UserToolGate::NeedsApproval { reason } => {
                debug!(tool_name, %reason, "Per-user policy escalating to approval");
                true
            }
        };

        // SECURITY: the shell-Full bypass only applies to the global
        // `require_approval` list — a user-policy `NeedsApproval` MUST
        // still route through the approval queue. Without `!force_approval`
        // here, a user whose RBAC policy demanded approval would have the
        // call execute directly under Full mode, defeating Phase-2.
        let skip_approval_for_full_exec = shell_exec_full_mode && !force_approval;

        if !skip_approval_for_full_exec
            && (force_approval || kh.requires_approval_with_context(tool_name, sender_id, channel))
        {
            let agent_id_str = caller_agent_id.unwrap_or("unknown");
            let input_str = input.to_string();
            let summary = format!(
                "{}: {}",
                tool_name,
                librefang_types::truncate_str(&input_str, 200)
            );
            let deferred_allowed_env_vars =
                allowed_env_vars.map(|vars| vars.to_vec()).or_else(|| {
                    exec_policy.and_then(|policy| {
                        if policy.allowed_env_vars.is_empty() {
                            None
                        } else {
                            Some(policy.allowed_env_vars.clone())
                        }
                    })
                });
            let deferred = librefang_types::tool::DeferredToolExecution {
                agent_id: agent_id_str.to_string(),
                tool_use_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                input: input.clone(),
                allowed_tools: allowed_tools.map(|a| a.to_vec()),
                allowed_env_vars: deferred_allowed_env_vars,
                exec_policy: exec_policy.cloned(),
                sender_id: sender_id.map(|s| s.to_string()),
                channel: channel.map(|c| c.to_string()),
                workspace_root: workspace_root.map(|p| p.to_path_buf()),
                // When the user gate demanded approval, hand-tagged agents
                // must NOT auto-approve — see kernel `submit_tool_approval`.
                force_human: force_approval,
                // Persist the SessionId into the v36 deferred_payload
                // so a post-restart `Allow once` re-binds to the same
                // editor's `acp_fs_client` / `acp_terminal_client`
                // (#3313 review, H1). `None` for non-UUID session
                // strings or non-session contexts — same fallback as
                // the live path. `SessionId: Copy`, no clone needed.
                session_id: parsed_session_id,
            };
            match kh
                .submit_tool_approval(agent_id_str, tool_name, &summary, deferred, session_id)
                .await
            {
                Ok(librefang_types::tool::ToolApprovalSubmission::Pending { request_id }) => {
                    return ToolResult::waiting_approval(
                        tool_use_id.to_string(),
                        request_id.to_string(),
                        tool_name.to_string(),
                    );
                }
                Ok(librefang_types::tool::ToolApprovalSubmission::AutoApproved) => {
                    // Hand agents are auto-approved — fall through to execute_tool_raw
                    debug!(
                        tool_name,
                        "Auto-approved for hand agent — proceeding with execution"
                    );
                }
                Err(e) => {
                    warn!(tool_name, error = %e, "Approval system error");
                    return ToolResult::error(
                        tool_use_id.to_string(),
                        format!("Approval system error: {e}"),
                    );
                }
            }
        }
    }

    // Check for truncated tool call arguments from the LLM driver (#2027).
    // When the LLM's response is cut off mid-JSON (max_tokens exceeded), the
    // driver marks the input with __args_truncated. Return a helpful error
    // so the LLM can retry with smaller content.
    if input
        .get(crate::drivers::openai::TRUNCATED_ARGS_KEY)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let error_msg = input["__error"].as_str().unwrap_or(
            "Tool call arguments were truncated. Try smaller content or split into multiple calls.",
        );
        return ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: error_msg.to_string(),
            is_error: true,
            ..Default::default()
        };
    }

    debug!(tool_name, "Executing tool");
    // `parsed_session_id` is computed once at the top of this fn so
    // both the deferred-approval payload (v36 H1 fix) and this
    // ToolExecContext below see the same SessionId.
    let ctx = ToolExecContext {
        kernel,
        allowed_tools,
        available_tools,
        caller_agent_id,
        skill_registry,
        allowed_skills,
        mcp_connections,
        web_ctx,
        browser_ctx,
        allowed_env_vars,
        workspace_root,
        media_engine,
        media_drivers,
        exec_policy,
        tts_engine,
        docker_config,
        process_manager,
        process_registry,
        sender_id,
        channel,
        session_id: parsed_session_id,
        spill_threshold_bytes: 0,
        max_artifact_bytes: 0,
        checkpoint_manager,
        interrupt,
        dangerous_command_checker,
    };
    execute_tool_raw(tool_use_id, tool_name, input, &ctx).await
}
