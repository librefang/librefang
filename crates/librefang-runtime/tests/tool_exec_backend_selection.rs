//! Integration tests for the tool-exec backend dispatch path (#3332).
//!
//! Mirrors the end-to-end resolution that a real deployment performs:
//! `config.toml` → `KernelConfig.tool_exec` + `agent.toml` →
//! `AgentManifest.tool_exec_backend` → resolver → `build_backend` →
//! trait-object dispatch.
//!
//! These run against the runtime crate directly, not through the API
//! server, because the dispatch contract lives entirely in the
//! types + runtime layer. A parallel version of the same test exists
//! in `librefang-api/tests/` so the API crate exercises the same
//! seam from its build.

use librefang_runtime::tool_exec_backend::{build_backend, ExecSpec};
use librefang_types::agent::AgentManifest;
use librefang_types::config::{DockerSandboxConfig, KernelConfig};
use librefang_types::tool_exec::{resolve_backend_kind, BackendKind, ToolExecConfig};

#[test]
fn default_kernel_config_resolves_to_local() {
    let cfg = KernelConfig::default();
    let manifest = AgentManifest::default();
    let kind = resolve_backend_kind(manifest.tool_exec_backend, cfg.tool_exec.kind);
    assert_eq!(kind, BackendKind::Local);
}

#[test]
fn config_toml_kind_local_loads() {
    let cfg: KernelConfig = toml::from_str("[tool_exec]\nkind = \"local\"").unwrap();
    assert_eq!(cfg.tool_exec.kind, BackendKind::Local);
}

#[test]
fn config_toml_kind_docker_loads() {
    let cfg: KernelConfig = toml::from_str("[tool_exec]\nkind = \"docker\"").unwrap();
    assert_eq!(cfg.tool_exec.kind, BackendKind::Docker);
}

#[test]
fn agent_manifest_override_wins_over_global() {
    let manifest = AgentManifest {
        tool_exec_backend: Some(BackendKind::Ssh),
        ..Default::default()
    };
    let mut cfg = KernelConfig::default();
    cfg.tool_exec.kind = BackendKind::Docker;
    let kind = resolve_backend_kind(manifest.tool_exec_backend, cfg.tool_exec.kind);
    assert_eq!(kind, BackendKind::Ssh);
}

#[test]
fn agent_manifest_no_field_falls_back_to_global() {
    let manifest = AgentManifest::default();
    assert!(manifest.tool_exec_backend.is_none());
    let mut cfg = KernelConfig::default();
    cfg.tool_exec.kind = BackendKind::Docker;
    let kind = resolve_backend_kind(manifest.tool_exec_backend, cfg.tool_exec.kind);
    assert_eq!(kind, BackendKind::Docker);
}

#[test]
fn build_backend_local_dispatches_to_local_impl() {
    let cfg = ToolExecConfig::default();
    let docker_cfg = DockerSandboxConfig::default();
    let backend = build_backend(
        BackendKind::Local,
        &cfg,
        &docker_cfg,
        "agent-1",
        std::env::temp_dir(),
        vec![],
    )
    .expect("local backend always builds");
    assert_eq!(backend.kind(), BackendKind::Local);
}

#[test]
fn build_backend_docker_dispatches_to_docker_impl() {
    let cfg = ToolExecConfig::default();
    let docker_cfg = DockerSandboxConfig::default();
    let backend = build_backend(
        BackendKind::Docker,
        &cfg,
        &docker_cfg,
        "agent-1",
        std::env::temp_dir(),
        vec![],
    )
    .expect("docker backend builds even when daemon absent");
    assert_eq!(backend.kind(), BackendKind::Docker);
}

#[test]
fn build_backend_ssh_without_subtable_or_feature_errors() {
    let cfg = ToolExecConfig::default();
    let docker_cfg = DockerSandboxConfig::default();
    let result = build_backend(
        BackendKind::Ssh,
        &cfg,
        &docker_cfg,
        "agent-1",
        std::env::temp_dir(),
        vec![],
    );
    assert!(result.is_err());
}

#[test]
fn build_backend_daytona_without_subtable_or_feature_errors() {
    let cfg = ToolExecConfig::default();
    let docker_cfg = DockerSandboxConfig::default();
    let result = build_backend(
        BackendKind::Daytona,
        &cfg,
        &docker_cfg,
        "agent-1",
        std::env::temp_dir(),
        vec![],
    );
    assert!(result.is_err());
}

#[tokio::test]
async fn end_to_end_local_dispatch_runs_command() {
    if !cfg!(unix) {
        return; // POSIX-only smoke
    }
    // 1. Operator's config.toml — empty / no [tool_exec] section.
    let cfg: KernelConfig = toml::from_str("").unwrap();
    assert_eq!(cfg.tool_exec.kind, BackendKind::Local);

    // 2. Default agent manifest — no per-agent override.
    let manifest = AgentManifest::default();

    // 3. Resolve kind.
    let kind = resolve_backend_kind(manifest.tool_exec_backend, cfg.tool_exec.kind);
    assert_eq!(kind, BackendKind::Local);

    // 4. Build the backend.
    let docker_cfg = DockerSandboxConfig::default();
    let backend = build_backend(
        kind,
        &cfg.tool_exec,
        &docker_cfg,
        "agent-1",
        std::env::temp_dir(),
        vec![],
    )
    .expect("local backend always builds");

    // 5. Run a benign command.
    let outcome = backend
        .run_command(ExecSpec::new("echo end-to-end-3332"))
        .await
        .expect("local exec succeeds");
    assert_eq!(outcome.exit_code, 0);
    assert!(
        outcome.stdout.contains("end-to-end-3332"),
        "stdout was: {:?}",
        outcome.stdout
    );
}
