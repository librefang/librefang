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
    fn groups_default_to_empty_and_are_omitted_from_serialized_config() {
        let cfg = KernelConfig::default();
        assert!(cfg.groups.is_empty());
        let serialized = toml::to_string(&cfg).unwrap();
        assert!(
            !serialized.contains("[[groups]]"),
            "an empty group list must not leave a stranded [[groups]] section behind"
        );
    }

    #[test]
    fn groups_round_trip_through_toml() {
        let toml_str = r#"
            [[groups]]
            name = "oncall"
            description = "Support rota"
            members = ["alice", "bob"]
            roles = ["approver"]
        "#;
        let cfg: KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.groups.len(), 1);
        assert_eq!(cfg.groups[0].name, "oncall");
        assert_eq!(cfg.groups[0].description, "Support rota");
        assert!(cfg.groups[0].has_member("alice"));
        assert!(!cfg.groups[0].has_member("carol"));
    }

    #[test]
    fn groups_omitted_optional_fields_default_to_empty() {
        let toml_str = r#"
            [[groups]]
            name = "minimal"
        "#;
        let cfg: KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.groups[0].description, "");
        assert!(cfg.groups[0].members.is_empty());
        assert!(cfg.groups[0].roles.is_empty());
    }

    #[test]
    fn groups_for_user_sorts_by_name_regardless_of_declaration_order() {
        let cfg = KernelConfig {
            groups: vec![
                crate::config::GroupConfig {
                    name: "zulu".to_string(),
                    members: vec!["alice".to_string()],
                    ..Default::default()
                },
                crate::config::GroupConfig {
                    name: "alpha".to_string(),
                    members: vec!["alice".to_string()],
                    ..Default::default()
                },
                crate::config::GroupConfig {
                    name: "other".to_string(),
                    members: vec!["bob".to_string()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let names: Vec<&str> = cfg
            .groups_for_user("alice")
            .into_iter()
            .map(|g| g.name.as_str())
            .collect();
        assert_eq!(names, vec!["alpha", "zulu"]);
        assert!(cfg.groups_for_user("nobody").is_empty());
    }

    #[test]
    fn roles_for_user_unions_group_names_and_declared_roles() {
        let cfg = KernelConfig {
            groups: vec![
                crate::config::GroupConfig {
                    name: "oncall".to_string(),
                    members: vec!["alice".to_string()],
                    roles: vec!["approver".to_string()],
                    ..Default::default()
                },
                crate::config::GroupConfig {
                    name: "billing".to_string(),
                    members: vec!["alice".to_string()],
                    // `approver` is conferred twice; the set collapses it.
                    roles: vec!["approver".to_string(), "auditor".to_string()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let roles: Vec<String> = cfg.roles_for_user("alice").into_iter().collect();
        assert_eq!(roles, vec!["approver", "auditor", "billing", "oncall"]);
        assert!(cfg.roles_for_user("bob").is_empty());
    }

    #[test]
    fn roles_for_user_does_not_leak_the_rbac_privilege_level() {
        // `UserConfig.role` is a different ladder. A group named `owner` must
        // not silently confer owner privilege, and the RBAC role must not show
        // up in the group-derived set either.
        //
        // #7746 UPDATE — kept, and widened rather than relaxed.
        //
        // #7745 wrote this test to pin a separation it described as temporary:
        // "connecting the two ladders is #7746's job, under an operator-defined
        // mapping". #7746 has landed and the separation still holds, because the
        // connection it built is not a promotion path from group membership to
        // privilege. `[external_auth.role_map]` and `[external_auth.group_map]`
        // are two independent operator-written maps over the same IdP claim
        // values: the first confers a `UserRole`, the second confers membership,
        // and an operator who wants one claim to do both writes it into both
        // maps deliberately. Nothing derives privilege from a group name.
        //
        // So the assertion is unchanged and the coverage is extended to the arm
        // that did not exist when it was written — `effective_roles_for` with an
        // IdP-derived membership must leak the ladder no more than
        // `roles_for_user` does, which is the property that would silently
        // invert if a later change decided a group could carry a `UserRole`.
        let cfg = KernelConfig {
            users: vec![UserConfig {
                name: "alice".to_string(),
                role: "admin".to_string(),
                ..Default::default()
            }],
            groups: vec![
                crate::config::GroupConfig {
                    name: "oncall".to_string(),
                    members: vec!["alice".to_string()],
                    ..Default::default()
                },
                // Named for a privilege level on purpose: the worst-case group
                // name an identity provider could hand us.
                crate::config::GroupConfig {
                    name: "owner".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let roles = cfg.roles_for_user("alice");
        assert!(roles.contains("oncall"));
        assert!(!roles.contains("admin"));

        let idp = std::collections::BTreeSet::from(["owner".to_string()]);
        let effective = cfg.effective_roles_for("alice", &idp);
        // The IdP-conferred group is present as a *role string* …
        assert!(effective.contains("owner"));
        // … and the caller's RBAC level is still nowhere in the set. The
        // `owner` here is the name of a team, and the only thing that reads it
        // is channel-binding resolution; `UserRole` is resolved by
        // `AuthManager` from `UserConfig.role` and `role_map`, neither of which
        // this function touches.
        assert!(!effective.contains("admin"));
        assert_eq!(cfg.users[0].role, "admin");
    }

    #[test]
    fn effective_roles_for_matches_roles_for_user_when_no_claims_are_present() {
        // Every non-OIDC credential path calls the effective resolvers with an
        // empty claim set, so the two must agree exactly there — otherwise
        // #7746 would have quietly changed what a plain API-key caller resolves
        // to.
        let cfg = KernelConfig {
            groups: vec![
                crate::config::GroupConfig {
                    name: "oncall".to_string(),
                    members: vec!["alice".to_string()],
                    roles: vec!["approver".to_string()],
                    ..Default::default()
                },
                crate::config::GroupConfig {
                    name: "billing".to_string(),
                    members: vec!["bob".to_string()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let none = std::collections::BTreeSet::new();
        for user in ["alice", "bob", "nobody"] {
            assert_eq!(
                cfg.effective_roles_for(user, &none),
                cfg.roles_for_user(user),
                "effective_roles_for({user}) must reduce to roles_for_user with no claims"
            );
            let declared: std::collections::BTreeSet<String> = cfg
                .groups_for_user(user)
                .into_iter()
                .map(|g| g.name.clone())
                .collect();
            assert_eq!(cfg.effective_groups_for(user, &none), declared);
        }
    }

    #[test]
    fn effective_groups_union_declared_membership_with_idp_claims() {
        // The precedence decision, pinned: a set has no ladder, so both grants
        // survive and neither can retract the other. In particular the IdP
        // dropping `oncall` does not remove alice from a `members` list an
        // operator typed — see `ExternalAuthConfig::group_map`.
        let cfg = KernelConfig {
            groups: vec![
                crate::config::GroupConfig {
                    name: "oncall".to_string(),
                    members: vec!["alice".to_string()],
                    roles: vec!["approver".to_string()],
                    ..Default::default()
                },
                crate::config::GroupConfig {
                    name: "compliance".to_string(),
                    roles: vec!["auditor".to_string()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        // Declared only.
        let none = std::collections::BTreeSet::new();
        assert_eq!(
            cfg.effective_groups_for("alice", &none),
            std::collections::BTreeSet::from(["oncall".to_string()]),
        );

        // Declared ∪ claimed, with the claimed half overlapping the declared
        // half — the union collapses the duplicate rather than double-counting.
        let claimed =
            std::collections::BTreeSet::from(["oncall".to_string(), "compliance".to_string()]);
        assert_eq!(
            cfg.effective_groups_for("alice", &claimed),
            std::collections::BTreeSet::from(["compliance".to_string(), "oncall".to_string()]),
        );
        assert_eq!(
            cfg.effective_roles_for("alice", &claimed),
            std::collections::BTreeSet::from([
                "approver".to_string(),
                "auditor".to_string(),
                "compliance".to_string(),
                "oncall".to_string(),
            ]),
        );

        // Claimed only, for a user with no `[[groups]]` entry at all — the
        // deployment where every membership comes from the IdP.
        assert_eq!(
            cfg.effective_groups_for("carol", &claimed),
            std::collections::BTreeSet::from(["compliance".to_string(), "oncall".to_string()]),
        );
    }

    #[test]
    fn effective_groups_drop_names_that_match_no_declared_group() {
        // Belt and braces behind `translate_oidc_groups`: the returned set
        // promises "names of groups that exist", and a phantom name here would
        // become a `Principal::group_named` that owns things and that no
        // operator can see in `[[groups]]`.
        let cfg = KernelConfig {
            groups: vec![crate::config::GroupConfig {
                name: "oncall".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let claimed = std::collections::BTreeSet::from(["oncall".to_string(), "ghost".to_string()]);
        assert_eq!(
            cfg.effective_groups_for("alice", &claimed),
            std::collections::BTreeSet::from(["oncall".to_string()]),
        );
        assert!(!cfg.effective_roles_for("alice", &claimed).contains("ghost"));
    }

    #[test]
    fn external_auth_group_mapping_defaults_are_off_and_scope_is_not_read() {
        let cfg = KernelConfig::default();
        assert!(
            cfg.external_auth.group_map.is_empty(),
            "group mapping must be opt-in: an IdP claim grants no membership until an operator writes the map"
        );
        assert_eq!(
            cfg.external_auth.claim_paths,
            vec!["roles".to_string(), "groups".to_string()],
            "the default claim paths are the two identity assertions; `scope` describes what a client app was granted and is an explicit opt-in"
        );
    }

    #[test]
    fn external_auth_group_mapping_round_trips_through_toml() {
        let toml_str = r#"
            [external_auth]
            enabled = true
            claim_paths = ["realm_access.roles", "resource_access.<client>.roles", "groups"]

            [external_auth.group_map]
            "platform-oncall" = "oncall"

            [external_auth.role_map]
            "librefang-owners" = "owner"
        "#;
        let cfg: KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            cfg.external_auth.group_map.get("platform-oncall"),
            Some(&"oncall".to_string())
        );
        assert_eq!(
            cfg.external_auth.claim_paths,
            vec![
                "realm_access.roles".to_string(),
                "resource_access.<client>.roles".to_string(),
                "groups".to_string(),
            ]
        );
        // The two maps are independent: declaring one must not disturb the other.
        assert_eq!(
            cfg.external_auth.role_map.get("librefang-owners"),
            Some(&"owner".to_string())
        );
    }

    #[test]
    fn default_owner_defaults_to_none_and_is_omitted_from_serialized_config() {
        let cfg = KernelConfig::default();
        assert!(cfg.default_owner.is_none());
        assert!(cfg.default_owner_principal().is_none());
        let serialized = toml::to_string(&cfg).unwrap();
        assert!(!serialized.contains("default_owner"));
    }

    #[test]
    fn default_owner_parses_a_group_spec() {
        let cfg: KernelConfig = toml::from_str("default_owner = \"group:platform\"").unwrap();
        assert_eq!(
            cfg.default_owner_principal(),
            Some(crate::principal::Principal::group_named("platform"))
        );
    }

    #[test]
    fn a_malformed_default_owner_resolves_to_none_rather_than_failing_the_load() {
        // A typo in this key must leave artifacts unowned — recoverable —
        // rather than refusing to start the daemon.
        let cfg: KernelConfig = toml::from_str("default_owner = \"role:admin\"").unwrap();
        assert_eq!(cfg.default_owner, Some("role:admin".to_string()));
        assert_eq!(cfg.default_owner_principal(), None);
    }

    #[test]
    fn principal_name_resolves_both_arms_back_to_their_declared_name() {
        use crate::principal::Principal;
        let cfg = KernelConfig {
            users: vec![UserConfig {
                name: "alice".to_string(),
                ..Default::default()
            }],
            groups: vec![crate::config::GroupConfig {
                name: "oncall".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            cfg.principal_name(&Principal::user_named("alice")),
            Some("alice")
        );
        assert_eq!(
            cfg.principal_name(&Principal::group_named("oncall")),
            Some("oncall")
        );
        // A user and a group sharing a name must not resolve through each
        // other — the per-kind namespace is what keeps a single owner column
        // unambiguous.
        assert_eq!(cfg.principal_name(&Principal::group_named("alice")), None);
        assert_eq!(cfg.principal_name(&Principal::user_named("oncall")), None);
        // A principal whose declaration has since been removed is `None`, not
        // an error: callers render the canonical `kind:uuid` string instead.
        assert_eq!(cfg.principal_name(&Principal::user_named("carol")), None);
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
        // #6445: an unset dm/group policy is `None`, NOT the enum default.
        // `None` gates nothing (DMs flow, all group messages processed), whereas `Some(GroupPolicy::MentionOnly)` — the old default — silently dropped non-mention group traffic whenever any single override field was written.
        assert_eq!(ov.dm_policy, None);
        assert_eq!(ov.group_policy, None);
        assert!(ov.group_trigger_patterns.is_empty());
        assert_eq!(ov.rate_limit_per_user, 0);
        assert!(!ov.threading);
        assert!(ov.output_format.is_none());
        assert!(ov.model.is_none());
        assert!(ov.thread_ownership_enabled);
        assert_eq!(ov.conversation_ownership_ttl_seconds, 600);
        assert!(!ov.conversation_ownership_include_dms);
    }

    #[test]
    fn test_conversation_ownership_knobs_roundtrip() {
        let toml_str = r#"
            conversation_ownership_ttl_seconds = 120
            conversation_ownership_include_dms = true
        "#;
        let ov: ChannelOverrides = toml::from_str(toml_str).unwrap();
        assert_eq!(ov.conversation_ownership_ttl_seconds, 120);
        assert!(ov.conversation_ownership_include_dms);
        // Unset knobs keep their #5323 defaults.
        let bare: ChannelOverrides = toml::from_str("").unwrap();
        assert_eq!(bare.conversation_ownership_ttl_seconds, 600);
        assert!(!bare.conversation_ownership_include_dms);
    }

    #[test]
    fn absent_group_and_dm_policy_deserialize_to_none_6445() {
        // #6445: writing an unrelated field must NOT materialize a policy the operator never set.
        // A `[channel_overrides]` table that mentions only `threading` leaves both policies `None` (no gating), instead of the pre-fix behaviour where `group_policy` silently became `MentionOnly`.
        let partial: ChannelOverrides = toml::from_str("threading = true").unwrap();
        assert!(partial.threading);
        assert_eq!(partial.group_policy, None);
        assert_eq!(partial.dm_policy, None);

        // An explicitly written policy still round-trips to `Some(_)`.
        let explicit: ChannelOverrides =
            toml::from_str("group_policy = \"all\"\ndm_policy = \"ignore\"").unwrap();
        assert_eq!(explicit.group_policy, Some(GroupPolicy::All));
        assert_eq!(explicit.dm_policy, Some(DmPolicy::Ignore));
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
            dm_policy: Some(DmPolicy::Ignore),
            group_policy: Some(GroupPolicy::CommandsOnly),
            group_trigger_patterns: vec!["(?i)\\bbot\\b".to_string()],
            rate_limit_per_user: 10,
            threading: true,
            output_format: Some(OutputFormat::TelegramHtml),
            ..Default::default()
        };
        let json = serde_json::to_string(&ov).unwrap();
        let back: ChannelOverrides = serde_json::from_str(&json).unwrap();
        assert_eq!(back.dm_policy, Some(DmPolicy::Ignore));
        assert_eq!(back.group_policy, Some(GroupPolicy::CommandsOnly));
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

    // ── Reasoning mode (#7946) ─────────────────────────────────────────

    /// The global `[thinking]` table must accept `reasoning_mode` from TOML.
    /// A missing `#[serde(default)]` would make the whole table fail to parse;
    /// a missing struct field would make the key an unknown-field error or a
    /// silent drop, depending on the deserializer.
    #[test]
    fn test_thinking_config_parses_reasoning_mode_from_toml() {
        for (literal, expected) in [
            ("none", ReasoningMode::None),
            ("low", ReasoningMode::Low),
            ("high", ReasoningMode::High),
            ("max", ReasoningMode::Max),
        ] {
            let toml_str = format!(
                r#"
                log_level = "info"
                [thinking]
                budget_tokens = 2048
                reasoning_mode = "{literal}"
                "#
            );
            let config: KernelConfig = toml::from_str(&toml_str).unwrap();
            let tc = config.thinking.expect("thinking table parsed");
            assert_eq!(tc.reasoning_mode, Some(expected), "literal {literal:?}");
            assert_eq!(tc.budget_tokens, 2048, "literal {literal:?}");
        }
    }

    /// A `[thinking]` table that omits `reasoning_mode` still parses, and the
    /// field defaults to "no explicit mode" — i.e. the pre-#7946 budget-bucket
    /// behaviour, not a mode of its own.
    #[test]
    fn test_thinking_config_reasoning_mode_defaults_to_none() {
        let toml_str = r#"
            log_level = "info"
            [thinking]
            budget_tokens = 8192
        "#;
        let config: KernelConfig = toml::from_str(toml_str).unwrap();
        let tc = config.thinking.expect("thinking table parsed");
        assert_eq!(tc.reasoning_mode, None);
        assert_eq!(ThinkingConfig::default().reasoning_mode, None);
    }

    /// An unrecognised mode is an error, not a silent fallback to the default.
    #[test]
    fn test_thinking_config_rejects_an_unknown_reasoning_mode() {
        let toml_str = r#"
            log_level = "info"
            [thinking]
            reasoning_mode = "medium"
        "#;
        assert!(
            toml::from_str::<KernelConfig>(toml_str).is_err(),
            "`medium` is reachable only as the budget-bucket fallback and is not a settable mode",
        );
    }

    /// The serde spelling is the documented spelling.
    #[test]
    fn test_reasoning_mode_serde_spelling_matches_as_str() {
        for mode in [
            ReasoningMode::None,
            ReasoningMode::Low,
            ReasoningMode::High,
            ReasoningMode::Max,
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!("\"{}\"", mode.as_str()));
            let back: ReasoningMode = serde_json::from_str(&json).unwrap();
            assert_eq!(back, mode);
        }
    }

    /// `ThinkingOverride::resolve` is the per-call rung: the explicit mode wins
    /// over the legacy boolean whenever both are present.
    #[test]
    fn test_thinking_override_resolve_precedence() {
        assert_eq!(
            ThinkingOverride::resolve(None, None),
            ThinkingOverride::Inherit
        );
        assert_eq!(
            ThinkingOverride::resolve(Some(true), None),
            ThinkingOverride::Enable
        );
        assert_eq!(
            ThinkingOverride::resolve(Some(false), None),
            ThinkingOverride::Disable
        );
        assert_eq!(
            ThinkingOverride::resolve(None, Some(ReasoningMode::Max)),
            ThinkingOverride::Mode(ReasoningMode::Max)
        );
        for boolean in [None, Some(true), Some(false)] {
            assert_eq!(
                ThinkingOverride::resolve(boolean, Some(ReasoningMode::Low)),
                ThinkingOverride::Mode(ReasoningMode::Low),
                "reasoning_mode must win over thinking={boolean:?}",
            );
        }
        // The `From<Option<bool>>` shim every pre-#7946 call site goes through.
        assert_eq!(ThinkingOverride::from(Some(true)), ThinkingOverride::Enable);
        assert_eq!(ThinkingOverride::from(None), ThinkingOverride::Inherit);
        assert_eq!(ThinkingOverride::default(), ThinkingOverride::Inherit);
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
    // #6612 — the parent `McpServerConfigEntry` carried `deny_unknown_fields` but `McpTransportEntry` one level down did not, so an unsupported key inside `[mcp_servers.transport]` was accepted and dropped while the identical typo on the parent failed the load.
    // These tests pin the guard on the enum and on the two HttpCompat structs below it, and pin the happy path for all four transport variants so the guard cannot regress a working config.
    // ---------------------------------------------------------------

    #[test]
    fn mcp_transport_rejects_unknown_key_in_transport_table_6612() {
        // The reporter's exact shape: a valid stdio transport plus a nested `env` table.
        // The env-passing mechanism that does exist is the parent's `env: Vec<String>` name list, so this table was silently discarded and the subprocess ran on its script's hardcoded fallback defaults.
        let toml_src = r#"
            [[mcp_servers]]
            name = "local-thing"
            timeout_secs = 30

            [mcp_servers.transport]
            command = "/usr/bin/python3"
            args = ["/opt/mcp/server.py"]
            type = "stdio"

            [mcp_servers.transport.env]
            MY_API_KEY = "secret"
            MY_API_URL = "https://example.invalid"
        "#;
        let err = toml::from_str::<KernelConfig>(toml_src).expect_err(
            "an unsupported table under [mcp_servers.transport] must be rejected, not dropped",
        );
        let msg = err.to_string();
        // Match the serde diagnostic, not the bare key name.
        // `toml`'s Display echoes the offending span of the input, so a `contains("env")` assertion would also be satisfied by the source text and would keep passing if the guard were removed.
        assert!(
            msg.contains("unknown field `env`"),
            "error must name the offending key so the operator can find it, got: {msg}",
        );
    }

    #[test]
    fn mcp_transport_rejects_unknown_scalar_on_url_variants_6612() {
        // `deny_unknown_fields` on an internally-tagged enum is applied per variant, so the non-stdio variants need their own coverage — a guard that only bound the first variant would still pass the test above.
        for (variant, url_key) in [("sse", "url"), ("http", "url")] {
            let toml_src = format!(
                r#"
                [[mcp_servers]]
                name = "remote"
                transport = {{ type = "{variant}", {url_key} = "https://example.invalid/mcp", timout_secs = 30 }}
                "#
            );
            let err = toml::from_str::<KernelConfig>(&toml_src).unwrap_err();
            assert!(
                err.to_string().contains("unknown field `timout_secs`"),
                "{variant} transport must reject an unknown key, got: {err}",
            );
        }
    }

    #[test]
    fn http_compat_transport_rejects_keys_outside_its_own_field_set_6612() {
        // The `HttpCompat` variant's own field set is `{base_url, headers, tools}`, and the two tests below it only reach the nested structs — a guard that bound every variant except this one would still pass all of them.
        // `command` is the interesting case rather than a nonsense word: it is a real field of the *stdio* variant, so before the guard an operator who changed `type` from `stdio` to `http_compat` and left the old key behind got it silently dropped, with no hint that the line was now inert.
        let stale_stdio_key = r#"
            [[mcp_servers]]
            name = "compat"

            [mcp_servers.transport]
            type = "http_compat"
            base_url = "https://example.invalid"
            command = "/usr/bin/mcp"
        "#;
        let err = toml::from_str::<KernelConfig>(stale_stdio_key)
            .expect_err("a field belonging to a different variant must be rejected, not dropped");
        assert!(
            err.to_string().contains("unknown field `command`"),
            "error must name the stale key, got: {err}",
        );

        // A misspelled `base_url` is the failure that hurts most: `base_url` is the one required field, so the entry cannot fall back to a default and the error has to name the key rather than reporting a missing field the operator believes they wrote.
        let misspelled_required = r#"
            [[mcp_servers]]
            name = "compat"

            [mcp_servers.transport]
            type = "http_compat"
            base_urls = "https://example.invalid"
        "#;
        let err = toml::from_str::<KernelConfig>(misspelled_required)
            .expect_err("a misspelled base_url must be rejected");
        assert!(
            err.to_string().contains("unknown field `base_urls`"),
            "error must name the misspelled key rather than only reporting `base_url` missing, got: {err}",
        );
    }

    #[test]
    fn mcp_transport_rejects_unknown_key_in_http_compat_headers_and_tools_6612() {
        // `[[mcp_servers.transport.headers]]` and `[[mcp_servers.transport.tools]]` are arrays of tables nested inside an array of tables.
        // Every field on both structs except `name` / `path` has a `#[serde(default)]`, so an unguarded typo leaves the entry wired to a default rather than the operator's intent.
        let header_typo = r#"
            [[mcp_servers]]
            name = "compat"

            [mcp_servers.transport]
            type = "http_compat"
            base_url = "https://example.invalid"

            [[mcp_servers.transport.headers]]
            name = "Authorization"
            value_from_env = "TOKEN"
        "#;
        let err = toml::from_str::<KernelConfig>(header_typo)
            .expect_err("a misspelled header field must be rejected");
        assert!(
            err.to_string().contains("unknown field `value_from_env`"),
            "error must name the offending header key, got: {err}",
        );

        let tool_typo = r#"
            [[mcp_servers]]
            name = "compat"

            [mcp_servers.transport]
            type = "http_compat"
            base_url = "https://example.invalid"

            [[mcp_servers.transport.tools]]
            name = "forecast"
            path = "/forecast"
            responce_mode = "text"
        "#;
        let err = toml::from_str::<KernelConfig>(tool_typo)
            .expect_err("a misspelled tool field must be rejected");
        assert!(
            err.to_string().contains("unknown field `responce_mode`"),
            "error must name the offending tool key, got: {err}",
        );
    }

    #[test]
    fn every_valid_mcp_transport_variant_still_round_trips_6612() {
        // Drift sentinel for the guard: all four variants must still parse, and the HttpCompat nested arrays must actually carry their entries.
        // Asserting only `transport.is_some()` would pass even if `headers` / `tools` silently fell back to their `#[serde(default)]` empty vectors.
        let cfg: KernelConfig = toml::from_str(
            r#"
            [[mcp_servers]]
            name = "stdio-server"
            transport = { type = "stdio", command = "/usr/bin/mcp", args = ["--flag"] }

            [[mcp_servers]]
            name = "sse-server"
            transport = { type = "sse", url = "https://example.invalid/sse" }

            [[mcp_servers]]
            name = "http-server"
            transport = { type = "http", url = "https://example.invalid/mcp" }

            [[mcp_servers]]
            name = "compat-server"

            [mcp_servers.transport]
            type = "http_compat"
            base_url = "https://example.invalid"

            [[mcp_servers.transport.headers]]
            name = "Authorization"
            value_env = "COMPAT_TOKEN"

            [[mcp_servers.transport.tools]]
            name = "forecast"
            path = "/forecast/{city}"
            method = "get"
            request_mode = "query"
            response_mode = "text"

            # `input_schema` is a free-form `serde_json::Value`, so the guard must not reach into it — a JSON Schema is operator-authored content with arbitrary keys, not a struct with a known field set.
            [mcp_servers.transport.tools.input_schema]
            type = "object"

            [mcp_servers.transport.tools.input_schema.properties.city]
            type = "string"
            description = "City to forecast"
            "#,
        )
        .expect("every valid transport variant must still parse under deny_unknown_fields");
        assert_eq!(cfg.mcp_servers.len(), 4);

        match &cfg.mcp_servers[0].transport {
            Some(McpTransportEntry::Stdio { command, args }) => {
                assert_eq!(command, "/usr/bin/mcp");
                assert_eq!(args, &["--flag"]);
            }
            other => panic!("expected stdio transport, got {other:?}"),
        }
        match &cfg.mcp_servers[1].transport {
            Some(McpTransportEntry::Sse { url }) => {
                assert_eq!(url, "https://example.invalid/sse");
            }
            other => panic!("expected sse transport, got {other:?}"),
        }
        match &cfg.mcp_servers[2].transport {
            Some(McpTransportEntry::Http { url }) => {
                assert_eq!(url, "https://example.invalid/mcp");
            }
            other => panic!("expected http transport, got {other:?}"),
        }
        match &cfg.mcp_servers[3].transport {
            Some(McpTransportEntry::HttpCompat {
                base_url,
                headers,
                tools,
            }) => {
                assert_eq!(base_url, "https://example.invalid");
                assert_eq!(headers.len(), 1, "nested header array must survive");
                assert_eq!(headers[0].name, "Authorization");
                assert_eq!(headers[0].value_env.as_deref(), Some("COMPAT_TOKEN"));
                assert_eq!(tools.len(), 1, "nested tool array must survive");
                assert_eq!(tools[0].path, "/forecast/{city}");
                assert!(matches!(tools[0].method, HttpCompatMethod::Get));
                assert!(matches!(
                    tools[0].request_mode,
                    HttpCompatRequestMode::Query
                ));
                assert!(matches!(
                    tools[0].response_mode,
                    HttpCompatResponseMode::Text
                ));
                assert_eq!(
                    tools[0].input_schema["properties"]["city"]["description"], "City to forecast",
                    "free-form input_schema content must pass through untouched",
                );
            }
            other => panic!("expected http_compat transport, got {other:?}"),
        }
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

    #[test]
    fn detect_legacy_channel_blocks_flags_pre_sidecar_vendor_tables() {
        // Old single-table (`[channels.wechat]`) and array-of-tables
        // (`[[channels.telegram]]`, the old `OneOrMany`) per-vendor
        // blocks both flag. The known scalar `[channels]` settings do not.
        let raw: toml::Value = toml::from_str(
            r#"
            [channels]
            file_download_max_bytes = 1024

            [channels.wechat]
            bot_token = "secret"

            [[channels.telegram]]
            bot_token = "123:abc"
            "#,
        )
        .expect("toml parse");
        assert_eq!(
            KernelConfig::detect_legacy_channel_blocks(&raw),
            vec!["telegram".to_string(), "wechat".to_string()]
        );
    }

    #[test]
    fn detect_legacy_channel_blocks_empty_without_channels_section() {
        let raw: toml::Value = toml::from_str(r#"log_level = "debug""#).expect("toml parse");
        assert!(KernelConfig::detect_legacy_channel_blocks(&raw).is_empty());
    }

    #[test]
    fn detect_legacy_channel_blocks_ignores_known_scalar_settings() {
        // The surviving `ChannelsConfig` scalar fields must never be
        // mislabelled as legacy vendor blocks.
        let raw: toml::Value = toml::from_str(
            r#"
            [channels]
            file_download_max_bytes = 1024
            file_upload_max_bytes = 2048
            file_download_dir = "/tmp/dl"
            "#,
        )
        .expect("toml parse");
        assert!(KernelConfig::detect_legacy_channel_blocks(&raw).is_empty());
    }

    #[test]
    fn detect_legacy_channel_blocks_leaves_scalar_typo_to_generic_pass() {
        // A scalar typo under `[channels]` is NOT a vendor block — it is
        // left to the generic unknown-nested-field warning rather than
        // being mislabelled a migrated channel.
        let raw: toml::Value = toml::from_str(
            r#"
            [channels]
            file_downlod_max_bytes = 1024
            "#,
        )
        .expect("toml parse");
        assert!(KernelConfig::detect_legacy_channel_blocks(&raw).is_empty());
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
            context_engine,
            thinking,
            exec_policy,
            max_history_messages,
            tool_exec_backend,
            rl_export,
            // Overrides `[task_board] assignee_wake` (#6728).
            assignee_wake,

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
            // #7744: `owner` is a per-agent identity, not an override of a
            // global `KernelConfig` section — `default_owner` is a scalar
            // fallback, not an `[owner]` table an operator could relocate.
            owner,
            // Instance provenance, not a config.toml-facing override.
            source_template,
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
            "context_engine",
            "thinking",
            "exec_policy",
            "max_history_messages",
            "tool_exec",
            "tool_exec_backend",
            "rl_export",
            "assignee_wake",
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
