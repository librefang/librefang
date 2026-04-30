//! Host function implementations for the WASM sandbox.
//!
//! Each function checks capabilities before executing. Deny-by-default:
//! if no matching capability is found, the operation is rejected.
//!
//! These functions are called from the `host_call` dispatch in `sandbox.rs`.
//! They receive `&GuestState` (not `&mut`) and return JSON values.

use crate::sandbox::{GuestState, MAX_GUEST_RESULT_BYTES};
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
    // macOS aliases /tmp → /private/tmp, /var → /private/var, /etc → /private/etc
    // at the firmlink layer, so safe_resolve_path's canonicalize() always
    // pushes guest-supplied paths under /private/. Strip the prefix so
    // operator grants written against the user-facing path (/tmp/*, /var/log/*, …)
    // match the canonicalised value.
    if cfg!(target_os = "macos") {
        let aliased = match required {
            Capability::FileRead(p) => p
                .strip_prefix("/private/")
                .map(|rest| Capability::FileRead(format!("/{rest}"))),
            Capability::FileWrite(p) => p
                .strip_prefix("/private/")
                .map(|rest| Capability::FileWrite(format!("/{rest}"))),
            _ => None,
        };
        if let Some(aliased) = aliased {
            for granted in capabilities {
                if capability_matches(granted, &aliased) {
                    return Ok(());
                }
            }
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
#[derive(Debug)]
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

    // Reject userinfo (@) in authority — prevents SSRF bypass via host confusion (#3527).
    if let Some(after_scheme) = url.split_once("://").map(|(_, rest)| rest) {
        let authority_end = after_scheme
            .find(['/', '?', '#'])
            .unwrap_or(after_scheme.len());
        if after_scheme[..authority_end].contains('@') {
            return Err(json!({"error": "SSRF blocked: URLs with userinfo are not permitted"}));
        }
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
    // SECURITY: refuse to follow a leaf symlink.
    //
    // safe_resolve_parent canonicalises the *parent* directory and appends
    // file_name verbatim, so the capability check sees a path inside the
    // grant — but if the leaf itself is a symlink that points out of the
    // grant (e.g. attacker pre-stages /grant/dir/sym -> /etc/passwd) then
    // std::fs::write follows it and clobbers the real target.  The audit
    // of #3925 flagged this as a HIGH symlink bypass.
    //
    // Cross-platform approach: refuse the write if the leaf is a symlink
    // (symlink_metadata does not follow).  On Unix we additionally pass
    // O_NOFOLLOW so the kernel rejects the open atomically — closes the
    // narrow TOCTOU window between the lstat and the open.  Linux's
    // O_NOFOLLOW value is 0o400000; we hard-code rather than pull in
    // libc since this crate doesn't already depend on it.
    if let Ok(meta) = std::fs::symlink_metadata(&write_path) {
        if meta.file_type().is_symlink() {
            return json!({
                "error": "fs_write denied: refusing to follow a symlink leaf"
            });
        }
    }
    use std::io::Write;
    let mut open_opts = std::fs::OpenOptions::new();
    open_opts.write(true).truncate(true).create(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Linux O_NOFOLLOW. The BSD family uses a different value
        // (0x0100) and gets its own block below.
        open_opts.custom_flags(0o0_400_000);
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
    ))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // BSD-family O_NOFOLLOW = 0x0100. Closes the same TOCTOU
        // window the Linux block above closes (between lstat and open),
        // so an attacker who can write to the parent dir can't race a
        // regular file → symlink swap.
        open_opts.custom_flags(0x0100);
    }
    let mut f = match open_opts.open(&write_path) {
        Ok(f) => f,
        Err(e) => {
            // ELOOP (40) is the kernel rejecting O_NOFOLLOW on Linux;
            // surface it as a deny rather than a generic open failure.
            #[cfg(target_os = "linux")]
            if e.raw_os_error() == Some(40) {
                return json!({
                    "error": "fs_write denied: refusing to follow a symlink leaf"
                });
            }
            return json!({"error": format!("fs_write failed: {e}")});
        }
    };
    if let Err(e) = f.write_all(content.as_bytes()) {
        return json!({"error": format!("fs_write failed: {e}")});
    }
    json!({"ok": true})
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

    // SECURITY: Use block_in_place instead of block_on so the tokio scheduler
    // can continue making progress (including the epoch-increment watchdog)
    // while this thread is parked waiting for the async HTTP call to complete.
    // block_on inside spawn_blocking creates a nested runtime and bypasses the
    // epoch watchdog, allowing a WASM guest to stall the host indefinitely.
    let handle = state.tokio_handle.clone();
    tokio::task::block_in_place(|| {
        handle.block_on(async {
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
fn sanitize_shell_env(cmd: &mut tokio::process::Command) {
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

/// Wall-clock timeout per `shell_exec` invocation (#3529).
const SHELL_EXEC_TIMEOUT_SECS: u64 = 30;
/// Per-stream stdout/stderr byte cap; child is killed on overflow (#3529).
const SHELL_EXEC_MAX_OUTPUT_BYTES: usize = 1024 * 1024;

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

    let args: Vec<String> = params
        .get("args")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let command = command.to_string();
    let handle = state.tokio_handle.clone();

    // block_in_place to bridge sync WASM host call → async tokio::process (same pattern as host_net_fetch).
    tokio::task::block_in_place(|| {
        handle.block_on(async move { run_shell_exec(&command, &args).await })
    })
}

async fn run_shell_exec(command: &str, args: &[String]) -> serde_json::Value {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;

    // Command::new does NOT use a shell — safe from shell injection.
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args);
    sanitize_shell_env(&mut cmd);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW; tokio Command exposes this directly on Windows.
        cmd.creation_flags(0x0800_0000);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return json!({"error": format!("shell_exec failed to spawn: {e}")}),
    };
    let mut stdout_pipe = child.stdout.take().expect("stdout piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");

    let timeout = std::time::Duration::from_secs(SHELL_EXEC_TIMEOUT_SECS);
    let exec = async {
        // select! loop drains both pipes concurrently. Breaking as soon as
        // either hits the cap lets us kill the child immediately, which causes
        // the other pipe to see EOF instead of hanging indefinitely (#3529).
        let cap = SHELL_EXEC_MAX_OUTPUT_BYTES;
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        let mut truncated = false;
        let mut out_done = false;
        let mut err_done = false;
        while !out_done || !err_done {
            let mut out_chunk = [0u8; 8192];
            let mut err_chunk = [0u8; 8192];
            tokio::select! {
                n = stdout_pipe.read(&mut out_chunk), if !out_done => match n? {
                    0 => out_done = true,
                    n => {
                        let take = cap.saturating_sub(stdout_buf.len()).min(n);
                        stdout_buf.extend_from_slice(&out_chunk[..take]);
                        if stdout_buf.len() >= cap { truncated = true; break; }
                    }
                },
                n = stderr_pipe.read(&mut err_chunk), if !err_done => match n? {
                    0 => err_done = true,
                    n => {
                        let take = cap.saturating_sub(stderr_buf.len()).min(n);
                        stderr_buf.extend_from_slice(&err_chunk[..take]);
                        if stderr_buf.len() >= cap { truncated = true; break; }
                    }
                },
            }
        }
        if truncated {
            let _ = child.start_kill();
        }
        let status = child.wait().await?;
        Ok::<_, std::io::Error>((status, stdout_buf, stderr_buf, truncated))
    };

    match tokio::time::timeout(timeout, exec).await {
        Ok(Ok((status, stdout_bytes, stderr_bytes, truncated))) => {
            if truncated {
                return json!({"error": format!(
                    "shell_exec output exceeded {SHELL_EXEC_MAX_OUTPUT_BYTES} bytes; child killed"
                )});
            }
            let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
            let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();
            json!({
                "ok": {
                    "exit_code": status.code(),
                    "stdout": stdout,
                    "stderr": stderr,
                }
            })
        }
        Ok(Err(e)) => json!({"error": format!("shell_exec failed: {e}")}),
        Err(_) => {
            // child + pipes drop here → kill_on_drop reaps the subprocess.
            json!({"error": format!(
                "shell_exec timed out after {SHELL_EXEC_TIMEOUT_SECS}s; child killed"
            )})
        }
    }
}

// ---------------------------------------------------------------------------
// Environment (capability-checked)
// ---------------------------------------------------------------------------

/// Hard-coded blocklist of env var name substrings that WASM plugins can
/// NEVER read, regardless of their declared `EnvRead` capability.
///
/// The check is case-insensitive. Any variable whose upper-cased name
/// contains one of these substrings is silently suppressed — the caller
/// receives `null` rather than the real value, and no error is returned so
/// that well-behaved plugins can't probe for the existence of secrets.
const BLOCKED_ENV_SUBSTRINGS: &[&str] = &[
    "KEY",
    "SECRET",
    "TOKEN",
    "PASSWORD",
    "CREDENTIAL",
    "PRIVATE",
];

/// Specific full names (upper-cased) that are always blocked regardless of
/// whether they contain a blocked substring. This catches names that are
/// conventional secrets but do not contain any of the substrings above.
const BLOCKED_ENV_EXACT: &[&str] = &[
    "LIBREFANG_VAULT_KEY",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "GROQ_API_KEY",
    "GEMINI_API_KEY",
    "GITHUB_TOKEN",
    "NPM_TOKEN",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
];

/// Returns `true` if the env var name matches the blocklist and must not be
/// returned to a WASM guest.
fn is_blocked_env_var(name: &str) -> bool {
    let upper = name.to_uppercase();
    // Exact-name check (belt-and-suspenders — all of these also match the
    // boundary check below, but an explicit list is easier to audit).
    if BLOCKED_ENV_EXACT.contains(&upper.as_str()) {
        return true;
    }
    // Word-boundary substring check.  Plain `contains` flagged
    // `MONKEYHOUSE`, `KEYBOARD_LAYOUT`, `TOKENIZER_OPTS`,
    // `PRIVATELABEL_NAME` and similar non-secret config vars, leaving
    // `EnvRead("*")` plugins unable to read benign settings.  Require a
    // non-alphanumeric boundary on at least one side of the match
    // (start/end of string counts), so e.g. `AWS_API_KEY` and
    // `MY_PASSWORD_HASH` still match while `MONKEYHOUSE` does not.
    BLOCKED_ENV_SUBSTRINGS
        .iter()
        .any(|sub| has_word_boundary_substring(&upper, sub))
}

/// `true` iff `needle` appears in `haystack` as its own word —
/// i.e. with a non-alphanumeric boundary (string edge or any char that
/// is not an ASCII letter / digit) on **both** sides.  Env-var
/// convention separates words with `_`; `-` / `.` are also covered for
/// odd names like `MY-API-KEY` or `KEY.PRIVATE`.
///
/// Both-sides matters: a single-side rule would still flag
/// `KEYBOARD_LAYOUT` (left edge = start-of-string is a boundary, but
/// right edge = `'B'` is alphanumeric, so it isn't actually a `KEY`
/// word).  Real secret names always have a boundary on the side
/// closest to the credential token: `OPENAI_API_KEY`, `MY_PASSWORD`,
/// `FOO_TOKEN`, `KEY_FOO` all satisfy both-sides.
fn has_word_boundary_substring(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    let n = needle.len();
    let mut start = 0;
    while let Some(rel) = haystack[start..].find(needle) {
        let idx = start + rel;
        let before_ok = idx == 0 || !bytes[idx - 1].is_ascii_alphanumeric();
        let end = idx + n;
        let after_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        // Advance past this occurrence to find any later one with a
        // boundary.  Stop if we'd loop on a zero-length needle.
        start = idx + n.max(1);
        if start >= bytes.len() {
            break;
        }
    }
    false
}

fn host_env_read(state: &GuestState, params: &serde_json::Value) -> serde_json::Value {
    let name = match params.get("name").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => return json!({"error": "Missing 'name' parameter"}),
    };
    if let Err(e) = check_capability(&state.capabilities, &Capability::EnvRead(name.to_string())) {
        return e;
    }
    // SECURITY: Never expose secrets to WASM guests even when the capability
    // grants wildcard access.  Silently return null so the caller cannot
    // distinguish "blocked" from "variable not set".
    if is_blocked_env_var(name) {
        return json!({"ok": null});
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
    // SECURITY: Prefix the key with the agent_id to give each agent its own
    // isolated KV namespace (Bug #3837). Without this prefix all agents share
    // a flat key space, so agent A can read or overwrite agent B's keys.
    let namespaced_key = format!("{}:{key}", state.agent_id);
    match kernel.memory_recall(&namespaced_key, None) {
        Ok(Some(val)) => {
            // SECURITY (#3866): cap the value returned to the guest so a
            // value stored before this cap existed cannot be used to push
            // an unbounded buffer back through host_call.
            let serialized_len = serde_json::to_vec(&val).map(|v| v.len()).unwrap_or(0);
            if serialized_len > MAX_GUEST_RESULT_BYTES {
                return json!({"error": format!(
                    "Stored value too large to return: {serialized_len} bytes (max {MAX_GUEST_RESULT_BYTES})"
                )});
            }
            json!({"ok": val})
        }
        Ok(None) => json!({"ok": null}),
        Err(e) => json!({"error": e}),
    }
}

/// Maximum key length accepted by `host_kv_set` (Bug #3866).
///
/// Keys are namespaced and stored verbatim in SQLite; an unbounded key
/// lets a guest pump megabytes of bytes into the index per call.
const MAX_KV_KEY_BYTES: usize = 1024;

/// Maximum serialized value size accepted by `host_kv_set` (Bug #3866).
///
/// Aliased from `MAX_GUEST_RESULT_BYTES` so all 1 MiB payload caps stay in
/// sync with a single definition.
const MAX_KV_VALUE_BYTES: usize = MAX_GUEST_RESULT_BYTES;

fn host_kv_set(state: &GuestState, params: &serde_json::Value) -> serde_json::Value {
    let key = match params.get("key").and_then(|k| k.as_str()) {
        Some(k) => k,
        None => return json!({"error": "Missing 'key' parameter"}),
    };
    // SECURITY (#3866): cap key length before storing.
    if key.len() > MAX_KV_KEY_BYTES {
        return json!({"error": format!(
            "Key too large: {} bytes (max {MAX_KV_KEY_BYTES})",
            key.len()
        )});
    }
    // Capability check before deserialising/cloning the value so a guest
    // without MemoryWrite cannot force the host to serialize a 1 MiB blob.
    if let Err(e) = check_capability(
        &state.capabilities,
        &Capability::MemoryWrite(key.to_string()),
    ) {
        return e;
    }
    let value_ref = match params.get("value") {
        Some(v) => v,
        None => return json!({"error": "Missing 'value' parameter"}),
    };
    // SECURITY (#3866): cap serialized value size to bound SQLite growth.
    // Serialize via the borrow first so we don't clone a potentially large
    // value before knowing it passes the cap. Fail-closed: a serialization
    // error rejects the call rather than silently skipping the size guard.
    let serialized_len = match serde_json::to_vec(value_ref) {
        Ok(v) => v.len(),
        Err(e) => return json!({"error": format!("Failed to serialize value: {e}")}),
    };
    if serialized_len > MAX_KV_VALUE_BYTES {
        return json!({"error": format!(
            "Value too large: {serialized_len} bytes (max {MAX_KV_VALUE_BYTES})"
        )});
    }
    let value = value_ref.clone();
    let kernel = match &state.kernel {
        Some(k) => k,
        None => return json!({"error": "No kernel handle available"}),
    };
    // SECURITY: Prefix the key with the agent_id so each agent's KV entries
    // live in a separate namespace and cannot be accessed by other agents
    // (Bug #3837).
    let namespaced_key = format!("{}:{key}", state.agent_id);
    match kernel.memory_store(&namespaced_key, value, None) {
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
    // SECURITY: Use block_in_place so the epoch watchdog can make progress
    // while the host-to-agent round-trip is in flight. Plain block_on inside
    // spawn_blocking creates a nested runtime that starves the watchdog.
    let handle = state.tokio_handle.clone();
    match tokio::task::block_in_place(|| handle.block_on(kernel.send_to_agent(target, message))) {
        Ok(response) => json!({"ok": response}),
        Err(e) => json!({"error": e}),
    }
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
    // SECURITY: Enforce capability inheritance — child <= parent.
    // Use block_in_place so the epoch watchdog can make progress during the
    // synchronous wait; plain block_on inside spawn_blocking bypasses it.
    let handle = state.tokio_handle.clone();
    match tokio::task::block_in_place(|| {
        handle.block_on(kernel.spawn_agent_checked(
            manifest_toml,
            Some(&state.agent_id),
            &state.capabilities,
        ))
    }) {
        Ok((id, name)) => json!({"ok": {"id": id, "name": name}}),
        Err(e) => json!({"error": e}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state(capabilities: Vec<Capability>) -> GuestState {
        GuestState::for_test(
            capabilities,
            None,
            "test-agent".to_string(),
            tokio::runtime::Handle::current(),
        )
    }

    /// Word-boundary blocklist: real secret-shaped names match, benign
    /// names that merely embed the substring don't.
    #[test]
    fn test_is_blocked_env_var_word_boundary() {
        // Real secrets — must block.
        for name in &[
            "OPENAI_API_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "GITHUB_TOKEN",
            "MY_PASSWORD",
            "DB_PRIVATE_KEY",
            "API_CREDENTIAL_FILE",
            // Standalone tokens at string start/end.
            "KEY",
            "SECRET_FOO",
            "FOO_TOKEN",
        ] {
            assert!(
                is_blocked_env_var(name),
                "{name} must be blocked (real secret)"
            );
        }

        // Benign config — must NOT block.  These were all false positives
        // under the old plain `contains` check.
        for name in &[
            "MONKEYHOUSE",
            "KEYBOARD_LAYOUT",
            "TOKENIZER_OPTS",
            "PRIVATELABEL_NAME",
            "PASSWORDLIST_FILE",
            "MASTERKEYBOARD",
        ] {
            assert!(
                !is_blocked_env_var(name),
                "{name} must NOT be blocked (benign config)"
            );
        }

        // Boundary punctuation other than `_` is also a boundary.
        assert!(is_blocked_env_var("MY-API-KEY"));
        assert!(is_blocked_env_var("KEY.PRIVATE"));
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
        // The capability gate runs *after* canonicalize, so the test path must
        // exist or canonicalize fails first with "Cannot resolve path" on
        // Windows (os error 3) and the assertion never sees the deny error.
        // Cargo.toml is guaranteed to exist in every crate dir during tests.
        let state = test_state(vec![]);
        let result = host_fs_read(&state, &json!({"path": "Cargo.toml"}));
        let err = result["error"].as_str().unwrap();
        assert!(err.contains("denied"), "expected denied, got: {err}");
    }

    #[tokio::test]
    async fn test_fs_write_denied_no_capability() {
        // host_fs_write canonicalizes the *parent*, so the parent must exist.
        // std::env::temp_dir() exists on every supported platform.
        let state = test_state(vec![]);
        let target = std::env::temp_dir().join("librefang_wasm_test_denied.txt");
        let target_str = target.to_string_lossy().to_string();
        let result = host_fs_write(&state, &json!({"path": target_str, "content": "hello"}));
        let err = result["error"].as_str().unwrap();
        assert!(err.contains("denied"), "expected denied, got: {err}");
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
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
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

    /// Regression for #3529: a runaway child must be killed once it
    /// exceeds the per-stream output cap. `yes` floods stdout indefinitely;
    /// without the cap + kill_on_drop the host would happily fill memory.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_shell_exec_kills_child_on_output_cap() {
        let state = test_state(vec![Capability::ShellExec("/usr/bin/yes".to_string())]);
        let started = std::time::Instant::now();
        let result = host_shell_exec(
            &state,
            &json!({
                "command": "/usr/bin/yes",
                "args": [],
            }),
        );
        let elapsed = started.elapsed();
        let err = result["error"].as_str().expect("expected output-cap error");
        assert!(
            err.contains("output exceeded"),
            "expected output-cap kill, got: {err}"
        );
        // Must have aborted well before the 30s timeout fires.
        assert!(
            elapsed < std::time::Duration::from_secs(15),
            "child not killed promptly; elapsed = {elapsed:?}"
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

    /// Regression: #3362 — a WASM guest with `EnvRead("*")` must not be able
    /// to read secrets even with a wildcard capability.  The blocklist must
    /// suppress any variable whose name contains KEY, SECRET, TOKEN, PASSWORD,
    /// CREDENTIAL, or PRIVATE (case-insensitive).
    #[tokio::test]
    async fn test_env_read_blocklist_suppresses_secrets() {
        let state = test_state(vec![Capability::EnvRead("*".to_string())]);

        let blocked_names = [
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "GROQ_API_KEY",
            "GEMINI_API_KEY",
            "LIBREFANG_VAULT_KEY",
            "GITHUB_TOKEN",
            "NPM_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
            "MY_CUSTOM_SECRET",
            "DATABASE_PASSWORD",
            "DEPLOY_CREDENTIAL",
            "RSA_PRIVATE_KEY",
            // Lower-case variants must also be blocked.
            "my_api_key",
            "db_password",
        ];

        for var_name in &blocked_names {
            // Stamp a known value so we'd catch any leak.
            std::env::set_var(var_name, "should-not-leak");
            let result = host_env_read(&state, &json!({"name": var_name}));
            // Must NOT return an error (no capability denial) — but value must be null.
            assert!(
                result.get("error").is_none(),
                "Blocklist should not return capability error for {var_name}: {result:?}"
            );
            let val = result.get("ok");
            assert!(
                val.is_some() && val.unwrap().is_null(),
                "Blocked var {var_name} must return null, got: {result:?}"
            );
            std::env::remove_var(var_name);
        }
    }

    #[test]
    fn test_is_blocked_env_var() {
        // Exact names
        assert!(is_blocked_env_var("ANTHROPIC_API_KEY"));
        assert!(is_blocked_env_var("LIBREFANG_VAULT_KEY"));
        assert!(is_blocked_env_var("AWS_SESSION_TOKEN"));
        // Substring matches
        assert!(is_blocked_env_var("MY_SECRET_THING"));
        assert!(is_blocked_env_var("DB_PASSWORD"));
        assert!(is_blocked_env_var("DEPLOY_TOKEN"));
        assert!(is_blocked_env_var("PRIVATE_KEY_DATA"));
        // Case-insensitive
        assert!(is_blocked_env_var("my_api_key"));
        assert!(is_blocked_env_var("db_password"));
        // Safe vars must NOT be blocked
        assert!(!is_blocked_env_var("PATH"));
        assert!(!is_blocked_env_var("HOME"));
        assert!(!is_blocked_env_var("LANG"));
        assert!(!is_blocked_env_var("TERM"));
        assert!(!is_blocked_env_var("USER"));
    }

    #[tokio::test]
    async fn test_kv_get_no_kernel() {
        let state = test_state(vec![Capability::MemoryRead("*".to_string())]);
        let result = host_kv_get(&state, &json!({"key": "test"}));
        let err = result["error"].as_str().unwrap();
        assert!(err.contains("kernel"));
    }

    // ---------------------------------------------------------------------------
    // Mock KernelHandle for KV namespace isolation tests (Bug #3837)
    // ---------------------------------------------------------------------------

    struct RecordingKernel {
        /// Records every (namespaced_key, value) passed to memory_store.
        stored: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
        /// Records every namespaced_key passed to memory_recall.
        recalled: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingKernel {
        fn new() -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
                stored: std::sync::Mutex::new(Vec::new()),
                recalled: std::sync::Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait::async_trait]
    impl librefang_kernel_handle::KernelHandle for RecordingKernel {
        async fn spawn_agent(&self, _: &str, _: Option<&str>) -> Result<(String, String), String> {
            Err("not implemented".to_string())
        }
        async fn send_to_agent(&self, _: &str, _: &str) -> Result<String, String> {
            Err("not implemented".to_string())
        }
        fn list_agents(&self) -> Vec<librefang_kernel_handle::AgentInfo> {
            vec![]
        }
        fn kill_agent(&self, _: &str) -> Result<(), String> {
            Err("not implemented".to_string())
        }
        fn memory_store(
            &self,
            key: &str,
            value: serde_json::Value,
            _peer_id: Option<&str>,
        ) -> Result<(), String> {
            self.stored.lock().unwrap().push((key.to_string(), value));
            Ok(())
        }
        fn memory_recall(
            &self,
            key: &str,
            _peer_id: Option<&str>,
        ) -> Result<Option<serde_json::Value>, String> {
            self.recalled.lock().unwrap().push(key.to_string());
            Ok(None)
        }
        fn memory_list(&self, _: Option<&str>) -> Result<Vec<String>, String> {
            Ok(vec![])
        }
        fn find_agents(&self, _: &str) -> Vec<librefang_kernel_handle::AgentInfo> {
            vec![]
        }
        async fn task_post(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: Option<&str>,
        ) -> Result<String, String> {
            Err("not implemented".to_string())
        }
        async fn task_claim(&self, _: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(None)
        }
        async fn task_complete(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
            Err("not implemented".to_string())
        }
        async fn task_list(&self, _: Option<&str>) -> Result<Vec<serde_json::Value>, String> {
            Ok(vec![])
        }
        async fn task_delete(&self, _: &str) -> Result<bool, String> {
            Ok(false)
        }
        async fn task_retry(&self, _: &str) -> Result<bool, String> {
            Ok(false)
        }
        async fn task_get(&self, _: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(None)
        }
        async fn task_update_status(&self, _: &str, _: &str) -> Result<bool, String> {
            Ok(false)
        }
        async fn publish_event(&self, _: &str, _: serde_json::Value) -> Result<(), String> {
            Ok(())
        }
        async fn knowledge_add_entity(
            &self,
            _: librefang_types::memory::Entity,
        ) -> Result<String, String> {
            Err("not implemented".to_string())
        }
        async fn knowledge_add_relation(
            &self,
            _: librefang_types::memory::Relation,
        ) -> Result<String, String> {
            Err("not implemented".to_string())
        }
        async fn knowledge_query(
            &self,
            _: librefang_types::memory::GraphPattern,
        ) -> Result<Vec<librefang_types::memory::GraphMatch>, String> {
            Ok(vec![])
        }
    }

    fn state_with_kernel(
        agent_id: &str,
        capabilities: Vec<Capability>,
        kernel: std::sync::Arc<RecordingKernel>,
    ) -> GuestState {
        GuestState::for_test(
            capabilities,
            Some(kernel),
            agent_id.to_string(),
            tokio::runtime::Handle::current(),
        )
    }

    /// Regression test for Bug #3837: kv_get must namespace the key with
    /// agent_id so that two agents cannot read each other's KV entries.
    #[tokio::test]
    async fn test_kv_get_key_is_namespaced_with_agent_id() {
        let kernel = RecordingKernel::new();
        let state = state_with_kernel(
            "agent-alice",
            vec![Capability::MemoryRead("*".to_string())],
            std::sync::Arc::clone(&kernel),
        );
        host_kv_get(&state, &json!({"key": "secret"}));

        let recalled = kernel.recalled.lock().unwrap();
        assert_eq!(recalled.len(), 1, "Expected exactly one memory_recall call");
        assert_eq!(
            recalled[0], "agent-alice:secret",
            "kv_get must prefix the key with agent_id to isolate namespaces"
        );
    }

    /// Regression test for Bug #3837: kv_set must namespace the key with
    /// agent_id so that two agents cannot overwrite each other's KV entries.
    #[tokio::test]
    async fn test_kv_set_key_is_namespaced_with_agent_id() {
        let kernel = RecordingKernel::new();
        let state = state_with_kernel(
            "agent-bob",
            vec![Capability::MemoryWrite("*".to_string())],
            std::sync::Arc::clone(&kernel),
        );
        host_kv_set(&state, &json!({"key": "counter", "value": 42}));

        let stored = kernel.stored.lock().unwrap();
        assert_eq!(stored.len(), 1, "Expected exactly one memory_store call");
        assert_eq!(
            stored[0].0, "agent-bob:counter",
            "kv_set must prefix the key with agent_id to isolate namespaces"
        );
    }

    /// Regression test for Bug #3866: host_kv_set must reject value
    /// payloads larger than `MAX_KV_VALUE_BYTES` (1 MiB) before persisting.
    #[tokio::test]
    async fn test_kv_set_rejects_oversized_value() {
        let kernel = RecordingKernel::new();
        let state = state_with_kernel(
            "agent-bob",
            vec![Capability::MemoryWrite("*".to_string())],
            std::sync::Arc::clone(&kernel),
        );
        // Build a value whose JSON serialization exceeds the 1 MiB cap.
        let huge = "x".repeat(MAX_KV_VALUE_BYTES + 16);
        let result = host_kv_set(&state, &json!({"key": "k", "value": huge}));
        let err = result["error"].as_str().unwrap_or("");
        assert!(
            err.contains("too large"),
            "Expected 'too large' rejection, got: {err}"
        );
        // Crucially, the kernel must NOT have stored anything.
        let stored = kernel.stored.lock().unwrap();
        assert_eq!(
            stored.len(),
            0,
            "Oversized value must not reach memory_store"
        );
    }

    /// Regression test for Bug #3866: host_kv_set must reject keys longer
    /// than `MAX_KV_KEY_BYTES` (1024 bytes) before persisting.
    #[tokio::test]
    async fn test_kv_set_rejects_oversized_key() {
        let kernel = RecordingKernel::new();
        let state = state_with_kernel(
            "agent-bob",
            vec![Capability::MemoryWrite("*".to_string())],
            std::sync::Arc::clone(&kernel),
        );
        let long_key = "k".repeat(MAX_KV_KEY_BYTES + 1);
        let result = host_kv_set(&state, &json!({"key": long_key, "value": 1}));
        let err = result["error"].as_str().unwrap_or("");
        assert!(
            err.contains("Key too large"),
            "Expected 'Key too large' rejection, got: {err}"
        );
        let stored = kernel.stored.lock().unwrap();
        assert_eq!(stored.len(), 0, "Oversized key must not reach memory_store");
    }

    /// Two agents using the same guest key must produce different namespaced
    /// keys — that is the whole point of the namespace isolation.
    #[tokio::test]
    async fn test_kv_two_agents_same_guest_key_different_namespaced_keys() {
        let kernel_a = RecordingKernel::new();
        let state_a = state_with_kernel(
            "agent-alice",
            vec![Capability::MemoryRead("*".to_string())],
            std::sync::Arc::clone(&kernel_a),
        );
        host_kv_get(&state_a, &json!({"key": "shared_name"}));

        let kernel_b = RecordingKernel::new();
        let state_b = state_with_kernel(
            "agent-bob",
            vec![Capability::MemoryRead("*".to_string())],
            std::sync::Arc::clone(&kernel_b),
        );
        host_kv_get(&state_b, &json!({"key": "shared_name"}));

        let recalled_a = kernel_a.recalled.lock().unwrap();
        let recalled_b = kernel_b.recalled.lock().unwrap();

        assert_ne!(
            recalled_a[0], recalled_b[0],
            "Different agents must produce different namespaced keys for the same guest key"
        );
        assert!(
            recalled_a[0].starts_with("agent-alice:"),
            "Alice's key must be prefixed with her agent_id"
        );
        assert!(
            recalled_b[0].starts_with("agent-bob:"),
            "Bob's key must be prefixed with his agent_id"
        );
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

    /// Regression for #3527: reject userinfo (@) in authority to prevent SSRF bypass.
    #[test]
    fn test_ssrf_rejects_urls_with_userinfo() {
        // Bare userinfo, attacker-controlled real host
        let r = is_ssrf_target("http://x@169.254.169.254/");
        assert!(r.is_err(), "must reject userinfo URLs");
        let err = r.unwrap_err()["error"].as_str().unwrap_or("").to_string();
        assert!(err.contains("userinfo"), "wrong error: {err}");

        // user:pass form
        assert!(is_ssrf_target("http://user:pass@127.0.0.1/").is_err());

        // The exact bypass shape: parser-confusing host:port@evil
        assert!(is_ssrf_target("http://allowed.com:80@169.254.169.254/").is_err());
        assert!(is_ssrf_target("https://allowed.com@127.0.0.1:9000/x").is_err());

        // Userinfo with empty password is still userinfo
        assert!(is_ssrf_target("http://user:@8.8.8.8/").is_err());

        // `@` later in the path is fine
        assert!(is_ssrf_target("https://api.openai.com/v1/users/me@example").is_ok());

        // `@` in query string is fine
        assert!(is_ssrf_target("https://api.openai.com/?email=me@example.com").is_ok());
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

    /// Regression for #3814: capability check must use the canonical path,
    /// not the raw path supplied by the guest. A traversal path like
    /// `../../etc/passwd` must be rejected by path resolution *before* any
    /// capability comparison can be made — it must never reach the file read.
    #[tokio::test]
    async fn test_fs_read_traversal_rejected_before_capability_check() {
        // Even with a wildcard FileRead grant, traversal paths are rejected.
        let state = test_state(vec![Capability::FileRead("*".to_string())]);
        let result = host_fs_read(&state, &json!({"path": "../../etc/passwd"}));
        let err = result["error"].as_str().unwrap();
        assert!(
            err.contains("traversal") || err.contains("forbidden"),
            "traversal path must be rejected; got: {err}"
        );
    }

    /// Regression for #3814: same for fs_write.
    #[tokio::test]
    async fn test_fs_write_traversal_rejected_before_capability_check() {
        let state = test_state(vec![Capability::FileWrite("*".to_string())]);
        let result = host_fs_write(
            &state,
            &json!({"path": "../../tmp/evil.txt", "content": "x"}),
        );
        let err = result["error"].as_str().unwrap();
        assert!(
            err.contains("traversal") || err.contains("forbidden"),
            "traversal path must be rejected; got: {err}"
        );
    }
}
