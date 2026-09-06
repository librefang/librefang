//! Docker container sandbox — OS-level isolation for agent code execution.
//!
//! Provides secure command execution inside Docker containers with strict
//! resource limits, network isolation, and capability dropping.

use librefang_types::config::{DockerSandboxConfig, DockerScope};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncReadExt;
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

/// Derive a per-container suffix so two live containers never collide on a Docker name.
///
/// Names used to be `{prefix}-{agent_suffix}`, deterministic per agent, which held only
/// because every container was destroyed by the end of the tool call that created it. Once
/// containers are pooled an agent can legitimately have several alive at once — `scope =
/// "shared"`, or two `docker_exec` calls overlapping — and `docker run --name` refuses the
/// duplicate. Mixing a process-local counter with the wall clock keeps the suffix distinct
/// across concurrent calls and across a daemon restart that left containers behind.
fn container_instance_suffix() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let digest = Sha256::digest(format!("{n}:{nanos}").as_bytes());
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
///
/// Public so a caller can fail an unsafe command before it acquires or creates a container.
/// [`exec_in_sandbox`] still calls it, so the check is never skipped by a caller that forgets.
pub fn validate_command(command: &str) -> Result<(), String> {
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
        "{}-{}-{}",
        config.container_prefix,
        agent_id_container_suffix(agent_id),
        container_instance_suffix()
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

/// Drain a reader into a Vec capped at `cap` bytes, continuing to read to
/// EOF so the underlying pipe never blocks the child. Returns the captured
/// (capped) bytes, the true total observed, and whether truncation occurred.
///
/// Mirrors `LocalBackend::read_capped` in
/// `librefang-runtime/src/tool_exec_backend.rs`; kept module-level so the
/// streaming cap can be exercised by a deterministic regression test without
/// a live Docker daemon.
async fn read_capped<R: AsyncReadExt + Unpin>(
    r: &mut R,
    cap: usize,
) -> std::io::Result<(Vec<u8>, usize, bool)> {
    let mut buf = Vec::with_capacity(std::cmp::min(cap, 8 * 1024));
    let mut total = 0usize;
    let mut truncated = false;
    let mut chunk = [0u8; 8 * 1024];
    loop {
        let n = r.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n);
        if buf.len() < cap {
            let take = std::cmp::min(n, cap - buf.len());
            buf.extend_from_slice(&chunk[..take]);
            if take < n {
                truncated = true;
            }
        } else {
            truncated = true;
        }
    }
    Ok((buf, total, truncated))
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

    // Spawn + stream so a runaway in-container command (`yes`,
    // `head -c 20G /dev/zero`, …) can't OOM-kill the daemon by emitting
    // more bytes than the cap before truncation runs. `cmd.output()`
    // buffered the ENTIRE stdout/stderr into host memory first; instead
    // we drain each pipe into a Vec capped at `max_output`, then keep
    // draining to EOF (counting the true total) so the pipe never blocks
    // the child. Mirrors `LocalBackend::read_capped` in
    // `librefang-runtime/src/tool_exec_backend.rs`. `kill_on_drop(true)`
    // (set above) SIGKILLs the child if the timeout drops this future.
    let max_output = 50_000usize;

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Docker exec failed: {e}"))?;
    let mut stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| "Docker exec: child stdout pipe missing".to_string())?;
    let mut stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| "Docker exec: child stderr pipe missing".to_string())?;

    let stdout_fut = read_capped(&mut stdout_pipe, max_output);
    let stderr_fut = read_capped(&mut stderr_pipe, max_output);
    let wait_fut = child.wait();

    let combined = async move {
        let ((stdout_res, stderr_res), wait_res) =
            tokio::join!(async { tokio::join!(stdout_fut, stderr_fut) }, wait_fut);
        let (stdout_buf, stdout_total, stdout_trunc) =
            stdout_res.map_err(|e| format!("Docker exec stdout read: {e}"))?;
        let (stderr_buf, stderr_total, stderr_trunc) =
            stderr_res.map_err(|e| format!("Docker exec stderr read: {e}"))?;
        let status = wait_res.map_err(|e| format!("Docker exec wait: {e}"))?;
        Ok::<_, String>((
            stdout_buf,
            stdout_total,
            stdout_trunc,
            stderr_buf,
            stderr_total,
            stderr_trunc,
            status,
        ))
    };

    let (stdout_buf, stdout_total, stdout_trunc, stderr_buf, stderr_total, stderr_trunc, status) =
        tokio::time::timeout(timeout, combined)
            .await
            .map_err(|_| format!("Docker exec timed out after {}s", timeout.as_secs()))??;

    let mut stdout = String::from_utf8_lossy(&stdout_buf).into_owned();
    let mut stderr = String::from_utf8_lossy(&stderr_buf).into_owned();
    if stdout_trunc {
        stdout.push_str(&format!("... [truncated, {stdout_total} total bytes]"));
    }
    if stderr_trunc {
        stderr.push_str(&format!("... [truncated, {stderr_total} total bytes]"));
    }
    let exit_code = status.code().unwrap_or(-1);

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
// Container Pool (Gap 5) — reuse containers across tool calls
// ---------------------------------------------------------------------------

use dashmap::DashMap;
use std::sync::Arc;

/// The workload a pooled container belongs to, derived from [`DockerScope`].
///
/// This is the half of [`PoolKey`] that answers *who* may be handed a released container back.
/// The other half — the config fingerprint and the workspace path — answers whether the container was built the way the caller needs at all.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PoolOwner {
    /// `scope = "session"`: the container belongs to one session of one agent.
    Session {
        agent_id: String,
        session_id: String,
    },
    /// `scope = "agent"`: the container belongs to one agent and is reused across its sessions.
    Agent { agent_id: String },
    /// `scope = "shared"`: any caller with the same config and workspace may reuse the container.
    Shared,
}

impl PoolOwner {
    /// Whether a released container can be handed to a *different* workload under this scope.
    ///
    /// Only `shared` can: the other two bake the agent (and, for `session`, the session) into the key, so the only caller that can ever acquire the container is the one that released it.
    /// It is the precondition for `reuse_cool_secs` to apply at all; under `shared` the cooldown then narrows further to a reuse by an agent other than the one that released the container — see [`ContainerPool::acquire`].
    pub fn crosses_workloads(&self) -> bool {
        matches!(self, PoolOwner::Shared)
    }
}

/// Identity a pooled container is reused under.
///
/// Two calls share a container only when every component matches:
///
/// - `owner` — the [`DockerScope`]-derived workload boundary.
/// - `config_hash` — the container-shaping `[docker]` fields, fingerprinted by the `config_hash` function below; a container built from a different image or network is not a substitute.
/// - `workspace` — the host path bind-mounted at `config.workdir`. It is a separate component rather than part of `config_hash` because it is per-call rather than per-config, and leaving it out would let `scope = "shared"` hand agent A's container, with A's workspace still mounted, to agent B.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PoolKey {
    pub owner: PoolOwner,
    pub config_hash: u64,
    pub workspace: String,
}

impl PoolKey {
    /// Derive the pool key for one sandbox call, or `None` when the call cannot be pooled.
    ///
    /// `None` means "create a container for this call and destroy it afterwards" — the pre-pool behaviour.
    /// The only case that produces it is `scope = "session"` at a call site with no LibreFang session to pin the container to (the REST tool bridge, and any other out-of-band invocation): there is no session boundary to honour, and widening such a call to an agent-scoped container would give it a container the operator never asked to share.
    pub fn derive(
        config: &DockerSandboxConfig,
        agent_id: &str,
        session_id: Option<&str>,
        workspace: &Path,
    ) -> Option<Self> {
        let owner = match config.scope {
            DockerScope::Session => PoolOwner::Session {
                agent_id: agent_id.to_string(),
                session_id: session_id?.to_string(),
            },
            DockerScope::Agent => PoolOwner::Agent {
                agent_id: agent_id.to_string(),
            },
            DockerScope::Shared => PoolOwner::Shared,
        };
        Some(Self {
            owner,
            config_hash: config_hash(config),
            workspace: workspace.display().to_string(),
        })
    }
}

/// Pool entry for a reusable container.
#[derive(Debug, Clone)]
struct PoolEntry {
    container: SandboxContainer,
    pool_key: PoolKey,
    /// The agent that put this container back, which is whose leftovers are inside it.
    ///
    /// Under `session` / `agent` scope the key already names that agent, so this is redundant there.
    /// Under `shared` the key deliberately carries no agent, and this is the only record of who used the container last — which is what [`ContainerPool::acquire`] compares against to tell a handover from an agent picking its own container back up.
    /// It is not `container.agent_id`: that field records who *created* the container and keeps saying so after the container has changed hands.
    released_by: String,
    last_used: std::time::Instant,
}

/// Whether a pooled container has aged out under the configured reaper policy.
///
/// `0` disables the axis it is written on rather than expiring everything immediately: an operator writing `idle_timeout_secs = 0` means "never reap on idleness", and the opposite reading would destroy every container the instant it was released, silently turning the pool back into per-call create/destroy.
fn entry_is_stale(
    idle: Duration,
    age: Duration,
    idle_timeout_secs: u64,
    max_age_secs: u64,
) -> bool {
    (idle_timeout_secs > 0 && idle.as_secs() > idle_timeout_secs)
        || (max_age_secs > 0 && age.as_secs() > max_age_secs)
}

/// Container pool for reusing Docker containers across tool calls.
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

    /// Take an idle container matching `key` out of the pool, or `None`.
    ///
    /// `reuse_cool_secs` is the operator's `[docker] reuse_cool_secs`: the settling time a released container must spend idle before it is handed to a *different* workload.
    /// Two things narrow what counts as "different", and both are load-bearing.
    ///
    /// Under `session` / `agent` scope the key already pins the container to one workload, so there is nobody to protect from its leftovers; applying the cooldown there would mean the second tool call of a session never reuses the container the first one built, which is the whole point of the scope.
    /// Under `shared` the key carries no agent, so `released_by` is what says whose leftovers are inside: an agent picking up the container it released a moment ago is not a handover and does not wait, while a genuinely different agent does.
    /// That second distinction is what makes `shared` mean "one container per (config, workspace)" under a single caller.
    /// Without it the default `reuse_cool_secs = 300` blocks every acquire — and because a blocked acquire is answered by *creating another container* rather than by waiting, one agent calling `docker_exec` on a short interval would inflate the shared pool instead of reusing anything.
    pub fn acquire(
        &self,
        key: &PoolKey,
        agent_id: &str,
        reuse_cool_secs: u64,
    ) -> Option<SandboxContainer> {
        let mut found_id = None;
        for entry in self.entries.iter() {
            if &entry.pool_key != key {
                continue;
            }
            let handover = key.owner.crosses_workloads() && entry.released_by != agent_id;
            if handover && entry.last_used.elapsed().as_secs() < reuse_cool_secs {
                continue;
            }
            found_id = Some(entry.key().clone());
            break;
        }
        if let Some(id) = found_id {
            self.entries.remove(&id).map(|(_, e)| e.container)
        } else {
            None
        }
    }

    /// Release a container back to the pool under the key it was acquired or created for.
    ///
    /// `released_by` is the agent that just finished with it — the caller's own id, not `container.agent_id`, which stays pinned to whoever created the container.
    /// Only `shared` scope reads it back; see [`ContainerPool::acquire`].
    pub fn release(&self, container: SandboxContainer, key: PoolKey, released_by: &str) {
        self.entries.insert(
            container.container_id.clone(),
            PoolEntry {
                container,
                pool_key: key,
                released_by: released_by.to_string(),
                last_used: std::time::Instant::now(),
            },
        );
    }

    /// Destroy pooled containers idle past `idle_timeout_secs` or older than `max_age_secs`.
    /// Either bound may be `0` to disable it.
    ///
    /// Age is measured from `SandboxContainer::created_at`, i.e. from `docker run`, not from the
    /// last release. Timing it from the pool entry would restart the clock on every tool call and
    /// leave a busy container immortal — exactly the container `max_age_secs` exists to retire.
    pub async fn cleanup(&self, idle_timeout_secs: u64, max_age_secs: u64) {
        let now = chrono::Utc::now();
        let stale_keys: Vec<String> = self
            .entries
            .iter()
            .filter(|e| {
                let age = (now - e.container.created_at)
                    .to_std()
                    .unwrap_or(Duration::ZERO);
                entry_is_stale(e.last_used.elapsed(), age, idle_timeout_secs, max_age_secs)
            })
            .map(|e| e.key().clone())
            .collect();

        // Remove before destroying: a concurrent `acquire()` racing this reaper tick must never
        // be handed a container that is mid-destroy. If `remove` returns `None`, the entry was
        // already checked out (or reaped) by someone else and is no longer ours to tear down.
        for key in stale_keys {
            if let Some((_, entry)) = self.entries.remove(&key) {
                debug!(container_id = %entry.container.container_id, "Cleaning up stale pool container");
                let _ = destroy_sandbox(&entry.container).await;
            }
        }
    }

    /// Destroy every pooled container and empty the pool.
    ///
    /// A pooled container deliberately outlives the tool call that created it, so without a drain on daemon shutdown every restart would strand one container per live pool key.
    pub async fn drain(&self) {
        let keys: Vec<String> = self.entries.iter().map(|e| e.key().clone()).collect();

        // Same remove-then-destroy ordering as `cleanup`: never destroy an entry that a
        // concurrent `acquire()`/`release()` has already taken out of the map.
        for key in keys {
            if let Some((_, entry)) = self.entries.remove(&key) {
                debug!(container_id = %entry.container.container_id, "Draining pool container on shutdown");
                let _ = destroy_sandbox(&entry.container).await;
            }
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

static GLOBAL_POOL: std::sync::OnceLock<Arc<ContainerPool>> = std::sync::OnceLock::new();

/// The process-wide container pool.
///
/// Host-scoped rather than kernel-scoped, because the resource being pooled is the host's Docker daemon: two kernels in one process would still be creating containers on the same daemon and must not each keep a private view of what is live.
/// The `docker_exec` tool path and the kernel's reaper loop hold this same handle.
pub fn global_pool() -> Arc<ContainerPool> {
    GLOBAL_POOL
        .get_or_init(|| Arc::new(ContainerPool::new()))
        .clone()
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
    // On systemd hosts `/var/run` is a symlink to `/run`, so the real
    // Docker socket path is `/run/docker.sock`; block both the socket
    // and the `/run` runtime dir that shadows it (host-root escape).
    "/run/docker.sock",
    "/run",
    "/root",
    "/boot",
];

/// Component-aware path containment: `path` is inside `prefix` only when it
/// EQUALS `prefix` or the matched prefix is followed by a path separator.
///
/// Raw `str::starts_with` over-blocks (`"/development"` matched `"/dev"`) and
/// under-specifies containment; matching on component boundaries fixes both.
/// A trailing slash on `prefix` (e.g. a user-configured `"/data/secrets/"`)
/// is normalised away so it does not cause a false negative.
fn path_is_within(path: &str, prefix: &str) -> bool {
    let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
    match path.strip_prefix(prefix) {
        Some(rest) => rest.is_empty() || rest.starts_with('/'),
        None => false,
    }
}

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
        if path_is_within(path, blocked_path) {
            return Err(format!(
                "Bind mount to '{blocked_path}' is blocked for security"
            ));
        }
    }

    // Check user-configured blocked paths
    for bp in blocked {
        if path_is_within(path, bp.as_str()) {
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
        if path_is_within(&canonical_str, blocked_path) {
            return Err(format!(
                "Bind mount resolves to blocked path via symlink: {} → {}",
                path, canonical_str
            ));
        }
    }
    // Also check user-configured blocked paths against resolved path
    for bp in blocked {
        if path_is_within(&canonical_str, bp.as_str()) {
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
    config.cpu_limit.to_bits().hash(&mut hasher);
    config.read_only_root.hash(&mut hasher);
    config.cap_add.hash(&mut hasher);
    config.tmpfs.hash(&mut hasher);
    config.pids_limit.hash(&mut hasher);
    config.blocked_mounts.hash(&mut hasher);
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

    fn test_container(id: &str, agent: &str) -> SandboxContainer {
        SandboxContainer {
            container_id: id.to_string(),
            agent_id: agent.to_string(),
            created_at: chrono::Utc::now(),
        }
    }

    fn scoped_config(scope: DockerScope) -> DockerSandboxConfig {
        DockerSandboxConfig {
            enabled: true,
            scope,
            ..Default::default()
        }
    }

    fn key_for(
        scope: DockerScope,
        agent: &str,
        session: Option<&str>,
        workspace: &str,
    ) -> Option<PoolKey> {
        PoolKey::derive(
            &scoped_config(scope),
            agent,
            session,
            std::path::Path::new(workspace),
        )
    }

    #[test]
    fn test_container_pool_empty() {
        let pool = ContainerPool::new();
        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn test_container_pool_release_acquire() {
        let pool = ContainerPool::new();
        let key = key_for(DockerScope::Session, "agent1", Some("sess-a"), "/ws/agent1").unwrap();
        pool.release(test_container("test123", "agent1"), key.clone(), "agent1");
        assert_eq!(pool.len(), 1);

        let acquired = pool.acquire(&key, "agent1", 0);
        assert!(acquired.is_some());
        assert_eq!(acquired.unwrap().container_id, "test123");
        assert!(pool.is_empty());
    }

    #[test]
    fn test_container_pool_config_mismatch() {
        let pool = ContainerPool::new();
        let key = key_for(DockerScope::Agent, "agent1", None, "/ws/agent1").unwrap();
        pool.release(test_container("test123", "agent1"), key, "agent1");

        // Same scope and agent, different image — the container on hand was not built the
        // way this caller needs, so it must not be handed over.
        let mut other_config = scoped_config(DockerScope::Agent);
        other_config.image = "alpine:3.20".to_string();
        let other_key = PoolKey::derive(
            &other_config,
            "agent1",
            None,
            std::path::Path::new("/ws/agent1"),
        )
        .unwrap();
        assert!(pool.acquire(&other_key, "agent1", 0).is_none());
    }

    /// `scope = "session"` means one container per session. Before the pool key carried the
    /// scope, `acquire` matched on a bare config hash, so a container released by one session
    /// was handed to the next session of the same agent.
    #[test]
    fn test_session_scope_does_not_leak_across_sessions() {
        let pool = ContainerPool::new();
        let first = key_for(DockerScope::Session, "agent1", Some("sess-a"), "/ws/agent1").unwrap();
        let second = key_for(DockerScope::Session, "agent1", Some("sess-b"), "/ws/agent1").unwrap();
        assert_ne!(first, second);

        pool.release(test_container("c-a", "agent1"), first.clone(), "agent1");
        assert!(pool.acquire(&second, "agent1", 0).is_none());
        assert_eq!(
            pool.acquire(&first, "agent1", 0).map(|c| c.container_id),
            Some("c-a".to_string())
        );
    }

    /// `scope = "agent"` means one container per agent, reused across sessions — the same two
    /// sessions that must NOT share under `session` scope MUST share under `agent` scope.
    #[test]
    fn test_agent_scope_reuses_across_sessions() {
        let pool = ContainerPool::new();
        let from_first =
            key_for(DockerScope::Agent, "agent1", Some("sess-a"), "/ws/agent1").unwrap();
        let from_second =
            key_for(DockerScope::Agent, "agent1", Some("sess-b"), "/ws/agent1").unwrap();
        assert_eq!(from_first, from_second);

        pool.release(test_container("c-a", "agent1"), from_first, "agent1");
        assert_eq!(
            pool.acquire(&from_second, "agent1", 0)
                .map(|c| c.container_id),
            Some("c-a".to_string())
        );
    }

    /// `scope = "agent"` still separates agents from each other.
    #[test]
    fn test_agent_scope_separates_agents() {
        let pool = ContainerPool::new();
        let a = key_for(DockerScope::Agent, "agent1", None, "/ws/agent1").unwrap();
        let b = key_for(DockerScope::Agent, "agent2", None, "/ws/agent2").unwrap();
        pool.release(test_container("c-a", "agent1"), a, "agent1");
        assert!(pool.acquire(&b, "agent2", 0).is_none());
    }

    /// `scope = "shared"` pools across agents — but only agents that mount the same workspace.
    #[test]
    fn test_shared_scope_reuses_across_agents_with_same_workspace() {
        let pool = ContainerPool::new();
        let a = key_for(DockerScope::Shared, "agent1", None, "/ws/common").unwrap();
        let b = key_for(DockerScope::Shared, "agent2", None, "/ws/common").unwrap();
        assert_eq!(a, b);

        pool.release(test_container("c-a", "agent1"), a, "agent1");
        assert_eq!(
            pool.acquire(&b, "agent2", 0).map(|c| c.container_id),
            Some("c-a".to_string())
        );
    }

    /// The workspace is bind-mounted into the container, so a shared container may only be
    /// handed to a caller that mounts the same host path. Without the workspace in the key,
    /// `scope = "shared"` would give agent B a container with agent A's workspace mounted.
    #[test]
    fn test_shared_scope_does_not_cross_workspaces() {
        let pool = ContainerPool::new();
        let a = key_for(DockerScope::Shared, "agent1", None, "/ws/agent1").unwrap();
        let b = key_for(DockerScope::Shared, "agent2", None, "/ws/agent2").unwrap();
        assert_ne!(a, b);

        pool.release(test_container("c-a", "agent1"), a, "agent1");
        assert!(pool.acquire(&b, "agent2", 0).is_none());
    }

    /// `scope = "session"` at a call site with no session (the REST tool bridge, cron) has no
    /// session boundary to honour, so the call is unpoolable: create, exec, destroy.
    #[test]
    fn test_session_scope_without_session_id_is_unpoolable() {
        assert!(key_for(DockerScope::Session, "agent1", None, "/ws/agent1").is_none());
        // The scopes that do not name a session are unaffected.
        assert!(key_for(DockerScope::Agent, "agent1", None, "/ws/agent1").is_some());
        assert!(key_for(DockerScope::Shared, "agent1", None, "/ws/agent1").is_some());
    }

    /// `reuse_cool_secs` never gates `session` / `agent` scope: the key already pins the
    /// container to one workload, and making the second tool call of a session skip the
    /// container the first one built is exactly what the scope exists to prevent.
    #[test]
    fn test_reuse_cooldown_never_gates_session_or_agent_scope() {
        let pool = ContainerPool::new();

        let agent = key_for(DockerScope::Agent, "agent1", None, "/ws/agent1").unwrap();
        pool.release(test_container("c-agent", "agent1"), agent.clone(), "agent1");
        assert!(
            pool.acquire(&agent, "agent1", 300).is_some(),
            "an agent-scoped container is reusable by its own agent immediately"
        );

        let session =
            key_for(DockerScope::Session, "agent1", Some("sess-a"), "/ws/agent1").unwrap();
        pool.release(
            test_container("c-session", "agent1"),
            session.clone(),
            "agent1",
        );
        assert!(
            pool.acquire(&session, "agent1", 300).is_some(),
            "a session-scoped container is reusable by its own session immediately"
        );
    }

    /// Under `shared`, `reuse_cool_secs` is a *handover* delay, not a per-acquire delay.
    ///
    /// The distinction decides whether the documented "one container per (config, workspace)"
    /// is true. A blocked acquire is answered by creating another container rather than by
    /// waiting (see `tool_docker_exec` / `DockerBackend::run_command`), so if the cooldown
    /// applied to the releasing agent's own next call, an agent calling `docker_exec` every
    /// 10s would stack up `reuse_cool_secs / 10` containers before any reuse began — the pool
    /// inflating instead of collapsing to one. This walks the whole sequence rather than a
    /// single acquire, because a single acquire cannot tell the two readings apart.
    #[test]
    fn test_shared_cooldown_delays_handover_not_self_reuse() {
        let pool = ContainerPool::new();
        let shared = key_for(DockerScope::Shared, "agent1", None, "/ws/common").unwrap();

        // agent1 releases, then calls again straight away: its own leftovers, no wait.
        pool.release(test_container("c-1", "agent1"), shared.clone(), "agent1");
        assert_eq!(
            pool.acquire(&shared, "agent1", 300).map(|c| c.container_id),
            Some("c-1".to_string()),
            "an agent must reuse the shared container it just released, not create another"
        );

        // Repeat: the same agent stays on the same container call after call.
        pool.release(test_container("c-1", "agent1"), shared.clone(), "agent1");
        assert_eq!(
            pool.acquire(&shared, "agent1", 300).map(|c| c.container_id),
            Some("c-1".to_string())
        );

        // agent2 arriving inside the settling window is a real handover and is made to wait.
        pool.release(test_container("c-1", "agent1"), shared.clone(), "agent1");
        assert!(
            pool.acquire(&shared, "agent2", 300).is_none(),
            "a container another agent just used is still cooling down"
        );
        // Past the window (0 here stands in for elapsed >= reuse_cool_secs) it is handed over.
        assert_eq!(
            pool.acquire(&shared, "agent2", 0).map(|c| c.container_id),
            Some("c-1".to_string())
        );

        // Having taken it over, agent2 is now the releaser and no longer waits on itself,
        // while agent1 coming back is the one that has to settle.
        pool.release(test_container("c-1", "agent2"), shared.clone(), "agent2");
        assert!(
            pool.acquire(&shared, "agent1", 300).is_none(),
            "the cooldown follows the last user, not the container's creator"
        );
        assert_eq!(
            pool.acquire(&shared, "agent2", 300).map(|c| c.container_id),
            Some("c-1".to_string())
        );
    }

    #[test]
    fn test_pool_owner_crosses_workloads() {
        assert!(PoolOwner::Shared.crosses_workloads());
        assert!(!PoolOwner::Agent {
            agent_id: "a".into()
        }
        .crosses_workloads());
        assert!(!PoolOwner::Session {
            agent_id: "a".into(),
            session_id: "s".into()
        }
        .crosses_workloads());
    }

    /// Reaper policy, exercised without a Docker daemon: `cleanup` only decides *which*
    /// entries to destroy, and that decision is this function.
    #[test]
    fn test_entry_is_stale_idle_and_age_bounds() {
        let zero = Duration::from_secs(0);
        // Idle past the idle timeout.
        assert!(entry_is_stale(Duration::from_secs(400), zero, 300, 604800));
        assert!(!entry_is_stale(Duration::from_secs(200), zero, 300, 604800));
        // Older than the max age even though it was used a moment ago.
        assert!(entry_is_stale(zero, Duration::from_secs(700), 300, 600));
        assert!(!entry_is_stale(zero, Duration::from_secs(500), 300, 600));
    }

    /// `0` disables an axis. The opposite reading would reap every container the instant it
    /// was released, quietly collapsing the pool back into per-call create/destroy.
    #[test]
    fn test_entry_is_stale_zero_disables_axis() {
        let long = Duration::from_secs(10 * 365 * 24 * 3600);
        assert!(!entry_is_stale(long, long, 0, 0));
        assert!(entry_is_stale(long, Duration::from_secs(0), 1, 0));
        assert!(entry_is_stale(Duration::from_secs(0), long, 0, 1));
    }

    /// Pooled containers coexist, so their Docker names must differ. Before the instance
    /// suffix, every container for one agent was named `{prefix}-{sha(agent)[..8]}` and the
    /// second concurrent `docker run` failed with a name conflict.
    #[test]
    fn test_container_instance_suffix_is_unique() {
        use std::collections::HashSet;
        let mut seen = HashSet::with_capacity(1000);
        for _ in 0..1000 {
            let suffix = container_instance_suffix();
            assert_eq!(suffix.len(), 8);
            assert!(
                suffix.chars().all(|c| c.is_ascii_hexdigit()),
                "instance suffix must satisfy the Docker name grammar: {suffix}"
            );
            assert!(seen.insert(suffix), "duplicate container instance suffix");
        }
    }

    /// The composed container name still passes the Docker name validator with the default
    /// prefix, so the instance suffix cannot push a default deployment over the 63-char cap.
    #[test]
    fn test_composed_container_name_is_valid() {
        let config = DockerSandboxConfig::default();
        let name = format!(
            "{}-{}-{}",
            config.container_prefix,
            agent_id_container_suffix("some-agent-id"),
            container_instance_suffix()
        );
        assert!(sanitize_container_name(&name).is_ok(), "name: {name}");
    }

    /// `container_prefix` is an operator-settable key, and the instance suffix costs it nine
    /// characters of headroom: the name is `{prefix}-{8 hex}-{8 hex}`, so the 63-char Docker cap
    /// bites at a 45-char prefix where it used to bite at 54. Pinned here so a future change to
    /// the name format cannot move the cap without someone noticing and updating the `[docker]`
    /// tables in the configuration docs.
    #[test]
    fn test_container_prefix_length_boundary() {
        fn composed(prefix: &str) -> Result<String, String> {
            sanitize_container_name(&format!(
                "{}-{}-{}",
                prefix,
                agent_id_container_suffix("some-agent-id"),
                container_instance_suffix()
            ))
        }
        assert!(
            composed(&"p".repeat(45)).is_ok(),
            "a 45-char container_prefix must still compose to a valid name"
        );
        let too_long = composed(&"p".repeat(46));
        assert!(
            too_long
                .as_ref()
                .is_err_and(|e| e.contains("Container name too long")),
            "a 46-char container_prefix must be rejected, got: {too_long:?}"
        );
        // The shipped default has plenty of room.
        assert!(composed(&DockerSandboxConfig::default().container_prefix).is_ok());
    }

    /// Requires a live Docker daemon: exercises the acquire → exec → release → re-acquire
    /// round trip against real containers, including that a released container is the one
    /// handed back. Run with `cargo test -p librefang-runtime-sandbox-docker -- --ignored`.
    #[tokio::test]
    #[ignore = "requires a running Docker daemon"]
    async fn test_pool_round_trip_against_live_daemon() {
        if !is_docker_available().await {
            panic!("Docker daemon required; run without --ignored to skip this test");
        }
        let config = scoped_config(DockerScope::Agent);
        let workspace = std::env::temp_dir();
        let pool = ContainerPool::new();
        let key = PoolKey::derive(&config, "pool-round-trip", None, &workspace)
            .expect("agent scope always yields a key");

        let container = create_sandbox(&config, "pool-round-trip", &workspace)
            .await
            .expect("create sandbox");
        let id = container.container_id.clone();
        pool.release(container, key.clone(), "pool-round-trip");

        let reused = pool
            .acquire(&key, "pool-round-trip", config.reuse_cool_secs)
            .expect("reuse");
        assert_eq!(reused.container_id, id);
        let out = exec_in_sandbox(&reused, "echo pooled", Duration::from_secs(30))
            .await
            .expect("exec in reused container");
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("pooled"));

        pool.release(reused, key, "pool-round-trip");
        pool.drain().await;
        assert!(pool.is_empty());
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

    /// Audit (LOW security): on systemd hosts `/var/run` is a symlink to
    /// `/run`, so the real Docker socket path `/run/docker.sock` must be
    /// blocked too — otherwise a bind mount of it hands the sandbox
    /// host-root. `/run` itself is blocked as it shadows the socket.
    #[test]
    fn test_validate_bind_mount_blocks_run_docker_sock() {
        assert!(validate_bind_mount("/run/docker.sock", &[]).is_err());
        assert!(validate_bind_mount("/run", &[]).is_err());
        assert!(validate_bind_mount("/run/user/1000", &[]).is_err());
    }

    /// Audit (LOW correctness): component-aware containment must not
    /// over-block a sibling path that merely shares a textual prefix with
    /// a blocked path. `/development` is NOT inside `/dev`, and
    /// `/rundir` is NOT inside `/run`.
    #[test]
    fn test_validate_bind_mount_sibling_prefix_not_blocked() {
        assert!(validate_bind_mount("/development", &[]).is_ok());
        assert!(validate_bind_mount("/development/src", &[]).is_ok());
        assert!(validate_bind_mount("/rundir", &[]).is_ok());
        // Exact blocked path and a true child of it still reject.
        assert!(validate_bind_mount("/dev", &[]).is_err());
        assert!(validate_bind_mount("/dev/null", &[]).is_err());
    }

    /// Audit (correctness): a trailing slash on a user-configured blocked
    /// path must still match its children (no false negative).
    #[test]
    fn test_validate_bind_mount_custom_blocked_trailing_slash() {
        let blocked = vec!["/data/secrets/".to_string()];
        assert!(validate_bind_mount("/data/secrets/vault", &blocked).is_err());
        assert!(validate_bind_mount("/data/secretstore", &blocked).is_ok());
    }

    /// Audit (HIGH panic-dos): the streaming reader caps at `cap` bytes
    /// while still draining to EOF, so an unbounded producer cannot buffer
    /// its full output into host memory. Without the cap the captured
    /// buffer would grow to the full ~200 KB the child emits.
    #[tokio::test]
    async fn test_read_capped_bounds_unbounded_output() {
        // Emit far more than the cap: 200 KB of 'A'.
        let payload = 200_000usize;
        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(format!("head -c {payload} /dev/zero | tr '\\0' A"))
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn producer");
        let mut pipe = child.stdout.take().expect("stdout pipe");
        let cap = 50_000usize;
        let (buf, total, truncated) = read_capped(&mut pipe, cap).await.expect("read");
        let _ = child.wait().await;
        assert_eq!(buf.len(), cap, "captured buffer must be capped at {cap}");
        assert!(truncated, "producer exceeding the cap must set truncated");
        assert_eq!(total, payload, "true total must count every byte drained");
    }

    /// The cap must NOT truncate output that fits: small outputs pass
    /// through whole with `truncated == false`.
    #[tokio::test]
    async fn test_read_capped_passes_small_output() {
        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("printf hello")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn producer");
        let mut pipe = child.stdout.take().expect("stdout pipe");
        let (buf, total, truncated) = read_capped(&mut pipe, 50_000).await.expect("read");
        let _ = child.wait().await;
        assert_eq!(&buf, b"hello");
        assert_eq!(total, 5);
        assert!(!truncated);
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

    #[test]
    fn test_config_hash_covers_container_security_boundary() {
        let baseline = DockerSandboxConfig::default();
        let mut variants = Vec::new();

        let mut config = baseline.clone();
        config.cap_add = vec!["CHOWN".into()];
        variants.push(("cap_add", config));

        let mut config = baseline.clone();
        config.read_only_root = !config.read_only_root;
        variants.push(("read_only_root", config));

        let mut config = baseline.clone();
        config.pids_limit += 1;
        variants.push(("pids_limit", config));

        let mut config = baseline.clone();
        config.tmpfs.push("/run:size=8m".into());
        variants.push(("tmpfs", config));

        let mut config = baseline.clone();
        config.cpu_limit = 2.0;
        variants.push(("cpu_limit", config));

        let mut config = baseline.clone();
        config.blocked_mounts.push("/home/private".into());
        variants.push(("blocked_mounts", config));

        let baseline_hash = config_hash(&baseline);
        for (field, config) in variants {
            assert_ne!(
                baseline_hash,
                config_hash(&config),
                "changing {field} must prevent container pool reuse"
            );
        }
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
