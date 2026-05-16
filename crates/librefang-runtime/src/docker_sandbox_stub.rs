//! Stub `docker_sandbox` module for `--no-default-features` builds
//! (#3710 Phase 1).
//!
//! Functions return errors so any path that accidentally reaches them
//! gets a clear signal instead of silent success. The `tool_runner`
//! dispatch arm for `docker_exec` and `tool_exec_backend::DockerBackend`
//! are both `#[cfg(feature = "docker-sandbox")]`-gated, so these stubs
//! should not be hit by any real code path when the feature is off.

#![allow(unused_variables, dead_code)]

use librefang_types::config::DockerSandboxConfig;

pub struct SandboxContainer {
    pub container_id: String,
}

pub async fn is_docker_available() -> bool {
    false
}

pub async fn create_sandbox(
    _config: &DockerSandboxConfig,
    _agent_id: &str,
    _workspace: &std::path::Path,
) -> Result<SandboxContainer, String> {
    Err("docker-sandbox feature is disabled in this build".to_string())
}

pub async fn exec_in_sandbox(
    _container: &SandboxContainer,
    _command: &str,
    _timeout: std::time::Duration,
) -> Result<DockerExecOutcome, String> {
    Err("docker-sandbox feature is disabled in this build".to_string())
}

pub async fn destroy_sandbox(_container: &SandboxContainer) -> Result<(), String> {
    Ok(())
}

pub fn validate_bind_mount(_path: &str, _blocked: &[String]) -> Result<(), String> {
    Err("docker-sandbox feature is disabled in this build".to_string())
}

pub fn config_hash(_config: &DockerSandboxConfig) -> u64 {
    0
}

pub struct DockerExecOutcome {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}
