//! Integration tests for tool-exec-backend selection (#3332).
//!
//! These tests exercise the resolver across the **public surface** that
//! a deployment actually touches:
//! - `config.toml` → `KernelConfig.tool_exec.kind`
//! - `agent.toml` → `AgentManifest.tool_exec_backend`
//! - `librefang_types::tool_exec::resolve_backend_kind`
//! - `librefang_runtime::tool_exec_backend::build_backend`
//!
//! No live LLM, no daemon — they're cheap unit-style tests that live in
//! the `librefang-api` test crate so the dispatch path stays exercised
//! from a downstream consumer's perspective (the API crate is the
//! highest-level workspace consumer of the kernel + runtime).

use librefang_runtime::tool_exec_backend::build_backend;
use librefang_types::agent::AgentManifest;
use librefang_types::config::KernelConfig;
use librefang_types::tool_exec::{resolve_backend_kind, BackendKind, ToolExecConfig};

#[test]
fn default_kernel_config_resolves_to_local() {
    let cfg = KernelConfig::default();
    let manifest = AgentManifest::default();
    let kind = resolve_backend_kind(manifest.tool_exec_backend, cfg.tool_exec.kind);
    assert_eq!(kind, BackendKind::Local);
}

#[test]
fn config_toml_kind_local_is_loaded() {
    let toml_str = r#"
        [tool_exec]
        kind = "local"
    "#;
    let cfg: KernelConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.tool_exec.kind, BackendKind::Local);
    assert!(cfg.tool_exec.ssh.is_none());
    assert!(cfg.tool_exec.daytona.is_none());
}

#[test]
fn config_toml_kind_docker_is_loaded() {
    let toml_str = r#"
        [tool_exec]
        kind = "docker"
    "#;
    let cfg: KernelConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.tool_exec.kind, BackendKind::Docker);
}

#[test]
fn config_toml_with_ssh_subtable_round_trips() {
    let toml_str = r#"
        [tool_exec]
        kind = "ssh"
        [tool_exec.ssh]
        host = "build.example.com"
        port = 2222
        user = "agent"
        timeout_secs = 45
    "#;
    let cfg: KernelConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.tool_exec.kind, BackendKind::Ssh);
    let ssh = cfg.tool_exec.ssh.unwrap();
    assert_eq!(ssh.host, "build.example.com");
    assert_eq!(ssh.port, 2222);
    assert_eq!(ssh.user, "agent");
    assert_eq!(ssh.timeout_secs, 45);
}

#[test]
fn config_toml_with_daytona_subtable_round_trips() {
    let toml_str = r#"
        [tool_exec]
        kind = "daytona"
        [tool_exec.daytona]
        api_url = "https://daytona.example.com"
        api_key_env = "MY_DAYTONA_KEY"
        image = "python:3.12"
    "#;
    let cfg: KernelConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.tool_exec.kind, BackendKind::Daytona);
    let dt = cfg.tool_exec.daytona.unwrap();
    assert_eq!(dt.api_url, "https://daytona.example.com");
    assert_eq!(dt.api_key_env, "MY_DAYTONA_KEY");
    assert_eq!(dt.image, "python:3.12");
}

#[test]
fn agent_manifest_tool_exec_backend_field_round_trips() {
    let toml_str = r#"
        name = "alice"
        version = "0.0.0"
        description = ""
        author = ""
        module = "builtin:chat"
        tool_exec_backend = "ssh"

        [model]
        provider = "ollama"
        model = "test-model"
        api_key_env = "OLLAMA_API_KEY"
        message_timeout_secs = 300

        [resources]

        [capabilities]
    "#;
    let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
    assert_eq!(manifest.tool_exec_backend, Some(BackendKind::Ssh));
}

#[test]
fn agent_manifest_no_field_resolves_to_global() {
    // Agent omits the field → falls back to KernelConfig.tool_exec.kind.
    let manifest = AgentManifest::default();
    assert!(manifest.tool_exec_backend.is_none());

    let mut cfg = KernelConfig::default();
    cfg.tool_exec.kind = BackendKind::Docker;
    let kind = resolve_backend_kind(manifest.tool_exec_backend, cfg.tool_exec.kind);
    assert_eq!(kind, BackendKind::Docker);
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
fn build_backend_local_dispatches_to_local_impl() {
    let cfg = ToolExecConfig::default();
    let docker_cfg = librefang_types::config::DockerSandboxConfig::default();
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
    let docker_cfg = librefang_types::config::DockerSandboxConfig::default();
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
fn build_backend_ssh_without_subtable_returns_not_configured() {
    let cfg = ToolExecConfig::default(); // ssh subtable missing
    let docker_cfg = librefang_types::config::DockerSandboxConfig::default();
    let result = build_backend(
        BackendKind::Ssh,
        &cfg,
        &docker_cfg,
        "agent-1",
        std::env::temp_dir(),
        vec![],
    );
    assert!(result.is_err(), "ssh without [tool_exec.ssh] must error");
}

#[test]
fn build_backend_daytona_without_subtable_returns_not_configured() {
    let cfg = ToolExecConfig::default(); // daytona subtable missing
    let docker_cfg = librefang_types::config::DockerSandboxConfig::default();
    let result = build_backend(
        BackendKind::Daytona,
        &cfg,
        &docker_cfg,
        "agent-1",
        std::env::temp_dir(),
        vec![],
    );
    assert!(
        result.is_err(),
        "daytona without [tool_exec.daytona] must error"
    );
}

/// End-to-end resolution: config.toml → manifest.toml → resolver →
/// build_backend → trait impl. Walks the same path a real deployment
/// follows when the daemon boots and dispatches a tool call.
#[tokio::test]
async fn end_to_end_resolution_local_runs_command() {
    // 1. Operator writes config.toml with default tool_exec.
    let cfg_toml = r#"
        # No [tool_exec] section — default is `local`.
    "#;
    let cfg: KernelConfig = toml::from_str(cfg_toml).unwrap();
    assert_eq!(cfg.tool_exec.kind, BackendKind::Local);

    // 2. Operator declares an agent with no override.
    let manifest = AgentManifest::default();

    // 3. Kernel resolves the backend kind for this agent.
    let kind = resolve_backend_kind(manifest.tool_exec_backend, cfg.tool_exec.kind);
    assert_eq!(kind, BackendKind::Local);

    // 4. Kernel builds the backend impl.
    let docker_cfg = librefang_types::config::DockerSandboxConfig::default();
    let backend = build_backend(
        kind,
        &cfg.tool_exec,
        &docker_cfg,
        "agent-1",
        std::env::temp_dir(),
        vec![],
    )
    .expect("local backend always builds");

    // 5. Run a benign command. POSIX-only — the test runners we ship
    //    (Ubuntu CI + macOS CI) all have `sh`.
    #[cfg(unix)]
    {
        let outcome = backend
            .run_command(librefang_runtime::tool_exec_backend::ExecSpec::new(
                "echo end-to-end",
            ))
            .await
            .expect("local exec must succeed");
        assert_eq!(outcome.exit_code, 0);
        assert!(
            outcome.stdout.contains("end-to-end"),
            "stdout was: {:?}",
            outcome.stdout
        );
    }
    // Silence unused-variable warning on non-unix platforms — we
    // still want the build phases (resolver + factory) to be
    // exercised on Windows CI even when the dispatch step is gated.
    #[cfg(not(unix))]
    let _ = backend;
}
