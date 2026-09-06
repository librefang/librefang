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
use std::sync::Arc;

#[derive(Debug, Clone)]
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

// The pool mirrors the real crate's surface so `tool_exec_backend::DockerBackend`, which is
// not feature-gated, compiles against either module. Nothing can be pooled in a build with
// no Docker support, so `derive` yields no key and every other entry point is a no-op.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PoolOwner {
    Session {
        agent_id: String,
        session_id: String,
    },
    Agent {
        agent_id: String,
    },
    Shared,
}

impl PoolOwner {
    pub fn crosses_workloads(&self) -> bool {
        matches!(self, PoolOwner::Shared)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PoolKey {
    pub owner: PoolOwner,
    pub config_hash: u64,
    pub workspace: String,
}

impl PoolKey {
    pub fn derive(
        _config: &DockerSandboxConfig,
        _agent_id: &str,
        _session_id: Option<&str>,
        _workspace: &std::path::Path,
    ) -> Option<Self> {
        None
    }
}

#[derive(Default)]
pub struct ContainerPool;

impl ContainerPool {
    pub fn new() -> Self {
        Self
    }

    pub fn acquire(
        &self,
        _key: &PoolKey,
        _agent_id: &str,
        _reuse_cool_secs: u64,
    ) -> Option<SandboxContainer> {
        None
    }

    pub fn release(&self, _container: SandboxContainer, _key: PoolKey, _released_by: &str) {}

    pub async fn cleanup(&self, _idle_timeout_secs: u64, _max_age_secs: u64) {}

    pub async fn drain(&self) {}

    pub fn len(&self) -> usize {
        0
    }

    pub fn is_empty(&self) -> bool {
        true
    }
}

static GLOBAL_POOL: std::sync::OnceLock<Arc<ContainerPool>> = std::sync::OnceLock::new();

pub fn global_pool() -> Arc<ContainerPool> {
    GLOBAL_POOL
        .get_or_init(|| Arc::new(ContainerPool::new()))
        .clone()
}

pub fn validate_command(_command: &str) -> Result<(), String> {
    Err("docker-sandbox feature is disabled in this build".to_string())
}
