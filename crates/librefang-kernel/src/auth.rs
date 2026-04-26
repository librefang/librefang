//! RBAC authentication and authorization for multi-user access control.
//!
//! The AuthManager maps platform user identities (Telegram ID, Discord ID, etc.)
//! to LibreFang users with roles, then enforces permission checks on actions.

use dashmap::DashMap;
use librefang_channels::types::{ChannelRoleQuery, SenderContext};
use librefang_types::agent::UserId;
use librefang_types::config::{ChannelRoleMapping, UserConfig};
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::tool_policy::ToolGroup;
use librefang_types::user_policy::{
    ResolvedUserPolicy, UserMemoryAccess, UserToolDecision, UserToolGate,
};
use std::fmt;
use tracing::{debug, info, warn};

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
    ///
    /// Accepts `owner` / `admin` / `user` / `viewer`. The synonym `guest`
    /// maps to `Viewer` so that operators using the RBAC-M4 channel-role
    /// mapping vocabulary (`guest_role = "guest"`) get a sensible
    /// default-deny floor without having to learn the legacy name.
    /// Unknown strings fall through to `User` — lenient on the
    /// `UserConfig.role` boot path because a typo there is visible to the
    /// operator (audit + dashboard show `User`). Channel-mapping translators
    /// MUST use [`UserRole::try_from_str_role`] instead so a typo in
    /// `[channel_role_mapping]` fails closed to `Viewer`.
    ///
    /// **Behavior change in M4 (#3054):** the literal string `"guest"`
    /// used to fall through the `_` arm and resolve to `User`; it now
    /// resolves to `Viewer`. Operators with `[users.x] role = "guest"`
    /// in a deployed `config.toml` will see that user demoted to read-
    /// only on upgrade. This is intentional — `"guest"` was always a
    /// misnomer that produced the wrong privilege level.
    pub fn from_str_role(s: &str) -> Self {
        Self::try_from_str_role(s).unwrap_or(UserRole::User)
    }

    /// Strict variant: returns `None` for any unrecognized role string. Used
    /// by the channel-role-mapping translators so a typo (e.g. `admn` or
    /// `creator_role = "ower"`) does not silently become `User` privilege.
    /// The resolver falls through to `Viewer` when the translator returns
    /// `None`, preserving the design's default-deny floor.
    pub fn try_from_str_role(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "owner" => Some(UserRole::Owner),
            "admin" => Some(UserRole::Admin),
            "user" => Some(UserRole::User),
            "viewer" | "guest" => Some(UserRole::Viewer),
            _ => None,
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

/// Cache key for resolved channel roles.
///
/// Scoped per (channel, account, chat, user) so that:
/// - The same Telegram user gets distinct cache entries for two different
///   group chats — they can be admin in one and a regular member in the
///   other.
/// - Multi-bot deployments (`account_id`) keep separate caches, since
///   different bots may have different visibility into a chat.
///
/// Slack ignores `chat_id` at the platform layer (workspace-scoped roles)
/// but we keep it in the key for uniformity — every cache hit/miss costs
/// the same regardless.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RoleCacheKey {
    channel: String,
    account_id: String,
    chat_id: String,
    user_id: String,
}

impl RoleCacheKey {
    fn from_sender(sender: &SenderContext) -> Self {
        Self {
            channel: sender.channel.clone(),
            account_id: sender.account_id.clone().unwrap_or_default(),
            chat_id: sender.chat_id.clone().unwrap_or_default(),
            user_id: sender.user_id.clone(),
        }
    }
}

/// RBAC authentication and authorization manager.
pub struct AuthManager {
    /// Known users by their LibreFang user ID.
    users: DashMap<UserId, UserIdentity>,
    /// Channel binding index: "channel_type:platform_id" → UserId.
    channel_index: DashMap<String, UserId>,
    /// Resolved channel-role cache: `(channel, account, chat, user) → UserRole`.
    /// Populated lazily by [`AuthManager::resolve_role_for_sender`]; the
    /// design contract is that the cache lives for the session's lifetime
    /// and is invalidated on session restart via [`AuthManager::invalidate_role_cache`].
    role_cache: DashMap<RoleCacheKey, UserRole>,
    /// Tool groups (categories) referenced by per-user policies. Cloned
    /// from `KernelConfig.tool_policy.groups` at construction.
    /// `RwLock<Arc<…>>` so `config_reload` can swap the snapshot in
    /// place while resolution-path readers (`tool_groups()`) only pay
    /// for an `Arc::clone` instead of a per-call `Vec` clone.
    tool_groups: std::sync::RwLock<std::sync::Arc<Vec<ToolGroup>>>,
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
            role_cache: DashMap::new(),
            tool_groups: std::sync::RwLock::new(std::sync::Arc::new(tool_groups.to_vec())),
        };
        manager.populate(user_configs);
        manager
    }

    fn populate(&self, user_configs: &[UserConfig]) {
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

            self.users.insert(user_id, identity);

            // Index channel bindings. Only the explicit (channel_type,
            // platform_id) tuple is registered — there is **no** bare
            // `platform_id` fallback. RBAC M3 (#3054) closes the cross-
            // channel attribution leak where two users sharing the same
            // platform-id on different channels would alias to whichever
            // was registered first, with the worst case granting Owner
            // rights to an unrelated inbound on a third channel.
            for (channel_type, platform_id) in &config.channel_bindings {
                let key = format!("{channel_type}:{platform_id}");
                self.channel_index.insert(key, user_id);
            }

            info!(
                user = %config.name,
                role = %role,
                bindings = config.channel_bindings.len(),
                "Registered user"
            );
        }
    }

    /// Replace the in-memory user/channel indexes from a fresh
    /// `KernelConfig`. Used by the config hot-reload path
    /// (`HotAction::ReloadAuth`) so policy edits to `[[users]]`,
    /// `[users.tool_policy]`, and `[tool_policy.groups]` take effect
    /// without a daemon restart.
    ///
    /// This is intentionally a "stop-the-world" replace inside the
    /// `config_reload_lock` write guard — concurrent `identify`/
    /// `resolve_user_tool_decision` calls will observe a clean snapshot
    /// either before or after the swap, never a torn one.
    pub fn reload(&self, user_configs: &[UserConfig], tool_groups: &[ToolGroup]) {
        self.users.clear();
        self.channel_index.clear();
        // Panic on a poisoned lock: silently keeping the stale snapshot
        // would mean `/api/config/reload` reports success while the new
        // `[tool_policy.groups]` are never enforced — exactly the
        // failure mode `HotAction::ReloadAuth` exists to prevent.
        *self
            .tool_groups
            .write()
            .expect("AuthManager.tool_groups RwLock poisoned during reload") =
            std::sync::Arc::new(tool_groups.to_vec());
        self.populate(user_configs);
        info!(
            users = self.users.len(),
            tool_groups = tool_groups.len(),
            "AuthManager reloaded from config"
        );
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

    /// Resolve the effective LibreFang role for a sender.
    ///
    /// Precedence (RBAC M4 design decision §4):
    /// 1. **Explicit `UserConfig.role`** — wins outright when the sender is
    ///    bound to a registered user (`channel_bindings`). The platform
    ///    role is not even queried.
    /// 2. **Channel-derived role** — when no explicit role exists, query
    ///    the platform via `role_query` and translate via `mapping`.
    /// 3. **Default-deny** — fall through to [`UserRole::Viewer`] (the
    ///    minimum privilege; cannot chat by default).
    ///
    /// The result is cached per (channel, account, chat, user) for the
    /// session lifetime; subsequent calls do not re-hit the platform API.
    /// Caches are cleared by [`AuthManager::invalidate_role_cache`] (called
    /// on session restart).
    ///
    /// Returns `UserRole::Viewer` on any platform error so a flaky external
    /// API can never accidentally elevate privileges (fail-closed).
    /// **Transient platform errors are NOT cached** — the next call
    /// re-queries the platform so a momentary 5xx / timeout doesn't
    /// lock the user out for the rest of the session. Only definitive
    /// outcomes (`Ok(Some)` translated, `Ok(None)`, no-translator-
    /// configured) populate the cache.
    ///
    /// **Status:** public surface added in M4 (RBAC #3054); production
    /// wiring (per-message agent loop + dashboard auth) lands in M5.
    /// Not invoked from production paths yet — do not assume it's
    /// safe to delete as unused.
    pub async fn resolve_role_for_sender(
        &self,
        sender: &SenderContext,
        mapping: &ChannelRoleMapping,
        role_query: Option<&dyn ChannelRoleQuery>,
    ) -> UserRole {
        // 1. Explicit UserConfig.role wins. Look up by channel binding
        //    *before* hitting the cache so explicit-role changes during
        //    config reload take effect immediately.
        if let Some(user_id) = self.identify(&sender.channel, &sender.user_id) {
            if let Some(identity) = self.get_user(user_id) {
                debug!(
                    user = %identity.name,
                    role = %identity.role,
                    "resolve_role_for_sender: explicit user role"
                );
                return identity.role;
            }
        }

        // 2. Cache lookup for the channel-derived path.
        let cache_key = RoleCacheKey::from_sender(sender);
        if let Some(cached) = self.role_cache.get(&cache_key) {
            return *cached.value();
        }

        // 3. Translate via the per-channel mapping.
        //
        // `transient` distinguishes "platform call failed" (don't
        // cache — retry next time) from "platform definitively says
        // no role" (cache the Viewer fallback so we don't hammer the
        // API). Without this split, a single 5xx during session warm-
        // up would lock the user at Viewer until session restart.
        let has_mapping_for_channel = match sender.channel.as_str() {
            "telegram" => mapping.telegram.is_some(),
            "discord" => mapping.discord.is_some(),
            "slack" => mapping.slack.is_some(),
            _ => false,
        };
        let (resolved, transient) = match (role_query, has_mapping_for_channel) {
            (Some(query), true) => {
                let chat_id = sender.chat_id.as_deref().unwrap_or("");
                match query.lookup_role(chat_id, &sender.user_id).await {
                    Ok(Some(platform_role)) => (
                        translate_platform_role(mapping, &sender.channel, &platform_role),
                        false,
                    ),
                    Ok(None) => (None, false),
                    Err(e) => {
                        warn!(
                            channel = %sender.channel,
                            user = %sender.user_id,
                            error = %e,
                            "channel role lookup failed; returning default-deny \
                             without caching so the next call re-queries"
                        );
                        (None, true)
                    }
                }
            }
            // No platform query available, or no mapping configured for
            // this channel — fall through to default-deny. Cache it:
            // missing config is a stable state, not a transient one.
            _ => (None, false),
        };

        let role = resolved.unwrap_or(UserRole::Viewer);
        if !transient {
            self.role_cache.insert(cache_key, role);
        }
        role
    }

    /// Drop all cached channel-role resolutions. Called when a session
    /// restarts so a user whose platform role changed mid-session sees the
    /// updated permissions on next interaction.
    pub fn invalidate_role_cache(&self) {
        self.role_cache.clear();
    }

    /// Drop only the cache entries for a single sender — used when a
    /// targeted invalidation suffices (e.g. an admin tooling hook).
    pub fn invalidate_role_cache_for(&self, sender: &SenderContext) {
        self.role_cache.remove(&RoleCacheKey::from_sender(sender));
    }
    /// Resolve a `sender_id` and `channel` pair to a known user, if any.
    ///
    /// Requires an explicit `(channel, sender_id)` tuple. The bare-`sender_id`
    /// fallback was removed in RBAC M3 (#3054) because it silently aliased
    /// users that share a platform-id on different channels — first writer
    /// won the attribution and any inbound from that platform-id on a
    /// third unbound channel inherited the first user's role. Callers
    /// that don't know the channel must either supply one or accept that
    /// the user is unrecognised.
    pub fn resolve_user(&self, sender_id: Option<&str>, channel: Option<&str>) -> Option<UserId> {
        let (Some(ch), Some(sid)) = (channel, sender_id) else {
            return None;
        };
        let key = format!("{ch}:{sid}");
        self.channel_index.get(&key).map(|r| *r.value())
    }

    /// Cheap snapshot of the kernel's tool groups (used for per-user
    /// category evaluation). Returns an `Arc::clone` of the live
    /// snapshot so the resolution hot path doesn't pay a `Vec` clone
    /// per tool call. Config reload swaps the inner `Arc` in place
    /// (`reload()`); existing `Arc` clones held by in-flight evaluations
    /// keep pointing at the pre-swap snapshot for their lifetime.
    pub fn tool_groups(&self) -> std::sync::Arc<Vec<ToolGroup>> {
        self.tool_groups
            .read()
            .expect("AuthManager.tool_groups RwLock poisoned")
            .clone()
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
    ///
    /// `system_call=true` opts the call out of RBAC entirely. ONLY use
    /// this for kernel-internal call sites where there is no end-user
    /// causally responsible for the invocation — cron fires, fork turns,
    /// internal event triggers, etc. Channel messages and direct user
    /// invocations MUST always pass `false` so an unrecognised sender
    /// fails closed (RBAC M3, #3054). The flag exists so every escape
    /// hatch is visible at compile time / grep — no implicit fail-open
    /// based on `sender_id.is_none()` like the previous implementation.
    pub fn resolve_user_tool_decision(
        &self,
        tool_name: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
        system_call: bool,
    ) -> UserToolGate {
        // No registered users → guest mode (default-allow with minimal
        // perms — design decision #2). The runtime keeps its existing
        // approval/capability gates.
        if self.users.is_empty() {
            return UserToolGate::Allow;
        }

        // Explicit system-internal invocations bypass RBAC. Today the
        // only caller that sets this flag is the cron dispatcher (via
        // `LibreFangKernel::resolve_user_tool_decision` matching
        // `channel == "cron"`); future system-fire sites should be
        // wired through the same trait method, never by inventing a
        // new sentinel string here.
        if system_call {
            return UserToolGate::Allow;
        }

        let Some(user_id) = self.resolve_user(sender_id, channel) else {
            // RBAC is enabled but the sender isn't recognised. Default-deny
            // for tools that don't appear on the read-only safe list, route
            // everything else through an admin approval. We no longer
            // fall-OPEN when `sender_id.is_none()` — design decision #2 is
            // default-deny, and an internal call without a sender ID must
            // be marked `system_call=true` explicitly.
            return guest_gate(tool_name);
        };

        let groups = self.tool_groups();
        let Some(identity) = self.get_user(user_id) else {
            return UserToolGate::Allow;
        };

        // Layer A — apply the user's own policy.
        let user_decision = identity
            .policy
            .evaluate(tool_name, channel, groups.as_slice());

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

/// Translate a platform-native role into a LibreFang [`UserRole`] using
/// the channel's configured mapping. Returns `None` when:
/// - no mapping exists for this channel,
/// - the platform-role tokens did not match any configured mapping
///   entry, or
/// - the matched LibreFang role string is unrecognized (typo'd
///   `[channel_role_mapping]` entries fail closed to default-deny
///   rather than being demoted to `User`).
///
/// Per-platform precedence rules (Discord = highest privilege wins,
/// Telegram/Slack = single-token flat lookup) are inlined per channel
/// — there are exactly three platforms and each has bespoke semantics,
/// so a trait + dyn dispatch was over-abstraction.
fn translate_platform_role(
    mapping: &ChannelRoleMapping,
    channel: &str,
    role: &librefang_channels::types::PlatformRole,
) -> Option<UserRole> {
    match channel {
        "telegram" => mapping
            .telegram
            .as_ref()
            .and_then(|m| translate_telegram_role(m, role)),
        "discord" => mapping
            .discord
            .as_ref()
            .and_then(|m| translate_discord_role(m, role)),
        "slack" => mapping
            .slack
            .as_ref()
            .and_then(|m| translate_slack_role(m, role)),
        _ => None,
    }
}

fn translate_telegram_role(
    cfg: &librefang_types::config::TelegramRoleMapping,
    role: &librefang_channels::types::PlatformRole,
) -> Option<UserRole> {
    // Telegram's status token is one of `creator` / `administrator` /
    // `member` / `restricted`. `restricted` is deliberately unmapped —
    // operators wanting to grant restricted members a role use
    // `member_role` and accept that the ~22 fine-grained restriction
    // flags are invisible at this layer (out of scope for M4).
    let primary = role.roles.first()?;
    let mapped = match primary.as_str() {
        "creator" => cfg.creator_role.as_deref(),
        "administrator" => cfg.admin_role.as_deref(),
        "member" => cfg.member_role.as_deref(),
        _ => None,
    };
    // Strict mapping: a typo in `[channel_role_mapping.telegram]` (e.g.
    // `admin_role = "admn"`) falls through to None → Viewer rather
    // than silently granting `User`.
    mapped.and_then(UserRole::try_from_str_role)
}

fn translate_discord_role(
    cfg: &librefang_types::config::DiscordRoleMapping,
    role: &librefang_channels::types::PlatformRole,
) -> Option<UserRole> {
    // Walk every role token the user holds and pick the
    // highest-privilege match from `role_map`. Discord users routinely
    // hold multiple roles simultaneously and operators expect the most
    // privileged mapping to win — taking the literal first match would
    // mean role-list ordering on Discord's side decides LibreFang
    // permissions, which is not under our control.
    let mut best: Option<UserRole> = None;
    for name in &role.roles {
        if let Some(mapped_str) = cfg.role_map.get(name) {
            // Strict mapping: typo in `role_map` (e.g. `Moderator = "admn"`)
            // is skipped rather than defaulting to `User`, so unrecognized
            // role-name → privilege drift is impossible.
            if let Some(candidate) = UserRole::try_from_str_role(mapped_str) {
                best = Some(match best {
                    Some(prev) => prev.max(candidate),
                    None => candidate,
                });
            }
        }
    }
    best
}

fn translate_slack_role(
    cfg: &librefang_types::config::SlackRoleMapping,
    role: &librefang_channels::types::PlatformRole,
) -> Option<UserRole> {
    // The Slack adapter pre-collapses to one of owner/admin/member/guest
    // in `parse_users_info_response`; the precedence ladder lives there,
    // not here.
    let primary = role.roles.first()?;
    let mapped = match primary.as_str() {
        "owner" => cfg.owner_role.as_deref(),
        "admin" => cfg.admin_role.as_deref(),
        "member" => cfg.member_role.as_deref(),
        "guest" => cfg.guest_role.as_deref(),
        _ => None,
    };
    // Strict mapping: typo in `[channel_role_mapping.slack]` falls
    // through to None → Viewer.
    mapped.and_then(UserRole::try_from_str_role)
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
        assert_eq!(UserRole::from_str_role("guest"), UserRole::Viewer);
        assert_eq!(UserRole::from_str_role("user"), UserRole::User);
        assert_eq!(UserRole::from_str_role("OWNER"), UserRole::Owner);
        assert_eq!(UserRole::from_str_role("unknown"), UserRole::User);

        // try_from_str_role: strict variant used by channel translators.
        // Channel-role mapping typos must NOT silently grant `User` privilege.
        assert_eq!(UserRole::try_from_str_role("owner"), Some(UserRole::Owner));
        assert_eq!(UserRole::try_from_str_role("admin"), Some(UserRole::Admin));
        assert_eq!(UserRole::try_from_str_role("user"), Some(UserRole::User));
        assert_eq!(
            UserRole::try_from_str_role("viewer"),
            Some(UserRole::Viewer)
        );
        assert_eq!(UserRole::try_from_str_role("guest"), Some(UserRole::Viewer));
        assert_eq!(UserRole::try_from_str_role("ADMIN"), Some(UserRole::Admin));
        // Typos and unknown role names are None — the resolver falls through
        // to Viewer (default-deny) rather than User.
        assert_eq!(UserRole::try_from_str_role("admn"), None);
        assert_eq!(UserRole::try_from_str_role("ower"), None);
        assert_eq!(UserRole::try_from_str_role(""), None);
        assert_eq!(UserRole::try_from_str_role("Moderator"), None);
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
        let gate =
            mgr.resolve_user_tool_decision("shell_exec", Some("111"), Some("telegram"), false);
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
        let gate =
            mgr.resolve_user_tool_decision("shell_exec", Some("111"), Some("telegram"), false);
        assert!(matches!(gate, UserToolGate::NeedsApproval { .. }));
    }

    #[test]
    fn rbac_m3_admin_role_passes_through_unconfigured() {
        let admin = user_with_policy("Admin", "admin", "999", None, None, None, HashMap::new());
        let mgr = AuthManager::with_tool_groups(&[admin], &[]);
        let gate =
            mgr.resolve_user_tool_decision("shell_exec", Some("999"), Some("telegram"), false);
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
        // RBAC M3 #3054 H6: bind Bob on BOTH telegram and discord so the
        // discord case can be attributed to Bob without the (now-removed)
        // bare-platform-id fallback. Without explicit bindings, an
        // unbound channel correctly fails closed via the guest gate.
        let mut bob = user_with_policy("Bob", "admin", "111", None, None, None, rules);
        bob.channel_bindings
            .insert("discord".to_string(), "111".to_string());
        let mgr = AuthManager::with_tool_groups(&[bob], &[]);

        // From telegram → channel rule denies even though admin role would.
        let from_tg =
            mgr.resolve_user_tool_decision("shell_exec", Some("111"), Some("telegram"), false);
        assert!(matches!(from_tg, UserToolGate::Deny { .. }));

        // From a different channel → no rule, admin role allows.
        let from_dc =
            mgr.resolve_user_tool_decision("shell_exec", Some("111"), Some("discord"), false);
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
        let gate =
            mgr.resolve_user_tool_decision("shell_exec", Some("111"), Some("telegram"), false);
        assert!(matches!(gate, UserToolGate::Deny { .. }));
        // Tool outside the denied group is fine.
        let ok = mgr.resolve_user_tool_decision("file_read", Some("111"), Some("telegram"), false);
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
        let safe =
            mgr.resolve_user_tool_decision("file_read", Some("guest42"), Some("telegram"), false);
        assert_eq!(safe, UserToolGate::Allow);
        let unsafe_ =
            mgr.resolve_user_tool_decision("shell_exec", Some("guest42"), Some("telegram"), false);
        assert!(matches!(unsafe_, UserToolGate::NeedsApproval { .. }));
    }

    /// H6 regression: two users sharing the same platform-id on
    /// different channels MUST NOT alias on a third unbound channel.
    /// The bare-`platform_id` index that previously did first-write-wins
    /// was removed; resolution now requires an explicit (channel, sid)
    /// tuple, so the third channel returns `None` (guest gate kicks in)
    /// rather than silently inheriting the first user's role.
    #[test]
    fn rbac_m3_platform_id_collision_no_longer_aliases_across_channels() {
        let alice = user_with_policy("Alice", "owner", "shared", None, None, None, HashMap::new());
        // Bob also uses platform-id "shared", but on Discord.
        let mut bob = user_with_policy("Bob", "user", "shared", None, None, None, HashMap::new());
        bob.channel_bindings.clear();
        bob.channel_bindings
            .insert("discord".to_string(), "shared".to_string());

        let mgr = AuthManager::with_tool_groups(&[alice, bob], &[]);

        // Alice on telegram → owner.
        assert_eq!(
            mgr.identify("telegram", "shared"),
            Some(UserId::from_name("Alice"))
        );
        // Bob on discord → user.
        assert_eq!(
            mgr.identify("discord", "shared"),
            Some(UserId::from_name("Bob"))
        );

        // Inbound on a THIRD channel (e.g. slack) carrying platform-id
        // "shared" must NOT silently attribute to whichever user was
        // registered first — must return None so the guest gate handles it.
        assert_eq!(
            mgr.resolve_user(Some("shared"), Some("slack")),
            None,
            "platform-id from a third channel must not alias to a registered user"
        );

        // shell_exec from that unattributed sender must therefore go
        // through the guest gate (NeedsApproval), not silently get
        // Alice's owner role.
        let gate =
            mgr.resolve_user_tool_decision("shell_exec", Some("shared"), Some("slack"), false);
        assert!(
            matches!(gate, UserToolGate::NeedsApproval { .. }),
            "third-channel inbound must NOT inherit Alice's role, got {gate:?}"
        );
    }

    /// H7 regression: when `sender_id` is `None` and `system_call=false`,
    /// the kernel must NOT silently fail-OPEN. Previously, the
    /// `sender_id.is_none()` branch returned `UserToolGate::Allow`,
    /// bypassing RBAC for any internal call that forgot to mark itself.
    /// Now the guest gate applies, and a tool that isn't on the read-
    /// only allowlist gets escalated to approval. The explicit
    /// `system_call=true` opt-out still works for cron / forks.
    #[test]
    fn rbac_m3_sender_none_no_system_flag_does_not_fail_open() {
        let alice = user_with_policy("Alice", "owner", "1", None, None, None, HashMap::new());
        let mgr = AuthManager::with_tool_groups(&[alice], &[]);

        // sender_id=None + system_call=false → guest gate (default-deny).
        let gate = mgr.resolve_user_tool_decision("shell_exec", None, None, false);
        assert!(
            matches!(gate, UserToolGate::NeedsApproval { .. }),
            "no sender + no system flag must NOT silently allow shell_exec, got {gate:?}"
        );

        // Read-only safe tool is still permitted via the guest gate.
        let safe = mgr.resolve_user_tool_decision("file_read", None, None, false);
        assert_eq!(safe, UserToolGate::Allow);

        // system_call=true preserves the legacy escape hatch for cron / forks.
        let cron = mgr.resolve_user_tool_decision("shell_exec", None, None, true);
        assert_eq!(cron, UserToolGate::Allow);
    }

    #[test]
    fn rbac_m3_no_users_keeps_legacy_behaviour() {
        let mgr = AuthManager::with_tool_groups(&[], &[]);
        // No registered users → guest mode (default-allow with minimal
        // perms). Existing approval gates take over.
        assert_eq!(
            mgr.resolve_user_tool_decision("shell_exec", Some("anyone"), Some("telegram"), false),
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

#[cfg(test)]
mod channel_role_tests {
    use super::*;
    use async_trait::async_trait;
    use librefang_channels::types::PlatformRole;
    use librefang_types::config::{
        ChannelRoleMapping, DiscordRoleMapping, SlackRoleMapping, TelegramRoleMapping,
    };
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Test double that counts how many times the platform was queried.
    struct StaticRoleQuery {
        result: Result<Option<PlatformRole>, String>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ChannelRoleQuery for StaticRoleQuery {
        async fn lookup_role(
            &self,
            _chat_id: &str,
            _user_id: &str,
        ) -> Result<Option<PlatformRole>, Box<dyn std::error::Error + Send + Sync>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.result
                .clone()
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })
        }
    }

    fn telegram_sender(user_id: &str, chat_id: &str) -> SenderContext {
        SenderContext {
            channel: "telegram".to_string(),
            user_id: user_id.to_string(),
            chat_id: Some(chat_id.to_string()),
            display_name: "Tester".to_string(),
            ..Default::default()
        }
    }

    fn telegram_only_mapping() -> ChannelRoleMapping {
        ChannelRoleMapping {
            telegram: Some(TelegramRoleMapping {
                creator_role: Some("owner".to_string()),
                admin_role: Some("admin".to_string()),
                member_role: Some("user".to_string()),
            }),
            discord: None,
            slack: None,
        }
    }

    #[tokio::test]
    async fn channel_role_explicit_user_config_wins() {
        // RBAC M4 design decision §4: explicit role > channel-derived.
        // Even when the platform reports `member`, the explicit Owner role
        // assigned in UserConfig must take precedence.
        let configs = vec![UserConfig {
            name: "Alice".to_string(),
            role: "owner".to_string(),
            channel_bindings: {
                let mut m = HashMap::new();
                m.insert("telegram".to_string(), "tg-alice".to_string());
                m
            },
            api_key_hash: None,
            tool_policy: None,
            tool_categories: None,
            memory_access: None,
            channel_tool_rules: HashMap::new(),
        }];
        let mgr = AuthManager::new(&configs);
        let calls = Arc::new(AtomicUsize::new(0));
        let query = StaticRoleQuery {
            result: Ok(Some(PlatformRole::single("member"))),
            calls: calls.clone(),
        };
        let sender = telegram_sender("tg-alice", "chat-1");
        let role = mgr
            .resolve_role_for_sender(&sender, &telegram_only_mapping(), Some(&query))
            .await;
        assert_eq!(role, UserRole::Owner);
        // Platform must NOT be queried when explicit role is present.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn channel_role_telegram_creator_maps_to_owner() {
        let mgr = AuthManager::new(&[]);
        let calls = Arc::new(AtomicUsize::new(0));
        let query = StaticRoleQuery {
            result: Ok(Some(PlatformRole::single("creator"))),
            calls: calls.clone(),
        };
        let sender = telegram_sender("tg-bob", "chat-1");
        let role = mgr
            .resolve_role_for_sender(&sender, &telegram_only_mapping(), Some(&query))
            .await;
        assert_eq!(role, UserRole::Owner);
    }

    #[tokio::test]
    async fn channel_role_telegram_admin_maps() {
        let mgr = AuthManager::new(&[]);
        let calls = Arc::new(AtomicUsize::new(0));
        let query = StaticRoleQuery {
            result: Ok(Some(PlatformRole::single("administrator"))),
            calls: calls.clone(),
        };
        let role = mgr
            .resolve_role_for_sender(
                &telegram_sender("tg-bob", "chat-1"),
                &telegram_only_mapping(),
                Some(&query),
            )
            .await;
        assert_eq!(role, UserRole::Admin);
    }

    #[tokio::test]
    async fn channel_role_telegram_member_maps() {
        let mgr = AuthManager::new(&[]);
        let calls = Arc::new(AtomicUsize::new(0));
        let query = StaticRoleQuery {
            result: Ok(Some(PlatformRole::single("member"))),
            calls: calls.clone(),
        };
        let role = mgr
            .resolve_role_for_sender(
                &telegram_sender("tg-bob", "chat-1"),
                &telegram_only_mapping(),
                Some(&query),
            )
            .await;
        assert_eq!(role, UserRole::User);
    }

    #[tokio::test]
    async fn channel_role_discord_picks_highest_privilege_match() {
        // User has both "Member" and "Moderator" roles; the resolver must
        // pick the higher-privilege one regardless of role ordering.
        let mut role_map = HashMap::new();
        role_map.insert("Moderator".to_string(), "admin".to_string());
        role_map.insert("Member".to_string(), "user".to_string());
        role_map.insert("Guest".to_string(), "guest".to_string());
        let mapping = ChannelRoleMapping {
            telegram: None,
            discord: Some(DiscordRoleMapping { role_map }),
            slack: None,
        };
        let mgr = AuthManager::new(&[]);
        let calls = Arc::new(AtomicUsize::new(0));
        let query = StaticRoleQuery {
            result: Ok(Some(PlatformRole::many(vec![
                "Member".to_string(),
                "Moderator".to_string(),
            ]))),
            calls: calls.clone(),
        };
        let sender = SenderContext {
            channel: "discord".to_string(),
            user_id: "dc-user".to_string(),
            chat_id: Some("guild-1".to_string()),
            ..Default::default()
        };
        let role = mgr
            .resolve_role_for_sender(&sender, &mapping, Some(&query))
            .await;
        assert_eq!(role, UserRole::Admin);
    }

    #[tokio::test]
    async fn channel_role_discord_unmapped_role_falls_back_to_viewer() {
        // User holds a guild role that operator did not put in role_map —
        // result is default-deny (Viewer), not an error.
        let mut role_map = HashMap::new();
        role_map.insert("Moderator".to_string(), "admin".to_string());
        let mapping = ChannelRoleMapping {
            discord: Some(DiscordRoleMapping { role_map }),
            ..Default::default()
        };
        let mgr = AuthManager::new(&[]);
        let calls = Arc::new(AtomicUsize::new(0));
        let query = StaticRoleQuery {
            result: Ok(Some(PlatformRole::single("RandomVanityRole"))),
            calls: calls.clone(),
        };
        let sender = SenderContext {
            channel: "discord".to_string(),
            user_id: "dc-user".to_string(),
            chat_id: Some("guild-1".to_string()),
            ..Default::default()
        };
        let role = mgr
            .resolve_role_for_sender(&sender, &mapping, Some(&query))
            .await;
        assert_eq!(role, UserRole::Viewer);
    }

    #[tokio::test]
    async fn channel_role_slack_owner_admin_member_guest() {
        let mapping = ChannelRoleMapping {
            slack: Some(SlackRoleMapping {
                owner_role: Some("owner".to_string()),
                admin_role: Some("admin".to_string()),
                member_role: Some("user".to_string()),
                guest_role: Some("guest".to_string()),
            }),
            ..Default::default()
        };
        let cases = [
            ("owner", UserRole::Owner),
            ("admin", UserRole::Admin),
            ("member", UserRole::User),
            ("guest", UserRole::Viewer),
        ];
        for (raw, expected) in cases {
            let mgr = AuthManager::new(&[]);
            let calls = Arc::new(AtomicUsize::new(0));
            let query = StaticRoleQuery {
                result: Ok(Some(PlatformRole::single(raw))),
                calls: calls.clone(),
            };
            let sender = SenderContext {
                channel: "slack".to_string(),
                user_id: "U-test".to_string(),
                chat_id: Some("workspace".to_string()),
                ..Default::default()
            };
            let role = mgr
                .resolve_role_for_sender(&sender, &mapping, Some(&query))
                .await;
            assert_eq!(role, expected, "slack {raw} should map to {expected}");
        }
    }

    #[tokio::test]
    async fn channel_role_caches_per_session() {
        // Second call with the same sender must NOT re-query the platform.
        let mgr = AuthManager::new(&[]);
        let calls = Arc::new(AtomicUsize::new(0));
        let query = StaticRoleQuery {
            result: Ok(Some(PlatformRole::single("administrator"))),
            calls: calls.clone(),
        };
        let sender = telegram_sender("tg-bob", "chat-1");
        let r1 = mgr
            .resolve_role_for_sender(&sender, &telegram_only_mapping(), Some(&query))
            .await;
        let r2 = mgr
            .resolve_role_for_sender(&sender, &telegram_only_mapping(), Some(&query))
            .await;
        assert_eq!(r1, UserRole::Admin);
        assert_eq!(r2, UserRole::Admin);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "platform must be queried only once per session per (channel,chat,user)"
        );
    }

    #[tokio::test]
    async fn channel_role_cache_invalidation_re_queries() {
        let mgr = AuthManager::new(&[]);
        let calls = Arc::new(AtomicUsize::new(0));
        let query = StaticRoleQuery {
            result: Ok(Some(PlatformRole::single("administrator"))),
            calls: calls.clone(),
        };
        let sender = telegram_sender("tg-bob", "chat-1");
        let _ = mgr
            .resolve_role_for_sender(&sender, &telegram_only_mapping(), Some(&query))
            .await;
        mgr.invalidate_role_cache();
        let _ = mgr
            .resolve_role_for_sender(&sender, &telegram_only_mapping(), Some(&query))
            .await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn channel_role_lookup_failure_falls_back_to_viewer_not_error() {
        // Fail-closed: a transport error from the platform must never
        // elevate privileges. The user gets default-deny.
        let mgr = AuthManager::new(&[]);
        let calls = Arc::new(AtomicUsize::new(0));
        let query = StaticRoleQuery {
            result: Err("network unreachable".to_string()),
            calls: calls.clone(),
        };
        let sender = telegram_sender("tg-bob", "chat-1");
        let role = mgr
            .resolve_role_for_sender(&sender, &telegram_only_mapping(), Some(&query))
            .await;
        assert_eq!(role, UserRole::Viewer);
    }

    /// Test double whose first `lookup_role` call fails and every
    /// subsequent call succeeds with `success`. Exercises the
    /// "transient platform error must not poison the cache" path.
    struct FailThenSucceedQuery {
        success: PlatformRole,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ChannelRoleQuery for FailThenSucceedQuery {
        async fn lookup_role(
            &self,
            _chat_id: &str,
            _user_id: &str,
        ) -> Result<Option<PlatformRole>, Box<dyn std::error::Error + Send + Sync>> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Err("transient 503".into())
            } else {
                Ok(Some(self.success.clone()))
            }
        }
    }

    #[tokio::test]
    async fn channel_role_transient_failure_does_not_poison_cache() {
        // Regression: an `Err` from the platform used to be cached as
        // Viewer for the rest of the session, locking the user out
        // until restart. Now we return Viewer for the failing call but
        // skip the cache write, so the next call re-queries and picks
        // up the recovered platform.
        let mgr = AuthManager::new(&[]);
        let calls = Arc::new(AtomicUsize::new(0));
        let query = FailThenSucceedQuery {
            success: PlatformRole::single("administrator"),
            calls: calls.clone(),
        };
        let sender = telegram_sender("tg-bob", "chat-1");

        // First call: platform fails → fail-closed Viewer, no cache.
        let r1 = mgr
            .resolve_role_for_sender(&sender, &telegram_only_mapping(), Some(&query))
            .await;
        assert_eq!(r1, UserRole::Viewer, "first call must fail closed");

        // Second call: platform recovers → must re-query (proves no
        // cached Viewer is shadowing the recovery) AND must reflect
        // the now-administrator role.
        let r2 = mgr
            .resolve_role_for_sender(&sender, &telegram_only_mapping(), Some(&query))
            .await;
        assert_eq!(
            r2,
            UserRole::Admin,
            "second call must pick up the recovered role, not a cached Viewer"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "platform must be re-queried after a transient failure"
        );

        // Third call: platform still up → cache hit, no extra query.
        let r3 = mgr
            .resolve_role_for_sender(&sender, &telegram_only_mapping(), Some(&query))
            .await;
        assert_eq!(r3, UserRole::Admin);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "successful resolution must populate the cache so subsequent calls hit it"
        );
    }

    #[tokio::test]
    async fn channel_role_no_mapping_for_channel_yields_viewer() {
        // Slack mapping configured but the sender is on Telegram — the
        // resolver has nothing to translate against, so default-deny.
        let mapping = ChannelRoleMapping {
            slack: Some(SlackRoleMapping {
                owner_role: Some("owner".to_string()),
                admin_role: Some("admin".to_string()),
                member_role: Some("user".to_string()),
                guest_role: Some("guest".to_string()),
            }),
            ..Default::default()
        };
        let mgr = AuthManager::new(&[]);
        let calls = Arc::new(AtomicUsize::new(0));
        let query = StaticRoleQuery {
            result: Ok(Some(PlatformRole::single("creator"))),
            calls: calls.clone(),
        };
        let role = mgr
            .resolve_role_for_sender(&telegram_sender("tg-bob", "chat-1"), &mapping, Some(&query))
            .await;
        assert_eq!(role, UserRole::Viewer);
        // No translator → no need to query the platform.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn channel_role_partial_mapping_unset_status_falls_through() {
        // Mapping has admin_role + member_role but no creator_role.
        // A `creator` user should get None → Viewer, not be promoted to
        // admin or another lower-privilege match.
        let mapping = ChannelRoleMapping {
            telegram: Some(TelegramRoleMapping {
                admin_role: Some("admin".to_string()),
                member_role: Some("user".to_string()),
                creator_role: None,
            }),
            ..Default::default()
        };
        let mgr = AuthManager::new(&[]);
        let calls = Arc::new(AtomicUsize::new(0));
        let query = StaticRoleQuery {
            result: Ok(Some(PlatformRole::single("creator"))),
            calls: calls.clone(),
        };
        let role = mgr
            .resolve_role_for_sender(&telegram_sender("u1", "c1"), &mapping, Some(&query))
            .await;
        assert_eq!(role, UserRole::Viewer);
    }

    #[tokio::test]
    async fn channel_role_typo_in_mapping_falls_closed_to_viewer() {
        // RBAC M4 fail-closed: a typo in [channel_role_mapping.*] must NOT
        // silently translate to UserRole::User. Three paths to cover —
        // Telegram, Discord, Slack — each fed an unrecognized role-name
        // string. The resolver must return Viewer (not User).

        // Telegram: `creator_role = "ower"` (typo) — should yield Viewer.
        {
            let mapping = ChannelRoleMapping {
                telegram: Some(TelegramRoleMapping {
                    creator_role: Some("ower".to_string()), // typo
                    admin_role: Some("admn".to_string()),   // typo
                    member_role: Some("guest".to_string()), // valid synonym for Viewer
                }),
                discord: None,
                slack: None,
            };
            let mgr = AuthManager::new(&[]);
            let calls = Arc::new(AtomicUsize::new(0));
            let query = StaticRoleQuery {
                result: Ok(Some(PlatformRole::single("creator"))),
                calls: calls.clone(),
            };
            let role = mgr
                .resolve_role_for_sender(
                    &telegram_sender("tg-typo", "chat-1"),
                    &mapping,
                    Some(&query),
                )
                .await;
            assert_eq!(
                role,
                UserRole::Viewer,
                "telegram creator_role typo must fail closed"
            );
        }

        // Discord: `role_map = { Moderator = "admn" }` (typo) — Moderator
        // user should NOT become User. Falls through to Viewer.
        {
            let mut role_map = HashMap::new();
            role_map.insert("Moderator".to_string(), "admn".to_string()); // typo
            role_map.insert("Member".to_string(), "viewer".to_string());
            let mapping = ChannelRoleMapping {
                telegram: None,
                discord: Some(DiscordRoleMapping { role_map }),
                slack: None,
            };
            let mgr = AuthManager::new(&[]);
            let calls = Arc::new(AtomicUsize::new(0));
            let query = StaticRoleQuery {
                result: Ok(Some(PlatformRole::single("Moderator"))),
                calls: calls.clone(),
            };
            let sender = SenderContext {
                channel: "discord".to_string(),
                user_id: "user-typo".to_string(),
                chat_id: Some("guild-1".to_string()),
                display_name: "Tester".to_string(),
                ..Default::default()
            };
            let role = mgr
                .resolve_role_for_sender(&sender, &mapping, Some(&query))
                .await;
            assert_eq!(
                role,
                UserRole::Viewer,
                "discord role_map typo must fail closed"
            );
        }

        // Slack: `admin_role = "admn"` typo — Slack admin user falls through
        // to Viewer rather than being silently demoted to User.
        {
            let mapping = ChannelRoleMapping {
                telegram: None,
                discord: None,
                slack: Some(SlackRoleMapping {
                    owner_role: Some("owner".to_string()),
                    admin_role: Some("admn".to_string()), // typo
                    member_role: Some("viewer".to_string()),
                    guest_role: Some("guest".to_string()),
                }),
            };
            let mgr = AuthManager::new(&[]);
            let calls = Arc::new(AtomicUsize::new(0));
            let query = StaticRoleQuery {
                result: Ok(Some(PlatformRole::single("admin"))),
                calls: calls.clone(),
            };
            let sender = SenderContext {
                channel: "slack".to_string(),
                user_id: "U-TYPO".to_string(),
                chat_id: Some("C-1".to_string()),
                display_name: "Tester".to_string(),
                ..Default::default()
            };
            let role = mgr
                .resolve_role_for_sender(&sender, &mapping, Some(&query))
                .await;
            assert_eq!(
                role,
                UserRole::Viewer,
                "slack admin_role typo must fail closed"
            );
        }
    }
}
