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
        assert!(warnings.is_empty());
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
        };
        let json = serde_json::to_string(&uc).unwrap();
        let back: UserConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "Alice");
        assert_eq!(back.role, "owner");
        assert_eq!(back.channel_bindings.get("telegram").unwrap(), "123456");
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
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Discord"));
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
}
