//! Docker container sandbox — OS-level isolation for agent code execution.
//!
//! Provides secure command execution inside Docker containers with strict
//! resource limits, network isolation, and capability dropping.

use librefang_types::config::DockerSandboxConfig;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;
use tracing::{debug, error, warn};

/// SECURITY: Allowlist of Linux capabilities considered safe to grant back
/// after `--cap-drop ALL`. Derived from Docker's default capability set, with
/// the most dangerous defaults trimmed:
///
/// - `NET_RAW` is retained (ping / traceroute need it) but documented as a
///   minor SSRF amplifier; the network-namespace boundary is the real
///   protection here.
///
/// Excluded by design (each is a sandbox-collapse vector):
/// - `SYS_ADMIN` — near-root: mount, kexec, BPF, namespace manipulation.
/// - `NET_ADMIN` — reconfigure interfaces, firewall rules, raw sockets to
///   arbitrary protocols.
/// - `SYS_PTRACE` — attach to other processes in the namespace; trivially
///   defeats `no-new-privileges` for any other root-mapped UID.
/// - `SYS_MODULE`, `SYS_BOOT`, `SYS_RAWIO`, `SYS_TIME`, `SYS_NICE`,
///   `SYS_RESOURCE`, `SYS_PACCT`, `SYS_TTY_CONFIG`,
///   `LEASE`, `LINUX_IMMUTABLE`, `MAC_ADMIN`, `MAC_OVERRIDE`,
///   `IPC_LOCK`, `IPC_OWNER`, `BLOCK_SUSPEND`, `WAKE_ALARM`,
///   `BPF`, `PERFMON`, `CHECKPOINT_RESTORE`, `AUDIT_CONTROL`,
///   `AUDIT_READ`, `SYSLOG`.
const SAFE_CAPS: &[&str] = &[
    "CHOWN",
    "DAC_OVERRIDE",
    "FOWNER",
    "FSETID",
    "KILL",
    "SETGID",
    "SETUID",
    "SETPCAP",
    "NET_BIND_SERVICE",
    "NET_RAW",
    "SYS_CHROOT",
    "MKNOD",
    "AUDIT_WRITE",
    "SETFCAP",
];

/// SECURITY: Validate the Docker `--network` argument.
///
/// Rejects:
/// - `host` — shares the host network namespace; container can reach
///   `127.0.0.1`, cloud-metadata (`169.254.169.254`), and the daemon's
///   listener on port 4545.
/// - `container:<name>` — joins another container's network namespace,
///   inheriting whatever that container can reach (including `host`
///   transitively).
/// - Anything outside `[A-Za-z0-9_-]+`, which Docker's own network-name
///   grammar rejects anyway; we fail-fast with a typed error rather than
///   defer to a `docker run` failure.
fn validate_network(network: &str) -> Result<(), String> {
    if network.is_empty() {
        return Err("Docker network cannot be empty".into());
    }
    let lower = network.to_ascii_lowercase();
    if lower == "host" {
        return Err(
            "Docker network='host' is forbidden: shares host network namespace, \
             exposing loopback, cloud-metadata (169.254.169.254), and the daemon \
             port to the sandbox"
                .into(),
        );
    }
    if lower.starts_with("container:") {
        return Err(format!(
            "Docker network='{network}' (container:* form) is forbidden: \
             inherits the target container's namespace, transitively defeating isolation"
        ));
    }
    // `bridge`, `none`, and user-defined network names: alphanumeric + `_-`.
    if !network
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "Invalid Docker network name: {network} (allowed: [A-Za-z0-9_-]+)"
        ));
    }
    Ok(())
}

/// SECURITY: Validate a single `--cap-add` value against the safe allowlist.
///
/// Capability names are matched case-insensitively against `SAFE_CAPS` after
/// stripping an optional `CAP_` prefix (Docker accepts both `CHOWN` and
/// `CAP_CHOWN`). Anything not in the allowlist — including syntactically
/// valid but unsafe caps like `SYS_ADMIN` — fails closed with a typed error.
fn validate_capability(cap: &str) -> Result<(), String> {
    if cap.is_empty() {
        return Err("Capability name cannot be empty".into());
    }
    // Character-set check is still useful to reject shell metacharacters
    // before they reach `docker run`.
    if !cap.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(format!(
            "Invalid capability syntax: {cap} (allowed: alphanumeric + underscore)"
        ));
    }
    let upper = cap.to_ascii_uppercase();
    let stripped = upper.strip_prefix("CAP_").unwrap_or(&upper);
    if SAFE_CAPS.contains(&stripped) {
        Ok(())
    } else {
        Err(format!(
            "Capability '{cap}' is not in the safe allowlist. Dangerous capabilities \
             (SYS_ADMIN, NET_ADMIN, SYS_PTRACE, SYS_MODULE, SYS_BOOT, BPF, etc.) collapse \
             the sandbox and are refused at config-load time."
        ))
    }
}

/// SECURITY: Full validation pass for `network` + `cap_add` at config load.
///
/// Fails fast with a typed error AND emits an `error!` log so the daemon
/// startup surface records the rejection even if the caller swallows the
/// `Result`.
pub fn validate_sandbox_config(config: &DockerSandboxConfig) -> Result<(), String> {
    if let Err(e) = validate_network(&config.network) {
        error!(network = %config.network, error = %e, "Docker sandbox network rejected");
        return Err(e);
    }
    for cap in &config.cap_add {
        if let Err(e) = validate_capability(cap) {
            error!(cap = %cap, error = %e, "Docker sandbox capability rejected");
            return Err(e);
        }
    }
    Ok(())
}

/// A running sandbox container.
#[derive(Debug, Clone)]
pub struct SandboxContainer {
    pub container_id: String,
    pub agent_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Result of executing a command in the sandbox.
#[derive(Debug, Clone)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// SECURITY: Validate container name — alphanumeric + dash only.
///
/// Historical behaviour replaced disallowed characters with `-`, which is
/// lossy: agent ids `"foo/bar"` and `"foo-bar"` both collapsed to
/// `"foo-bar"`, causing distinct agents to fight over the same Docker
/// container name. The fix is at the call site
/// (`agent_id_container_suffix`), which now derives a bijective
/// SHA-256-hex prefix; this function therefore only validates and rejects
/// names that contain disallowed characters rather than silently mangling
/// them.
fn sanitize_container_name(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("Container name cannot be empty".into());
    }
    if name.len() > 63 {
        return Err("Container name too long (max 63 chars)".into());
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '-') {
        return Err(format!(
            "Invalid container name: {name} (only alphanumeric and '-' allowed)"
        ));
    }
    Ok(name.to_string())
}

/// Derive a collision-resistant, Docker-name-safe suffix from an agent id.
///
/// `SHA-256(agent_id)[..8 hex chars]` is bijective with cryptographic
/// confidence (2^32 space; for the realistic number of agents on a single
/// host, distinct ids produce distinct suffixes). The output is
/// `[0-9a-f]{8}`, which always satisfies Docker's
/// `[a-zA-Z0-9_.-]{1,128}` container-name grammar. Replaces the previous
/// lossy `safe_truncate_str(agent_id, 8)` + character-replacement path
/// that collapsed e.g. `"foo/bar"` and `"foo-bar"` to the same suffix.
fn agent_id_container_suffix(agent_id: &str) -> String {
    let digest = Sha256::digest(agent_id.as_bytes());
    let hex = format!("{digest:x}");
    hex[..8].to_string()
}

/// SECURITY: Validate Docker image name — only allow safe characters.
fn validate_image_name(image: &str) -> Result<(), String> {
    if image.is_empty() {
        return Err("Docker image name cannot be empty".into());
    }
    // Allow: alphanumeric, dots, colons, slashes, dashes, underscores
    if !image
        .chars()
        .all(|c| c.is_alphanumeric() || ".:/-_".contains(c))
    {
        return Err(format!("Invalid Docker image name: {image}"));
    }
    Ok(())
}

/// SECURITY: Sanitize command — reject dangerous shell metacharacters.
/// Delegates to the comprehensive subprocess_sandbox check.
fn validate_command(command: &str) -> Result<(), String> {
    if command.is_empty() {
        return Err("Command cannot be empty".into());
    }
    if let Some(reason) = self::helpers::contains_shell_metacharacters(command) {
        return Err(format!(
            "Command blocked: contains {reason} — potential injection"
        ));
    }
    Ok(())
}

/// Check if Docker is available on this system.
pub async fn is_docker_available() -> bool {
    match tokio::process::Command::new("docker")
        .arg("version")
        .arg("--format")
        .arg("{{.Server.Version}}")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
    {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// Create and start a sandbox container for an agent.
pub async fn create_sandbox(
    config: &DockerSandboxConfig,
    agent_id: &str,
    workspace: &Path,
) -> Result<SandboxContainer, String> {
    validate_image_name(&config.image)?;
    // SECURITY: Fail-fast on dangerous network / cap_add values before we
    // shell out to `docker run`. See `validate_sandbox_config` for the
    // boundary rationale.
    validate_sandbox_config(config)?;
    let container_name = sanitize_container_name(&format!(
        "{}-{}",
        config.container_prefix,
        agent_id_container_suffix(agent_id)
    ))?;

    let mut cmd = tokio::process::Command::new("docker");
    cmd.arg("run").arg("-d").arg("--name").arg(&container_name);

    // Resource limits
    cmd.arg("--memory").arg(&config.memory_limit);
    cmd.arg("--cpus").arg(config.cpu_limit.to_string());
    cmd.arg("--pids-limit").arg(config.pids_limit.to_string());

    // Security: drop ALL capabilities, prevent privilege escalation
    cmd.arg("--cap-drop").arg("ALL");
    cmd.arg("--security-opt").arg("no-new-privileges");

    // Add back specific capabilities if configured. `validate_sandbox_config`
    // above has already rejected anything outside the SAFE_CAPS allowlist,
    // so this loop is now a pure pass-through — no warn-and-skip.
    for cap in &config.cap_add {
        cmd.arg("--cap-add").arg(cap);
    }

    // Read-only root filesystem
    if config.read_only_root {
        cmd.arg("--read-only");
    }

    // Network isolation
    cmd.arg("--network").arg(&config.network);

    // tmpfs mounts
    for tmpfs_mount in &config.tmpfs {
        cmd.arg("--tmpfs").arg(tmpfs_mount);
    }

    // Mount workspace read-only. SECURITY: validate the host path against the
    // symlink-escape and blocked-path checks BEFORE handing it to `docker run`.
    // Without this gate the checks were dead code and a `workspace` resolving
    // through a symlink into e.g. /etc would still be mounted into the sandbox.
    let ws_str = workspace.display().to_string();
    validate_bind_mount(&ws_str, &config.blocked_mounts)?;
    cmd.arg("-v").arg(format!("{ws_str}:{}:ro", config.workdir));

    // Working directory
    cmd.arg("-w").arg(&config.workdir);

    // Image + command to keep container alive
    cmd.arg(&config.image).arg("sleep").arg("infinity");

    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    debug!(container = %container_name, image = %config.image, "Creating Docker sandbox");

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("Failed to run docker: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Docker create failed: {}", stderr.trim()));
    }

    let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    Ok(SandboxContainer {
        container_id,
        agent_id: agent_id.to_string(),
        created_at: chrono::Utc::now(),
    })
}

/// Execute a command inside an existing sandbox container.
pub async fn exec_in_sandbox(
    container: &SandboxContainer,
    command: &str,
    timeout: Duration,
) -> Result<ExecResult, String> {
    validate_command(command)?;

    let mut cmd = tokio::process::Command::new("docker");
    cmd.arg("exec")
        .arg(&container.container_id)
        .arg("sh")
        .arg("-c")
        .arg(command);

    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // SIGKILL the `docker exec` client when the future is dropped — tokio does
    // NOT kill the child on drop by default, so without this a timeout would
    // leave the host-side `docker exec` process running past the deadline and
    // leak one per timed-out call. (The in-container workload is bounded
    // separately by the caller destroying the container.)
    cmd.kill_on_drop(true);

    debug!(container = %container.container_id, "Executing in Docker sandbox");

    let output = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| format!("Docker exec timed out after {}s", timeout.as_secs()))?
        .map_err(|e| format!("Docker exec failed: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    // Truncate large outputs (char-boundary safe to avoid UTF-8 panics)
    let max_output = 50_000;
    let stdout = if stdout.len() > max_output {
        let safe_end = self::helpers::safe_truncate_str(&stdout, max_output);
        format!("{}... [truncated, {} total bytes]", safe_end, stdout.len())
    } else {
        stdout
    };
    let stderr = if stderr.len() > max_output {
        let safe_end = self::helpers::safe_truncate_str(&stderr, max_output);
        format!("{}... [truncated, {} total bytes]", safe_end, stderr.len())
    } else {
        stderr
    };

    Ok(ExecResult {
        stdout,
        stderr,
        exit_code,
    })
}

/// Stop and remove a sandbox container.
pub async fn destroy_sandbox(container: &SandboxContainer) -> Result<(), String> {
    debug!(container = %container.container_id, "Destroying Docker sandbox");

    let output = tokio::process::Command::new("docker")
        .arg("rm")
        .arg("-f")
        .arg(&container.container_id)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to destroy container: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(container = %container.container_id, "Docker rm failed: {}", stderr.trim());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Container Pool (Gap 5) — reuse containers across sessions
// ---------------------------------------------------------------------------

use dashmap::DashMap;
use std::sync::Arc;

/// Pool entry for a reusable container.
#[derive(Debug, Clone)]
struct PoolEntry {
    container: SandboxContainer,
    config_hash: u64,
    last_used: std::time::Instant,
    created: std::time::Instant,
}

/// Container pool for reusing Docker containers.
pub struct ContainerPool {
    entries: Arc<DashMap<String, PoolEntry>>,
}

impl ContainerPool {
    /// Create a new container pool.
    pub fn new() -> Self {
        Self {
            entries: Arc::new(DashMap::new()),
        }
    }

    /// Acquire a container from the pool matching the config hash, or None.
    pub fn acquire(&self, config_hash: u64, cool_secs: u64) -> Option<SandboxContainer> {
        let mut found_key = None;
        for entry in self.entries.iter() {
            if entry.config_hash == config_hash && entry.last_used.elapsed().as_secs() >= cool_secs
            {
                found_key = Some(entry.key().clone());
                break;
            }
        }
        if let Some(key) = found_key {
            self.entries.remove(&key).map(|(_, e)| e.container)
        } else {
            None
        }
    }

    /// Release a container back to the pool.
    pub fn release(&self, container: SandboxContainer, config_hash: u64) {
        self.entries.insert(
            container.container_id.clone(),
            PoolEntry {
                container,
                config_hash,
                last_used: std::time::Instant::now(),
                created: std::time::Instant::now(),
            },
        );
    }

    /// Cleanup containers older than max_age or idle longer than idle_timeout.
    pub async fn cleanup(&self, idle_timeout_secs: u64, max_age_secs: u64) {
        let to_remove: Vec<(String, SandboxContainer)> = self
            .entries
            .iter()
            .filter(|e| {
                e.last_used.elapsed().as_secs() > idle_timeout_secs
                    || e.created.elapsed().as_secs() > max_age_secs
            })
            .map(|e| (e.key().clone(), e.container.clone()))
            .collect();

        for (key, container) in to_remove {
            debug!(container_id = %container.container_id, "Cleaning up stale pool container");
            let _ = destroy_sandbox(&container).await;
            self.entries.remove(&key);
        }
    }

    /// Number of containers in the pool.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for ContainerPool {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Bind Mount Validation (Gap 5) — prevent mounting sensitive host paths
// ---------------------------------------------------------------------------

/// Default blocked mount paths (always blocked regardless of config).
const BLOCKED_MOUNT_PATHS: &[&str] = &[
    "/etc",
    "/proc",
    "/sys",
    "/dev",
    "/var/run/docker.sock",
    "/root",
    "/boot",
];

/// Validate a bind mount path for security.
///
/// Blocks:
/// - Sensitive system paths (/etc, /proc, /sys, Docker socket)
/// - Non-absolute paths
/// - Symlink escape attempts
/// - Paths in the configured blocked_mounts list
pub fn validate_bind_mount(path: &str, blocked: &[String]) -> Result<(), String> {
    let p = std::path::Path::new(path);

    // Must be absolute (Docker bind mounts use Unix paths, so check for '/' prefix
    // in addition to platform-native is_absolute check)
    if !p.is_absolute() && !path.starts_with('/') {
        return Err(format!("Bind mount path must be absolute: {path}"));
    }

    // Check for path traversal
    for component in p.components() {
        if let std::path::Component::ParentDir = component {
            return Err(format!("Bind mount path contains '..': {path}"));
        }
    }

    // Check default blocked paths
    for blocked_path in BLOCKED_MOUNT_PATHS {
        if path.starts_with(blocked_path) {
            return Err(format!(
                "Bind mount to '{blocked_path}' is blocked for security"
            ));
        }
    }

    // Check user-configured blocked paths
    for bp in blocked {
        if path.starts_with(bp.as_str()) {
            return Err(format!("Bind mount to '{bp}' is blocked by configuration"));
        }
    }

    // Symlink escape check: canonicalize path and verify resolved target.
    // If the path does not exist, we walk up to find the closest existing
    // ancestor, canonicalize *that*, and verify the would-be child is still
    // outside blocked paths.  This prevents an attacker from creating a
    // symlink at a non-existent path that later resolves into /etc, /proc, etc.
    let canonical = if p.exists() {
        p.canonicalize()
            .map_err(|e| format!("Cannot canonicalize bind mount path '{path}': {e}"))?
    } else {
        // Walk ancestors until we find one that exists.
        let mut ancestor = p.to_path_buf();
        let mut suffix_parts: Vec<std::ffi::OsString> = Vec::new();
        loop {
            if let Some(parent) = ancestor.parent() {
                if let Some(file_name) = ancestor.file_name() {
                    suffix_parts.push(file_name.to_os_string());
                }
                ancestor = parent.to_path_buf();
                if ancestor.exists() {
                    break;
                }
            } else {
                // Reached filesystem root without finding an existing dir — reject.
                return Err(format!("Bind mount path has no existing ancestor: {path}"));
            }
        }
        let mut resolved = ancestor.canonicalize().map_err(|e| {
            format!("Cannot canonicalize ancestor of bind mount path '{path}': {e}")
        })?;
        for part in suffix_parts.into_iter().rev() {
            resolved.push(part);
        }
        resolved
    };

    let canonical_str = canonical.to_string_lossy();
    for blocked_path in BLOCKED_MOUNT_PATHS {
        if canonical_str.starts_with(blocked_path) {
            return Err(format!(
                "Bind mount resolves to blocked path via symlink: {} → {}",
                path, canonical_str
            ));
        }
    }
    // Also check user-configured blocked paths against resolved path
    for bp in blocked {
        if canonical_str.starts_with(bp.as_str()) {
            return Err(format!(
                "Bind mount resolves to blocked path via symlink: {} → {}",
                path, canonical_str
            ));
        }
    }

    Ok(())
}

/// Hash a Docker sandbox config for pool matching.
pub fn config_hash(config: &DockerSandboxConfig) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    config.image.hash(&mut hasher);
    config.network.hash(&mut hasher);
    config.memory_limit.hash(&mut hasher);
    config.workdir.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_container_name_valid() {
        let result = sanitize_container_name("librefang-sandbox-abc123").unwrap();
        assert_eq!(result, "librefang-sandbox-abc123");
    }

    #[test]
    fn test_sanitize_container_name_special_chars_rejected() {
        // Previously these were silently lossy-replaced with '-', which
        // caused agent-id collisions (e.g. "foo/bar" → "foo-bar" ==
        // "foo-bar"). The validator now rejects disallowed characters
        // outright; the bijective `agent_id_container_suffix` is
        // responsible for keeping the input shape valid before this
        // function ever sees it.
        assert!(sanitize_container_name("test;rm -rf /").is_err());
        assert!(sanitize_container_name("foo/bar").is_err());
        assert!(sanitize_container_name("a b").is_err());
        assert!(sanitize_container_name("a_b").is_err());
    }

    #[test]
    fn test_sanitize_container_name_empty() {
        assert!(sanitize_container_name("").is_err());
    }

    /// Audit regression (docs/issues/docker-container-name-collisions.md,
    /// sub-finding "this"): two distinct agent ids that map to the same
    /// 8-char sanitized prefix used to share a Docker container name.
    /// With the SHA-256 hex suffix, distinct ids produce distinct
    /// suffixes.
    #[test]
    fn test_agent_id_suffix_no_slash_dash_collision() {
        assert_ne!(
            agent_id_container_suffix("foo/bar"),
            agent_id_container_suffix("foo-bar"),
        );
    }

    /// 1000 distinct agent ids produce 1000 distinct suffixes; at the
    /// 2^32 space of an 8-char hex prefix the probability of any
    /// collision in this set is negligible (birthday bound
    /// ~ 1000^2 / 2 / 2^32 ≈ 1.2e-4 per single accidental collision,
    /// effectively zero for the structured inputs below).
    #[test]
    fn test_agent_id_suffix_sweep_distinct() {
        use std::collections::HashSet;
        let mut seen = HashSet::with_capacity(1000);
        for i in 0..1000u32 {
            let id = format!("agent-{i}");
            assert!(
                seen.insert(agent_id_container_suffix(&id)),
                "collision at id {id}"
            );
        }
        assert_eq!(seen.len(), 1000);
    }

    /// Suffix derivation is deterministic across calls — same id, same
    /// suffix, every time.
    #[test]
    fn test_agent_id_suffix_deterministic() {
        let a = agent_id_container_suffix("some-agent-id");
        let b = agent_id_container_suffix("some-agent-id");
        assert_eq!(a, b);
        assert_eq!(a.len(), 8);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// Audit: shell-meta-double-quote-bypass — sanity that the docker
    /// sandbox's denylist mirrors the subprocess sandbox fix: command
    /// substitution / variable expansion inside double quotes must
    /// also reject, since `sh -c` expands them regardless of quoting.
    /// Without this assertion the docker variant could silently
    /// regress while the subprocess variant stays correct.
    #[test]
    fn test_docker_metachar_command_substitution_in_double_quotes_blocked() {
        use self::helpers::contains_shell_metacharacters;
        assert!(contains_shell_metacharacters(r#"echo "$(id)""#).is_some());
        assert!(contains_shell_metacharacters(r#"echo "`id`""#).is_some());
        assert!(contains_shell_metacharacters(r#"echo "${IFS}id""#).is_some());
        // Chaining / redirection inside double quotes still passes
        // (sh treats them literally).
        assert!(contains_shell_metacharacters(r#"echo "a && b""#).is_none());
        assert!(contains_shell_metacharacters(r#"echo "a > b""#).is_none());
    }

    #[test]
    fn test_sanitize_container_name_too_long() {
        let long = "a".repeat(100);
        assert!(sanitize_container_name(&long).is_err());
    }

    #[test]
    fn test_validate_image_name_valid() {
        assert!(validate_image_name("python:3.12-slim").is_ok());
        assert!(validate_image_name("ubuntu:22.04").is_ok());
        assert!(validate_image_name("registry.example.com/my-image:latest").is_ok());
    }

    #[test]
    fn test_validate_image_name_empty() {
        assert!(validate_image_name("").is_err());
    }

    #[test]
    fn test_validate_image_name_invalid() {
        assert!(validate_image_name("image;rm -rf /").is_err());
        assert!(validate_image_name("image`whoami`").is_err());
        assert!(validate_image_name("image$(id)").is_err());
    }

    #[test]
    fn test_validate_command_valid() {
        assert!(validate_command("python script.py").is_ok());
        assert!(validate_command("ls -la /workspace").is_ok());
    }

    #[test]
    fn test_validate_command_pipe_blocked() {
        // SECURITY: Pipes now blocked by comprehensive metacharacter check
        assert!(validate_command("echo hello | grep h").is_err());
    }

    #[test]
    fn test_validate_command_empty() {
        assert!(validate_command("").is_err());
    }

    #[test]
    fn test_validate_command_backticks() {
        assert!(validate_command("echo `whoami`").is_err());
    }

    #[test]
    fn test_validate_command_dollar_paren() {
        assert!(validate_command("echo $(id)").is_err());
    }

    #[test]
    fn test_validate_command_dollar_brace() {
        assert!(validate_command("echo ${HOME}").is_err());
    }

    #[tokio::test]
    async fn test_docker_available() {
        // Just verify it doesn't panic — result depends on Docker installation
        let _ = is_docker_available().await;
    }

    #[test]
    fn test_config_defaults() {
        let config = DockerSandboxConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.image, "python:3.12-slim");
        assert_eq!(config.container_prefix, "librefang-sandbox");
        assert_eq!(config.workdir, "/workspace");
        assert_eq!(config.network, "none");
        assert_eq!(config.memory_limit, "512m");
        assert_eq!(config.cpu_limit, 1.0);
        assert_eq!(config.timeout_secs, 60);
        assert!(config.read_only_root);
        assert!(config.cap_add.is_empty());
        assert_eq!(config.tmpfs, vec!["/tmp:size=64m"]);
        assert_eq!(config.pids_limit, 100);
    }

    #[tokio::test]
    async fn create_sandbox_rejects_blocked_workspace_mount() {
        // `/etc` is a default blocked mount path. `create_sandbox` must refuse a
        // workspace under it — and because `validate_bind_mount` runs before any
        // `docker run`, this returns an error without shelling out to docker.
        // The check used to be dead code: never invoked on the creation path.
        let config = DockerSandboxConfig::default();
        let err = create_sandbox(&config, "agent-1", std::path::Path::new("/etc/secret"))
            .await
            .expect_err("a blocked workspace mount must be rejected");
        assert!(
            err.contains("blocked"),
            "expected a blocked-mount error, got: {err}"
        );
    }

    #[test]
    fn test_exec_result_fields() {
        let result = ExecResult {
            stdout: "hello".to_string(),
            stderr: String::new(),
            exit_code: 0,
        };
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "hello");
    }

    // ── Container Pool tests ──────────────────────────────────────────

    #[test]
    fn test_container_pool_empty() {
        let pool = ContainerPool::new();
        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn test_container_pool_release_acquire() {
        let pool = ContainerPool::new();
        let container = SandboxContainer {
            container_id: "test123".to_string(),
            agent_id: "agent1".to_string(),
            created_at: chrono::Utc::now(),
        };
        pool.release(container, 12345);
        assert_eq!(pool.len(), 1);

        // Acquire with same hash — should succeed (cool_secs=0 for test)
        let acquired = pool.acquire(12345, 0);
        assert!(acquired.is_some());
        assert_eq!(acquired.unwrap().container_id, "test123");
        assert!(pool.is_empty());
    }

    #[test]
    fn test_container_pool_hash_mismatch() {
        let pool = ContainerPool::new();
        let container = SandboxContainer {
            container_id: "test123".to_string(),
            agent_id: "agent1".to_string(),
            created_at: chrono::Utc::now(),
        };
        pool.release(container, 12345);

        // Acquire with different hash — should fail
        let acquired = pool.acquire(99999, 0);
        assert!(acquired.is_none());
    }

    // ── Bind Mount Validation tests ──────────────────────────────────

    #[test]
    fn test_validate_bind_mount_valid() {
        assert!(validate_bind_mount("/home/user/workspace", &[]).is_ok());
        assert!(validate_bind_mount("/tmp/sandbox", &[]).is_ok());
    }

    #[test]
    fn test_validate_bind_mount_non_absolute() {
        assert!(validate_bind_mount("relative/path", &[]).is_err());
    }

    #[test]
    fn test_validate_bind_mount_blocked_paths() {
        assert!(validate_bind_mount("/etc/passwd", &[]).is_err());
        assert!(validate_bind_mount("/proc/self", &[]).is_err());
        assert!(validate_bind_mount("/sys/kernel", &[]).is_err());
        assert!(validate_bind_mount("/var/run/docker.sock", &[]).is_err());
    }

    #[test]
    fn test_validate_bind_mount_traversal() {
        assert!(validate_bind_mount("/home/user/../etc/passwd", &[]).is_err());
    }

    #[test]
    fn test_validate_bind_mount_custom_blocked() {
        let blocked = vec!["/data/secrets".to_string()];
        assert!(validate_bind_mount("/data/secrets/vault", &blocked).is_err());
        assert!(validate_bind_mount("/data/public", &blocked).is_ok());
    }

    #[test]
    fn test_config_hash_deterministic() {
        let c1 = DockerSandboxConfig::default();
        let c2 = DockerSandboxConfig::default();
        assert_eq!(config_hash(&c1), config_hash(&c2));
    }

    // ── Network / cap_add allowlist tests (audit: docker-network-cap-add) ──

    #[test]
    fn test_validate_network_rejects_host() {
        let err = validate_network("host").unwrap_err();
        assert!(
            err.contains("host"),
            "host rejection message should mention 'host': {err}"
        );
        // Case-insensitive: `HOST`, `Host` etc. must also fail.
        assert!(validate_network("HOST").is_err());
        assert!(validate_network("Host").is_err());
    }

    #[test]
    fn test_validate_network_rejects_container_form() {
        assert!(validate_network("container:foo").is_err());
        assert!(validate_network("container:abc123").is_err());
        assert!(validate_network("CONTAINER:foo").is_err());
    }

    #[test]
    fn test_validate_network_accepts_safe_modes() {
        assert!(validate_network("bridge").is_ok());
        assert!(validate_network("none").is_ok());
        assert!(validate_network("my-user-net").is_ok());
        assert!(validate_network("librefang_agents").is_ok());
    }

    #[test]
    fn test_validate_network_rejects_bad_chars_and_empty() {
        assert!(validate_network("").is_err());
        assert!(validate_network("net;rm -rf /").is_err());
        assert!(validate_network("net$(id)").is_err());
        assert!(validate_network("net with space").is_err());
    }

    #[test]
    fn test_validate_capability_rejects_dangerous() {
        // The classic sandbox-collapse trio plus a few extras.
        for bad in [
            "SYS_ADMIN",
            "NET_ADMIN",
            "SYS_PTRACE",
            "SYS_MODULE",
            "SYS_BOOT",
            "BPF",
            "PERFMON",
        ] {
            assert!(
                validate_capability(bad).is_err(),
                "dangerous cap {bad} must be rejected"
            );
            // CAP_-prefixed form must also be rejected.
            let prefixed = format!("CAP_{bad}");
            assert!(
                validate_capability(&prefixed).is_err(),
                "dangerous cap {prefixed} must be rejected"
            );
        }
    }

    #[test]
    fn test_validate_capability_accepts_safe() {
        for good in [
            "CHOWN",
            "DAC_OVERRIDE",
            "FOWNER",
            "NET_BIND_SERVICE",
            "SETUID",
            "SETGID",
        ] {
            assert!(
                validate_capability(good).is_ok(),
                "safe cap {good} must be accepted"
            );
            // CAP_-prefixed form must also be accepted.
            let prefixed = format!("CAP_{good}");
            assert!(
                validate_capability(&prefixed).is_ok(),
                "safe cap {prefixed} must be accepted"
            );
            // Case insensitivity.
            assert!(validate_capability(&good.to_ascii_lowercase()).is_ok());
        }
    }

    #[test]
    fn test_validate_capability_rejects_bad_syntax_and_empty() {
        assert!(validate_capability("").is_err());
        assert!(validate_capability("SYS;ADMIN").is_err());
        assert!(validate_capability("$(id)").is_err());
    }

    #[test]
    fn test_validate_sandbox_config_happy_path() {
        let mut config = DockerSandboxConfig::default();
        // Defaults: network = "none", cap_add empty → must pass.
        assert!(validate_sandbox_config(&config).is_ok());

        // Bridge + safe caps → must pass.
        config.network = "bridge".into();
        config.cap_add = vec!["CHOWN".into(), "NET_BIND_SERVICE".into()];
        assert!(validate_sandbox_config(&config).is_ok());
    }

    #[test]
    fn test_validate_sandbox_config_rejects_host_network() {
        let config = DockerSandboxConfig {
            network: "host".into(),
            ..DockerSandboxConfig::default()
        };
        assert!(validate_sandbox_config(&config).is_err());
    }

    #[test]
    fn test_validate_sandbox_config_rejects_dangerous_cap() {
        let config = DockerSandboxConfig {
            network: "none".into(),
            cap_add: vec!["SYS_ADMIN".into()],
            ..DockerSandboxConfig::default()
        };
        let err = validate_sandbox_config(&config).unwrap_err();
        assert!(
            err.contains("SYS_ADMIN") || err.contains("allowlist"),
            "rejection message should reference the cap or allowlist: {err}"
        );
    }

    #[test]
    fn test_safe_caps_size_and_contents() {
        // Pin the allowlist size so any future widening is a conscious
        // diff that has to update this assertion.
        assert_eq!(SAFE_CAPS.len(), 14, "SAFE_CAPS size changed — review");
        // Spot-check a few entries that the audit specifically called out
        // as the minimum safe set.
        for expected in ["CHOWN", "DAC_OVERRIDE", "FOWNER", "SETUID", "SETGID"] {
            assert!(
                SAFE_CAPS.contains(&expected),
                "SAFE_CAPS must include {expected}"
            );
        }
        // And confirm none of the dangerous ones slipped in.
        for forbidden in ["SYS_ADMIN", "NET_ADMIN", "SYS_PTRACE", "BPF"] {
            assert!(
                !SAFE_CAPS.contains(&forbidden),
                "SAFE_CAPS must NOT include {forbidden}"
            );
        }
    }

    #[test]
    fn test_config_hash_different_images() {
        let c1 = DockerSandboxConfig::default();
        let c2 = DockerSandboxConfig {
            image: "node:20-slim".to_string(),
            ..Default::default()
        };
        assert_ne!(config_hash(&c1), config_hash(&c2));
    }
}

/// Tiny self-contained helpers inlined from `librefang-runtime::subprocess_sandbox`
/// and `librefang-runtime::str_utils` so this crate has no cyclic dep back into
/// the parent runtime crate. The originals stay in their home modules; this is
/// a duplicate-by-design copy bounded to ~60 LOC of pure-string logic.
///
/// Exposed as `pub` (not `pub(crate)`) so the parent `librefang-runtime` crate
/// can drive a parity test asserting these byte-for-byte mirror the canonical
/// implementations — see
/// `crates/librefang-runtime/tests/docker_sandbox_helpers_parity.rs`. The
/// shell-metacharacter check is a security boundary on Docker `exec`; the
/// parity test guards against silent drift when the canonical denylist gains
/// a new entry.
pub mod helpers {
    /// UTF-8-safe truncate (mirrors `librefang_runtime::str_utils::safe_truncate_str`).
    #[inline]
    pub fn safe_truncate_str(s: &str, max_bytes: usize) -> &str {
        if s.len() <= max_bytes {
            return s;
        }
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }

    /// Shell-metacharacter denylist (mirrors
    /// `librefang_runtime::subprocess_sandbox::contains_shell_metacharacters`).
    ///
    /// Quoting handling (audit: shell-meta-double-quote-bypass):
    /// command substitution (`` ` `` , `$(`) and variable expansion
    /// (`${`) fire inside double quotes too, so they MUST be
    /// scanned on the raw string. The chaining / redirection /
    /// globbing metacharacters are only meaningful outside quoted
    /// regions and stay on the strip-then-scan path so legitimate
    /// quoted arguments aren't false-positive-rejected.
    pub fn contains_shell_metacharacters(command: &str) -> Option<String> {
        if command.contains('\n') || command.contains('\r') {
            return Some("embedded newline".to_string());
        }
        if command.contains('\0') {
            return Some("null byte".to_string());
        }
        // Audit: shell-meta-double-quote-bypass — `sh -c` / `bash -c`
        // expand these sequences inside `"…"` too. Scan the raw
        // string, never the strip_quoted_regions output.
        if command.contains('`') {
            return Some("backtick command substitution".to_string());
        }
        if command.contains("$(") {
            return Some("$() command substitution".to_string());
        }
        if command.contains("${") {
            return Some("${} variable expansion".to_string());
        }
        let unquoted = strip_quoted_regions(command);
        if unquoted.contains(';') {
            return Some("semicolon command chaining".to_string());
        }
        if unquoted.contains('|') {
            return Some("pipe operator".to_string());
        }
        if unquoted.contains('>') || unquoted.contains('<') {
            return Some("I/O redirection".to_string());
        }
        if unquoted.contains('{') || unquoted.contains('}') {
            return Some("brace expansion".to_string());
        }
        if unquoted.contains('&') {
            return Some("ampersand operator".to_string());
        }
        None
    }

    fn strip_quoted_regions(command: &str) -> String {
        let mut result = String::with_capacity(command.len());
        let chars: Vec<char> = command.chars().collect();
        let len = chars.len();
        let mut i = 0;
        while i < len {
            match chars[i] {
                '\'' => {
                    i += 1;
                    while i < len && chars[i] != '\'' {
                        i += 1;
                    }
                    if i < len {
                        i += 1;
                    }
                    result.push(' ');
                }
                '"' => {
                    i += 1;
                    while i < len && chars[i] != '"' {
                        if chars[i] == '\\' && i + 1 < len {
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                    if i < len {
                        i += 1;
                    }
                    result.push(' ');
                }
                c => {
                    result.push(c);
                    i += 1;
                }
            }
        }
        result
    }
}
