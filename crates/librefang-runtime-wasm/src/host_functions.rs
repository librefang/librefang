//! Host function implementations for the WASM sandbox.
//!
//! Each function checks capabilities before executing. Deny-by-default:
//! if no matching capability is found, the operation is rejected.
//!
//! These functions are called from the `host_call` dispatch in `sandbox.rs`.
//! They receive `&GuestState` (not `&mut`) and return JSON values.

use crate::sandbox::GuestState;
use librefang_types::capability::{capability_matches, Capability};
use serde_json::json;
use std::net::ToSocketAddrs;
use std::path::{Component, Path};
use tracing::debug;

/// Dispatch a host call to the appropriate handler.
///
/// Returns JSON: `{"ok": ...}` on success, `{"error": "..."}` on failure.
pub fn dispatch(state: &GuestState, method: &str, params: &serde_json::Value) -> serde_json::Value {
    debug!(method, "WASM host_call dispatch");
    match method {
        // Always allowed (no capability check)
        "time_now" => host_time_now(),

        // Filesystem — requires FileRead/FileWrite
        "fs_read" => host_fs_read(state, params),
        "fs_write" => host_fs_write(state, params),
        "fs_list" => host_fs_list(state, params),

        // Network — requires NetConnect
        "net_fetch" => host_net_fetch(state, params),

        // Shell — requires ShellExec
        "shell_exec" => host_shell_exec(state, params),

        // Environment — requires EnvRead
        "env_read" => host_env_read(state, params),

        // Memory KV — requires MemoryRead/MemoryWrite
        "kv_get" => host_kv_get(state, params),
        "kv_set" => host_kv_set(state, params),

        // Agent interaction — requires AgentMessage/AgentSpawn
        "agent_send" => host_agent_send(state, params),
        "agent_spawn" => host_agent_spawn(state, params),

        _ => json!({"error": format!("Unknown host method: {method}")}),
    }
}

// ---------------------------------------------------------------------------
// Capability checking
// ---------------------------------------------------------------------------

/// Check that the guest has a capability matching `required`.
/// Returns `Ok(())` if granted, `Err(json)` with an error response if denied.
fn check_capability(
    capabilities: &[Capability],
    required: &Capability,
) -> Result<(), serde_json::Value> {
    for granted in capabilities {
        if capability_matches(granted, required) {
            return Ok(());
        }
    }
    Err(json!({"error": format!("Capability denied: {required:?}")}))
}

// ---------------------------------------------------------------------------
// Path traversal protection
// ---------------------------------------------------------------------------

/// Secure path resolution — NEVER returns raw unchecked paths.
/// Rejects traversal components, resolves symlinks where possible.
fn safe_resolve_path(path: &str) -> Result<std::path::PathBuf, serde_json::Value> {
    let p = Path::new(path);

    // Phase 1: Reject any path with ".." components (even if they'd resolve safely)
    for component in p.components() {
        if matches!(component, Component::ParentDir) {
            return Err(json!({"error": "Path traversal denied: '..' components forbidden"}));
        }
    }

    // Phase 2: Canonicalize to resolve symlinks and normalize
    std::fs::canonicalize(p).map_err(|e| json!({"error": format!("Cannot resolve path: {e}")}))
}

/// For writes where the file may not exist yet: canonicalize the parent, validate the filename.
fn safe_resolve_parent(path: &str) -> Result<std::path::PathBuf, serde_json::Value> {
    let p = Path::new(path);

    for component in p.components() {
        if matches!(component, Component::ParentDir) {
            return Err(json!({"error": "Path traversal denied: '..' components forbidden"}));
        }
    }

    let parent = p
        .parent()
        .filter(|par| !par.as_os_str().is_empty())
        .ok_or_else(|| json!({"error": "Invalid path: no parent directory"}))?;

    let canonical_parent = std::fs::canonicalize(parent)
        .map_err(|e| json!({"error": format!("Cannot resolve parent directory: {e}")}))?;

    let file_name = p
        .file_name()
        .ok_or_else(|| json!({"error": "Invalid path: no file name"}))?;

    // Double-check filename doesn't contain traversal (belt-and-suspenders)
    if file_name.to_string_lossy().contains("..") {
        return Err(json!({"error": "Path traversal denied in file name"}));
    }

    Ok(canonical_parent.join(file_name))
}

// ---------------------------------------------------------------------------
// SSRF protection
// ---------------------------------------------------------------------------

/// SSRF-validated DNS resolution result for the WASM host function path.
struct SsrfResolved {
    hostname: String,
    resolved: Vec<std::net::SocketAddr>,
}

/// SSRF protection: check if a hostname resolves to a private/internal IP.
/// Returns the resolved addresses on success so the caller can pin DNS and
/// prevent TOCTOU / DNS-rebinding attacks.
fn is_ssrf_target(url: &str) -> Result<SsrfResolved, serde_json::Value> {
    // Only allow http:// and https:// schemes (block file://, gopher://, ftp://)
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(json!({"error": "Only http:// and https:// URLs are allowed"}));
    }

    let host = extract_host_from_url(url);
    let hostname = host.split(':').next().unwrap_or(&host);

    // Check hostname-based blocklist first (catches metadata endpoints)
    let blocked_hostnames = [
        "localhost",
        "metadata.google.internal",
        "metadata.aws.internal",
        "instance-data",
        "169.254.169.254",
    ];
    if blocked_hostnames.contains(&hostname) {
        return Err(json!({"error": format!("SSRF blocked: {hostname} is a restricted hostname")}));
    }

    // Resolve DNS and check every returned IP
    let port = if url.starts_with("https") { 443 } else { 80 };
    let socket_addr = format!("{hostname}:{port}");
    let mut resolved = Vec::new();
    match socket_addr.to_socket_addrs() {
        Ok(addrs) => {
            for addr in addrs {
                // Canonicalise IPv4-mapped IPv6 (::ffff:X.X.X.X) before any
                // safety check — see canonical_ip below.
                let ip = canonical_ip(&addr.ip());
                if ip.is_loopback() || ip.is_unspecified() || is_private_ip(&ip) {
                    return Err(json!({"error": format!(
                        "SSRF blocked: {hostname} resolves to private IP {ip}"
                    )}));
                }
                resolved.push(addr);
            }
        }
        Err(e) => {
            return Err(json!({"error": format!(
                "SSRF blocked: DNS resolution failed for {hostname}: {e}"
            )}));
        }
    }
    if resolved.is_empty() {
        return Err(json!({"error": format!(
            "SSRF blocked: DNS resolution returned no addresses for {hostname}"
        )}));
    }
    Ok(SsrfResolved {
        hostname: hostname.to_string(),
        resolved,
    })
}

/// Unwrap IPv4-mapped IPv6 (`::ffff:X.X.X.X`) to its IPv4 form. All other
/// addresses are returned unchanged. Keeps downstream IP checks operating on
/// the address the OS will actually connect to.
fn canonical_ip(ip: &std::net::IpAddr) -> std::net::IpAddr {
    match ip {
        std::net::IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => std::net::IpAddr::V4(v4),
            None => std::net::IpAddr::V6(*v6),
        },
        std::net::IpAddr::V4(_) => *ip,
    }
}

fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match canonical_ip(ip) {
        std::net::IpAddr::V4(v4) => {
            let octets = v4.octets();
            matches!(
                octets,
                [10, ..] | [172, 16..=31, ..] | [192, 168, ..] | [169, 254, ..]
            )
        }
        std::net::IpAddr::V6(v6) => {
            let segments = v6.segments();
            (segments[0] & 0xfe00) == 0xfc00 || (segments[0] & 0xffc0) == 0xfe80
        }
    }
}

// ---------------------------------------------------------------------------
// Always-allowed functions
// ---------------------------------------------------------------------------

fn host_time_now() -> serde_json::Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    json!({"ok": now})
}

// ---------------------------------------------------------------------------
// Filesystem (capability-checked)
// ---------------------------------------------------------------------------

fn host_fs_read(state: &GuestState, params: &serde_json::Value) -> serde_json::Value {
    let path = match params.get("path").and_then(|p| p.as_str()) {
        Some(p) => p,
        None => return json!({"error": "Missing 'path' parameter"}),
    };
    // SECURITY: Canonicalize first so the capability check sees the real path,
    // not an attacker-controlled raw string with "../" sequences.
    let canonical = match safe_resolve_path(path) {
        Ok(c) => c,
        Err(e) => return e,
    };
    // Capability check against the canonical path prevents traversal bypass.
    if let Err(e) = check_capability(
        &state.capabilities,
        &Capability::FileRead(canonical.to_string_lossy().into_owned()),
    ) {
        return e;
    }
    match std::fs::read_to_string(&canonical) {
        Ok(content) => json!({"ok": content}),
        Err(e) => json!({"error": format!("fs_read failed: {e}")}),
    }
}

fn host_fs_write(state: &GuestState, params: &serde_json::Value) -> serde_json::Value {
    let path = match params.get("path").and_then(|p| p.as_str()) {
        Some(p) => p,
        None => return json!({"error": "Missing 'path' parameter"}),
    };
    let content = match params.get("content").and_then(|c| c.as_str()) {
        Some(c) => c,
        None => return json!({"error": "Missing 'content' parameter"}),
    };
    // SECURITY: Canonicalize (via parent) first so the capability check sees
    // the real destination, not a raw path with "../" traversal sequences.
    let write_path = match safe_resolve_parent(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    // Capability check against the canonical path prevents traversal bypass.
    if let Err(e) = check_capability(
        &state.capabilities,
        &Capability::FileWrite(write_path.to_string_lossy().into_owned()),
    ) {
        return e;
    }
    match std::fs::write(&write_path, content) {
        Ok(()) => json!({"ok": true}),
        Err(e) => json!({"error": format!("fs_write failed: {e}")}),
    }
}

fn host_fs_list(state: &GuestState, params: &serde_json::Value) -> serde_json::Value {
    let path = match params.get("path").and_then(|p| p.as_str()) {
        Some(p) => p,
        None => return json!({"error": "Missing 'path' parameter"}),
    };
    // SECURITY: Canonicalize first so the capability check sees the real path,
    // not an attacker-controlled raw string with "../" sequences.
    let canonical = match safe_resolve_path(path) {
        Ok(c) => c,
        Err(e) => return e,
    };
    // Capability check against the canonical path prevents traversal bypass.
    if let Err(e) = check_capability(
        &state.capabilities,
        &Capability::FileRead(canonical.to_string_lossy().into_owned()),
    ) {
        return e;
    }
    match std::fs::read_dir(&canonical) {
        Ok(entries) => {
            let names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            json!({"ok": names})
        }
        Err(e) => json!({"error": format!("fs_list failed: {e}")}),
    }
}

// ---------------------------------------------------------------------------
// Network (capability-checked)
// ---------------------------------------------------------------------------

fn host_net_fetch(state: &GuestState, params: &serde_json::Value) -> serde_json::Value {
    let url = match params.get("url").and_then(|u| u.as_str()) {
        Some(u) => u,
        None => return json!({"error": "Missing 'url' parameter"}),
    };
    let method = params
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("GET");
    let body = params.get("body").and_then(|b| b.as_str()).unwrap_or("");

    // SECURITY: SSRF protection — resolve DNS once and validate IPs
    let ssrf_result = match is_ssrf_target(url) {
        Ok(r) => r,
        Err(e) => return e,
    };

    // Extract host:port from URL for capability check
    let host = extract_host_from_url(url);
    if let Err(e) = check_capability(&state.capabilities, &Capability::NetConnect(host)) {
        return e;
    }

    // SECURITY (Bug #3867): WASM execution runs inside `tokio::task::spawn_blocking`.
    // Using `Handle::block_on` directly inside a blocking thread can deadlock
    // on single-threaded runtimes and — more critically — bypasses the
    // epoch-based watchdog: the epoch only ticks inside the async context, so a
    // blocking async call that stalls never trips the timeout.
    //
    // `tokio::task::block_in_place` tells the runtime to move other tasks off
    // the current thread before blocking, which avoids the deadlock and keeps
    // the scheduler responsive.  The epoch watchdog thread is unaffected —
    // it runs independently and will still fire `increment_epoch` when the
    // wall-clock deadline is reached.
    tokio::task::block_in_place(|| {
        state.tokio_handle.block_on(async {
            // Build a DNS-pinned client so the HTTP request connects to the
            // same IPs we already validated (prevents DNS-rebinding TOCTOU).
            let mut builder = librefang_http::proxied_client_builder();
            for addr in &ssrf_result.resolved {
                builder = builder.resolve(&ssrf_result.hostname, *addr);
            }
            let client = builder.build().expect("HTTP client build");
            let request = match method.to_uppercase().as_str() {
                "POST" => client.post(url).body(body.to_string()),
                "PUT" => client.put(url).body(body.to_string()),
                "DELETE" => client.delete(url),
                _ => client.get(url),
            };
            match request.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    match resp.text().await {
                        Ok(text) => json!({"ok": {"status": status, "body": text}}),
                        Err(e) => json!({"error": format!("Failed to read response: {e}")}),
                    }
                }
                Err(e) => json!({"error": format!("Request failed: {e}")}),
            }
        })
    })
}

/// Extract host:port from a URL for capability checking.
fn extract_host_from_url(url: &str) -> String {
    if let Some(after_scheme) = url.split("://").nth(1) {
        let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);
        if host_port.contains(':') {
            host_port.to_string()
        } else if url.starts_with("https") {
            format!("{host_port}:443")
        } else {
            format!("{host_port}:80")
        }
    } else {
        url.to_string()
    }
}

// ---------------------------------------------------------------------------
// Shell (capability-checked)
// ---------------------------------------------------------------------------

/// Environment variables re-added after `env_clear` on a sandboxed child
/// process. Mirrors the list in `librefang-runtime/src/subprocess_sandbox.rs`
/// so that WASM guests invoking `shell_exec` get the same stripped-down
/// environment as the top-level shell tool. Keeping the list inline (rather
/// than taking a dependency on `librefang-runtime`) avoids a crate cycle.
const WASM_SHELL_SAFE_ENV_VARS: &[&str] = &[
    "PATH", "HOME", "TMPDIR", "TMP", "TEMP", "LANG", "LC_ALL", "TERM",
];

/// Clear the child's environment and re-add only the safe allowlist. The
/// WASM sandbox path used to skip this, so an agent whose `ShellExec`
/// capability was granted inherited the entire daemon environment — every
/// LLM provider API key, vault master key override, cloud metadata token,
/// etc. — regardless of how tightly its Wasm fuel and epoch budget were
/// capped. This closes that exfiltration hole while leaving the capability
/// gate above untouched.
fn sanitize_shell_env(cmd: &mut std::process::Command) {
    cmd.env_clear();
    for var in WASM_SHELL_SAFE_ENV_VARS {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }
    #[cfg(windows)]
    for var in [
        "USERPROFILE",
        "SYSTEMROOT",
        "APPDATA",
        "LOCALAPPDATA",
        "COMSPEC",
        "WINDIR",
        "PATHEXT",
    ] {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }
}

fn host_shell_exec(state: &GuestState, params: &serde_json::Value) -> serde_json::Value {
    let command = match params.get("command").and_then(|c| c.as_str()) {
        Some(c) => c,
        None => return json!({"error": "Missing 'command' parameter"}),
    };
    if let Err(e) = check_capability(
        &state.capabilities,
        &Capability::ShellExec(command.to_string()),
    ) {
        return e;
    }

    let args: Vec<&str> = params
        .get("args")
        .and_then(|a| a.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    // Command::new does NOT use a shell — safe from shell injection.
    // Each argument is passed directly to the process.
    let mut cmd = std::process::Command::new(command);
    cmd.args(&args);
    sanitize_shell_env(&mut cmd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    match cmd.output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            json!({
                "ok": {
                    "exit_code": output.status.code(),
                    "stdout": stdout,
                    "stderr": stderr,
                }
            })
        }
        Err(e) => json!({"error": format!("shell_exec failed: {e}")}),
    }
}

// ---------------------------------------------------------------------------
// Environment (capability-checked)
// ---------------------------------------------------------------------------

fn host_env_read(state: &GuestState, params: &serde_json::Value) -> serde_json::Value {
    let name = match params.get("name").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => return json!({"error": "Missing 'name' parameter"}),
    };
    if let Err(e) = check_capability(&state.capabilities, &Capability::EnvRead(name.to_string())) {
        return e;
    }
    match std::env::var(name) {
        Ok(val) => json!({"ok": val}),
        Err(_) => json!({"ok": null}),
    }
}

// ---------------------------------------------------------------------------
// Memory KV (capability-checked, uses kernel handle)
// ---------------------------------------------------------------------------

fn host_kv_get(state: &GuestState, params: &serde_json::Value) -> serde_json::Value {
    let key = match params.get("key").and_then(|k| k.as_str()) {
        Some(k) => k,
        None => return json!({"error": "Missing 'key' parameter"}),
    };
    if let Err(e) = check_capability(
        &state.capabilities,
        &Capability::MemoryRead(key.to_string()),
    ) {
        return e;
    }
    let kernel = match &state.kernel {
        Some(k) => k,
        None => return json!({"error": "No kernel handle available"}),
    };
    match kernel.memory_recall(key, None) {
        Ok(Some(val)) => json!({"ok": val}),
        Ok(None) => json!({"ok": null}),
        Err(e) => json!({"error": e}),
    }
}

fn host_kv_set(state: &GuestState, params: &serde_json::Value) -> serde_json::Value {
    let key = match params.get("key").and_then(|k| k.as_str()) {
        Some(k) => k,
        None => return json!({"error": "Missing 'key' parameter"}),
    };
    let value = match params.get("value") {
        Some(v) => v.clone(),
        None => return json!({"error": "Missing 'value' parameter"}),
    };
    if let Err(e) = check_capability(
        &state.capabilities,
        &Capability::MemoryWrite(key.to_string()),
    ) {
        return e;
    }
    let kernel = match &state.kernel {
        Some(k) => k,
        None => return json!({"error": "No kernel handle available"}),
    };
    match kernel.memory_store(key, value, None) {
        Ok(()) => json!({"ok": true}),
        Err(e) => json!({"error": e}),
    }
}

// ---------------------------------------------------------------------------
// Agent interaction (capability-checked, uses kernel handle)
// ---------------------------------------------------------------------------

fn host_agent_send(state: &GuestState, params: &serde_json::Value) -> serde_json::Value {
    let target = match params.get("target").and_then(|t| t.as_str()) {
        Some(t) => t,
        None => return json!({"error": "Missing 'target' parameter"}),
    };
    let message = match params.get("message").and_then(|m| m.as_str()) {
        Some(m) => m,
        None => return json!({"error": "Missing 'message' parameter"}),
    };
    if let Err(e) = check_capability(
        &state.capabilities,
        &Capability::AgentMessage(target.to_string()),
    ) {
        return e;
    }
    let kernel = match &state.kernel {
        Some(k) => k,
        None => return json!({"error": "No kernel handle available"}),
    };
    // SECURITY (Bug #3867): use block_in_place so tokio can move other tasks
    // off this thread before blocking, avoiding deadlock and preserving the
    // epoch watchdog's ability to interrupt.
    tokio::task::block_in_place(|| {
        match state
            .tokio_handle
            .block_on(kernel.send_to_agent(target, message))
        {
            Ok(response) => json!({"ok": response}),
            Err(e) => json!({"error": e}),
        }
    })
}

fn host_agent_spawn(state: &GuestState, params: &serde_json::Value) -> serde_json::Value {
    if let Err(e) = check_capability(&state.capabilities, &Capability::AgentSpawn) {
        return e;
    }
    let manifest_toml = match params.get("manifest").and_then(|m| m.as_str()) {
        Some(m) => m,
        None => return json!({"error": "Missing 'manifest' parameter"}),
    };
    let kernel = match &state.kernel {
        Some(k) => k,
        None => return json!({"error": "No kernel handle available"}),
    };
    // SECURITY: Enforce capability inheritance — child <= parent
    // SECURITY (Bug #3867): use block_in_place so tokio can move other tasks
    // off this thread before blocking, avoiding deadlock and preserving the
    // epoch watchdog's ability to interrupt.
    tokio::task::block_in_place(|| {
        match state.tokio_handle.block_on(kernel.spawn_agent_checked(
            manifest_toml,
            Some(&state.agent_id),
            &state.capabilities,
        )) {
            Ok((id, name)) => json!({"ok": {"id": id, "name": name}}),
            Err(e) => json!({"error": e}),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state(capabilities: Vec<Capability>) -> GuestState {
        GuestState {
            capabilities,
            kernel: None,
            agent_id: "test-agent".to_string(),
            tokio_handle: tokio::runtime::Handle::current(),
        }
    }

    #[tokio::test]
    async fn test_time_now_always_allowed() {
        let result = host_time_now();
        assert!(result.get("ok").is_some());
        let ts = result["ok"].as_u64().unwrap();
        assert!(ts > 1_700_000_000);
    }

    #[tokio::test]
    async fn test_fs_read_denied_no_capability() {
        let state = test_state(vec![]);
        let result = host_fs_read(&state, &json!({"path": "/etc/passwd"}));
        let err = result["error"].as_str().unwrap();
        assert!(err.contains("denied"));
    }

    #[tokio::test]
    async fn test_fs_write_denied_no_capability() {
        let state = test_state(vec![]);
        let result = host_fs_write(&state, &json!({"path": "/tmp/test", "content": "hello"}));
        let err = result["error"].as_str().unwrap();
        assert!(err.contains("denied"));
    }

    #[tokio::test]
    async fn test_fs_read_granted_wildcard() {
        let state = test_state(vec![Capability::FileRead("*".to_string())]);
        let result = host_fs_read(&state, &json!({"path": "Cargo.toml"}));
        // Should not be capability-denied (may still fail on path)
        if let Some(err) = result.get("error") {
            let msg = err.as_str().unwrap_or("");
            assert!(
                !msg.contains("denied"),
                "Should not be capability-denied: {msg}"
            );
        }
    }

    #[tokio::test]
    async fn test_shell_exec_denied() {
        let state = test_state(vec![]);
        let result = host_shell_exec(&state, &json!({"command": "ls"}));
        let err = result["error"].as_str().unwrap();
        assert!(err.contains("denied"));
    }

    /// Regression: a WASM guest with an explicit `ShellExec("*")` capability
    /// used to inherit the daemon's full environment, including every LLM
    /// provider API key. The fix strips the env before exec so only the
    /// hard-coded safe allowlist (PATH, HOME, LANG, …) survives. Stamp a
    /// fake secret into the parent environment, drive the host call, and
    /// verify that the child's `env` output does not contain it.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_shell_exec_strips_parent_env_secrets() {
        // Use a unique key per run so concurrent tests don't collide.
        let key = format!("LF_WASM_FAKE_SECRET_{}", std::process::id());
        let value = "sk-should-not-reach-child";
        // SAFETY: key is unique per-process (includes PID); no other test
        // thread races on this particular env var.
        unsafe { std::env::set_var(&key, value) };

        // Use the explicit absolute path so the capability check passes even
        // with the new separator-aware glob — `*` does not cross `/` so we
        // grant the exact command we are about to run.
        let state = test_state(vec![Capability::ShellExec("/usr/bin/env".to_string())]);
        let result = host_shell_exec(
            &state,
            &json!({
                "command": "/usr/bin/env",
                "args": [],
            }),
        );

        // Tidy up the parent env regardless of assertion outcome.
        std::env::remove_var(&key);

        let ok = result
            .get("ok")
            .expect("shell_exec should succeed with matching ShellExec capability");
        let stdout = ok
            .get("stdout")
            .and_then(|s| s.as_str())
            .unwrap_or_default();
        assert!(
            !stdout.contains(&key) && !stdout.contains(value),
            "WASM shell_exec child must not inherit parent secrets; got stdout:\n{stdout}"
        );
        // And PATH (on the safe allowlist) should still be present so
        // legitimate shell invocations keep working.
        assert!(
            stdout.contains("PATH="),
            "WASM shell_exec child must still see PATH; got stdout:\n{stdout}"
        );
    }

    #[tokio::test]
    async fn test_env_read_denied() {
        let state = test_state(vec![]);
        let result = host_env_read(&state, &json!({"name": "HOME"}));
        let err = result["error"].as_str().unwrap();
        assert!(err.contains("denied"));
    }

    #[tokio::test]
    async fn test_env_read_granted() {
        let state = test_state(vec![Capability::EnvRead("PATH".to_string())]);
        let result = host_env_read(&state, &json!({"name": "PATH"}));
        assert!(result.get("ok").is_some(), "Expected ok: {:?}", result);
    }

    #[tokio::test]
    async fn test_kv_get_no_kernel() {
        let state = test_state(vec![Capability::MemoryRead("*".to_string())]);
        let result = host_kv_get(&state, &json!({"key": "test"}));
        let err = result["error"].as_str().unwrap();
        assert!(err.contains("kernel"));
    }

    #[tokio::test]
    async fn test_agent_send_denied() {
        let state = test_state(vec![]);
        let result = host_agent_send(&state, &json!({"target": "some-agent", "message": "hello"}));
        let err = result["error"].as_str().unwrap();
        assert!(err.contains("denied"));
    }

    #[tokio::test]
    async fn test_agent_spawn_denied() {
        let state = test_state(vec![]);
        let result = host_agent_spawn(&state, &json!({"manifest": "name = 'test'"}));
        let err = result["error"].as_str().unwrap();
        assert!(err.contains("denied"));
    }

    #[tokio::test]
    async fn test_dispatch_unknown_method() {
        let state = test_state(vec![]);
        let result = dispatch(&state, "bogus_method", &json!({}));
        let err = result["error"].as_str().unwrap();
        assert!(err.contains("Unknown"));
    }

    #[tokio::test]
    async fn test_missing_params() {
        let state = test_state(vec![Capability::FileRead("*".to_string())]);
        let result = host_fs_read(&state, &json!({}));
        let err = result["error"].as_str().unwrap();
        assert!(err.contains("Missing"));
    }

    #[test]
    fn test_safe_resolve_path_traversal() {
        assert!(safe_resolve_path("../etc/passwd").is_err());
        assert!(safe_resolve_path("/tmp/../../etc/passwd").is_err());
        assert!(safe_resolve_path("foo/../bar").is_err());
    }

    /// Regression for #3814: capability check must run AFTER canonicalization.
    ///
    /// A capability granting access to `/tmp/allowed` must NOT permit a guest
    /// that passes `../allowed` when the working directory is `/tmp/sub` —
    /// the raw string does not literally match `/tmp/allowed`, but
    /// canonicalization would resolve it to the same inode. Conversely, a
    /// traversal attempt aimed at a path outside any granted capability must
    /// be denied even if the raw string superficially matches a prefix.
    ///
    /// We test the deny path here: a FileRead("*") wildcard is granted but
    /// the path `../etc/passwd` is rejected during canonicalization (contains
    /// `..`), so the capability check is never even reached. This validates
    /// that canonicalization is the first gate.
    #[tokio::test]
    async fn test_fs_read_traversal_rejected_before_capability_check() {
        // Even with a wildcard capability, ".." in the path must be caught by
        // safe_resolve_path before we ever consult the capability list.
        let state = test_state(vec![Capability::FileRead("*".to_string())]);
        let result = host_fs_read(&state, &json!({"path": "../etc/passwd"}));
        let err = result["error"].as_str().unwrap_or("");
        assert!(
            err.contains("traversal") || err.contains("resolve") || err.contains("Cannot"),
            "Expected path-traversal or resolution error, got: {err}"
        );
    }

    #[test]
    fn test_safe_resolve_parent_traversal() {
        assert!(safe_resolve_parent("../malicious.txt").is_err());
        assert!(safe_resolve_parent("/tmp/../../etc/shadow").is_err());
    }

    #[test]
    fn test_ssrf_private_ips_blocked() {
        assert!(is_ssrf_target("http://127.0.0.1:8080/secret").is_err());
        assert!(is_ssrf_target("http://localhost:3000/api").is_err());
        assert!(is_ssrf_target("http://169.254.169.254/metadata").is_err());
        assert!(is_ssrf_target("http://metadata.google.internal/v1/instance").is_err());
    }

    #[test]
    fn test_ssrf_public_ips_allowed() {
        assert!(is_ssrf_target("https://api.openai.com/v1/chat").is_ok());
        assert!(is_ssrf_target("https://google.com").is_ok());
    }

    #[test]
    fn test_ssrf_scheme_validation() {
        assert!(is_ssrf_target("file:///etc/passwd").is_err());
        assert!(is_ssrf_target("gopher://evil.com").is_err());
        assert!(is_ssrf_target("ftp://example.com").is_err());
    }

    #[test]
    fn test_is_private_ip() {
        use std::net::IpAddr;
        assert!(is_private_ip(&"10.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(&"172.16.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(&"192.168.1.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(&"169.254.169.254".parse::<IpAddr>().unwrap()));
        assert!(!is_private_ip(&"8.8.8.8".parse::<IpAddr>().unwrap()));
        assert!(!is_private_ip(&"1.1.1.1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn test_is_private_ip_recognises_ipv4_mapped_v6() {
        use std::net::IpAddr;
        // IPv4-mapped IPv6 (::ffff:X.X.X.X) must be canonicalised to its
        // IPv4 form so the private-range checks actually fire. Without
        // canonicalisation, the V6 branch only catches fc00::/7 + fe80::/10
        // and leaves ::ffff:10.0.0.1, ::ffff:169.254.169.254 etc. as
        // "public" — the exact bypass fixed in web_fetch.rs.
        assert!(is_private_ip(&"::ffff:10.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(
            &"::ffff:169.254.169.254".parse::<IpAddr>().unwrap()
        ));
        assert!(is_private_ip(
            &"::ffff:192.168.1.1".parse::<IpAddr>().unwrap()
        ));
        // Real IPv6 still takes the V6 branch.
        assert!(!is_private_ip(&"2001:db8::1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn test_extract_host_from_url() {
        assert_eq!(
            extract_host_from_url("https://api.openai.com/v1/chat"),
            "api.openai.com:443"
        );
        assert_eq!(
            extract_host_from_url("http://localhost:8080/api"),
            "localhost:8080"
        );
        assert_eq!(
            extract_host_from_url("http://example.com"),
            "example.com:80"
        );
    }
}
