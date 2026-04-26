//! End-to-end RBAC M3 (#3054 Phase 2) integration test.
//!
//! Boots a real `LibreFangKernel` (no stubs) with `[[users]]` + per-user
//! `tool_policy` + `[tool_policy.groups]`, then calls
//! `KernelHandle::resolve_user_tool_decision` through the actual trait
//! object. Pins the user-policy + role-escalation contract end-to-end so
//! a future refactor cannot silently fail open.
//!
//! Cases (named in the PR review):
//!   (a) user denies tool but agent allows it via capability → Deny
//!       (Deny short-circuits before agent capability matters; verified
//!        by the user-policy gate returning Deny.)
//!   (b) user allows but agent doesn't have capability → tool_runner
//!       short-circuits on agent capability before consulting the user
//!       gate. Tested separately at the `execute_tool` layer in
//!       `librefang-runtime`; here we cover the kernel-level surface.
//!   (c) both allow → Allow
//!   (d) user policy says NeedsApproval (User role + no allow-list) →
//!       NeedsApproval

use librefang_kernel::LibreFangKernel;
use librefang_runtime::kernel_handle::KernelHandle;
use librefang_types::config::{DefaultModelConfig, KernelConfig, UserConfig};
use librefang_types::tool_policy::{ToolGroup, ToolPolicy};
use librefang_types::user_policy::{UserToolCategories, UserToolGate, UserToolPolicy};
use std::collections::HashMap;

fn boot(users: Vec<UserConfig>, groups: Vec<ToolGroup>) -> std::sync::Arc<LibreFangKernel> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    std::fs::create_dir_all(home.join("data")).unwrap();

    // Leak the tempdir so the path stays alive for the lifetime of the
    // kernel — drop ordering is irrelevant here, the test is short-lived.
    Box::leak(Box::new(tmp));

    let mut tool_policy = ToolPolicy::default();
    tool_policy.groups = groups;

    let config = KernelConfig {
        home_dir: home.clone(),
        data_dir: home.join("data"),
        users,
        tool_policy,
        default_model: DefaultModelConfig {
            provider: "openai".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key_env: "RBAC_M3_TEST_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 60,
            extra_params: HashMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };
    // The default driver only initialises lazily; api_key_env doesn't need
    // to resolve for the kernel to boot. This is enough for our purposes —
    // we never call the LLM.
    std::env::set_var("RBAC_M3_TEST_KEY", "fake-key-for-boot");
    std::sync::Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"))
}

fn user(
    name: &str,
    role: &str,
    platform_id: &str,
    tool_policy: Option<UserToolPolicy>,
    tool_categories: Option<UserToolCategories>,
) -> UserConfig {
    let mut bindings = HashMap::new();
    bindings.insert("telegram".to_string(), platform_id.to_string());
    UserConfig {
        name: name.to_string(),
        role: role.to_string(),
        channel_bindings: bindings,
        api_key_hash: None,
        tool_policy,
        tool_categories,
        memory_access: None,
        channel_tool_rules: HashMap::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn evaluate_tool_call_user_deny_short_circuits() {
    // (a) User has explicit `denied_tools = ["shell_exec"]`. Even though
    // a real agent might list `shell_exec` in its capability allowlist,
    // the kernel-level user gate must hard-deny first.
    let kernel = boot(
        vec![user(
            "Bob",
            "user",
            "111",
            Some(UserToolPolicy {
                allowed_tools: vec![],
                denied_tools: vec!["shell_exec".into()],
            }),
            None,
        )],
        vec![],
    );

    let kh: &dyn KernelHandle = &*kernel;
    let gate = kh.resolve_user_tool_decision("shell_exec", Some("111"), Some("telegram"));
    match gate {
        UserToolGate::Deny { reason } => assert!(
            reason.contains("Bob"),
            "deny reason must surface user name: {reason}"
        ),
        other => panic!("expected Deny, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn evaluate_tool_call_both_allow() {
    // (c) User has explicit allow-list including the tool, role is User.
    // Layer-1 returns Allow — kernel returns Allow.
    let kernel = boot(
        vec![user(
            "Bob",
            "user",
            "111",
            Some(UserToolPolicy {
                allowed_tools: vec!["file_read".into()],
                denied_tools: vec![],
            }),
            None,
        )],
        vec![],
    );

    let kh: &dyn KernelHandle = &*kernel;
    let gate = kh.resolve_user_tool_decision("file_read", Some("111"), Some("telegram"));
    assert_eq!(gate, UserToolGate::Allow);
}

#[tokio::test(flavor = "multi_thread")]
async fn evaluate_tool_call_user_role_no_allow_list_needs_approval() {
    // (d) User role with no per-user policy. Tool isn't in any allow-list,
    // so layer 1 yields NeedsRoleEscalation; user role < admin → NeedsApproval.
    let kernel = boot(vec![user("Bob", "user", "111", None, None)], vec![]);
    let kh: &dyn KernelHandle = &*kernel;
    let gate = kh.resolve_user_tool_decision("shell_exec", Some("111"), Some("telegram"));
    assert!(
        matches!(gate, UserToolGate::NeedsApproval { .. }),
        "User role without allow-list must escalate to NeedsApproval, got {gate:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn evaluate_tool_call_user_categories_resolve_against_kernel_groups() {
    // Bulk-deny by `tool_categories.denied_groups`. The group definitions
    // come from `[tool_policy.groups]` in `KernelConfig`.
    let kernel = boot(
        vec![user(
            "Bob",
            "admin",
            "111",
            None,
            Some(UserToolCategories {
                allowed_groups: vec![],
                denied_groups: vec!["shell_tools".into()],
            }),
        )],
        vec![ToolGroup {
            name: "shell_tools".into(),
            tools: vec!["shell_exec".into(), "shell_run".into()],
        }],
    );

    let kh: &dyn KernelHandle = &*kernel;
    let gate = kh.resolve_user_tool_decision("shell_exec", Some("111"), Some("telegram"));
    assert!(
        matches!(gate, UserToolGate::Deny { .. }),
        "category deny must reach Deny verdict, got {gate:?}"
    );
    // Tool outside the denied group + admin role → Allow (admin self-authorises).
    let other = kh.resolve_user_tool_decision("file_read", Some("111"), Some("telegram"));
    assert_eq!(other, UserToolGate::Allow);
}

#[tokio::test(flavor = "multi_thread")]
async fn evaluate_tool_call_unrecognised_sender_no_longer_fail_open() {
    // RBAC M3 #3054 H7: previously, `sender_id.is_none()` returned Allow
    // unconditionally, allowing anyone whose channel binding wasn't set
    // to bypass RBAC. The new contract makes this fail-CLOSED via the
    // guest gate (read-only safe tools allowed, everything else routed
    // through approval).
    let kernel = boot(vec![user("Alice", "owner", "1", None, None)], vec![]);
    let kh: &dyn KernelHandle = &*kernel;

    // Recognised sender on bound channel — Owner role bypasses the gate.
    let owner = kh.resolve_user_tool_decision("shell_exec", Some("1"), Some("telegram"));
    assert_eq!(owner, UserToolGate::Allow);

    // Unrecognised sender on the same channel falls through to the guest
    // gate — file_read is on the read-only allowlist, shell_exec is not.
    let safe = kh.resolve_user_tool_decision("file_read", Some("guest42"), Some("telegram"));
    assert_eq!(safe, UserToolGate::Allow);
    let unsafe_ = kh.resolve_user_tool_decision("shell_exec", Some("guest42"), Some("telegram"));
    assert!(
        matches!(unsafe_, UserToolGate::NeedsApproval { .. }),
        "unrecognised sender must NOT silently fail-open for shell_exec, got {unsafe_:?}"
    );
}

/// B3 — when the user-policy gate demanded approval and surfaces that
/// to the kernel via `DeferredToolExecution.force_human=true`, the
/// hand-agent auto-approve carve-out MUST NOT fire. We spawn a hand-
/// tagged agent through the public `spawn_agent` API and call
/// `submit_tool_approval` directly via the `KernelHandle` trait.
#[tokio::test(flavor = "multi_thread")]
async fn submit_tool_approval_hand_agent_force_human_skips_auto_approve() {
    use librefang_types::agent::AgentManifest;
    use librefang_types::tool::{DeferredToolExecution, ToolApprovalSubmission};

    let kernel = boot(vec![], vec![]);

    let mut manifest = AgentManifest::default();
    manifest.name = format!("hand-test-{}", uuid::Uuid::new_v4());
    manifest.is_hand = true;
    manifest.tags = vec!["hand:test".to_string()];
    manifest.module = "builtin:chat".to_string();
    let agent_id = kernel.spawn_agent(manifest).expect("spawn hand agent");

    // Sanity: without force_human → AutoApproved.
    let deferred_lax = DeferredToolExecution {
        agent_id: agent_id.0.to_string(),
        tool_use_id: "tu-1".to_string(),
        tool_name: "shell_exec".to_string(),
        input: serde_json::json!({"command": "ls"}),
        allowed_tools: None,
        allowed_env_vars: None,
        exec_policy: None,
        sender_id: None,
        channel: None,
        workspace_root: None,
        force_human: false,
    };
    let kh: &dyn KernelHandle = &*kernel;
    let lax = kh
        .submit_tool_approval(
            &agent_id.0.to_string(),
            "shell_exec",
            "summary",
            deferred_lax,
            None,
        )
        .await
        .expect("submit");
    assert_eq!(
        lax,
        ToolApprovalSubmission::AutoApproved,
        "without force_human a hand-tagged agent must auto-approve"
    );

    // With force_human=true, the carve-out MUST be skipped.
    let deferred_strict = DeferredToolExecution {
        agent_id: agent_id.0.to_string(),
        tool_use_id: "tu-2".to_string(),
        tool_name: "shell_exec".to_string(),
        input: serde_json::json!({"command": "ls"}),
        allowed_tools: None,
        allowed_env_vars: None,
        exec_policy: None,
        sender_id: None,
        channel: None,
        workspace_root: None,
        force_human: true,
    };
    let strict = kh
        .submit_tool_approval(
            &agent_id.0.to_string(),
            "shell_exec",
            "summary",
            deferred_strict,
            None,
        )
        .await
        .expect("submit");
    match strict {
        ToolApprovalSubmission::Pending { .. } => {}
        ToolApprovalSubmission::AutoApproved => {
            panic!("RBAC M3 (#3054): force_human=true on a hand-tagged agent MUST NOT auto-approve")
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn evaluate_tool_call_reload_picks_up_new_policy() {
    // H4 — `[[users]]`/`[tool_policy.groups]` edits via `/api/config/reload`
    // must invalidate the AuthManager. Without `HotAction::ReloadAuth`
    // this regression test would observe the stale policy.
    let kernel = boot(
        vec![user(
            "Bob",
            "user",
            "111",
            Some(UserToolPolicy {
                allowed_tools: vec!["file_read".into()],
                denied_tools: vec![],
            }),
            None,
        )],
        vec![],
    );

    let kh: &dyn KernelHandle = &*kernel;
    // Initial: file_read allowed.
    assert_eq!(
        kh.resolve_user_tool_decision("file_read", Some("111"), Some("telegram")),
        UserToolGate::Allow
    );

    // Reload with file_read denied.
    let new_users = vec![user(
        "Bob",
        "user",
        "111",
        Some(UserToolPolicy {
            allowed_tools: vec![],
            denied_tools: vec!["file_read".into()],
        }),
        None,
    )];
    kernel.auth_manager().reload(&new_users, &[]);

    // After reload: file_read must now be denied.
    let gate = kh.resolve_user_tool_decision("file_read", Some("111"), Some("telegram"));
    assert!(
        matches!(gate, UserToolGate::Deny { .. }),
        "after AuthManager reload the new deny must take effect, got {gate:?}"
    );
}
