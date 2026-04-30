use librefang_kernel_handle::KernelHandle;
use librefang_types::config::UserConfig;
use librefang_types::user_policy::{UserMemoryAccess, UserToolGate, UserToolPolicy};
use std::collections::HashMap;

mod common;

use common::{boot_kernel as boot, boot_kernel_with_users as boot_with_users};

fn telegram_user_with_policy(
    name: &str,
    sender_id: &str,
    tool_policy: Option<UserToolPolicy>,
    memory_access: Option<UserMemoryAccess>,
) -> UserConfig {
    let mut channel_bindings = HashMap::new();
    channel_bindings.insert("telegram".to_string(), sender_id.to_string());

    UserConfig {
        name: name.to_string(),
        role: "user".to_string(),
        channel_bindings,
        tool_policy,
        memory_access,
        ..UserConfig::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_resolve_user_tool_decision_default_allow_for_unconfigured_user() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let gate = kh.resolve_user_tool_decision("any_tool", Some("unknown_user"), Some("telegram"));
    assert_eq!(
        gate,
        UserToolGate::Allow,
        "no registered users must default-allow (guest mode)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_acl_for_sender_default_none_for_unconfigured_user() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let acl = kh.memory_acl_for_sender(Some("user1"), Some("channel"));
    assert!(
        acl.is_none(),
        "no registered users must return None (no per-user restriction)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_resolve_user_tool_decision_uses_sender_and_channel_policy() {
    let (kernel, _tmp) = boot_with_users(vec![telegram_user_with_policy(
        "Bob",
        "111",
        Some(UserToolPolicy {
            allowed_tools: vec![],
            denied_tools: vec!["shell_exec".into()],
        }),
        None,
    )]);
    let kh: &dyn KernelHandle = &kernel;

    let matched = kh.resolve_user_tool_decision("shell_exec", Some("111"), Some("telegram"));
    assert!(
        matches!(matched, UserToolGate::Deny { .. }),
        "bound Telegram sender must receive the configured deny, got {matched:?}"
    );

    let unknown = kh.resolve_user_tool_decision("shell_exec", Some("guest"), Some("telegram"));
    assert!(
        matches!(unknown, UserToolGate::NeedsApproval { .. }),
        "unknown sender must use the guest gate instead of Bob's deny, got {unknown:?}"
    );

    let wrong_channel = kh.resolve_user_tool_decision("shell_exec", Some("111"), Some("discord"));
    assert!(
        matches!(wrong_channel, UserToolGate::NeedsApproval { .. }),
        "same sender id on a different channel must not match Bob's Telegram policy, got {wrong_channel:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_acl_for_sender_uses_sender_and_channel_binding() {
    let acl = UserMemoryAccess {
        readable_namespaces: vec!["kv:bob".to_string()],
        writable_namespaces: vec![],
        pii_access: true,
        export_allowed: false,
        delete_allowed: false,
    };
    let (kernel, _tmp) = boot_with_users(vec![telegram_user_with_policy(
        "Bob",
        "111",
        None,
        Some(acl.clone()),
    )]);
    let kh: &dyn KernelHandle = &kernel;

    assert_eq!(
        kh.memory_acl_for_sender(Some("111"), Some("telegram")),
        Some(acl),
        "bound Telegram sender must receive configured memory ACL"
    );
    assert!(
        kh.memory_acl_for_sender(Some("guest"), Some("telegram"))
            .is_none(),
        "unknown sender must not receive Bob's memory ACL"
    );
    assert!(
        kh.memory_acl_for_sender(Some("111"), Some("discord"))
            .is_none(),
        "same sender id on a different channel must not receive Bob's memory ACL"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_requires_approval_with_context_delegates_to_requires_approval() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let with_ctx = kh.requires_approval_with_context("tool", Some("sender"), Some("channel"));
    let plain = kh.requires_approval("tool");
    assert_eq!(
        with_ctx, plain,
        "requires_approval_with_context must delegate to requires_approval"
    );
    assert!(!plain, "default config must not require approval");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_is_tool_denied_with_context_default_false() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let denied = kh.is_tool_denied_with_context("any_tool", Some("sender"), Some("channel"));
    assert!(!denied, "default config must not deny any tool");
}
