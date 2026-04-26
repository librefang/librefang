//! RBAC authentication and authorization for multi-user access control.
//!
//! The AuthManager maps platform user identities (Telegram ID, Discord ID, etc.)
//! to LibreFang users with roles, then enforces permission checks on actions.

use dashmap::DashMap;
use librefang_channels::types::{ChannelRoleQuery, SenderContext};
use librefang_types::agent::UserId;
use librefang_types::config::{ChannelRoleMapping, UserConfig};
use librefang_types::error::{LibreFangError, LibreFangResult};
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
}

impl AuthManager {
    /// Create a new AuthManager from kernel user configuration.
    pub fn new(user_configs: &[UserConfig]) -> Self {
        let manager = Self {
            users: DashMap::new(),
            channel_index: DashMap::new(),
            role_cache: DashMap::new(),
        };

        for config in user_configs {
            let user_id = UserId::from_name(&config.name);
            let role = UserRole::from_str_role(&config.role);
            let identity = UserIdentity {
                id: user_id,
                name: config.name.clone(),
                role,
            };

            manager.users.insert(user_id, identity);

            // Index channel bindings
            for (channel_type, platform_id) in &config.channel_bindings {
                let key = format!("{channel_type}:{platform_id}");
                manager.channel_index.insert(key, user_id);
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
            },
            UserConfig {
                name: "ReadOnly".to_string(),
                role: "viewer".to_string(),
                channel_bindings: HashMap::new(),
                api_key_hash: None,
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
