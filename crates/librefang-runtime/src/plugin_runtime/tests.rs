use super::*;

#[test]
fn version_args_are_runtime_specific() {
    // Go and Lua have their own conventions.
    assert_eq!(PluginRuntime::Go.version_args(), &["version"]);
    assert_eq!(PluginRuntime::Lua.version_args(), &["-v"]);
    // Everyone else uses --version.
    assert_eq!(PluginRuntime::Python.version_args(), &["--version"]);
    assert_eq!(PluginRuntime::Node.version_args(), &["--version"]);
    assert_eq!(PluginRuntime::Ruby.version_args(), &["--version"]);
}

/// Secure-by-default (#2): a `HookConfig` built without explicit overrides
/// must deny both network and filesystem. A plugin that needs either must
/// opt in. Regression guard against silently reverting to the old
/// allow-by-default sandbox-bypass posture.
#[test]
fn hook_config_default_denies_network_and_filesystem() {
    let c = HookConfig::default();
    assert!(
        !c.allow_network,
        "HookConfig::default() must deny network (secure-by-default)"
    );
    assert!(
        !c.allow_filesystem,
        "HookConfig::default() must deny filesystem (secure-by-default)"
    );
}

#[test]
fn plugin_stderr_target_is_stable() {
    // Operator log filters and journalctl pipelines key off this
    // string. Changing it is a breaking change — bump the docs and
    // CHANGELOG together if you ever do.
    assert_eq!(PLUGIN_STDERR_TARGET, "plugin_stderr");
}

#[test]
fn doctor_reports_python_as_available() {
    // Python is on every CI runner we target. A green doctor probe
    // verifies the full path: Command::spawn -> try_wait -> read pipes.
    let status = check_runtime_status(PluginRuntime::Python);
    assert_eq!(status.runtime, "python");
    assert!(
        status.available,
        "python probe failed: {status:?} (version_args mismatch?)"
    );
    assert!(status.launcher.is_some());
    assert!(status.version.is_some());
}

#[test]
fn doctor_reports_native_without_probing() {
    let status = check_runtime_status(PluginRuntime::Native);
    assert_eq!(status.runtime, "native");
    assert!(status.available, "native should always be available");
    assert!(status.launcher.is_none());
    assert!(status.version.is_none());
}

#[test]
fn doctor_flags_missing_launcher() {
    let status = check_runtime_status(PluginRuntime::V); // v is rarely installed
                                                         // We can't assert unavailable deterministically (V *might* be
                                                         // installed), so just check the response shape stays consistent.
    assert_eq!(status.runtime, "v");
    if !status.available {
        assert!(status.launcher.is_none());
        assert!(status.version.is_none());
        assert!(!status.install_hint.is_empty());
    }
}

#[test]
fn from_tag_defaults_to_python() {
    assert_eq!(PluginRuntime::from_tag(None), PluginRuntime::Python);
    assert_eq!(PluginRuntime::from_tag(Some("")), PluginRuntime::Python);
    assert_eq!(
        PluginRuntime::from_tag(Some("python")),
        PluginRuntime::Python
    );
    assert_eq!(PluginRuntime::from_tag(Some("py")), PluginRuntime::Python);
}

#[test]
fn from_tag_normalizes_case_and_aliases() {
    assert_eq!(PluginRuntime::from_tag(Some("V")), PluginRuntime::V);
    assert_eq!(PluginRuntime::from_tag(Some("VLang")), PluginRuntime::V);
    assert_eq!(PluginRuntime::from_tag(Some("Node")), PluginRuntime::Node);
    assert_eq!(PluginRuntime::from_tag(Some("JS")), PluginRuntime::Node);
    assert_eq!(PluginRuntime::from_tag(Some("golang")), PluginRuntime::Go);
    assert_eq!(
        PluginRuntime::from_tag(Some("binary")),
        PluginRuntime::Native
    );
}

#[test]
fn from_tag_unknown_falls_back_to_python() {
    assert_eq!(
        PluginRuntime::from_tag(Some("brainfuck")),
        PluginRuntime::Python
    );
}

#[test]
fn from_tag_new_runtimes() {
    assert_eq!(PluginRuntime::from_tag(Some("ruby")), PluginRuntime::Ruby);
    assert_eq!(PluginRuntime::from_tag(Some("rb")), PluginRuntime::Ruby);
    assert_eq!(PluginRuntime::from_tag(Some("bash")), PluginRuntime::Bash);
    assert_eq!(PluginRuntime::from_tag(Some("sh")), PluginRuntime::Bash);
    assert_eq!(PluginRuntime::from_tag(Some("bun")), PluginRuntime::Bun);
    assert_eq!(PluginRuntime::from_tag(Some("php")), PluginRuntime::Php);
    assert_eq!(PluginRuntime::from_tag(Some("lua")), PluginRuntime::Lua);
}

#[test]
fn from_tag_full_path_uses_custom_runtime() {
    assert_eq!(
        PluginRuntime::from_tag(Some("/opt/homebrew/bin/python3")),
        PluginRuntime::Custom("/opt/homebrew/bin/python3".to_string())
    );
    assert_eq!(
        PluginRuntime::from_tag(Some("C:\\Python313\\python.exe")),
        PluginRuntime::Custom("C:\\Python313\\python.exe".to_string())
    );
}

#[test]
fn parse_output_picks_last_json_line() {
    let lines = vec![
        "warming up...".to_string(),
        "{\"type\":\"ingest_result\",\"memories\":[]}".to_string(),
    ];
    let v = parse_output(&lines).unwrap();
    assert_eq!(v["type"], "ingest_result");
}

#[test]
fn parse_output_falls_back_to_text_wrapper() {
    let lines = vec!["just plain text".to_string()];
    let v = parse_output(&lines).unwrap();
    assert_eq!(v["text"], "just plain text");
}

#[test]
fn parse_output_empty_is_error() {
    assert!(matches!(
        parse_output(&[]),
        Err(PluginRuntimeError::EmptyOutput)
    ));
}

#[test]
fn validate_path_traversal_rejects_parent_dir() {
    assert!(validate_path_traversal("../etc/passwd").is_err());
    assert!(validate_path_traversal("hooks/../evil.sh").is_err());
    assert!(validate_path_traversal("hooks/ingest.py").is_ok());
}

#[test]
fn build_command_shapes() {
    let (l, a) = build_command(PluginRuntime::V, "hooks/ingest.v").unwrap();
    assert_eq!(l, "v");
    assert!(a.contains(&"run".to_string()));
    assert!(a.contains(&"hooks/ingest.v".to_string()));

    let (l, a) = build_command(PluginRuntime::Native, "hooks/ingest").unwrap();
    assert_eq!(l, "hooks/ingest");
    assert!(a.is_empty());

    let (l, a) = build_command(PluginRuntime::Go, "hooks/ingest.go").unwrap();
    assert_eq!(l, "go");
    assert_eq!(a, vec!["run".to_string(), "hooks/ingest.go".to_string()]);

    let (l, a) = build_command(PluginRuntime::Deno, "hooks/ingest.ts").unwrap();
    assert_eq!(l, "deno");
    assert!(a.contains(&"--allow-read".to_string()));

    let (l, a) = build_command(
        PluginRuntime::Custom("/opt/homebrew/bin/python3".to_string()),
        "hooks/ingest.py",
    )
    .unwrap();
    assert_eq!(l, "/opt/homebrew/bin/python3");
    assert_eq!(a, vec!["hooks/ingest.py".to_string()]);
}

/// End-to-end: scaffold a sh-based native hook, run it, check JSON round-trip.
/// Uses `sh` so it works without V/Go/Node installed. Skipped on Windows
/// (no /bin/sh by default).
#[cfg(unix)]
#[tokio::test]
async fn native_runtime_round_trip() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let hook = tmp.path().join("echo_hook");
    std::fs::write(
        &hook,
        "#!/bin/sh\nread _input\nprintf '{\"type\":\"ingest_result\",\"memories\":[]}\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let input = serde_json::json!({
        "type": "ingest",
        "agent_id": "agent-42",
        "message": "hello",
    });
    let out = run_hook_json(
        "ingest",
        hook.to_str().unwrap(),
        PluginRuntime::Native,
        &input,
        // This test pins the spawn / JSON round-trip path with the sandbox
        // explicitly opened. The locked-down (deny-by-default) path is
        // covered separately by `locked_down_default_hook_completes_round_trip`.
        &HookConfig {
            allow_network: true,
            allow_filesystem: true,
            ..Default::default()
        },
    )
    .await
    .expect("native hook ran");
    assert_eq!(out["type"], "ingest_result");
    assert!(out["memories"].is_array());
}

/// Secure-by-default end-to-end (#2): a hook run under the *default* config
/// — deny network, deny filesystem, with `seccomp-sandbox` now in the
/// default feature set — must still complete a normal JSON round-trip.
/// On Linux the child runs behind the unconditional seccomp `KillProcess`
/// allowlist; this is the regression guard for the allowlist being complete
/// enough to launch a plain `/bin/sh` interpreter. A too-narrow allowlist
/// SIGSYS-kills the child before it reads stdin, surfacing as "Broken pipe"
/// (the failure mode that originally blocked enabling seccomp by default).
#[cfg(unix)]
#[tokio::test]
async fn locked_down_default_hook_completes_round_trip() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let hook = tmp.path().join("locked_hook");
    std::fs::write(
        &hook,
        "#!/bin/sh\nread _input\nprintf '{\"type\":\"ingest_result\",\"memories\":[]}\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let input = serde_json::json!({
        "type": "ingest",
        "agent_id": "agent-7",
        "message": "locked down",
    });
    // `HookConfig::default()` is now deny-network + deny-filesystem. With the
    // default feature set, the seccomp filter is applied unconditionally on
    // Linux — the round trip must still succeed.
    let out = run_hook_json(
        "ingest",
        hook.to_str().unwrap(),
        PluginRuntime::Native,
        &input,
        &HookConfig::default(),
    )
    .await
    .expect("locked-down hook ran under default sandbox");
    assert_eq!(out["type"], "ingest_result");
    assert!(out["memories"].is_array());
}

/// Python runtime goes through the same unified spawn path as V/Go/Node —
/// proves there's no special-case shim anymore.
#[cfg(unix)]
#[tokio::test]
async fn python_runtime_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let hook = tmp.path().join("ingest.py");
    std::fs::write(
        &hook,
        "import json, sys\n\
         req = json.loads(sys.stdin.read())\n\
         print(json.dumps({\"type\": \"ingest_result\", \"echo\": req[\"message\"]}))\n",
    )
    .unwrap();

    // Skip test if no python interpreter is on PATH (CI can vary).
    let have_python = ["python3", "python"].iter().any(|bin| {
        std::process::Command::new(bin)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    });
    if !have_python {
        eprintln!("skipping python_runtime_round_trip: no python on PATH");
        return;
    }

    let input = serde_json::json!({
        "type": "ingest",
        "agent_id": "agent-1",
        "message": "ping",
    });
    let out = run_hook_json(
        "ingest",
        hook.to_str().unwrap(),
        PluginRuntime::Python,
        &input,
        // Spawn / round-trip path with the sandbox explicitly opened (see
        // native_runtime_round_trip).
        &HookConfig {
            allow_network: true,
            allow_filesystem: true,
            ..Default::default()
        },
    )
    .await
    .expect("python hook ran");
    assert_eq!(out["type"], "ingest_result");
    assert_eq!(out["echo"], "ping");
}

/// Timeout path: a hook that sleeps forever should be killed.
#[cfg(unix)]
#[tokio::test]
async fn native_runtime_timeout_is_enforced() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let hook = tmp.path().join("slow_hook");
    std::fs::write(&hook, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let config = HookConfig {
        timeout_secs: 1,
        // Exercises the timeout path; sandbox opened so the child reaches the
        // `sleep` (the locked-down path is covered separately).
        allow_network: true,
        allow_filesystem: true,
        ..Default::default()
    };
    let err = run_hook_json(
        "ingest",
        hook.to_str().unwrap(),
        PluginRuntime::Native,
        &serde_json::json!({"type": "ingest"}),
        &config,
    )
    .await
    .expect_err("should time out");
    // A non-Timeout error here means the seccomp allowlist is missing a syscall
    // the second exec (/usr/bin/sleep) needs at glibc startup.
    assert!(
        matches!(err, PluginRuntimeError::Timeout(1)),
        "expected Timeout(1), got {err:?}"
    );
}

/// #3534 follow-up: when one stream blows the cap we must kill the child
/// immediately, not wait for the *other* stream's reader to also drain.
/// The original `tokio::join!` implementation deadlocked here — the
/// over-cap stream's future broke out, but the surviving future kept
/// waiting for EOF that never came (the child was blocked writing into
/// the now-undrained pipe), so the kill only happened after the outer
/// hook timeout fired. Regression test asserts the error fires well
/// before the configured 30 s timeout.
#[cfg(unix)]
#[tokio::test]
async fn overflow_kills_child_without_waiting_for_timeout() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let hook = tmp.path().join("flood.sh");
    // Spew >1 MiB to stdout (the cap), then keep dribbling stderr forever.
    // If the kill is delayed (old `join!` bug) this script keeps the
    // process alive until the hook timeout fires.
    std::fs::write(
        &hook,
        "#!/bin/sh\n\
         yes 'x' | head -c 2000000\n\
         while true; do echo err 1>&2; sleep 1; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let config = HookConfig {
        timeout_secs: 30,
        // Exercises the stream-overflow kill path; sandbox opened (see
        // native_runtime_round_trip).
        allow_network: true,
        allow_filesystem: true,
        ..Default::default()
    };
    let started = std::time::Instant::now();
    let err = run_hook_json(
        "ingest",
        hook.to_str().unwrap(),
        PluginRuntime::Native,
        &serde_json::json!({"type": "ingest"}),
        &config,
    )
    .await
    .expect_err("should report stream cap exceeded");
    let elapsed = started.elapsed();
    assert!(
        matches!(err, PluginRuntimeError::InvalidOutput(ref m) if m.contains("exceeded")),
        "expected InvalidOutput, got {err:?}"
    );
    // 5 s leaves headroom for slow CI but is far below the 30 s timeout
    // that the buggy join! implementation would have hit.
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "kill took {elapsed:?}, expected <5s (timeout was 30s) — child likely deadlocked"
    );
}

/// Missing script surfaces ScriptNotFound (the launcher-not-found path is
/// exercised on real systems where `v` / `go` / `deno` aren't installed).
#[tokio::test]
async fn missing_script_is_script_not_found() {
    let err = run_hook_json(
        "test_hook",
        "hooks/does-not-exist.v",
        PluginRuntime::V,
        &serde_json::json!({}),
        &HookConfig::default(),
    )
    .await
    .expect_err("should fail");
    assert!(matches!(err, PluginRuntimeError::ScriptNotFound(_)));
}
