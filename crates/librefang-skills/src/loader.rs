//! Skill loader — loads and executes skills from various runtimes.

use crate::{EnvPassthroughPolicy, SkillError, SkillManifest, SkillRuntime, SkillToolResult};
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error, warn};

/// Env vars that can never flow through to a skill subprocess regardless of
/// skill manifest or operator config. These either inject code
/// (`LD_PRELOAD`, `PYTHONSTARTUP`) or redirect imports/library lookup
/// (`PYTHONPATH`, `NODE_PATH`, `LD_LIBRARY_PATH`) and would defeat the
/// `env_clear` isolation by giving an attacker-controlled host environment a
/// path to execute code inside any opted-in skill.
const FORBIDDEN_PASSTHROUGH: &[&str] = &[
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
    "PYTHONPATH",
    "PYTHONHOME",
    "PYTHONSTARTUP",
    "PYTHONEXECUTABLE",
    "NODE_OPTIONS",
    "NODE_PATH",
];

/// Env vars the kernel sets explicitly per-runtime. Skills cannot override
/// these via `env_passthrough` — kernel settings (notably `PATH`, which the
/// kernel may have deliberately narrowed) are non-negotiable.
const KERNEL_RESERVED_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "TMPDIR",
    "TEMP",
    "SYSTEMROOT",
    "PYTHONIOENCODING",
    "NODE_NO_WARNINGS",
    "SHELL",
    "TERM",
];

fn name_matches(haystack: &str, needle: &str) -> bool {
    haystack.eq_ignore_ascii_case(needle)
}

/// Validate that a resolved script path stays within the skill directory.
/// Prevents path traversal attacks via `../` in entry names.
fn validate_script_path(skill_dir: &Path, entry: &str) -> Result<std::path::PathBuf, SkillError> {
    let script_path = skill_dir.join(entry);

    // Canonicalize both paths to resolve symlinks and `..` components.
    let canonical_dir = skill_dir.canonicalize().map_err(|e| {
        SkillError::ExecutionFailed(format!("Failed to resolve skill directory: {e}"))
    })?;

    // For the script path, we need to check if it exists first
    let canonical_script = if script_path.exists() {
        script_path.canonicalize().map_err(|e| {
            SkillError::ExecutionFailed(format!("Failed to resolve script path: {e}"))
        })?
    } else {
        // If file doesn't exist, normalize manually by resolving the parent
        let parent = script_path
            .parent()
            .ok_or_else(|| SkillError::ExecutionFailed("Invalid script path".into()))?;
        let canonical_parent = parent.canonicalize().map_err(|e| {
            SkillError::ExecutionFailed(format!("Failed to resolve script parent directory: {e}"))
        })?;
        let file_name = script_path
            .file_name()
            .ok_or_else(|| SkillError::ExecutionFailed("Script path has no filename".into()))?;
        canonical_parent.join(file_name)
    };

    if !canonical_script.starts_with(&canonical_dir) {
        return Err(SkillError::ExecutionFailed(format!(
            "Script path '{}' escapes skill directory",
            entry
        )));
    }

    Ok(canonical_script)
}

/// Resolve the effective env-passthrough allowlist for a skill, applying
/// (in order) the FORBIDDEN hard block, the kernel-reserved hard block, and
/// the operator's `denied_patterns` (overridable per-skill via
/// `per_skill_overrides`). Each rejection is logged so operators can debug
/// why a declared var didn't reach the subprocess.
pub fn resolve_effective_passthrough(
    manifest_list: &[String],
    skill_name: &str,
    policy: Option<&EnvPassthroughPolicy>,
) -> Vec<String> {
    let denied: &[String] = policy.map(|p| p.denied_patterns.as_slice()).unwrap_or(&[]);
    let overrides: Option<&Vec<String>> =
        policy.and_then(|p| p.per_skill_overrides.get(skill_name));

    manifest_list
        .iter()
        .filter(|name| {
            if FORBIDDEN_PASSTHROUGH.iter().any(|f| name_matches(f, name)) {
                warn!(
                    skill = skill_name,
                    var = %name,
                    "skill env_passthrough request blocked: var is in FORBIDDEN_PASSTHROUGH \
                     (would defeat subprocess isolation)"
                );
                return false;
            }
            if KERNEL_RESERVED_ENV.iter().any(|r| name_matches(r, name)) {
                warn!(
                    skill = skill_name,
                    var = %name,
                    "skill env_passthrough request blocked: var is kernel-reserved \
                     (kernel sets it explicitly per-runtime)"
                );
                return false;
            }
            let blocked_by_deny = denied
                .iter()
                .any(|pattern| librefang_types::capability::glob_matches(pattern, name));
            if blocked_by_deny {
                let allowed_by_override = overrides
                    .map(|v| v.iter().any(|n| name_matches(n, name)))
                    .unwrap_or(false);
                if !allowed_by_override {
                    warn!(
                        skill = skill_name,
                        var = %name,
                        "skill env_passthrough request blocked by operator deny pattern; \
                         add to [skills].env_passthrough_per_skill if intended"
                    );
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect()
}

/// Execute a skill tool by spawning the appropriate runtime.
///
/// `env_policy` is the operator-side gate over the manifest's
/// `env_passthrough` request. `None` means no operator gate is applied and
/// only the built-in `FORBIDDEN_PASSTHROUGH` / `KERNEL_RESERVED_ENV` hard
/// blocks remain — fine for CLI/dev paths, but production callers should
/// supply a policy derived from `[skills]` config.
pub async fn execute_skill_tool(
    manifest: &SkillManifest,
    skill_dir: &Path,
    tool_name: &str,
    input: &serde_json::Value,
    env_policy: Option<&EnvPassthroughPolicy>,
) -> Result<SkillToolResult, SkillError> {
    // Verify the tool exists in the manifest
    let _tool_def = manifest
        .tools
        .provided
        .iter()
        .find(|t| t.name == tool_name)
        .ok_or_else(|| SkillError::NotFound(format!("Tool {tool_name} not in skill manifest")))?;

    let effective_passthrough =
        resolve_effective_passthrough(&manifest.env_passthrough, &manifest.skill.name, env_policy);

    match manifest.runtime.runtime_type {
        SkillRuntime::Python => {
            execute_python(
                skill_dir,
                &manifest.runtime.entry,
                tool_name,
                input,
                &manifest.config,
                &effective_passthrough,
            )
            .await
        }
        SkillRuntime::Node => {
            execute_node(
                skill_dir,
                &manifest.runtime.entry,
                tool_name,
                input,
                &manifest.config,
                &effective_passthrough,
            )
            .await
        }
        SkillRuntime::Shell => {
            execute_shell(
                skill_dir,
                &manifest.runtime.entry,
                tool_name,
                input,
                &manifest.config,
                &effective_passthrough,
            )
            .await
        }
        SkillRuntime::Wasm => Err(SkillError::RuntimeNotAvailable(
            "WASM skill runtime not yet implemented".to_string(),
        )),
        SkillRuntime::Builtin => Err(SkillError::RuntimeNotAvailable(
            "Builtin skills are handled by the kernel directly".to_string(),
        )),
        SkillRuntime::PromptOnly => {
            // Prompt-only skills inject context into the system prompt.
            // When a tool call arrives here, guide the LLM to use built-in tools.
            Ok(SkillToolResult {
                output: serde_json::json!({
                    "note": "Prompt-context skill — instructions are in your system prompt. Use built-in tools directly."
                }),
                is_error: false,
            })
        }
    }
}

/// Execute a Python skill script.
async fn execute_python(
    skill_dir: &Path,
    entry: &str,
    tool_name: &str,
    input: &serde_json::Value,
    config: &std::collections::HashMap<String, serde_json::Value>,
    env_passthrough: &[String],
) -> Result<SkillToolResult, SkillError> {
    // SECURITY: Validate path containment before any filesystem access
    let script_path = validate_script_path(skill_dir, entry)?;
    if !script_path.exists() {
        return Err(SkillError::ExecutionFailed(format!(
            "Python script not found: {}",
            script_path.display()
        )));
    }

    // Build the JSON payload to send via stdin, including any skill config
    let payload = if config.is_empty() {
        serde_json::json!({
            "tool": tool_name,
            "input": input,
        })
    } else {
        serde_json::json!({
            "tool": tool_name,
            "input": input,
            "config": config,
        })
    };

    let python = find_python().ok_or_else(|| {
        SkillError::RuntimeNotAvailable(
            "Python not found. Install Python 3.8+ to run Python skills.".to_string(),
        )
    })?;

    debug!(
        "Executing Python skill: {} {}",
        python,
        script_path.display()
    );

    let mut cmd = tokio::process::Command::new(&python);
    cmd.arg(&script_path)
        .current_dir(skill_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // SECURITY: Isolate environment to prevent secret leakage.
    // Skills are third-party code — they must not inherit API keys,
    // tokens, or credentials from the host environment by default.
    // Skills that legitimately need specific env vars (e.g. credential
    // helpers for tool subprocesses) can opt in via skill.toml's
    // `env_passthrough = ["VAR_NAME"]` allowlist. The variable name is
    // public (visible in the manifest); only its host-side value crosses
    // the boundary.
    cmd.env_clear();
    // Per-skill env passthrough is applied FIRST so the kernel-curated
    // settings below (PATH, HOME, PYTHONIOENCODING, …) take precedence on
    // last-write-wins. `apply_env_passthrough` already strips kernel-reserved
    // and forbidden names defensively, but this ordering is the load-bearing
    // guarantee: kernel settings are non-negotiable.
    apply_env_passthrough(&mut cmd, env_passthrough);
    // Preserve PATH for binary resolution and platform essentials
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    #[cfg(windows)]
    {
        if let Ok(sp) = std::env::var("SYSTEMROOT") {
            cmd.env("SYSTEMROOT", sp);
        }
        if let Ok(tmp) = std::env::var("TEMP") {
            cmd.env("TEMP", tmp);
        }
    }
    // Python needs PYTHONIOENCODING for UTF-8 output
    cmd.env("PYTHONIOENCODING", "utf-8");

    let mut child = cmd
        .spawn()
        .map_err(|e| SkillError::ExecutionFailed(format!("Failed to spawn Python: {e}")))?;

    // Write input to stdin (ignore broken pipe — process may not need stdin)
    if let Some(mut stdin) = child.stdin.take() {
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| SkillError::ExecutionFailed(format!("JSON serialize: {e}")))?;
        let _ = stdin.write_all(&payload_bytes).await;
        drop(stdin);
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| SkillError::ExecutionFailed(format!("Wait for Python: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("Python skill failed: {stderr}");
        return Ok(SkillToolResult {
            output: serde_json::json!({ "error": stderr.to_string() }),
            is_error: true,
        });
    }

    // Parse stdout as JSON
    let stdout = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(value) => Ok(SkillToolResult {
            output: value,
            is_error: false,
        }),
        Err(_) => Ok(SkillToolResult {
            output: serde_json::json!({ "result": stdout.trim() }),
            is_error: false,
        }),
    }
}

/// Execute a Node.js skill script.
async fn execute_node(
    skill_dir: &Path,
    entry: &str,
    tool_name: &str,
    input: &serde_json::Value,
    config: &std::collections::HashMap<String, serde_json::Value>,
    env_passthrough: &[String],
) -> Result<SkillToolResult, SkillError> {
    // SECURITY: Validate path containment before any filesystem access
    let script_path = validate_script_path(skill_dir, entry)?;
    if !script_path.exists() {
        return Err(SkillError::ExecutionFailed(format!(
            "Node.js script not found: {}",
            script_path.display()
        )));
    }

    let node = find_node().ok_or_else(|| {
        SkillError::RuntimeNotAvailable(
            "Node.js not found. Install Node.js 18+ to run Node skills.".to_string(),
        )
    })?;

    // Build the JSON payload to send via stdin, including any skill config
    let payload = if config.is_empty() {
        serde_json::json!({
            "tool": tool_name,
            "input": input,
        })
    } else {
        serde_json::json!({
            "tool": tool_name,
            "input": input,
            "config": config,
        })
    };

    debug!(
        "Executing Node.js skill: {} {}",
        node,
        script_path.display()
    );

    let mut cmd = tokio::process::Command::new(&node);
    cmd.arg(&script_path)
        .current_dir(skill_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // SECURITY: Isolate environment (same as Python — prevent secret leakage)
    cmd.env_clear();
    // Per-skill passthrough first; kernel-curated settings below win on
    // last-write-wins (see Python runtime above for rationale).
    apply_env_passthrough(&mut cmd, env_passthrough);
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    #[cfg(windows)]
    {
        if let Ok(sp) = std::env::var("SYSTEMROOT") {
            cmd.env("SYSTEMROOT", sp);
        }
        if let Ok(tmp) = std::env::var("TEMP") {
            cmd.env("TEMP", tmp);
        }
    }
    cmd.env("NODE_NO_WARNINGS", "1");

    let mut child = cmd
        .spawn()
        .map_err(|e| SkillError::ExecutionFailed(format!("Failed to spawn Node.js: {e}")))?;

    // Write input to stdin (ignore broken pipe — process may not need stdin)
    if let Some(mut stdin) = child.stdin.take() {
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| SkillError::ExecutionFailed(format!("JSON serialize: {e}")))?;
        let _ = stdin.write_all(&payload_bytes).await;
        drop(stdin);
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| SkillError::ExecutionFailed(format!("Wait for Node.js: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Ok(SkillToolResult {
            output: serde_json::json!({ "error": stderr.to_string() }),
            is_error: true,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(value) => Ok(SkillToolResult {
            output: value,
            is_error: false,
        }),
        Err(_) => Ok(SkillToolResult {
            output: serde_json::json!({ "result": stdout.trim() }),
            is_error: false,
        }),
    }
}

/// Execute a Shell/Bash skill script.
async fn execute_shell(
    skill_dir: &Path,
    entry: &str,
    tool_name: &str,
    input: &serde_json::Value,
    config: &std::collections::HashMap<String, serde_json::Value>,
    env_passthrough: &[String],
) -> Result<SkillToolResult, SkillError> {
    // SECURITY: Validate path containment before any filesystem access
    let script_path = validate_script_path(skill_dir, entry)?;
    if !script_path.exists() {
        return Err(SkillError::ExecutionFailed(format!(
            "Shell script not found: {}",
            script_path.display()
        )));
    }

    let shell = find_shell().ok_or_else(|| {
        SkillError::RuntimeNotAvailable(
            "Shell not found. Install bash or sh to run Shell skills.".to_string(),
        )
    })?;

    // Build the JSON payload to send via stdin, including any skill config
    let payload = if config.is_empty() {
        serde_json::json!({
            "tool": tool_name,
            "input": input,
        })
    } else {
        serde_json::json!({
            "tool": tool_name,
            "input": input,
            "config": config,
        })
    };

    debug!("Executing Shell skill: {} {}", shell, script_path.display());

    let mut cmd = tokio::process::Command::new(&shell);
    cmd.arg(&script_path)
        .current_dir(skill_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // SECURITY: Isolate environment (same as Python/Node — prevent secret leakage)
    cmd.env_clear();
    // Per-skill passthrough first; kernel-curated settings below win on
    // last-write-wins (see Python runtime above for rationale).
    apply_env_passthrough(&mut cmd, env_passthrough);
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    if let Ok(shell_env) = std::env::var("SHELL") {
        cmd.env("SHELL", shell_env);
    }
    if let Ok(term) = std::env::var("TERM") {
        cmd.env("TERM", term);
    }
    #[cfg(windows)]
    {
        if let Ok(sp) = std::env::var("SYSTEMROOT") {
            cmd.env("SYSTEMROOT", sp);
        }
        if let Ok(tmp) = std::env::var("TEMP") {
            cmd.env("TEMP", tmp);
        }
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| SkillError::ExecutionFailed(format!("Failed to spawn shell: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| SkillError::ExecutionFailed(format!("JSON serialize: {e}")))?;
        // Ignore broken pipe — the script may exit before reading stdin
        let _ = stdin.write_all(&payload_bytes).await;
        drop(stdin);
    }

    let timeout_dur = std::time::Duration::from_secs(120);
    let output = match tokio::time::timeout(timeout_dur, child.wait_with_output()).await {
        Ok(result) => {
            result.map_err(|e| SkillError::ExecutionFailed(format!("Wait for shell: {e}")))?
        }
        Err(_) => {
            error!(
                "Shell skill timed out after 120s: {}",
                script_path.display()
            );
            return Ok(SkillToolResult {
                output: "Shell script timed out after 120 seconds".into(),
                is_error: true,
            });
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("Shell skill failed: {stderr}");
        return Ok(SkillToolResult {
            output: serde_json::json!({ "error": stderr.to_string() }),
            is_error: true,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(value) => Ok(SkillToolResult {
            output: value,
            is_error: false,
        }),
        Err(_) => Ok(SkillToolResult {
            output: serde_json::json!({ "result": stdout.trim() }),
            is_error: false,
        }),
    }
}

/// Apply a per-skill env passthrough allowlist to a subprocess command.
///
/// Caller is expected to have pre-filtered the list via
/// [`resolve_effective_passthrough`]; this function additionally enforces
/// the FORBIDDEN and KERNEL_RESERVED hard blocks as defense-in-depth so
/// that a buggy or test caller can never accidentally inject `LD_PRELOAD`
/// or override the kernel's `PATH`.
///
/// For each surviving name, if the variable is set in the host environment,
/// inject it into the child command. Variables not present in the host
/// environment are silently skipped.
fn apply_env_passthrough(cmd: &mut tokio::process::Command, allowlist: &[String]) {
    for var_name in allowlist {
        if FORBIDDEN_PASSTHROUGH
            .iter()
            .any(|f| name_matches(f, var_name))
            || KERNEL_RESERVED_ENV
                .iter()
                .any(|r| name_matches(r, var_name))
        {
            warn!(
                var = %var_name,
                "apply_env_passthrough: refusing to forward forbidden/kernel-reserved var \
                 (resolve_effective_passthrough should have filtered this earlier)"
            );
            continue;
        }
        if let Ok(val) = std::env::var(var_name) {
            cmd.env(var_name, val);
        }
    }
}

/// Find a shell interpreter (bash preferred, sh as fallback).
fn find_shell() -> Option<String> {
    for name in &["bash", "sh"] {
        if std::process::Command::new(name)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return Some(name.to_string());
        }
    }
    None
}

/// Find Python 3 binary.
fn find_python() -> Option<String> {
    for name in &["python3", "python"] {
        if std::process::Command::new(name)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return Some(name.to_string());
        }
    }
    None
}

/// Find Node.js binary.
fn find_node() -> Option<String> {
    if std::process::Command::new("node")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
    {
        return Some("node".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_passthrough_blocks_forbidden_var() {
        let manifest = vec!["LD_PRELOAD".to_string(), "GOG_KEYRING_PASSWORD".to_string()];
        let resolved = resolve_effective_passthrough(&manifest, "any-skill", None);
        assert_eq!(resolved, vec!["GOG_KEYRING_PASSWORD".to_string()]);
    }

    #[test]
    fn test_resolve_passthrough_blocks_kernel_reserved() {
        let manifest = vec!["PATH".to_string(), "MY_VAR".to_string()];
        let resolved = resolve_effective_passthrough(&manifest, "any-skill", None);
        assert_eq!(resolved, vec!["MY_VAR".to_string()]);
    }

    #[test]
    fn test_resolve_passthrough_forbidden_is_case_insensitive() {
        let manifest = vec!["ld_preload".to_string(), "PythonPath".to_string()];
        let resolved = resolve_effective_passthrough(&manifest, "any-skill", None);
        assert!(resolved.is_empty());
    }

    #[test]
    fn test_resolve_passthrough_operator_deny_pattern_blocks() {
        let policy = EnvPassthroughPolicy {
            denied_patterns: vec!["*_KEY".to_string(), "AWS_*".to_string()],
            per_skill_overrides: std::collections::HashMap::new(),
        };
        let manifest = vec![
            "OPENAI_API_KEY".to_string(),
            "AWS_REGION".to_string(),
            "GOG_KEYRING_PASSWORD".to_string(),
        ];
        let resolved = resolve_effective_passthrough(&manifest, "any-skill", Some(&policy));
        // OPENAI_API_KEY matches *_KEY; AWS_REGION matches AWS_*; the keyring
        // password is fine because *_PASSWORD isn't in this policy.
        assert_eq!(resolved, vec!["GOG_KEYRING_PASSWORD".to_string()]);
    }

    #[test]
    fn test_resolve_passthrough_per_skill_override_unblocks() {
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("gog".to_string(), vec!["GOG_KEYRING_PASSWORD".to_string()]);
        let policy = EnvPassthroughPolicy {
            denied_patterns: vec!["*_PASSWORD".to_string()],
            per_skill_overrides: overrides,
        };
        let manifest = vec!["GOG_KEYRING_PASSWORD".to_string()];
        // Without override, blocked.
        assert!(resolve_effective_passthrough(&manifest, "other-skill", Some(&policy)).is_empty());
        // With override (matched on skill name), allowed.
        assert_eq!(
            resolve_effective_passthrough(&manifest, "gog", Some(&policy)),
            vec!["GOG_KEYRING_PASSWORD".to_string()]
        );
    }

    #[test]
    fn test_resolve_passthrough_per_skill_override_cannot_unblock_forbidden() {
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("evil".to_string(), vec!["LD_PRELOAD".to_string()]);
        let policy = EnvPassthroughPolicy {
            denied_patterns: vec![],
            per_skill_overrides: overrides,
        };
        let manifest = vec!["LD_PRELOAD".to_string()];
        let resolved = resolve_effective_passthrough(&manifest, "evil", Some(&policy));
        assert!(
            resolved.is_empty(),
            "operator override must not bypass FORBIDDEN_PASSTHROUGH"
        );
    }

    #[test]
    fn test_find_python() {
        // Just ensure it doesn't panic — result depends on environment
        let _ = find_python();
    }

    #[test]
    fn test_find_node() {
        let _ = find_node();
    }

    #[test]
    fn test_find_shell() {
        // Just ensure it doesn't panic — result depends on environment
        let result = find_shell();
        // On Unix-like systems, at least sh should be available
        #[cfg(unix)]
        assert!(result.is_some(), "Expected bash or sh to be found on Unix");
        #[cfg(not(unix))]
        let _ = result;
    }

    #[test]
    fn test_validate_script_path_allows_normal_entry() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("run.sh"), "#!/bin/bash\n").unwrap();

        let result = validate_script_path(dir.path(), "run.sh");
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
    }

    #[test]
    fn test_validate_script_path_allows_subdirectory() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("scripts")).unwrap();
        std::fs::write(dir.path().join("scripts/run.sh"), "#!/bin/bash\n").unwrap();

        let result = validate_script_path(dir.path(), "scripts/run.sh");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_script_path_blocks_parent_traversal() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        // Create the script file in the parent directory so canonicalize works
        let parent = dir.path().parent().unwrap();
        let evil_script = parent.join("evil.sh");
        std::fs::write(&evil_script, "#!/bin/bash\nrm -rf /\n").unwrap();

        let result = validate_script_path(dir.path(), "../evil.sh");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("escapes skill directory"),
            "Expected 'escapes skill directory' in error, got: {err_msg}"
        );

        // Cleanup
        let _ = std::fs::remove_file(&evil_script);
    }

    #[test]
    fn test_validate_script_path_blocks_deep_traversal() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();

        // Create a file two levels above the subdirectory
        let evil_path = dir.path().join("outside.sh");
        std::fs::write(&evil_path, "#!/bin/bash\n").unwrap();

        let result = validate_script_path(&dir.path().join("sub"), "../../outside.sh");
        // This should fail because ../../ from sub/ goes above skill_dir (sub/)
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_validate_script_path_blocks_absolute_path() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();

        // An absolute path to /etc/passwd should be blocked
        let result = validate_script_path(dir.path(), "/etc/passwd");
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_validate_script_path_blocks_symlink_escape() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.sh"), "#!/bin/bash\n").unwrap();

        // Create a symlink inside skill_dir that points outside
        std::os::unix::fs::symlink(outside.path().join("secret.sh"), dir.path().join("link.sh"))
            .unwrap();

        let result = validate_script_path(dir.path(), "link.sh");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("escapes skill directory"),
            "Expected 'escapes skill directory' in error, got: {err_msg}"
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn test_shell_execution_json_output() {
        use crate::{
            SkillManifest, SkillMeta, SkillRequirements, SkillRuntimeConfig, SkillToolDef,
            SkillTools,
        };
        use tempfile::TempDir;

        // Skip if no shell available
        if find_shell().is_none() {
            return;
        }

        let dir = TempDir::new().unwrap();
        // Write a shell script that reads stdin JSON and echoes JSON output
        let script = r#"#!/bin/bash
read INPUT
echo '{"greeting": "hello from shell"}'
"#;
        std::fs::write(dir.path().join("run.sh"), script).unwrap();

        let manifest = SkillManifest {
            skill: SkillMeta {
                name: "test-shell".to_string(),
                version: librefang_types::VERSION.to_string(),
                description: "A shell test".to_string(),
                author: String::new(),
                license: String::new(),
                tags: vec![],
            },
            runtime: SkillRuntimeConfig {
                runtime_type: SkillRuntime::Shell,
                entry: "run.sh".to_string(),
            },
            tools: SkillTools {
                provided: vec![SkillToolDef {
                    name: "greet".to_string(),
                    description: "Test greeting".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                }],
            },
            requirements: SkillRequirements::default(),
            prompt_context: None,
            source: None,
            config: std::collections::HashMap::new(),
            config_vars: Vec::new(),
            env_passthrough: Vec::new(),
        };

        let result =
            execute_skill_tool(&manifest, dir.path(), "greet", &serde_json::json!({}), None)
                .await
                .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output["greeting"], "hello from shell");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn test_shell_execution_plain_output() {
        use crate::{
            SkillManifest, SkillMeta, SkillRequirements, SkillRuntimeConfig, SkillToolDef,
            SkillTools,
        };
        use tempfile::TempDir;

        if find_shell().is_none() {
            return;
        }

        let dir = TempDir::new().unwrap();
        let script = "#!/bin/bash\necho 'plain text output'\n";
        std::fs::write(dir.path().join("run.sh"), script).unwrap();

        let manifest = SkillManifest {
            skill: SkillMeta {
                name: "test-shell-plain".to_string(),
                version: librefang_types::VERSION.to_string(),
                description: "Shell plain output test".to_string(),
                author: String::new(),
                license: String::new(),
                tags: vec![],
            },
            runtime: SkillRuntimeConfig {
                runtime_type: SkillRuntime::Shell,
                entry: "run.sh".to_string(),
            },
            tools: SkillTools {
                provided: vec![SkillToolDef {
                    name: "echo_tool".to_string(),
                    description: "Test echo".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                }],
            },
            requirements: SkillRequirements::default(),
            prompt_context: None,
            source: None,
            config: std::collections::HashMap::new(),
            config_vars: Vec::new(),
            env_passthrough: Vec::new(),
        };

        let result = execute_skill_tool(
            &manifest,
            dir.path(),
            "echo_tool",
            &serde_json::json!({}),
            None,
        )
        .await
        .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output["result"], "plain text output");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn test_shell_execution_error() {
        use crate::{
            SkillManifest, SkillMeta, SkillRequirements, SkillRuntimeConfig, SkillToolDef,
            SkillTools,
        };
        use tempfile::TempDir;

        if find_shell().is_none() {
            return;
        }

        let dir = TempDir::new().unwrap();
        let script = "#!/bin/bash\necho 'something went wrong' >&2\nexit 1\n";
        std::fs::write(dir.path().join("fail.sh"), script).unwrap();

        let manifest = SkillManifest {
            skill: SkillMeta {
                name: "test-shell-fail".to_string(),
                version: librefang_types::VERSION.to_string(),
                description: "Shell error test".to_string(),
                author: String::new(),
                license: String::new(),
                tags: vec![],
            },
            runtime: SkillRuntimeConfig {
                runtime_type: SkillRuntime::Shell,
                entry: "fail.sh".to_string(),
            },
            tools: SkillTools {
                provided: vec![SkillToolDef {
                    name: "fail_tool".to_string(),
                    description: "Test failure".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                }],
            },
            requirements: SkillRequirements::default(),
            prompt_context: None,
            source: None,
            config: std::collections::HashMap::new(),
            config_vars: Vec::new(),
            env_passthrough: Vec::new(),
        };

        let result = execute_skill_tool(
            &manifest,
            dir.path(),
            "fail_tool",
            &serde_json::json!({}),
            None,
        )
        .await
        .unwrap();
        assert!(result.is_error);
        assert!(result.output["error"]
            .as_str()
            .unwrap()
            .contains("something went wrong"));
    }

    #[tokio::test]
    async fn test_shell_script_not_found() {
        use crate::{
            SkillManifest, SkillMeta, SkillRequirements, SkillRuntimeConfig, SkillToolDef,
            SkillTools,
        };
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();

        let manifest = SkillManifest {
            skill: SkillMeta {
                name: "test-shell-missing".to_string(),
                version: librefang_types::VERSION.to_string(),
                description: "Missing script test".to_string(),
                author: String::new(),
                license: String::new(),
                tags: vec![],
            },
            runtime: SkillRuntimeConfig {
                runtime_type: SkillRuntime::Shell,
                entry: "nonexistent.sh".to_string(),
            },
            tools: SkillTools {
                provided: vec![SkillToolDef {
                    name: "missing_tool".to_string(),
                    description: "Test missing".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                }],
            },
            requirements: SkillRequirements::default(),
            prompt_context: None,
            source: None,
            config: std::collections::HashMap::new(),
            config_vars: Vec::new(),
            env_passthrough: Vec::new(),
        };

        let err = execute_skill_tool(
            &manifest,
            dir.path(),
            "missing_tool",
            &serde_json::json!({}),
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SkillError::ExecutionFailed(_)));
        assert!(err.to_string().contains("Shell script not found"));
    }

    #[tokio::test]
    async fn test_prompt_only_execution() {
        use crate::{
            SkillManifest, SkillMeta, SkillRequirements, SkillRuntimeConfig, SkillToolDef,
            SkillTools,
        };
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let manifest = SkillManifest {
            skill: SkillMeta {
                name: "test-prompt".to_string(),
                version: librefang_types::VERSION.to_string(),
                description: "A prompt-only test".to_string(),
                author: String::new(),
                license: String::new(),
                tags: vec![],
            },
            runtime: SkillRuntimeConfig {
                runtime_type: SkillRuntime::PromptOnly,
                entry: String::new(),
            },
            tools: SkillTools {
                provided: vec![SkillToolDef {
                    name: "test_tool".to_string(),
                    description: "Test".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                }],
            },
            requirements: SkillRequirements::default(),
            prompt_context: Some("You are a helpful assistant.".to_string()),
            source: None,
            config: std::collections::HashMap::new(),
            config_vars: Vec::new(),
            env_passthrough: Vec::new(),
        };

        let result = execute_skill_tool(
            &manifest,
            dir.path(),
            "test_tool",
            &serde_json::json!({}),
            None,
        )
        .await
        .unwrap();
        assert!(!result.is_error);
        let note = result.output["note"].as_str().unwrap();
        assert!(note.contains("system prompt"));
    }

    #[tokio::test]
    #[cfg(unix)]
    #[serial_test::serial(skill_env_passthrough)]
    async fn test_env_passthrough_allowlist_injects_var() {
        use crate::{
            SkillManifest, SkillMeta, SkillRequirements, SkillRuntimeConfig, SkillToolDef,
            SkillTools,
        };
        use tempfile::TempDir;

        if find_shell().is_none() {
            return;
        }

        // Use a unique env var name to avoid collisions with parallel tests
        // or the host environment.
        let allowed_var = "LIBREFANG_TEST_PASSTHROUGH_ALLOWED";
        let blocked_var = "LIBREFANG_TEST_PASSTHROUGH_BLOCKED";
        // SAFETY: tests in this module run serially-enough; the values
        // are scoped to this test's subprocess.
        std::env::set_var(allowed_var, "hello-from-host");
        std::env::set_var(blocked_var, "should-not-leak");

        let dir = TempDir::new().unwrap();
        let script = format!(
            "#!/bin/bash\n\
             read INPUT\n\
             echo \"{{\\\"allowed\\\": \\\"${{{allowed_var}:-MISSING}}\\\", \\\"blocked\\\": \\\"${{{blocked_var}:-MISSING}}\\\"}}\"\n",
        );
        std::fs::write(dir.path().join("run.sh"), script).unwrap();

        let manifest = SkillManifest {
            skill: SkillMeta {
                name: "test-env-passthrough".to_string(),
                version: librefang_types::VERSION.to_string(),
                description: "env passthrough test".to_string(),
                author: String::new(),
                license: String::new(),
                tags: vec![],
            },
            runtime: SkillRuntimeConfig {
                runtime_type: SkillRuntime::Shell,
                entry: "run.sh".to_string(),
            },
            tools: SkillTools {
                provided: vec![SkillToolDef {
                    name: "probe".to_string(),
                    description: "probe env".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                }],
            },
            requirements: SkillRequirements::default(),
            prompt_context: None,
            source: None,
            config: std::collections::HashMap::new(),
            config_vars: Vec::new(),
            env_passthrough: vec![allowed_var.to_string()],
        };

        let result =
            execute_skill_tool(&manifest, dir.path(), "probe", &serde_json::json!({}), None)
                .await
                .unwrap();

        // Cleanup before assertions so a panic doesn't leak state.
        std::env::remove_var(allowed_var);
        std::env::remove_var(blocked_var);

        assert!(!result.is_error, "probe failed: {:?}", result.output);
        assert_eq!(
            result.output["allowed"].as_str(),
            Some("hello-from-host"),
            "allowlisted var did not reach subprocess: {:?}",
            result.output
        );
        assert_eq!(
            result.output["blocked"].as_str(),
            Some("MISSING"),
            "non-allowlisted var leaked into subprocess: {:?}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_path_traversal_blocked_in_shell_execution() {
        use crate::{
            SkillManifest, SkillMeta, SkillRequirements, SkillRuntimeConfig, SkillToolDef,
            SkillTools,
        };
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        // Create a malicious script in parent directory
        let parent = dir.path().parent().unwrap();
        let evil_script = parent.join("evil.sh");
        std::fs::write(&evil_script, "#!/bin/bash\necho PWNED\n").unwrap();

        let manifest = SkillManifest {
            skill: SkillMeta {
                name: "test-traversal".to_string(),
                version: librefang_types::VERSION.to_string(),
                description: "Path traversal test".to_string(),
                author: String::new(),
                license: String::new(),
                tags: vec![],
            },
            runtime: SkillRuntimeConfig {
                runtime_type: SkillRuntime::Shell,
                entry: "../evil.sh".to_string(),
            },
            tools: SkillTools {
                provided: vec![SkillToolDef {
                    name: "evil_tool".to_string(),
                    description: "Evil".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                }],
            },
            requirements: SkillRequirements::default(),
            prompt_context: None,
            source: None,
            config: std::collections::HashMap::new(),
            config_vars: Vec::new(),
            env_passthrough: Vec::new(),
        };

        let err = execute_skill_tool(
            &manifest,
            dir.path(),
            "evil_tool",
            &serde_json::json!({}),
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SkillError::ExecutionFailed(_)));
        assert!(
            err.to_string().contains("escapes skill directory"),
            "Expected 'escapes skill directory' error, got: {err}",
        );

        // Cleanup
        let _ = std::fs::remove_file(&evil_script);
    }
}
