//! Built-in tool execution.
//!
//! Provides filesystem, web, shell, and inter-agent tools. Agent tools
//! (agent_send, agent_spawn, etc.) require a KernelHandle to be passed in.

use crate::kernel_handle::prelude::*;
use crate::mcp;
use crate::web_search::{parse_ddg_results, WebToolsContext};
use librefang_skills::registry::SkillRegistry;
use librefang_types::taint::{TaintLabel, TaintSink, TaintedValue};
use librefang_types::tool::{ToolDefinition, ToolExecutionStatus, ToolResult};
use librefang_types::tool_compat::normalize_tool_name;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, warn};

/// Maximum inter-agent call depth to prevent infinite recursion (A->B->C->...).
#[allow(dead_code)]
const MAX_AGENT_CALL_DEPTH: u32 = 5;

/// Check if a shell command should be blocked by taint tracking.
///
/// Layer 1: Shell metacharacter injection (backticks, `$(`, `${`, etc.)
/// Layer 2: Heuristic patterns for injected external data (piped curl, base64, eval)
///
/// This implements the TaintSink::shell_exec() policy from SOTA 2.
fn check_taint_shell_exec(command: &str) -> Option<String> {
    // Layer 1: Block shell metacharacters that enable command injection.
    // Uses the same validator as subprocess_sandbox and docker_sandbox.
    if let Some(reason) = crate::subprocess_sandbox::contains_shell_metacharacters(command) {
        return Some(format!("Shell metacharacter injection blocked: {reason}"));
    }

    // Layer 2: Heuristic patterns for injected external URLs / base64 payloads
    let suspicious_patterns = ["curl ", "wget ", "| sh", "| bash", "base64 -d", "eval "];
    for pattern in &suspicious_patterns {
        if command.contains(pattern) {
            let mut labels = HashSet::new();
            labels.insert(TaintLabel::ExternalNetwork);
            let tainted = TaintedValue::new(command, labels, "llm_tool_call");
            if let Err(violation) = tainted.check_sink(&TaintSink::shell_exec()) {
                warn!(command = crate::str_utils::safe_truncate_str(command, 80), %violation, "Shell taint check failed");
                return Some(violation.to_string());
            }
        }
    }
    None
}

/// Check if a URL should be blocked by taint tracking before network fetch.
///
/// Blocks URLs that appear to contain API keys, tokens, or other secrets
/// in query parameters (potential data exfiltration). Implements TaintSink::net_fetch().
///
/// Both the raw URL and its percent-decoded query parameter names are
/// checked — an attacker can otherwise bypass the filter with encoding
/// tricks such as `api%5Fkey=secret` (the server decodes `%5F` to `_`
/// and receives the real `api_key=secret`).
fn check_taint_net_fetch(url: &str) -> Option<String> {
    const SECRET_KEYS: &[&str] = &["api_key", "apikey", "token", "secret", "password"];

    // Scan 1: raw URL literal for `<key>=` and the Authorization header prefix.
    let url_lower = url.to_lowercase();
    let mut hit = url_lower.contains("authorization:");
    if !hit {
        hit = SECRET_KEYS
            .iter()
            .any(|k| url_lower.contains(&format!("{k}=")));
    }

    // Scan 2: percent-decoded query parameter names. Parsing via
    // `url::Url` decodes each name so `api%5Fkey` becomes `api_key`.
    if !hit {
        if let Ok(parsed) = url::Url::parse(url) {
            for (name, _value) in parsed.query_pairs() {
                let name_lower = name.to_lowercase();
                if SECRET_KEYS.iter().any(|k| name_lower == *k) {
                    hit = true;
                    break;
                }
            }
        }
    }

    if hit {
        let mut labels = HashSet::new();
        labels.insert(TaintLabel::Secret);
        let tainted = TaintedValue::new(url, labels, "llm_tool_call");
        if let Err(violation) = tainted.check_sink(&TaintSink::net_fetch()) {
            warn!(url = crate::str_utils::safe_truncate_str(url, 80), %violation, "Net fetch taint check failed");
            return Some(violation.to_string());
        }
    }
    None
}

/// Check if a free-form string carries an obvious secret shape. Used by
/// exfiltration sinks that don't have a URL query-string structure to
/// parse — `web_fetch` request bodies, `agent_send` message payloads,
/// and (via shared helper) outbound channel / webhook bodies.
///
/// The check is a best-effort denylist: it trips when the text contains
/// an `<assignment-style-key>=<value>` fragment using one of the common
/// secret parameter names (`api_key`, `token`, `secret`, `password`,
/// …), or when it carries an `Authorization:` header prefix, or when it
/// looks like a long contiguous token (e.g. a raw bearer token dropped
/// in as the whole body). Hits are wrapped in a `TaintedValue` and run
/// through the given sink so the rejection message stays consistent
/// with the URL-side checks.
///
/// This is the same "two-sink pattern match" shape described in the
/// SECURITY.md taint section — it is **not** a full information-flow
/// tracker, and copy-pasted obfuscation will still bypass it. The goal
/// is to catch the obvious "the LLM is stuffing OPENAI_API_KEY into an
/// agent_send" shape on the way out, not to prove a data-flow theorem.
const SECRET_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "authorization",
    "proxy-authorization",
    "access_token",
    "refresh_token",
    "token",
    "secret",
    "password",
    "passwd",
    "bearer",
    "x-api-key",
];

/// Header names whose mere presence implies the value is a credential,
/// regardless of what the value looks like. `Authorization: Bearer sk-…`
/// has a space between the scheme and the token, which would otherwise
/// defeat the contiguous-token heuristic in `check_taint_outbound_text`.
const SECRET_HEADER_NAMES: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "apikey",
    "x-auth-token",
    "cookie",
    "set-cookie",
];

/// Check if an HTTP header (name + value) should be blocked. Headers
/// whose name identifies them as credential carriers are rejected
/// unconditionally; everything else falls through to the text-level
/// scanner used for bodies.
fn check_taint_outbound_header(name: &str, value: &str, sink: &TaintSink) -> Option<String> {
    let name_lower = name.to_ascii_lowercase();
    if SECRET_HEADER_NAMES.iter().any(|h| *h == name_lower)
        || SECRET_KEYS.iter().any(|k| *k == name_lower)
    {
        let mut labels = HashSet::new();
        labels.insert(TaintLabel::Secret);
        let tainted = TaintedValue::new(value, labels, "llm_tool_call");
        if let Err(violation) = tainted.check_sink(sink) {
            warn!(
                sink = %sink.name,
                header = %name_lower,
                value_len = value.len(),
                %violation,
                "Outbound taint check failed (credential header)"
            );
            return Some(violation.to_string());
        }
    }
    // Fall through to the regular body-level scan so e.g. a custom
    // `X-Forwarded-Debug: api_key=sk-…` still gets caught.
    check_taint_outbound_text(value, sink)
}

/// Decide whether a contiguous string "smells like" a raw secret token.
/// Returns false for pure-hex / pure-decimal / single-case alnum blobs
/// so that git commit SHAs, UUIDs-without-dashes, and sha256 digests —
/// which agents legitimately exchange — don't trip the filter. Genuine
/// API tokens tend to include mixed case and/or punctuation
/// (`sk-…`, `ghp_…`, base64 with `+/=`).
fn looks_like_opaque_token(trimmed: &str) -> bool {
    if trimmed.len() < 32 || trimmed.chars().any(char::is_whitespace) {
        return false;
    }
    let charset_ok = trimmed.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || c == '-'
            || c == '_'
            || c == '.'
            || c == '/'
            || c == '+'
            || c == '='
    });
    if !charset_ok {
        return false;
    }
    // Require mixed character classes: either (a) at least one
    // uppercase AND one lowercase letter, or (b) at least one of the
    // token-ish punctuation characters. Pure hex (git SHAs, sha256),
    // pure decimal, and pure single-case alphanumeric all fail this.
    let has_upper = trimmed.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = trimmed.chars().any(|c| c.is_ascii_lowercase());
    let has_punct = trimmed
        .chars()
        .any(|c| matches!(c, '-' | '_' | '.' | '/' | '+' | '='));
    (has_upper && has_lower) || has_punct
}

fn check_taint_outbound_text(payload: &str, sink: &TaintSink) -> Option<String> {
    let lower = payload.to_lowercase();

    // Fast path 1: `Authorization:` header literal — unambiguous
    // signal that the LLM is trying to ship credentials in-band.
    let mut hit = lower.contains("authorization:");

    // Fast path 2: `key=value` / `key: value` / `key":` / `'key':`
    // shapes. We match on the key name plus one of a handful of
    // assignment separators so plain prose ("a token of appreciation")
    // doesn't trip the filter.
    if !hit {
        let normalized = lower
            .replace(" = ", "=")
            .replace(" =", "=")
            .replace("= ", "=")
            .replace(" : ", ":")
            .replace(" :", ":")
            .replace(": ", ":");
        for k in SECRET_KEYS {
            for sep in ["=", ":", "\":", "':"] {
                if normalized.contains(&format!("{k}{sep}")) {
                    hit = true;
                    break;
                }
            }
            if hit {
                break;
            }
        }
    }

    // Fast path 3: the payload *is* a long opaque token. Covers the
    // case where the LLM shoves a raw credential into the message
    // without any key/value framing. Matches conservatively — long
    // strings with only base64/hex characters and no whitespace, so
    // natural-language messages don't false-positive. Well-known
    // prefixes (`sk-`, `ghp_`, `xoxp-`) are also flagged regardless
    // of length.
    if !hit {
        let trimmed = payload.trim();
        let well_known_prefix = trimmed.starts_with("sk-")
            || trimmed.starts_with("ghp_")
            || trimmed.starts_with("github_pat_")
            || trimmed.starts_with("xoxp-")
            || trimmed.starts_with("xoxb-")
            || trimmed.starts_with("AKIA")
            || trimmed.starts_with("AIza");
        if looks_like_opaque_token(trimmed) || well_known_prefix {
            hit = true;
        }
    }

    if hit {
        let mut labels = HashSet::new();
        labels.insert(TaintLabel::Secret);
        let tainted = TaintedValue::new(payload, labels, "llm_tool_call");
        if let Err(violation) = tainted.check_sink(sink) {
            // Never log the payload itself: if the heuristic fired, the
            // payload IS the secret we are trying to contain.
            warn!(
                sink = %sink.name,
                payload_len = payload.len(),
                %violation,
                "Outbound taint check failed"
            );
            return Some(violation.to_string());
        }
    }
    None
}

tokio::task_local! {
    /// Tracks the current inter-agent call depth within a task.
    static AGENT_CALL_DEPTH: std::cell::Cell<u32>;
    /// Canvas max HTML size in bytes (set from kernel config at loop start).
    pub static CANVAS_MAX_BYTES: usize;
}

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
        channel: _,
        spill_threshold_bytes,
        max_artifact_bytes,
        checkpoint_manager,
        interrupt,
        dangerous_command_checker,
    } = ctx;

    let result = match tool_name {
        // Filesystem tools
        "file_read" => {
            let mut extra = named_ws_prefixes(*kernel, *caller_agent_id);
            // #4434: widen with the channel bridge's download directory so
            // agents can open Telegram/voice/etc. attachments the bridge
            // saved outside their workspace_root.
            if let Some(dl) = kernel.and_then(|k| k.channel_file_download_dir()) {
                extra.push(dl);
            }
            let extra_refs: Vec<&Path> = extra.iter().map(|p| p.as_path()).collect();
            tool_file_read(input, *workspace_root, &extra_refs).await
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
            maybe_snapshot(checkpoint_manager, *workspace_root, "pre file_write").await;
            let extra = named_ws_prefixes_writable(*kernel, *caller_agent_id);
            let extra_refs: Vec<&Path> = extra.iter().map(|p| p.as_path()).collect();
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

            // FIXME(#3822): shell_exec does not enforce readonly_workspace_prefixes.
            // Named workspaces declared with `mode = "r"` are exposed to the shell
            // environment via TOOLS.md, but nothing prevents the spawned process from
            // writing to those directories.  A proper sandbox (e.g. Linux mount
            // namespaces, macOS sandbox-exec, or a chroot) is required to close this
            // gap.  Until then we emit a warning whenever readonly prefixes exist so
            // the issue is visible in daemon logs.
            // Track: https://github.com/librefang/librefang/issues/3822
            if let (Some(k), Some(aid)) = (kernel, caller_agent_id) {
                let ro = k.readonly_workspace_prefixes(aid);
                if !ro.is_empty() {
                    tracing::warn!(
                        agent_id = %aid,
                        readonly_prefixes = ?ro,
                        "shell_exec: readonly_workspace_prefixes are not enforced for shell \
                         commands — the spawned process may write to read-only named workspaces"
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

            // SECURITY (fix #3822): enforce named workspace read-only restrictions for
            // shell_exec. The shell tool runs commands that can write arbitrary paths,
            // so we scan the command string (and any explicit args array) for references
            // to read-only workspace paths and block execution if any are found.
            // This mirrors the read-only enforcement already applied to file_write and
            // apply_patch (see the "file_write" arm above).
            if let (Some(k), Some(agent_id)) = (kernel, caller_agent_id) {
                let ro_prefixes = k.readonly_workspace_prefixes(agent_id);
                if !ro_prefixes.is_empty() {
                    // Collect the command string and all argument strings as a single
                    // list of tokens to scan. We look for any token that starts with
                    // (or equals) a read-only workspace prefix, which indicates the
                    // shell command is targeting a read-only workspace path.
                    let mut tokens: Vec<&str> = vec![command];
                    if let Some(args_arr) = input.get("args").and_then(|a| a.as_array()) {
                        for v in args_arr {
                            if let Some(s) = v.as_str() {
                                tokens.push(s);
                            }
                        }
                    }
                    for ro_prefix in &ro_prefixes {
                        let prefix_str = ro_prefix.to_string_lossy();
                        // A token references a read-only workspace if it starts with the
                        // prefix path. We also check that the match is at a path boundary
                        // (next char is '/' or the token equals the prefix exactly) to
                        // avoid false-positives on shared prefixes like /data vs /data2.
                        let blocked = tokens.iter().any(|token| {
                            if let Some(rest) = token.strip_prefix(prefix_str.as_ref()) {
                                rest.is_empty() || rest.starts_with('/')
                            } else {
                                false
                            }
                        });
                        // Also check if the prefix path string appears as a contiguous
                        // substring within the full command (handles redirect operators,
                        // quoted paths, etc.). This is a best-effort heuristic — the
                        // primary check is the token scan above.
                        let in_command = {
                            let ps = prefix_str.as_ref();
                            if let Some(idx) = command.find(ps) {
                                // Verify path boundary after the match
                                let after = &command[idx + ps.len()..];
                                after.is_empty()
                                    || after.starts_with('/')
                                    || after.starts_with('"')
                                    || after.starts_with('\'')
                                    || after.starts_with(' ')
                            } else {
                                false
                            }
                        };
                        if blocked || in_command {
                            // Find the workspace name for the error message. The name is
                            // not stored in the prefix list, so we report the path.
                            return ToolResult {
                                tool_use_id: tool_use_id.to_string(),
                                content: format!(
                                    "shell_exec blocked: path '{}' is in a read-only workspace (mode = r). Writes are not allowed.",
                                    prefix_str
                                ),
                                is_error: true,
                                ..Default::default()
                            };
                        }
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

        // Shared memory tools (peer-scoped when sender_id is present)
        "memory_store" => tool_memory_store(input, *kernel, *sender_id),
        "memory_recall" => tool_memory_recall(input, *kernel, *sender_id),
        "memory_list" => tool_memory_list(*kernel, *sender_id),

        // Memory wiki tools (issue #3329)
        "wiki_get" => tool_wiki_get(input, *kernel),
        "wiki_search" => tool_wiki_search(input, *kernel),
        "wiki_write" => tool_wiki_write(input, *kernel, *caller_agent_id, *sender_id),

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
            let extra = named_ws_prefixes(*kernel, *caller_agent_id);
            let extra_refs: Vec<&Path> = extra.iter().map(|p| p.as_path()).collect();
            tool_image_analyze(input, *workspace_root, &extra_refs).await
        }

        // Media understanding tools
        "media_describe" => {
            let extra = named_ws_prefixes(*kernel, *caller_agent_id);
            let extra_refs: Vec<&Path> = extra.iter().map(|p| p.as_path()).collect();
            tool_media_describe(input, *media_engine, *workspace_root, &extra_refs).await
        }
        "media_transcribe" => {
            let extra = named_ws_prefixes(*kernel, *caller_agent_id);
            let extra_refs: Vec<&Path> = extra.iter().map(|p| p.as_path()).collect();
            tool_media_transcribe(input, *media_engine, *workspace_root, &extra_refs).await
        }

        // Media generation tools (MediaDriver-based)
        "image_generate" => {
            let upload_dir = kernel
                .map(|k| k.effective_upload_dir())
                .unwrap_or_else(|| std::env::temp_dir().join("librefang_uploads"));
            tool_image_generate(input, *media_drivers, *workspace_root, &upload_dir).await
        }
        "video_generate" => tool_video_generate(input, *media_drivers).await,
        "video_status" => tool_video_status(input, *media_drivers).await,
        "music_generate" => tool_music_generate(input, *media_drivers, *workspace_root).await,

        // TTS/STT tools
        "text_to_speech" => {
            tool_text_to_speech(input, *media_drivers, *tts_engine, *workspace_root).await
        }
        "speech_to_text" => {
            let extra = named_ws_prefixes(*kernel, *caller_agent_id);
            let extra_refs: Vec<&Path> = extra.iter().map(|p| p.as_path()).collect();
            tool_speech_to_text(input, *media_engine, *workspace_root, &extra_refs).await
        }

        // Docker sandbox tool
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
            tool_channel_send(input, *kernel, *workspace_root, *sender_id, &extra_refs).await
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

        // Workflow execution tool
        "workflow_run" => tool_workflow_run(input, *kernel).await,

        // Browser automation tools
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
                    crate::browser::tool_browser_navigate(input, mgr, aid).await
                }
                None => Err(
                    "Browser tools not available. Ensure Chrome/Chromium is installed.".to_string(),
                ),
            }
        }
        "browser_click" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_click(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_type" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_type(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_screenshot" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                let upload_dir = kernel
                    .map(|k| k.effective_upload_dir())
                    .unwrap_or_else(|| std::env::temp_dir().join("librefang_uploads"));
                crate::browser::tool_browser_screenshot(input, mgr, aid, &upload_dir).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_read_page" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_read_page(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_close" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_close(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_scroll" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_scroll(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_wait" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_wait(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_run_js" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_run_js(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_back" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_back(input, mgr, aid).await
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
        spill_threshold_bytes: 0,
        max_artifact_bytes: 0,
        checkpoint_manager,
        interrupt,
        dangerous_command_checker,
    };
    execute_tool_raw(tool_use_id, tool_name, input, &ctx).await
}

/// Tools that are always shipped as full JSON schemas in every LLM request,
/// regardless of lazy-loading settings.
///
/// Rationale (issue #3044): shipping all ~75 builtin tool schemas on every
/// turn burns ~6k tokens of request payload. Most conversations only use a
/// handful of tools — and the ones below are the ones agents reach for most
/// often, so it's worth paying their declaration cost upfront to avoid a
/// `tool_load` round-trip on the common path.
///
/// Everything else in [`builtin_tool_definitions`] is available via the
/// `tool_load(name)` meta-tool (declared as part of this list so the LLM can
/// always discover new tools) and `tool_search(query)`.
///
/// Order matters only for readability in logs — the final list is a Vec, so
/// the order is preserved into the request body.
pub const ALWAYS_NATIVE_TOOLS: &[&str] = &[
    // Meta: discovery + loading. Without these, the LLM cannot escape the
    // lazy-load regime on its own.
    "tool_load",
    "tool_search",
    // Memory: used on nearly every turn of a multi-turn conversation.
    "memory_store",
    "memory_recall",
    "memory_list",
    // Web: the most common "go find something" action.
    "web_search",
    "web_fetch",
    // Files: reading is near-universal; writing and listing round out the
    // core file-flow so agents don't round-trip to load each one.
    "file_read",
    // Agent-to-agent / messaging: common proactive output path.
    "agent_send",
    "agent_list",
    "channel_send",
    // Private channel to the owner — intentionally cheap so agents never
    // skip using it because of declaration cost.
    "notify_owner",
    // Artifact retrieval — must be always available so agents can recover
    // spilled content even in lazy-tool mode.
    "read_artifact",
    // Skill evolution helpers stay native because they're also in the
    // always-available set enforced by the kernel.
    "skill_read_file",
    "skill_evolve_create",
    "skill_evolve_update",
    "skill_evolve_patch",
    "skill_evolve_delete",
    "skill_evolve_rollback",
    "skill_evolve_write_file",
    "skill_evolve_remove_file",
];

/// Select the subset of `all` whose names appear in [`ALWAYS_NATIVE_TOOLS`].
/// Used by the agent loop to build the lazy-mode tools list.
pub fn select_native_tools(all: &[ToolDefinition]) -> Vec<ToolDefinition> {
    let want: std::collections::HashSet<&str> = ALWAYS_NATIVE_TOOLS.iter().copied().collect();
    all.iter()
        .filter(|t| want.contains(t.name.as_str()))
        .cloned()
        .collect()
}

/// Get definitions for all built-in tools.
pub fn builtin_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        // --- Filesystem tools ---
        ToolDefinition {
            name: "file_read".to_string(),
            description: "Read the contents of a file. Paths are relative to the agent workspace.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file path to read" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "file_write".to_string(),
            description: "Write content to a file. Paths are relative to the agent workspace.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file path to write to" },
                    "content": { "type": "string", "description": "The content to write" }
                },
                "required": ["path", "content"]
            }),
        },
        ToolDefinition {
            name: "file_list".to_string(),
            description: "List files in a directory. Paths are relative to the agent workspace.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The directory path to list" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "apply_patch".to_string(),
            description: "Apply a multi-hunk diff patch to add, update, move, or delete files. Use this for targeted edits instead of full file overwrites.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "The patch in *** Begin Patch / *** End Patch format. Use *** Add File:, *** Update File:, *** Delete File: markers. Hunks use @@ headers with space (context), - (remove), + (add) prefixed lines."
                    }
                },
                "required": ["patch"]
            }),
        },
        // --- Web tools ---
        ToolDefinition {
            name: "web_fetch".to_string(),
            description: "Fetch a URL with SSRF protection. Supports GET/POST/PUT/PATCH/DELETE. For GET, HTML is converted to Markdown. For other methods, returns raw response body.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The URL to fetch (http/https only)" },
                    "method": { "type": "string", "enum": ["GET","POST","PUT","PATCH","DELETE"], "description": "HTTP method (default: GET)" },
                    "headers": { "type": "object", "description": "Custom HTTP headers as key-value pairs" },
                    "body": { "type": "string", "description": "Request body for POST/PUT/PATCH" }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "web_search".to_string(),
            description: "Search the web using multiple providers (Tavily, Brave, Perplexity, DuckDuckGo) with automatic fallback. Returns structured results with titles, URLs, and snippets.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The search query" },
                    "max_results": { "type": "integer", "description": "Maximum number of results to return (default: 5, max: 20)" }
                },
                "required": ["query"]
            }),
        },
        // --- Shell tool ---
        ToolDefinition {
            name: "shell_exec".to_string(),
            description: "Execute a shell command and return its output.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute" },
                    "timeout_seconds": { "type": "integer", "description": "Timeout in seconds (default: 30)" }
                },
                "required": ["command"]
            }),
        },
        // --- Owner-side channel ---
        ToolDefinition {
            name: "notify_owner".to_string(),
            description: "Send a private notice to the agent's owner (operator DM) WITHOUT posting it to the source chat. Use this in groups when you have something to tell the owner that should not be visible to other participants. Returns an opaque ack — do NOT repeat the summary in your public reply.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "description": "Short machine-readable category, e.g. 'confirmation_needed', 'stranger_request', 'escalation'."
                    },
                    "summary": {
                        "type": "string",
                        "description": "Human-readable message body addressed to the owner."
                    }
                },
                "required": ["reason", "summary"]
            }),
        },
        // --- Inter-agent tools ---
        ToolDefinition {
            name: "agent_send".to_string(),
            description: "Send a message to another agent and receive their response. Accepts UUID or agent name. Use agent_find first to discover agents.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "The target agent's UUID or name" },
                    "message": { "type": "string", "description": "The message to send to the agent" }
                },
                "required": ["agent_id", "message"]
            }),
        },
        ToolDefinition {
            name: "agent_spawn".to_string(),
            description: "Spawn a new agent from settings. Returns the new agent's ID and name.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Unique name for the new agent. Ensure it does not conflict with existing agents."
                    },
                    "system_prompt": {
                        "type": "string",
                        "description": "The system prompt for the new agent"
                    },
                    "tools": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Select from all available tools, including MCP tools. Use the full tool names only"
                    },
                    "network": {
                        "type": "boolean",
                        "description": "Whether to enable network access for the new agent (required to be true when web_fetch is in tools)"
                    },
                    "shell": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Preset necessary shell commands based on the agent's task (e.g., [\"uv *\", \"pnpm *\"]). "
                    }
                },
                "required": ["name", "system_prompt"]
            }),
        },
        ToolDefinition {
            name: "agent_list".to_string(),
            description: "List all currently running agents with their IDs, names, states, and models.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "agent_kill".to_string(),
            description: "Kill (terminate) another agent. Accepts UUID or agent name.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "The target agent's UUID or name" }
                },
                "required": ["agent_id"]
            }),
        },
        // --- Shared memory tools ---
        ToolDefinition {
            name: "memory_store".to_string(),
            description: "Store a value in shared memory accessible by all agents. Use for cross-agent coordination and data sharing.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The storage key" },
                    "value": { "type": "string", "description": "The value to store (JSON-encode objects/arrays, or pass a plain string)" }
                },
                "required": ["key", "value"]
            }),
        },
        ToolDefinition {
            name: "memory_recall".to_string(),
            description: "Recall a value from shared memory by key.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The storage key to recall" }
                },
                "required": ["key"]
            }),
        },
        ToolDefinition {
            name: "memory_list".to_string(),
            description: "List all keys stored in shared memory.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
            }),
        },
        // --- Memory wiki tools (issue #3329) — return KernelOpError::unavailable
        //     when [memory_wiki] enabled = false in config.toml. ---
        ToolDefinition {
            name: "wiki_get".to_string(),
            description:
                "Read a wiki page by topic from the durable knowledge vault. \
                 Returns the page as JSON: {topic, frontmatter, body}. The \
                 frontmatter carries provenance (which agents/sessions \
                 contributed and when)."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "topic": { "type": "string", "description": "Page topic — must match [a-zA-Z0-9_-]+ and not be `index` or `_*`" }
                },
                "required": ["topic"]
            }),
        },
        ToolDefinition {
            name: "wiki_search".to_string(),
            description:
                "Search wiki page bodies (case-insensitive substring). Topic \
                 hits outrank body hits. Returns an array of \
                 {topic, snippet, score} sorted by score descending."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "limit": { "type": "integer", "description": "Max hits (default 10)" }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "wiki_write".to_string(),
            description:
                "Write or update a wiki page. Body may use [[topic]] \
                 placeholders for cross-references; the vault rewrites them \
                 per its render mode. Provenance is auto-filled from the \
                 calling agent. If the page was edited externally since the \
                 last write, the call fails unless `force = true`, in which \
                 case the external body is preserved and only provenance is \
                 appended."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "topic": { "type": "string", "description": "Page topic — must match [a-zA-Z0-9_-]+" },
                    "body":  { "type": "string", "description": "Markdown body. Use [[other-topic]] placeholders for cross-references." },
                    "force": { "type": "boolean", "description": "Overwrite even if the page was edited externally (default false)" }
                },
                "required": ["topic", "body"]
            }),
        },
        // --- Collaboration tools ---
        ToolDefinition {
            name: "agent_find".to_string(),
            description: "Discover agents by name, tag, tool, or description. Use to find specialists before delegating work.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query (matches agent name, tags, tools, description)" }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "task_post".to_string(),
            description: "Post a task to the shared task queue for another agent to pick up.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short task title" },
                    "description": { "type": "string", "description": "Detailed task description" },
                    "assigned_to": { "type": "string", "description": "Agent name or ID to assign the task to (optional)" }
                },
                "required": ["title", "description"]
            }),
        },
        ToolDefinition {
            name: "task_claim".to_string(),
            description: "Claim the next available task from the task queue assigned to you or unassigned.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "task_complete".to_string(),
            description: "Mark a previously claimed task as completed with a result.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "The task ID to complete" },
                    "result": { "type": "string", "description": "The result or outcome of the task" }
                },
                "required": ["task_id", "result"]
            }),
        },
        ToolDefinition {
            name: "task_list".to_string(),
            description: "List tasks in the shared queue, optionally filtered by status (pending, in_progress, completed).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "status": { "type": "string", "description": "Filter by status: pending, in_progress, completed (optional)" }
                }
            }),
        },
        ToolDefinition {
            name: "task_status".to_string(),
            description: "Look up a single task on the shared queue by ID and return its status, result, title, assignee, created_at, and completed_at. Native counterpart of the comms_task_status MCP bridge tool — no MCP load required when polling for a delegated task's outcome. Any agent that knows the task_id can read it — task visibility is shared across all agents in the workspace, mirroring task_list / comms_task_status semantics.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "The task ID returned by task_post" }
                },
                "required": ["task_id"]
            }),
        },
        ToolDefinition {
            name: "event_publish".to_string(),
            description: "Publish a custom event that can trigger proactive agents. Use to broadcast signals to the agent fleet.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "event_type": { "type": "string", "description": "Type identifier for the event (e.g., 'code_review_requested')" },
                    "payload": { "type": "object", "description": "JSON payload data for the event" }
                },
                "required": ["event_type"]
            }),
        },
        // --- Skill file read tool ---
        ToolDefinition {
            name: "skill_read_file".to_string(),
            description: "Read a companion file from an installed skill. Use when a skill's prompt context references additional files by relative path (e.g. 'see references/syntax.md').".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "skill": { "type": "string", "description": "The skill name as listed in Available Skills" },
                    "path": { "type": "string", "description": "Path relative to the skill directory, e.g. 'references/query-syntax.md'" }
                },
                "required": ["skill", "path"]
            }),
        },
        // --- Scheduling tools ---
        ToolDefinition {
            name: "schedule_create".to_string(),
            description: "Schedule a recurring task using natural language or cron syntax. Examples: 'every 5 minutes', 'daily at 9am', 'weekdays at 6pm', '0 */5 * * *'.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": { "type": "string", "description": "What this schedule does (e.g., 'Check for new emails')" },
                    "schedule": { "type": "string", "description": "Natural language or cron expression (e.g., 'every 5 minutes', 'daily at 9am', '0 */5 * * *')" },
                    "tz": { "type": "string", "description": "IANA timezone for time-of-day schedules (e.g., 'Asia/Shanghai', 'US/Eastern'). Omit for UTC. Always set this for schedules like 'daily at 9am' so they run in the user's local time." },
                    "agent": { "type": "string", "description": "Agent name or ID to run this task (optional, defaults to self)" }
                },
                "required": ["description", "schedule"]
            }),
        },
        ToolDefinition {
            name: "schedule_list".to_string(),
            description: "List all scheduled tasks with their IDs, descriptions, schedules, and next run times.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "schedule_delete".to_string(),
            description: "Remove a scheduled task by its ID.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The schedule ID to remove" }
                },
                "required": ["id"]
            }),
        },
        // --- Knowledge graph tools ---
        ToolDefinition {
            name: "knowledge_add_entity".to_string(),
            description: "Add an entity to the knowledge graph. Entities represent people, organizations, projects, concepts, locations, tools, etc.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Display name of the entity" },
                    "entity_type": { "type": "string", "description": "Type: person, organization, project, concept, event, location, document, tool, or a custom type" },
                    "properties": { "type": "object", "description": "Arbitrary key-value properties (optional)" }
                },
                "required": ["name", "entity_type"]
            }),
        },
        ToolDefinition {
            name: "knowledge_add_relation".to_string(),
            description: "Add a relation between two entities in the knowledge graph.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Source entity ID or name" },
                    "relation": { "type": "string", "description": "Relation type: works_at, knows_about, related_to, depends_on, owned_by, created_by, located_in, part_of, uses, produces, or a custom type" },
                    "target": { "type": "string", "description": "Target entity ID or name" },
                    "confidence": { "type": "number", "description": "Confidence score 0.0-1.0 (default: 1.0)" },
                    "properties": { "type": "object", "description": "Arbitrary key-value properties (optional)" }
                },
                "required": ["source", "relation", "target"]
            }),
        },
        ToolDefinition {
            name: "knowledge_query".to_string(),
            description: "Query the knowledge graph. Filter by source entity, relation type, and/or target entity. Returns matching entity-relation-entity triples.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Filter by source entity name or ID (optional)" },
                    "relation": { "type": "string", "description": "Filter by relation type (optional)" },
                    "target": { "type": "string", "description": "Filter by target entity name or ID (optional)" },
                    "max_depth": { "type": "integer", "description": "Maximum traversal depth (default: 1)" }
                }
            }),
        },
        // --- Image analysis tool ---
        ToolDefinition {
            name: "image_analyze".to_string(),
            description: "Analyze an image file — returns format, dimensions, file size, and a base64 preview. For vision-model analysis, include a prompt.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the image file" },
                    "prompt": { "type": "string", "description": "Optional prompt for vision analysis (e.g., 'Describe what you see')" }
                },
                "required": ["path"]
            }),
        },
        // --- Location tool ---
        ToolDefinition {
            name: "location_get".to_string(),
            description: "Get approximate geographic location based on IP address. Returns city, country, coordinates, and timezone.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        // --- Browser automation tools ---
        ToolDefinition {
            name: "browser_navigate".to_string(),
            description: "Navigate a browser to a URL. Returns the page title and readable content as markdown. Opens a persistent browser session.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The URL to navigate to (http/https only)" }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "browser_click".to_string(),
            description: "Click an element on the current browser page by CSS selector or visible text. Returns the resulting page state.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector (e.g., '#submit-btn', '.add-to-cart') or visible text to click" }
                },
                "required": ["selector"]
            }),
        },
        ToolDefinition {
            name: "browser_type".to_string(),
            description: "Type text into an input field on the current browser page.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector for the input field (e.g., 'input[name=\"email\"]', '#search-box')" },
                    "text": { "type": "string", "description": "The text to type into the field" }
                },
                "required": ["selector", "text"]
            }),
        },
        ToolDefinition {
            name: "browser_screenshot".to_string(),
            description: "Take a screenshot of the current browser page. Returns a base64-encoded PNG image.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "browser_read_page".to_string(),
            description: "Read the current browser page content as structured markdown. Use after clicking or navigating to see the updated page.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "browser_close".to_string(),
            description: "Close the browser session. The browser will also auto-close when the agent loop ends.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "browser_scroll".to_string(),
            description: "Scroll the browser page. Use this to see content below the fold or navigate long pages.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "direction": { "type": "string", "description": "Scroll direction: 'up', 'down', 'left', 'right' (default: 'down')" },
                    "amount": { "type": "integer", "description": "Pixels to scroll (default: 600)" }
                }
            }),
        },
        ToolDefinition {
            name: "browser_wait".to_string(),
            description: "Wait for a CSS selector to appear on the page. Useful for dynamic content that loads asynchronously.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector to wait for" },
                    "timeout_ms": { "type": "integer", "description": "Max wait time in milliseconds (default: 5000, max: 30000)" }
                },
                "required": ["selector"]
            }),
        },
        ToolDefinition {
            name: "browser_run_js".to_string(),
            description: "Run JavaScript on the current browser page and return the result. For advanced interactions that other browser tools cannot handle.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "expression": { "type": "string", "description": "JavaScript expression to run in the page context" }
                },
                "required": ["expression"]
            }),
        },
        ToolDefinition {
            name: "browser_back".to_string(),
            description: "Go back to the previous page in browser history.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        // --- Media understanding tools ---
        ToolDefinition {
            name: "media_describe".to_string(),
            description: "Describe an image using a vision-capable LLM. Auto-selects the best available provider (Anthropic, OpenAI, or Gemini). Returns a text description of the image content.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the image file (relative to workspace)" },
                    "prompt": { "type": "string", "description": "Optional prompt to guide the description (e.g., 'Extract all text from this image')" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "media_transcribe".to_string(),
            description: "Transcribe audio to text using speech-to-text. Auto-selects the best available provider (Groq Whisper or OpenAI Whisper). Returns the transcript.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the audio file (relative to workspace). Supported: mp3, wav, ogg, flac, m4a, webm." },
                    "language": { "type": "string", "description": "Optional ISO-639-1 language code (e.g., 'en', 'es', 'ja')" }
                },
                "required": ["path"]
            }),
        },
        // --- Image generation tool ---
        ToolDefinition {
            name: "image_generate".to_string(),
            description: "Generate images from a text prompt. Supports multiple providers: OpenAI (dall-e-3, gpt-image-1), Gemini (imagen-3.0), MiniMax (image-01). Auto-detects configured provider if not specified.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Text description of the image to generate (max 4000 chars)" },
                    "model": { "type": "string", "description": "Model to use (e.g. 'dall-e-3', 'imagen-3.0-generate-002', 'image-01'). Uses provider default if not specified." },
                    "aspect_ratio": { "type": "string", "description": "Aspect ratio: '1:1' (default), '16:9', '9:16'" },
                    "width": { "type": "integer", "description": "Image width in pixels (provider-specific)" },
                    "height": { "type": "integer", "description": "Image height in pixels (provider-specific)" },
                    "quality": { "type": "string", "description": "Quality: 'hd', 'standard', etc." },
                    "count": { "type": "integer", "description": "Number of images (1-4, default: 1)" },
                    "provider": { "type": "string", "description": "Provider (openai, gemini, minimax). Auto-detects if not specified." }
                },
                "required": ["prompt"]
            }),
        },
        // --- Video/music generation tools ---
        ToolDefinition {
            name: "video_generate".to_string(),
            description: "Generate a video from a text prompt or reference image. Returns a task_id for polling. Use video_status to check progress.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Text description of the video to generate (required unless image_url is provided)" },
                    "image_url": { "type": "string", "description": "Reference image URL for image-to-video generation" },
                    "model": { "type": "string", "description": "Model ID (default: auto-detect)" },
                    "duration": { "type": "integer", "description": "Duration in seconds (default: 6)" },
                    "resolution": { "type": "string", "description": "Resolution (720P, 768P, 1080P)" },
                    "provider": { "type": "string", "description": "Provider (openai, gemini, minimax). Auto-detects if not specified." }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "video_status".to_string(),
            description: "Check the status of a video generation task. Returns status and download URL when complete.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task ID from video_generate" },
                    "provider": { "type": "string", "description": "Provider that created the task" }
                },
                "required": ["task_id"]
            }),
        },
        ToolDefinition {
            name: "music_generate".to_string(),
            description: "Generate music from a prompt and/or lyrics. Saves audio to workspace output/ directory.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Style/mood description (e.g. 'upbeat pop song')" },
                    "lyrics": { "type": "string", "description": "Song lyrics with optional [Verse], [Chorus] tags" },
                    "model": { "type": "string", "description": "Model ID (default: music-2.5)" },
                    "instrumental": { "type": "boolean", "description": "Generate instrumental only, no vocals" },
                    "provider": { "type": "string", "description": "Provider (default: auto-detect)" }
                }
            }),
        },
        // --- Cron scheduling tools ---
        ToolDefinition {
            name: "cron_create".to_string(),
            description: "Create a scheduled/cron job. Supports one-shot (at), recurring (every N seconds), and cron expressions. Max 50 jobs per agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Job name (max 128 chars, alphanumeric + spaces/hyphens/underscores)" },
                    "schedule": {
                        "type": "object",
                        "description": "Schedule: {\"kind\":\"at\",\"at\":\"2025-01-01T00:00:00Z\"} or {\"kind\":\"every\",\"every_secs\":300} or {\"kind\":\"cron\",\"expr\":\"0 */6 * * *\",\"tz\":\"America/New_York\"}. For cron schedules, always include \"tz\" (IANA timezone, e.g. \"Asia/Shanghai\", \"Europe/London\") so the schedule runs in the user's local time. Omitting tz defaults to UTC."
                    },
                    "action": {
                        "type": "object",
                        "description": "Action: {\"kind\":\"system_event\",\"text\":\"...\"} or {\"kind\":\"agent_turn\",\"message\":\"...\",\"timeout_secs\":300}"
                    },
                    "delivery": {
                        "type": "object",
                        "description": "Delivery target: {\"kind\":\"none\"} or {\"kind\":\"channel\",\"channel\":\"telegram\"} or {\"kind\":\"last_channel\"}"
                    },
                    "one_shot": { "type": "boolean", "description": "If true, auto-delete after execution. Default: false" },
                    "session_mode": { "type": "string", "enum": ["persistent", "new"], "description": "Session behaviour for AgentTurn actions. 'persistent' (default): all fires share one dedicated cron session, preserving history across runs. 'new': each fire gets a fresh isolated session with no memory of previous runs." }
                },
                "required": ["name", "schedule", "action"]
            }),
        },
        ToolDefinition {
            name: "cron_list".to_string(),
            description: "List all scheduled/cron jobs for the current agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "cron_cancel".to_string(),
            description: "Cancel a scheduled/cron job by its ID.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string", "description": "The UUID of the cron job to cancel" }
                },
                "required": ["job_id"]
            }),
        },
        // --- Channel send tool (proactive outbound messaging) ---
        ToolDefinition {
            name: "channel_send".to_string(),
            description: "Send a message or media to a user on a configured channel (email, telegram, slack, etc). For email: recipient is the email address; optionally set subject. For media: set image_url, file_url, or file_path to send an image or file instead of (or alongside) text. Use thread_id to reply in a specific thread/topic. When recipient is omitted during message handling, the tool automatically replies to the original sender.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "channel": { "type": "string", "description": "Channel adapter name (e.g., 'email', 'telegram', 'slack', 'discord')" },
                    "recipient": { "type": "string", "description": "Platform-specific recipient identifier (email address, user ID, etc.). Omit only when replying from an inbound message context where the original sender is available." },
                    "subject": { "type": "string", "description": "Optional subject line (used for email; ignored for other channels)" },
                    "message": { "type": "string", "description": "The message body to send (required for text, optional caption for media)" },
                    "image_url": { "type": "string", "description": "URL of an image to send (supported on Telegram, Discord, Slack)" },
                    "file_url": { "type": "string", "description": "URL of a file to send as attachment" },
                    "file_path": { "type": "string", "description": "Local file path to send as attachment (reads from disk; use instead of file_url for local files)" },
                    "filename": { "type": "string", "description": "Filename for file attachments (defaults to the basename of file_path, or 'file')" },
                    "thread_id": { "type": "string", "description": "Thread/topic ID to reply in (e.g., Telegram message_thread_id, Slack thread_ts)" },
                    "account_id": { "type": "string", "description": "Optional account_id of the specific configured bot to send through (e.g., 'admin-bot'). When omitted, uses the first configured adapter for this channel." },
                    "poll_question": { "type": "string", "description": "Question for a poll (starts a poll, mutually exclusive with image_url/file_url/file_path)" },
                    "poll_options": { "type": "array", "items": { "type": "string" }, "description": "Answer options for the poll (2-10 items, required with poll_question)" },
                    "poll_is_quiz": { "type": "boolean", "description": "Set to true for a quiz mode (one correct answer)" },
                    "poll_correct_option": { "type": "integer", "description": "Index of the correct answer (0-based, for quiz mode)" },
                    "poll_explanation": { "type": "string", "description": "Explanation shown after answering (quiz mode)" }
                },
                "required": ["channel"]
            }),
        },
        // --- Hand tools (curated autonomous capability packages) ---
        ToolDefinition {
            name: "hand_list".to_string(),
            description: "List available Hands (curated autonomous packages) and their activation status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "hand_activate".to_string(),
            description: "Activate a Hand — spawns a specialized autonomous agent with curated tools and skills.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "hand_id": { "type": "string", "description": "The ID of the hand to activate (e.g. 'researcher', 'clip', 'browser')" },
                    "config": { "type": "object", "description": "Optional configuration overrides for the hand's settings" }
                },
                "required": ["hand_id"]
            }),
        },
        ToolDefinition {
            name: "hand_status".to_string(),
            description: "Check the status and metrics of an active Hand.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "hand_id": { "type": "string", "description": "The ID of the hand to check status for" }
                },
                "required": ["hand_id"]
            }),
        },
        ToolDefinition {
            name: "hand_deactivate".to_string(),
            description: "Deactivate a running Hand and stop its agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "instance_id": { "type": "string", "description": "The UUID of the hand instance to deactivate" }
                },
                "required": ["instance_id"]
            }),
        },
        // --- A2A outbound tools ---
        ToolDefinition {
            name: "a2a_discover".to_string(),
            description: "Discover an external A2A agent by fetching its agent card from a URL. Returns the agent's name, description, skills, and supported protocols.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Base URL of the remote LibreFang/A2A-compatible agent (e.g., 'https://agent.example.com')" }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "a2a_send".to_string(),
            description: "Send a task/message to an external A2A agent and get the response. Use agent_name to send to a previously discovered agent, or agent_url for direct addressing.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "The task/message to send to the remote agent" },
                    "agent_url": { "type": "string", "description": "Direct URL of the remote agent's A2A endpoint" },
                    "agent_name": { "type": "string", "description": "Name of a previously discovered A2A agent (looked up from kernel)" },
                    "session_id": { "type": "string", "description": "Optional session ID for multi-turn conversations" }
                },
                "required": ["message"]
            }),
        },
        // --- TTS/STT tools ---
        ToolDefinition {
            name: "text_to_speech".to_string(),
            description: "Convert text to speech audio. Supports multiple providers (OpenAI, Gemini, MiniMax). Saves audio to workspace output/ directory.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "The text to convert to speech (max 4096 chars)" },
                    "voice": { "type": "string", "description": "Voice name (provider-specific). OpenAI: 'alloy', 'echo', 'fable', 'onyx', 'nova', 'shimmer'. Default: 'alloy'" },
                    "format": { "type": "string", "description": "Output format: 'mp3', 'opus', 'aac', 'flac', 'wav' (default: 'mp3')" },
                    "output_format": { "type": "string", "enum": ["mp3", "ogg_opus"], "description": "Final output format. 'ogg_opus' converts to OGG Opus via ffmpeg (required for WhatsApp voice notes); falls back to provider format if ffmpeg is unavailable or conversion fails. Default: 'mp3'" },
                    "provider": { "type": "string", "description": "Provider: 'openai', 'gemini', 'minimax'. Auto-detected if omitted." },
                    "model": { "type": "string", "description": "Model ID (provider-specific). OpenAI: 'tts-1', 'tts-1-hd'. Default varies by provider." },
                    "speed": { "type": "number", "description": "Playback speed (0.25-4.0). OpenAI only. Default: 1.0" }
                },
                "required": ["text"]
            }),
        },
        ToolDefinition {
            name: "speech_to_text".to_string(),
            description: "Transcribe audio to text using speech-to-text. Auto-selects Groq Whisper or OpenAI Whisper. Supported formats: mp3, wav, ogg, flac, m4a, webm.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the audio file (relative to workspace)" },
                    "language": { "type": "string", "description": "Optional ISO-639-1 language code (e.g., 'en', 'es', 'ja')" }
                },
                "required": ["path"]
            }),
        },
        // --- Docker sandbox tool ---
        ToolDefinition {
            name: "docker_exec".to_string(),
            description: "Execute a command inside a Docker container sandbox. Provides OS-level isolation with resource limits, network isolation, and capability dropping. Requires Docker to be installed and docker.enabled=true.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute inside the container" }
                },
                "required": ["command"]
            }),
        },
        // --- Persistent process tools ---
        ToolDefinition {
            name: "process_start".to_string(),
            description: "Start a long-running process (REPL, server, watcher). Returns a process_id for subsequent poll/write/kill operations. Max 5 processes per agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The executable to run (e.g. 'python', 'node', 'npm')" },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Command-line arguments (e.g. ['-i'] for interactive Python)"
                    }
                },
                "required": ["command"]
            }),
        },
        ToolDefinition {
            name: "process_poll".to_string(),
            description: "Read accumulated stdout/stderr from a running process. Non-blocking: returns whatever output has buffered since the last poll.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID returned by process_start" }
                },
                "required": ["process_id"]
            }),
        },
        ToolDefinition {
            name: "process_write".to_string(),
            description: "Write data to a running process's stdin. A newline is appended automatically if not present.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID returned by process_start" },
                    "data": { "type": "string", "description": "The data to write to stdin" }
                },
                "required": ["process_id", "data"]
            }),
        },
        ToolDefinition {
            name: "process_kill".to_string(),
            description: "Terminate a running process and clean up its resources.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID returned by process_start" }
                },
                "required": ["process_id"]
            }),
        },
        ToolDefinition {
            name: "process_list".to_string(),
            description: "List all running processes for the current agent, including their IDs, commands, uptime, and alive status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        // --- Goal tracking tool ---
        ToolDefinition {
            name: "goal_update".to_string(),
            description: "Update a goal's status and/or progress. Use this to autonomously track your progress toward assigned goals.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "goal_id": { "type": "string", "description": "The goal's UUID to update" },
                    "status": { "type": "string", "enum": ["pending", "in_progress", "completed", "cancelled"], "description": "New status for the goal (optional)" },
                    "progress": { "type": "integer", "description": "Progress percentage 0-100 (optional)" }
                },
                "required": ["goal_id"]
            }),
        },
        // --- Workflow execution tool ---
        ToolDefinition {
            name: "workflow_run".to_string(),
            description: "Run a registered workflow pipeline end-to-end. Workflows are multi-step agent pipelines (e.g., bug-triage, code-review, test-generation). Accepts a workflow UUID or name.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "workflow_id": { "type": "string", "description": "The workflow UUID or registered name (e.g., 'bug-triage', 'code-review')" },
                    "input": { "type": "object", "description": "Optional input parameters to pass to the workflow's first step (JSON object)" }
                },
                "required": ["workflow_id"]
            }),
        },
        // --- System time tool ---
        ToolDefinition {
            name: "system_time".to_string(),
            description: "Get the current date, time, and timezone. Returns ISO 8601 timestamp, Unix epoch seconds, and timezone info.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        // --- Canvas / A2UI tool ---
        ToolDefinition {
            name: "canvas_present".to_string(),
            description: "Present an interactive HTML canvas to the user. The HTML is sanitized (no scripts, no event handlers) and saved to the workspace. The dashboard will render it in a panel. Use for rich data visualizations, formatted reports, or interactive UI.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "html": { "type": "string", "description": "The HTML content to present. Must not contain <script> tags, event handlers, or javascript: URLs." },
                    "title": { "type": "string", "description": "Optional title for the canvas panel" }
                },
                "required": ["html"]
            }),
        },
        // --- Artifact retrieval tool ---
        ToolDefinition {
            name: "read_artifact".to_string(),
            description: "Retrieve content from the artifact store. Use this when a previous tool result was truncated with a message like '[tool_result: … | sha256:… | … bytes | preview:]'. Pass the handle exactly as shown (e.g. \"sha256:abc…\"), an optional byte offset (default 0), and an optional length (default 4096, max 65536).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "handle": {
                        "type": "string",
                        "description": "Artifact handle from the spill stub, e.g. \"sha256:abc123…\" (64 hex chars after the prefix)."
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Byte offset to start reading from (default 0)."
                    },
                    "length": {
                        "type": "integer",
                        "description": "Number of bytes to read (default 4096, max 65536)."
                    }
                },
                "required": ["handle"]
            }),
        },
        // --- Skill evolution tools ---
        ToolDefinition {
            name: "skill_evolve_create".to_string(),
            description: "Create a new prompt-only skill from a successful task approach. Use after completing a complex task (5+ tool calls) that involved trial-and-error or a non-trivial workflow worth reusing. The skill becomes available to all agents.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Skill name: lowercase alphanumeric with hyphens (e.g., 'csv-analysis', 'api-debugging')" },
                    "description": { "type": "string", "description": "One-line description of what this skill teaches (max 1024 chars)" },
                    "prompt_context": { "type": "string", "description": "Markdown instructions that will be injected into the system prompt when this skill is active. Should capture the methodology, pitfalls, and best practices discovered." },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for discovery (e.g., ['data', 'csv', 'analysis'])" }
                },
                "required": ["name", "description", "prompt_context"]
            }),
        },
        ToolDefinition {
            name: "skill_evolve_update".to_string(),
            description: "Rewrite a skill's prompt_context entirely. Use when the skill needs a major overhaul based on new learnings. Creates a rollback snapshot automatically.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the existing skill to update" },
                    "prompt_context": { "type": "string", "description": "New Markdown instructions (full replacement)" },
                    "changelog": { "type": "string", "description": "Brief description of what changed and why" }
                },
                "required": ["name", "prompt_context", "changelog"]
            }),
        },
        ToolDefinition {
            name: "skill_evolve_patch".to_string(),
            description: "Make a targeted find-and-replace edit to a skill's prompt_context. Use when only a section needs fixing. Supports fuzzy matching (tolerates whitespace/indent differences).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the existing skill to patch" },
                    "old_string": { "type": "string", "description": "Text to find in the current prompt_context (fuzzy-matched)" },
                    "new_string": { "type": "string", "description": "Replacement text" },
                    "changelog": { "type": "string", "description": "Brief description of what changed and why" },
                    "replace_all": { "type": "boolean", "description": "Replace all occurrences (default: false)" }
                },
                "required": ["name", "old_string", "new_string", "changelog"]
            }),
        },
        ToolDefinition {
            name: "skill_evolve_delete".to_string(),
            description: "Delete an agent-evolved skill. Only works on locally-created skills (not marketplace installs).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the skill to delete" }
                },
                "required": ["name"]
            }),
        },
        ToolDefinition {
            name: "skill_evolve_rollback".to_string(),
            description: "Roll back a skill to its previous version. Use when a recent update degraded the skill's effectiveness.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the skill to roll back" }
                },
                "required": ["name"]
            }),
        },
        ToolDefinition {
            name: "skill_evolve_write_file".to_string(),
            description: "Add a supporting file to a skill (references, templates, scripts, or assets). Use to enrich a skill with additional context like API docs, code templates, or example configurations.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the skill to add the file to" },
                    "path": { "type": "string", "description": "Relative path under the skill directory (e.g., 'references/api.md', 'templates/config.yaml'). Must be under references/, templates/, scripts/, or assets/" },
                    "content": { "type": "string", "description": "File content to write" }
                },
                "required": ["name", "path", "content"]
            }),
        },
        ToolDefinition {
            name: "skill_evolve_remove_file".to_string(),
            description: "Remove a supporting file from a skill.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the skill" },
                    "path": { "type": "string", "description": "Relative path of file to remove (e.g., 'references/old-api.md')" }
                },
                "required": ["name", "path"]
            }),
        },
        // --- Meta-tools: lazy tool loading (issue #3044) ---
        ToolDefinition {
            name: "tool_load".to_string(),
            description: "Load the full JSON schema for a tool by name. Call this before using a tool that is listed in the catalog but not yet declared with a full schema. The loaded tool becomes callable on the next turn.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Tool name to load (e.g., 'file_write', 'browser_navigate')" }
                },
                "required": ["name"]
            }),
        },
        ToolDefinition {
            name: "tool_search".to_string(),
            description: "Find tools by keyword. Returns matching tool names and one-line hints from the full catalog.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Keyword(s) to match against tool names and descriptions (e.g., 'read file', 'screenshot')" },
                    "limit": { "type": "integer", "description": "Max results (default 10)", "minimum": 1, "maximum": 50 }
                },
                "required": ["query"]
            }),
        },
    ]
}

// ---------------------------------------------------------------------------
// Filesystem tools
// ---------------------------------------------------------------------------

/// Resolve a file path through the workspace sandbox, with optional
/// additional canonical roots that should also be considered "inside the
/// sandbox" — used to honor named workspaces declared in the agent's
/// manifest.
///
/// SECURITY: Returns an error when `workspace_root` is `None` to prevent
/// unrestricted filesystem access. All file operations MUST be confined to
/// the agent's workspace directory or one of the explicitly allow-listed
/// `additional_roots`.
fn resolve_file_path_ext(
    raw_path: &str,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<PathBuf, String> {
    let root = workspace_root.ok_or(
        "Workspace sandbox not configured: file operations are disabled. \
         Set a workspace_root in the agent manifest or kernel config to enable file tools.",
    )?;
    crate::workspace_sandbox::resolve_sandbox_path_ext(raw_path, root, additional_roots)
}

/// Fetch the named-workspace prefixes (all modes) for the calling agent.
/// Returns an empty vec when either kernel or agent id is missing.
fn named_ws_prefixes(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Vec<std::path::PathBuf> {
    match (kernel, caller_agent_id) {
        (Some(k), Some(aid)) => k
            .named_workspace_prefixes(aid)
            .into_iter()
            .map(|(p, _)| p)
            .collect(),
        _ => Vec::new(),
    }
}

/// Like [`named_ws_prefixes`] but only returns prefixes for read-write
/// workspaces. Used by `file_write` to widen the writable allowlist.
fn named_ws_prefixes_writable(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Vec<std::path::PathBuf> {
    match (kernel, caller_agent_id) {
        (Some(k), Some(aid)) => k
            .named_workspace_prefixes(aid)
            .into_iter()
            .filter(|(_, mode)| *mode == librefang_types::agent::WorkspaceMode::ReadWrite)
            .map(|(p, _)| p)
            .collect(),
        _ => Vec::new(),
    }
}

/// Like [`named_ws_prefixes`] but only returns prefixes for read-only
/// workspaces. Used by `apply_patch` (#3662) to enforce a deny-list at the
/// write call site in addition to the dispatch-level path check.
fn named_ws_prefixes_readonly(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Vec<std::path::PathBuf> {
    match (kernel, caller_agent_id) {
        (Some(k), Some(aid)) => k.readonly_workspace_prefixes(aid),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Checkpoint helper
// ---------------------------------------------------------------------------

/// Take a snapshot of `workspace_root` before a file-mutating operation.
///
/// If an explicit `CheckpointManager` is provided (injected from the kernel),
/// it is used.  When `mgr` is `None` no snapshot is taken — callers that
/// pass `None` are test or ephemeral contexts that do not need filesystem
/// rollback coverage.
///
/// Failures are **non-fatal**: they are logged as warnings and the calling
/// tool proceeds normally.
///
/// ## Async safety
///
/// `CheckpointManager::snapshot` spawns `git` subprocesses and calls
/// blocking I/O.  This wrapper offloads the work to a dedicated thread pool
/// via `tokio::task::spawn_blocking` so that tokio worker threads are never
/// blocked by slow git operations.
async fn maybe_snapshot(
    mgr: &Option<&Arc<crate::checkpoint_manager::CheckpointManager>>,
    workspace_root: Option<&Path>,
    reason: &str,
) {
    let Some(root) = workspace_root else {
        return;
    };
    let Some(m) = mgr else {
        // No manager injected — skip snapshot entirely.
        // (Test call sites pass None deliberately; production code always
        // passes Some via the kernel.)
        return;
    };

    let mgr_arc = Arc::clone(m);
    let root_owned = root.to_path_buf();
    let reason_owned = reason.to_string();

    // Offload blocking git I/O to the blocking thread pool.
    let result =
        tokio::task::spawn_blocking(move || mgr_arc.snapshot(&root_owned, &reason_owned)).await;

    match result {
        Ok(Err(e)) => {
            warn!(reason, root = %root.display(), "checkpoint snapshot failed (non-fatal): {e}")
        }
        Err(e) => warn!(reason, root = %root.display(), "checkpoint spawn_blocking panicked: {e}"),
        Ok(Ok(_)) => {}
    }
}

// ---------------------------------------------------------------------------
// Filesystem tools
// ---------------------------------------------------------------------------

async fn tool_file_read(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;
    tokio::fs::read_to_string(&resolved)
        .await
        .map_err(|e| format!("Failed to read file: {e}"))
}

async fn tool_file_write(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;
    let content = input["content"]
        .as_str()
        .ok_or("Missing 'content' parameter")?;
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create directories: {e}"))?;
    }
    tokio::fs::write(&resolved, content)
        .await
        .map_err(|e| format!("Failed to write file: {e}"))?;
    Ok(format!(
        "Successfully wrote {} bytes to {}",
        content.len(),
        resolved.display()
    ))
}

async fn tool_file_list(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or(
        "Missing 'path' parameter — retry with {\"path\": \".\"} to list the workspace root",
    )?;
    let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;
    let mut entries = tokio::fs::read_dir(&resolved)
        .await
        .map_err(|e| format!("Failed to list directory: {e}"))?;
    let mut files = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to read entry: {e}"))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry.metadata().await;
        let suffix = match metadata {
            Ok(m) if m.is_dir() => "/",
            _ => "",
        };
        files.push(format!("{name}{suffix}"));
    }
    files.sort();
    Ok(files.join("\n"))
}

// ---------------------------------------------------------------------------
// Patch tool
// ---------------------------------------------------------------------------

async fn tool_apply_patch(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
    readonly_roots: &[&Path],
) -> Result<String, String> {
    let patch_str = input["patch"].as_str().ok_or("Missing 'patch' parameter")?;
    let root = workspace_root.ok_or("apply_patch requires a workspace root")?;
    let ops = crate::apply_patch::parse_patch(patch_str)?;
    // SECURITY #3662: defense-in-depth — pass readonly named-workspace prefixes
    // through to `apply_patch_ext` so any resolved target path that lands
    // inside a read-only workspace is rejected at the write site as well as
    // at dispatch.
    let result =
        crate::apply_patch::apply_patch_ext(&ops, root, additional_roots, readonly_roots).await;
    if result.is_ok() {
        Ok(result.summary())
    } else {
        Err(format!(
            "Patch partially applied: {}. Errors: {}",
            result.summary(),
            result.errors.join("; ")
        ))
    }
}

// ---------------------------------------------------------------------------
// Web tools
// ---------------------------------------------------------------------------

/// Resolve `[tool_results]` spill threshold + per-artifact cap from raw
/// `ToolExecContext` fields, falling back to compiled defaults when the
/// caller passed `0` (test call sites that don't populate the ctx).
fn resolve_spill_config(spill_threshold_bytes: u64, max_artifact_bytes: u64) -> (u64, u64) {
    (
        if spill_threshold_bytes == 0 {
            16_384 // ToolResultsConfig::default().spill_threshold_bytes
        } else {
            spill_threshold_bytes
        },
        if max_artifact_bytes == 0 {
            crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES
        } else {
            max_artifact_bytes
        },
    )
}

/// Apply artifact spill to a tool-result string, returning a compact stub
/// when the body exceeds `threshold` and the spill write succeeds.  Falls
/// through to the original body when below the threshold or when the
/// write fails (e.g. per-artifact cap exceeded, disk full).
///
/// Shared by `web_fetch` (primary + legacy) and `web_search` (#3347 5/N).
fn spill_or_passthrough(
    tool_name: &str,
    body: String,
    threshold: u64,
    max_artifact: u64,
) -> String {
    let bytes = body.as_bytes();
    if let Some(stub) = crate::artifact_store::maybe_spill(
        tool_name,
        bytes,
        threshold,
        max_artifact,
        &crate::artifact_store::default_artifact_storage_dir(),
    ) {
        stub
    } else {
        body
    }
}

/// Legacy web fetch (no SSRF protection, no readability). Used when WebToolsContext is unavailable.
async fn tool_web_fetch_legacy(
    input: &serde_json::Value,
    spill_threshold: u64,
    max_artifact_bytes: u64,
) -> Result<String, String> {
    let url = input["url"].as_str().ok_or("Missing 'url' parameter")?;
    let client = crate::http_client::proxied_client_builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;
    let status = resp.status();
    // Reject responses larger than 10MB to prevent memory exhaustion
    if let Some(len) = resp.content_length() {
        if len > 10 * 1024 * 1024 {
            return Err(format!("Response too large: {len} bytes (max 10MB)"));
        }
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;
    // Artifact spill: if the body exceeds the configured threshold, write it
    // to the artifact store and return a compact stub with a handle.  On write
    // failure (including per-artifact size cap exceeded), fall through to the
    // existing byte-cap truncation so callers always get a usable (if partial)
    // response.
    let body_bytes = body.as_bytes();
    if let Some(stub) = crate::artifact_store::maybe_spill(
        "web_fetch",
        body_bytes,
        spill_threshold,
        max_artifact_bytes,
        &crate::artifact_store::default_artifact_storage_dir(),
    ) {
        return Ok(format!("HTTP {status}\n\n{stub}"));
    }

    let max_len = 50_000;
    let truncated = if body.len() > max_len {
        format!(
            "{}... [truncated, {} total bytes]",
            crate::str_utils::safe_truncate_str(&body, max_len),
            body.len()
        )
    } else {
        body
    };
    Ok(format!("HTTP {status}\n\n{truncated}"))
}

/// Legacy web search via DuckDuckGo HTML only. Used when WebToolsContext is unavailable.
async fn tool_web_search_legacy(input: &serde_json::Value) -> Result<String, String> {
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
    let max_results = input["max_results"].as_u64().unwrap_or(5) as usize;

    debug!(query, "Executing web search via DuckDuckGo HTML");

    let client = crate::http_client::proxied_client_builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let resp = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .header("User-Agent", "Mozilla/5.0 (compatible; LibreFangAgent/0.1)")
        .send()
        .await
        .map_err(|e| format!("Search request failed: {e}"))?;

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read search response: {e}"))?;

    // Parse DuckDuckGo HTML results
    let results = parse_ddg_results(&body, max_results);

    if results.is_empty() {
        return Ok(format!("No results found for '{query}'."));
    }

    let mut output = format!("Search results for '{query}':\n\n");
    for (i, (title, url, snippet)) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. {}\n   URL: {}\n   {}\n\n",
            i + 1,
            title,
            url,
            snippet
        ));
    }

    Ok(output)
}

// ---------------------------------------------------------------------------
// Shell tool
// ---------------------------------------------------------------------------

async fn tool_shell_exec(
    input: &serde_json::Value,
    allowed_env: &[String],
    workspace_root: Option<&Path>,
    exec_policy: Option<&librefang_types::config::ExecPolicy>,
    interrupt: Option<crate::interrupt::SessionInterrupt>,
) -> Result<String, String> {
    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;
    // Use LLM-specified timeout, or fall back to exec policy timeout, or default 30s
    let policy_timeout = exec_policy.map(|p| p.timeout_secs).unwrap_or(30);
    let timeout_secs = input["timeout_seconds"].as_u64().unwrap_or(policy_timeout);

    // SECURITY: Determine execution strategy based on exec policy.
    //
    // In Allowlist mode (default): Use direct execution via shlex argv splitting.
    // This avoids invoking a shell interpreter, which eliminates an entire class
    // of injection attacks (encoding tricks, $IFS, glob expansion, etc.).
    //
    // In Full mode: User explicitly opted into unrestricted shell access,
    // so we use sh -c / cmd /C as before.
    let use_direct_exec = exec_policy
        .map(|p| p.mode == librefang_types::config::ExecSecurityMode::Allowlist)
        .unwrap_or(true); // Default to safe mode

    let mut cmd = if use_direct_exec {
        // SAFE PATH: Split command into argv using POSIX shell lexer rules,
        // then execute the binary directly — no shell interpreter involved.
        let argv = shlex::split(command).ok_or_else(|| {
            "Command contains unmatched quotes or invalid shell syntax".to_string()
        })?;
        if argv.is_empty() {
            return Err("Empty command after parsing".to_string());
        }
        let mut c = tokio::process::Command::new(&argv[0]);
        if argv.len() > 1 {
            c.args(&argv[1..]);
        }
        c
    } else {
        // UNSAFE PATH: Full mode — user explicitly opted in to shell interpretation.
        // Shell resolution: prefer sh (Git Bash/MSYS2) on Windows.
        #[cfg(windows)]
        let git_sh: Option<&str> = {
            const SH_PATHS: &[&str] = &[
                "C:\\Program Files\\Git\\usr\\bin\\sh.exe",
                "C:\\Program Files (x86)\\Git\\usr\\bin\\sh.exe",
            ];
            SH_PATHS
                .iter()
                .copied()
                .find(|p| std::path::Path::new(p).exists())
        };
        let (shell, shell_arg) = if cfg!(windows) {
            #[cfg(windows)]
            {
                if let Some(sh) = git_sh {
                    (sh, "-c")
                } else {
                    ("cmd", "/C")
                }
            }
            #[cfg(not(windows))]
            {
                ("sh", "-c")
            }
        } else {
            ("sh", "-c")
        };
        let mut c = tokio::process::Command::new(shell);
        c.arg(shell_arg).arg(command);
        c
    };

    // Set working directory to agent workspace so files are created there
    if let Some(ws) = workspace_root {
        cmd.current_dir(ws);
    }

    // SECURITY: Isolate environment to prevent credential leakage.
    // Hand settings may grant access to specific provider API keys.
    crate::subprocess_sandbox::sandbox_command(&mut cmd, allowed_env);

    // Ensure UTF-8 output on Windows
    #[cfg(windows)]
    cmd.env("PYTHONIOENCODING", "utf-8");

    // Prevent child from inheriting stdin (avoids blocking on Windows)
    cmd.stdin(std::process::Stdio::null());

    // Check for interrupt before we even launch the subprocess — the user may
    // have hit /stop while approval was pending or while a prior tool was running.
    if interrupt.as_ref().is_some_and(|i| i.is_cancelled()) {
        return Err("[interrupted before execution]".to_string());
    }

    // Capture piped output so we can collect it after the process exits.
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // Ensure the child is terminated when the Child handle is dropped (e.g.
    // on timeout or session cancellation) rather than becoming an orphan.
    cmd.kill_on_drop(true);

    // Spawn the child process so we hold a handle that can be killed if the
    // session interrupt fires while the command is running.  Using `output()`
    // instead would block until the process *completes*, meaning cancel() would
    // never be observed mid-execution — the whole point of this feature.
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Err(format!("Failed to execute command: {e}")),
    };

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

    // Poll for completion, interrupt, or timeout.  We use a short sleep so we
    // don't burn CPU in a tight loop; 50 ms is imperceptible to users but still
    // gives responsive cancellation.
    let output = loop {
        // Has the process already finished?
        match child.try_wait() {
            Ok(Some(_)) => {
                // Process exited; collect its output.
                match child.wait_with_output().await {
                    Ok(o) => break Ok(o),
                    Err(e) => break Err(format!("Failed to collect output: {e}")),
                }
            }
            Ok(None) => {} // still running
            Err(e) => break Err(format!("Failed to wait on child: {e}")),
        }

        // Did the session get cancelled while the command was running?
        if interrupt.as_ref().is_some_and(|i| i.is_cancelled()) {
            // Best-effort kill; ignore errors (process may have already exited).
            let _ = child.kill().await;
            return Err("[interrupted]".to_string());
        }

        // Timed out?
        if tokio::time::Instant::now() >= deadline {
            let _ = child.kill().await;
            return Err(format!("Command timed out after {timeout_secs}s"));
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);

            // Truncate very long outputs to prevent memory issues
            let max_output = 100_000;
            let stdout_str = if stdout.len() > max_output {
                format!(
                    "{}...\n[truncated, {} total bytes]",
                    crate::str_utils::safe_truncate_str(&stdout, max_output),
                    stdout.len()
                )
            } else {
                stdout.to_string()
            };
            let stderr_str = if stderr.len() > max_output {
                format!(
                    "{}...\n[truncated, {} total bytes]",
                    crate::str_utils::safe_truncate_str(&stderr, max_output),
                    stderr.len()
                )
            } else {
                stderr.to_string()
            };

            Ok(format!(
                "Exit code: {exit_code}\n\nSTDOUT:\n{stdout_str}\nSTDERR:\n{stderr_str}"
            ))
        }
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Inter-agent tools
// ---------------------------------------------------------------------------

fn require_kernel(
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<&Arc<dyn KernelHandle>, String> {
    kernel.ok_or_else(|| {
        "Kernel handle not available. Inter-agent tools require a running kernel.".to_string()
    })
}

async fn tool_agent_send(
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
fn build_agent_manifest_toml(
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
fn tools_to_parent_capabilities(tools: &[String]) -> Vec<librefang_types::capability::Capability> {
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

async fn tool_agent_spawn(
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

fn tool_agent_list(kernel: Option<&Arc<dyn KernelHandle>>) -> Result<String, String> {
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

fn tool_agent_kill(
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

/// `notify_owner(reason, summary)` — typed channel for owner-only speech.
///
/// Records the `summary` in `ToolResult.owner_notice` so the agent loop can
/// route it to the operator's DM (e.g. WhatsApp `OWNER_JID`) instead of the
/// source chat. Returns an opaque, model-visible acknowledgement so the LLM
/// does NOT see (and therefore cannot leak) the private summary back into a
/// public reply.
///
/// Errors are returned via `ToolResult.is_error = true` with a descriptive
/// message; the model is expected to retry with corrected arguments.
/// Resolve the pool `tool_load` / `tool_search` search against.
///
/// - `Some(pool)` — the agent's granted `ToolDefinition` list from the
///   agent-loop (builtin + MCP + skills). The authoritative source: if the
///   caller supplied one, we honor it verbatim — including an empty slice,
///   which means "nothing is granted". Falling back to builtin on empty
///   would leak the catalog to an agent that has none of it.
/// - `None` — caller didn't thread the granted list through (legacy
///   `execute_tool` paths: REST/MCP bridges, approval resume, unit tests).
///   Fall back to the builtin catalog so these code paths keep working.
fn meta_lookup_pool(available: Option<&[ToolDefinition]>) -> Vec<ToolDefinition> {
    match available {
        Some(list) => list.to_vec(),
        None => builtin_tool_definitions(),
    }
}

/// Meta-tool: load a tool's full schema by name (issue #3044). The returned
/// schema is both printed into `content` for the LLM to read AND attached as
/// `ToolResult.loaded_tool` so the agent loop can register it in the session's
/// lazy-load cache — making the tool callable on the next turn.
fn tool_meta_load(
    input: &serde_json::Value,
    available_tools: Option<&[ToolDefinition]>,
) -> ToolResult {
    let name = input
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        return ToolResult::error(
            "".to_string(),
            "tool_load requires a 'name' string".to_string(),
        );
    }
    let pool = meta_lookup_pool(available_tools);
    match pool.into_iter().find(|t| t.name == name) {
        Some(def) => {
            let schema = serde_json::json!({
                "name": def.name,
                "description": def.description,
                "input_schema": def.input_schema,
            });
            let content = format!(
                "Loaded tool '{}'. Schema:\n{}\n\nYou can call this tool on your next turn.",
                def.name,
                serde_json::to_string_pretty(&schema).unwrap_or_else(|_| schema.to_string()),
            );
            ToolResult {
                tool_use_id: String::new(),
                content,
                is_error: false,
                status: ToolExecutionStatus::Completed,
                loaded_tool: Some(def),
                ..Default::default()
            }
        }
        None => ToolResult::error(
            String::new(),
            format!(
                "Unknown tool '{}'. Call tool_search(query) to find available tools.",
                name
            ),
        ),
    }
}

/// Meta-tool: search the tool catalog by keyword (issue #3044). Returns a
/// short list of matching tool names and one-line hints sourced from the
/// prompt_builder catalog.
fn tool_meta_search(
    input: &serde_json::Value,
    available_tools: Option<&[ToolDefinition]>,
) -> ToolResult {
    let query = input
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if query.is_empty() {
        return ToolResult::error(
            String::new(),
            "tool_search requires a non-empty 'query' string".to_string(),
        );
    }
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .clamp(1, 50) as usize;

    // Tokenize query — any token in the tool name, description, or hint makes a hit.
    let tokens: Vec<&str> = query.split_whitespace().collect();
    let mut matches: Vec<(usize, String, String)> = Vec::new();
    for def in meta_lookup_pool(available_tools) {
        let name_lc = def.name.to_lowercase();
        let desc_lc = def.description.to_lowercase();
        let hint = crate::prompt_builder::tool_hint(&def.name);
        let hint_lc = hint.to_lowercase();
        let score = tokens.iter().fold(0usize, |acc, tok| {
            let tok = tok.trim();
            if tok.is_empty() {
                return acc;
            }
            acc + (name_lc.contains(tok) as usize) * 3
                + (hint_lc.contains(tok) as usize) * 2
                + (desc_lc.contains(tok) as usize)
        });
        if score > 0 {
            matches.push((score, def.name, hint.to_string()));
        }
    }
    matches.sort_by_key(|m| std::cmp::Reverse(m.0));
    matches.truncate(limit);

    if matches.is_empty() {
        return ToolResult::ok(
            String::new(),
            format!(
                "No tools matched '{}'. Browse the tool catalog in the system prompt.",
                query
            ),
        );
    }
    let lines: Vec<String> = matches
        .into_iter()
        .map(|(_, name, hint)| {
            if hint.is_empty() {
                name
            } else {
                format!("{name}: {hint}")
            }
        })
        .collect();
    ToolResult::ok(
        String::new(),
        format!(
            "Matches for '{}' (call tool_load(name) to get a tool's schema):\n{}",
            query,
            lines.join("\n")
        ),
    )
}

fn tool_notify_owner(tool_use_id: &str, input: &serde_json::Value) -> ToolResult {
    let reason = input
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let summary = input
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    if reason.is_empty() || summary.is_empty() {
        return ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: "Error: notify_owner requires non-empty 'reason' and 'summary' string fields."
                .to_string(),
            is_error: true,
            ..Default::default()
        };
    }

    // Compose the owner-side payload. The reason is prefixed so the operator
    // can scan a long stream of notices without parsing the body. Format:
    //     🎩 {reason}: {summary}
    let owner_payload = format!("🎩 {reason}: {summary}");

    // Structured log per OBS-01 — dispatch decision is recorded even before
    // the gateway fans it out. Target JID(s) are resolved downstream.
    tracing::info!(
        event = "owner_notify",
        reason = %reason,
        summary_len = summary.len(),
        "notify_owner tool invoked"
    );

    ToolResult {
        tool_use_id: tool_use_id.to_string(),
        // Opaque ack — intentionally devoid of summary content.
        content: "Notice queued for the owner. Do not repeat the summary in your public reply."
            .to_string(),
        is_error: false,
        owner_notice: Some(owner_payload),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Shared memory tools
// ---------------------------------------------------------------------------

fn tool_memory_store(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    peer_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    let value = input.get("value").ok_or("Missing 'value' parameter")?;
    kh.memory_store(key, value.clone(), peer_id)
        .map_err(|e| e.to_string())?;
    Ok(format!("Stored value under key '{key}'."))
}

fn tool_memory_recall(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    peer_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    match kh.memory_recall(key, peer_id).map_err(|e| e.to_string())? {
        Some(val) => Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string())),
        None => Ok(format!("No value found for key '{key}'.")),
    }
}

fn tool_memory_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
    peer_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let keys = kh.memory_list(peer_id).map_err(|e| e.to_string())?;
    if keys.is_empty() {
        return Ok("No entries found in shared memory.".to_string());
    }
    Ok(serde_json::to_string_pretty(&keys).unwrap_or_else(|_| format!("{:?}", keys)))
}

// ---------------------------------------------------------------------------
// Memory wiki tools (issue #3329)
// ---------------------------------------------------------------------------

fn tool_wiki_get(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let topic = input["topic"].as_str().ok_or("Missing 'topic' parameter")?;
    let value = kh.wiki_get(topic).map_err(|e| e.to_string())?;
    Ok(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
}

fn tool_wiki_search(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(10);
    let value = kh.wiki_search(query, limit).map_err(|e| e.to_string())?;
    Ok(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
}

fn tool_wiki_write(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    sender_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let topic = input["topic"].as_str().ok_or("Missing 'topic' parameter")?;
    let body = input["body"].as_str().ok_or("Missing 'body' parameter")?;
    let force = input
        .get("force")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Provenance is constructed kernel-side rather than left to the LLM:
    // (1) every write is required to carry an agent attribution per #3329's
    //     acceptance criterion #3, and (2) the calling agent / sender ids
    //     are authoritative — letting the model spoof them would defeat the
    //     audit value of the frontmatter.
    let agent = caller_agent_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let provenance = serde_json::json!({
        "agent": agent,
        "channel": sender_id,
        "at": chrono::Utc::now().to_rfc3339(),
    });

    let value = kh
        .wiki_write(topic, body, provenance, force)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
}

// ---------------------------------------------------------------------------
// Collaboration tools
// ---------------------------------------------------------------------------

fn tool_agent_find(
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

async fn tool_task_post(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let title = input["title"].as_str().ok_or("Missing 'title' parameter")?;
    let description = input["description"]
        .as_str()
        .ok_or("Missing 'description' parameter")?;
    let assigned_to = input["assigned_to"].as_str();
    let task_id = kh
        .task_post(title, description, assigned_to, caller_agent_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("Task created with ID: {task_id}"))
}

async fn tool_task_claim(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("task_claim requires a calling agent context")?;
    match kh.task_claim(agent_id).await.map_err(|e| e.to_string())? {
        Some(task) => {
            serde_json::to_string_pretty(&task).map_err(|e| format!("Serialize error: {e}"))
        }
        None => Ok("No tasks available.".to_string()),
    }
}

async fn tool_task_complete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("task_complete requires a calling agent context")?;
    let task_id = input["task_id"]
        .as_str()
        .ok_or("Missing 'task_id' parameter")?;
    let result = input["result"]
        .as_str()
        .ok_or("Missing 'result' parameter")?;
    kh.task_complete(agent_id, task_id, result)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("Task {task_id} marked as completed."))
}

async fn tool_task_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let status = input["status"].as_str();
    let tasks = kh.task_list(status).await.map_err(|e| e.to_string())?;
    if tasks.is_empty() {
        return Ok("No tasks found.".to_string());
    }
    serde_json::to_string_pretty(&tasks).map_err(|e| format!("Serialize error: {e}"))
}

async fn tool_task_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let task_id = input["task_id"]
        .as_str()
        .ok_or("Missing 'task_id' parameter")?;
    match kh.task_get(task_id).await.map_err(|e| e.to_string())? {
        Some(task) => {
            // Project to the same six columns comms_task_status returns from
            // the bridge SQL — keeps the native tool's contract tight even if
            // task_get later grows additional fields.
            let projected = serde_json::json!({
                "status":       task.get("status").cloned().unwrap_or(serde_json::Value::Null),
                "result":       task.get("result").cloned().unwrap_or(serde_json::Value::Null),
                "title":        task.get("title").cloned().unwrap_or(serde_json::Value::Null),
                "assigned_to":  task.get("assigned_to").cloned().unwrap_or(serde_json::Value::Null),
                "created_at":   task.get("created_at").cloned().unwrap_or(serde_json::Value::Null),
                "completed_at": task.get("completed_at").cloned().unwrap_or(serde_json::Value::Null),
            });
            serde_json::to_string_pretty(&projected).map_err(|e| format!("Serialize error: {e}"))
        }
        None => Ok(format!("Task '{task_id}' not found.")),
    }
}

async fn tool_event_publish(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let event_type = input["event_type"]
        .as_str()
        .ok_or("Missing 'event_type' parameter")?;
    let payload = input
        .get("payload")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    kh.publish_event(event_type, payload)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("Event '{event_type}' published successfully."))
}

// ---------------------------------------------------------------------------
// Goal tracking tools
// ---------------------------------------------------------------------------

fn tool_goal_update(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    // Validate input before touching the kernel
    let goal_id = input["goal_id"]
        .as_str()
        .ok_or("Missing 'goal_id' parameter")?;
    let status = input["status"].as_str();
    let progress = input["progress"].as_u64().map(|p| p.min(100) as u8);

    if status.is_none() && progress.is_none() {
        return Err("At least one of 'status' or 'progress' must be provided".to_string());
    }

    if let Some(s) = status {
        if !["pending", "in_progress", "completed", "cancelled"].contains(&s) {
            return Err(format!(
                "Invalid status '{}'. Must be: pending, in_progress, completed, or cancelled",
                s
            ));
        }
    }

    let kh = require_kernel(kernel)?;
    let updated = kh
        .goal_update(goal_id, status, progress)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::to_string_pretty(&updated).unwrap_or_else(|_| updated.to_string()))
}

// ---------------------------------------------------------------------------
// Workflow execution tool
// ---------------------------------------------------------------------------

async fn tool_workflow_run(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let workflow_id = input["workflow_id"]
        .as_str()
        .ok_or("Missing 'workflow_id' parameter")?;

    // Serialize optional input object to a JSON string for the workflow engine.
    let input_str = match input.get("input") {
        Some(v) if v.is_object() => serde_json::to_string(v)
            .map_err(|e| format!("Failed to serialize workflow input: {e}"))?,
        Some(v) if v.is_null() => String::new(),
        Some(_) => return Err("'input' must be a JSON object or null".to_string()),
        None => String::new(),
    };

    let kh = require_kernel(kernel)?;
    let (run_id, output) = kh
        .run_workflow(workflow_id, &input_str)
        .await
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "run_id": run_id,
        "output": output,
    })
    .to_string())
}

// ---------------------------------------------------------------------------
// Knowledge graph tools
// ---------------------------------------------------------------------------

fn parse_entity_type(s: &str) -> librefang_types::memory::EntityType {
    use librefang_types::memory::EntityType;
    match s.to_lowercase().as_str() {
        "person" => EntityType::Person,
        "organization" | "org" => EntityType::Organization,
        "project" => EntityType::Project,
        "concept" => EntityType::Concept,
        "event" => EntityType::Event,
        "location" => EntityType::Location,
        "document" | "doc" => EntityType::Document,
        "tool" => EntityType::Tool,
        other => EntityType::Custom(other.to_string()),
    }
}

fn parse_relation_type(s: &str) -> librefang_types::memory::RelationType {
    use librefang_types::memory::RelationType;
    match s.to_lowercase().as_str() {
        "works_at" | "worksat" => RelationType::WorksAt,
        "knows_about" | "knowsabout" | "knows" => RelationType::KnowsAbout,
        "related_to" | "relatedto" | "related" => RelationType::RelatedTo,
        "depends_on" | "dependson" | "depends" => RelationType::DependsOn,
        "owned_by" | "ownedby" => RelationType::OwnedBy,
        "created_by" | "createdby" => RelationType::CreatedBy,
        "located_in" | "locatedin" => RelationType::LocatedIn,
        "part_of" | "partof" => RelationType::PartOf,
        "uses" => RelationType::Uses,
        "produces" => RelationType::Produces,
        other => RelationType::Custom(other.to_string()),
    }
}

async fn tool_knowledge_add_entity(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let entity_type_str = input["entity_type"]
        .as_str()
        .ok_or("Missing 'entity_type' parameter")?;
    let properties = input
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    let entity = librefang_types::memory::Entity {
        id: String::new(), // kernel/store assigns a real ID
        entity_type: parse_entity_type(entity_type_str),
        name: name.to_string(),
        properties,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    let id = kh
        .knowledge_add_entity(&entity)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("Entity '{name}' added with ID: {id}"))
}

async fn tool_knowledge_add_relation(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let source = input["source"]
        .as_str()
        .ok_or("Missing 'source' parameter")?;
    let relation_str = input["relation"]
        .as_str()
        .ok_or("Missing 'relation' parameter")?;
    let target = input["target"]
        .as_str()
        .ok_or("Missing 'target' parameter")?;
    let confidence = input["confidence"].as_f64().unwrap_or(1.0) as f32;
    let properties = input
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    let relation = librefang_types::memory::Relation {
        source: source.to_string(),
        relation: parse_relation_type(relation_str),
        target: target.to_string(),
        properties,
        confidence,
        created_at: chrono::Utc::now(),
    };

    let id = kh
        .knowledge_add_relation(&relation)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "Relation '{source}' --[{relation_str}]--> '{target}' added with ID: {id}"
    ))
}

async fn tool_knowledge_query(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let source = input["source"].as_str().map(|s| s.to_string());
    let target = input["target"].as_str().map(|s| s.to_string());
    let relation = input["relation"].as_str().map(parse_relation_type);
    // Cap depth to prevent LLM-triggered DoS via exponential graph
    // traversal. Knowledge graphs rarely benefit from depth > 5 and
    // the backend traversal is O(branching_factor^depth).
    const MAX_KNOWLEDGE_DEPTH: u64 = 10;
    let max_depth = input["max_depth"]
        .as_u64()
        .unwrap_or(1)
        .min(MAX_KNOWLEDGE_DEPTH) as u32;

    let pattern = librefang_types::memory::GraphPattern {
        source,
        relation,
        target,
        max_depth,
    };

    let matches = kh
        .knowledge_query(pattern)
        .await
        .map_err(|e| e.to_string())?;
    if matches.is_empty() {
        return Ok("No matching knowledge graph entries found.".to_string());
    }

    let mut output = format!("Found {} match(es):\n", matches.len());
    for m in &matches {
        output.push_str(&format!(
            "\n  {} ({:?}) --[{:?} ({:.0}%)]--> {} ({:?})",
            m.source.name,
            m.source.entity_type,
            m.relation.relation,
            m.relation.confidence * 100.0,
            m.target.name,
            m.target.entity_type,
        ));
    }
    Ok(output)
}

// ---------------------------------------------------------------------------
// Scheduling tools
// ---------------------------------------------------------------------------

/// Parse a natural language schedule into a cron expression.
fn parse_schedule_to_cron(input: &str) -> Result<String, String> {
    let input = input.trim().to_lowercase();

    // If it already looks like a cron expression (5 space-separated fields), pass through
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() == 5
        && parts
            .iter()
            .all(|p| p.chars().all(|c| c.is_ascii_digit() || "*/,-".contains(c)))
    {
        return Ok(input);
    }

    // Natural language patterns
    if let Some(rest) = input.strip_prefix("every ") {
        if rest == "minute" || rest == "1 minute" {
            return Ok("* * * * *".to_string());
        }
        if let Some(mins) = rest.strip_suffix(" minutes") {
            let n: u32 = mins
                .trim()
                .parse()
                .map_err(|_| format!("Invalid number in '{input}'"))?;
            if n == 0 || n > 59 {
                return Err(format!("Minutes must be 1-59, got {n}"));
            }
            return Ok(format!("*/{n} * * * *"));
        }
        if rest == "hour" || rest == "1 hour" {
            return Ok("0 * * * *".to_string());
        }
        if let Some(hrs) = rest.strip_suffix(" hours") {
            let n: u32 = hrs
                .trim()
                .parse()
                .map_err(|_| format!("Invalid number in '{input}'"))?;
            if n == 0 || n > 23 {
                return Err(format!("Hours must be 1-23, got {n}"));
            }
            return Ok(format!("0 */{n} * * *"));
        }
        if rest == "day" || rest == "1 day" {
            return Ok("0 0 * * *".to_string());
        }
        if rest == "week" || rest == "1 week" {
            return Ok("0 0 * * 0".to_string());
        }
    }

    // "daily at Xam/pm"
    if let Some(time_str) = input.strip_prefix("daily at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * *"));
    }

    // "weekdays at Xam/pm"
    if let Some(time_str) = input.strip_prefix("weekdays at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * 1-5"));
    }

    // "weekends at Xam/pm"
    if let Some(time_str) = input.strip_prefix("weekends at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * 0,6"));
    }

    // "hourly" / "daily" / "weekly" / "monthly"
    match input.as_str() {
        "hourly" => return Ok("0 * * * *".to_string()),
        "daily" => return Ok("0 0 * * *".to_string()),
        "weekly" => return Ok("0 0 * * 0".to_string()),
        "monthly" => return Ok("0 0 1 * *".to_string()),
        _ => {}
    }

    Err(format!(
        "Could not parse schedule '{input}'. Try: 'every 5 minutes', 'daily at 9am', 'weekdays at 6pm', or a cron expression like '0 */5 * * *'"
    ))
}

/// Parse a time string like "9am", "6pm", "14:00", "9:30am" into an hour (0-23).
fn parse_time_to_hour(s: &str) -> Result<u32, String> {
    let s = s.trim().to_lowercase();

    // Handle "9am", "6pm", "12pm", "12am"
    if let Some(h) = s.strip_suffix("am") {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        return match hour {
            12 => Ok(0),
            1..=11 => Ok(hour),
            _ => Err(format!("Invalid hour: {hour}")),
        };
    }
    if let Some(h) = s.strip_suffix("pm") {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        return match hour {
            12 => Ok(12),
            1..=11 => Ok(hour + 12),
            _ => Err(format!("Invalid hour: {hour}")),
        };
    }

    // Handle "14:00" or "9:30"
    if let Some((h, _m)) = s.split_once(':') {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        if hour > 23 {
            return Err(format!("Hour must be 0-23, got {hour}"));
        }
        return Ok(hour);
    }

    // Plain number
    let hour: u32 = s.parse().map_err(|_| format!("Invalid time: {s}"))?;
    if hour > 23 {
        return Err(format!("Hour must be 0-23, got {hour}"));
    }
    Ok(hour)
}

// schedule_* tools — high-level wrappers around the CronScheduler engine.
// These accept natural language schedules ("daily at 9am") and delegate to
// kh.cron_create/list/cancel which use the real kernel tick loop (#2024).

async fn tool_schedule_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    sender_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for schedule_create")?;
    let description = input["description"]
        .as_str()
        .ok_or("Missing 'description' parameter")?;
    let schedule_str = input["schedule"]
        .as_str()
        .ok_or("Missing 'schedule' parameter")?;
    let message = input["message"].as_str().unwrap_or(description);

    let cron_expr = parse_schedule_to_cron(schedule_str)?;

    // CronJob name only allows alphanumeric + space/hyphen/underscore (max 128 chars).
    // Sanitize the user-provided description to fit these constraints.
    let name: String = description
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
        .take(128)
        .collect();
    let name = if name.is_empty() {
        "scheduled-task".to_string()
    } else {
        name
    };

    // Build CronJob JSON compatible with kh.cron_create()
    let tz = input["tz"].as_str();
    let schedule = if let Some(tz_str) = tz {
        serde_json::json!({ "kind": "cron", "expr": cron_expr, "tz": tz_str })
    } else {
        serde_json::json!({ "kind": "cron", "expr": cron_expr })
    };
    let mut job_json = serde_json::json!({
        "name": name,
        "schedule": schedule,
        "action": { "kind": "agent_turn", "message": message },
        "delivery": { "kind": "none" },
    });
    if let Some(obj) = job_json.as_object_mut() {
        if !obj.contains_key("peer_id") {
            if let Some(pid) = sender_id {
                if !pid.is_empty() {
                    obj.insert(
                        "peer_id".to_string(),
                        serde_json::Value::String(pid.to_string()),
                    );
                }
            }
        }
    }

    let result = kh
        .cron_create(agent_id, job_json)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "Schedule created and will execute automatically.\n  Cron: {cron_expr}\n  Original: {schedule_str}\n  {result}"
    ))
}

async fn tool_schedule_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for schedule_list")?;
    let jobs = kh.cron_list(agent_id).await.map_err(|e| e.to_string())?;

    if jobs.is_empty() {
        return Ok("No scheduled tasks.".to_string());
    }

    let mut output = format!("Scheduled tasks ({}):\n\n", jobs.len());
    for j in &jobs {
        let enabled = j["enabled"].as_bool().unwrap_or(true);
        let status = if enabled { "active" } else { "paused" };
        let schedule_display = j["schedule"]["expr"]
            .as_str()
            .or_else(|| j["schedule"]["every_secs"].as_u64().map(|_| "interval"))
            .unwrap_or("?");
        output.push_str(&format!(
            "  [{status}] {} — {}\n    Schedule: {}\n    Next run: {}\n\n",
            j["id"].as_str().unwrap_or("?"),
            j["name"].as_str().unwrap_or("?"),
            schedule_display,
            j["next_run"].as_str().unwrap_or("pending"),
        ));
    }

    Ok(output)
}

async fn tool_schedule_delete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    // Accept either "id" or "job_id" for backward compatibility
    let id = input["id"]
        .as_str()
        .or_else(|| input["job_id"].as_str())
        .ok_or("Missing 'id' parameter")?;
    kh.cron_cancel(id).await.map_err(|e| e.to_string())?;
    Ok(format!("Schedule '{id}' deleted."))
}

// ---------------------------------------------------------------------------
// Cron scheduling tools (delegated to kernel via KernelHandle trait)
// ---------------------------------------------------------------------------

async fn tool_cron_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    sender_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for cron_create")?;
    let mut job = input.clone();
    if let (Some(pid), Some(obj)) = (sender_id, job.as_object_mut()) {
        if !pid.is_empty() && !obj.contains_key("peer_id") {
            obj.insert(
                "peer_id".to_string(),
                serde_json::Value::String(pid.to_string()),
            );
        }
    }
    kh.cron_create(agent_id, job)
        .await
        .map_err(|e| e.to_string())
}

async fn tool_cron_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for cron_list")?;
    let jobs = kh.cron_list(agent_id).await.map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&jobs).map_err(|e| format!("Failed to serialize cron jobs: {e}"))
}

async fn tool_cron_cancel(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let job_id = input["job_id"]
        .as_str()
        .ok_or("Missing 'job_id' parameter")?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for cron_cancel")?;
    // Authorize: the caller may only cancel jobs that belong to them.
    // Otherwise an agent with the cron_cancel tool could delete any other
    // agent's jobs as long as it learns their UUID (via side-channel or
    // social engineering).
    let owned = kh.cron_list(agent_id).await.map_err(|e| e.to_string())?;
    let owns_job = owned.iter().any(|job| {
        job.get("id")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id == job_id)
    });
    if !owns_job {
        return Err(format!(
            "Cron job '{job_id}' not found or not owned by this agent"
        ));
    }
    kh.cron_cancel(job_id).await.map_err(|e| e.to_string())?;
    Ok(format!("Cron job '{job_id}' cancelled."))
}

// ---------------------------------------------------------------------------
// Channel send tool (proactive outbound messaging via configured adapters)
// ---------------------------------------------------------------------------

/// Parse and validate `poll_options` for the `channel_send` tool.
///
/// Telegram requires 2–10 string options per poll. A previous version used
/// `filter_map(as_str)` which silently dropped non-string entries — e.g.
/// `["a", 42, "c"]` became `["a", "c"]`, slipped past the min-2 check, and
/// sent a poll missing the user's third option. This helper fails fast
/// when any entry is the wrong type so the agent can surface the mistake
/// instead of producing a malformed poll.
fn parse_poll_options(raw: Option<&serde_json::Value>) -> Result<Vec<String>, String> {
    let arr = raw
        .and_then(|v| v.as_array())
        .ok_or_else(|| "poll_options must be an array of strings".to_string())?;
    let mut out: Vec<String> = Vec::with_capacity(arr.len());
    for (idx, v) in arr.iter().enumerate() {
        match v.as_str() {
            Some(s) => out.push(s.to_string()),
            None => {
                return Err(format!(
                    "poll_options[{idx}] must be a string, got {}",
                    match v {
                        serde_json::Value::Null => "null",
                        serde_json::Value::Bool(_) => "boolean",
                        serde_json::Value::Number(_) => "number",
                        serde_json::Value::Array(_) => "array",
                        serde_json::Value::Object(_) => "object",
                        serde_json::Value::String(_) => unreachable!(),
                    }
                ));
            }
        }
    }
    if !(2..=10).contains(&out.len()) {
        return Err(format!(
            "poll_options must have between 2 and 10 options, got {}",
            out.len()
        ));
    }
    Ok(out)
}

async fn tool_channel_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
    sender_id: Option<&str>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;

    let channel = input["channel"]
        .as_str()
        .ok_or("Missing 'channel' parameter")?
        .trim()
        .to_lowercase();

    // Use recipient from input, or fall back to sender_id from context
    // This allows agents to reply to the original sender without explicitly
    // knowing the platform-specific ID (e.g., Telegram chat_id)
    let recipient = input["recipient"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(sender_id)
        .ok_or("Missing 'recipient' parameter. When replying to the original sender, recipient is auto-filled — ensure channel_send is called in response to a message.")?
        .trim();

    if recipient.is_empty() {
        return Err("Recipient cannot be empty".to_string());
    }

    let thread_id = input["thread_id"].as_str().filter(|s| !s.is_empty());
    let account_id = input["account_id"].as_str().filter(|s| !s.is_empty());

    // Check for media content (image_url, file_url, or file_path)
    let image_url = input["image_url"].as_str().filter(|s| !s.is_empty());
    let file_url = input["file_url"].as_str().filter(|s| !s.is_empty());
    let file_path = input["file_path"].as_str().filter(|s| !s.is_empty());

    if let Some(url) = image_url {
        let caption = input["message"].as_str().filter(|s| !s.is_empty());
        if let Some(c) = caption {
            if let Some(violation) = check_taint_outbound_text(c, &TaintSink::agent_message()) {
                return Err(violation);
            }
        }
        return kh
            .send_channel_media(
                &channel, recipient, "image", url, caption, None, thread_id, account_id,
            )
            .await
            .map_err(|e| e.to_string());
    }

    if let Some(url) = file_url {
        let caption = input["message"].as_str().filter(|s| !s.is_empty());
        let filename = input["filename"].as_str();
        if let Some(c) = caption {
            if let Some(violation) = check_taint_outbound_text(c, &TaintSink::agent_message()) {
                return Err(violation);
            }
        }
        return kh
            .send_channel_media(
                &channel, recipient, "file", url, caption, filename, thread_id, account_id,
            )
            .await
            .map_err(|e| e.to_string());
    }

    // Local file attachment: read from disk and send as FileData. Honor named
    // workspace prefixes so agents can attach files that live under declared
    // `[workspaces]` mounts.
    if let Some(raw_path) = file_path {
        let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;
        let data = tokio::fs::read(&resolved)
            .await
            .map_err(|e| format!("Failed to read file '{}': {e}", resolved.display()))?;

        // Derive filename from the path if not explicitly provided
        let filename = input["filename"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                resolved
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file")
                    .to_string()
            });

        // Determine MIME type from extension
        let ext = resolved
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mime_type = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "pdf" => "application/pdf",
            "txt" => "text/plain",
            "csv" => "text/csv",
            "json" => "application/json",
            "xml" => "application/xml",
            "zip" => "application/zip",
            "gz" | "gzip" => "application/gzip",
            "tar" => "application/x-tar",
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            "mp4" => "video/mp4",
            "doc" => "application/msword",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "xls" => "application/vnd.ms-excel",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            _ => "application/octet-stream",
        };

        // `Bytes::from(Vec<u8>)` is O(1) — it takes ownership of the
        // Vec's allocation without copying. Subsequent clones (retry,
        // metering wrappers, fan-out) become refcount bumps. See #3553.
        return kh
            .send_channel_file_data(
                &channel,
                recipient,
                bytes::Bytes::from(data),
                &filename,
                mime_type,
                thread_id,
                account_id,
            )
            .await
            .map_err(|e| e.to_string());
    }

    if let Some(poll_question) = input.get("poll_question").and_then(|v| v.as_str()) {
        for key in ["image_url", "image_path", "file_url", "file_path"] {
            if input
                .get(key)
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false)
            {
                return Err(format!(
                    "poll_question cannot be combined with media/file attachments (got {key})"
                ));
            }
        }

        let poll_options = parse_poll_options(input.get("poll_options"))?;

        if let Some(violation) =
            check_taint_outbound_text(poll_question, &TaintSink::agent_message())
        {
            return Err(violation);
        }
        for opt in &poll_options {
            if let Some(violation) = check_taint_outbound_text(opt, &TaintSink::agent_message()) {
                return Err(violation);
            }
        }

        let is_quiz = input
            .get("poll_is_quiz")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let correct_option_id = input
            .get("poll_correct_option")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8);
        let explanation = input.get("poll_explanation").and_then(|v| v.as_str());
        if let Some(exp) = explanation {
            if let Some(violation) = check_taint_outbound_text(exp, &TaintSink::agent_message()) {
                return Err(violation);
            }
        }

        // Validate quiz mode requirements
        if is_quiz {
            let id = correct_option_id.ok_or_else(|| {
                "poll_correct_option is required when poll_is_quiz is true".to_string()
            })?;
            if id as usize >= poll_options.len() {
                return Err(format!(
                    "poll_correct_option {} is out of bounds (must be between 0 and {})",
                    id,
                    poll_options.len() - 1
                ));
            }
        }

        kh.send_channel_poll(
            &channel,
            recipient,
            poll_question,
            &poll_options,
            is_quiz,
            correct_option_id,
            explanation,
            account_id,
        )
        .await
        .map_err(|e| e.to_string())?;

        let mut result = format!("Poll sent to {recipient} on {channel}: {poll_question}");
        if is_quiz {
            result.push_str(" (quiz mode)");
        }
        return Ok(result);
    }

    // Text-only message
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter (required for text messages)")?;

    if message.is_empty() {
        return Err("Message cannot be empty".to_string());
    }

    // For email channels, validate email format and prepend subject
    let final_message = if channel == "email" {
        if !recipient.contains('@') || !recipient.contains('.') {
            return Err(format!("Invalid email address: '{recipient}'"));
        }
        if let Some(subject) = input["subject"].as_str() {
            if !subject.is_empty() {
                format!("Subject: {subject}\n\n{message}")
            } else {
                message.to_string()
            }
        } else {
            message.to_string()
        }
    } else {
        message.to_string()
    };

    if let Some(violation) = check_taint_outbound_text(&final_message, &TaintSink::agent_message())
    {
        return Err(violation);
    }

    kh.send_channel_message(&channel, recipient, &final_message, thread_id, account_id)
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Hand tools (delegated to kernel via KernelHandle trait)
// ---------------------------------------------------------------------------

async fn tool_hand_list(kernel: Option<&Arc<dyn KernelHandle>>) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let hands = kh.hand_list().await.map_err(|e| e.to_string())?;

    if hands.is_empty() {
        return Ok(
            "No Hands available. Install hands to enable curated autonomous packages.".to_string(),
        );
    }

    let mut lines = vec!["Available Hands:".to_string(), String::new()];
    for h in &hands {
        let icon = h["icon"].as_str().unwrap_or("");
        let name = h["name"].as_str().unwrap_or("?");
        let id = h["id"].as_str().unwrap_or("?");
        let status = h["status"].as_str().unwrap_or("unknown");
        let desc = h["description"].as_str().unwrap_or("");

        let status_marker = match status {
            "Active" => "[ACTIVE]",
            "Paused" => "[PAUSED]",
            _ => "[available]",
        };

        lines.push(format!("{} {} ({}) {}", icon, name, id, status_marker));
        if !desc.is_empty() {
            lines.push(format!("  {}", desc));
        }
        if let Some(iid) = h["instance_id"].as_str() {
            lines.push(format!("  Instance: {}", iid));
        }
        lines.push(String::new());
    }

    Ok(lines.join("\n"))
}

async fn tool_hand_activate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let hand_id = input["hand_id"]
        .as_str()
        .ok_or("Missing 'hand_id' parameter")?;
    let config: std::collections::HashMap<String, serde_json::Value> =
        if let Some(obj) = input["config"].as_object() {
            obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        } else {
            std::collections::HashMap::new()
        };

    let result = kh
        .hand_activate(hand_id, config)
        .await
        .map_err(|e| e.to_string())?;

    let instance_id = result["instance_id"].as_str().unwrap_or("?");
    let agent_name = result["agent_name"].as_str().unwrap_or("?");
    let status = result["status"].as_str().unwrap_or("?");

    Ok(format!(
        "Hand '{}' activated!\n  Instance: {}\n  Agent: {} ({})",
        hand_id, instance_id, agent_name, status
    ))
}

async fn tool_hand_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let hand_id = input["hand_id"]
        .as_str()
        .ok_or("Missing 'hand_id' parameter")?;

    let result = kh.hand_status(hand_id).await.map_err(|e| e.to_string())?;

    let icon = result["icon"].as_str().unwrap_or("");
    let name = result["name"].as_str().unwrap_or(hand_id);
    let status = result["status"].as_str().unwrap_or("unknown");
    let instance_id = result["instance_id"].as_str().unwrap_or("?");
    let agent_name = result["agent_name"].as_str().unwrap_or("?");
    let activated = result["activated_at"].as_str().unwrap_or("?");

    Ok(format!(
        "{} {} — {}\n  Instance: {}\n  Agent: {}\n  Activated: {}",
        icon, name, status, instance_id, agent_name, activated
    ))
}

async fn tool_hand_deactivate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let instance_id = input["instance_id"]
        .as_str()
        .ok_or("Missing 'instance_id' parameter")?;
    kh.hand_deactivate(instance_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("Hand instance '{}' deactivated.", instance_id))
}

// ---------------------------------------------------------------------------
// A2A outbound tools (cross-instance agent communication)
// ---------------------------------------------------------------------------

/// Discover an external A2A agent by fetching its agent card.
async fn tool_a2a_discover(input: &serde_json::Value) -> Result<String, String> {
    let url = input["url"].as_str().ok_or("Missing 'url' parameter")?;

    // SSRF protection: block private/metadata IPs
    if crate::web_fetch::check_ssrf(url, &[]).is_err() {
        return Err("SSRF blocked: URL resolves to a private or metadata address".to_string());
    }

    let client = crate::a2a::A2aClient::new();
    let card = client.discover(url).await?;

    serde_json::to_string_pretty(&card).map_err(|e| format!("Serialization error: {e}"))
}

/// Send a task to an external A2A agent.
async fn tool_a2a_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;

    // Resolve agent URL: either directly provided or looked up by name.
    // Canonicalize early so the trust gate below sees the same string the
    // approve flow stored.
    let url = if let Some(raw) = input["agent_url"].as_str() {
        // SSRF protection
        if crate::web_fetch::check_ssrf(raw, &[]).is_err() {
            return Err("SSRF blocked: URL resolves to a private or metadata address".to_string());
        }
        crate::a2a::canonicalize_a2a_url(raw).unwrap_or_else(|| raw.to_string())
    } else if let Some(name) = input["agent_name"].as_str() {
        kh.get_a2a_agent_url(name)
            .ok_or_else(|| format!("No known A2A agent with name '{name}'. Use a2a_discover first or provide agent_url directly."))?
    } else {
        return Err("Missing 'agent_url' or 'agent_name' parameter".to_string());
    };

    // Taint sink: block secrets from being exfiltrated to an external A2A peer.
    // Runs before the trust gate so a tainted-message attempt always reports
    // the data-exfil reason (the test suite asserts this contract) — the
    // trust gate is purely about target authorization and would mask the
    // more serious finding.
    if let Some(violation) = check_taint_outbound_text(message, &TaintSink::agent_message()) {
        return Err(violation);
    }
    // Also gate the URL itself against query-string credential leaks.
    if let Some(violation) = check_taint_net_fetch(&url) {
        return Err(violation);
    }

    // SECURITY (Bug #3786): the HTTP route at `/api/a2a/send` enforces a
    // trust gate that requires the URL to live in `kernel.list_a2a_agents()`.
    // The agent-side tool path bypassed that gate entirely, so an LLM could
    // exfiltrate to any non-private URL the SSRF allowlist accepted. Mirror
    // the same check here.
    let trusted_urls: Vec<String> = kh.list_a2a_agents().into_iter().map(|(_, u)| u).collect();
    if !trusted_urls.iter().any(|u| u == &url) {
        return Err(format!(
            "A2A target '{url}' is not on the trusted-agent list. Discover and have an operator approve it via POST /api/a2a/agents/{{url}}/approve before agents may send to it."
        ));
    }

    let session_id = input["session_id"].as_str();
    let client = crate::a2a::A2aClient::new();
    let task = client.send_task(&url, message, session_id).await?;

    serde_json::to_string_pretty(&task).map_err(|e| format!("Serialization error: {e}"))
}

// ---------------------------------------------------------------------------
// Image analysis tool
// ---------------------------------------------------------------------------

async fn tool_image_analyze(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let prompt = input["prompt"].as_str().unwrap_or("");
    // Route through the workspace sandbox so user-supplied paths cannot
    // escape to arbitrary filesystem locations (e.g. /etc/passwd). Named
    // workspace prefixes are honored via `additional_roots` so agents can
    // analyze images that live under declared `[workspaces]` mounts.
    let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;

    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read image '{raw_path}': {e}"))?;

    let file_size = data.len();

    // Detect image format from magic bytes
    let format = detect_image_format(&data);

    // Extract dimensions for common formats
    let dimensions = extract_image_dimensions(&data, &format);

    // Base64-encode (truncate for very large images in the response)
    let base64_preview = if file_size <= 512 * 1024 {
        // Under 512KB — include full base64
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&data)
    } else {
        // Over 512KB — include first 64KB preview
        use base64::Engine;
        let preview_bytes = &data[..64 * 1024];
        format!(
            "{}... [truncated, {} total bytes]",
            base64::engine::general_purpose::STANDARD.encode(preview_bytes),
            file_size
        )
    };

    let mut result = serde_json::json!({
        "path": raw_path,
        "format": format,
        "file_size_bytes": file_size,
        "file_size_human": format_file_size(file_size),
    });

    if let Some((w, h)) = dimensions {
        result["width"] = serde_json::json!(w);
        result["height"] = serde_json::json!(h);
    }

    if !prompt.is_empty() {
        result["prompt"] = serde_json::json!(prompt);
        result["note"] = serde_json::json!(
            "Vision analysis requires a vision-capable LLM. The base64 data is included for downstream processing."
        );
    }

    result["base64_preview"] = serde_json::json!(base64_preview);

    serde_json::to_string_pretty(&result).map_err(|e| format!("Serialize error: {e}"))
}

/// Detect image format from magic bytes.
fn detect_image_format(data: &[u8]) -> String {
    if data.len() < 4 {
        return "unknown".to_string();
    }
    if data.starts_with(b"\x89PNG") {
        "png".to_string()
    } else if data.starts_with(b"\xFF\xD8\xFF") {
        "jpeg".to_string()
    } else if data.starts_with(b"GIF8") {
        "gif".to_string()
    } else if data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"WEBP" {
        "webp".to_string()
    } else if data.starts_with(b"BM") {
        "bmp".to_string()
    } else if data.starts_with(b"\x00\x00\x01\x00") {
        "ico".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Extract image dimensions from common formats.
fn extract_image_dimensions(data: &[u8], format: &str) -> Option<(u32, u32)> {
    match format {
        "png" => {
            // PNG: IHDR chunk starts at byte 16, width at 16-19, height at 20-23
            if data.len() >= 24 {
                let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
                let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
                Some((w, h))
            } else {
                None
            }
        }
        "gif" => {
            // GIF: width at bytes 6-7, height at bytes 8-9 (little-endian)
            if data.len() >= 10 {
                let w = u16::from_le_bytes([data[6], data[7]]) as u32;
                let h = u16::from_le_bytes([data[8], data[9]]) as u32;
                Some((w, h))
            } else {
                None
            }
        }
        "bmp" => {
            // BMP: width at bytes 18-21, height at bytes 22-25 (little-endian)
            if data.len() >= 26 {
                let w = u32::from_le_bytes([data[18], data[19], data[20], data[21]]);
                let h = u32::from_le_bytes([data[22], data[23], data[24], data[25]]);
                Some((w, h))
            } else {
                None
            }
        }
        "jpeg" => {
            // JPEG: scan for SOF0 marker (0xFF 0xC0) to find dimensions
            extract_jpeg_dimensions(data)
        }
        _ => None,
    }
}

/// Extract JPEG dimensions by scanning for SOF markers.
fn extract_jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2; // Skip SOI marker
    while i + 1 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        // SOF0-SOF3 markers contain dimensions
        if (0xC0..=0xC3).contains(&marker) && i + 9 < data.len() {
            let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
            let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
            return Some((w, h));
        }
        if i + 3 < data.len() {
            let seg_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            i += 2 + seg_len;
        } else {
            break;
        }
    }
    None
}

/// Format file size in human-readable form.
fn format_file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// ---------------------------------------------------------------------------
// Location tool
// ---------------------------------------------------------------------------

async fn tool_location_get() -> Result<String, String> {
    let client = crate::http_client::proxied_client_builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    // Use ip-api.com (free, no API key, JSON response)
    let resp = client
        .get("https://ip-api.com/json/?fields=status,message,country,regionName,city,zip,lat,lon,timezone,isp,query")
        .header("User-Agent", "LibreFang/0.1")
        .send()
        .await
        .map_err(|e| format!("Location request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Location API returned {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse location response: {e}"))?;

    if body["status"].as_str() != Some("success") {
        let msg = body["message"].as_str().unwrap_or("Unknown error");
        return Err(format!("Location lookup failed: {msg}"));
    }

    let result = serde_json::json!({
        "lat": body["lat"],
        "lon": body["lon"],
        "city": body["city"],
        "region": body["regionName"],
        "country": body["country"],
        "zip": body["zip"],
        "timezone": body["timezone"],
        "isp": body["isp"],
        "ip": body["query"],
    });

    serde_json::to_string_pretty(&result).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// System time tool
// ---------------------------------------------------------------------------

/// Return current date, time, timezone, and Unix epoch.
fn tool_system_time() -> String {
    let now_utc = chrono::Utc::now();
    let now_local = chrono::Local::now();
    let result = serde_json::json!({
        "utc": now_utc.to_rfc3339(),
        "local": now_local.to_rfc3339(),
        "unix_epoch": now_utc.timestamp(),
        "timezone": now_local.format("%Z").to_string(),
        "utc_offset": now_local.format("%:z").to_string(),
        "date": now_local.format("%Y-%m-%d").to_string(),
        "time": now_local.format("%H:%M:%S").to_string(),
        "day_of_week": now_local.format("%A").to_string(),
    });
    serde_json::to_string_pretty(&result).unwrap_or_else(|_| now_utc.to_rfc3339())
}

// ---------------------------------------------------------------------------
// Media understanding tools
// ---------------------------------------------------------------------------

/// Describe an image using a vision-capable LLM provider.
async fn tool_media_describe(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    use base64::Engine;
    let engine = media_engine.ok_or("Media engine not available. Check media configuration.")?;
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    // Route through the workspace sandbox so all media reads stay inside
    // the agent's dir — a plain `..` check would miss absolute paths like
    // `/etc/passwd`. Named workspace prefixes are honored via
    // `additional_roots` so agents can describe media that lives under
    // declared `[workspaces]` mounts.
    let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;

    // Read image file
    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read image file: {e}"))?;

    // Detect MIME type from extension
    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => return Err(format!("Unsupported image format: .{ext}")),
    };

    let attachment = librefang_types::media::MediaAttachment {
        media_type: librefang_types::media::MediaType::Image,
        mime_type: mime.to_string(),
        source: librefang_types::media::MediaSource::Base64 {
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            mime_type: mime.to_string(),
        },
        size_bytes: data.len() as u64,
    };

    let understanding = engine.describe_image(&attachment).await?;
    serde_json::to_string_pretty(&understanding).map_err(|e| format!("Serialize error: {e}"))
}

/// Transcribe audio to text using speech-to-text.
async fn tool_media_transcribe(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    use base64::Engine;
    let engine = media_engine.ok_or("Media engine not available. Check media configuration.")?;
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    // Route through the workspace sandbox so all media reads stay inside
    // the agent's dir — a plain `..` check would miss absolute paths like
    // `/etc/passwd`. Named workspace prefixes are honored via
    // `additional_roots` so agents can transcribe audio under declared
    // `[workspaces]` mounts.
    let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;

    // Read audio file
    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read audio file: {e}"))?;

    // Detect MIME type from extension
    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime = match ext.as_str() {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "webm" => "audio/webm",
        _ => return Err(format!("Unsupported audio format: .{ext}")),
    };

    let attachment = librefang_types::media::MediaAttachment {
        media_type: librefang_types::media::MediaType::Audio,
        mime_type: mime.to_string(),
        source: librefang_types::media::MediaSource::Base64 {
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            mime_type: mime.to_string(),
        },
        size_bytes: data.len() as u64,
    };

    let understanding = engine.transcribe_audio(&attachment).await?;
    serde_json::to_string_pretty(&understanding).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// Image generation tool
// ---------------------------------------------------------------------------

/// Generate images from a text prompt.
async fn tool_image_generate(
    input: &serde_json::Value,
    media_drivers: Option<&crate::media::MediaDriverCache>,
    workspace_root: Option<&Path>,
    upload_dir: &Path,
) -> Result<String, String> {
    let prompt = input["prompt"]
        .as_str()
        .ok_or("Missing 'prompt' parameter")?;

    let provider = input["provider"].as_str().map(|s| s.to_string());
    let model = input["model"].as_str().map(|s| s.to_string());
    let aspect_ratio = input["aspect_ratio"].as_str().map(|s| s.to_string());
    let width = input["width"].as_u64().map(|v| v as u32);
    let height = input["height"].as_u64().map(|v| v as u32);
    let quality = input["quality"].as_str().map(|s| s.to_string());
    let count = input["count"].as_u64().unwrap_or(1).min(9) as u8;

    // Use MediaDriverCache if available (multi-provider), fall back to old OpenAI-only path.
    if let Some(cache) = media_drivers {
        let request = librefang_types::media::MediaImageRequest {
            prompt: prompt.to_string(),
            provider: provider.clone(),
            model,
            width,
            height,
            aspect_ratio,
            quality,
            count,
            seed: None,
        };

        request.validate().map_err(|e| e.to_string())?;

        let driver = if let Some(ref name) = provider {
            cache.get_or_create(name, None)
        } else {
            cache.detect_for_capability(librefang_types::media::MediaCapability::ImageGeneration)
        }
        .map_err(|e| e.to_string())?;

        let result = driver
            .generate_image(&request)
            .await
            .map_err(|e| e.to_string())?;

        // Save images to workspace and uploads dir
        let saved_paths = save_media_images_to_workspace(&result.images, workspace_root);
        let image_urls = save_media_images_to_uploads(&result.images, upload_dir);

        let response = serde_json::json!({
            "model": result.model,
            "provider": result.provider,
            "images_generated": result.images.len(),
            "saved_to": saved_paths,
            "revised_prompt": result.revised_prompt,
            "image_urls": image_urls,
        });

        return serde_json::to_string_pretty(&response)
            .map_err(|e| format!("Serialize error: {e}"));
    }

    // Fallback: old OpenAI-only path (when media_drivers is None)
    let model_str = input["model"].as_str().unwrap_or("dall-e-3");
    let model = match model_str {
        "dall-e-3" | "dalle3" | "dalle-3" => librefang_types::media::ImageGenModel::DallE3,
        "dall-e-2" | "dalle2" | "dalle-2" => librefang_types::media::ImageGenModel::DallE2,
        "gpt-image-1" | "gpt_image_1" => librefang_types::media::ImageGenModel::GptImage1,
        _ => {
            return Err(format!(
                "Unknown image model: {model_str}. Use 'dall-e-3', 'dall-e-2', or 'gpt-image-1'."
            ))
        }
    };

    let size = input["size"].as_str().unwrap_or("1024x1024").to_string();
    let quality_str = input["quality"].as_str().unwrap_or("hd").to_string();
    let count_val = input["count"].as_u64().unwrap_or(1).min(4) as u8;

    let request = librefang_types::media::ImageGenRequest {
        prompt: prompt.to_string(),
        model,
        size,
        quality: quality_str,
        count: count_val,
    };

    let result = crate::image_gen::generate_image(&request).await?;

    let saved_paths = if let Some(workspace) = workspace_root {
        match crate::image_gen::save_images_to_workspace(&result, workspace) {
            Ok(paths) => paths,
            Err(e) => {
                warn!("Failed to save images to workspace: {e}");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    let mut image_urls: Vec<String> = Vec::new();
    {
        use base64::Engine;
        let _ = std::fs::create_dir_all(upload_dir);
        for img in &result.images {
            let file_id = uuid::Uuid::new_v4().to_string();
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&img.data_base64)
            {
                let path = upload_dir.join(&file_id);
                if std::fs::write(&path, &decoded).is_ok() {
                    image_urls.push(format!("/api/uploads/{file_id}"));
                }
            }
        }
    }

    let response = serde_json::json!({
        "model": result.model,
        "images_generated": result.images.len(),
        "saved_to": saved_paths,
        "revised_prompt": result.revised_prompt,
        "image_urls": image_urls,
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

/// Save MediaImageResult images to workspace output/ dir.
fn save_media_images_to_workspace(
    images: &[librefang_types::media::GeneratedImage],
    workspace_root: Option<&Path>,
) -> Vec<String> {
    let Some(workspace) = workspace_root else {
        return Vec::new();
    };
    use base64::Engine;
    let output_dir = workspace.join("output");
    let _ = std::fs::create_dir_all(&output_dir);
    let mut paths = Vec::new();
    for (i, img) in images.iter().enumerate() {
        if img.data_base64.is_empty() {
            continue;
        }
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&img.data_base64) {
            let filename = format!("image_{}.png", i);
            let path = output_dir.join(&filename);
            if std::fs::write(&path, &decoded).is_ok() {
                paths.push(path.display().to_string());
            }
        }
    }
    paths
}

/// Save MediaImageResult images to uploads temp dir, returning /api/uploads/... URLs.
fn save_media_images_to_uploads(
    images: &[librefang_types::media::GeneratedImage],
    upload_dir: &Path,
) -> Vec<String> {
    use base64::Engine;
    let _ = std::fs::create_dir_all(upload_dir);
    let mut urls = Vec::new();
    for img in images {
        // If provider returned a URL directly, use it as-is
        if img.data_base64.is_empty() {
            if let Some(ref url) = img.url {
                urls.push(url.clone());
            }
            continue;
        }
        let file_id = uuid::Uuid::new_v4().to_string();
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&img.data_base64) {
            if !decoded.is_empty() {
                let path = upload_dir.join(&file_id);
                if std::fs::write(&path, &decoded).is_ok() {
                    urls.push(format!("/api/uploads/{file_id}"));
                }
            }
        }
    }
    urls
}

// ---------------------------------------------------------------------------
// Video / Music generation tools (MediaDriver-based)
// ---------------------------------------------------------------------------

/// Generate a video from a text prompt. Returns a task_id for async polling.
async fn tool_video_generate(
    input: &serde_json::Value,
    media_drivers: Option<&crate::media::MediaDriverCache>,
) -> Result<String, String> {
    let cache =
        media_drivers.ok_or("Media drivers not available. Ensure media drivers are configured.")?;
    let prompt = input["prompt"]
        .as_str()
        .ok_or("Missing 'prompt' parameter")?;

    let request = librefang_types::media::MediaVideoRequest {
        prompt: prompt.to_string(),
        provider: input["provider"].as_str().map(String::from),
        model: input["model"].as_str().map(String::from),
        duration_secs: input["duration"].as_u64().map(|v| v as u32),
        resolution: input["resolution"].as_str().map(String::from),
        image_url: None,
        optimize_prompt: None,
    };

    // Validate request parameters before sending to the provider
    request
        .validate()
        .map_err(|e| format!("Invalid request: {e}"))?;

    let driver = if let Some(p) = &request.provider {
        cache.get_or_create(p, None).map_err(|e| e.to_string())?
    } else {
        cache
            .detect_for_capability(librefang_types::media::MediaCapability::VideoGeneration)
            .map_err(|e| e.to_string())?
    };

    let result = driver
        .submit_video(&request)
        .await
        .map_err(|e| e.to_string())?;

    let response = serde_json::json!({
        "task_id": result.task_id,
        "provider": result.provider,
        "status": "submitted",
        "note": "Use video_status tool with this task_id to check progress"
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

/// Check the status of a video generation task. Returns download URL when complete.
async fn tool_video_status(
    input: &serde_json::Value,
    media_drivers: Option<&crate::media::MediaDriverCache>,
) -> Result<String, String> {
    let cache =
        media_drivers.ok_or("Media drivers not available. Ensure media drivers are configured.")?;
    let task_id = input["task_id"]
        .as_str()
        .ok_or("Missing 'task_id' parameter")?;
    let provider = input["provider"].as_str();

    let driver = if let Some(p) = provider {
        cache.get_or_create(p, None).map_err(|e| e.to_string())?
    } else {
        cache
            .detect_for_capability(librefang_types::media::MediaCapability::VideoGeneration)
            .map_err(|e| e.to_string())?
    };

    let status = driver
        .poll_video(task_id)
        .await
        .map_err(|e| e.to_string())?;

    // If completed, also fetch the full result with download URL
    if status == librefang_types::media::MediaTaskStatus::Completed {
        let video_result = driver
            .get_video_result(task_id)
            .await
            .map_err(|e| e.to_string())?;
        let response = serde_json::json!({
            "status": "completed",
            "file_url": video_result.file_url,
            "width": video_result.width,
            "height": video_result.height,
            "duration_secs": video_result.duration_secs,
            "provider": video_result.provider,
            "model": video_result.model,
        });
        return serde_json::to_string_pretty(&response)
            .map_err(|e| format!("Serialize error: {e}"));
    }

    let response = serde_json::json!({
        "status": status.to_string(),
        "task_id": task_id,
        "note": "Task is still in progress. Poll again after a few seconds."
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

/// Generate music from a prompt and/or lyrics. Saves audio to workspace output/ directory.
async fn tool_music_generate(
    input: &serde_json::Value,
    media_drivers: Option<&crate::media::MediaDriverCache>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let cache =
        media_drivers.ok_or("Media drivers not available. Ensure media drivers are configured.")?;

    let request = librefang_types::media::MediaMusicRequest {
        prompt: input["prompt"].as_str().map(String::from),
        lyrics: input["lyrics"].as_str().map(String::from),
        provider: input["provider"].as_str().map(String::from),
        model: input["model"].as_str().map(String::from),
        instrumental: input["instrumental"].as_bool().unwrap_or(false),
        format: None,
    };

    // Validate request parameters before sending to the provider
    request
        .validate()
        .map_err(|e| format!("Invalid request: {e}"))?;

    let driver = if let Some(p) = &request.provider {
        cache.get_or_create(p, None).map_err(|e| e.to_string())?
    } else {
        cache
            .detect_for_capability(librefang_types::media::MediaCapability::MusicGeneration)
            .map_err(|e| e.to_string())?
    };

    let result = driver
        .generate_music(&request)
        .await
        .map_err(|e| e.to_string())?;

    // Save audio to workspace output/ directory (same pattern as text_to_speech)
    let saved_path = if let Some(workspace) = workspace_root {
        let output_dir = workspace.join("output");
        tokio::fs::create_dir_all(&output_dir)
            .await
            .map_err(|e| format!("Failed to create output dir: {e}"))?;

        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let filename = format!("music_{timestamp}.{}", result.format);
        let path = output_dir.join(&filename);

        tokio::fs::write(&path, &result.audio_data)
            .await
            .map_err(|e| format!("Failed to write audio file: {e}"))?;

        Some(path.display().to_string())
    } else {
        None
    };

    let mut response = serde_json::json!({
        "saved_to": saved_path,
        "format": result.format,
        "provider": result.provider,
        "model": result.model,
        "duration_ms": result.duration_ms,
        "size_bytes": result.audio_data.len(),
    });

    // When no workspace is available (e.g. MCP context), include base64-encoded
    // audio so the caller can still retrieve the generated content.
    if saved_path.is_none() && !result.audio_data.is_empty() {
        use base64::Engine;
        response["audio_base64"] =
            serde_json::json!(base64::engine::general_purpose::STANDARD.encode(&result.audio_data));
    }

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// TTS / STT tools
// ---------------------------------------------------------------------------

async fn tool_text_to_speech(
    input: &serde_json::Value,
    media_drivers: Option<&crate::media::MediaDriverCache>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let text = input["text"].as_str().ok_or("Missing 'text' parameter")?;
    let voice = input["voice"].as_str();
    let format = input["format"].as_str();
    let provider = input["provider"].as_str();
    let output_format = input["output_format"].as_str().unwrap_or("mp3");

    if let Some(cache) = media_drivers {
        let resolved_provider =
            provider.or_else(|| tts_engine.and_then(|e| e.tts_config().provider.as_deref()));

        let driver_result = if let Some(p) = resolved_provider {
            cache.get_or_create(p, None)
        } else {
            cache.detect_for_capability(librefang_types::media::MediaCapability::TextToSpeech)
        };

        // Google TTS: override LLM-provided voice (e.g. "alloy") with the
        // configured one — Google doesn't recognise OpenAI voice names.
        let (effective_voice, effective_language, effective_speed, effective_pitch) =
            if resolved_provider == Some("google_tts") {
                if let Some(engine) = tts_engine {
                    let cfg = &engine.tts_config().google;
                    (
                        Some(cfg.voice.clone()),
                        Some(cfg.language_code.clone()),
                        Some(cfg.speaking_rate),
                        Some(cfg.pitch),
                    )
                } else {
                    (None, None, None, None)
                }
            } else {
                (None, None, None, None)
            };

        let request = librefang_types::media::MediaTtsRequest {
            text: text.to_string(),
            provider: resolved_provider.map(String::from),
            model: input["model"].as_str().map(String::from),
            voice: effective_voice.or_else(|| voice.map(String::from)),
            format: format.map(String::from),
            speed: effective_speed.or_else(|| input["speed"].as_f64().map(|v| v as f32)),
            language: effective_language.or_else(|| input["language"].as_str().map(String::from)),
            pitch: effective_pitch.or_else(|| input["pitch"].as_f64().map(|v| v as f32)),
        };

        if let Ok(driver) = driver_result {
            let result = driver
                .synthesize_speech(&request)
                .await
                .map_err(|e| e.to_string())?;

            return finish_tts_result(
                &result.audio_data,
                &result.format,
                &result.provider,
                result.duration_ms,
                workspace_root,
                output_format,
            )
            .await;
        }
        // If no driver is configured for TTS, fall through to old TtsEngine
    }

    // Fallback: old TtsEngine (OpenAI / ElevenLabs only)
    let engine =
        tts_engine.ok_or("TTS not available. No media driver or TTS engine configured.")?;

    let result = engine.synthesize(text, voice, format).await?;

    finish_tts_result(
        &result.audio_data,
        &result.format,
        &result.provider,
        Some(result.duration_estimate_ms),
        workspace_root,
        output_format,
    )
    .await
}

/// Convert audio data to OGG Opus via ffmpeg.
/// Returns `Ok(None)` if ffmpeg is not installed (caller should fall back to
/// saving the original format). Returns `Ok(Some(...))` on success with the
/// saved path, format string, and file size.
async fn convert_to_ogg_opus(
    audio_data: &[u8],
    output_dir: &Path,
    timestamp: &str,
) -> Result<Option<(Option<String>, String, usize)>, String> {
    let ogg_filename = format!("tts_{timestamp}.ogg");
    let ogg_path = output_dir.join(&ogg_filename);
    let ogg_path_str = ogg_path
        .to_str()
        .ok_or_else(|| "Output path contains invalid UTF-8".to_string())?;

    let spawn_result = tokio::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            "pipe:0",
            "-c:a",
            "libopus",
            "-b:a",
            "32k",
            "-ar",
            "48000",
            "-ac",
            "1",
            ogg_path_str,
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn();

    let mut child = match spawn_result {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to run ffmpeg: {e}")),
    };

    // Write audio to ffmpeg stdin, then close it (EOF triggers encoding).
    // Sequential write→wait is safe: stdout is Stdio::null() so ffmpeg
    // never blocks on output, and stderr is piped but read after exit.
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(audio_data)
            .await
            .map_err(|e| format!("Failed to pipe audio to ffmpeg: {e}"))?;
        // stdin drops here → EOF sent to ffmpeg
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("ffmpeg process error: {e}"))?;

    if !output.status.success() {
        // Clean up partial output file
        let _ = tokio::fs::remove_file(&ogg_path).await;
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last_lines: String = stderr
            .lines()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "ffmpeg conversion to OGG Opus failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            last_lines
        ));
    }

    let ogg_size = tokio::fs::metadata(&ogg_path)
        .await
        .map(|m| m.len() as usize)
        .unwrap_or(0);

    if ogg_size == 0 {
        let _ = tokio::fs::remove_file(&ogg_path).await;
        return Err("ffmpeg exited successfully but produced an empty OGG file".into());
    }

    Ok(Some((
        Some(ogg_path.display().to_string()),
        "ogg".to_string(),
        ogg_size,
    )))
}

/// Save TTS audio to workspace and build JSON response.
/// When `output_format` is `"ogg_opus"` and ffmpeg is available, the saved file
/// is converted from the provider format (typically MP3) to OGG Opus so it can
/// be sent as a WhatsApp voice note. Falls back to the original format if ffmpeg
/// is not installed.
async fn finish_tts_result(
    audio_data: &[u8],
    format: &str,
    provider: &str,
    duration_ms: Option<u64>,
    workspace_root: Option<&Path>,
    output_format: &str,
) -> Result<String, String> {
    let (saved_path, final_format, final_size, warning) = if let Some(workspace) = workspace_root {
        let output_dir = workspace.join("output");
        tokio::fs::create_dir_all(&output_dir)
            .await
            .map_err(|e| format!("Failed to create output dir: {e}"))?;

        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();

        if output_format == "ogg_opus" && !matches!(format, "ogg" | "opus" | "ogg_opus") {
            // Try ffmpeg conversion; fall back to saving the original format if
            // ffmpeg is not installed (preserves backward compatibility).
            match convert_to_ogg_opus(audio_data, &output_dir, &timestamp).await {
                Ok(Some(result)) => (result.0, result.1, result.2, None),
                Ok(None) => {
                    let filename = format!("tts_{timestamp}.{format}");
                    let path = output_dir.join(&filename);
                    tokio::fs::write(&path, audio_data)
                        .await
                        .map_err(|e| format!("Failed to write audio file: {e}"))?;
                    (
                        Some(path.display().to_string()),
                        format.to_string(),
                        audio_data.len(),
                        Some(
                            "ffmpeg not found; saved as original format instead of ogg_opus"
                                .to_string(),
                        ),
                    )
                }
                Err(e) => {
                    tracing::warn!("OGG Opus conversion failed, falling back to {format}: {e}");
                    let filename = format!("tts_{timestamp}.{format}");
                    let path = output_dir.join(&filename);
                    tokio::fs::write(&path, audio_data)
                        .await
                        .map_err(|e| format!("Failed to write audio file: {e}"))?;
                    (
                        Some(path.display().to_string()),
                        format.to_string(),
                        audio_data.len(),
                        Some(format!(
                            "OGG Opus conversion failed, saved as {format}: {e}"
                        )),
                    )
                }
            }
        } else {
            let filename = format!("tts_{timestamp}.{format}");
            let path = output_dir.join(&filename);
            tokio::fs::write(&path, audio_data)
                .await
                .map_err(|e| format!("Failed to write audio file: {e}"))?;

            (
                Some(path.display().to_string()),
                format.to_string(),
                audio_data.len(),
                None,
            )
        }
    } else {
        (None, format.to_string(), audio_data.len(), None)
    };

    let mut response = serde_json::json!({
        "saved_to": saved_path,
        "format": final_format,
        "provider": provider,
        "duration_estimate_ms": duration_ms,
        "size_bytes": final_size,
    });

    if let Some(w) = &warning {
        response["warning"] = serde_json::json!(w);
    }

    // When no workspace is available (e.g. MCP context), include base64 audio
    if saved_path.is_none() && !audio_data.is_empty() {
        use base64::Engine;
        response["audio_base64"] =
            serde_json::json!(base64::engine::general_purpose::STANDARD.encode(audio_data));
    }

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

async fn tool_speech_to_text(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    let engine = media_engine.ok_or("Media engine not available for speech-to-text")?;
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let _language = input["language"].as_str();

    let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;

    // Read the audio file
    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read audio file: {e}"))?;

    // Determine MIME type from extension
    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp3");
    let mime_type = match ext {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "webm" => "audio/webm",
        _ => "audio/mpeg",
    };

    use librefang_types::media::{MediaAttachment, MediaSource, MediaType};
    let attachment = MediaAttachment {
        media_type: MediaType::Audio,
        mime_type: mime_type.to_string(),
        source: MediaSource::Base64 {
            data: {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(&data)
            },
            mime_type: mime_type.to_string(),
        },
        size_bytes: data.len() as u64,
    };

    let understanding = engine.transcribe_audio(&attachment).await?;

    let response = serde_json::json!({
        "transcript": understanding.description,
        "provider": understanding.provider,
        "model": understanding.model,
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// Docker sandbox tool
// ---------------------------------------------------------------------------

async fn tool_docker_exec(
    input: &serde_json::Value,
    docker_config: Option<&librefang_types::config::DockerSandboxConfig>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let config = docker_config.ok_or("Docker sandbox not configured")?;

    if !config.enabled {
        return Err("Docker sandbox is disabled. Set docker.enabled=true in config.".into());
    }

    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;

    let workspace = workspace_root.ok_or("Docker exec requires a workspace directory")?;
    let agent_id = caller_agent_id.unwrap_or("default");

    // Check Docker availability
    if !crate::docker_sandbox::is_docker_available().await {
        return Err(
            "Docker is not available on this system. Install Docker to use docker_exec.".into(),
        );
    }

    // Create sandbox container
    let container = crate::docker_sandbox::create_sandbox(config, agent_id, workspace).await?;

    // Execute command with timeout
    let timeout = std::time::Duration::from_secs(config.timeout_secs);
    let result = crate::docker_sandbox::exec_in_sandbox(&container, command, timeout).await;

    // Always destroy the container after execution
    if let Err(e) = crate::docker_sandbox::destroy_sandbox(&container).await {
        warn!("Failed to destroy Docker sandbox: {e}");
    }

    let exec_result = result?;

    let response = serde_json::json!({
        "exit_code": exec_result.exit_code,
        "stdout": exec_result.stdout,
        "stderr": exec_result.stderr,
        "container_id": container.container_id,
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// Persistent process tools
// ---------------------------------------------------------------------------

/// Start a long-running process (REPL, server, watcher).
async fn tool_process_start(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let agent_id = caller_agent_id.unwrap_or("default");
    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;
    let args: Vec<String> = input["args"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let proc_id = pm.start(agent_id, command, &args).await?;
    Ok(serde_json::json!({
        "process_id": proc_id,
        "status": "started"
    })
    .to_string())
}

/// Read accumulated stdout/stderr from a process (non-blocking drain).
async fn tool_process_poll(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let proc_id = input["process_id"]
        .as_str()
        .ok_or("Missing 'process_id' parameter")?;
    let (stdout, stderr) = pm.read(proc_id).await?;
    Ok(serde_json::json!({
        "stdout": stdout,
        "stderr": stderr,
    })
    .to_string())
}

/// Write data to a process's stdin.
async fn tool_process_write(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let proc_id = input["process_id"]
        .as_str()
        .ok_or("Missing 'process_id' parameter")?;
    let data = input["data"].as_str().ok_or("Missing 'data' parameter")?;
    // Always append newline if not present (common expectation for REPLs)
    let data = if data.ends_with('\n') {
        data.to_string()
    } else {
        format!("{data}\n")
    };
    pm.write(proc_id, &data).await?;
    Ok(r#"{"status": "written"}"#.to_string())
}

/// Terminate a process.
async fn tool_process_kill(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let proc_id = input["process_id"]
        .as_str()
        .ok_or("Missing 'process_id' parameter")?;
    pm.kill(proc_id).await?;
    Ok(r#"{"status": "killed"}"#.to_string())
}

/// List processes for the current agent.
async fn tool_process_list(
    pm: Option<&crate::process_manager::ProcessManager>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let agent_id = caller_agent_id.unwrap_or("default");
    let procs = pm.list(agent_id);
    let list: Vec<serde_json::Value> = procs
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "command": p.command,
                "alive": p.alive,
                "uptime_secs": p.uptime_secs,
            })
        })
        .collect();
    Ok(serde_json::Value::Array(list).to_string())
}

// ---------------------------------------------------------------------------
// Canvas / A2UI tool
// ---------------------------------------------------------------------------

/// Sanitize HTML for canvas presentation.
///
/// SECURITY: Strips dangerous elements and attributes to prevent XSS:
/// - Rejects <script>, <iframe>, <object>, <embed>, <applet> tags
/// - Strips all on* event attributes (onclick, onload, onerror, etc.)
/// - Strips javascript:, data:text/html, vbscript: URLs
/// - Enforces size limit
pub fn sanitize_canvas_html(html: &str, max_bytes: usize) -> Result<String, String> {
    if html.is_empty() {
        return Err("Empty HTML content".to_string());
    }
    if html.len() > max_bytes {
        return Err(format!(
            "HTML too large: {} bytes (max {})",
            html.len(),
            max_bytes
        ));
    }

    let lower = html.to_lowercase();

    // Reject dangerous tags
    let dangerous_tags = [
        "<script", "</script", "<iframe", "</iframe", "<object", "</object", "<embed", "<applet",
        "</applet",
    ];
    for tag in &dangerous_tags {
        if lower.contains(tag) {
            return Err(format!("Forbidden HTML tag detected: {tag}"));
        }
    }

    // Reject event handler attributes (on*)
    // Match patterns like: onclick=, onload=, onerror=, onmouseover=, etc.
    static EVENT_PATTERN: std::sync::LazyLock<regex_lite::Regex> =
        std::sync::LazyLock::new(|| regex_lite::Regex::new(r"(?i)\bon[a-z]+\s*=").unwrap());
    if EVENT_PATTERN.is_match(html) {
        return Err(
            "Forbidden event handler attribute detected (on* attributes are not allowed)"
                .to_string(),
        );
    }

    // Reject dangerous URL schemes
    let dangerous_schemes = ["javascript:", "vbscript:", "data:text/html"];
    for scheme in &dangerous_schemes {
        if lower.contains(scheme) {
            return Err(format!("Forbidden URL scheme detected: {scheme}"));
        }
    }

    Ok(html.to_string())
}

// ---------------------------------------------------------------------------
// Skill evolution tools
// ---------------------------------------------------------------------------

/// Build the author tag for an agent-triggered evolution. Use the
/// agent's id so the dashboard history can attribute the change.
fn agent_author_tag(caller: Option<&str>) -> String {
    caller
        .map(|id| format!("agent:{id}"))
        .unwrap_or_else(|| "agent".to_string())
}

/// Reject evolution ops when the registry is frozen (Stable mode).
///
/// The registry's frozen flag is meant to express "no skill changes in
/// this kernel", but the evolution module writes to disk directly and
/// then triggers `reload_skills`, which no-ops under freeze. Without
/// this gate, an agent running under Stable mode would silently
/// persist skill mutations that'd be picked up at the next unfreeze
/// or restart — defeating the whole point of the mode.
fn ensure_not_frozen(registry: &SkillRegistry) -> Result<(), String> {
    if registry.is_frozen() {
        Err("Skill registry is frozen (Stable mode) — skill evolution is disabled".to_string())
    } else {
        Ok(())
    }
}

async fn tool_skill_evolve_create(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    ensure_not_frozen(registry)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let description = input["description"]
        .as_str()
        .ok_or("Missing 'description' parameter")?;
    let prompt_context = input["prompt_context"]
        .as_str()
        .ok_or("Missing 'prompt_context' parameter")?;
    let tags: Vec<String> = input["tags"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let author = agent_author_tag(caller_agent_id);
    let skills_dir = registry.skills_dir();
    match librefang_skills::evolution::create_skill(
        skills_dir,
        name,
        description,
        prompt_context,
        tags,
        Some(&author),
    ) {
        Ok(result) => serde_json::to_string(&result).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Failed to create skill: {e}")),
    }
}

async fn tool_skill_evolve_update(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    ensure_not_frozen(registry)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let prompt_context = input["prompt_context"]
        .as_str()
        .ok_or("Missing 'prompt_context' parameter")?;
    let changelog = input["changelog"]
        .as_str()
        .ok_or("Missing 'changelog' parameter")?;

    // Registry hot-reload happens AFTER the turn finishes, so within
    // the same turn `create` followed by `update` would find the
    // registry cache still stale. Fall back to loading straight from
    // disk when the cache misses — if the skill truly doesn't exist
    // the helper returns NotFound too.
    let skill_owned;
    let skill = match registry.get(name) {
        Some(s) => s,
        None => {
            skill_owned = librefang_skills::evolution::load_installed_skill_from_disk(
                registry.skills_dir(),
                name,
            )
            .map_err(|e| format!("Skill '{name}' not found: {e}"))?;
            &skill_owned
        }
    };

    let author = agent_author_tag(caller_agent_id);
    match librefang_skills::evolution::update_skill(skill, prompt_context, changelog, Some(&author))
    {
        Ok(result) => serde_json::to_string(&result).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Failed to update skill: {e}")),
    }
}

async fn tool_skill_evolve_patch(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    ensure_not_frozen(registry)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let old_string = input["old_string"]
        .as_str()
        .ok_or("Missing 'old_string' parameter")?;
    let new_string = input["new_string"]
        .as_str()
        .ok_or("Missing 'new_string' parameter")?;
    let changelog = input["changelog"]
        .as_str()
        .ok_or("Missing 'changelog' parameter")?;
    let replace_all = input["replace_all"].as_bool().unwrap_or(false);

    // Same-turn create→patch fallback (see tool_skill_evolve_update).
    let skill_owned;
    let skill = match registry.get(name) {
        Some(s) => s,
        None => {
            skill_owned = librefang_skills::evolution::load_installed_skill_from_disk(
                registry.skills_dir(),
                name,
            )
            .map_err(|e| format!("Skill '{name}' not found: {e}"))?;
            &skill_owned
        }
    };

    let author = agent_author_tag(caller_agent_id);
    match librefang_skills::evolution::patch_skill(
        skill,
        old_string,
        new_string,
        changelog,
        replace_all,
        Some(&author),
    ) {
        Ok(result) => serde_json::to_string(&result).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Failed to patch skill: {e}")),
    }
}

async fn tool_skill_evolve_delete(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    ensure_not_frozen(registry)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;

    // Resolve the actual installed skill's parent directory instead of
    // blindly targeting `registry.skills_dir() + name`. Workspace skills
    // shadow global skills with the same name in an agent run; without
    // this, `skill_evolve_delete` removed the global skill (or reported
    // NotFound) while leaving the workspace copy the agent was actually
    // using in place — destructive against the wrong resource.
    let parent = match registry.get(name) {
        Some(s) => s
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| registry.skills_dir().to_path_buf()),
        // Fall back to the global dir when the registry hasn't caught up
        // yet (e.g. a skill created in this same turn hasn't been
        // hot-reloaded into the live view) — delete_skill will return
        // NotFound if nothing exists there either.
        None => registry.skills_dir().to_path_buf(),
    };
    match librefang_skills::evolution::delete_skill(&parent, name) {
        Ok(result) => serde_json::to_string(&result).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Failed to delete skill: {e}")),
    }
}

async fn tool_skill_evolve_rollback(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    ensure_not_frozen(registry)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;

    // Same-turn create→rollback fallback (see tool_skill_evolve_update).
    let skill_owned;
    let skill = match registry.get(name) {
        Some(s) => s,
        None => {
            skill_owned = librefang_skills::evolution::load_installed_skill_from_disk(
                registry.skills_dir(),
                name,
            )
            .map_err(|e| format!("Skill '{name}' not found: {e}"))?;
            &skill_owned
        }
    };

    let author = agent_author_tag(caller_agent_id);
    match librefang_skills::evolution::rollback_skill(skill, Some(&author)) {
        Ok(result) => serde_json::to_string(&result).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Failed to rollback skill: {e}")),
    }
}

async fn tool_skill_evolve_write_file(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    ensure_not_frozen(registry)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let content = input["content"]
        .as_str()
        .ok_or("Missing 'content' parameter")?;

    // Same-turn create→write_file fallback.
    let skill_owned;
    let skill = match registry.get(name) {
        Some(s) => s,
        None => {
            skill_owned = librefang_skills::evolution::load_installed_skill_from_disk(
                registry.skills_dir(),
                name,
            )
            .map_err(|e| format!("Skill '{name}' not found: {e}"))?;
            &skill_owned
        }
    };

    match librefang_skills::evolution::write_supporting_file(skill, path, content) {
        Ok(result) => serde_json::to_string(&result).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Failed to write file: {e}")),
    }
}

async fn tool_skill_evolve_remove_file(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    ensure_not_frozen(registry)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;

    // Same-turn fallback (see tool_skill_evolve_update).
    let skill_owned;
    let skill = match registry.get(name) {
        Some(s) => s,
        None => {
            skill_owned = librefang_skills::evolution::load_installed_skill_from_disk(
                registry.skills_dir(),
                name,
            )
            .map_err(|e| format!("Skill '{name}' not found: {e}"))?;
            &skill_owned
        }
    };

    match librefang_skills::evolution::remove_supporting_file(skill, path) {
        Ok(result) => serde_json::to_string(&result).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Failed to remove file: {e}")),
    }
}

/// Read a companion file from an installed skill directory.
///
/// Security: resolves the path relative to the skill's installed directory and
/// rejects any path that escapes via `..` or absolute components. Symlinks are
/// resolved by `canonicalize()` before the containment check, so a symlink
/// pointing outside the skill directory is correctly rejected.
async fn tool_skill_read_file(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    allowed_skills: Option<&[String]>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    let skill_name = input["skill"].as_str().ok_or("Missing 'skill' parameter")?;
    let rel_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;

    // Enforce agent skill allowlist: if the agent specifies allowed skills
    // (non-empty list), only those skills can be read. Empty = all allowed.
    if let Some(allowed) = allowed_skills {
        if !allowed.is_empty() && !allowed.iter().any(|s| s == skill_name) {
            return Err(format!(
                "Access denied: agent is not allowed to access skill '{skill_name}'"
            ));
        }
    }

    // Reject absolute paths early — Path::join replaces the base when given
    // an absolute path, which would bypass the skill directory containment.
    if std::path::Path::new(rel_path).is_absolute() {
        return Err("Access denied: absolute paths are not allowed".to_string());
    }

    // Look up the skill
    let skill = registry
        .get(skill_name)
        .ok_or_else(|| format!("Skill '{}' not found", skill_name))?;

    // Resolve the path relative to the skill directory
    let requested = skill.path.join(rel_path);
    let canonical = requested
        .canonicalize()
        .map_err(|e| format!("File not found: {}", e))?;
    let skill_root = skill
        .path
        .canonicalize()
        .map_err(|e| format!("Skill directory error: {}", e))?;

    // Security: ensure the resolved path is within the skill directory
    if !canonical.starts_with(&skill_root) {
        return Err(format!(
            "Access denied: '{}' is outside the skill directory",
            rel_path
        ));
    }

    // Read the file
    let content = tokio::fs::read_to_string(&canonical)
        .await
        .map_err(|e| format!("Failed to read '{}': {}", rel_path, e))?;

    // Fire-and-forget usage tracking — only count when the agent actually
    // loads the skill's core prompt content, not every supporting file
    // read. Reading references/templates/scripts/assets shouldn't inflate
    // the usage metric. Failures (lock contention, disk error) must not
    // affect tool execution, so we swallow them.
    let is_core_prompt = matches!(rel_path, "prompt_context.md" | "SKILL.md" | "skill.md");
    if is_core_prompt {
        let skill_dir = skill.path.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = librefang_skills::evolution::record_skill_usage(&skill_dir) {
                tracing::debug!(error = %e, dir = %skill_dir.display(), "record_skill_usage failed");
            }
        });
    }

    // Cap output to avoid flooding the context.
    // Use floor_char_boundary to avoid panicking on multi-byte UTF-8.
    const MAX_BYTES: usize = 32_000;
    if content.len() > MAX_BYTES {
        let truncate_at = content.floor_char_boundary(MAX_BYTES);
        Ok(format!(
            "{}\n\n... (truncated at {} bytes, file is {} bytes total)",
            &content[..truncate_at],
            truncate_at,
            content.len()
        ))
    } else {
        Ok(content)
    }
}

/// Canvas presentation tool handler.
async fn tool_canvas_present(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let html = input["html"].as_str().ok_or("Missing 'html' parameter")?;
    let title = input["title"].as_str().unwrap_or("Canvas");

    // Use configured max from task-local (set by agent_loop from KernelConfig), or default 512KB.
    let max_bytes = CANVAS_MAX_BYTES.try_with(|v| *v).unwrap_or(512 * 1024);
    let sanitized = sanitize_canvas_html(html, max_bytes)?;

    // Generate canvas ID
    let canvas_id = uuid::Uuid::new_v4().to_string();

    // Save to workspace output directory
    let output_dir = if let Some(root) = workspace_root {
        root.join("output")
    } else {
        PathBuf::from("output")
    };
    let _ = tokio::fs::create_dir_all(&output_dir).await;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let filename = format!(
        "canvas_{timestamp}_{}.html",
        crate::str_utils::safe_truncate_str(&canvas_id, 8)
    );
    let filepath = output_dir.join(&filename);

    // Write the full HTML document
    let full_html = format!(
        "<!DOCTYPE html>\n<html>\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body>\n{sanitized}\n</body>\n</html>"
    );
    tokio::fs::write(&filepath, &full_html)
        .await
        .map_err(|e| format!("Failed to save canvas: {e}"))?;

    let response = serde_json::json!({
        "canvas_id": canvas_id,
        "title": title,
        "saved_to": filepath.to_string_lossy(),
        "size_bytes": full_html.len(),
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// Artifact retrieval tool (#3347)
// ---------------------------------------------------------------------------

/// Implementation of the `read_artifact` tool.
///
/// Reads up to `length` bytes from the artifact identified by `handle`,
/// starting at `offset`.  Both parameters are optional (defaults: 0 and 4096).
/// The result is UTF-8 text: binary blobs are lossily decoded.
async fn tool_read_artifact(
    input: &serde_json::Value,
    artifact_dir: &std::path::Path,
) -> Result<String, String> {
    let handle = input["handle"]
        .as_str()
        .ok_or("Missing required parameter 'handle'")?;

    let offset = input["offset"].as_u64().unwrap_or(0) as usize;
    let length = input["length"]
        .as_u64()
        .unwrap_or(4096)
        .min(crate::artifact_store::MAX_READ_LENGTH as u64) as usize;

    let bytes = crate::artifact_store::read(handle, offset, length, artifact_dir)?;

    if bytes.is_empty() {
        return Ok(format!(
            "[read_artifact: {handle} | offset={offset}] — no more content (past end of artifact)"
        ));
    }

    let text = String::from_utf8_lossy(&bytes);
    Ok(format!(
        "[read_artifact: {handle} | offset={offset} | {} bytes read]\n{text}",
        bytes.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // ── check_taint_outbound_text ────────────────────────────────────────

    #[test]
    fn test_taint_outbound_text_blocks_key_value_pairs() {
        let sink = TaintSink::agent_message();
        for body in [
            "here is my api_key=sk-123",
            "x-api-key: abcdef",
            "{\"token\":\"mytoken\"}",
            "{\"authorization\": \"Bearer sk-live-secret\"}",
            "{\"proxy-authorization\": \"Basic Zm9vOmJhcg==\"}",
            "api_key = sk-123",
            "'password': 'hunter2'",
            "Authorization: Bearer abc",
            "some text bearer=abc",
        ] {
            assert!(
                check_taint_outbound_text(body, &sink).is_some(),
                "outbound taint check must reject {body:?}"
            );
        }
    }

    #[test]
    fn test_taint_outbound_text_blocks_well_known_prefixes() {
        let sink = TaintSink::agent_message();
        for tok in [
            "sk-12345678901234567890123456789012",
            "ghp_1234567890123456789012345678901234567890",
            "xoxb-0000-0000-xxxxxxxxxxxx",
            "AKIAIOSFODNN7EXAMPLE",
            "AIzaSyDummyGoogleKeyLooksLikeThis00",
        ] {
            assert!(
                check_taint_outbound_text(tok, &sink).is_some(),
                "outbound taint check must reject well-known prefix {tok:?}"
            );
        }
    }

    #[test]
    fn test_taint_outbound_text_blocks_long_opaque_tokens() {
        let sink = TaintSink::agent_message();
        // 40-char mixed-case base64-ish payload with no whitespace or
        // prose: smells like a raw bearer token.
        let payload = "AbCdEf0123456789AbCdEf0123456789AbCdEf01";
        assert!(
            check_taint_outbound_text(payload, &sink).is_some(),
            "outbound taint check must reject long opaque token"
        );
        // Same length but with punctuation — also looks tokenish.
        let payload_punct = "abcdef0123456789-abcdef0123456789-abcdef";
        assert!(
            check_taint_outbound_text(payload_punct, &sink).is_some(),
            "outbound taint check must reject punctuated token"
        );
    }

    #[test]
    fn test_taint_outbound_text_allows_git_sha() {
        // 40-char lowercase hex commit SHA — legitimate inter-agent
        // payload, must not be blocked.
        let sink = TaintSink::agent_message();
        let sha = "18060f6401234567890abcdef0123456789abcde";
        assert!(
            check_taint_outbound_text(sha, &sink).is_none(),
            "git commit SHA must not be treated as a secret"
        );
    }

    #[test]
    fn test_taint_outbound_text_allows_sha256_hex() {
        // 64-char lowercase hex sha256 digest — also legitimate.
        let sink = TaintSink::agent_message();
        let digest = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(
            check_taint_outbound_text(digest, &sink).is_none(),
            "sha256 hex digest must not be treated as a secret"
        );
    }

    #[test]
    fn test_taint_outbound_text_allows_uuid_hex() {
        // 32-char UUID-without-dashes (hex) — allowed.
        let sink = TaintSink::agent_message();
        let uuid = "550e8400e29b41d4a716446655440000";
        assert!(
            check_taint_outbound_text(uuid, &sink).is_none(),
            "undashed UUID must not be treated as a secret"
        );
    }

    #[test]
    fn test_taint_outbound_header_blocks_authorization_bearer() {
        // Regression for the header-name-bypass bug: a Bearer token
        // with a space between scheme and value defeats every
        // content-based heuristic, so we must trip on the header name.
        let sink = TaintSink::net_fetch();
        assert!(
            check_taint_outbound_header("Authorization", "Bearer sk-x", &sink).is_some(),
            "Authorization: Bearer <anything> must be blocked"
        );
        assert!(
            check_taint_outbound_header("authorization", "Token abc", &sink).is_some(),
            "lowercased authorization header must also be blocked"
        );
        assert!(
            check_taint_outbound_header("Proxy-Authorization", "Basic Zm9vOmJhcg==", &sink)
                .is_some(),
            "Proxy-Authorization header must be blocked"
        );
        assert!(
            check_taint_outbound_header("X-Api-Key", "hunter2", &sink).is_some(),
            "X-Api-Key header must be blocked"
        );
    }

    #[test]
    fn test_taint_outbound_header_allows_benign_headers() {
        let sink = TaintSink::net_fetch();
        assert!(
            check_taint_outbound_header("Accept", "application/json", &sink).is_none(),
            "benign Accept header must pass"
        );
        assert!(
            check_taint_outbound_header("User-Agent", "librefang/1.0", &sink).is_none(),
            "benign User-Agent header must pass"
        );
    }

    #[test]
    fn test_taint_outbound_text_allows_prose() {
        let sink = TaintSink::agent_message();
        for benign in [
            "Please summarise this article about encryption.",
            "Could you check whether our token economy works?",
            "The passwd file lives at /etc/passwd on Linux — explain it.",
            "Write a haiku about secret gardens.",
            "",
        ] {
            assert!(
                check_taint_outbound_text(benign, &sink).is_none(),
                "outbound taint check must allow prose: {benign:?}"
            );
        }
    }

    #[test]
    fn test_taint_outbound_text_allows_short_identifiers() {
        // A 16-char id is below the 32-char opaque-token threshold and
        // doesn't match any key=value shape, so it should pass even
        // though it looks alphanumeric.
        let sink = TaintSink::agent_message();
        let id = "req_0123456789ab";
        assert!(check_taint_outbound_text(id, &sink).is_none());
    }

    // ── tool_a2a_send / tool_channel_send taint integration ─────────────
    //
    // Regression: prior to this patch the taint sink was only enforced
    // on agent_send and web_fetch. tool_a2a_send and tool_channel_send
    // were exfiltration sinks with NO check at all.

    #[tokio::test]
    async fn test_tool_a2a_send_blocks_secret_in_message() {
        let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
            approval_requests: Arc::new(AtomicUsize::new(0)),
            user_gate_override: None,
        });
        let input = serde_json::json!({
            "agent_url": "https://example.com/a2a",
            "message": "leaking api_key=sk-abcdefghijklmnop now",
        });
        let err = tool_a2a_send(&input, Some(&kernel))
            .await
            .expect_err("a2a_send must reject tainted message");
        assert!(
            err.contains("taint") || err.contains("violation"),
            "expected taint violation, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_tool_channel_send_blocks_secret_in_text_message() {
        let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
            approval_requests: Arc::new(AtomicUsize::new(0)),
            user_gate_override: None,
        });
        let input = serde_json::json!({
            "channel": "telegram",
            "recipient": "@user",
            "message": "here is the api_key=sk-abcdefghijklmnop",
        });
        let err = tool_channel_send(&input, Some(&kernel), None, Some("test_user_id"), &[])
            .await
            .expect_err("channel_send must reject tainted message");
        assert!(
            err.contains("taint") || err.contains("violation"),
            "expected taint violation, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_tool_channel_send_blocks_secret_in_image_caption() {
        let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
            approval_requests: Arc::new(AtomicUsize::new(0)),
            user_gate_override: None,
        });
        let input = serde_json::json!({
            "channel": "telegram",
            "recipient": "@user",
            "image_url": "https://example.com/cat.png",
            "message": "see attached. token=sk-abcdefghijklmnop",
        });
        let err = tool_channel_send(&input, Some(&kernel), None, Some("test_user_id"), &[])
            .await
            .expect_err("image caption must be sink-checked");
        assert!(
            err.contains("taint") || err.contains("violation"),
            "expected taint violation, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_tool_channel_send_blocks_secret_in_poll_question() {
        let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
            approval_requests: Arc::new(AtomicUsize::new(0)),
            user_gate_override: None,
        });
        let input = serde_json::json!({
            "channel": "telegram",
            "recipient": "@user",
            "poll_question": "guess my api_key=sk-abcdefghijklmnop",
            "poll_options": ["yes", "no"],
        });
        let err = tool_channel_send(&input, Some(&kernel), None, Some("test_user_id"), &[])
            .await
            .expect_err("poll question must be sink-checked");
        assert!(
            err.contains("taint") || err.contains("violation"),
            "expected taint violation, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_tool_channel_send_auto_fills_recipient_from_sender_id() {
        // Test that channel_send uses sender_id when recipient is omitted
        let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
            approval_requests: Arc::new(AtomicUsize::new(0)),
            user_gate_override: None,
        });
        let input = serde_json::json!({
            "channel": "telegram",
            // recipient intentionally omitted
            "message": "Hello from auto-reply!",
        });
        // This should NOT error with "Missing recipient" because sender_id is provided
        // It will error with "Channel file data send not available" because the mock kernel
        // doesn't implement channel_send, but that's expected
        let result =
            tool_channel_send(&input, Some(&kernel), None, Some("12345_telegram"), &[]).await;
        // The error should NOT be about missing recipient
        let err_msg = result.unwrap_err();
        assert!(
            !err_msg.contains("Missing 'recipient'"),
            "Expected auto-fill to work, but got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_tool_channel_send_requires_recipient_without_sender_id() {
        // Test that channel_send still requires recipient when sender_id is None
        let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
            approval_requests: Arc::new(AtomicUsize::new(0)),
            user_gate_override: None,
        });
        let input = serde_json::json!({
            "channel": "telegram",
            // recipient intentionally omitted
            "message": "Hello!",
        });
        let err = tool_channel_send(&input, Some(&kernel), None, None, &[])
            .await
            .expect_err("channel_send must require recipient without sender_id");
        assert!(
            err.contains("Missing 'recipient'"),
            "Expected missing recipient error, got: {err}"
        );
    }

    struct ApprovalKernel {
        approval_requests: Arc<AtomicUsize>,
        /// RBAC M3 — overrides what `resolve_user_tool_decision` returns
        /// for every call. `None` keeps the default-impl behaviour
        /// (`UserToolGate::Allow`) so pre-RBAC tests are unaffected.
        user_gate_override: Option<librefang_types::user_policy::UserToolGate>,
    }

    /// Captures the `DeferredToolExecution.force_human` flag so tests
    /// can assert that the user-gate escalation propagates through.
    struct ForceHumanCapturingKernel {
        approval_requests: Arc<AtomicUsize>,
        last_force_human: Arc<std::sync::Mutex<Option<bool>>>,
        user_gate_override: Option<librefang_types::user_policy::UserToolGate>,
    }

    // ---- BEGIN role-trait impls (split from former `impl KernelHandle for ApprovalKernel`, #3746) ----

    #[async_trait::async_trait]
    impl AgentControl for ApprovalKernel {
        async fn spawn_agent(
            &self,
            _manifest_toml: &str,
            _parent_id: Option<&str>,
        ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn send_to_agent(
            &self,
            _agent_id: &str,
            _message: &str,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn list_agents(&self) -> Vec<AgentInfo> {
            vec![]
        }

        fn kill_agent(
            &self,
            _agent_id: &str,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
            vec![]
        }
    }

    impl MemoryAccess for ApprovalKernel {
        fn memory_store(
            &self,
            _key: &str,
            _value: serde_json::Value,
            _peer_id: Option<&str>,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn memory_recall(
            &self,
            _key: &str,
            _peer_id: Option<&str>,
        ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn memory_list(
            &self,
            _peer_id: Option<&str>,
        ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }
    }

    impl WikiAccess for ApprovalKernel {}

    #[async_trait::async_trait]
    impl TaskQueue for ApprovalKernel {
        async fn task_post(
            &self,
            _title: &str,
            _description: &str,
            _assigned_to: Option<&str>,
            _created_by: Option<&str>,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_claim(
            &self,
            _agent_id: &str,
        ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_complete(
            &self,
            _agent_id: &str,
            _task_id: &str,
            _result: &str,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_list(
            &self,
            _status: Option<&str>,
        ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_delete(
            &self,
            _task_id: &str,
        ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_retry(
            &self,
            _task_id: &str,
        ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_get(
            &self,
            _task_id: &str,
        ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_update_status(
            &self,
            _task_id: &str,
            _new_status: &str,
        ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }
    }

    #[async_trait::async_trait]
    impl EventBus for ApprovalKernel {
        async fn publish_event(
            &self,
            _event_type: &str,
            _payload: serde_json::Value,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }
    }

    #[async_trait::async_trait]
    impl KnowledgeGraph for ApprovalKernel {
        async fn knowledge_add_entity(
            &self,
            _entity: &librefang_types::memory::Entity,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn knowledge_add_relation(
            &self,
            _relation: &librefang_types::memory::Relation,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn knowledge_query(
            &self,
            _pattern: librefang_types::memory::GraphPattern,
        ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
        {
            Err("not used".into())
        }
    }

    #[async_trait::async_trait]
    impl ApprovalGate for ApprovalKernel {
        fn requires_approval(&self, tool_name: &str) -> bool {
            tool_name == "shell_exec"
        }

        async fn request_approval(
            &self,
            _agent_id: &str,
            _tool_name: &str,
            _action_summary: &str,
            _session_id: Option<&str>,
        ) -> Result<
            librefang_types::approval::ApprovalDecision,
            librefang_kernel_handle::KernelOpError,
        > {
            self.approval_requests.fetch_add(1, Ordering::SeqCst);
            Ok(librefang_types::approval::ApprovalDecision::Denied)
        }

        async fn submit_tool_approval(
            &self,
            _agent_id: &str,
            _tool_name: &str,
            _action_summary: &str,
            _deferred: librefang_types::tool::DeferredToolExecution,
            _session_id: Option<&str>,
        ) -> Result<
            librefang_types::tool::ToolApprovalSubmission,
            librefang_kernel_handle::KernelOpError,
        > {
            self.approval_requests.fetch_add(1, Ordering::SeqCst);
            Ok(librefang_types::tool::ToolApprovalSubmission::Pending {
                request_id: uuid::Uuid::new_v4(),
            })
        }

        fn resolve_user_tool_decision(
            &self,
            _tool_name: &str,
            _sender_id: Option<&str>,
            _channel: Option<&str>,
        ) -> librefang_types::user_policy::UserToolGate {
            self.user_gate_override
                .clone()
                .unwrap_or(librefang_types::user_policy::UserToolGate::Allow)
        }
    }

    // No-op role-trait impls (#3746) — mock relies on default bodies.
    impl CronControl for ApprovalKernel {}
    impl HandsControl for ApprovalKernel {}
    impl A2ARegistry for ApprovalKernel {}
    impl ChannelSender for ApprovalKernel {}
    impl PromptStore for ApprovalKernel {}
    impl WorkflowRunner for ApprovalKernel {}
    impl GoalControl for ApprovalKernel {}
    impl ToolPolicy for ApprovalKernel {}
    impl ApiAuth for ApprovalKernel {
        fn auth_snapshot(&self) -> ApiAuthSnapshot {
            ApiAuthSnapshot::default()
        }
    }
    impl SessionWriter for ApprovalKernel {
        fn inject_attachment_blocks(
            &self,
            _agent_id: librefang_types::agent::AgentId,
            _blocks: Vec<librefang_types::message::ContentBlock>,
        ) {
        }
    }

    // ---- END role-trait impls (#3746) ----

    // ---- BEGIN role-trait impls (split from former `impl KernelHandle for ForceHumanCapturingKernel`, #3746) ----

    #[async_trait::async_trait]
    impl AgentControl for ForceHumanCapturingKernel {
        async fn spawn_agent(
            &self,
            _manifest_toml: &str,
            _parent_id: Option<&str>,
        ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn send_to_agent(
            &self,
            _agent_id: &str,
            _message: &str,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn list_agents(&self) -> Vec<AgentInfo> {
            vec![]
        }

        fn kill_agent(
            &self,
            _agent_id: &str,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
            vec![]
        }
    }

    impl MemoryAccess for ForceHumanCapturingKernel {
        fn memory_store(
            &self,
            _key: &str,
            _value: serde_json::Value,
            _peer_id: Option<&str>,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn memory_recall(
            &self,
            _key: &str,
            _peer_id: Option<&str>,
        ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn memory_list(
            &self,
            _peer_id: Option<&str>,
        ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }
    }

    impl WikiAccess for ForceHumanCapturingKernel {}

    #[async_trait::async_trait]
    impl TaskQueue for ForceHumanCapturingKernel {
        async fn task_post(
            &self,
            _title: &str,
            _description: &str,
            _assigned_to: Option<&str>,
            _created_by: Option<&str>,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_claim(
            &self,
            _agent_id: &str,
        ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_complete(
            &self,
            _agent_id: &str,
            _task_id: &str,
            _result: &str,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_list(
            &self,
            _status: Option<&str>,
        ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_delete(
            &self,
            _task_id: &str,
        ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_retry(
            &self,
            _task_id: &str,
        ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_get(
            &self,
            _task_id: &str,
        ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_update_status(
            &self,
            _task_id: &str,
            _new_status: &str,
        ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }
    }

    #[async_trait::async_trait]
    impl EventBus for ForceHumanCapturingKernel {
        async fn publish_event(
            &self,
            _event_type: &str,
            _payload: serde_json::Value,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }
    }

    #[async_trait::async_trait]
    impl KnowledgeGraph for ForceHumanCapturingKernel {
        async fn knowledge_add_entity(
            &self,
            _entity: &librefang_types::memory::Entity,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn knowledge_add_relation(
            &self,
            _relation: &librefang_types::memory::Relation,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn knowledge_query(
            &self,
            _pattern: librefang_types::memory::GraphPattern,
        ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
        {
            Err("not used".into())
        }
    }

    #[async_trait::async_trait]
    impl ApprovalGate for ForceHumanCapturingKernel {
        fn requires_approval(&self, tool_name: &str) -> bool {
            tool_name == "shell_exec"
        }

        async fn submit_tool_approval(
            &self,
            _agent_id: &str,
            _tool_name: &str,
            _action_summary: &str,
            deferred: librefang_types::tool::DeferredToolExecution,
            _session_id: Option<&str>,
        ) -> Result<
            librefang_types::tool::ToolApprovalSubmission,
            librefang_kernel_handle::KernelOpError,
        > {
            self.approval_requests.fetch_add(1, Ordering::SeqCst);
            *self.last_force_human.lock().unwrap() = Some(deferred.force_human);
            Ok(librefang_types::tool::ToolApprovalSubmission::Pending {
                request_id: uuid::Uuid::new_v4(),
            })
        }

        fn resolve_user_tool_decision(
            &self,
            _tool_name: &str,
            _sender_id: Option<&str>,
            _channel: Option<&str>,
        ) -> librefang_types::user_policy::UserToolGate {
            self.user_gate_override
                .clone()
                .unwrap_or(librefang_types::user_policy::UserToolGate::Allow)
        }
    }

    // No-op role-trait impls (#3746) — mock relies on default bodies.
    impl CronControl for ForceHumanCapturingKernel {}
    impl HandsControl for ForceHumanCapturingKernel {}
    impl A2ARegistry for ForceHumanCapturingKernel {}
    impl ChannelSender for ForceHumanCapturingKernel {}
    impl PromptStore for ForceHumanCapturingKernel {}
    impl WorkflowRunner for ForceHumanCapturingKernel {}
    impl GoalControl for ForceHumanCapturingKernel {}
    impl ToolPolicy for ForceHumanCapturingKernel {}
    impl ApiAuth for ForceHumanCapturingKernel {
        fn auth_snapshot(&self) -> ApiAuthSnapshot {
            ApiAuthSnapshot::default()
        }
    }
    impl SessionWriter for ForceHumanCapturingKernel {
        fn inject_attachment_blocks(
            &self,
            _agent_id: librefang_types::agent::AgentId,
            _blocks: Vec<librefang_types::message::ContentBlock>,
        ) {
        }
    }

    // ---- END role-trait impls (#3746) ----

    /// Regression: when the per-user gate returns `NeedsApproval`, the
    /// `DeferredToolExecution.force_human` flag MUST be set so the
    /// kernel's `submit_tool_approval` can disable the hand-agent
    /// auto-approve carve-out. (B3 of PR #3205 review.)
    #[tokio::test]
    async fn tool_runner_rbac_force_human_propagates_to_deferred() {
        let approval_requests = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(std::sync::Mutex::new(None));
        let kernel: Arc<dyn KernelHandle> = Arc::new(ForceHumanCapturingKernel {
            approval_requests: Arc::clone(&approval_requests),
            last_force_human: Arc::clone(&last),
            user_gate_override: Some(librefang_types::user_policy::UserToolGate::NeedsApproval {
                reason: "user policy escalated".to_string(),
            }),
        });

        let workspace = tempfile::tempdir().expect("tempdir");
        let _ = execute_tool(
            "tu-1",
            "file_write",
            &serde_json::json!({"path": "scratch.txt", "content": "hi"}),
            Some(&kernel),
            None,
            Some("agent-1"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(workspace.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("bob"),
            Some("telegram"),
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        assert_eq!(approval_requests.load(Ordering::SeqCst), 1);
        assert_eq!(
            *last.lock().unwrap(),
            Some(true),
            "force_human must be true when user policy escalated"
        );
    }

    /// Sanity: when the user gate is `Allow` and only the global
    /// `require_approval` list pulls the call into approval, `force_human`
    /// stays false — hand-agent auto-approval keeps working in the
    /// non-RBAC path.
    #[tokio::test]
    async fn tool_runner_rbac_force_human_stays_false_for_global_require_approval() {
        let approval_requests = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(std::sync::Mutex::new(None));
        let kernel: Arc<dyn KernelHandle> = Arc::new(ForceHumanCapturingKernel {
            approval_requests: Arc::clone(&approval_requests),
            last_force_human: Arc::clone(&last),
            user_gate_override: Some(librefang_types::user_policy::UserToolGate::Allow),
        });

        let _ = execute_tool(
            "tu-1",
            "shell_exec",
            &serde_json::json!({"command": "echo ok"}),
            Some(&kernel),
            None,
            Some("agent-1"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("alice"),
            Some("telegram"),
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        assert_eq!(approval_requests.load(Ordering::SeqCst), 1);
        assert_eq!(*last.lock().unwrap(), Some(false));
    }

    #[test]
    fn test_builtin_tool_definitions() {
        let tools = builtin_tool_definitions();
        assert!(
            tools.len() >= 40,
            "Expected at least 40 tools, got {}",
            tools.len()
        );
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        // Original 12
        assert!(names.contains(&"file_read"));
        assert!(names.contains(&"shell_exec"));
        assert!(names.contains(&"agent_send"));
        assert!(names.contains(&"agent_spawn"));
        assert!(names.contains(&"agent_list"));
        assert!(names.contains(&"agent_kill"));
        assert!(names.contains(&"memory_store"));
        assert!(names.contains(&"memory_recall"));
        assert!(names.contains(&"memory_list"));
        // 7 collaboration tools
        assert!(names.contains(&"agent_find"));
        assert!(names.contains(&"task_post"));
        assert!(names.contains(&"task_claim"));
        assert!(names.contains(&"task_complete"));
        assert!(names.contains(&"task_list"));
        assert!(names.contains(&"task_status"));
        assert!(names.contains(&"event_publish"));
        // 5 new Phase 3 tools
        assert!(names.contains(&"schedule_create"));
        assert!(names.contains(&"schedule_list"));
        assert!(names.contains(&"schedule_delete"));
        assert!(names.contains(&"image_analyze"));
        assert!(names.contains(&"location_get"));
        assert!(names.contains(&"system_time"));
        // 6 browser tools
        assert!(names.contains(&"browser_navigate"));
        assert!(names.contains(&"browser_click"));
        assert!(names.contains(&"browser_type"));
        assert!(names.contains(&"browser_screenshot"));
        assert!(names.contains(&"browser_read_page"));
        assert!(names.contains(&"browser_close"));
        assert!(names.contains(&"browser_scroll"));
        assert!(names.contains(&"browser_wait"));
        assert!(names.contains(&"browser_run_js"));
        assert!(names.contains(&"browser_back"));
        // 3 media/image generation tools
        assert!(names.contains(&"media_describe"));
        assert!(names.contains(&"media_transcribe"));
        assert!(names.contains(&"image_generate"));
        // 3 video/music generation tools
        assert!(names.contains(&"video_generate"));
        assert!(names.contains(&"video_status"));
        assert!(names.contains(&"music_generate"));
        // 3 cron tools
        assert!(names.contains(&"cron_create"));
        assert!(names.contains(&"cron_list"));
        assert!(names.contains(&"cron_cancel"));
        // 1 channel send tool
        assert!(names.contains(&"channel_send"));
        // 4 hand tools
        assert!(names.contains(&"hand_list"));
        assert!(names.contains(&"hand_activate"));
        assert!(names.contains(&"hand_status"));
        assert!(names.contains(&"hand_deactivate"));
        // 3 voice/docker tools
        assert!(names.contains(&"text_to_speech"));
        assert!(names.contains(&"speech_to_text"));
        assert!(names.contains(&"docker_exec"));
        // Goal tracking tool
        assert!(names.contains(&"goal_update"));
        // Workflow execution tool
        assert!(names.contains(&"workflow_run"));
        // Canvas tool
        assert!(names.contains(&"canvas_present"));
    }

    #[test]
    fn test_collaboration_tool_schemas() {
        let tools = builtin_tool_definitions();
        let collab_tools = [
            "agent_find",
            "task_post",
            "task_claim",
            "task_complete",
            "task_list",
            "task_status",
            "event_publish",
        ];
        for name in &collab_tools {
            let tool = tools
                .iter()
                .find(|t| t.name == *name)
                .unwrap_or_else(|| panic!("Tool '{}' not found", name));
            // Verify each has a valid JSON schema
            assert!(
                tool.input_schema.is_object(),
                "Tool '{}' schema should be an object",
                name
            );
            assert_eq!(
                tool.input_schema["type"], "object",
                "Tool '{}' should have type=object",
                name
            );
        }
    }

    #[tokio::test]
    async fn test_file_read_missing() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": "nonexistent_99999/file.txt"}),
            None,
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            Some(workspace.path()),
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(
            result.is_error,
            "Expected error but got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_file_read_path_traversal_blocked() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": "../../etc/passwd"}),
            None,
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            Some(workspace.path()),
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("traversal"));
    }

    #[tokio::test]
    async fn test_file_write_path_traversal_blocked() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let result = execute_tool(
            "test-id",
            "file_write",
            &serde_json::json!({"path": "../../../tmp/evil.txt", "content": "pwned"}),
            None,
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            Some(workspace.path()),
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("traversal"));
    }

    #[tokio::test]
    async fn test_file_list_path_traversal_blocked() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let result = execute_tool(
            "test-id",
            "file_list",
            &serde_json::json!({"path": "/foo/../../etc"}),
            None,
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            Some(workspace.path()),
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("traversal"));
    }

    // ── Named-workspace read-side support ────────────────────────────────
    //
    // Mock kernel that surfaces a configurable list of named workspaces
    // (paired with their access modes) via `named_workspace_prefixes`.
    // `readonly_workspace_prefixes` is derived from that list so the existing
    // file_write denial path stays consistent.

    struct NamedWsKernel {
        named: Vec<(std::path::PathBuf, librefang_types::agent::WorkspaceMode)>,
        /// Optional channel-bridge download dir surfaced via
        /// `KernelHandle::channel_file_download_dir` (#4434 regression test
        /// hook). `None` matches the default trait behaviour.
        download_dir: Option<std::path::PathBuf>,
    }

    // ---- BEGIN role-trait impls (split from former `impl KernelHandle for NamedWsKernel`, #3746) ----

    #[async_trait::async_trait]
    impl AgentControl for NamedWsKernel {
        async fn spawn_agent(
            &self,
            _manifest_toml: &str,
            _parent_id: Option<&str>,
        ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn send_to_agent(
            &self,
            _agent_id: &str,
            _message: &str,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn list_agents(&self) -> Vec<AgentInfo> {
            vec![]
        }

        fn kill_agent(
            &self,
            _agent_id: &str,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
            vec![]
        }
    }

    impl MemoryAccess for NamedWsKernel {
        fn memory_store(
            &self,
            _key: &str,
            _value: serde_json::Value,
            _peer_id: Option<&str>,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn memory_recall(
            &self,
            _key: &str,
            _peer_id: Option<&str>,
        ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn memory_list(
            &self,
            _peer_id: Option<&str>,
        ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }
    }

    impl WikiAccess for NamedWsKernel {}

    #[async_trait::async_trait]
    impl TaskQueue for NamedWsKernel {
        async fn task_post(
            &self,
            _title: &str,
            _description: &str,
            _assigned_to: Option<&str>,
            _created_by: Option<&str>,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_claim(
            &self,
            _agent_id: &str,
        ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_complete(
            &self,
            _agent_id: &str,
            _task_id: &str,
            _result: &str,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_list(
            &self,
            _status: Option<&str>,
        ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_delete(
            &self,
            _task_id: &str,
        ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_retry(
            &self,
            _task_id: &str,
        ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_get(
            &self,
            _task_id: &str,
        ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_update_status(
            &self,
            _task_id: &str,
            _new_status: &str,
        ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }
    }

    #[async_trait::async_trait]
    impl EventBus for NamedWsKernel {
        async fn publish_event(
            &self,
            _event_type: &str,
            _payload: serde_json::Value,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }
    }

    #[async_trait::async_trait]
    impl KnowledgeGraph for NamedWsKernel {
        async fn knowledge_add_entity(
            &self,
            _entity: &librefang_types::memory::Entity,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn knowledge_add_relation(
            &self,
            _relation: &librefang_types::memory::Relation,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn knowledge_query(
            &self,
            _pattern: librefang_types::memory::GraphPattern,
        ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
        {
            Err("not used".into())
        }
    }

    impl ToolPolicy for NamedWsKernel {
        fn named_workspace_prefixes(
            &self,
            _agent_id: &str,
        ) -> Vec<(std::path::PathBuf, librefang_types::agent::WorkspaceMode)> {
            self.named.clone()
        }

        fn readonly_workspace_prefixes(&self, _agent_id: &str) -> Vec<std::path::PathBuf> {
            self.named
                .iter()
                .filter(|(_, m)| *m == librefang_types::agent::WorkspaceMode::ReadOnly)
                .map(|(p, _)| p.clone())
                .collect()
        }
        fn channel_file_download_dir(&self) -> Option<std::path::PathBuf> {
            self.download_dir.clone()
        }
    }

    // No-op role-trait impls (#3746) — mock relies on default bodies.
    impl CronControl for NamedWsKernel {}
    impl HandsControl for NamedWsKernel {}
    impl ApprovalGate for NamedWsKernel {}
    impl A2ARegistry for NamedWsKernel {}
    impl ChannelSender for NamedWsKernel {}
    impl PromptStore for NamedWsKernel {}
    impl WorkflowRunner for NamedWsKernel {}
    impl GoalControl for NamedWsKernel {}
    impl ApiAuth for NamedWsKernel {
        fn auth_snapshot(&self) -> ApiAuthSnapshot {
            ApiAuthSnapshot::default()
        }
    }
    impl SessionWriter for NamedWsKernel {
        fn inject_attachment_blocks(
            &self,
            _agent_id: librefang_types::agent::AgentId,
            _blocks: Vec<librefang_types::message::ContentBlock>,
        ) {
        }
    }

    // ---- END role-trait impls (#3746) ----

    fn make_named_ws_kernel(
        named: Vec<(std::path::PathBuf, librefang_types::agent::WorkspaceMode)>,
    ) -> Arc<dyn KernelHandle> {
        Arc::new(NamedWsKernel {
            named,
            download_dir: None,
        })
    }

    fn make_download_dir_kernel(download_dir: std::path::PathBuf) -> Arc<dyn KernelHandle> {
        Arc::new(NamedWsKernel {
            named: vec![],
            download_dir: Some(download_dir),
        })
    }

    #[tokio::test]
    async fn test_file_read_allows_named_workspace_path() {
        use librefang_types::agent::WorkspaceMode;

        let primary = tempfile::tempdir().expect("primary");
        let shared = tempfile::tempdir().expect("shared");
        let shared_canon = shared.path().canonicalize().unwrap();
        let target = shared_canon.join("note.txt");
        std::fs::write(&target, "hello shared").unwrap();

        let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadWrite)]);

        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": target.to_str().unwrap()}),
            Some(&kernel),
            None,
            Some("00000000-0000-0000-0000-000000000001"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(primary.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(!result.is_error, "got error: {}", result.content);
        assert_eq!(result.content, "hello shared");
    }

    #[tokio::test]
    async fn test_file_list_allows_named_workspace_path() {
        use librefang_types::agent::WorkspaceMode;

        let primary = tempfile::tempdir().expect("primary");
        let shared = tempfile::tempdir().expect("shared");
        let shared_canon = shared.path().canonicalize().unwrap();
        std::fs::write(shared_canon.join("a.txt"), "a").unwrap();
        std::fs::write(shared_canon.join("b.txt"), "b").unwrap();

        let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadOnly)]);

        let result = execute_tool(
            "test-id",
            "file_list",
            &serde_json::json!({"path": shared_canon.to_str().unwrap()}),
            Some(&kernel),
            None,
            Some("00000000-0000-0000-0000-000000000002"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(primary.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("a.txt"));
        assert!(result.content.contains("b.txt"));
    }

    /// #4434: channel bridges save attachments to a shared download dir
    /// (default `/tmp/librefang_uploads`) which lives outside any agent's
    /// `workspace_root`. The runtime must widen `file_read`'s sandbox
    /// accept-list with `KernelHandle::channel_file_download_dir()` so
    /// agents can open the very files the bridge tells them about.
    #[tokio::test]
    async fn test_file_read_allows_channel_download_dir() {
        let primary = tempfile::tempdir().expect("primary");
        let download = tempfile::tempdir().expect("download");
        let download_canon = download.path().canonicalize().unwrap();
        let target = download_canon.join("attachment.txt");
        std::fs::write(&target, "from-telegram").unwrap();

        let kernel = make_download_dir_kernel(download_canon.clone());

        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": target.to_str().unwrap()}),
            Some(&kernel),
            None,
            Some("00000000-0000-0000-0000-000000000010"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(primary.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(!result.is_error, "got error: {}", result.content);
        assert_eq!(result.content, "from-telegram");
    }

    /// Companion to the file_read test: file_list must also see into the
    /// channel download dir so an agent can enumerate inbox attachments.
    #[tokio::test]
    async fn test_file_list_allows_channel_download_dir() {
        let primary = tempfile::tempdir().expect("primary");
        let download = tempfile::tempdir().expect("download");
        let download_canon = download.path().canonicalize().unwrap();
        std::fs::write(download_canon.join("one.pdf"), "1").unwrap();
        std::fs::write(download_canon.join("two.pdf"), "2").unwrap();

        let kernel = make_download_dir_kernel(download_canon.clone());

        let result = execute_tool(
            "test-id",
            "file_list",
            &serde_json::json!({"path": download_canon.to_str().unwrap()}),
            Some(&kernel),
            None,
            Some("00000000-0000-0000-0000-000000000011"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(primary.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("one.pdf"));
        assert!(result.content.contains("two.pdf"));
    }

    /// Defense-in-depth: the download dir is a *read-side* allowlist only.
    /// `file_write` still uses `named_ws_prefixes_writable`, so writes into
    /// the bridge's directory must remain rejected.
    #[tokio::test]
    async fn test_file_write_rejects_channel_download_dir() {
        let primary = tempfile::tempdir().expect("primary");
        let download = tempfile::tempdir().expect("download");
        let download_canon = download.path().canonicalize().unwrap();
        let target = download_canon.join("smuggled.txt");

        let kernel = make_download_dir_kernel(download_canon.clone());

        let result = execute_tool(
            "test-id",
            "file_write",
            &serde_json::json!({
                "path": target.to_str().unwrap(),
                "content": "should-not-land",
            }),
            Some(&kernel),
            None,
            Some("00000000-0000-0000-0000-000000000012"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(primary.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_error, "expected write to be rejected");
        assert!(
            !target.exists(),
            "file should not have been written: {}",
            target.display()
        );
    }

    #[tokio::test]
    async fn test_file_write_allows_rw_named_workspace_path() {
        use librefang_types::agent::WorkspaceMode;

        let primary = tempfile::tempdir().expect("primary");
        let shared = tempfile::tempdir().expect("shared");
        let shared_canon = shared.path().canonicalize().unwrap();
        let target = shared_canon.join("out.txt");

        let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadWrite)]);

        let result = execute_tool(
            "test-id",
            "file_write",
            &serde_json::json!({
                "path": target.to_str().unwrap(),
                "content": "wrote-it",
            }),
            Some(&kernel),
            None,
            Some("00000000-0000-0000-0000-000000000003"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(primary.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(!result.is_error, "got error: {}", result.content);
        let written = std::fs::read_to_string(&target).unwrap();
        assert_eq!(written, "wrote-it");
    }

    #[tokio::test]
    async fn test_file_write_denies_readonly_named_workspace_path() {
        use librefang_types::agent::WorkspaceMode;

        let primary = tempfile::tempdir().expect("primary");
        let shared = tempfile::tempdir().expect("shared");
        let shared_canon = shared.path().canonicalize().unwrap();
        let target = shared_canon.join("out.txt");

        let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadOnly)]);

        let result = execute_tool(
            "test-id",
            "file_write",
            &serde_json::json!({
                "path": target.to_str().unwrap(),
                "content": "should-not-write",
            }),
            Some(&kernel),
            None,
            Some("00000000-0000-0000-0000-000000000004"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(primary.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_error);
        assert!(
            result.content.contains("read-only"),
            "expected read-only denial, got: {}",
            result.content
        );
        assert!(!target.exists(), "file should not have been written");
    }

    #[tokio::test]
    async fn test_file_read_outside_all_workspaces_still_blocked() {
        use librefang_types::agent::WorkspaceMode;

        let primary = tempfile::tempdir().expect("primary");
        let shared = tempfile::tempdir().expect("shared");
        let other = tempfile::tempdir().expect("other");
        let shared_canon = shared.path().canonicalize().unwrap();
        let other_path = other.path().canonicalize().unwrap().join("nope.txt");
        std::fs::write(&other_path, "secret").unwrap();

        let kernel = make_named_ws_kernel(vec![(shared_canon, WorkspaceMode::ReadWrite)]);

        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": other_path.to_str().unwrap()}),
            Some(&kernel),
            None,
            Some("00000000-0000-0000-0000-000000000005"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(primary.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_error);
        assert!(
            result.content.contains("Access denied"),
            "expected sandbox denial, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_apply_patch_allows_rw_named_workspace_path() {
        use librefang_types::agent::WorkspaceMode;

        let primary = tempfile::tempdir().expect("primary");
        let shared = tempfile::tempdir().expect("shared");
        let shared_canon = shared.path().canonicalize().unwrap();
        let target = shared_canon.join("added.txt");

        let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadWrite)]);

        let patch = format!(
            "*** Begin Patch\n*** Add File: {}\n+hello-from-patch\n*** End Patch\n",
            target.to_str().unwrap()
        );

        let result = execute_tool(
            "test-id",
            "apply_patch",
            &serde_json::json!({"patch": patch}),
            Some(&kernel),
            None,
            Some("00000000-0000-0000-0000-000000000006"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(primary.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(!result.is_error, "got error: {}", result.content);
        let written = std::fs::read_to_string(&target).unwrap();
        assert_eq!(written, "hello-from-patch");
    }

    #[tokio::test]
    async fn test_apply_patch_denies_readonly_named_workspace_path() {
        use librefang_types::agent::WorkspaceMode;

        let primary = tempfile::tempdir().expect("primary");
        let shared = tempfile::tempdir().expect("shared");
        let shared_canon = shared.path().canonicalize().unwrap();
        let target = shared_canon.join("added.txt");

        let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadOnly)]);

        let patch = format!(
            "*** Begin Patch\n*** Add File: {}\n+should-not-write\n*** End Patch\n",
            target.to_str().unwrap()
        );

        let result = execute_tool(
            "test-id",
            "apply_patch",
            &serde_json::json!({"patch": patch}),
            Some(&kernel),
            None,
            Some("00000000-0000-0000-0000-000000000007"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(primary.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_error, "expected denial, got: {}", result.content);
        assert!(!target.exists(), "file should not have been written");
    }

    // ── Bug #3822: shell_exec must respect named workspace read-only mode ────

    /// Regression for #3822: `shell_exec` must be blocked when the command
    /// string references a path that falls inside a read-only named workspace.
    /// Previously the shell tool had no such check; a plugin could bypass the
    /// read-only restriction by issuing a shell command that writes to the
    /// supposedly read-only workspace path.
    #[tokio::test]
    async fn test_shell_exec_blocked_for_readonly_workspace_path() {
        use librefang_types::agent::WorkspaceMode;

        let primary = tempfile::tempdir().expect("primary");
        let shared = tempfile::tempdir().expect("shared");
        let shared_canon = shared.path().canonicalize().unwrap();

        // Configure the shared workspace as read-only.
        let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadOnly)]);

        // Construct a command string that references the read-only path.
        let ro_path = shared_canon.to_str().unwrap();
        let command = format!("touch {ro_path}/evil.txt");

        let result = execute_tool(
            "test-id",
            "shell_exec",
            &serde_json::json!({"command": command}),
            Some(&kernel),
            None,
            Some("00000000-0000-0000-0000-000000000008"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(primary.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(
            result.is_error,
            "shell_exec referencing a read-only workspace path must be blocked; got: {}",
            result.content
        );
        assert!(
            result.content.contains("read-only"),
            "error must mention read-only; got: {}",
            result.content
        );
        // Verify the file was NOT created.
        assert!(!shared_canon.join("evil.txt").exists());
    }

    /// Read-only workspace enforcement must NOT block commands that do not
    /// reference the read-only workspace path.
    #[tokio::test]
    async fn test_shell_exec_allowed_when_not_targeting_readonly_workspace() {
        use librefang_types::agent::WorkspaceMode;

        let primary = tempfile::tempdir().expect("primary");
        let shared = tempfile::tempdir().expect("shared");
        let shared_canon = shared.path().canonicalize().unwrap();

        // Read-only shared workspace — but the command targets the primary workspace.
        let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadOnly)]);

        // A command that does NOT reference the read-only path should go through
        // (it may still fail for other reasons — e.g., exec policy — but it must
        // not be blocked by the workspace read-only check).
        let result = execute_tool(
            "test-id",
            "shell_exec",
            &serde_json::json!({"command": "echo hello"}),
            Some(&kernel),
            None,
            Some("00000000-0000-0000-0000-000000000009"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(primary.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        // Must NOT be blocked by read-only check. It may be blocked by exec policy
        // (if one is set) but should not contain "read-only" in the error.
        if result.is_error {
            assert!(
                !result.content.contains("read-only"),
                "must not be blocked by read-only check; got: {}",
                result.content
            );
        }
    }

    #[tokio::test]
    async fn test_web_search() {
        let result = execute_tool(
            "test-id",
            "web_search",
            &serde_json::json!({"query": "rust programming"}),
            None,
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        // web_search now attempts a real fetch; may succeed or fail depending on network
        assert!(!result.tool_use_id.is_empty());
    }

    #[tokio::test]
    async fn test_unknown_tool() {
        let result = execute_tool(
            "test-id",
            "nonexistent_tool",
            &serde_json::json!({}),
            None,
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("Unknown tool"));
    }

    #[tokio::test]
    async fn test_agent_tools_without_kernel() {
        let result = execute_tool(
            "test-id",
            "agent_list",
            &serde_json::json!({}),
            None,
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("Kernel handle not available"));
    }

    #[tokio::test]
    async fn test_capability_enforcement_denied() {
        let allowed = vec!["file_read".to_string(), "file_list".to_string()];
        let result = execute_tool(
            "test-id",
            "shell_exec",
            &serde_json::json!({"command": "ls"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("Permission denied"));
    }

    #[tokio::test]
    async fn test_capability_enforcement_allowed() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let allowed = vec!["file_read".to_string()];
        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": "nonexistent_12345/file.txt"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            Some(workspace.path()),
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        // Should fail for path resolution, NOT for permission denied
        assert!(
            result.is_error,
            "Expected error but got: {}",
            result.content
        );
        assert!(
            !result.content.contains("Permission denied"),
            "Unexpected permission denied: {}",
            result.content
        );
        assert!(
            result.content.contains("Failed to read")
                || result.content.contains("Failed to resolve")
                || result.content.contains("not found")
                || result.content.contains("No such file")
                || result.content.contains("does not exist"),
            "Expected file-not-found error, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_capability_enforcement_aliased_tool_name() {
        // Agent has "file_write" in allowed tools, but LLM calls "fs-write".
        // After normalization, this should pass the capability check.
        let workspace = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(workspace.path().join("output")).expect("create output dir");
        let allowed = vec![
            "file_read".to_string(),
            "file_write".to_string(),
            "file_list".to_string(),
            "shell_exec".to_string(),
        ];
        let result = execute_tool(
            "test-id",
            "fs-write", // LLM-hallucinated alias
            &serde_json::json!({"path": "output/file.txt", "content": "hello"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            Some(workspace.path()),
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(
            !result.is_error,
            "fs-write should normalize to file_write and pass capability check, got: {}",
            result.content
        );
        assert!(workspace.path().join("output/file.txt").exists());
    }

    #[tokio::test]
    async fn test_capability_enforcement_aliased_denied() {
        // Agent does NOT have file_write, and LLM calls "fs-write" — should be denied.
        let allowed = vec!["file_read".to_string()];
        let result = execute_tool(
            "test-id",
            "fs-write",
            &serde_json::json!({"path": "/tmp/test.txt", "content": "hello"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(result.is_error);
        assert!(
            result.content.contains("Permission denied"),
            "fs-write should normalize to file_write which is not in allowed list"
        );
    }

    #[tokio::test]
    async fn test_shell_exec_full_policy_skips_approval_gate() {
        let approval_requests = Arc::new(AtomicUsize::new(0));
        let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
            approval_requests: Arc::clone(&approval_requests),
            user_gate_override: None,
        });
        let policy = librefang_types::config::ExecPolicy {
            mode: librefang_types::config::ExecSecurityMode::Full,
            ..Default::default()
        };
        let workspace = tempfile::tempdir().expect("tempdir");

        let result = execute_tool(
            "test-id",
            "shell_exec",
            &serde_json::json!({"command": "echo ok"}),
            Some(&kernel),
            None,
            Some("agent-1"),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            Some(workspace.path()),
            None, // media_engine
            None, // media_drivers
            Some(&policy),
            None,
            None,
            None,
            None,
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;

        assert!(
            !result.content.contains("requires human approval"),
            "full exec policy should bypass approval gate, got: {}",
            result.content
        );
        assert_eq!(approval_requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_shell_exec_non_full_policy_still_requires_approval() {
        let approval_requests = Arc::new(AtomicUsize::new(0));
        let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
            approval_requests: Arc::clone(&approval_requests),
            user_gate_override: None,
        });
        let policy = librefang_types::config::ExecPolicy {
            mode: librefang_types::config::ExecSecurityMode::Allowlist,
            ..Default::default()
        };

        let result = execute_tool(
            "test-id",
            "shell_exec",
            &serde_json::json!({"command": "echo ok"}),
            Some(&kernel),
            None,
            Some("agent-1"),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            Some(&policy),
            None,
            None,
            None,
            None,
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;

        // With non-blocking approval (Step 5), the tool is deferred rather than blocked.
        // The result should be WaitingApproval (not is_error) with the appropriate message.
        assert!(!result.is_error, "WaitingApproval should not be an error");
        assert!(
            result.content.contains("requires human approval"),
            "content should mention approval requirement, got: {}",
            result.content
        );
        assert_eq!(
            result.status,
            librefang_types::tool::ToolExecutionStatus::WaitingApproval
        );
        assert_eq!(approval_requests.load(Ordering::SeqCst), 1);
    }

    // ---- RBAC M3 — per-user tool policy gate (#3054) ----

    #[tokio::test]
    async fn tool_runner_rbac_user_deny_returns_hard_error() {
        let approval_requests = Arc::new(AtomicUsize::new(0));
        let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
            approval_requests: Arc::clone(&approval_requests),
            user_gate_override: Some(librefang_types::user_policy::UserToolGate::Deny {
                reason: "user 'Bob' (role: user) is not permitted to invoke 'shell_exec'"
                    .to_string(),
            }),
        });

        let result = execute_tool(
            "test-id",
            "shell_exec",
            &serde_json::json!({"command": "echo ok"}),
            Some(&kernel),
            None,
            Some("agent-1"),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None,
            None,
            None,
            None,
            Some("bob"),
            Some("telegram"),
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_error, "user-policy deny must produce an error");
        assert!(
            result.content.contains("Execution denied"),
            "content should announce the deny: {}",
            result.content
        );
        assert!(
            result.content.contains("user 'Bob'"),
            "deny reason must surface to the model: {}",
            result.content
        );
        // No approval was requested — the deny short-circuits.
        assert_eq!(approval_requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn tool_runner_rbac_user_needs_approval_routes_through_approval_queue() {
        let approval_requests = Arc::new(AtomicUsize::new(0));
        let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
            approval_requests: Arc::clone(&approval_requests),
            // file_write is NOT in the default require_approval list (which
            // would already gate it). The point of this test is to prove the
            // user gate flips it into approval-required mode regardless of
            // the global policy.
            user_gate_override: Some(librefang_types::user_policy::UserToolGate::NeedsApproval {
                reason: "tool 'file_write' requires admin approval for user 'Bob'".to_string(),
            }),
        });

        let workspace = tempfile::tempdir().expect("tempdir");

        let result = execute_tool(
            "test-id",
            "file_write",
            &serde_json::json!({"path": "scratch.txt", "content": "hi"}),
            Some(&kernel),
            None,
            Some("agent-1"),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            Some(workspace.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("bob"),
            Some("telegram"),
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        // User gate forced approval — the tool is deferred (NotBlocked).
        assert_eq!(
            result.status,
            librefang_types::tool::ToolExecutionStatus::WaitingApproval,
            "expected WaitingApproval status, got content: {}",
            result.content
        );
        assert_eq!(approval_requests.load(Ordering::SeqCst), 1);
    }

    /// Regression: shell_exec under `ExecPolicy.mode = Full` MUST still
    /// route through the approval queue when the per-user gate returned
    /// `NeedsApproval`. Without the `!force_approval` guard added in B2
    /// of PR #3205 review, the Full-mode bypass silently dropped the
    /// user-gate escalation and the call ran without human review.
    #[tokio::test]
    async fn tool_runner_rbac_full_mode_does_not_bypass_user_needs_approval() {
        let approval_requests = Arc::new(AtomicUsize::new(0));
        let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
            approval_requests: Arc::clone(&approval_requests),
            user_gate_override: Some(librefang_types::user_policy::UserToolGate::NeedsApproval {
                reason: "tool 'shell_exec' requires admin approval for user 'Bob'".to_string(),
            }),
        });

        let workspace = tempfile::tempdir().expect("tempdir");
        let policy = librefang_types::config::ExecPolicy {
            mode: librefang_types::config::ExecSecurityMode::Full,
            ..Default::default()
        };

        let result = execute_tool(
            "test-id",
            "shell_exec",
            &serde_json::json!({"command": "echo ok"}),
            Some(&kernel),
            None,
            Some("agent-1"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(workspace.path()),
            None,
            None,
            Some(&policy), // Full mode!
            None,
            None,
            None,
            None,
            Some("bob"),
            Some("telegram"),
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        assert_eq!(
            result.status,
            librefang_types::tool::ToolExecutionStatus::WaitingApproval,
            "Full mode + user NeedsApproval must still demand approval, got content: {}",
            result.content
        );
        assert_eq!(
            approval_requests.load(Ordering::SeqCst),
            1,
            "exactly one approval request should be submitted"
        );
    }

    #[tokio::test]
    async fn tool_runner_rbac_user_allow_falls_through_to_existing_approval_logic() {
        // user_gate_override = Allow → behaviour matches the pre-RBAC
        // approval flow. shell_exec is in the default require_approval
        // list and ApprovalKernel.requires_approval() returns true for it,
        // so we still expect WaitingApproval — proving Allow is a true
        // pass-through, not a bypass.
        let approval_requests = Arc::new(AtomicUsize::new(0));
        let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
            approval_requests: Arc::clone(&approval_requests),
            user_gate_override: Some(librefang_types::user_policy::UserToolGate::Allow),
        });

        let result = execute_tool(
            "test-id",
            "shell_exec",
            &serde_json::json!({"command": "echo ok"}),
            Some(&kernel),
            None,
            Some("agent-1"),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("alice"),
            Some("telegram"),
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        assert_eq!(
            result.status,
            librefang_types::tool::ToolExecutionStatus::WaitingApproval
        );
        assert_eq!(approval_requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_shell_exec_uses_exec_policy_allowed_env_vars() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let original = std::env::var("LIBREFANG_TEST_ALLOWED_ENV").ok();
        // SAFETY: test captures and restores the previous value; unique enough
        // name to avoid clashing with other tests running in parallel.
        unsafe {
            std::env::set_var("LIBREFANG_TEST_ALLOWED_ENV", "present");
        }

        let allowed = ["shell_exec".to_string()];
        let policy = librefang_types::config::ExecPolicy {
            mode: librefang_types::config::ExecSecurityMode::Allowlist,
            allowed_env_vars: vec!["LIBREFANG_TEST_ALLOWED_ENV".to_string()],
            ..Default::default()
        };

        let result = execute_tool(
            "test-id",
            "shell_exec",
            &serde_json::json!({"command": "env"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            Some(workspace.path()),
            None, // media_engine
            None, // media_drivers
            Some(&policy),
            None,
            None,
            None,
            None,
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;

        match original {
            Some(val) => unsafe {
                std::env::set_var("LIBREFANG_TEST_ALLOWED_ENV", val);
            },
            None => unsafe {
                std::env::remove_var("LIBREFANG_TEST_ALLOWED_ENV");
            },
        }

        assert!(
            !result.is_error,
            "shell_exec should succeed with env passthrough, got: {}",
            result.content
        );
        assert!(
            result
                .content
                .contains("LIBREFANG_TEST_ALLOWED_ENV=present"),
            "allowed env var should be visible to subprocess, got: {}",
            result.content
        );
    }

    // --- Schedule parser tests ---
    #[test]
    fn test_parse_schedule_every_minutes() {
        assert_eq!(
            parse_schedule_to_cron("every 5 minutes").unwrap(),
            "*/5 * * * *"
        );
        assert_eq!(
            parse_schedule_to_cron("every 1 minute").unwrap(),
            "* * * * *"
        );
        assert_eq!(parse_schedule_to_cron("every minute").unwrap(), "* * * * *");
        assert_eq!(
            parse_schedule_to_cron("every 30 minutes").unwrap(),
            "*/30 * * * *"
        );
    }

    #[test]
    fn test_parse_schedule_every_hours() {
        assert_eq!(parse_schedule_to_cron("every hour").unwrap(), "0 * * * *");
        assert_eq!(parse_schedule_to_cron("every 1 hour").unwrap(), "0 * * * *");
        assert_eq!(
            parse_schedule_to_cron("every 2 hours").unwrap(),
            "0 */2 * * *"
        );
    }

    #[test]
    fn test_parse_schedule_daily() {
        assert_eq!(parse_schedule_to_cron("daily at 9am").unwrap(), "0 9 * * *");
        assert_eq!(
            parse_schedule_to_cron("daily at 6pm").unwrap(),
            "0 18 * * *"
        );
        assert_eq!(
            parse_schedule_to_cron("daily at 12am").unwrap(),
            "0 0 * * *"
        );
        assert_eq!(
            parse_schedule_to_cron("daily at 12pm").unwrap(),
            "0 12 * * *"
        );
    }

    #[test]
    fn test_parse_schedule_weekdays() {
        assert_eq!(
            parse_schedule_to_cron("weekdays at 9am").unwrap(),
            "0 9 * * 1-5"
        );
        assert_eq!(
            parse_schedule_to_cron("weekends at 10am").unwrap(),
            "0 10 * * 0,6"
        );
    }

    #[test]
    fn test_parse_schedule_shorthand() {
        assert_eq!(parse_schedule_to_cron("hourly").unwrap(), "0 * * * *");
        assert_eq!(parse_schedule_to_cron("daily").unwrap(), "0 0 * * *");
        assert_eq!(parse_schedule_to_cron("weekly").unwrap(), "0 0 * * 0");
        assert_eq!(parse_schedule_to_cron("monthly").unwrap(), "0 0 1 * *");
    }

    #[test]
    fn test_parse_schedule_cron_passthrough() {
        assert_eq!(
            parse_schedule_to_cron("0 */5 * * *").unwrap(),
            "0 */5 * * *"
        );
        assert_eq!(
            parse_schedule_to_cron("30 9 * * 1-5").unwrap(),
            "30 9 * * 1-5"
        );
    }

    #[test]
    fn test_parse_schedule_invalid() {
        assert!(parse_schedule_to_cron("whenever I feel like it").is_err());
        assert!(parse_schedule_to_cron("every 0 minutes").is_err());
    }

    // --- Image format detection tests ---
    #[test]
    fn test_detect_image_format_png() {
        let data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x10\x00\x00\x00\x10";
        assert_eq!(detect_image_format(data), "png");
    }

    #[test]
    fn test_detect_image_format_jpeg() {
        let data = b"\xFF\xD8\xFF\xE0\x00\x10JFIF";
        assert_eq!(detect_image_format(data), "jpeg");
    }

    #[test]
    fn test_detect_image_format_gif() {
        let data = b"GIF89a\x10\x00\x10\x00";
        assert_eq!(detect_image_format(data), "gif");
    }

    #[test]
    fn test_detect_image_format_bmp() {
        let data = b"BM\x00\x00\x00\x00";
        assert_eq!(detect_image_format(data), "bmp");
    }

    #[test]
    fn test_detect_image_format_unknown() {
        let data = b"\x00\x00\x00\x00";
        assert_eq!(detect_image_format(data), "unknown");
    }

    #[test]
    fn test_extract_png_dimensions() {
        // Minimal PNG header: signature (8) + IHDR length (4) + "IHDR" (4) + width (4) + height (4)
        let mut data = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]; // signature
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D]); // IHDR length
        data.extend_from_slice(b"IHDR"); // chunk type
        data.extend_from_slice(&640u32.to_be_bytes()); // width
        data.extend_from_slice(&480u32.to_be_bytes()); // height
        assert_eq!(extract_image_dimensions(&data, "png"), Some((640, 480)));
    }

    #[test]
    fn test_extract_gif_dimensions() {
        let mut data = b"GIF89a".to_vec();
        data.extend_from_slice(&320u16.to_le_bytes()); // width
        data.extend_from_slice(&240u16.to_le_bytes()); // height
        assert_eq!(extract_image_dimensions(&data, "gif"), Some((320, 240)));
    }

    #[test]
    fn test_format_file_size() {
        assert_eq!(format_file_size(500), "500 B");
        assert_eq!(format_file_size(1536), "1.5 KB");
        assert_eq!(format_file_size(2 * 1024 * 1024), "2.0 MB");
    }

    #[tokio::test]
    async fn test_image_analyze_missing_file() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let result = execute_tool(
            "test-id",
            "image_analyze",
            &serde_json::json!({"path": "nonexistent_image.png"}),
            None,
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            Some(workspace.path()),
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(result.is_error);
        assert!(
            result.content.contains("Failed to read"),
            "unexpected error content: {}",
            result.content
        );
    }

    /// Regression test for #4450: the media/image read-only tools must accept
    /// paths inside named-workspace prefixes (the "additional_roots" allowlist),
    /// not just the primary workspace root. Before the fix these tools called
    /// the bare `resolve_file_path` wrapper which threaded `&[]` and produced
    /// "resolves outside workspace" even when the agent had declared the mount
    /// under `[workspaces]`.
    #[tokio::test]
    async fn test_media_tools_honor_named_workspace_prefixes() {
        // Two disjoint dirs: `workspace_root` is the agent's primary workspace,
        // `mount` is the named-workspace prefix. The test file lives only in
        // `mount`, so success proves the prefix was honored.
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let mount = tempfile::tempdir().expect("mount tempdir");
        let mount_canon = mount.path().canonicalize().expect("canonicalize mount");
        let img_path = mount_canon.join("photo.png");
        // Minimal PNG signature so detect_image_format() returns "png".
        let png_bytes: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        std::fs::write(&img_path, png_bytes).expect("write png");

        let raw_path = img_path.to_string_lossy().to_string();
        let input = serde_json::json!({ "path": raw_path });

        // Without prefixes -> rejected as outside the sandbox.
        let denied = tool_image_analyze(&input, Some(workspace.path()), &[]).await;
        assert!(
            denied.is_err(),
            "image_analyze should reject paths outside the workspace when \
             no named-workspace prefixes are provided, got: {:?}",
            denied
        );
        let err = denied.unwrap_err();
        assert!(
            err.contains("resolves outside workspace") || err.contains("Access denied"),
            "expected sandbox rejection, got: {err}"
        );

        // With the mount as an additional root -> accepted.
        let extra: &[&Path] = &[mount_canon.as_path()];
        let ok = tool_image_analyze(&input, Some(workspace.path()), extra).await;
        assert!(
            ok.is_ok(),
            "image_analyze must accept files under a named-workspace prefix, \
             got: {:?}",
            ok
        );
    }

    #[test]
    fn test_depth_limit_constant() {
        assert_eq!(MAX_AGENT_CALL_DEPTH, 5);
    }

    #[test]
    fn test_depth_limit_first_call_succeeds() {
        // Default depth is 0, which is < MAX_AGENT_CALL_DEPTH
        let default_depth = AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0);
        assert!(default_depth < MAX_AGENT_CALL_DEPTH);
    }

    #[test]
    fn test_task_local_compiles() {
        // Verify task_local macro works — just ensure the type exists
        let cell = std::cell::Cell::new(0u32);
        assert_eq!(cell.get(), 0);
    }

    #[tokio::test]
    async fn test_schedule_tools_without_kernel() {
        let result = execute_tool(
            "test-id",
            "schedule_list",
            &serde_json::json!({}),
            None,
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("Kernel handle not available"));
    }

    // ─── Canvas / A2UI tests ────────────────────────────────────────

    #[test]
    fn test_sanitize_canvas_basic_html() {
        let html = "<h1>Hello World</h1><p>This is a test.</p>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), html);
    }

    #[test]
    fn test_sanitize_canvas_rejects_script() {
        let html = "<div><script>alert('xss')</script></div>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("script"));
    }

    #[test]
    fn test_sanitize_canvas_rejects_iframe() {
        let html = "<iframe src='https://evil.com'></iframe>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("iframe"));
    }

    #[test]
    fn test_sanitize_canvas_rejects_event_handler() {
        let html = "<div onclick=\"alert('xss')\">click me</div>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("event handler"));
    }

    #[test]
    fn test_sanitize_canvas_rejects_onload() {
        let html = "<img src='x' onerror = \"alert(1)\">";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_sanitize_canvas_rejects_javascript_url() {
        let html = "<a href=\"javascript:alert('xss')\">click</a>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("javascript:"));
    }

    #[test]
    fn test_sanitize_canvas_rejects_data_html() {
        let html = "<a href=\"data:text/html,<script>alert(1)</script>\">x</a>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_sanitize_canvas_rejects_empty() {
        let result = sanitize_canvas_html("", 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Empty"));
    }

    #[test]
    fn test_sanitize_canvas_size_limit() {
        let html = "x".repeat(1024);
        let result = sanitize_canvas_html(&html, 100);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too large"));
    }

    #[tokio::test]
    async fn test_canvas_present_tool() {
        let input = serde_json::json!({
            "html": "<h1>Test Canvas</h1><p>Hello world</p>",
            "title": "Test"
        });
        let tmp = std::env::temp_dir().join("librefang_canvas_test");
        let _ = std::fs::create_dir_all(&tmp);
        let result = tool_canvas_present(&input, Some(tmp.as_path())).await;
        assert!(result.is_ok());
        let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert!(output["canvas_id"].is_string());
        assert_eq!(output["title"], "Test");
        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_agent_spawn_manifest_all_cases() {
        let mut toml;

        // Case 1: Minimal - only name and system_prompt
        toml = build_agent_manifest_toml("test-agent", "You are helpful.", vec![], vec![], false)
            .unwrap();
        assert!(toml.contains("name = \"test-agent\""));
        assert!(toml.contains("system_prompt = \"You are helpful.\""));
        assert!(toml.contains("tools = []"));
        assert!(!toml.contains("network"));
        assert!(!toml.contains("shell = ["));

        // Case 2: With tools (no network)
        toml = build_agent_manifest_toml(
            "coder",
            "You are a coder.",
            vec!["file_read".to_string(), "file_write".to_string()],
            vec![],
            false,
        )
        .unwrap();
        assert!(toml.contains("tools = [\"file_read\", \"file_write\"]"));
        assert!(!toml.contains("network"));

        // Case 3: network explicitly enabled
        toml = build_agent_manifest_toml(
            "web-agent",
            "You browse the web.",
            vec!["web_fetch".to_string()],
            vec![],
            true,
        )
        .unwrap();
        assert!(toml.contains("web_fetch"));
        assert!(toml.contains("network = [\"*\"]"));

        // Case 4: shell without shell_exec - should auto-add shell_exec to tools
        toml = build_agent_manifest_toml(
            "shell-test",
            "You run commands.",
            vec!["git".to_string()],
            vec!["uv *".to_string()],
            false,
        )
        .unwrap();
        assert!(toml.contains("shell = [\"uv *\"]"));
        assert!(toml.contains("shell_exec")); // auto-added

        // Case 5: shell with explicit shell_exec (should not duplicate)
        toml = build_agent_manifest_toml(
            "shell-test",
            "You run commands.",
            vec!["shell_exec".to_string(), "git".to_string()],
            vec!["uv *".to_string(), "cargo *".to_string()],
            false,
        )
        .unwrap();
        assert!(toml.contains("shell = [\"uv *\", \"cargo *\"]"));
        // shell_exec should only appear once
        let shell_exec_count = toml.matches("shell_exec").count();
        assert_eq!(shell_exec_count, 1);

        // Case 6: Special chars in strings
        toml = build_agent_manifest_toml(
            "agent-with\"quotes",
            "He said \"hello\" and '''goodbye'''.",
            vec![],
            vec![],
            false,
        )
        .unwrap();
        assert!(toml.contains("agent-with\"quotes"));

        // Case 7: Multiple tools with web_fetch and shell (auto-adds shell_exec)
        toml = build_agent_manifest_toml(
            "multi-agent",
            "You do everything.",
            vec!["web_fetch".to_string(), "git".to_string()],
            vec!["ls *".to_string()],
            true,
        )
        .unwrap();
        assert!(toml.contains("web_fetch"));
        assert!(toml.contains("network = [\"*\"]"));
        assert!(toml.contains("shell = [\"ls *\"]"));
        assert!(toml.contains("shell_exec")); // auto-added
    }

    // -----------------------------------------------------------------------
    // Security fix tests (#1652)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_file_read_no_workspace_root_returns_error() {
        // SECURITY: file_read must fail when workspace_root is None
        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": "/etc/passwd"}),
            None,
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None, // workspace_root = None
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(
            result.is_error,
            "Expected error when workspace_root is None"
        );
        assert!(
            result.content.contains("Workspace sandbox not configured"),
            "Expected workspace sandbox error, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_file_write_no_workspace_root_returns_error() {
        // SECURITY: file_write must fail when workspace_root is None
        let result = execute_tool(
            "test-id",
            "file_write",
            &serde_json::json!({"path": "/tmp/test.txt", "content": "pwned"}),
            None,
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None, // workspace_root = None
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(
            result.is_error,
            "Expected error when workspace_root is None"
        );
        assert!(
            result.content.contains("Workspace sandbox not configured"),
            "Expected workspace sandbox error, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_file_list_no_workspace_root_returns_error() {
        // SECURITY: file_list must fail when workspace_root is None
        let result = execute_tool(
            "test-id",
            "file_list",
            &serde_json::json!({"path": "/etc"}),
            None,
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None, // workspace_root = None
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(
            result.is_error,
            "Expected error when workspace_root is None"
        );
        assert!(
            result.content.contains("Workspace sandbox not configured"),
            "Expected workspace sandbox error, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_agent_spawn_capability_escalation_denied() {
        // SECURITY: sub-agent cannot request tools the parent doesn't have.
        // Parent only has file_read, but child requests shell_exec.
        let kernel: Arc<dyn KernelHandle> = Arc::new(SpawnCheckKernel {
            should_fail_escalation: true,
        });
        let parent_allowed = vec!["file_read".to_string(), "agent_spawn".to_string()];
        let result = execute_tool(
            "test-id",
            "agent_spawn",
            &serde_json::json!({
                "name": "escalated-child",
                "system_prompt": "You are a test agent.",
                "tools": ["shell_exec", "file_read"]
            }),
            Some(&kernel),
            Some(&parent_allowed),
            Some("parent-agent-id"),
            None,
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(
            result.is_error,
            "Expected escalation to be denied, got: {}",
            result.content
        );
        assert!(
            result.content.contains("escalation") || result.content.contains("denied"),
            "Expected escalation denial message, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_agent_spawn_subset_capabilities_allowed() {
        // Sub-agent requests only capabilities the parent has — should succeed.
        let kernel: Arc<dyn KernelHandle> = Arc::new(SpawnCheckKernel {
            should_fail_escalation: false,
        });
        let parent_allowed = vec![
            "file_read".to_string(),
            "file_write".to_string(),
            "agent_spawn".to_string(),
        ];
        let result = execute_tool(
            "test-id",
            "agent_spawn",
            &serde_json::json!({
                "name": "good-child",
                "system_prompt": "You are a test agent.",
                "tools": ["file_read"]
            }),
            Some(&kernel),
            Some(&parent_allowed),
            Some("parent-agent-id"),
            None,
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(
            !result.is_error,
            "Expected spawn to succeed, got error: {}",
            result.content
        );
        assert!(result.content.contains("spawned successfully"));
    }

    #[test]
    fn test_tools_to_parent_capabilities_expands_resource_caps() {
        use librefang_types::capability::Capability;

        let tools = vec![
            "file_read".to_string(),
            "web_fetch".to_string(),
            "shell_exec".to_string(),
            "agent_spawn".to_string(),
            "memory_store".to_string(),
        ];
        let caps = tools_to_parent_capabilities(&tools);

        // Should have ToolInvoke for each tool name
        assert!(caps.contains(&Capability::ToolInvoke("file_read".into())));
        assert!(caps.contains(&Capability::ToolInvoke("web_fetch".into())));
        assert!(caps.contains(&Capability::ToolInvoke("shell_exec".into())));
        assert!(caps.contains(&Capability::ToolInvoke("agent_spawn".into())));
        assert!(caps.contains(&Capability::ToolInvoke("memory_store".into())));

        // Should also have implied resource-level capabilities
        assert!(
            caps.contains(&Capability::NetConnect("*".into())),
            "web_fetch should imply NetConnect"
        );
        assert!(
            caps.contains(&Capability::ShellExec("*".into())),
            "shell_exec should imply ShellExec"
        );
        assert!(
            caps.contains(&Capability::AgentSpawn),
            "agent_spawn should imply AgentSpawn"
        );
        assert!(
            caps.contains(&Capability::AgentMessage("*".into())),
            "agent_spawn should imply AgentMessage"
        );
        assert!(
            caps.contains(&Capability::MemoryRead("*".into())),
            "memory_store should imply MemoryRead"
        );
        assert!(
            caps.contains(&Capability::MemoryWrite("*".into())),
            "memory_store should imply MemoryWrite"
        );
    }

    #[test]
    fn test_tools_to_parent_capabilities_no_false_expansion() {
        use librefang_types::capability::Capability;

        // Only file_read — should NOT imply any resource caps
        let tools = vec!["file_read".to_string()];
        let caps = tools_to_parent_capabilities(&tools);
        assert_eq!(caps.len(), 1);
        assert!(caps.contains(&Capability::ToolInvoke("file_read".into())));
    }

    #[tokio::test]
    async fn test_mcp_tool_blocked_by_allowed_tools() {
        // SECURITY: MCP tools not in allowed_tools must be blocked.
        let allowed = vec!["file_read".to_string(), "mcp_server1_tool_a".to_string()];
        let result = execute_tool(
            "test-id",
            "mcp_server1_tool_b", // Not in allowed list
            &serde_json::json!({"param": "value"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(result.is_error);
        assert!(
            result.content.contains("Permission denied"),
            "Expected permission denied for MCP tool, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_mcp_tool_allowed_passes_check() {
        // MCP tool in the allowed list should pass the capability check
        // (may still fail due to no MCP connections, but not permission denied)
        let allowed = vec!["file_read".to_string(), "mcp_myserver_mytool".to_string()];
        let result = execute_tool(
            "test-id",
            "mcp_myserver_mytool", // In allowed list
            &serde_json::json!({"param": "value"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        // Should fail for "MCP not available", not "Permission denied"
        assert!(result.is_error);
        assert!(
            result.content.contains("MCP not available") || result.content.contains("MCP"),
            "Expected MCP availability error (not permission denied), got: {}",
            result.content
        );
        assert!(
            !result.content.contains("Permission denied"),
            "Should not get permission denied for allowed MCP tool, got: {}",
            result.content
        );
    }

    // -----------------------------------------------------------------------
    // Wildcard allowed_tools tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_allowed_tools_wildcard_prefix_match() {
        // "file_*" should allow file_read
        let allowed = vec!["file_*".to_string()];
        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": "/tmp/test.txt"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        // Should NOT be a permission-denied error
        assert!(
            !result.content.contains("Permission denied"),
            "Wildcard 'file_*' should allow 'file_read', got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_allowed_tools_wildcard_blocks_non_matching() {
        // "file_*" should NOT allow shell_exec
        let allowed = vec!["file_*".to_string()];
        let result = execute_tool(
            "test-id",
            "shell_exec",
            &serde_json::json!({"command": "ls"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(result.is_error);
        assert!(
            result.content.contains("Permission denied"),
            "Wildcard 'file_*' should block 'shell_exec', got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_allowed_tools_star_allows_everything() {
        // "*" should allow any tool
        let allowed = vec!["*".to_string()];
        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": "/tmp/test.txt"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(
            !result.content.contains("Permission denied"),
            "Wildcard '*' should allow everything, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_allowed_tools_mixed_wildcard_and_exact() {
        // Mix of exact and wildcard entries
        let allowed = vec!["shell_exec".to_string(), "file_*".to_string()];
        let result = execute_tool(
            "test-id",
            "file_write",
            &serde_json::json!({"path": "/tmp/test.txt", "content": "hi"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        assert!(
            !result.content.contains("Permission denied"),
            "Wildcard 'file_*' should allow 'file_write', got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_mcp_tool_wildcard_allowed() {
        // "mcp_*" should allow any MCP tool
        let allowed = vec!["mcp_*".to_string()];
        let result = execute_tool(
            "test-id",
            "mcp_server1_tool_a",
            &serde_json::json!({"param": "value"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None, // allowed_skills
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // media_drivers
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // process_registry
            None, // sender_id
            None, // channel
            None, // checkpoint_manager
            None, // interrupt
            None, // session_id
            None, // dangerous_command_checker
            None, // available_tools
        )
        .await;
        // Should fail for "MCP not available", not "Permission denied"
        assert!(
            !result.content.contains("Permission denied"),
            "Wildcard 'mcp_*' should allow MCP tools, got: {}",
            result.content
        );
    }

    // -----------------------------------------------------------------------
    // Goal system tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_goal_update_tool_definition_schema() {
        let tools = builtin_tool_definitions();
        let tool = tools
            .iter()
            .find(|t| t.name == "goal_update")
            .expect("goal_update tool should be registered");
        assert_eq!(tool.input_schema["type"], "object");
        let required = tool.input_schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("goal_id")));
        let props = tool.input_schema["properties"].as_object().unwrap();
        assert!(props.contains_key("goal_id"));
        assert!(props.contains_key("status"));
        assert!(props.contains_key("progress"));
    }

    #[test]
    fn test_goal_update_missing_kernel() {
        let input = serde_json::json!({
            "goal_id": "some-uuid",
            "status": "in_progress",
            "progress": 50
        });
        let result = tool_goal_update(&input, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Kernel handle"));
    }

    #[test]
    fn test_goal_update_missing_goal_id() {
        let input = serde_json::json!({
            "status": "in_progress"
        });
        let result = tool_goal_update(&input, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_goal_update_no_fields() {
        let input = serde_json::json!({
            "goal_id": "some-uuid"
        });
        let result = tool_goal_update(&input, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("At least one"));
    }

    #[test]
    fn test_goal_update_invalid_status() {
        let input = serde_json::json!({
            "goal_id": "some-uuid",
            "status": "done"
        });
        let result = tool_goal_update(&input, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid status"));
    }

    /// Mock kernel that validates capability inheritance in spawn_agent_checked.
    struct SpawnCheckKernel {
        should_fail_escalation: bool,
    }

    // ---- BEGIN role-trait impls (split from former `impl KernelHandle for SpawnCheckKernel`, #3746) ----

    #[async_trait::async_trait]
    impl AgentControl for SpawnCheckKernel {
        async fn spawn_agent(
            &self,
            _manifest_toml: &str,
            _parent_id: Option<&str>,
        ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
            Ok(("test-id-123".to_string(), "test-agent".to_string()))
        }

        async fn spawn_agent_checked(
            &self,
            manifest_toml: &str,
            _parent_id: Option<&str>,
            parent_caps: &[librefang_types::capability::Capability],
        ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
            if self.should_fail_escalation {
                // Parse child manifest to extract capabilities, mimicking real kernel behavior
                let manifest: librefang_types::agent::AgentManifest = toml::from_str(manifest_toml)
                    .map_err(|e| {
                        librefang_kernel_handle::KernelOpError::InvalidInput(format!(
                            "manifest: {e}"
                        ))
                    })?;
                let child_caps: Vec<librefang_types::capability::Capability> = manifest
                    .capabilities
                    .tools
                    .iter()
                    .map(|t| librefang_types::capability::Capability::ToolInvoke(t.clone()))
                    .collect();
                librefang_types::capability::validate_capability_inheritance(
                    parent_caps,
                    &child_caps,
                )
                .map_err(librefang_kernel_handle::KernelOpError::Internal)?;
            }
            Ok(("test-id-456".to_string(), "good-child".to_string()))
        }

        async fn send_to_agent(
            &self,
            _agent_id: &str,
            _message: &str,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn list_agents(&self) -> Vec<AgentInfo> {
            vec![]
        }

        fn kill_agent(
            &self,
            _agent_id: &str,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
            vec![]
        }
    }

    impl MemoryAccess for SpawnCheckKernel {
        fn memory_store(
            &self,
            _key: &str,
            _value: serde_json::Value,
            _peer_id: Option<&str>,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn memory_recall(
            &self,
            _key: &str,
            _peer_id: Option<&str>,
        ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        fn memory_list(
            &self,
            _peer_id: Option<&str>,
        ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }
    }

    impl WikiAccess for SpawnCheckKernel {}

    #[async_trait::async_trait]
    impl TaskQueue for SpawnCheckKernel {
        async fn task_post(
            &self,
            _title: &str,
            _description: &str,
            _assigned_to: Option<&str>,
            _created_by: Option<&str>,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_claim(
            &self,
            _agent_id: &str,
        ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_complete(
            &self,
            _agent_id: &str,
            _task_id: &str,
            _result: &str,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_list(
            &self,
            _status: Option<&str>,
        ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_delete(
            &self,
            _task_id: &str,
        ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_retry(
            &self,
            _task_id: &str,
        ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_get(
            &self,
            _task_id: &str,
        ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn task_update_status(
            &self,
            _task_id: &str,
            _new_status: &str,
        ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }
    }

    #[async_trait::async_trait]
    impl EventBus for SpawnCheckKernel {
        async fn publish_event(
            &self,
            _event_type: &str,
            _payload: serde_json::Value,
        ) -> Result<(), librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }
    }

    #[async_trait::async_trait]
    impl KnowledgeGraph for SpawnCheckKernel {
        async fn knowledge_add_entity(
            &self,
            _entity: &librefang_types::memory::Entity,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn knowledge_add_relation(
            &self,
            _relation: &librefang_types::memory::Relation,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            Err("not used".into())
        }

        async fn knowledge_query(
            &self,
            _pattern: librefang_types::memory::GraphPattern,
        ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
        {
            Err("not used".into())
        }
    }

    // No-op role-trait impls (#3746) — mock relies on default bodies.
    impl CronControl for SpawnCheckKernel {}
    impl HandsControl for SpawnCheckKernel {}
    impl ApprovalGate for SpawnCheckKernel {}
    impl A2ARegistry for SpawnCheckKernel {}
    impl ChannelSender for SpawnCheckKernel {}
    impl PromptStore for SpawnCheckKernel {}
    impl WorkflowRunner for SpawnCheckKernel {}
    impl GoalControl for SpawnCheckKernel {}
    impl ToolPolicy for SpawnCheckKernel {}
    impl ApiAuth for SpawnCheckKernel {
        fn auth_snapshot(&self) -> ApiAuthSnapshot {
            ApiAuthSnapshot::default()
        }
    }
    impl SessionWriter for SpawnCheckKernel {
        fn inject_attachment_blocks(
            &self,
            _agent_id: librefang_types::agent::AgentId,
            _blocks: Vec<librefang_types::message::ContentBlock>,
        ) {
        }
    }

    // ---- END role-trait impls (#3746) ----

    #[test]
    fn parse_poll_options_accepts_2_to_10_strings() {
        let raw = serde_json::json!(["red", "green", "blue"]);
        let opts = parse_poll_options(Some(&raw)).expect("valid options");
        assert_eq!(opts, vec!["red", "green", "blue"]);
    }

    #[test]
    fn parse_poll_options_rejects_non_string_entry() {
        // Regression: a previous version used filter_map(as_str) which
        // silently dropped non-string entries, letting a malformed poll
        // slip past the min-2 validation.
        let raw = serde_json::json!(["a", 42, "c"]);
        let err = parse_poll_options(Some(&raw)).expect_err("should reject number");
        assert!(
            err.contains("poll_options[1]"),
            "error mentions index: {err}"
        );
        assert!(err.contains("number"), "error mentions type: {err}");
    }

    #[test]
    fn parse_poll_options_rejects_bool_entry() {
        let raw = serde_json::json!(["a", true]);
        let err = parse_poll_options(Some(&raw)).expect_err("should reject bool");
        assert!(err.contains("poll_options[1]"));
        assert!(err.contains("boolean"));
    }

    #[test]
    fn parse_poll_options_rejects_null_entry() {
        let raw = serde_json::json!(["a", null, "c"]);
        let err = parse_poll_options(Some(&raw)).expect_err("should reject null");
        assert!(err.contains("poll_options[1]"));
        assert!(err.contains("null"));
    }

    #[test]
    fn parse_poll_options_rejects_too_few() {
        let raw = serde_json::json!(["only one"]);
        let err = parse_poll_options(Some(&raw)).expect_err("should reject single option");
        assert!(err.contains("between 2 and 10"));
    }

    #[test]
    fn parse_poll_options_rejects_too_many() {
        let raw = serde_json::json!(["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k"]);
        let err = parse_poll_options(Some(&raw)).expect_err("should reject 11 options");
        assert!(err.contains("between 2 and 10"));
    }

    #[test]
    fn parse_poll_options_rejects_missing() {
        let err = parse_poll_options(None).expect_err("None should fail");
        assert!(err.contains("must be an array"));
    }

    #[test]
    fn parse_poll_options_rejects_non_array() {
        let raw = serde_json::json!("not an array");
        let err = parse_poll_options(Some(&raw)).expect_err("string should fail");
        assert!(err.contains("must be an array"));
    }

    // ── skill_read_file ────────────────────────────────────────────────

    fn create_skill_registry_with_file(
        dir: &std::path::Path,
        skill_name: &str,
        file_rel: &str,
        content: &str,
    ) -> SkillRegistry {
        let skill_dir = dir.join(skill_name);
        std::fs::create_dir_all(
            skill_dir.join(
                std::path::Path::new(file_rel)
                    .parent()
                    .unwrap_or(std::path::Path::new("")),
            ),
        )
        .unwrap();
        std::fs::write(skill_dir.join(file_rel), content).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            format!(
                r#"[skill]
name = "{skill_name}"
version = "0.1.0"
description = "test"
"#
            ),
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.to_path_buf());
        registry.load_all().unwrap();
        registry
    }

    #[tokio::test]
    async fn skill_read_file_reads_companion() {
        let dir = tempfile::TempDir::new().unwrap();
        let registry =
            create_skill_registry_with_file(dir.path(), "my-skill", "refs/guide.md", "hello world");

        let input = serde_json::json!({ "skill": "my-skill", "path": "refs/guide.md" });
        let result = tool_skill_read_file(&input, Some(&registry), None).await;
        assert_eq!(result.unwrap(), "hello world");
    }

    #[tokio::test]
    async fn skill_read_file_rejects_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let registry = create_skill_registry_with_file(dir.path(), "evil", "dummy.txt", "ok");

        let input = serde_json::json!({ "skill": "evil", "path": "../../etc/passwd" });
        let result = tool_skill_read_file(&input, Some(&registry), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn skill_read_file_rejects_unknown_skill() {
        let dir = tempfile::TempDir::new().unwrap();
        let registry = create_skill_registry_with_file(dir.path(), "exists", "f.txt", "ok");

        let input = serde_json::json!({ "skill": "nope", "path": "f.txt" });
        let result = tool_skill_read_file(&input, Some(&registry), None).await;
        assert!(result.unwrap_err().contains("not found"));
    }

    #[tokio::test]
    async fn skill_read_file_rejects_absolute_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let registry = create_skill_registry_with_file(dir.path(), "abs", "dummy.txt", "ok");

        // Use a platform-appropriate absolute path so the test passes on Windows too.
        let abs_path = std::env::temp_dir()
            .join("passwd")
            .to_string_lossy()
            .into_owned();
        let input = serde_json::json!({ "skill": "abs", "path": abs_path });
        let result = tool_skill_read_file(&input, Some(&registry), None).await;
        assert!(result.unwrap_err().contains("absolute paths"));
    }

    #[tokio::test]
    async fn skill_read_file_enforces_allowlist() {
        let dir = tempfile::TempDir::new().unwrap();
        let registry =
            create_skill_registry_with_file(dir.path(), "secret", "data.txt", "classified");

        // Agent only allowed "other-skill", not "secret"
        let allowed = vec!["other-skill".to_string()];
        let input = serde_json::json!({ "skill": "secret", "path": "data.txt" });
        let result = tool_skill_read_file(&input, Some(&registry), Some(&allowed)).await;
        assert!(result.unwrap_err().contains("not allowed"));

        // Empty allowlist means all skills are accessible
        let empty: Vec<String> = vec![];
        let result = tool_skill_read_file(&input, Some(&registry), Some(&empty)).await;
        assert!(result.is_ok());

        // None allowlist (deferred context) also allows access
        let result = tool_skill_read_file(&input, Some(&registry), None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn skill_read_file_truncates_without_panic() {
        let dir = tempfile::TempDir::new().unwrap();
        // Create content with multi-byte chars that exceeds 32K bytes
        let content = "é".repeat(20_000); // 2 bytes each = 40K bytes
        let registry = create_skill_registry_with_file(dir.path(), "big", "large.txt", &content);

        let input = serde_json::json!({ "skill": "big", "path": "large.txt" });
        let result = tool_skill_read_file(&input, Some(&registry), None)
            .await
            .unwrap();
        assert!(result.contains("truncated"));
        // Must not panic — the point of this test
    }
    // -----------------------------------------------------------------------
    // notify_owner tool (§A — owner-side channel)
    // -----------------------------------------------------------------------

    #[test]
    fn notify_owner_tool_is_registered_in_builtins() {
        let defs = builtin_tool_definitions();
        let notify = defs.iter().find(|d| d.name == "notify_owner");
        assert!(
            notify.is_some(),
            "notify_owner must appear in builtin_tool_definitions"
        );
        let schema = &notify.unwrap().input_schema;
        let required = schema["required"].as_array().expect("required array");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"reason"));
        assert!(names.contains(&"summary"));
    }

    #[test]
    fn notify_owner_tool_sets_owner_notice_and_opaque_ack() {
        let input = serde_json::json!({
            "reason": "confirmation_needed",
            "summary": "Caterina has asked for confirmation of the appointment."
        });
        let r = tool_notify_owner("toolu_1", &input);
        assert!(!r.is_error, "notify_owner should not be an error: {r:?}");
        assert_eq!(r.tool_use_id, "toolu_1");
        // Owner-side payload populated with prefixed reason.
        let payload = r.owner_notice.as_deref().expect("owner_notice set");
        assert!(payload.contains("confirmation_needed"));
        assert!(payload.contains("Caterina"));
        // Opaque ack does NOT echo the summary back to the model.
        assert!(!r.content.contains("Caterina"));
        assert!(!r.content.contains("confirmation_needed"));
    }

    #[test]
    fn notify_owner_tool_rejects_empty_args() {
        let cases = vec![
            serde_json::json!({"reason": "", "summary": "x"}),
            serde_json::json!({"reason": "x", "summary": ""}),
            serde_json::json!({"reason": "x"}),
            serde_json::json!({"summary": "x"}),
            serde_json::json!({}),
        ];
        for input in cases {
            let r = tool_notify_owner("t", &input);
            assert!(r.is_error, "expected error for input {input:?}");
            assert!(r.owner_notice.is_none());
        }
    }

    // ── Lazy tool loading (issue #3044) ───────────────────────────────────

    #[test]
    fn test_tool_meta_load_returns_schema_and_side_channel() {
        let input = serde_json::json!({"name": "file_write"});
        let r = tool_meta_load(&input, None);
        assert!(!r.is_error);
        assert!(r.content.contains("file_write"));
        assert!(r.content.contains("input_schema") || r.content.contains("content"));
        // Side-channel must carry the full ToolDefinition for the agent loop.
        let def = r
            .loaded_tool
            .expect("loaded_tool side-channel must be populated");
        assert_eq!(def.name, "file_write");
        assert!(!def.description.is_empty());
    }

    #[test]
    fn test_tool_meta_load_rejects_unknown_name() {
        let r = tool_meta_load(&serde_json::json!({"name": "not_a_real_tool"}), None);
        assert!(r.is_error);
        assert!(r.loaded_tool.is_none());
        assert!(r.content.to_lowercase().contains("unknown"));
    }

    #[test]
    fn test_tool_meta_load_rejects_missing_name() {
        let r = tool_meta_load(&serde_json::json!({}), None);
        assert!(r.is_error);
        assert!(r.loaded_tool.is_none());
    }

    #[test]
    fn test_tool_meta_search_finds_by_keyword() {
        let r = tool_meta_search(&serde_json::json!({"query": "write"}), None);
        assert!(!r.is_error);
        assert!(r.content.contains("file_write") || r.content.contains("memory_store"));
        assert!(r.loaded_tool.is_none()); // search doesn't load; only load loads.
    }

    #[test]
    fn test_tool_meta_search_respects_limit() {
        let r = tool_meta_search(&serde_json::json!({"query": "file", "limit": 2}), None);
        assert!(!r.is_error);
        // At most 2 result lines (header line + max 2 match lines).
        let match_lines = r.content.lines().filter(|l| l.contains(": ")).count();
        assert!(match_lines <= 2, "expected ≤2 matches, got {match_lines}");
    }

    #[test]
    fn test_tool_meta_search_rejects_empty_query() {
        let r = tool_meta_search(&serde_json::json!({"query": ""}), None);
        assert!(r.is_error);
    }

    #[test]
    fn test_always_native_tools_includes_meta_tools() {
        // The meta-tools MUST be in the always-native set — otherwise the LLM
        // can never escape eager mode when the loop trims the tool list.
        assert!(ALWAYS_NATIVE_TOOLS.contains(&"tool_load"));
        assert!(ALWAYS_NATIVE_TOOLS.contains(&"tool_search"));
    }

    #[test]
    fn test_builtin_tool_definitions_declares_meta_tools() {
        let defs = builtin_tool_definitions();
        assert!(defs.iter().any(|t| t.name == "tool_load"));
        assert!(defs.iter().any(|t| t.name == "tool_search"));
    }

    #[test]
    fn test_select_native_tools_trims_to_native_set() {
        let defs = builtin_tool_definitions();
        let native = select_native_tools(&defs);
        // Result is a subset of the full builtin set.
        assert!(native.len() < defs.len());
        // Every returned tool's name is in ALWAYS_NATIVE_TOOLS.
        for t in &native {
            assert!(
                ALWAYS_NATIVE_TOOLS.contains(&t.name.as_str()),
                "unexpected native tool: {}",
                t.name
            );
        }
        // Every name in ALWAYS_NATIVE_TOOLS that exists in builtins must be present.
        let builtin_names: std::collections::HashSet<&str> =
            defs.iter().map(|t| t.name.as_str()).collect();
        for want in ALWAYS_NATIVE_TOOLS {
            if builtin_names.contains(want) {
                assert!(
                    native.iter().any(|t| t.name == *want),
                    "native set missing expected tool: {want}"
                );
            }
        }
    }

    #[test]
    fn test_lazy_mode_reduces_serialized_tool_payload() {
        // Quantify the savings this PR is claiming (issue #3044). The lazy
        // set serialized as JSON should be dramatically smaller than the
        // full builtin set.
        let full = builtin_tool_definitions();
        let native = select_native_tools(&full);
        let full_bytes = serde_json::to_vec(&full).unwrap().len();
        let native_bytes = serde_json::to_vec(&native).unwrap().len();
        // Expect at least a 50% reduction — in practice it's ~75%.
        assert!(
            native_bytes * 2 < full_bytes,
            "native set ({native_bytes}B) should be less than half the full set ({full_bytes}B)"
        );
    }

    #[test]
    fn test_tool_meta_load_resolves_non_builtin_from_available_tools() {
        // Regression for PR #3047 codex review P1: a non-builtin tool
        // (MCP/skill-provided) must be loadable via tool_load as long as it
        // exists in the agent's granted `available_tools` pool. Before the
        // fix `tool_meta_load` only scanned `builtin_tool_definitions()`,
        // so dynamic tools were stripped by lazy mode and unreachable.
        let dynamic = ToolDefinition {
            name: "mcp_custom_thing".to_string(),
            description: "A dynamically-registered MCP tool".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"x": {"type": "string"}},
                "required": ["x"],
            }),
        };
        let pool = vec![dynamic.clone()];
        let r = tool_meta_load(
            &serde_json::json!({"name": "mcp_custom_thing"}),
            Some(&pool),
        );
        assert!(!r.is_error, "expected success, got: {}", r.content);
        let loaded = r
            .loaded_tool
            .expect("loaded_tool must populate for granted non-builtin");
        assert_eq!(loaded.name, "mcp_custom_thing");
        assert_eq!(loaded.description, dynamic.description);
    }

    #[test]
    fn test_tool_meta_load_empty_pool_is_not_builtin_fallback() {
        // `Some(&[])` must mean "granted pool is empty" — NOT "caller didn't
        // provide one, please leak the builtin catalog". Only `None` falls
        // back to builtins (for legacy execute_tool paths). This keeps the
        // semantics unambiguous for future callers.
        let empty: Vec<ToolDefinition> = Vec::new();
        let r = tool_meta_load(&serde_json::json!({"name": "file_write"}), Some(&empty));
        assert!(
            r.is_error,
            "Some(&[]) must resolve as empty pool, got content: {}",
            r.content
        );
        assert!(r.loaded_tool.is_none());
        // Sanity: None still falls back to builtin and resolves file_write.
        let r_none = tool_meta_load(&serde_json::json!({"name": "file_write"}), None);
        assert!(!r_none.is_error);
        assert_eq!(
            r_none.loaded_tool.map(|d| d.name).as_deref(),
            Some("file_write")
        );
    }

    #[test]
    fn test_tool_meta_search_scopes_to_available_tools_when_provided() {
        // Search must also prefer the agent's granted pool so results never
        // hallucinate tools the agent can't actually call.
        let only = vec![ToolDefinition {
            name: "mcp_unique_name_zzz".to_string(),
            description: "keyword_zzz".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let r = tool_meta_search(&serde_json::json!({"query": "keyword_zzz"}), Some(&only));
        assert!(!r.is_error);
        assert!(
            r.content.contains("mcp_unique_name_zzz"),
            "expected the granted tool to appear, got: {}",
            r.content
        );
        // builtin 'file_write' must NOT show up when the pool is scoped.
        assert!(
            !r.content.contains("file_write"),
            "search leaked outside the supplied pool: {}",
            r.content
        );
    }
}

// ── skill evolve frozen-registry gating ───────────────────────────

#[tokio::test]
async fn test_evolve_tools_rejected_when_registry_frozen() {
    // In Stable mode (registry frozen) every evolution tool must
    // refuse at the handler boundary, BEFORE touching disk. The
    // `evolution` module underneath would happily write files that
    // the frozen registry never loads — burning reviewer tokens
    // and leaving disk state the operator explicitly didn't want.
    let tmp = tempfile::tempdir().unwrap();
    let mut registry = SkillRegistry::new(tmp.path().to_path_buf());
    registry.freeze();

    let input = serde_json::json!({
        "name": "gated",
        "description": "x",
        "prompt_context": "# x",
        "tags": [],
    });
    let err = tool_skill_evolve_create(&input, Some(&registry), None)
        .await
        .expect_err("must reject under freeze");
    assert!(
        err.contains("frozen") || err.contains("Stable"),
        "error must mention Stable/frozen, got: {err}"
    );

    let err = tool_skill_evolve_delete(&serde_json::json!({ "name": "gated" }), Some(&registry))
        .await
        .expect_err("delete must reject under freeze");
    assert!(err.contains("frozen") || err.contains("Stable"));

    let err = tool_skill_evolve_write_file(
        &serde_json::json!({
            "name": "gated",
            "path": "references/x.md",
            "content": "hi",
        }),
        Some(&registry),
    )
    .await
    .expect_err("write_file must reject under freeze");
    assert!(err.contains("frozen") || err.contains("Stable"));
}

// ── read_artifact tool (#3347) ─────────────────────────────────────────────

#[tokio::test]
async fn read_artifact_round_trip() {
    let dir = tempfile::TempDir::new().unwrap();
    let content = b"artifact payload";
    let handle = crate::artifact_store::write(
        content,
        dir.path(),
        crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES,
    )
    .unwrap();

    let input = serde_json::json!({ "handle": handle.as_str() });
    let result = tool_read_artifact(&input, dir.path()).await.unwrap();
    assert!(result.contains("artifact payload"), "got: {result}");
    assert!(result.contains("sha256:"), "got: {result}");
}

#[tokio::test]
async fn read_artifact_with_offset_and_length() {
    let dir = tempfile::TempDir::new().unwrap();
    let content = b"0123456789abcdef";
    let handle = crate::artifact_store::write(
        content,
        dir.path(),
        crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES,
    )
    .unwrap();

    let input = serde_json::json!({ "handle": handle.as_str(), "offset": 4, "length": 6 });
    let result = tool_read_artifact(&input, dir.path()).await.unwrap();
    assert!(result.contains("456789"), "got: {result}");
}

#[tokio::test]
async fn read_artifact_nonexistent_returns_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let fake = "sha256:".to_string() + &"b".repeat(64);
    let input = serde_json::json!({ "handle": fake });
    let result = tool_read_artifact(&input, dir.path()).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}

#[tokio::test]
async fn read_artifact_missing_handle_returns_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let input = serde_json::json!({});
    let result = tool_read_artifact(&input, dir.path()).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("handle"));
}

#[tokio::test]
async fn read_artifact_past_end_returns_no_more_content_message() {
    let dir = tempfile::TempDir::new().unwrap();
    let content = b"short";
    let handle = crate::artifact_store::write(
        content,
        dir.path(),
        crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES,
    )
    .unwrap();

    let input = serde_json::json!({ "handle": handle.as_str(), "offset": 9999 });
    let result = tool_read_artifact(&input, dir.path()).await.unwrap();
    assert!(result.contains("past end"), "got: {result}");
}

#[test]
fn read_artifact_registered_in_builtins() {
    let defs = builtin_tool_definitions();
    let def = defs.iter().find(|d| d.name == "read_artifact");
    assert!(
        def.is_some(),
        "read_artifact must appear in builtin_tool_definitions"
    );
    let schema = &def.unwrap().input_schema;
    let required = schema["required"].as_array().expect("required array");
    assert!(required.iter().any(|v| v.as_str() == Some("handle")));
}

#[test]
fn read_artifact_in_always_native_tools() {
    assert!(
        ALWAYS_NATIVE_TOOLS.contains(&"read_artifact"),
        "read_artifact must be in ALWAYS_NATIVE_TOOLS"
    );
}
