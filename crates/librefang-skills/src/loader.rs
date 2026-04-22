//! Skill loader — loads and executes skills from various runtimes.

use crate::{SkillError, SkillManifest, SkillRuntime, SkillToolResult};
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error};

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

/// Execute a skill tool by spawning the appropriate runtime.
pub async fn execute_skill_tool(
    manifest: &SkillManifest,
    skill_dir: &Path,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<SkillToolResult, SkillError> {
    // Verify the tool exists in the manifest
    let _tool_def = manifest
        .tools
        .provided
        .iter()
        .find(|t| t.name == tool_name)
        .ok_or_else(|| SkillError::NotFound(format!("Tool {tool_name} not in skill manifest")))?;

    match manifest.runtime.runtime_type {
        SkillRuntime::Python => {
            execute_python(
                skill_dir,
                &manifest.runtime.entry,
                tool_name,
                input,
                &manifest.config,
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
    // tokens, or credentials from the host environment.
    cmd.env_clear();
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
    // Node needs NODE_PATH sometimes
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
        };

        let result = execute_skill_tool(&manifest, dir.path(), "greet", &serde_json::json!({}))
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
        };

        let result = execute_skill_tool(&manifest, dir.path(), "echo_tool", &serde_json::json!({}))
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
        };

        let result = execute_skill_tool(&manifest, dir.path(), "fail_tool", &serde_json::json!({}))
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
        };

        let err = execute_skill_tool(
            &manifest,
            dir.path(),
            "missing_tool",
            &serde_json::json!({}),
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
        };

        let result = execute_skill_tool(&manifest, dir.path(), "test_tool", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(!result.is_error);
        let note = result.output["note"].as_str().unwrap();
        assert!(note.contains("system prompt"));
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
        };

        let err = execute_skill_tool(&manifest, dir.path(), "evil_tool", &serde_json::json!({}))
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
