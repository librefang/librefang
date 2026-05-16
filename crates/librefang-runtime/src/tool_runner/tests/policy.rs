use super::*;

// ---- RBAC M3 — per-user tool policy gate (#3054) ----

#[tokio::test]
async fn tool_runner_rbac_user_deny_returns_hard_error() {
    let approval_requests = Arc::new(AtomicUsize::new(0));
    let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
        approval_requests: Arc::clone(&approval_requests),
        user_gate_override: Some(librefang_types::user_policy::UserToolGate::Deny {
            reason: "user 'Bob' (role: user) is not permitted to invoke 'shell_exec'".to_string(),
        }),
    });

    let result = execute_tool(
        "test-id",
        "shell_exec",
        &serde_json::json!({"command": "echo ok"}),
        Some(&kernel),
        None,
        Some("agent-1"),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None,
        None,
        None,
        None,
        Some("bob"),
        Some("telegram"),
        None,
        None,
        None,
        None,
        None,
    )
    .await;

    assert!(result.is_error, "user-policy deny must produce an error");
    assert!(
        result.content.contains("Execution denied"),
        "content should announce the deny: {}",
        result.content
    );
    assert!(
        result.content.contains("user 'Bob'"),
        "deny reason must surface to the model: {}",
        result.content
    );
    // No approval was requested — the deny short-circuits.
    assert_eq!(approval_requests.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn tool_runner_rbac_user_needs_approval_routes_through_approval_queue() {
    let approval_requests = Arc::new(AtomicUsize::new(0));
    let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
        approval_requests: Arc::clone(&approval_requests),
        // file_write is NOT in the default require_approval list (which
        // would already gate it). The point of this test is to prove the
        // user gate flips it into approval-required mode regardless of
        // the global policy.
        user_gate_override: Some(librefang_types::user_policy::UserToolGate::NeedsApproval {
            reason: "tool 'file_write' requires admin approval for user 'Bob'".to_string(),
        }),
    });

    let workspace = tempfile::tempdir().expect("tempdir");

    let result = execute_tool(
        "test-id",
        "file_write",
        &serde_json::json!({"path": "scratch.txt", "content": "hi"}),
        Some(&kernel),
        None,
        Some("agent-1"),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        Some(workspace.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("bob"),
        Some("telegram"),
        None,
        None,
        None,
        None,
        None,
    )
    .await;

    // User gate forced approval — the tool is deferred (NotBlocked).
    assert_eq!(
        result.status,
        librefang_types::tool::ToolExecutionStatus::WaitingApproval,
        "expected WaitingApproval status, got content: {}",
        result.content
    );
    assert_eq!(approval_requests.load(Ordering::SeqCst), 1);
}

/// Regression: shell_exec under `ExecPolicy.mode = Full` MUST still
/// route through the approval queue when the per-user gate returned
/// `NeedsApproval`. Without the `!force_approval` guard added in B2
/// of PR #3205 review, the Full-mode bypass silently dropped the
/// user-gate escalation and the call ran without human review.
#[tokio::test]
async fn tool_runner_rbac_full_mode_does_not_bypass_user_needs_approval() {
    let approval_requests = Arc::new(AtomicUsize::new(0));
    let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
        approval_requests: Arc::clone(&approval_requests),
        user_gate_override: Some(librefang_types::user_policy::UserToolGate::NeedsApproval {
            reason: "tool 'shell_exec' requires admin approval for user 'Bob'".to_string(),
        }),
    });

    let workspace = tempfile::tempdir().expect("tempdir");
    let policy = librefang_types::config::ExecPolicy {
        mode: librefang_types::config::ExecSecurityMode::Full,
        ..Default::default()
    };

    let result = execute_tool(
        "test-id",
        "shell_exec",
        &serde_json::json!({"command": "echo ok"}),
        Some(&kernel),
        None,
        Some("agent-1"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(workspace.path()),
        None,
        None,
        Some(&policy), // Full mode!
        None,
        None,
        None,
        None,
        Some("bob"),
        Some("telegram"),
        None,
        None,
        None,
        None,
        None,
    )
    .await;

    assert_eq!(
        result.status,
        librefang_types::tool::ToolExecutionStatus::WaitingApproval,
        "Full mode + user NeedsApproval must still demand approval, got content: {}",
        result.content
    );
    assert_eq!(
        approval_requests.load(Ordering::SeqCst),
        1,
        "exactly one approval request should be submitted"
    );
}

#[tokio::test]
async fn tool_runner_rbac_user_allow_falls_through_to_existing_approval_logic() {
    // user_gate_override = Allow → behaviour matches the pre-RBAC
    // approval flow. shell_exec is in the default require_approval
    // list and ApprovalKernel.requires_approval() returns true for it,
    // so we still expect WaitingApproval — proving Allow is a true
    // pass-through, not a bypass.
    let approval_requests = Arc::new(AtomicUsize::new(0));
    let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
        approval_requests: Arc::clone(&approval_requests),
        user_gate_override: Some(librefang_types::user_policy::UserToolGate::Allow),
    });

    let result = execute_tool(
        "test-id",
        "shell_exec",
        &serde_json::json!({"command": "echo ok"}),
        Some(&kernel),
        None,
        Some("agent-1"),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("alice"),
        Some("telegram"),
        None,
        None,
        None,
        None,
        None,
    )
    .await;

    assert_eq!(
        result.status,
        librefang_types::tool::ToolExecutionStatus::WaitingApproval
    );
    assert_eq!(approval_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_shell_exec_uses_exec_policy_allowed_env_vars() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let original = std::env::var("LIBREFANG_TEST_ALLOWED_ENV").ok();
    // SAFETY: test captures and restores the previous value; unique enough
    // name to avoid clashing with other tests running in parallel.
    unsafe {
        std::env::set_var("LIBREFANG_TEST_ALLOWED_ENV", "present");
    }

    let allowed = ["shell_exec".to_string()];
    let policy = librefang_types::config::ExecPolicy {
        mode: librefang_types::config::ExecSecurityMode::Allowlist,
        allowed_env_vars: vec!["LIBREFANG_TEST_ALLOWED_ENV".to_string()],
        ..Default::default()
    };

    let result = execute_tool(
        "test-id",
        "shell_exec",
        &serde_json::json!({"command": "env"}),
        None,
        Some(&allowed),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        Some(workspace.path()),
        None, // media_engine
        None, // media_drivers
        Some(&policy),
        None,
        None,
        None,
        None,
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;

    match original {
        Some(val) => unsafe {
            std::env::set_var("LIBREFANG_TEST_ALLOWED_ENV", val);
        },
        None => unsafe {
            std::env::remove_var("LIBREFANG_TEST_ALLOWED_ENV");
        },
    }

    assert!(
        !result.is_error,
        "shell_exec should succeed with env passthrough, got: {}",
        result.content
    );
    assert!(
        result
            .content
            .contains("LIBREFANG_TEST_ALLOWED_ENV=present"),
        "allowed env var should be visible to subprocess, got: {}",
        result.content
    );
}

// --- Schedule parser tests ---
#[test]
fn test_parse_schedule_every_minutes() {
    assert_eq!(
        parse_schedule_to_cron("every 5 minutes").unwrap(),
        "*/5 * * * *"
    );
    assert_eq!(
        parse_schedule_to_cron("every 1 minute").unwrap(),
        "* * * * *"
    );
    assert_eq!(parse_schedule_to_cron("every minute").unwrap(), "* * * * *");
    assert_eq!(
        parse_schedule_to_cron("every 30 minutes").unwrap(),
        "*/30 * * * *"
    );
}

#[test]
fn test_parse_schedule_every_hours() {
    assert_eq!(parse_schedule_to_cron("every hour").unwrap(), "0 * * * *");
    assert_eq!(parse_schedule_to_cron("every 1 hour").unwrap(), "0 * * * *");
    assert_eq!(
        parse_schedule_to_cron("every 2 hours").unwrap(),
        "0 */2 * * *"
    );
}

#[test]
fn test_parse_schedule_daily() {
    assert_eq!(parse_schedule_to_cron("daily at 9am").unwrap(), "0 9 * * *");
    assert_eq!(
        parse_schedule_to_cron("daily at 6pm").unwrap(),
        "0 18 * * *"
    );
    assert_eq!(
        parse_schedule_to_cron("daily at 12am").unwrap(),
        "0 0 * * *"
    );
    assert_eq!(
        parse_schedule_to_cron("daily at 12pm").unwrap(),
        "0 12 * * *"
    );
}

#[test]
fn test_parse_schedule_weekdays() {
    assert_eq!(
        parse_schedule_to_cron("weekdays at 9am").unwrap(),
        "0 9 * * 1-5"
    );
    assert_eq!(
        parse_schedule_to_cron("weekends at 10am").unwrap(),
        "0 10 * * 0,6"
    );
}

#[test]
fn test_parse_schedule_shorthand() {
    assert_eq!(parse_schedule_to_cron("hourly").unwrap(), "0 * * * *");
    assert_eq!(parse_schedule_to_cron("daily").unwrap(), "0 0 * * *");
    assert_eq!(parse_schedule_to_cron("weekly").unwrap(), "0 0 * * 0");
    assert_eq!(parse_schedule_to_cron("monthly").unwrap(), "0 0 1 * *");
}

#[test]
fn test_parse_schedule_cron_passthrough() {
    assert_eq!(
        parse_schedule_to_cron("0 */5 * * *").unwrap(),
        "0 */5 * * *"
    );
    assert_eq!(
        parse_schedule_to_cron("30 9 * * 1-5").unwrap(),
        "30 9 * * 1-5"
    );
}

#[test]
fn test_parse_schedule_invalid() {
    assert!(parse_schedule_to_cron("whenever I feel like it").is_err());
    assert!(parse_schedule_to_cron("every 0 minutes").is_err());
}

// --- Image format detection tests ---
#[test]
fn test_detect_image_format_png() {
    let data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x10\x00\x00\x00\x10";
    assert_eq!(detect_image_format(data), "png");
}

#[test]
fn test_detect_image_format_jpeg() {
    let data = b"\xFF\xD8\xFF\xE0\x00\x10JFIF";
    assert_eq!(detect_image_format(data), "jpeg");
}

#[test]
fn test_detect_image_format_gif() {
    let data = b"GIF89a\x10\x00\x10\x00";
    assert_eq!(detect_image_format(data), "gif");
}

#[test]
fn test_detect_image_format_bmp() {
    let data = b"BM\x00\x00\x00\x00";
    assert_eq!(detect_image_format(data), "bmp");
}

#[test]
fn test_detect_image_format_unknown() {
    let data = b"\x00\x00\x00\x00";
    assert_eq!(detect_image_format(data), "unknown");
}

#[test]
fn test_extract_png_dimensions() {
    // Minimal PNG header: signature (8) + IHDR length (4) + "IHDR" (4) + width (4) + height (4)
    let mut data = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]; // signature
    data.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D]); // IHDR length
    data.extend_from_slice(b"IHDR"); // chunk type
    data.extend_from_slice(&640u32.to_be_bytes()); // width
    data.extend_from_slice(&480u32.to_be_bytes()); // height
    assert_eq!(extract_image_dimensions(&data, "png"), Some((640, 480)));
}

#[test]
fn test_extract_gif_dimensions() {
    let mut data = b"GIF89a".to_vec();
    data.extend_from_slice(&320u16.to_le_bytes()); // width
    data.extend_from_slice(&240u16.to_le_bytes()); // height
    assert_eq!(extract_image_dimensions(&data, "gif"), Some((320, 240)));
}

#[test]
fn test_format_file_size() {
    assert_eq!(format_file_size(500), "500 B");
    assert_eq!(format_file_size(1536), "1.5 KB");
    assert_eq!(format_file_size(2 * 1024 * 1024), "2.0 MB");
}

#[tokio::test]
async fn test_image_analyze_missing_file() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let result = execute_tool(
        "test-id",
        "image_analyze",
        &serde_json::json!({"path": "nonexistent_image.png"}),
        None,
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        Some(workspace.path()),
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(result.is_error);
    assert!(
        result.content.contains("Failed to read"),
        "unexpected error content: {}",
        result.content
    );
}

/// Regression test for #4450: the media/image read-only tools must accept
/// paths inside named-workspace prefixes (the "additional_roots" allowlist),
/// not just the primary workspace root. Before the fix these tools called
/// the bare `resolve_file_path` wrapper which threaded `&[]` and produced
/// "resolves outside workspace" even when the agent had declared the mount
/// under `[workspaces]`.
#[tokio::test]
async fn test_media_tools_honor_named_workspace_prefixes() {
    // Two disjoint dirs: `workspace_root` is the agent's primary workspace,
    // `mount` is the named-workspace prefix. The test file lives only in
    // `mount`, so success proves the prefix was honored.
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let mount = tempfile::tempdir().expect("mount tempdir");
    let mount_canon = mount.path().canonicalize().expect("canonicalize mount");
    let img_path = mount_canon.join("photo.png");
    // Minimal PNG signature so detect_image_format() returns "png".
    let png_bytes: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    std::fs::write(&img_path, png_bytes).expect("write png");

    let raw_path = img_path.to_string_lossy().to_string();
    let input = serde_json::json!({ "path": raw_path });

    // Without prefixes -> rejected as outside the sandbox.
    let denied = tool_image_analyze(&input, Some(workspace.path()), &[]).await;
    assert!(
        denied.is_err(),
        "image_analyze should reject paths outside the workspace when \
         no named-workspace prefixes are provided, got: {:?}",
        denied
    );
    let err = denied.unwrap_err();
    assert!(
        err.contains("resolves outside workspace") || err.contains("Access denied"),
        "expected sandbox rejection, got: {err}"
    );

    // With the mount as an additional root -> accepted.
    let extra: &[&Path] = &[mount_canon.as_path()];
    let ok = tool_image_analyze(&input, Some(workspace.path()), extra).await;
    assert!(
        ok.is_ok(),
        "image_analyze must accept files under a named-workspace prefix, \
         got: {:?}",
        ok
    );
}

#[test]
fn test_depth_limit_constant() {
    assert_eq!(MAX_AGENT_CALL_DEPTH, 5);
}

#[test]
fn test_depth_limit_first_call_succeeds() {
    // Default depth is 0, which is < MAX_AGENT_CALL_DEPTH
    let default_depth = AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0);
    assert!(default_depth < MAX_AGENT_CALL_DEPTH);
}

#[test]
fn test_task_local_compiles() {
    // Verify task_local macro works — just ensure the type exists
    let cell = std::cell::Cell::new(0u32);
    assert_eq!(cell.get(), 0);
}

#[tokio::test]
async fn test_schedule_tools_without_kernel() {
    let result = execute_tool(
        "test-id",
        "schedule_list",
        &serde_json::json!({}),
        None,
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("Kernel handle not available"));
}

// ─── Canvas / A2UI tests ────────────────────────────────────────

#[test]
fn test_sanitize_canvas_basic_html() {
    let html = "<h1>Hello World</h1><p>This is a test.</p>";
    let result = sanitize_canvas_html(html, 512 * 1024);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), html);
}

#[test]
fn test_sanitize_canvas_rejects_script() {
    let html = "<div><script>alert('xss')</script></div>";
    let result = sanitize_canvas_html(html, 512 * 1024);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("script"));
}

#[test]
fn test_sanitize_canvas_rejects_iframe() {
    let html = "<iframe src='https://evil.com'></iframe>";
    let result = sanitize_canvas_html(html, 512 * 1024);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("iframe"));
}

#[test]
fn test_sanitize_canvas_rejects_event_handler() {
    let html = "<div onclick=\"alert('xss')\">click me</div>";
    let result = sanitize_canvas_html(html, 512 * 1024);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("event handler"));
}

#[test]
fn test_sanitize_canvas_rejects_onload() {
    let html = "<img src='x' onerror = \"alert(1)\">";
    let result = sanitize_canvas_html(html, 512 * 1024);
    assert!(result.is_err());
}

#[test]
fn test_sanitize_canvas_rejects_javascript_url() {
    let html = "<a href=\"javascript:alert('xss')\">click</a>";
    let result = sanitize_canvas_html(html, 512 * 1024);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("javascript:"));
}

#[test]
fn test_sanitize_canvas_rejects_data_html() {
    let html = "<a href=\"data:text/html,<script>alert(1)</script>\">x</a>";
    let result = sanitize_canvas_html(html, 512 * 1024);
    assert!(result.is_err());
}

#[test]
fn test_sanitize_canvas_rejects_empty() {
    let result = sanitize_canvas_html("", 512 * 1024);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Empty"));
}

#[test]
fn test_sanitize_canvas_size_limit() {
    let html = "x".repeat(1024);
    let result = sanitize_canvas_html(&html, 100);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("too large"));
}

#[tokio::test]
async fn test_canvas_present_tool() {
    let input = serde_json::json!({
        "html": "<h1>Test Canvas</h1><p>Hello world</p>",
        "title": "Test"
    });
    let tmp = std::env::temp_dir().join("librefang_canvas_test");
    let _ = std::fs::create_dir_all(&tmp);
    let result = tool_canvas_present(&input, Some(tmp.as_path())).await;
    assert!(result.is_ok());
    let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert!(output["canvas_id"].is_string());
    assert_eq!(output["title"], "Test");
    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_agent_spawn_manifest_all_cases() {
    let mut toml;

    // Case 1: Minimal - only name and system_prompt
    toml =
        build_agent_manifest_toml("test-agent", "You are helpful.", vec![], vec![], false).unwrap();
    assert!(toml.contains("name = \"test-agent\""));
    assert!(toml.contains("system_prompt = \"You are helpful.\""));
    assert!(toml.contains("tools = []"));
    assert!(!toml.contains("network"));
    assert!(!toml.contains("shell = ["));

    // Case 2: With tools (no network)
    toml = build_agent_manifest_toml(
        "coder",
        "You are a coder.",
        vec!["file_read".to_string(), "file_write".to_string()],
        vec![],
        false,
    )
    .unwrap();
    assert!(toml.contains("tools = [\"file_read\", \"file_write\"]"));
    assert!(!toml.contains("network"));

    // Case 3: network explicitly enabled
    toml = build_agent_manifest_toml(
        "web-agent",
        "You browse the web.",
        vec!["web_fetch".to_string()],
        vec![],
        true,
    )
    .unwrap();
    assert!(toml.contains("web_fetch"));
    assert!(toml.contains("network = [\"*\"]"));

    // Case 4: shell without shell_exec - should auto-add shell_exec to tools
    toml = build_agent_manifest_toml(
        "shell-test",
        "You run commands.",
        vec!["git".to_string()],
        vec!["uv *".to_string()],
        false,
    )
    .unwrap();
    assert!(toml.contains("shell = [\"uv *\"]"));
    assert!(toml.contains("shell_exec")); // auto-added

    // Case 5: shell with explicit shell_exec (should not duplicate)
    toml = build_agent_manifest_toml(
        "shell-test",
        "You run commands.",
        vec!["shell_exec".to_string(), "git".to_string()],
        vec!["uv *".to_string(), "cargo *".to_string()],
        false,
    )
    .unwrap();
    assert!(toml.contains("shell = [\"uv *\", \"cargo *\"]"));
    // shell_exec should only appear once
    let shell_exec_count = toml.matches("shell_exec").count();
    assert_eq!(shell_exec_count, 1);

    // Case 6: Special chars in strings
    toml = build_agent_manifest_toml(
        "agent-with\"quotes",
        "He said \"hello\" and '''goodbye'''.",
        vec![],
        vec![],
        false,
    )
    .unwrap();
    assert!(toml.contains("agent-with\"quotes"));

    // Case 7: Multiple tools with web_fetch and shell (auto-adds shell_exec)
    toml = build_agent_manifest_toml(
        "multi-agent",
        "You do everything.",
        vec!["web_fetch".to_string(), "git".to_string()],
        vec!["ls *".to_string()],
        true,
    )
    .unwrap();
    assert!(toml.contains("web_fetch"));
    assert!(toml.contains("network = [\"*\"]"));
    assert!(toml.contains("shell = [\"ls *\"]"));
    assert!(toml.contains("shell_exec")); // auto-added
}

// -----------------------------------------------------------------------
// Security fix tests (#1652)
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_file_read_no_workspace_root_returns_error() {
    // SECURITY: file_read must fail when workspace_root is None.
    // Use a relative path so the inner sandbox resolver is the rejecter
    // (the absolute-path pre-ACP guard has its own coverage above —
    // and Windows treats `/etc/passwd` as relative, which would also
    // mask the inner-resolver path we want to exercise here).
    let result = execute_tool(
        "test-id",
        "file_read",
        &serde_json::json!({"path": "etc/passwd"}),
        None,
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None, // workspace_root = None
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(
        result.is_error,
        "Expected error when workspace_root is None"
    );
    assert!(
        result.content.contains("Workspace sandbox not configured"),
        "Expected workspace sandbox error, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_file_write_no_workspace_root_returns_error() {
    // SECURITY: file_write must fail when workspace_root is None.
    // Relative path so the inner resolver — not the absolute-path
    // pre-ACP guard — is what rejects the call (cross-platform: on
    // Windows `/tmp/test.txt` is relative anyway).
    let result = execute_tool(
        "test-id",
        "file_write",
        &serde_json::json!({"path": "tmp/test.txt", "content": "pwned"}),
        None,
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None, // workspace_root = None
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(
        result.is_error,
        "Expected error when workspace_root is None"
    );
    assert!(
        result.content.contains("Workspace sandbox not configured"),
        "Expected workspace sandbox error, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_file_list_no_workspace_root_returns_error() {
    // SECURITY: file_list must fail when workspace_root is None
    let result = execute_tool(
        "test-id",
        "file_list",
        &serde_json::json!({"path": "/etc"}),
        None,
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None, // workspace_root = None
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(
        result.is_error,
        "Expected error when workspace_root is None"
    );
    assert!(
        result.content.contains("Workspace sandbox not configured"),
        "Expected workspace sandbox error, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_agent_spawn_capability_escalation_denied() {
    // SECURITY: sub-agent cannot request tools the parent doesn't have.
    // Parent only has file_read, but child requests shell_exec.
    let kernel: Arc<dyn KernelHandle> = Arc::new(SpawnCheckKernel {
        should_fail_escalation: true,
    });
    let parent_allowed = vec!["file_read".to_string(), "agent_spawn".to_string()];
    let result = execute_tool(
        "test-id",
        "agent_spawn",
        &serde_json::json!({
            "name": "escalated-child",
            "system_prompt": "You are a test agent.",
            "tools": ["shell_exec", "file_read"]
        }),
        Some(&kernel),
        Some(&parent_allowed),
        Some("parent-agent-id"),
        None,
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(
        result.is_error,
        "Expected escalation to be denied, got: {}",
        result.content
    );
    assert!(
        result.content.contains("escalation") || result.content.contains("denied"),
        "Expected escalation denial message, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_agent_spawn_subset_capabilities_allowed() {
    // Sub-agent requests only capabilities the parent has — should succeed.
    let kernel: Arc<dyn KernelHandle> = Arc::new(SpawnCheckKernel {
        should_fail_escalation: false,
    });
    let parent_allowed = vec![
        "file_read".to_string(),
        "file_write".to_string(),
        "agent_spawn".to_string(),
    ];
    let result = execute_tool(
        "test-id",
        "agent_spawn",
        &serde_json::json!({
            "name": "good-child",
            "system_prompt": "You are a test agent.",
            "tools": ["file_read"]
        }),
        Some(&kernel),
        Some(&parent_allowed),
        Some("parent-agent-id"),
        None,
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(
        !result.is_error,
        "Expected spawn to succeed, got error: {}",
        result.content
    );
    assert!(result.content.contains("spawned successfully"));
}

#[test]
fn test_tools_to_parent_capabilities_expands_resource_caps() {
    use librefang_types::capability::Capability;

    let tools = vec![
        "file_read".to_string(),
        "web_fetch".to_string(),
        "shell_exec".to_string(),
        "agent_spawn".to_string(),
        "memory_store".to_string(),
    ];
    let caps = tools_to_parent_capabilities(&tools);

    // Should have ToolInvoke for each tool name
    assert!(caps.contains(&Capability::ToolInvoke("file_read".into())));
    assert!(caps.contains(&Capability::ToolInvoke("web_fetch".into())));
    assert!(caps.contains(&Capability::ToolInvoke("shell_exec".into())));
    assert!(caps.contains(&Capability::ToolInvoke("agent_spawn".into())));
    assert!(caps.contains(&Capability::ToolInvoke("memory_store".into())));

    // Should also have implied resource-level capabilities
    assert!(
        caps.contains(&Capability::NetConnect("*".into())),
        "web_fetch should imply NetConnect"
    );
    assert!(
        caps.contains(&Capability::ShellExec("*".into())),
        "shell_exec should imply ShellExec"
    );
    assert!(
        caps.contains(&Capability::AgentSpawn),
        "agent_spawn should imply AgentSpawn"
    );
    assert!(
        caps.contains(&Capability::AgentMessage("*".into())),
        "agent_spawn should imply AgentMessage"
    );
    assert!(
        caps.contains(&Capability::MemoryRead("*".into())),
        "memory_store should imply MemoryRead"
    );
    assert!(
        caps.contains(&Capability::MemoryWrite("*".into())),
        "memory_store should imply MemoryWrite"
    );
}

#[test]
fn test_tools_to_parent_capabilities_no_false_expansion() {
    use librefang_types::capability::Capability;

    // Only file_read — should NOT imply any resource caps
    let tools = vec!["file_read".to_string()];
    let caps = tools_to_parent_capabilities(&tools);
    assert_eq!(caps.len(), 1);
    assert!(caps.contains(&Capability::ToolInvoke("file_read".into())));
}

#[tokio::test]
async fn test_mcp_tool_blocked_by_allowed_tools() {
    // SECURITY: MCP tools not in allowed_tools must be blocked.
    let allowed = vec!["file_read".to_string(), "mcp_server1_tool_a".to_string()];
    let result = execute_tool(
        "test-id",
        "mcp_server1_tool_b", // Not in allowed list
        &serde_json::json!({"param": "value"}),
        None,
        Some(&allowed),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(result.is_error);
    assert!(
        result.content.contains("Permission denied"),
        "Expected permission denied for MCP tool, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_mcp_tool_allowed_passes_check() {
    // MCP tool in the allowed list should pass the capability check
    // (may still fail due to no MCP connections, but not permission denied)
    let allowed = vec!["file_read".to_string(), "mcp_myserver_mytool".to_string()];
    let result = execute_tool(
        "test-id",
        "mcp_myserver_mytool", // In allowed list
        &serde_json::json!({"param": "value"}),
        None,
        Some(&allowed),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    // Should fail for "MCP not available", not "Permission denied"
    assert!(result.is_error);
    assert!(
        result.content.contains("MCP not available") || result.content.contains("MCP"),
        "Expected MCP availability error (not permission denied), got: {}",
        result.content
    );
    assert!(
        !result.content.contains("Permission denied"),
        "Should not get permission denied for allowed MCP tool, got: {}",
        result.content
    );
}

// -----------------------------------------------------------------------
// Wildcard allowed_tools tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_allowed_tools_wildcard_prefix_match() {
    // "file_*" should allow file_read
    let allowed = vec!["file_*".to_string()];
    let result = execute_tool(
        "test-id",
        "file_read",
        &serde_json::json!({"path": "/tmp/test.txt"}),
        None,
        Some(&allowed),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    // Should NOT be a permission-denied error
    assert!(
        !result.content.contains("Permission denied"),
        "Wildcard 'file_*' should allow 'file_read', got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_allowed_tools_wildcard_blocks_non_matching() {
    // "file_*" should NOT allow shell_exec
    let allowed = vec!["file_*".to_string()];
    let result = execute_tool(
        "test-id",
        "shell_exec",
        &serde_json::json!({"command": "ls"}),
        None,
        Some(&allowed),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(result.is_error);
    assert!(
        result.content.contains("Permission denied"),
        "Wildcard 'file_*' should block 'shell_exec', got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_allowed_tools_star_allows_everything() {
    // "*" should allow any tool
    let allowed = vec!["*".to_string()];
    let result = execute_tool(
        "test-id",
        "file_read",
        &serde_json::json!({"path": "/tmp/test.txt"}),
        None,
        Some(&allowed),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(
        !result.content.contains("Permission denied"),
        "Wildcard '*' should allow everything, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_allowed_tools_mixed_wildcard_and_exact() {
    // Mix of exact and wildcard entries
    let allowed = vec!["shell_exec".to_string(), "file_*".to_string()];
    let result = execute_tool(
        "test-id",
        "file_write",
        &serde_json::json!({"path": "/tmp/test.txt", "content": "hi"}),
        None,
        Some(&allowed),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(
        !result.content.contains("Permission denied"),
        "Wildcard 'file_*' should allow 'file_write', got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_mcp_tool_wildcard_allowed() {
    // "mcp_*" should allow any MCP tool
    let allowed = vec!["mcp_*".to_string()];
    let result = execute_tool(
        "test-id",
        "mcp_server1_tool_a",
        &serde_json::json!({"param": "value"}),
        None,
        Some(&allowed),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    // Should fail for "MCP not available", not "Permission denied"
    assert!(
        !result.content.contains("Permission denied"),
        "Wildcard 'mcp_*' should allow MCP tools, got: {}",
        result.content
    );
}

// -----------------------------------------------------------------------
// Goal system tests
// -----------------------------------------------------------------------

#[test]
fn test_goal_update_tool_definition_schema() {
    let tools = builtin_tool_definitions();
    let tool = tools
        .iter()
        .find(|t| t.name == "goal_update")
        .expect("goal_update tool should be registered");
    assert_eq!(tool.input_schema["type"], "object");
    let required = tool.input_schema["required"].as_array().unwrap();
    assert!(required.contains(&serde_json::json!("goal_id")));
    let props = tool.input_schema["properties"].as_object().unwrap();
    assert!(props.contains_key("goal_id"));
    assert!(props.contains_key("status"));
    assert!(props.contains_key("progress"));
}

#[test]
fn test_goal_update_missing_kernel() {
    let input = serde_json::json!({
        "goal_id": "some-uuid",
        "status": "in_progress",
        "progress": 50
    });
    let result = tool_goal_update(&input, None);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Kernel handle"));
}

#[test]
fn test_goal_update_missing_goal_id() {
    let input = serde_json::json!({
        "status": "in_progress"
    });
    let result = tool_goal_update(&input, None);
    assert!(result.is_err());
}

#[test]
fn test_goal_update_no_fields() {
    let input = serde_json::json!({
        "goal_id": "some-uuid"
    });
    let result = tool_goal_update(&input, None);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("At least one"));
}

#[test]
fn test_goal_update_invalid_status() {
    let input = serde_json::json!({
        "goal_id": "some-uuid",
        "status": "done"
    });
    let result = tool_goal_update(&input, None);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid status"));
}

/// Mock kernel that validates capability inheritance in spawn_agent_checked.
struct SpawnCheckKernel {
    should_fail_escalation: bool,
}

// ---- BEGIN role-trait impls (split from former `impl KernelHandle for SpawnCheckKernel`, #3746) ----

#[async_trait::async_trait]
impl AgentControl for SpawnCheckKernel {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
        Ok(("test-id-123".to_string(), "test-agent".to_string()))
    }

    async fn spawn_agent_checked(
        &self,
        manifest_toml: &str,
        _parent_id: Option<&str>,
        parent_caps: &[librefang_types::capability::Capability],
    ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
        if self.should_fail_escalation {
            // Parse child manifest to extract capabilities, mimicking real kernel behavior
            let manifest: librefang_types::agent::AgentManifest = toml::from_str(manifest_toml)
                .map_err(|e| {
                    librefang_kernel_handle::KernelOpError::InvalidInput(format!("manifest: {e}"))
                })?;
            let child_caps: Vec<librefang_types::capability::Capability> = manifest
                .capabilities
                .tools
                .iter()
                .map(|t| librefang_types::capability::Capability::ToolInvoke(t.clone()))
                .collect();
            librefang_types::capability::validate_capability_inheritance(parent_caps, &child_caps)
                .map_err(librefang_kernel_handle::KernelOpError::Internal)?;
        }
        Ok(("test-id-456".to_string(), "good-child".to_string()))
    }

    async fn send_to_agent(
        &self,
        _agent_id: &str,
        _message: &str,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

impl MemoryAccess for SpawnCheckKernel {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn memory_recall(
        &self,
        _key: &str,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn memory_list(
        &self,
        _peer_id: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

impl WikiAccess for SpawnCheckKernel {}

#[async_trait::async_trait]
impl TaskQueue for SpawnCheckKernel {
    async fn task_post(
        &self,
        _title: &str,
        _description: &str,
        _assigned_to: Option<&str>,
        _created_by: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_claim(
        &self,
        _agent_id: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_list(
        &self,
        _status: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_delete(
        &self,
        _task_id: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_retry(
        &self,
        _task_id: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_get(
        &self,
        _task_id: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_update_status(
        &self,
        _task_id: &str,
        _new_status: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait::async_trait]
impl EventBus for SpawnCheckKernel {
    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait::async_trait]
impl KnowledgeGraph for SpawnCheckKernel {
    async fn knowledge_add_entity(
        &self,
        _entity: &librefang_types::memory::Entity,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn knowledge_add_relation(
        &self,
        _relation: &librefang_types::memory::Relation,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn knowledge_query(
        &self,
        _pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
    {
        Err("not used".into())
    }
}

// No-op role-trait impls (#3746) — mock relies on default bodies.
impl CronControl for SpawnCheckKernel {}
impl HandsControl for SpawnCheckKernel {}
impl ApprovalGate for SpawnCheckKernel {}
impl A2ARegistry for SpawnCheckKernel {}
impl ChannelSender for SpawnCheckKernel {}
impl PromptStore for SpawnCheckKernel {}
impl WorkflowRunner for SpawnCheckKernel {}
impl GoalControl for SpawnCheckKernel {}
impl ToolPolicy for SpawnCheckKernel {}
impl librefang_kernel_handle::CatalogQuery for SpawnCheckKernel {}
impl ApiAuth for SpawnCheckKernel {
    fn auth_snapshot(&self) -> ApiAuthSnapshot {
        ApiAuthSnapshot::default()
    }
}
impl SessionWriter for SpawnCheckKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}
impl AcpFsBridge for SpawnCheckKernel {}
impl AcpTerminalBridge for SpawnCheckKernel {}

// ---- END role-trait impls (#3746) ----
