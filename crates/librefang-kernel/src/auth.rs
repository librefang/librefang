//! RBAC authentication and authorization for multi-user access control.
//!
//! The AuthManager maps platform user identities (Telegram ID, Discord ID, etc.)
//! to LibreFang users with roles, then enforces permission checks on actions.

use dashmap::DashMap;
use librefang_types::agent::UserId;
use librefang_types::config::UserConfig;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::tool_policy::ToolGroup;
use librefang_types::user_policy::{
    ResolvedUserPolicy, UserMemoryAccess, UserToolDecision, UserToolGate,
};
use std::fmt;
use tracing::{debug, info};

/// User roles with hierarchical permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UserRole {
    /// Read-only access — can view agent output but cannot interact.
    Viewer = 0,
    /// Standard user — can chat with agents.
    User = 1,
    /// Admin — can spawn/kill agents, install skills, view usage.
    Admin = 2,
    /// Owner — full access including user management and config changes.
    Owner = 3,
}

impl fmt::Display for UserRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UserRole::Viewer => write!(f, "viewer"),
            UserRole::User => write!(f, "user"),
            UserRole::Admin => write!(f, "admin"),
            UserRole::Owner => write!(f, "owner"),
        }
    }
}

impl UserRole {
    /// Parse a role from a string.
    pub fn from_str_role(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "owner" => UserRole::Owner,
            "admin" => UserRole::Admin,
            "viewer" => UserRole::Viewer,
            _ => UserRole::User,
        }
    }
}

/// Actions that can be authorized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Chat with an agent.
    ChatWithAgent,
    /// Spawn a new agent.
    SpawnAgent,
    /// Kill a running agent.
    KillAgent,
    /// Install a skill.
    InstallSkill,
    /// View kernel configuration.
    ViewConfig,
    /// Modify kernel configuration.
    ModifyConfig,
    /// View usage/billing data.
    ViewUsage,
    /// Manage users (create, delete, change roles).
    ManageUsers,
}

impl Action {
    /// Minimum role required for this action.
    fn required_role(&self) -> UserRole {
        match self {
            Action::ChatWithAgent => UserRole::User,
            Action::ViewConfig => UserRole::User,
            Action::ViewUsage => UserRole::Admin,
            Action::SpawnAgent => UserRole::Admin,
            Action::KillAgent => UserRole::Admin,
            Action::InstallSkill => UserRole::Admin,
            Action::ModifyConfig => UserRole::Owner,
            Action::ManageUsers => UserRole::Owner,
        }
    }
}

/// A resolved user identity.
#[derive(Debug, Clone)]
pub struct UserIdentity {
    /// LibreFang user ID.
    pub id: UserId,
    /// Display name.
    pub name: String,
    /// Role.
    pub role: UserRole,
    /// Resolved per-user RBAC policy (RBAC M3). Built once at config-load
    /// from `UserConfig.{tool_policy,tool_categories,memory_access,
    /// channel_tool_rules}`. Defaults to `ResolvedUserPolicy::default()`
    /// when no per-user policy was declared.
    pub policy: ResolvedUserPolicy,
}

/// RBAC authentication and authorization manager.
pub struct AuthManager {
    /// Known users by their LibreFang user ID.
    users: DashMap<UserId, UserIdentity>,
    /// Channel binding index: "channel_type:platform_id" → UserId.
    channel_index: DashMap<String, UserId>,
    /// Sender-id index: any platform-id string in any channel binding maps
    /// to the owning UserId. Used by the runtime tool dispatcher to
    /// resolve a `sender_id` (which doesn't carry a channel prefix) to a
    /// known user. First write wins on collision.
    sender_index: DashMap<String, UserId>,
    /// Tool groups (categories) referenced by per-user policies. Cloned
    /// from `KernelConfig.tool_policy.groups` at construction.
    tool_groups: Vec<ToolGroup>,
}

impl AuthManager {
    /// Create a new AuthManager from kernel user configuration.
    ///
    /// Equivalent to `with_tool_groups(user_configs, &[])` — kept for
    /// existing callers / tests.
    pub fn new(user_configs: &[UserConfig]) -> Self {
        Self::with_tool_groups(user_configs, &[])
    }

    /// Create a new AuthManager with knowledge of the kernel's
    /// `ToolPolicy.groups` so per-user `tool_categories` can resolve
    /// group names to their tool patterns.
    pub fn with_tool_groups(user_configs: &[UserConfig], tool_groups: &[ToolGroup]) -> Self {
        let manager = Self {
            users: DashMap::new(),
            channel_index: DashMap::new(),
            sender_index: DashMap::new(),
            tool_groups: tool_groups.to_vec(),
        };

        for config in user_configs {
            let user_id = UserId::from_name(&config.name);
            let role = UserRole::from_str_role(&config.role);

            // Build the per-user policy snapshot. Optional fields fall
            // back to default (no opinion) so `evaluate` returns
            // NeedsRoleEscalation everywhere — i.e. existing behaviour.
            let policy = ResolvedUserPolicy {
                tool_policy: config.tool_policy.clone().unwrap_or_default(),
                channel_tool_rules: config.channel_tool_rules.clone(),
                tool_categories: config.tool_categories.clone().unwrap_or_default(),
                memory_access: config.memory_access.clone().unwrap_or_default(),
            };

            let identity = UserIdentity {
                id: user_id,
                name: config.name.clone(),
                role,
                policy,
            };

            manager.users.insert(user_id, identity);

            // Index channel bindings
            for (channel_type, platform_id) in &config.channel_bindings {
                let key = format!("{channel_type}:{platform_id}");
                manager.channel_index.insert(key, user_id);
                // Also expose the bare platform id as a sender-id alias
                // so the runtime can resolve users when a `sender_id`
                // arrives without a channel-type prefix.
                manager
                    .sender_index
                    .entry(platform_id.clone())
                    .or_insert(user_id);
            }

            info!(
                user = %config.name,
                role = %role,
                bindings = config.channel_bindings.len(),
                "Registered user"
            );
        }

        manager
    }

    /// Identify a user from a channel identity.
    ///
    /// Returns the LibreFang UserId if a matching channel binding exists,
    /// or None for unrecognized users.
    pub fn identify(&self, channel_type: &str, platform_id: &str) -> Option<UserId> {
        let key = format!("{channel_type}:{platform_id}");
        self.channel_index.get(&key).map(|r| *r.value())
    }

    /// Get a user's identity by their UserId.
    pub fn get_user(&self, user_id: UserId) -> Option<UserIdentity> {
        self.users.get(&user_id).map(|r| r.value().clone())
    }

    /// Authorize a user for an action.
    ///
    /// Returns Ok(()) if the user has sufficient permissions, or AuthDenied error.
    pub fn authorize(&self, user_id: UserId, action: &Action) -> LibreFangResult<()> {
        let identity = self
            .users
            .get(&user_id)
            .ok_or_else(|| LibreFangError::AuthDenied("Unknown user".to_string()))?;

        let required = action.required_role();
        if identity.role >= required {
            Ok(())
        } else {
            Err(LibreFangError::AuthDenied(format!(
                "User '{}' (role: {}) lacks permission for {:?} (requires: {})",
                identity.name, identity.role, action, required
            )))
        }
    }

    /// Check if RBAC is configured (any users registered).
    pub fn is_enabled(&self) -> bool {
        !self.users.is_empty()
    }

    /// Get the count of registered users.
    pub fn user_count(&self) -> usize {
        self.users.len()
    }

    /// List all registered users.
    pub fn list_users(&self) -> Vec<UserIdentity> {
        self.users.iter().map(|r| r.value().clone()).collect()
    }

    /// Resolve a `sender_id` and `channel` pair to a known user, if any.
    ///
    /// Resolution order:
    /// 1. Channel bindings (`channel:sender_id` exact key)
    /// 2. Sender-only fallback (any user whose channel_bindings contain
    ///    this platform_id verbatim)
    pub fn resolve_user(&self, sender_id: Option<&str>, channel: Option<&str>) -> Option<UserId> {
        if let (Some(ch), Some(sid)) = (channel, sender_id) {
            let key = format!("{ch}:{sid}");
            if let Some(uid) = self.channel_index.get(&key) {
                return Some(*uid);
            }
        }
        if let Some(sid) = sender_id {
            if let Some(uid) = self.sender_index.get(sid) {
                return Some(*uid);
            }
        }
        None
    }

    /// Reference to the kernel's tool groups (used for per-user category
    /// evaluation). Borrows the cached `Vec`; not cheap to keep across
    /// awaits — clone if needed.
    pub fn tool_groups(&self) -> &[ToolGroup] {
        &self.tool_groups
    }

    /// Get the resolved per-user RBAC policy for a user, if registered.
    pub fn user_policy(&self, user_id: UserId) -> Option<ResolvedUserPolicy> {
        self.users.get(&user_id).map(|r| r.value().policy.clone())
    }

    /// Get the memory namespace ACL for a user (if registered) merged
    /// with the role default. Returns the role-default ACL when the user
    /// has no registered customisation (`is_unconfigured`).
    pub fn memory_acl_for(&self, user_id: UserId) -> Option<UserMemoryAccess> {
        let identity = self.users.get(&user_id)?;
        let acl = &identity.value().policy.memory_access;
        if acl.is_unconfigured() {
            Some(default_memory_acl(identity.value().role))
        } else {
            Some(acl.clone())
        }
    }

    /// Resolve the runtime-facing tool gate for a sender + channel pair.
    ///
    /// See [`KernelHandle::resolve_user_tool_decision`] for the contract.
    /// This is the kernel-side implementation; the trait method is a
    /// thin wrapper that calls into here.
    pub fn resolve_user_tool_decision(
        &self,
        tool_name: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> UserToolGate {
        // No registered users → guest mode (default-allow with minimal
        // perms — design decision #2). The runtime keeps its existing
        // approval/capability gates.
        if self.users.is_empty() {
            return UserToolGate::Allow;
        }

        let Some(user_id) = self.resolve_user(sender_id, channel) else {
            // RBAC is enabled but the sender isn't recognised. Treat as
            // guest: hard-deny for tools that require a higher role,
            // pass-through otherwise. The role used here is `Viewer`
            // (the most restrictive) — this matches the principle of
            // least surprise without breaking system-internal calls
            // where sender_id is `None`.
            if sender_id.is_none() {
                return UserToolGate::Allow;
            }
            return guest_gate(tool_name);
        };

        let Some(identity) = self.get_user(user_id) else {
            return UserToolGate::Allow;
        };

        // Layer A — apply the user's own policy.
        let user_decision = identity
            .policy
            .evaluate(tool_name, channel, &self.tool_groups);

        match user_decision {
            UserToolDecision::Allow => UserToolGate::Allow,
            UserToolDecision::Deny => UserToolGate::Deny {
                reason: format!(
                    "user '{}' (role: {}) is not permitted to invoke '{}'",
                    identity.name, identity.role, tool_name
                ),
            },
            UserToolDecision::NeedsRoleEscalation => {
                // Layer B — would an admin/owner have allowed it?
                // Owner is the highest role; if their evaluation returns
                // anything other than Deny we escalate to approval. Otherwise
                // hard-deny.
                if identity.role >= UserRole::Admin {
                    // Admins can self-authorise — the existing approval
                    // gate already handles them.
                    UserToolGate::Allow
                } else {
                    debug!(
                        user = %identity.name,
                        tool = tool_name,
                        "User policy escalating to approval (admin would have permitted)"
                    );
                    UserToolGate::NeedsApproval {
                        reason: format!(
                            "tool '{}' requires admin approval for user '{}' (role: {})",
                            tool_name, identity.name, identity.role
                        ),
                    }
                }
            }
        }
    }
}

/// Default memory ACL for a role when the user did not declare one
/// explicitly. Conservative — viewers get nothing, owners get everything.
fn default_memory_acl(role: UserRole) -> UserMemoryAccess {
    match role {
        UserRole::Owner | UserRole::Admin => UserMemoryAccess {
            readable_namespaces: vec!["*".into()],
            writable_namespaces: vec!["*".into()],
            pii_access: true,
            export_allowed: true,
            delete_allowed: true,
        },
        UserRole::User => UserMemoryAccess {
            readable_namespaces: vec!["proactive".into(), "kv:*".into()],
            writable_namespaces: vec!["kv:*".into()],
            pii_access: false,
            export_allowed: false,
            delete_allowed: false,
        },
        UserRole::Viewer => UserMemoryAccess {
            readable_namespaces: vec!["proactive".into()],
            writable_namespaces: vec![],
            pii_access: false,
            export_allowed: false,
            delete_allowed: false,
        },
    }
}

/// Gate decision for an unrecognised sender. Mirrors design decision #2
/// (default-allow with minimal perms): allow well-known read-only tools,
/// require approval for anything else.
fn guest_gate(tool_name: &str) -> UserToolGate {
    const READ_ONLY_TOOLS: &[&str] = &[
        "file_read",
        "file_list",
        "glob",
        "grep",
        "web_search",
        "web_fetch",
        "list_agents",
        "list_skills",
        "tool_load",
        "tool_search",
    ];
    if READ_ONLY_TOOLS.contains(&tool_name) {
        UserToolGate::Allow
    } else {
        UserToolGate::NeedsApproval {
            reason: format!("tool '{tool_name}' is not allowed for unrecognised senders"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_configs() -> Vec<UserConfig> {
        vec![
            UserConfig {
                name: "Alice".to_string(),
                role: "owner".to_string(),
                channel_bindings: {
                    let mut m = HashMap::new();
                    m.insert("telegram".to_string(), "123456".to_string());
                    m.insert("discord".to_string(), "987654".to_string());
                    m
                },
                api_key_hash: None,
                tool_policy: None,
                tool_categories: None,
                memory_access: None,
                channel_tool_rules: HashMap::new(),
            },
            UserConfig {
                name: "Guest".to_string(),
                role: "user".to_string(),
                channel_bindings: {
                    let mut m = HashMap::new();
                    m.insert("telegram".to_string(), "999999".to_string());
                    m
                },
                api_key_hash: None,
                tool_policy: None,
                tool_categories: None,
                memory_access: None,
                channel_tool_rules: HashMap::new(),
            },
            UserConfig {
                name: "ReadOnly".to_string(),
                role: "viewer".to_string(),
                channel_bindings: HashMap::new(),
                api_key_hash: None,
                tool_policy: None,
                tool_categories: None,
                memory_access: None,
                channel_tool_rules: HashMap::new(),
            },
        ]
    }

    #[test]
    fn test_user_registration() {
        let manager = AuthManager::new(&test_configs());
        assert!(manager.is_enabled());
        assert_eq!(manager.user_count(), 3);
    }

    #[test]
    fn test_identify_from_channel() {
        let manager = AuthManager::new(&test_configs());

        // Alice on Telegram
        let owner_tg = manager.identify("telegram", "123456");
        assert!(owner_tg.is_some());

        // Alice on Discord
        let owner_dc = manager.identify("discord", "987654");
        assert!(owner_dc.is_some());

        // Same user across channels
        assert_eq!(owner_tg.unwrap(), owner_dc.unwrap());

        // Unknown user
        assert!(manager.identify("telegram", "unknown").is_none());
    }

    #[test]
    fn test_owner_can_do_everything() {
        let manager = AuthManager::new(&test_configs());
        let owner_id = manager.identify("telegram", "123456").unwrap();

        assert!(manager.authorize(owner_id, &Action::ChatWithAgent).is_ok());
        assert!(manager.authorize(owner_id, &Action::SpawnAgent).is_ok());
        assert!(manager.authorize(owner_id, &Action::KillAgent).is_ok());
        assert!(manager.authorize(owner_id, &Action::ManageUsers).is_ok());
        assert!(manager.authorize(owner_id, &Action::ModifyConfig).is_ok());
    }

    #[test]
    fn test_user_limited_access() {
        let manager = AuthManager::new(&test_configs());
        let guest_id = manager.identify("telegram", "999999").unwrap();

        // User can chat and view config
        assert!(manager.authorize(guest_id, &Action::ChatWithAgent).is_ok());
        assert!(manager.authorize(guest_id, &Action::ViewConfig).is_ok());

        // User cannot spawn/kill/manage
        assert!(manager.authorize(guest_id, &Action::SpawnAgent).is_err());
        assert!(manager.authorize(guest_id, &Action::KillAgent).is_err());
        assert!(manager.authorize(guest_id, &Action::ManageUsers).is_err());
    }

    #[test]
    fn test_viewer_read_only() {
        let manager = AuthManager::new(&test_configs());
        let users = manager.list_users();
        let viewer = users.iter().find(|u| u.name == "ReadOnly").unwrap();

        // Viewer cannot even chat
        assert!(manager
            .authorize(viewer.id, &Action::ChatWithAgent)
            .is_err());
    }

    #[test]
    fn test_unknown_user_denied() {
        let manager = AuthManager::new(&test_configs());
        let fake_id = UserId::new();
        assert!(manager.authorize(fake_id, &Action::ChatWithAgent).is_err());
    }

    #[test]
    fn test_no_users_means_disabled() {
        let manager = AuthManager::new(&[]);
        assert!(!manager.is_enabled());
        assert_eq!(manager.user_count(), 0);
    }

    #[test]
    fn test_role_parsing() {
        assert_eq!(UserRole::from_str_role("owner"), UserRole::Owner);
        assert_eq!(UserRole::from_str_role("admin"), UserRole::Admin);
        assert_eq!(UserRole::from_str_role("viewer"), UserRole::Viewer);
        assert_eq!(UserRole::from_str_role("user"), UserRole::User);
        assert_eq!(UserRole::from_str_role("OWNER"), UserRole::Owner);
        assert_eq!(UserRole::from_str_role("unknown"), UserRole::User);
    }

    #[test]
    fn test_user_ids_stable_across_manager_rebuilds() {
        // RBAC M1: AuthManager now derives ids via UserId::from_name so
        // restarting the daemon (or rebuilding the manager from the same
        // config) keeps audit-log attribution intact. Random v4 ids would
        // break correlation on every boot.
        let cfg = test_configs();
        let m1 = AuthManager::new(&cfg);
        let m2 = AuthManager::new(&cfg);

        let alice1 = m1.identify("telegram", "123456").unwrap();
        let alice2 = m2.identify("telegram", "123456").unwrap();
        assert_eq!(alice1, alice2, "same name must map to the same UserId");

        // The id is also discoverable directly from the configured name —
        // this is the contract the API-key path in middleware.rs depends on.
        assert_eq!(alice1, UserId::from_name("Alice"));
    }

    #[test]
    fn test_distinct_users_get_distinct_ids() {
        let manager = AuthManager::new(&test_configs());
        let alice = manager.identify("telegram", "123456").unwrap();
        let guest = manager.identify("telegram", "999999").unwrap();
        assert_ne!(alice, guest);
    }

    // ----- RBAC M3 — per-user tool policy resolution -----

    use librefang_types::user_policy::{
        ChannelToolPolicy, UserMemoryAccess, UserToolCategories, UserToolGate, UserToolPolicy,
    };

    fn user_with_policy(
        name: &str,
        role: &str,
        platform_id: &str,
        tool_policy: Option<UserToolPolicy>,
        tool_categories: Option<UserToolCategories>,
        memory_access: Option<UserMemoryAccess>,
        channel_tool_rules: HashMap<String, ChannelToolPolicy>,
    ) -> UserConfig {
        UserConfig {
            name: name.to_string(),
            role: role.to_string(),
            channel_bindings: {
                let mut m = HashMap::new();
                m.insert("telegram".to_string(), platform_id.to_string());
                m
            },
            api_key_hash: None,
            tool_policy,
            tool_categories,
            memory_access,
            channel_tool_rules,
        }
    }

    #[test]
    fn rbac_m3_tool_policy_user_deny_yields_hard_deny() {
        let bob = user_with_policy(
            "Bob",
            "user",
            "111",
            Some(UserToolPolicy {
                allowed_tools: vec![],
                denied_tools: vec!["shell_exec".into()],
            }),
            None,
            None,
            HashMap::new(),
        );
        let mgr = AuthManager::with_tool_groups(&[bob], &[]);
        let gate = mgr.resolve_user_tool_decision("shell_exec", Some("111"), Some("telegram"));
        match gate {
            UserToolGate::Deny { reason } => assert!(reason.contains("Bob")),
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn rbac_m3_user_role_no_policy_escalates_unknown_tools_to_approval() {
        // A regular user with no per-user policy. Tool isn't in allow-list,
        // so layer 1 yields NeedsRoleEscalation; user role < admin →
        // NeedsApproval.
        let bob = user_with_policy("Bob", "user", "111", None, None, None, HashMap::new());
        let mgr = AuthManager::with_tool_groups(&[bob], &[]);
        let gate = mgr.resolve_user_tool_decision("shell_exec", Some("111"), Some("telegram"));
        assert!(matches!(gate, UserToolGate::NeedsApproval { .. }));
    }

    #[test]
    fn rbac_m3_admin_role_passes_through_unconfigured() {
        let admin = user_with_policy("Admin", "admin", "999", None, None, None, HashMap::new());
        let mgr = AuthManager::with_tool_groups(&[admin], &[]);
        let gate = mgr.resolve_user_tool_decision("shell_exec", Some("999"), Some("telegram"));
        assert_eq!(gate, UserToolGate::Allow);
    }

    #[test]
    fn rbac_m3_channel_rule_precedence_over_default() {
        let mut rules = HashMap::new();
        rules.insert(
            "telegram".to_string(),
            ChannelToolPolicy {
                allowed_tools: vec![],
                denied_tools: vec!["shell_exec".into()],
            },
        );
        let bob = user_with_policy("Bob", "admin", "111", None, None, None, rules);
        let mgr = AuthManager::with_tool_groups(&[bob], &[]);

        // From telegram → channel rule denies even though admin role would.
        let from_tg = mgr.resolve_user_tool_decision("shell_exec", Some("111"), Some("telegram"));
        assert!(matches!(from_tg, UserToolGate::Deny { .. }));

        // From a different channel → no rule, admin role allows.
        let from_dc = mgr.resolve_user_tool_decision("shell_exec", Some("111"), Some("discord"));
        assert_eq!(from_dc, UserToolGate::Allow);
    }

    #[test]
    fn rbac_m3_categories_resolve_against_kernel_groups() {
        let groups = vec![ToolGroup {
            name: "shell_tools".into(),
            tools: vec!["shell_exec".into(), "shell_run".into()],
        }];
        let bob = user_with_policy(
            "Bob",
            "admin",
            "111",
            None,
            Some(UserToolCategories {
                allowed_groups: vec![],
                denied_groups: vec!["shell_tools".into()],
            }),
            None,
            HashMap::new(),
        );
        let mgr = AuthManager::with_tool_groups(&[bob], &groups);
        let gate = mgr.resolve_user_tool_decision("shell_exec", Some("111"), Some("telegram"));
        assert!(matches!(gate, UserToolGate::Deny { .. }));
        // Tool outside the denied group is fine.
        let ok = mgr.resolve_user_tool_decision("file_read", Some("111"), Some("telegram"));
        assert_eq!(ok, UserToolGate::Allow);
    }

    #[test]
    fn rbac_m3_unknown_sender_falls_through_to_guest_gate() {
        let mgr = AuthManager::with_tool_groups(
            &[user_with_policy(
                "Alice",
                "owner",
                "1",
                None,
                None,
                None,
                HashMap::new(),
            )],
            &[],
        );
        let safe = mgr.resolve_user_tool_decision("file_read", Some("guest42"), Some("telegram"));
        assert_eq!(safe, UserToolGate::Allow);
        let unsafe_ =
            mgr.resolve_user_tool_decision("shell_exec", Some("guest42"), Some("telegram"));
        assert!(matches!(unsafe_, UserToolGate::NeedsApproval { .. }));
    }

    #[test]
    fn rbac_m3_no_users_keeps_legacy_behaviour() {
        let mgr = AuthManager::with_tool_groups(&[], &[]);
        // No registered users → guest mode (default-allow with minimal
        // perms). Existing approval gates take over.
        assert_eq!(
            mgr.resolve_user_tool_decision("shell_exec", Some("anyone"), Some("telegram")),
            UserToolGate::Allow
        );
    }

    #[test]
    fn rbac_m3_memory_acl_falls_back_to_role_default() {
        let viewer = user_with_policy(
            "Viewer",
            "viewer",
            "501",
            None,
            None,
            None, // unconfigured
            HashMap::new(),
        );
        let mgr = AuthManager::with_tool_groups(&[viewer], &[]);
        let viewer_id = mgr.identify("telegram", "501").unwrap();
        let acl = mgr.memory_acl_for(viewer_id).unwrap();
        // Role-default for viewer: read proactive only, no PII, no writes.
        assert!(acl.can_read("proactive"));
        assert!(!acl.can_read("kv:secrets"));
        assert!(!acl.pii_access);
        assert!(acl.writable_namespaces.is_empty());
    }

    #[test]
    fn rbac_m3_memory_acl_user_override_wins() {
        let user = user_with_policy(
            "Bob",
            "user",
            "777",
            None,
            None,
            Some(UserMemoryAccess {
                readable_namespaces: vec!["shared".into()],
                writable_namespaces: vec!["kv:bob_*".into()],
                pii_access: true,
                export_allowed: false,
                delete_allowed: true,
            }),
            HashMap::new(),
        );
        let mgr = AuthManager::with_tool_groups(&[user], &[]);
        let id = mgr.identify("telegram", "777").unwrap();
        let acl = mgr.memory_acl_for(id).unwrap();
        assert!(acl.can_read("shared"));
        assert!(!acl.can_read("proactive"));
        assert!(acl.can_write("kv:bob_inbox"));
        assert!(acl.pii_access);
        assert!(acl.delete_allowed);
        assert!(!acl.export_allowed);
    }
}
