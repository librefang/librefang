//! Per-user RBAC policy primitives (RBAC M3, issue #3054 Phase 2).
//!
//! These types ride on top of the existing per-agent `ToolPolicy`, the
//! per-channel `ChannelToolRule` in [`crate::approval`], and the per-tool
//! taint labels in [`crate::taint`]. They do NOT replace those — a tool
//! call has to clear every layer (fail-closed AND).
//!
//! ## Resolution order (`UserToolPolicy::evaluate`)
//!
//! For a given `tool_name`, after the per-agent `ToolPolicy` and the
//! existing `ApprovalPolicy::channel_rules` have already returned an
//! intermediate decision, the per-user policy is consulted in this order:
//!
//! 1. `denied_tools` glob match → `Deny`
//! 2. `allowed_tools` glob match (when non-empty) → `Allow`
//! 3. `channel_tool_rules[channel]` (`ChannelToolPolicy`) → `Deny`/`Allow`
//! 4. `tool_categories.denied_groups` (matched against `ToolGroup::tools`)
//!    → `Deny`
//! 5. `tool_categories.allowed_groups` (when non-empty) → `Allow`
//! 6. Otherwise → `NeedsRoleEscalation`. The kernel translates that into
//!    an [`crate::approval::ApprovalRequest`] when an admin role would
//!    have allowed the call, or into a hard `Deny` when no role escalation
//!    is possible.
//!
//! Resolution is purely functional and side-effect free. The kernel owns
//! the cache (`AuthManager`) so we don't need a per-call hashmap here.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::capability::glob_matches;
use crate::tool_policy::ToolGroup;

/// Outcome of a per-user policy check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserToolDecision {
    /// User policy explicitly allows the tool — proceed with execution
    /// (still subject to per-agent and channel checks AND'd before this).
    Allow,
    /// User policy explicitly denies the tool — hard deny.
    Deny,
    /// User policy has no opinion. Caller decides whether to:
    ///   * fall through to the existing approval gate, or
    ///   * escalate to an [`ApprovalRequest`](crate::approval::ApprovalRequest)
    ///     when a higher role would have allowed it.
    NeedsRoleEscalation,
}

/// Per-user, per-channel allow/deny lists.
///
/// This is a strictly more permissive variant of
/// [`crate::approval::ChannelToolRule`]. The approval channel rule is
/// global to the agent; this one is keyed off the LibreFang user identity
/// resolved from the inbound message's channel binding. It allows
/// statements like "User Bob may run `shell_*` from his Telegram chat,
/// but only `web_*` from Discord".
///
/// Deny-wins inside one rule (mirrors `ChannelToolRule::check_tool`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ChannelToolPolicy {
    /// Tool patterns explicitly allowed when this user speaks via this
    /// channel. Empty = no allow-list (rule is deny-only or no-op).
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Tool patterns explicitly denied for this user on this channel.
    /// Always wins over `allowed_tools`.
    #[serde(default)]
    pub denied_tools: Vec<String>,
}

impl ChannelToolPolicy {
    /// Evaluate this rule against a tool name.
    ///
    /// * `Some(false)` — explicitly denied
    /// * `Some(true)`  — explicitly allowed
    /// * `None`        — no opinion (rule does not apply)
    pub fn check_tool(&self, tool_name: &str) -> Option<bool> {
        if self.denied_tools.iter().any(|p| glob_matches(p, tool_name)) {
            return Some(false);
        }
        if !self.allowed_tools.is_empty() {
            return Some(
                self.allowed_tools
                    .iter()
                    .any(|p| glob_matches(p, tool_name)),
            );
        }
        None
    }
}

/// Per-user allow/deny lists for tool invocations.
///
/// These rules layer on top of the per-agent
/// [`ToolPolicy`](crate::tool_policy::ToolPolicy) and the per-agent
/// channel rules in [`ApprovalPolicy`](crate::approval::ApprovalPolicy).
/// All layers must agree (fail-closed AND) for a call to proceed without
/// approval.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UserToolPolicy {
    /// Tool name patterns this user may invoke. Empty list means
    /// "no allow-list — defer to other layers". When non-empty, every
    /// invocation must match at least one pattern.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Tool name patterns this user must never invoke. Always wins.
    #[serde(default)]
    pub denied_tools: Vec<String>,
}

impl UserToolPolicy {
    /// Apply allow/deny lists against a tool name.
    ///
    /// Order: `denied_tools` first (deny-wins), then `allowed_tools`. If
    /// neither has an opinion, returns [`UserToolDecision::NeedsRoleEscalation`]
    /// so the caller can decide whether to escalate.
    pub fn check_tool(&self, tool_name: &str) -> UserToolDecision {
        if self.denied_tools.iter().any(|p| glob_matches(p, tool_name)) {
            return UserToolDecision::Deny;
        }
        if !self.allowed_tools.is_empty() {
            if self
                .allowed_tools
                .iter()
                .any(|p| glob_matches(p, tool_name))
            {
                return UserToolDecision::Allow;
            }
            // Allow-list set but tool not in it — needs escalation.
            return UserToolDecision::NeedsRoleEscalation;
        }
        UserToolDecision::NeedsRoleEscalation
    }
}

/// Bulk allow/deny by tool category — references existing `ToolGroup`
/// definitions by name (e.g. `"web_tools"`, `"code_tools"`). Group
/// definitions live in
/// [`KernelConfig.tool_policy.groups`](crate::tool_policy::ToolPolicy::groups).
///
/// Categories let admins say "this user only gets read-only categories"
/// without listing every tool individually.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UserToolCategories {
    /// Group names whose tools are allowed for this user. Empty = no
    /// category-level allow-list.
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    /// Group names whose tools are denied for this user. Always wins.
    #[serde(default)]
    pub denied_groups: Vec<String>,
}

impl UserToolCategories {
    /// Evaluate the category lists against a tool name and the registered
    /// `groups` from [`ToolPolicy`](crate::tool_policy::ToolPolicy).
    ///
    /// * `Some(false)` — tool belongs to a denied group
    /// * `Some(true)`  — tool belongs to an allowed group (when allow-list is set)
    /// * `None`        — categories have no opinion
    pub fn check_tool(&self, tool_name: &str, groups: &[ToolGroup]) -> Option<bool> {
        // denied_groups wins: any group match denies.
        for group_name in &self.denied_groups {
            if let Some(group) = groups.iter().find(|g| &g.name == group_name) {
                if group.tools.iter().any(|p| glob_matches(p, tool_name)) {
                    return Some(false);
                }
            }
        }
        if !self.allowed_groups.is_empty() {
            for group_name in &self.allowed_groups {
                if let Some(group) = groups.iter().find(|g| &g.name == group_name) {
                    if group.tools.iter().any(|p| glob_matches(p, tool_name)) {
                        return Some(true);
                    }
                }
            }
            // allow-list configured, none matched
            return Some(false);
        }
        None
    }
}

/// Per-user memory namespace ACL.
///
/// Memory in LibreFang is partitioned by *namespace* — typically the
/// agent ID for KV / proactive entries, plus a small set of well-known
/// shared scopes (`shared`, `proactive`, `kv`, …). This ACL gates which
/// of those a given user may read or write through the LLM-facing memory
/// tools.
///
/// PII handling: when `pii_access` is `false`, fragments tagged with
/// [`TaintLabel::Pii`](crate::taint::TaintLabel::Pii) MUST be redacted
/// before they reach the user. The redaction itself happens at the
/// memory call site (kernel + memory crate); this struct only declares
/// intent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UserMemoryAccess {
    /// Namespaces this user may read. Empty list with `pii_access=false`
    /// is a meaningful "no-read" deny-all — see [`Self::can_read`].
    #[serde(default)]
    pub readable_namespaces: Vec<String>,
    /// Namespaces this user may write to. Empty list = no-write
    /// (read-only).
    #[serde(default)]
    pub writable_namespaces: Vec<String>,
    /// Whether PII-tagged fragments may be returned to this user.
    /// When `false`, PII fields are redacted on read.
    #[serde(default)]
    pub pii_access: bool,
    /// Whether the user may export memory in bulk.
    #[serde(default)]
    pub export_allowed: bool,
    /// Whether the user may delete memory entries they can otherwise read.
    #[serde(default)]
    pub delete_allowed: bool,
}

impl UserMemoryAccess {
    /// Wildcard pattern matching against `readable_namespaces`. The
    /// special pattern `"*"` allows any namespace; otherwise an exact
    /// match or single-`*` glob is required (see
    /// [`crate::capability::glob_matches`] for the semantics).
    ///
    /// An empty `readable_namespaces` deny-all by default, **except**
    /// when no other restriction is configured at all (`pii_access=false`,
    /// `export_allowed=false`, `delete_allowed=false`, both lists empty)
    /// — that's an "unconfigured" sentinel and the caller (kernel) treats
    /// it as "no opinion, defer to role-default".
    pub fn can_read(&self, namespace: &str) -> bool {
        self.readable_namespaces
            .iter()
            .any(|p| glob_matches(p, namespace))
    }

    /// Wildcard match against `writable_namespaces`.
    pub fn can_write(&self, namespace: &str) -> bool {
        self.writable_namespaces
            .iter()
            .any(|p| glob_matches(p, namespace))
    }

    /// Returns true when no fields have been customised — i.e. the
    /// struct was just default-constructed during config load. The
    /// kernel uses this to fall back to the role-default ACL.
    pub fn is_unconfigured(&self) -> bool {
        self.readable_namespaces.is_empty()
            && self.writable_namespaces.is_empty()
            && !self.pii_access
            && !self.export_allowed
            && !self.delete_allowed
    }
}

/// Layered evaluator combining all per-user policy structs.
///
/// Used by the kernel-side resolver during a tool dispatch.  The runtime
/// crate doesn't depend on this directly — it consults the kernel via
/// the [`KernelHandle`](../../librefang_kernel_handle/index.html) trait.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ResolvedUserPolicy {
    /// Static allow/deny lists.
    #[serde(default)]
    pub tool_policy: UserToolPolicy,
    /// Per-channel overrides keyed by channel adapter name
    /// (e.g. `"telegram"`).
    #[serde(default)]
    pub channel_tool_rules: HashMap<String, ChannelToolPolicy>,
    /// Bulk category allow/deny.
    #[serde(default)]
    pub tool_categories: UserToolCategories,
    /// Memory namespace ACL.
    #[serde(default)]
    pub memory_access: UserMemoryAccess,
}

impl ResolvedUserPolicy {
    /// Run the four-layer per-user evaluation in order:
    /// 1. `tool_policy`
    /// 2. `channel_tool_rules[channel]` (when channel is `Some`)
    /// 3. `tool_categories` (consulted against `groups`)
    ///
    /// Within each step, an explicit deny short-circuits to
    /// [`UserToolDecision::Deny`]. An explicit allow short-circuits to
    /// [`UserToolDecision::Allow`]. Otherwise the next layer is
    /// consulted. If all layers abstain, returns
    /// [`UserToolDecision::NeedsRoleEscalation`].
    pub fn evaluate(
        &self,
        tool_name: &str,
        channel: Option<&str>,
        groups: &[ToolGroup],
    ) -> UserToolDecision {
        // Layer 1 — flat allow/deny lists.
        match self.tool_policy.check_tool(tool_name) {
            UserToolDecision::Allow => return UserToolDecision::Allow,
            UserToolDecision::Deny => return UserToolDecision::Deny,
            UserToolDecision::NeedsRoleEscalation => {}
        }

        // Layer 2 — channel-specific user rules.
        if let Some(ch) = channel {
            if let Some(rule) = self.channel_tool_rules.get(ch) {
                match rule.check_tool(tool_name) {
                    Some(false) => return UserToolDecision::Deny,
                    Some(true) => return UserToolDecision::Allow,
                    None => {}
                }
            }
        }

        // Layer 3 — tool categories.
        match self.tool_categories.check_tool(tool_name, groups) {
            Some(false) => UserToolDecision::Deny,
            Some(true) => UserToolDecision::Allow,
            None => UserToolDecision::NeedsRoleEscalation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(name: &str, tools: &[&str]) -> ToolGroup {
        ToolGroup {
            name: name.to_string(),
            tools: tools.iter().map(|s| s.to_string()).collect(),
        }
    }

    // ---- UserToolPolicy ----

    #[test]
    fn tool_policy_deny_wins_over_allow() {
        let p = UserToolPolicy {
            allowed_tools: vec!["shell_*".into()],
            denied_tools: vec!["shell_exec".into()],
        };
        assert_eq!(p.check_tool("shell_exec"), UserToolDecision::Deny);
        assert_eq!(p.check_tool("shell_run"), UserToolDecision::Allow);
    }

    #[test]
    fn tool_policy_empty_allow_means_no_opinion() {
        let p = UserToolPolicy::default();
        assert_eq!(
            p.check_tool("anything"),
            UserToolDecision::NeedsRoleEscalation
        );
    }

    #[test]
    fn tool_policy_allow_list_with_no_match_escalates() {
        let p = UserToolPolicy {
            allowed_tools: vec!["web_*".into()],
            denied_tools: vec![],
        };
        assert_eq!(
            p.check_tool("shell_exec"),
            UserToolDecision::NeedsRoleEscalation
        );
        assert_eq!(p.check_tool("web_search"), UserToolDecision::Allow);
    }

    // ---- ChannelToolPolicy ----

    #[test]
    fn channel_policy_deny_wins() {
        let p = ChannelToolPolicy {
            allowed_tools: vec!["*".into()],
            denied_tools: vec!["shell_exec".into()],
        };
        assert_eq!(p.check_tool("shell_exec"), Some(false));
        assert_eq!(p.check_tool("file_read"), Some(true));
    }

    #[test]
    fn channel_policy_no_opinion_when_empty() {
        let p = ChannelToolPolicy::default();
        assert_eq!(p.check_tool("anything"), None);
    }

    // ---- UserToolCategories ----

    #[test]
    fn categories_deny_group_wins() {
        let groups = vec![
            group("web_tools", &["web_search", "web_fetch"]),
            group("shell_tools", &["shell_exec"]),
        ];
        let cats = UserToolCategories {
            allowed_groups: vec!["web_tools".into(), "shell_tools".into()],
            denied_groups: vec!["shell_tools".into()],
        };
        assert_eq!(cats.check_tool("shell_exec", &groups), Some(false));
        assert_eq!(cats.check_tool("web_search", &groups), Some(true));
    }

    #[test]
    fn categories_allow_list_blocks_unmatched() {
        let groups = vec![group("web_tools", &["web_search"])];
        let cats = UserToolCategories {
            allowed_groups: vec!["web_tools".into()],
            denied_groups: vec![],
        };
        assert_eq!(cats.check_tool("web_search", &groups), Some(true));
        assert_eq!(cats.check_tool("shell_exec", &groups), Some(false));
    }

    #[test]
    fn categories_no_lists_no_opinion() {
        let cats = UserToolCategories::default();
        assert_eq!(cats.check_tool("anything", &[]), None);
    }

    // ---- UserMemoryAccess ----

    #[test]
    fn memory_access_glob_namespaces() {
        let acl = UserMemoryAccess {
            readable_namespaces: vec!["proactive".into(), "kv:*".into()],
            writable_namespaces: vec!["kv:user_*".into()],
            pii_access: false,
            export_allowed: false,
            delete_allowed: false,
        };
        assert!(acl.can_read("proactive"));
        assert!(acl.can_read("kv:foo"));
        assert!(!acl.can_read("shared"));
        assert!(acl.can_write("kv:user_alice"));
        assert!(!acl.can_write("kv:internal"));
    }

    #[test]
    fn memory_access_unconfigured_sentinel() {
        assert!(UserMemoryAccess::default().is_unconfigured());
        let configured = UserMemoryAccess {
            readable_namespaces: vec!["x".into()],
            ..Default::default()
        };
        assert!(!configured.is_unconfigured());
    }

    // ---- ResolvedUserPolicy.evaluate ----

    #[test]
    fn evaluate_layering_tool_policy_first() {
        let mut policy = ResolvedUserPolicy::default();
        policy.tool_policy.denied_tools = vec!["shell_exec".into()];
        policy.tool_categories.allowed_groups = vec!["shell_tools".into()];
        let groups = vec![group("shell_tools", &["shell_exec"])];

        // Even though categories allow it, tool_policy.deny wins (layer 1).
        assert_eq!(
            policy.evaluate("shell_exec", None, &groups),
            UserToolDecision::Deny
        );
    }

    #[test]
    fn evaluate_layering_channel_overrides_default() {
        let mut policy = ResolvedUserPolicy::default();
        policy.channel_tool_rules.insert(
            "telegram".into(),
            ChannelToolPolicy {
                allowed_tools: vec![],
                denied_tools: vec!["shell_exec".into()],
            },
        );
        // Channel rule denies on telegram, but discord has no rule.
        assert_eq!(
            policy.evaluate("shell_exec", Some("telegram"), &[]),
            UserToolDecision::Deny
        );
        assert_eq!(
            policy.evaluate("shell_exec", Some("discord"), &[]),
            UserToolDecision::NeedsRoleEscalation
        );
    }

    #[test]
    fn evaluate_categories_after_tool_policy_and_channel() {
        let mut policy = ResolvedUserPolicy::default();
        policy.tool_categories.allowed_groups = vec!["read_only".into()];
        let groups = vec![group("read_only", &["file_read", "web_search"])];

        // Layer 3 promotes web_search to Allow.
        assert_eq!(
            policy.evaluate("web_search", None, &groups),
            UserToolDecision::Allow
        );
        // Tool not in any allowed group → category layer denies.
        assert_eq!(
            policy.evaluate("shell_exec", None, &groups),
            UserToolDecision::Deny
        );
    }

    #[test]
    fn evaluate_empty_policy_always_escalates() {
        let policy = ResolvedUserPolicy::default();
        assert_eq!(
            policy.evaluate("anything", None, &[]),
            UserToolDecision::NeedsRoleEscalation
        );
    }

    // ---- serde roundtrip ----

    #[test]
    fn roundtrip_user_tool_policy_json() {
        let p = UserToolPolicy {
            allowed_tools: vec!["web_*".into()],
            denied_tools: vec!["shell_exec".into()],
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: UserToolPolicy = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn roundtrip_channel_tool_policy_toml() {
        let p = ChannelToolPolicy {
            allowed_tools: vec!["file_read".into()],
            denied_tools: vec!["shell_*".into()],
        };
        let s = toml::to_string(&p).unwrap();
        let back: ChannelToolPolicy = toml::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn roundtrip_user_tool_categories_json() {
        let c = UserToolCategories {
            allowed_groups: vec!["read_only".into()],
            denied_groups: vec!["dangerous".into()],
        };
        let s = serde_json::to_string(&c).unwrap();
        let back: UserToolCategories = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn roundtrip_user_memory_access_json() {
        let a = UserMemoryAccess {
            readable_namespaces: vec!["proactive".into(), "kv:*".into()],
            writable_namespaces: vec!["kv:scratch".into()],
            pii_access: true,
            export_allowed: false,
            delete_allowed: true,
        };
        let s = serde_json::to_string(&a).unwrap();
        let back: UserMemoryAccess = serde_json::from_str(&s).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn roundtrip_resolved_user_policy_toml() {
        let mut p = ResolvedUserPolicy::default();
        p.tool_policy.allowed_tools = vec!["web_*".into()];
        p.channel_tool_rules.insert(
            "telegram".into(),
            ChannelToolPolicy {
                allowed_tools: vec![],
                denied_tools: vec!["shell_*".into()],
            },
        );
        p.tool_categories.allowed_groups = vec!["read_only".into()];
        p.memory_access = UserMemoryAccess {
            readable_namespaces: vec!["proactive".into()],
            writable_namespaces: vec![],
            pii_access: false,
            export_allowed: false,
            delete_allowed: false,
        };
        let s = toml::to_string(&p).unwrap();
        let back: ResolvedUserPolicy = toml::from_str(&s).unwrap();
        assert_eq!(back, p);
    }
}
