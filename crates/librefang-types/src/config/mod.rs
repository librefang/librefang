//! Configuration types for the LibreFang kernel.
//!
//! This module splits configuration-related code into submodules by responsibility:
//! - `types`: All configuration struct and enum definitions
//! - `serde_helpers`: Custom serialization/deserialization helper functions
//! - `validation`: Configuration validation and safety boundary constraints
//! - `version`: Configuration version tracking

mod serde_helpers;
mod types;
mod validation;
mod version;

// Maintain backward compatibility: re-export all public types
pub use serde_helpers::*;
pub use types::*;
pub use version::*;

/// Default API listen port. Every place that needs the default port
/// should reference this constant so a rename is a single-line change.
pub const DEFAULT_API_PORT: u16 = 4545;

/// Default API listen address (loopback + default port).
pub const DEFAULT_API_LISTEN: &str = "127.0.0.1:4545";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = KernelConfig::default();
        assert_eq!(config.log_level, "info");
        assert_eq!(config.api_listen, DEFAULT_API_LISTEN);
        assert!(!config.network_enabled);
    }

    #[test]
    fn test_config_serialization() {
        let config = KernelConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("log_level"));
    }

    #[test]
    fn test_discord_config_defaults() {
        let dc = DiscordConfig::default();
        assert_eq!(dc.bot_token_env, "DISCORD_BOT_TOKEN");
        assert!(dc.allowed_guilds.is_empty());
        assert_eq!(dc.intents, 37376);
        assert!(dc.ignore_bots);
    }

    #[test]
    fn test_discord_config_ignore_bots_deserialization() {
        let toml_str = r#"
            bot_token_env = "DISCORD_BOT_TOKEN"
            ignore_bots = false
        "#;
        let dc: DiscordConfig = toml::from_str(toml_str).unwrap();
        assert!(!dc.ignore_bots);

        // Default (field omitted) should be true
        let toml_str2 = r#"
            bot_token_env = "DISCORD_BOT_TOKEN"
        "#;
        let dc2: DiscordConfig = toml::from_str(toml_str2).unwrap();
        assert!(dc2.ignore_bots);
    }

    #[test]
    fn test_slack_config_defaults() {
        let sl = SlackConfig::default();
        assert_eq!(sl.app_token_env, "SLACK_APP_TOKEN");
        assert_eq!(sl.bot_token_env, "SLACK_BOT_TOKEN");
        assert!(sl.allowed_channels.is_empty());
        assert!(sl.unfurl_links.is_none());
    }

    #[test]
    fn test_slack_config_unfurl_links_deserialization() {
        let toml_str = r#"
            app_token_env = "SLACK_APP_TOKEN"
            bot_token_env = "SLACK_BOT_TOKEN"
            unfurl_links = false
        "#;
        let sl: SlackConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(sl.unfurl_links, Some(false));

        let toml_str2 = r#"
            app_token_env = "SLACK_APP_TOKEN"
            bot_token_env = "SLACK_BOT_TOKEN"
            unfurl_links = true
        "#;
        let sl2: SlackConfig = toml::from_str(toml_str2).unwrap();
        assert_eq!(sl2.unfurl_links, Some(true));

        // Default (field omitted) should be None
        let toml_str3 = r#"
            app_token_env = "SLACK_APP_TOKEN"
            bot_token_env = "SLACK_BOT_TOKEN"
        "#;
        let sl3: SlackConfig = toml::from_str(toml_str3).unwrap();
        assert!(sl3.unfurl_links.is_none());
    }

    #[test]
    fn test_validate_no_channels() {
        let config = KernelConfig::default();
        let warnings = config.validate();
        // Only check that no *structural* warnings exist (e.g. bad ports, bad log levels).
        // Channel env-var warnings depend on the host environment and are ignored here.
        let structural: Vec<_> = warnings
            .iter()
            .filter(|w| !w.contains("is not set"))
            .filter(|w| !w.contains("does not exist"))
            .collect();
        assert!(
            structural.is_empty(),
            "default KernelConfig has structural warnings: {structural:?}"
        );
    }

    #[test]
    fn test_kernel_mode_default() {
        let mode = KernelMode::default();
        assert_eq!(mode, KernelMode::Default);
    }

    #[test]
    fn test_kernel_mode_serde() {
        let stable = KernelMode::Stable;
        let json = serde_json::to_string(&stable).unwrap();
        assert_eq!(json, "\"stable\"");
        let back: KernelMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, KernelMode::Stable);
    }

    #[test]
    fn channel_role_mapping_full_toml_roundtrip() {
        // All three platforms populated.
        let toml_src = r#"
[channel_role_mapping.telegram]
admin_role = "admin"
creator_role = "owner"
member_role = "user"

[channel_role_mapping.discord]
role_map = { "Moderator" = "admin", "Member" = "user", "Guest" = "viewer" }

[channel_role_mapping.slack]
admin_role = "admin"
member_role = "user"
guest_role = "viewer"
"#;
        let cfg: KernelConfig = toml::from_str(toml_src).expect("toml parse");
        let tg = cfg.channel_role_mapping.telegram.as_ref().unwrap();
        assert_eq!(tg.admin_role.as_deref(), Some("admin"));
        assert_eq!(tg.creator_role.as_deref(), Some("owner"));
        assert_eq!(tg.member_role.as_deref(), Some("user"));

        let dc = cfg.channel_role_mapping.discord.as_ref().unwrap();
        assert_eq!(dc.role_map.get("Moderator"), Some(&"admin".to_string()));
        assert_eq!(dc.role_map.get("Guest"), Some(&"viewer".to_string()));

        let sl = cfg.channel_role_mapping.slack.as_ref().unwrap();
        assert_eq!(sl.admin_role.as_deref(), Some("admin"));
        assert_eq!(sl.guest_role.as_deref(), Some("viewer"));
        assert!(sl.owner_role.is_none()); // Not set in source.

        // Round-trip back to TOML and reparse — survives serialization.
        let serialized = toml::to_string(&cfg).expect("toml serialize");
        let reparsed: KernelConfig = toml::from_str(&serialized).expect("toml reparse");
        assert!(!reparsed.channel_role_mapping.is_empty());
    }

    #[test]
    fn channel_role_mapping_partial_toml() {
        // Only Telegram configured — other platforms fall through to None.
        let toml_src = r#"
[channel_role_mapping.telegram]
admin_role = "admin"
"#;
        let cfg: KernelConfig = toml::from_str(toml_src).unwrap();
        let tg = cfg.channel_role_mapping.telegram.as_ref().unwrap();
        assert_eq!(tg.admin_role.as_deref(), Some("admin"));
        assert!(tg.creator_role.is_none());
        assert!(cfg.channel_role_mapping.discord.is_none());
        assert!(cfg.channel_role_mapping.slack.is_none());
    }

    #[test]
    fn channel_role_mapping_empty_default() {
        let cfg = KernelConfig::default();
        assert!(cfg.channel_role_mapping.is_empty());
        assert!(cfg.channel_role_mapping.telegram.is_none());
        assert!(cfg.channel_role_mapping.discord.is_none());
        assert!(cfg.channel_role_mapping.slack.is_none());
        // Empty mapping serialises to empty TOML output (skip_serializing_if).
        let serialized = toml::to_string(&cfg).unwrap();
        assert!(!serialized.contains("[channel_role_mapping"));
    }

    #[test]
    fn test_user_config_serde() {
        let uc = UserConfig {
            name: "Alice".to_string(),
            role: "owner".to_string(),
            channel_bindings: {
                let mut m = std::collections::HashMap::new();
                m.insert("telegram".to_string(), "123456".to_string());
                m
            },
            api_key_hash: None,
            budget: None,
            tool_policy: None,
            tool_categories: None,
            memory_access: None,
            channel_tool_rules: std::collections::HashMap::new(),
        };
        let json = serde_json::to_string(&uc).unwrap();
        let back: UserConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "Alice");
        assert_eq!(back.role, "owner");
        assert_eq!(back.channel_bindings.get("telegram").unwrap(), "123456");
    }

    #[test]
    fn test_user_config_with_tool_policy_serde() {
        use crate::user_policy::{
            ChannelToolPolicy, UserMemoryAccess, UserToolCategories, UserToolPolicy,
        };
        let mut channel_rules = std::collections::HashMap::new();
        channel_rules.insert(
            "telegram".to_string(),
            ChannelToolPolicy {
                allowed_tools: vec![],
                denied_tools: vec!["shell_*".to_string()],
            },
        );
        let uc = UserConfig {
            name: "Bob".to_string(),
            role: "user".to_string(),
            channel_bindings: std::collections::HashMap::new(),
            api_key_hash: None,
            budget: None,
            tool_policy: Some(UserToolPolicy {
                allowed_tools: vec!["web_*".to_string()],
                denied_tools: vec!["shell_exec".to_string()],
            }),
            tool_categories: Some(UserToolCategories {
                allowed_groups: vec!["read_only".to_string()],
                denied_groups: vec!["dangerous".to_string()],
            }),
            memory_access: Some(UserMemoryAccess {
                readable_namespaces: vec!["proactive".to_string(), "kv:*".to_string()],
                writable_namespaces: vec!["kv:scratch".to_string()],
                pii_access: false,
                export_allowed: false,
                delete_allowed: true,
            }),
            channel_tool_rules: channel_rules,
        };

        // JSON roundtrip
        let json = serde_json::to_string(&uc).unwrap();
        let back: UserConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tool_policy, uc.tool_policy);
        assert_eq!(back.tool_categories, uc.tool_categories);
        assert_eq!(back.memory_access, uc.memory_access);
        assert_eq!(back.channel_tool_rules, uc.channel_tool_rules);

        // TOML roundtrip
        let tomls = toml::to_string(&uc).unwrap();
        let back2: UserConfig = toml::from_str(&tomls).unwrap();
        assert!(back2.memory_access.as_ref().unwrap().delete_allowed);
        assert!(back2
            .channel_tool_rules
            .get("telegram")
            .unwrap()
            .denied_tools
            .contains(&"shell_*".to_string()));
    }

    #[test]
    fn test_user_config_omitted_optional_policy_defaults_to_none() {
        let toml_str = r#"
            name = "Carol"
            role = "user"
        "#;
        let uc: UserConfig = toml::from_str(toml_str).unwrap();
        assert!(uc.tool_policy.is_none());
        assert!(uc.tool_categories.is_none());
        assert!(uc.memory_access.is_none());
        assert!(uc.channel_tool_rules.is_empty());
    }

    #[test]
    fn test_kernel_config_users_with_tool_policy_toml() {
        let toml_str = r#"
            [[users]]
            name = "Alice"
            role = "admin"

            [users.tool_policy]
            denied_tools = ["shell_exec"]

            [users.memory_access]
            readable_namespaces = ["proactive", "kv:*"]
            writable_namespaces = ["kv:user_alice"]
            pii_access = true
            export_allowed = false
            delete_allowed = true
        "#;
        let config: KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.users.len(), 1);
        let alice = &config.users[0];
        assert_eq!(
            alice.tool_policy.as_ref().unwrap().denied_tools,
            vec!["shell_exec".to_string()]
        );
        let mem = alice.memory_access.as_ref().unwrap();
        assert!(mem.pii_access);
        assert!(mem.delete_allowed);
        assert!(!mem.export_allowed);
        assert_eq!(mem.readable_namespaces.len(), 2);
    }

    #[test]
    fn test_config_with_mode_and_language() {
        let config = KernelConfig {
            mode: KernelMode::Stable,
            language: "ar".to_string(),
            ..Default::default()
        };
        assert_eq!(config.mode, KernelMode::Stable);
        assert_eq!(config.language, "ar");
    }

    #[test]
    fn test_stable_prefix_mode_default_false() {
        let config = KernelConfig::default();
        assert!(!config.stable_prefix_mode);
    }

    #[test]
    fn test_stable_prefix_mode_toml_roundtrip() {
        let config = KernelConfig {
            stable_prefix_mode: true,
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let back: KernelConfig = toml::from_str(&toml_str).unwrap();
        assert!(back.stable_prefix_mode);
    }

    #[test]
    fn test_validate_missing_env_vars() {
        let mut config = KernelConfig::default();
        config.channels.discord = OneOrMany(vec![DiscordConfig {
            bot_token_env: "LIBREFANG_TEST_NONEXISTENT_VAR_DC".to_string(),
            ..Default::default()
        }]);
        let warnings = config.validate();
        assert!(
            warnings.iter().any(|w| w.contains("Discord")),
            "expected a Discord warning in: {warnings:?}"
        );
    }

    #[test]
    fn test_whatsapp_config_defaults() {
        let wa = WhatsAppConfig::default();
        assert_eq!(wa.access_token_env, "WHATSAPP_ACCESS_TOKEN");
        assert_eq!(wa.webhook_port, 8443);
        assert!(wa.allowed_users.is_empty());
    }

    #[test]
    fn test_signal_config_defaults() {
        let sig = SignalConfig::default();
        assert_eq!(sig.api_url, "http://localhost:8080");
        assert!(sig.phone_number.is_empty());
    }

    #[test]
    fn test_matrix_config_defaults() {
        let mx = MatrixConfig::default();
        assert_eq!(mx.homeserver_url, "https://matrix.org");
        assert_eq!(mx.access_token_env, "MATRIX_ACCESS_TOKEN");
        assert!(mx.allowed_rooms.is_empty());
    }

    #[test]
    fn test_email_config_defaults() {
        let em = EmailConfig::default();
        assert_eq!(em.imap_port, 993);
        assert_eq!(em.smtp_port, 587);
        assert_eq!(em.password_env, "EMAIL_PASSWORD");
        assert_eq!(em.folders, vec!["INBOX".to_string()]);
    }

    #[test]
    fn test_whatsapp_config_serde() {
        let wa = WhatsAppConfig {
            phone_number_id: "12345".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&wa).unwrap();
        let back: WhatsAppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.phone_number_id, "12345");
    }

    #[test]
    fn test_matrix_config_serde() {
        let mx = MatrixConfig {
            user_id: "@bot:matrix.org".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&mx).unwrap();
        let back: MatrixConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.user_id, "@bot:matrix.org");
    }

    #[test]
    fn test_channels_config_with_new_channels() {
        let config = KernelConfig {
            channels: ChannelsConfig {
                whatsapp: OneOrMany(vec![WhatsAppConfig::default()]),
                signal: OneOrMany(vec![SignalConfig::default()]),
                matrix: OneOrMany(vec![MatrixConfig::default()]),
                email: OneOrMany(vec![EmailConfig::default()]),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.channels.whatsapp.is_some());
        assert!(config.channels.signal.is_some());
        assert!(config.channels.matrix.is_some());
        assert!(config.channels.email.is_some());
    }

    #[test]
    fn test_teams_config_defaults() {
        let t = TeamsConfig::default();
        assert_eq!(t.app_password_env, "TEAMS_APP_PASSWORD");
        assert_eq!(t.webhook_port, 3978);
        assert!(t.allowed_tenants.is_empty());
        assert!(t.signature_required, "default-deny on Teams webhook");
    }

    #[test]
    fn test_mattermost_config_defaults() {
        let m = MattermostConfig::default();
        assert_eq!(m.token_env, "MATTERMOST_TOKEN");
        assert!(m.server_url.is_empty());
    }

    #[test]
    fn test_irc_config_defaults() {
        let irc = IrcConfig::default();
        assert_eq!(irc.server, "irc.libera.chat");
        assert_eq!(irc.port, 6667);
        assert_eq!(irc.nick, "librefang");
        assert!(!irc.use_tls);
    }

    #[test]
    fn test_google_chat_config_defaults() {
        let gc = GoogleChatConfig::default();
        assert_eq!(gc.service_account_env, "GOOGLE_CHAT_SERVICE_ACCOUNT");
        assert_eq!(gc.webhook_port, 8444);
    }

    #[test]
    fn test_twitch_config_defaults() {
        let tw = TwitchConfig::default();
        assert_eq!(tw.oauth_token_env, "TWITCH_OAUTH_TOKEN");
        assert_eq!(tw.nick, "librefang");
    }

    #[test]
    fn test_rocketchat_config_defaults() {
        let rc = RocketChatConfig::default();
        assert_eq!(rc.token_env, "ROCKETCHAT_TOKEN");
        assert!(rc.server_url.is_empty());
    }

    #[test]
    fn test_zulip_config_defaults() {
        let z = ZulipConfig::default();
        assert_eq!(z.api_key_env, "ZULIP_API_KEY");
        assert!(z.bot_email.is_empty());
    }

    #[test]
    fn test_xmpp_config_defaults() {
        let x = XmppConfig::default();
        assert_eq!(x.password_env, "XMPP_PASSWORD");
        assert_eq!(x.port, 5222);
        assert!(x.rooms.is_empty());
    }

    #[test]
    fn test_all_new_channel_configs_serde() {
        let config = KernelConfig {
            channels: ChannelsConfig {
                teams: OneOrMany(vec![TeamsConfig::default()]),
                mattermost: OneOrMany(vec![MattermostConfig::default()]),
                irc: OneOrMany(vec![IrcConfig::default()]),
                google_chat: OneOrMany(vec![GoogleChatConfig::default()]),
                twitch: OneOrMany(vec![TwitchConfig::default()]),
                rocketchat: OneOrMany(vec![RocketChatConfig::default()]),
                zulip: OneOrMany(vec![ZulipConfig::default()]),
                xmpp: OneOrMany(vec![XmppConfig::default()]),
                ..Default::default()
            },
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let back: KernelConfig = toml::from_str(&toml_str).unwrap();
        assert!(back.channels.teams.is_some());
        assert!(back.channels.mattermost.is_some());
        assert!(back.channels.irc.is_some());
        assert!(back.channels.google_chat.is_some());
        assert!(back.channels.twitch.is_some());
        assert!(back.channels.rocketchat.is_some());
        assert!(back.channels.zulip.is_some());
        assert!(back.channels.xmpp.is_some());
    }

    #[test]
    fn test_channel_overrides_defaults() {
        let ov = ChannelOverrides::default();
        assert_eq!(ov.dm_policy, DmPolicy::Respond);
        assert_eq!(ov.group_policy, GroupPolicy::MentionOnly);
        assert!(ov.group_trigger_patterns.is_empty());
        assert_eq!(ov.rate_limit_per_user, 0);
        assert!(!ov.threading);
        assert!(ov.output_format.is_none());
        assert!(ov.model.is_none());
    }

    #[test]
    fn test_fallback_config_serde_roundtrip() {
        let fb = FallbackProviderConfig {
            provider: "ollama".to_string(),
            model: "llama3.2:latest".to_string(),
            api_key_env: String::new(),
            base_url: None,
        };
        let json = serde_json::to_string(&fb).unwrap();
        let back: FallbackProviderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.provider, "ollama");
        assert_eq!(back.model, "llama3.2:latest");
        assert!(back.api_key_env.is_empty());
        assert!(back.base_url.is_none());
    }

    #[test]
    fn test_fallback_config_default_empty() {
        let config = KernelConfig::default();
        assert!(config.fallback_providers.is_empty());
    }

    #[test]
    fn test_fallback_config_in_toml() {
        let toml_str = r#"
            [[fallback_providers]]
            provider = "ollama"
            model = "llama3.2:latest"

            [[fallback_providers]]
            provider = "groq"
            model = "llama-3.3-70b-versatile"
            api_key_env = "GROQ_API_KEY"
        "#;
        let config: KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.fallback_providers.len(), 2);
        assert_eq!(config.fallback_providers[0].provider, "ollama");
        assert_eq!(config.fallback_providers[1].provider, "groq");
    }

    #[test]
    fn test_channel_overrides_serde() {
        let ov = ChannelOverrides {
            dm_policy: DmPolicy::Ignore,
            group_policy: GroupPolicy::CommandsOnly,
            group_trigger_patterns: vec!["(?i)\\bbot\\b".to_string()],
            rate_limit_per_user: 10,
            threading: true,
            output_format: Some(OutputFormat::TelegramHtml),
            ..Default::default()
        };
        let json = serde_json::to_string(&ov).unwrap();
        let back: ChannelOverrides = serde_json::from_str(&json).unwrap();
        assert_eq!(back.dm_policy, DmPolicy::Ignore);
        assert_eq!(back.group_policy, GroupPolicy::CommandsOnly);
        assert_eq!(back.group_trigger_patterns, vec!["(?i)\\bbot\\b"]);
        assert_eq!(back.rate_limit_per_user, 10);
        assert!(back.threading);
        assert_eq!(back.output_format, Some(OutputFormat::TelegramHtml));
    }

    #[test]
    fn test_clamp_bounds_zero_browser_timeout() {
        let mut config = KernelConfig::default();
        config.browser.timeout_secs = 0;
        config.clamp_bounds();
        assert_eq!(config.browser.timeout_secs, 30);
    }

    #[test]
    fn test_clamp_bounds_excessive_browser_sessions() {
        let mut config = KernelConfig::default();
        config.browser.max_sessions = 999;
        config.clamp_bounds();
        assert_eq!(config.browser.max_sessions, 100);
    }

    #[test]
    fn test_clamp_bounds_zero_fetch_bytes() {
        let mut config = KernelConfig::default();
        config.web.fetch.max_response_bytes = 0;
        config.clamp_bounds();
        assert_eq!(config.web.fetch.max_response_bytes, 5_000_000);
    }

    #[test]
    fn test_clamp_bounds_zero_fetch_timeout() {
        let mut config = KernelConfig::default();
        config.web.fetch.timeout_secs = 0;
        config.clamp_bounds();
        assert_eq!(config.web.fetch.timeout_secs, 30);
    }

    /// PR #3203 review item — `UserBudgetConfig::alert_threshold` is
    /// documented "clamped to 0..=1" but the field is bare `f64`. Without
    /// the clamp in `clamp_bounds`, an out-of-range value silently makes
    /// `alert_breach` either permanently false (>1) or permanently true
    /// (<0), which is exactly what the documentation promises NOT to happen.
    #[test]
    fn test_clamp_bounds_user_alert_threshold() {
        use crate::config::types::{UserBudgetConfig, UserConfig};
        let mut config = KernelConfig {
            users: vec![
                UserConfig {
                    name: "TooHigh".into(),
                    role: "user".into(),
                    channel_bindings: std::collections::HashMap::new(),
                    api_key_hash: None,
                    budget: Some(UserBudgetConfig {
                        alert_threshold: 5.0,
                        ..UserBudgetConfig::default()
                    }),
                    tool_policy: None,
                    tool_categories: None,
                    memory_access: None,
                    channel_tool_rules: std::collections::HashMap::new(),
                },
                UserConfig {
                    name: "Negative".into(),
                    role: "user".into(),
                    channel_bindings: std::collections::HashMap::new(),
                    api_key_hash: None,
                    budget: Some(UserBudgetConfig {
                        alert_threshold: -0.5,
                        ..UserBudgetConfig::default()
                    }),
                    tool_policy: None,
                    tool_categories: None,
                    memory_access: None,
                    channel_tool_rules: std::collections::HashMap::new(),
                },
                UserConfig {
                    name: "NaN".into(),
                    role: "user".into(),
                    channel_bindings: std::collections::HashMap::new(),
                    api_key_hash: None,
                    budget: Some(UserBudgetConfig {
                        alert_threshold: f64::NAN,
                        ..UserBudgetConfig::default()
                    }),
                    tool_policy: None,
                    tool_categories: None,
                    memory_access: None,
                    channel_tool_rules: std::collections::HashMap::new(),
                },
                UserConfig {
                    name: "InRange".into(),
                    role: "user".into(),
                    channel_bindings: std::collections::HashMap::new(),
                    api_key_hash: None,
                    budget: Some(UserBudgetConfig {
                        alert_threshold: 0.65,
                        ..UserBudgetConfig::default()
                    }),
                    tool_policy: None,
                    tool_categories: None,
                    memory_access: None,
                    channel_tool_rules: std::collections::HashMap::new(),
                },
            ],
            ..KernelConfig::default()
        };
        config.clamp_bounds();
        assert_eq!(
            config.users[0].budget.as_ref().unwrap().alert_threshold,
            1.0,
            "above-1 must clamp DOWN to 1.0"
        );
        assert_eq!(
            config.users[1].budget.as_ref().unwrap().alert_threshold,
            0.0,
            "below-0 must clamp UP to 0.0"
        );
        assert_eq!(
            config.users[2].budget.as_ref().unwrap().alert_threshold,
            0.8,
            "NaN must reset to default 0.8 (otherwise pct >= NaN is always false)"
        );
        assert_eq!(
            config.users[3].budget.as_ref().unwrap().alert_threshold,
            0.65,
            "in-range value must round-trip unchanged"
        );
    }

    /// PR #3205 review follow-up — `pii_access`/`export_allowed`/
    /// `delete_allowed` are no-ops without read access (the runtime
    /// guard checks the flag AND `readable_namespaces`). An admin who
    /// toggles a flag without declaring namespaces gets a silent
    /// privilege misconfiguration. `validate()` must surface this so
    /// the typo is caught at boot, not at first failed call.
    #[test]
    fn test_validate_warns_on_memory_access_flags_without_readable_namespaces() {
        use crate::config::types::UserConfig;
        use crate::user_policy::UserMemoryAccess;
        let config = KernelConfig {
            users: vec![
                UserConfig {
                    name: "PiiTypo".into(),
                    role: "user".into(),
                    channel_bindings: std::collections::HashMap::new(),
                    api_key_hash: None,
                    memory_access: Some(UserMemoryAccess {
                        pii_access: true,
                        ..UserMemoryAccess::default()
                    }),
                    tool_policy: None,
                    tool_categories: None,
                    channel_tool_rules: std::collections::HashMap::new(),
                    budget: None,
                },
                UserConfig {
                    name: "ProperlyConfigured".into(),
                    role: "user".into(),
                    channel_bindings: std::collections::HashMap::new(),
                    api_key_hash: None,
                    memory_access: Some(UserMemoryAccess {
                        pii_access: true,
                        readable_namespaces: vec!["proactive".into()],
                        ..UserMemoryAccess::default()
                    }),
                    tool_policy: None,
                    tool_categories: None,
                    channel_tool_rules: std::collections::HashMap::new(),
                    budget: None,
                },
            ],
            ..KernelConfig::default()
        };
        let warnings = config.validate();
        let pii_warnings: Vec<&String> = warnings
            .iter()
            .filter(|w| w.contains("memory_access") && w.contains("readable_namespaces"))
            .collect();
        assert_eq!(
            pii_warnings.len(),
            1,
            "exactly one warning expected (PiiTypo only); got: {warnings:#?}"
        );
        let w = pii_warnings[0];
        assert!(w.contains("PiiTypo"), "warning must name the user: {w}");
        assert!(w.contains("pii_access"), "warning must list the flag: {w}");
        assert!(
            !w.contains("ProperlyConfigured"),
            "warning must NOT name the correctly-configured user: {w}"
        );
    }

    /// `delete_allowed` is gated on **write** access (`check_delete` →
    /// `check_write`), not read access. The earlier validate pass
    /// grouped it under `readable_namespaces`, which would silently miss
    /// a user with read-but-no-write + `delete_allowed = true`. This
    /// test pins the corrected dual-pass semantics: the writable check
    /// fires independently of the readable check.
    #[test]
    fn test_validate_warns_on_delete_allowed_without_writable_namespaces() {
        use crate::config::types::UserConfig;
        use crate::user_policy::UserMemoryAccess;
        let config = KernelConfig {
            users: vec![
                // Has readable but NOT writable + delete_allowed = true →
                // delete will silently fail; must warn.
                UserConfig {
                    name: "DeleteTypo".into(),
                    role: "user".into(),
                    channel_bindings: std::collections::HashMap::new(),
                    api_key_hash: None,
                    memory_access: Some(UserMemoryAccess {
                        readable_namespaces: vec!["proactive".into()],
                        writable_namespaces: vec![],
                        delete_allowed: true,
                        ..UserMemoryAccess::default()
                    }),
                    tool_policy: None,
                    tool_categories: None,
                    channel_tool_rules: std::collections::HashMap::new(),
                    budget: None,
                },
                // Properly configured for delete: has writable + flag.
                // Must NOT trigger the new warning.
                UserConfig {
                    name: "DeleteOk".into(),
                    role: "user".into(),
                    channel_bindings: std::collections::HashMap::new(),
                    api_key_hash: None,
                    memory_access: Some(UserMemoryAccess {
                        readable_namespaces: vec!["proactive".into()],
                        writable_namespaces: vec!["proactive".into()],
                        delete_allowed: true,
                        ..UserMemoryAccess::default()
                    }),
                    tool_policy: None,
                    tool_categories: None,
                    channel_tool_rules: std::collections::HashMap::new(),
                    budget: None,
                },
            ],
            ..KernelConfig::default()
        };
        let warnings = config.validate();
        let delete_warnings: Vec<&String> = warnings
            .iter()
            .filter(|w| w.contains("delete_allowed") && w.contains("writable_namespaces"))
            .collect();
        assert_eq!(
            delete_warnings.len(),
            1,
            "exactly one warning expected (DeleteTypo only); got: {warnings:#?}"
        );
        let w = delete_warnings[0];
        assert!(w.contains("DeleteTypo"), "warning must name the user: {w}");
        assert!(
            !w.contains("DeleteOk"),
            "warning must NOT name the correctly-configured user: {w}"
        );
    }

    #[test]
    fn test_clamp_bounds_defaults_unchanged() {
        let mut config = KernelConfig::default();
        let browser_timeout = config.browser.timeout_secs;
        let browser_sessions = config.browser.max_sessions;
        let fetch_bytes = config.web.fetch.max_response_bytes;
        let fetch_timeout = config.web.fetch.timeout_secs;
        config.clamp_bounds();
        assert_eq!(config.browser.timeout_secs, browser_timeout);
        assert_eq!(config.browser.max_sessions, browser_sessions);
        assert_eq!(config.web.fetch.max_response_bytes, fetch_bytes);
        assert_eq!(config.web.fetch.timeout_secs, fetch_timeout);
    }

    #[test]
    fn test_resolve_api_key_env_convention() {
        let config = KernelConfig::default();
        // Unknown provider falls back to convention
        assert_eq!(config.resolve_api_key_env("nvidia"), "NVIDIA_API_KEY");
        assert_eq!(config.resolve_api_key_env("my-custom"), "MY_CUSTOM_API_KEY");
    }

    #[test]
    fn test_resolve_api_key_env_explicit_mapping() {
        let mut config = KernelConfig::default();
        config
            .provider_api_keys
            .insert("nvidia".to_string(), "NIM_KEY".to_string());
        // Explicit mapping takes precedence over convention
        assert_eq!(config.resolve_api_key_env("nvidia"), "NIM_KEY");
    }

    #[test]
    fn test_resolve_api_key_env_auth_profiles() {
        let mut config = KernelConfig::default();
        config.auth_profiles.insert(
            "nvidia".to_string(),
            vec![AuthProfile {
                name: "primary".to_string(),
                api_key_env: "NVIDIA_PRIMARY_KEY".to_string(),
                priority: 0,
            }],
        );
        // Auth profiles take precedence over convention (but not explicit mapping)
        assert_eq!(config.resolve_api_key_env("nvidia"), "NVIDIA_PRIMARY_KEY");
    }

    #[test]
    fn test_resolve_api_key_env_explicit_over_auth_profile() {
        let mut config = KernelConfig::default();
        config
            .provider_api_keys
            .insert("nvidia".to_string(), "NIM_KEY".to_string());
        config.auth_profiles.insert(
            "nvidia".to_string(),
            vec![AuthProfile {
                name: "primary".to_string(),
                api_key_env: "NVIDIA_PRIMARY_KEY".to_string(),
                priority: 0,
            }],
        );
        // Explicit mapping wins over auth profiles
        assert_eq!(config.resolve_api_key_env("nvidia"), "NIM_KEY");
    }

    #[test]
    fn test_provider_api_keys_toml_roundtrip() {
        let toml_str = r#"
            [provider_api_keys]
            nvidia = "NVIDIA_NIM_KEY"
            azure = "AZURE_OPENAI_KEY"
        "#;
        let config: KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.provider_api_keys.len(), 2);
        assert_eq!(
            config.provider_api_keys.get("nvidia").unwrap(),
            "NVIDIA_NIM_KEY"
        );
        assert_eq!(
            config.provider_api_keys.get("azure").unwrap(),
            "AZURE_OPENAI_KEY"
        );
    }

    #[test]
    fn test_provider_regions_toml_roundtrip() {
        let toml_str = r#"
            [provider_regions]
            qwen = "intl"
            minimax = "china"
        "#;
        let config: KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.provider_regions.len(), 2);
        assert_eq!(config.provider_regions.get("qwen").unwrap(), "intl");
        assert_eq!(config.provider_regions.get("minimax").unwrap(), "china");
    }

    #[test]
    fn test_one_or_many_single_toml_table() {
        // Single [channels.telegram] table should parse as OneOrMany with one element
        let toml_str = r#"
            [channels.telegram]
            bot_token_env = "MY_TG_TOKEN"
            account_id = "bot1"
        "#;
        let config: KernelConfig = toml::from_str(toml_str).unwrap();
        assert!(config.channels.telegram.is_some());
        assert_eq!(config.channels.telegram.len(), 1);
        let tg = config.channels.telegram.first().unwrap();
        assert_eq!(tg.bot_token_env, "MY_TG_TOKEN");
        assert_eq!(tg.account_id.as_deref(), Some("bot1"));
    }

    #[test]
    fn test_one_or_many_array_of_tables() {
        // [[channels.telegram]] should parse as OneOrMany with multiple elements
        let toml_str = r#"
            [[channels.telegram]]
            bot_token_env = "TG_TOKEN_1"
            account_id = "bot1"
            default_agent = "assistant"

            [[channels.telegram]]
            bot_token_env = "TG_TOKEN_2"
            account_id = "bot2"
            default_agent = "coder"
        "#;
        let config: KernelConfig = toml::from_str(toml_str).unwrap();
        assert!(config.channels.telegram.is_some());
        assert_eq!(config.channels.telegram.len(), 2);

        let bots: Vec<_> = config.channels.telegram.iter().collect();
        assert_eq!(bots[0].bot_token_env, "TG_TOKEN_1");
        assert_eq!(bots[0].account_id.as_deref(), Some("bot1"));
        assert_eq!(bots[0].default_agent.as_deref(), Some("assistant"));
        assert_eq!(bots[1].bot_token_env, "TG_TOKEN_2");
        assert_eq!(bots[1].account_id.as_deref(), Some("bot2"));
        assert_eq!(bots[1].default_agent.as_deref(), Some("coder"));
    }

    #[test]
    fn test_one_or_many_single_wechat_table() {
        let toml_str = r#"
            [channels.wechat]
            bot_token_env = "WECHAT_TOKEN_MAIN"
            account_id = "wechat-main"
            default_agent = "assistant"
        "#;
        let config: KernelConfig = toml::from_str(toml_str).unwrap();
        assert!(config.channels.wechat.is_some());
        assert_eq!(config.channels.wechat.len(), 1);

        let wechat = config.channels.wechat.first().unwrap();
        assert_eq!(wechat.bot_token_env, "WECHAT_TOKEN_MAIN");
        assert_eq!(wechat.account_id.as_deref(), Some("wechat-main"));
        assert_eq!(wechat.default_agent.as_deref(), Some("assistant"));
    }

    #[test]
    fn test_one_or_many_array_of_wecom_tables() {
        let toml_str = r#"
            [[channels.wecom]]
            bot_id = "bot-main"
            secret_env = "WECOM_SECRET_MAIN"
            account_id = "wecom-main"
            default_agent = "assistant"

            [[channels.wecom]]
            bot_id = "bot-sales"
            secret_env = "WECOM_SECRET_SALES"
            account_id = "wecom-sales"
            default_agent = "sales-assistant"
        "#;
        let config: KernelConfig = toml::from_str(toml_str).unwrap();
        assert!(config.channels.wecom.is_some());
        assert_eq!(config.channels.wecom.len(), 2);

        let bots: Vec<_> = config.channels.wecom.iter().collect();
        assert_eq!(bots[0].bot_id, "bot-main");
        assert_eq!(bots[0].secret_env, "WECOM_SECRET_MAIN");
        assert_eq!(bots[0].account_id.as_deref(), Some("wecom-main"));
        assert_eq!(bots[0].default_agent.as_deref(), Some("assistant"));
        assert_eq!(bots[1].bot_id, "bot-sales");
        assert_eq!(bots[1].secret_env, "WECOM_SECRET_SALES");
        assert_eq!(bots[1].account_id.as_deref(), Some("wecom-sales"));
        assert_eq!(bots[1].default_agent.as_deref(), Some("sales-assistant"));
    }

    #[test]
    fn test_one_or_many_empty_default() {
        let config = KernelConfig::default();
        assert!(config.channels.telegram.is_none());
        assert!(config.channels.telegram.is_empty());
        assert_eq!(config.channels.telegram.len(), 0);
        assert!(config.channels.telegram.first().is_none());
        assert!(config.channels.telegram.as_ref().is_none());
    }

    #[test]
    fn test_one_or_many_serialize_roundtrip() {
        // Single element serializes as a bare table, multi as array-of-tables
        let single = OneOrMany(vec![TelegramConfig::default()]);
        let json = serde_json::to_string(&single).unwrap();
        let back: OneOrMany<TelegramConfig> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 1);

        let multi = OneOrMany(vec![TelegramConfig::default(), TelegramConfig::default()]);
        let json = serde_json::to_string(&multi).unwrap();
        let back: OneOrMany<TelegramConfig> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 2);

        let empty: OneOrMany<TelegramConfig> = OneOrMany::default();
        let json = serde_json::to_string(&empty).unwrap();
        assert_eq!(json, "null");
    }

    #[test]
    fn test_account_id_in_channel_configs() {
        // Verify account_id field exists and defaults to None
        assert!(TelegramConfig::default().account_id.is_none());
        assert!(DiscordConfig::default().account_id.is_none());
        assert!(SlackConfig::default().account_id.is_none());
        assert!(WhatsAppConfig::default().account_id.is_none());
        assert!(SignalConfig::default().account_id.is_none());
        assert!(MatrixConfig::default().account_id.is_none());
        assert!(EmailConfig::default().account_id.is_none());
        assert!(WeChatConfig::default().account_id.is_none());
        assert!(WeComConfig::default().account_id.is_none());
    }

    #[test]
    fn test_redact_proxy_url_with_credentials() {
        assert_eq!(
            redact_proxy_url("http://user:pass@proxy.example.com:8080"),
            "http://***@proxy.example.com:8080"
        );
    }

    #[test]
    fn test_redact_proxy_url_without_credentials() {
        assert_eq!(
            redact_proxy_url("http://proxy.example.com:8080"),
            "http://proxy.example.com:8080"
        );
    }

    #[test]
    fn test_redact_proxy_url_empty() {
        assert_eq!(redact_proxy_url(""), "");
    }

    #[test]
    fn test_proxy_config_debug_redacts_credentials() {
        let cfg = ProxyConfig {
            http_proxy: Some("http://admin:secret@proxy:8080".to_string()),
            https_proxy: Some("http://proxy:8080".to_string()),
            no_proxy: Some("localhost".to_string()),
        };
        let debug = format!("{:?}", cfg);
        assert!(
            !debug.contains("secret"),
            "credentials leaked in Debug output: {debug}"
        );
        assert!(
            !debug.contains("admin"),
            "username leaked in Debug output: {debug}"
        );
        assert!(
            debug.contains("***"),
            "Debug output should contain redacted marker"
        );
    }

    // --- Config validation with tolerant mode tests ---

    #[test]
    fn test_strict_config_defaults_to_false() {
        let config = KernelConfig::default();
        assert!(!config.strict_config);
    }

    #[test]
    fn test_strict_config_toml_roundtrip() {
        let config = KernelConfig {
            strict_config: true,
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let back: KernelConfig = toml::from_str(&toml_str).unwrap();
        assert!(back.strict_config);
    }

    #[test]
    fn test_known_top_level_fields_not_empty() {
        let fields = KernelConfig::known_top_level_fields();
        assert!(fields.len() > 30, "expected many known fields");
        assert!(fields.contains(&"api_listen"));
        assert!(fields.contains(&"log_level"));
        assert!(fields.contains(&"strict_config"));
        // Aliases must also be present
        assert!(fields.contains(&"listen_addr"));
        assert!(fields.contains(&"approval_policy"));
    }

    #[test]
    fn test_detect_unknown_fields_clean() {
        let raw: toml::Value = toml::from_str(
            r#"
            log_level = "info"
            api_listen = "0.0.0.0:4545"
        "#,
        )
        .unwrap();
        let unknown = KernelConfig::detect_unknown_fields(&raw);
        assert!(unknown.is_empty());
    }

    #[test]
    fn test_detect_unknown_fields_with_typos() {
        let raw: toml::Value = toml::from_str(
            r#"
            log_level = "info"
            api_listn = "0.0.0.0:4545"
            frobnicate = true
        "#,
        )
        .unwrap();
        let unknown = KernelConfig::detect_unknown_fields(&raw);
        assert_eq!(unknown.len(), 2);
        assert!(unknown.contains(&"api_listn".to_string()));
        assert!(unknown.contains(&"frobnicate".to_string()));
    }

    #[test]
    fn test_detect_unknown_fields_aliases_accepted() {
        let raw: toml::Value = toml::from_str(
            r#"
            listen_addr = "0.0.0.0:4545"
            approval_policy = {}
        "#,
        )
        .unwrap();
        let unknown = KernelConfig::detect_unknown_fields(&raw);
        assert!(unknown.is_empty());
    }

    #[test]
    fn test_known_fields_cover_real_kernelconfig_fields() {
        // Regression test for strict_config rejecting valid fields whose names
        // were never added to the hand-maintained allowlists.
        let raw: toml::Value = toml::from_str(
            r#"
            max_history_messages = 20

            [auto_dream]
            enabled = false

            [memory]
            consolidation_interval_hours = 12
            fts_only = true
            soft_delete_retention_days = 14

            [memory.decay]
            decay_interval_hours = 24

            [memory.chunking]
            enabled = true

            [proactive_memory]
            extraction_threshold = 0.7
            duplicate_threshold = 0.5
            max_memories_per_agent = 500
            extract_categories = ["preference"]

            [triggers]
            cooldown_secs = 10
        "#,
        )
        .unwrap();

        let unknown_top = KernelConfig::detect_unknown_fields(&raw);
        assert!(
            unknown_top.is_empty(),
            "real top-level fields rejected: {unknown_top:?}"
        );

        let unknown_nested = KernelConfig::detect_unknown_nested_fields(&raw);
        assert!(
            unknown_nested.is_empty(),
            "real nested fields rejected: {unknown_nested:?}"
        );
    }

    #[test]
    fn test_validate_invalid_port_string() {
        let config = KernelConfig {
            api_listen: "0.0.0.0:notaport".to_string(),
            ..Default::default()
        };
        let warnings = config.validate();
        assert!(
            warnings.iter().any(|w| w.contains("not a valid u16")),
            "expected port parse warning, got: {warnings:?}"
        );
    }

    #[test]
    fn test_validate_port_zero_warns() {
        let config = KernelConfig {
            api_listen: "0.0.0.0:0".to_string(),
            ..Default::default()
        };
        let warnings = config.validate();
        assert!(
            warnings.iter().any(|w| w.contains("port is 0")),
            "expected port-zero warning, got: {warnings:?}"
        );
    }

    #[test]
    fn test_validate_missing_port_colon() {
        let config = KernelConfig {
            api_listen: "localhost".to_string(),
            ..Default::default()
        };
        let warnings = config.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("does not contain a port")),
            "expected missing-port warning, got: {warnings:?}"
        );
    }

    #[test]
    fn test_validate_bad_log_level() {
        let config = KernelConfig {
            log_level: "verbose".to_string(),
            ..Default::default()
        };
        let warnings = config.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("not a recognised level")),
            "expected bad log_level warning, got: {warnings:?}"
        );
    }

    #[test]
    fn test_validate_good_log_levels() {
        for level in &["trace", "debug", "info", "warn", "error", "off"] {
            let config = KernelConfig {
                log_level: level.to_string(),
                ..Default::default()
            };
            let warnings = config.validate();
            assert!(
                !warnings
                    .iter()
                    .any(|w| w.contains("not a recognised level")),
                "level '{}' should be accepted, got: {:?}",
                level,
                warnings
            );
        }
    }

    #[test]
    fn test_validate_max_cron_jobs_too_large() {
        let config = KernelConfig {
            max_cron_jobs: 100_000,
            ..Default::default()
        };
        let warnings = config.validate();
        assert!(
            warnings.iter().any(|w| w.contains("max_cron_jobs")),
            "expected max_cron_jobs warning, got: {warnings:?}"
        );
    }

    #[test]
    fn test_validate_network_enabled_without_secret() {
        let config = KernelConfig {
            network_enabled: true,
            network: NetworkConfig {
                shared_secret: String::new(),
                ..Default::default()
            },
            ..Default::default()
        };
        let warnings = config.validate();
        assert!(
            warnings.iter().any(|w| w.contains("shared_secret")),
            "expected shared_secret warning, got: {warnings:?}"
        );
    }

    #[test]
    fn test_validate_default_config_no_structural_errors() {
        // Default config should only have path warnings (home_dir may not exist
        // in test environment) but no port/log_level/structural issues.
        let config = KernelConfig::default();
        let warnings = config.validate();
        for w in &warnings {
            assert!(
                !w.contains("not a valid u16"),
                "default config should have valid port"
            );
            assert!(
                !w.contains("not a recognised level"),
                "default config should have valid log_level"
            );
        }
    }

    #[test]
    fn test_thinking_config_deserialization() {
        let toml_str = r#"
            [thinking]
            budget_tokens = 20000
            stream_thinking = true
        "#;
        let config: KernelConfig = toml::from_str(toml_str).unwrap();
        let tc = config.thinking.unwrap();
        assert_eq!(tc.budget_tokens, 20000);
        assert!(tc.stream_thinking);
    }

    #[test]
    fn test_thinking_config_defaults() {
        let tc = ThinkingConfig::default();
        assert_eq!(tc.budget_tokens, 10_000);
        assert!(!tc.stream_thinking);
    }

    #[test]
    fn test_thinking_config_absent_is_none() {
        let toml_str = r#"
            log_level = "info"
        "#;
        let config: KernelConfig = toml::from_str(toml_str).unwrap();
        assert!(config.thinking.is_none());
    }

    #[test]
    fn test_plugin_manifest_config_section_deserialization() {
        let toml_str = r#"
            name = "whisper-transcribe"
            version = "0.1.0"

            [config]
            model = { type = "string", default = "small", description = "Whisper model size" }
            language = { type = "string", default = "ru", description = "Transcription language (ISO 639-1)" }
            max_file_size_mb = { type = "number", default = 10, description = "Max audio file size in MB" }
        "#;

        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.name, "whisper-transcribe");
        assert_eq!(manifest.config.len(), 3);

        let model_field = manifest.config.get("model").unwrap();
        assert_eq!(model_field.field_type, PluginConfigFieldType::String);
        assert_eq!(
            model_field.default,
            Some(serde_json::Value::String("small".to_string()))
        );
        assert_eq!(
            model_field.description.as_deref(),
            Some("Whisper model size")
        );

        let size_field = manifest.config.get("max_file_size_mb").unwrap();
        assert_eq!(size_field.field_type, PluginConfigFieldType::Number);
        assert_eq!(size_field.default, Some(serde_json::json!(10)));
    }

    #[test]
    fn test_plugin_manifest_config_section_absent_is_empty() {
        let toml_str = r#"
            name = "my-plugin"
            version = "1.0.0"
        "#;

        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.config.is_empty());
    }

    #[test]
    fn test_plugin_config_field_defaults() {
        let field = PluginConfigField::default();
        assert_eq!(field.field_type, PluginConfigFieldType::String);
        assert!(field.default.is_none());
        assert!(field.description.is_none());
    }

    #[test]
    fn test_plugin_config_field_type_serde() {
        let string_type = PluginConfigFieldType::String;
        let json = serde_json::to_string(&string_type).unwrap();
        assert_eq!(json, "\"string\"");

        let number_type = PluginConfigFieldType::Number;
        let json = serde_json::to_string(&number_type).unwrap();
        assert_eq!(json, "\"number\"");

        let bool_type = PluginConfigFieldType::Boolean;
        let json = serde_json::to_string(&bool_type).unwrap();
        assert_eq!(json, "\"boolean\"");

        let back: PluginConfigFieldType = serde_json::from_str("\"string\"").unwrap();
        assert_eq!(back, PluginConfigFieldType::String);
    }

    #[test]
    fn test_plugin_manifest_config_serde_roundtrip() {
        let mut manifest = PluginManifest {
            name: "test-plugin".to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        };
        manifest.config.insert(
            "debug".to_string(),
            PluginConfigField {
                field_type: PluginConfigFieldType::Boolean,
                default: Some(serde_json::Value::Bool(false)),
                description: Some("Enable debug mode".to_string()),
            },
        );

        let json = serde_json::to_string(&manifest).unwrap();
        let back: PluginManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "test-plugin");
        let debug_field = back.config.get("debug").unwrap();
        assert_eq!(debug_field.field_type, PluginConfigFieldType::Boolean);
        assert_eq!(debug_field.default, Some(serde_json::Value::Bool(false)));
    }
}
