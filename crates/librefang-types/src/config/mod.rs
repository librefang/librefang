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

// Maintain backward compatibility: re-export all public types.
// `serde_helpers` re-export removed alongside `OneOrMany<T>` (its
// only public symbol) — restore if a future serde helper resurfaces.
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

    /// Per-channel `proxy = "…"` round-trips through TOML on each
    /// adapter that wires it through (#4795). Absent key must yield
    /// `None`; present key must round-trip the raw string. We do NOT
    /// validate the URL here — that's the adapter's job at init.
    // test_channel_proxy_roundtrips — Mattermost case removed in the
    // sidecar migration. The remaining adapters that carry a `proxy`
    // field already have their own dedicated round-trip tests
    // alongside their config types; the original case only covered
    // mattermost.

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

    // test_validate_missing_env_vars removed — its in-process witness
    // (WhatsApp) migrated to a sidecar; the remaining in-process
    // channel configs (`google_chat`, `webhook`) keep their env-var
    // checks but no longer drive a missing-var WARN via the
    // ChannelsConfig surface this test used to exercise.

    // test_whatsapp_config_defaults / test_whatsapp_config_serde
    // removed — whatsapp migrated to a sidecar
    // (librefang.sidecar.adapters.whatsapp) and the in-process
    // WhatsAppConfig was deleted alongside the
    // `channels.whatsapp` field on ChannelsConfig.

    // test_signal_config_defaults removed — signal migrated to a
    // sidecar (librefang.sidecar.adapters.signal) and the in-process
    // SignalConfig was deleted.

    // test_matrix_config_defaults removed — matrix migrated to a
    // sidecar (librefang.sidecar.adapters.matrix) and the in-process
    // MatrixConfig was deleted.

    // test_email_config_defaults +
    // test_email_config_tls_overrides_serde_roundtrip removed —
    // email migrated to a sidecar (librefang.sidecar.adapters.email)
    // and the in-process EmailConfig was deleted alongside the
    // `[channels.email]` field on ChannelsConfig. TLS knobs
    // (`EMAIL_TLS_ROOT_CA_PATH` / `EMAIL_TLS_ACCEPT_INVALID_CERTS`)
    // now live on the sidecar's env contract; round-trip is exercised
    // by `tests/test_email_adapter.py::test_tls_accept_invalid_certs_*`.

    // test_matrix_config_serde removed — matrix migrated to a sidecar.

    #[test]
    fn test_channels_config_with_new_channels() {
        // Witness rotation history: Matrix #5368 → Email → Teams →
        // WhatsApp → Webhook → GoogleChat — all sidecar-migrated.
        // With no in-process channel left, this test now only
        // exercises that `ChannelsConfig::default()` round-trips
        // through `KernelConfig` without erroring. Re-add a
        // per-channel assertion when a future in-process channel
        // brings a witness back.
        let config = KernelConfig {
            channels: ChannelsConfig::default(),
            ..Default::default()
        };
        assert!(
            config.channels.file_download_max_bytes > 0,
            "default ChannelsConfig must populate file_download_max_bytes"
        );
    }

    // test_teams_config_defaults removed — teams migrated to a
    // sidecar (librefang.sidecar.adapters.teams) and the in-process
    // TeamsConfig was deleted.

    // test_mattermost_config_defaults removed — mattermost migrated to
    // a sidecar (librefang.sidecar.adapters.mattermost) and the
    // in-process MattermostConfig was deleted.

    // test_google_chat_config_defaults removed — google_chat
    // migrated to a sidecar (librefang.sidecar.adapters.google_chat)
    // and the in-process GoogleChatConfig was deleted.

    #[test]
    fn test_all_new_channel_configs_serde() {
        // Witness rotation history: GoogleChat → Webhook (both
        // sidecar-migrated). With no in-process channel left, the
        // serde round-trip now only exercises that the default
        // `ChannelsConfig` survives a TOML emit + reparse — adapter-
        // specific field-shape coverage moved with each migration.
        let config = KernelConfig {
            channels: ChannelsConfig::default(),
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let back: KernelConfig = toml::from_str(&toml_str).unwrap();
        assert!(
            back.channels.file_download_max_bytes > 0,
            "default ChannelsConfig must round-trip with non-zero file_download_max_bytes"
        );
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

    // test_one_or_many_* (4 tests + the local `OoMTestRow` fixture)
    // retired alongside the `OneOrMany<T>` type. With every channel
    // sidecar-migrated `OneOrMany<T>` had zero production callers
    // and was deleted from `serde_helpers.rs`. Restore from git
    // history alongside the type if a future in-process channel
    // needs the single-table-or-array-of-tables shape back.

    // test_account_id_in_channel_configs removed — its witnesses
    // (WhatsApp + WeChat + DingTalk) all migrated to sidecars. The
    // remaining in-process channel configs (google_chat) don't
    // expose `account_id` so there's nothing left to assert.

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
    fn default_routing_section_parses_and_is_known_top_level() {
        // Regression for issue #4466: the init wizard writes Smart Router
        // selections under `[default_routing]`. The field must
        // (a) deserialise into KernelConfig and (b) be on the strict-mode
        // allowlist so users running `strict_config = true` don't see a
        // bogus unknown-field warning for their own wizard output.
        let raw: toml::Value = toml::from_str(
            r#"
            [default_routing]
            simple_model = "haiku"
            medium_model = "sonnet"
            complex_model = "opus"
            simple_threshold = 100
            complex_threshold = 500
        "#,
        )
        .unwrap();

        let unknown = KernelConfig::detect_unknown_fields(&raw);
        assert!(
            unknown.is_empty(),
            "default_routing must be allowlisted: {unknown:?}"
        );

        let cfg: KernelConfig = toml::from_str(
            r#"
            [default_routing]
            simple_model = "haiku"
            medium_model = "sonnet"
            complex_model = "opus"
            simple_threshold = 100
            complex_threshold = 500
        "#,
        )
        .unwrap();
        let r = cfg
            .default_routing
            .as_ref()
            .expect("default_routing must deserialise");
        assert_eq!(r.simple_model, "haiku");
        assert_eq!(r.medium_model, "sonnet");
        assert_eq!(r.complex_model, "opus");
        assert_eq!(r.simple_threshold, 100);
        assert_eq!(r.complex_threshold, 500);
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

    /// Regression for #4298: every top-level field that issue #4298
    /// flagged as missing from the hand-maintained allowlist must be
    /// accepted now that the allowlist is derived from `KernelConfig`'s
    /// JSON Schema.
    #[test]
    fn test_known_top_level_fields_cover_issue_4298_gaps() {
        let known: std::collections::HashSet<&str> = KernelConfig::known_top_level_fields()
            .iter()
            .copied()
            .collect();
        for field in [
            "agent_max_iterations",
            "allowed_mount_roots",
            "channel_role_mapping",
            "llm",
            "local_probe_interval_secs",
            "parallel_tools",
            "provider_request_timeout_secs",
            "require_auth_for_reads",
            "taint_rules",
            "tool_invoke",
            "trusted_hosts",
            "trusted_manifest_signers",
            "workflow_stale_timeout_minutes",
        ] {
            assert!(
                known.contains(field),
                "issue #4298 field `{field}` not in known_top_level_fields()"
            );
        }
    }

    /// Drift sentinel for #4298: every field that appears at the top
    /// level of a default-serialized `KernelConfig` must also appear in
    /// `known_top_level_fields()`. Since the allowlist is derived from
    /// the JSON Schema (which is generated by the same struct
    /// definition), this should hold automatically — the test exists
    /// to fail loudly if the derivation regresses.
    #[test]
    fn test_known_top_level_fields_match_serialized_default() {
        let raw = toml::Value::try_from(KernelConfig::default())
            .expect("KernelConfig default must serialize to TOML");
        let serialized: Vec<&str> = match &raw {
            toml::Value::Table(tbl) => tbl.keys().map(|s| s.as_str()).collect(),
            _ => panic!("KernelConfig must serialize as a TOML table"),
        };
        let known: std::collections::HashSet<&str> = KernelConfig::known_top_level_fields()
            .iter()
            .copied()
            .collect();
        for field in serialized {
            assert!(
                known.contains(field),
                "field `{field}` is emitted by KernelConfig::default() but is not in \
                 known_top_level_fields() — schema-derived allowlist drifted"
            );
        }
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

    // ---------------------------------------------------------------
    // #5129 — nested `serde(alias)` declarations must stay on the
    // strict-mode allowlist. Before the fix, schemars' JSON Schema
    // dropped `alias = "trust_proxy_headers"` on
    // `TerminalConfig.require_proxy_headers`, so strict_config = true
    // rejected the legacy spelling even though serde would have
    // accepted it.
    // ---------------------------------------------------------------

    #[test]
    fn strict_config_accepts_nested_serde_alias_5129() {
        let raw: toml::Value = toml::from_str(
            r#"
            strict_config = true

            [terminal]
            trust_proxy_headers = true
            "#,
        )
        .expect("toml parse");

        let unknown_top = KernelConfig::detect_unknown_fields(&raw);
        let unknown_nested = KernelConfig::detect_unknown_nested_fields(&raw);
        assert!(
            unknown_top.is_empty(),
            "top-level rejected: {unknown_top:?}",
        );
        assert!(
            unknown_nested.is_empty(),
            "nested rejected: {unknown_nested:?} — `trust_proxy_headers` is a serde(alias) for `require_proxy_headers`",
        );

        // And serde itself must still honour the alias on the way into
        // the struct — otherwise the allowlist agrees but the value
        // never lands.
        let cfg: KernelConfig = toml::from_str(
            r#"
            [terminal]
            trust_proxy_headers = true
            "#,
        )
        .expect("alias must deserialise");
        assert!(cfg.terminal.require_proxy_headers);
    }

    // ---------------------------------------------------------------
    // #5130 — typos inside repeated tables ([[mcp_servers]], …) used
    // to be silently dropped because the strict-mode walker only
    // descended into single-table paths. `deny_unknown_fields` on the
    // per-element struct catches them at serde-deserialize time,
    // regardless of repeated-vs-single shape.
    //
    // strict_config_rejects_typo_in_repeated_channel_table_5130
    // (originally on DiscordConfig / SlackConfig /
    // MattermostConfig / WhatsAppConfig / WebhookConfig) removed —
    // every per-channel struct that carried `deny_unknown_fields`
    // has migrated to a sidecar. McpServerConfigEntry is the only
    // remaining locked-down per-element struct.
    // ---------------------------------------------------------------

    #[test]
    fn strict_config_rejects_typo_in_repeated_mcp_servers_table_5130() {
        let toml_src = r#"
            [[mcp_servers]]
            name = "filesystem"
            # Typo: should be `timeout_secs`.
            timout_secs = 30
        "#;
        let err = toml::from_str::<KernelConfig>(toml_src)
            .expect_err("typo inside [[mcp_servers]] must be rejected by deny_unknown_fields");
        let msg = err.to_string();
        assert!(
            msg.contains("timout_secs") || msg.contains("unknown field"),
            "error must mention the offending field, got: {msg}",
        );
    }

    #[test]
    fn well_formed_repeated_mcp_servers_table_still_parses_5130() {
        // Drift sentinel: deny_unknown_fields must not regress the
        // happy path. If a future refactor renames a field on
        // McpServerConfigEntry without updating this fixture, the
        // test will fail loudly. (DiscordConfig, SlackConfig,
        // MattermostConfig, WhatsAppConfig, WebhookConfig were in
        // this set originally; all migrated to sidecars by v2026.5.)
        let cfg: KernelConfig = toml::from_str(
            r#"
            [[mcp_servers]]
            name = "filesystem"
            timeout_secs = 30
            "#,
        )
        .expect("well-formed repeated tables must still parse with deny_unknown_fields");
        assert_eq!(cfg.mcp_servers.len(), 1);
    }

    // ---------------------------------------------------------------
    // #5476 — `[agents.<name>.<override_key>]` blocks in config.toml
    // are silently ignored because `KernelConfig` has no `agents`
    // field. The detector lists each (agent, key) pair so the kernel
    // can emit a targeted warning pointing at agent.toml.
    // ---------------------------------------------------------------

    #[test]
    fn detect_misplaced_per_agent_overrides_flags_proactive_memory_5476() {
        let raw: toml::Value = toml::from_str(
            r#"
            [proactive_memory]
            enabled = true

            [agents.my-agent.proactive_memory]
            auto_memorize = true
            "#,
        )
        .expect("toml parse");
        let found = KernelConfig::detect_misplaced_per_agent_overrides(&raw);
        assert_eq!(
            found,
            vec![("my-agent".to_string(), "proactive_memory".to_string())]
        );
    }

    #[test]
    fn detect_misplaced_per_agent_overrides_handles_multiple_agents_5476() {
        let raw: toml::Value = toml::from_str(
            r#"
            [agents.beta.skill_workshop]
            enabled = true

            [agents.alpha.proactive_memory]
            auto_memorize = true

            [agents.alpha.compaction]
            keep_recent = 20
            "#,
        )
        .expect("toml parse");
        let found = KernelConfig::detect_misplaced_per_agent_overrides(&raw);
        // Sorted: (alpha, compaction), (alpha, proactive_memory), (beta, skill_workshop)
        assert_eq!(
            found,
            vec![
                ("alpha".to_string(), "compaction".to_string()),
                ("alpha".to_string(), "proactive_memory".to_string()),
                ("beta".to_string(), "skill_workshop".to_string()),
            ]
        );
    }

    #[test]
    fn detect_misplaced_per_agent_overrides_empty_without_agents_section_5476() {
        let raw: toml::Value = toml::from_str(
            r#"
            log_level = "debug"

            [proactive_memory]
            enabled = true
            "#,
        )
        .expect("toml parse");
        assert!(KernelConfig::detect_misplaced_per_agent_overrides(&raw).is_empty());
    }

    #[test]
    fn detect_misplaced_per_agent_overrides_ignores_unrelated_agent_keys_5476() {
        // Generic typos under `[agents.<name>]` should NOT be flagged
        // by this detector — they're caught (less specifically) by
        // the unknown-top-level pass. This detector exists only to
        // give actionable guidance for the override keys operators
        // actually try to set per-agent.
        let raw: toml::Value = toml::from_str(
            r#"
            [agents.my-agent.some_random_typo]
            value = 1
            "#,
        )
        .expect("toml parse");
        assert!(KernelConfig::detect_misplaced_per_agent_overrides(&raw).is_empty());
    }

    #[test]
    fn detect_misplaced_per_agent_overrides_flags_section_overrides_6() {
        // The four section-overrides that #5476 originally missed:
        // `thinking`, `exec_policy`, `max_history_messages`, and the
        // tool-exec backend (both the `tool_exec` global-section spelling
        // and the `tool_exec_backend` agent.toml spelling).
        let raw: toml::Value = toml::from_str(
            r#"
            [agents.a.thinking]
            enabled = true

            [agents.a.exec_policy]
            mode = "allowlist"

            [agents.b]
            max_history_messages = 80
            tool_exec_backend = "ssh"

            [agents.b.tool_exec]
            kind = "ssh"
            "#,
        )
        .expect("toml parse");
        let found = KernelConfig::detect_misplaced_per_agent_overrides(&raw);
        assert_eq!(
            found,
            vec![
                ("a".to_string(), "exec_policy".to_string()),
                ("a".to_string(), "thinking".to_string()),
                ("b".to_string(), "max_history_messages".to_string()),
                ("b".to_string(), "tool_exec".to_string()),
                ("b".to_string(), "tool_exec_backend".to_string()),
            ]
        );
    }

    /// Drift guard (#6): every `AgentManifest` field that acts as a
    /// per-agent override of a global `KernelConfig` section must be listed
    /// in `PER_AGENT_OVERRIDE_KEYS`, so `detect_misplaced_per_agent_overrides`
    /// emits the targeted "move this to agent.toml" warning for it.
    ///
    /// The original #5476 list was hand-maintained and silently drifted —
    /// it covered only `proactive_memory` / `skill_workshop` / `compaction`
    /// and missed `thinking`, `exec_policy`, `max_history_messages`, and the
    /// tool-exec backend. This test makes that class of omission a compile
    /// error: it exhaustively **destructures** `AgentManifest`, so adding a
    /// field to the struct fails to compile here until the contributor
    /// classifies it as either an override (→ add the config.toml-facing key
    /// to `PER_AGENT_OVERRIDE_KEYS`) or non-override (→ list it in `OTHER`
    /// below with the reason).
    ///
    /// We destructure rather than read a schema because `AgentManifest` does
    /// not derive `JsonSchema` (it would cascade the derive onto ~15 nested
    /// types) and a `serde_json::to_value(&default)` field walk would drop
    /// every `skip_serializing_if = "Option::is_none"` field that defaults to
    /// `None` — exactly the override fields we care about (`compaction`,
    /// `tool_exec_backend`). The exhaustive pattern sees every field
    /// regardless of serde attributes.
    #[test]
    fn per_agent_override_keys_cover_manifest_overrides_6() {
        use super::validation::PER_AGENT_OVERRIDE_KEYS;
        use crate::agent::AgentManifest;
        use std::collections::BTreeSet;

        // Exhaustive destructure: if a field is added to `AgentManifest`,
        // this stops compiling until the new binding is added to exactly one
        // of the two arms below. The bindings are otherwise unused.
        #[allow(unused_variables)]
        let AgentManifest {
            // --- Per-agent overrides of a global KernelConfig section. ---
            // Each maps to its config.toml-facing key in `expected_override_keys`.
            proactive_memory,
            skill_workshop,
            compaction,
            thinking,
            exec_policy,
            max_history_messages,
            tool_exec_backend,

            // --- OTHER: not a global-section override. -------------------
            // These are agent-only settings with no matching global
            // KernelConfig section to override (or are pure identity /
            // wiring), so placing them under `[agents.x.…]` in config.toml
            // is still a no-op but is intentionally NOT given the targeted
            // #5476 warning — it falls through to the generic unknown-key
            // pass instead. The detector is deliberately scoped to the
            // overrides operators actually try to relocate.
            name,
            version,
            description,
            author,
            module,
            schedule,
            session_mode,
            model,
            fallback_models,
            resources,
            priority,
            capabilities,
            profile,
            tools,
            skills,
            skills_disabled,
            mcp_servers,
            channels,
            mcp_disabled,
            metadata,
            tags,
            routing,
            autonomous,
            pinned_model,
            workspace,
            generate_identity_files,
            workspaces,
            tool_allowlist,
            tool_blocklist,
            tools_disabled,
            response_format,
            enabled,
            allowed_plugins,
            inherit_parent_context,
            context_injection,
            is_hand,
            web_search_augmentation,
            auto_dream_enabled,
            auto_dream_min_hours,
            auto_dream_min_sessions,
            show_progress,
            auto_evolve,
            channel_overrides,
            max_concurrent_invocations,
            cache_context,
            triggers,
            reconcile_orphans,
            async_tasks,
        } = AgentManifest::default();

        // The config.toml-facing keys each override field should be flagged
        // under. Most equal the manifest field name; `tool_exec_backend`
        // also surfaces under the global section spelling `tool_exec`
        // because that is what an operator copies from `[tool_exec]`.
        let expected: BTreeSet<&str> = [
            "proactive_memory",
            "skill_workshop",
            "compaction",
            "thinking",
            "exec_policy",
            "max_history_messages",
            "tool_exec",
            "tool_exec_backend",
        ]
        .into_iter()
        .collect();

        let actual: BTreeSet<&str> = PER_AGENT_OVERRIDE_KEYS.iter().copied().collect();

        let missing: Vec<&str> = expected.difference(&actual).copied().collect();
        assert!(
            missing.is_empty(),
            "PER_AGENT_OVERRIDE_KEYS is missing override keys {missing:?}; \
             an AgentManifest section-override field is not flagged by \
             detect_misplaced_per_agent_overrides (#6)"
        );

        let stale: Vec<&str> = actual.difference(&expected).copied().collect();
        assert!(
            stale.is_empty(),
            "PER_AGENT_OVERRIDE_KEYS lists keys {stale:?} that no longer map \
             to an AgentManifest section-override field (renamed/removed?)"
        );
    }
}
