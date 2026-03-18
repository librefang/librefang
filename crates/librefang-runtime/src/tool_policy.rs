//! Multi-layer tool policy resolution.
//!
//! Provides deny-wins, glob-pattern based tool access control with
//! agent-level and global rules, group expansion, and depth restrictions.
//!
//! The data types ([`ToolPolicy`], [`ToolPolicyRule`], [`PolicyEffect`],
//! [`ToolGroup`]) are defined in `librefang_types::tool_policy` and
//! re-exported here for convenience.

// Re-export data types from librefang-types so existing consumers keep working.
pub use librefang_types::tool_policy::{PolicyEffect, ToolGroup, ToolPolicy, ToolPolicyRule};

/// Result of a tool access check.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolAccessResult {
    /// Tool is allowed.
    Allowed,
    /// Tool is denied by a specific rule.
    Denied {
        rule_pattern: String,
        source: String,
    },
    /// Depth limit exceeded.
    DepthExceeded { current: u32, max: u32 },
}

/// Resolve whether a tool is accessible given the policy and current depth.
///
/// Priority: deny-wins, agent rules > global rules, explicit > wildcard.
pub fn resolve_tool_access(tool_name: &str, policy: &ToolPolicy, depth: u32) -> ToolAccessResult {
    // Check depth limit for subagent-related tools
    if is_subagent_tool(tool_name) && depth > policy.subagent_max_depth {
        return ToolAccessResult::DepthExceeded {
            current: depth,
            max: policy.subagent_max_depth,
        };
    }

    // Expand groups: check if tool_name matches any group tool pattern
    let expanded_tool_names = expand_groups(tool_name, &policy.groups);

    // Phase 1: Check agent rules (highest priority)
    // Deny-wins: if any deny matches, tool is denied regardless of allows
    for rule in &policy.agent_rules {
        if rule.effect == PolicyEffect::Deny
            && matches_pattern(&rule.pattern, tool_name, &expanded_tool_names)
        {
            return ToolAccessResult::Denied {
                rule_pattern: rule.pattern.clone(),
                source: "agent".to_string(),
            };
        }
    }

    // Phase 2: Check global rules for denies
    for rule in &policy.global_rules {
        if rule.effect == PolicyEffect::Deny
            && matches_pattern(&rule.pattern, tool_name, &expanded_tool_names)
        {
            return ToolAccessResult::Denied {
                rule_pattern: rule.pattern.clone(),
                source: "global".to_string(),
            };
        }
    }

    // Phase 3: If there are any allow rules, tool must match at least one
    let has_allow_rules = policy
        .agent_rules
        .iter()
        .any(|r| r.effect == PolicyEffect::Allow)
        || policy
            .global_rules
            .iter()
            .any(|r| r.effect == PolicyEffect::Allow);

    if has_allow_rules {
        let agent_allows = policy.agent_rules.iter().any(|r| {
            r.effect == PolicyEffect::Allow
                && matches_pattern(&r.pattern, tool_name, &expanded_tool_names)
        });
        let global_allows = policy.global_rules.iter().any(|r| {
            r.effect == PolicyEffect::Allow
                && matches_pattern(&r.pattern, tool_name, &expanded_tool_names)
        });

        if agent_allows || global_allows {
            return ToolAccessResult::Allowed;
        }

        return ToolAccessResult::Denied {
            rule_pattern: "(not in any allow list)".to_string(),
            source: "implicit_deny".to_string(),
        };
    }

    // No rules configured — allow by default
    ToolAccessResult::Allowed
}

/// Check if a tool name is related to subagent spawning.
fn is_subagent_tool(name: &str) -> bool {
    name == "agent_spawn" || name == "agent_call" || name == "spawn_agent"
}

/// Check if a tool name matches any expanded group tool names.
fn expand_groups(tool_name: &str, groups: &[ToolGroup]) -> Vec<String> {
    let mut expanded = vec![tool_name.to_string()];
    for group in groups {
        for pattern in &group.tools {
            if glob_match(pattern, tool_name) {
                // Add the group name as a pseudo-match
                expanded.push(format!("@{}", group.name));
            }
        }
    }
    expanded
}

/// Check if a pattern matches the tool name or any expanded name.
fn matches_pattern(pattern: &str, tool_name: &str, expanded: &[String]) -> bool {
    // Direct match
    if glob_match(pattern, tool_name) {
        return true;
    }
    // Group reference match (e.g., "@web_tools")
    if pattern.starts_with('@') {
        return expanded.iter().any(|e| e == pattern);
    }
    false
}

/// Simple glob matching supporting `*` as wildcard.
///
/// `*` matches any sequence of characters (including empty).
/// E.g., `"shell_*"` matches `"shell_exec"`, `"shell_write"`.
fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == text;
    }

    let parts: Vec<&str> = pattern.split('*').collect();

    if parts.len() == 2 {
        // Simple prefix/suffix match
        let prefix = parts[0];
        let suffix = parts[1];
        return text.starts_with(prefix)
            && text.ends_with(suffix)
            && text.len() >= prefix.len() + suffix.len();
    }

    // General glob: greedy left-to-right matching
    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            // Must match prefix
            if !text.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else if i == parts.len() - 1 {
            // Must match suffix
            if !text[pos..].ends_with(part) {
                return false;
            }
        } else {
            // Must find in remaining text
            match text[pos..].find(part) {
                Some(found) => pos = pos + found + part.len(),
                None => return false,
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Depth-aware subagent tool restrictions
// ---------------------------------------------------------------------------

/// Tools denied to ALL subagents (depth > 0). These are admin/scheduling tools
/// that should only be invoked by top-level agents.
const SUBAGENT_DENY_ALWAYS: &[&str] = &[
    "cron_create",
    "cron_cancel",
    "schedule_create",
    "schedule_delete",
    "hand_activate",
    "hand_deactivate",
    "process_start",
];

/// Tools denied to leaf subagents (depth >= max_depth - 1). Prevents deep spawn chains.
const SUBAGENT_DENY_LEAF: &[&str] = &["agent_spawn", "agent_kill"];

/// Filter a list of tools based on the current agent depth.
///
/// - `depth == 0`: no restrictions (top-level agent)
/// - `depth > 0`: strips SUBAGENT_DENY_ALWAYS tools
/// - `depth >= max_depth - 1`: additionally strips SUBAGENT_DENY_LEAF tools
pub fn filter_tools_by_depth(tools: &[String], depth: u32, max_depth: u32) -> Vec<String> {
    if depth == 0 {
        return tools.to_vec();
    }

    let is_leaf = max_depth > 0 && depth >= max_depth.saturating_sub(1);

    tools
        .iter()
        .filter(|name| {
            let n = name.as_str();
            if SUBAGENT_DENY_ALWAYS.contains(&n) {
                return false;
            }
            if is_leaf && SUBAGENT_DENY_LEAF.contains(&n) {
                return false;
            }
            true
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("shell_exec", "shell_exec"));
        assert!(!glob_match("shell_exec", "web_search"));
    }

    #[test]
    fn test_glob_match_wildcard() {
        assert!(glob_match("shell_*", "shell_exec"));
        assert!(glob_match("shell_*", "shell_write"));
        assert!(!glob_match("shell_*", "web_search"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn test_glob_match_prefix_suffix() {
        assert!(glob_match("mcp_*_list", "mcp_github_list"));
        assert!(!glob_match("mcp_*_list", "mcp_github_create"));
    }

    #[test]
    fn test_deny_wins() {
        let policy = ToolPolicy {
            agent_rules: vec![
                ToolPolicyRule {
                    pattern: "shell_*".to_string(),
                    effect: PolicyEffect::Allow,
                },
                ToolPolicyRule {
                    pattern: "shell_exec".to_string(),
                    effect: PolicyEffect::Deny,
                },
            ],
            ..Default::default()
        };

        let result = resolve_tool_access("shell_exec", &policy, 0);
        assert!(matches!(result, ToolAccessResult::Denied { .. }));

        // shell_write should still be allowed
        let result = resolve_tool_access("shell_write", &policy, 0);
        assert_eq!(result, ToolAccessResult::Allowed);
    }

    #[test]
    fn test_agent_rules_override_global() {
        let policy = ToolPolicy {
            agent_rules: vec![ToolPolicyRule {
                pattern: "web_search".to_string(),
                effect: PolicyEffect::Deny,
            }],
            global_rules: vec![ToolPolicyRule {
                pattern: "web_search".to_string(),
                effect: PolicyEffect::Allow,
            }],
            ..Default::default()
        };

        let result = resolve_tool_access("web_search", &policy, 0);
        assert!(matches!(result, ToolAccessResult::Denied { .. }));
    }

    #[test]
    fn test_group_expansion() {
        let policy = ToolPolicy {
            agent_rules: vec![ToolPolicyRule {
                pattern: "@web_tools".to_string(),
                effect: PolicyEffect::Deny,
            }],
            groups: vec![ToolGroup {
                name: "web_tools".to_string(),
                tools: vec!["web_*".to_string()],
            }],
            ..Default::default()
        };

        let result = resolve_tool_access("web_search", &policy, 0);
        assert!(matches!(result, ToolAccessResult::Denied { .. }));

        let result = resolve_tool_access("shell_exec", &policy, 0);
        assert_eq!(result, ToolAccessResult::Allowed);
    }

    #[test]
    fn test_depth_restriction() {
        let policy = ToolPolicy {
            subagent_max_depth: 3,
            ..Default::default()
        };

        let result = resolve_tool_access("agent_spawn", &policy, 4);
        assert!(matches!(result, ToolAccessResult::DepthExceeded { .. }));

        let result = resolve_tool_access("agent_spawn", &policy, 2);
        assert_eq!(result, ToolAccessResult::Allowed);
    }

    #[test]
    fn test_no_rules_allows_all() {
        let policy = ToolPolicy::default();
        let result = resolve_tool_access("anything", &policy, 0);
        assert_eq!(result, ToolAccessResult::Allowed);
    }

    #[test]
    fn test_implicit_deny_when_allow_rules_exist() {
        let policy = ToolPolicy {
            agent_rules: vec![ToolPolicyRule {
                pattern: "web_*".to_string(),
                effect: PolicyEffect::Allow,
            }],
            ..Default::default()
        };

        let result = resolve_tool_access("web_search", &policy, 0);
        assert_eq!(result, ToolAccessResult::Allowed);

        let result = resolve_tool_access("shell_exec", &policy, 0);
        assert!(matches!(result, ToolAccessResult::Denied { .. }));
    }

    // --- Depth-aware tool filtering tests ---

    #[test]
    fn test_depth_0_allows_all() {
        let tools: Vec<String> = vec!["cron_create", "agent_spawn", "web_search", "file_read"]
            .into_iter()
            .map(String::from)
            .collect();
        let filtered = filter_tools_by_depth(&tools, 0, 5);
        assert_eq!(filtered.len(), 4);
    }

    #[test]
    fn test_depth_1_denies_always() {
        let tools: Vec<String> = vec![
            "cron_create",
            "cron_cancel",
            "schedule_create",
            "schedule_delete",
            "hand_activate",
            "hand_deactivate",
            "process_start",
            "web_search",
            "file_read",
            "agent_spawn",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let filtered = filter_tools_by_depth(&tools, 1, 5);
        // Should keep: web_search, file_read, agent_spawn (not leaf)
        assert_eq!(filtered.len(), 3);
        assert!(filtered.contains(&"web_search".to_string()));
        assert!(filtered.contains(&"file_read".to_string()));
        assert!(filtered.contains(&"agent_spawn".to_string()));
    }

    #[test]
    fn test_leaf_depth_denies_spawn() {
        let tools: Vec<String> = vec!["agent_spawn", "agent_kill", "web_search", "file_read"]
            .into_iter()
            .map(String::from)
            .collect();
        // max_depth=5, depth=4 -> leaf (4 >= 5-1)
        let filtered = filter_tools_by_depth(&tools, 4, 5);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.contains(&"web_search".to_string()));
        assert!(filtered.contains(&"file_read".to_string()));
    }

    #[test]
    fn test_preserves_non_denied() {
        let tools: Vec<String> = vec!["web_search", "file_read", "shell_exec", "memory_store"]
            .into_iter()
            .map(String::from)
            .collect();
        let filtered = filter_tools_by_depth(&tools, 3, 5);
        assert_eq!(filtered, tools); // None of these are denied
    }

    #[test]
    fn test_empty_list() {
        let tools: Vec<String> = vec![];
        let filtered = filter_tools_by_depth(&tools, 2, 5);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_unknown_tools_preserved() {
        let tools: Vec<String> = vec!["custom_tool", "mcp_github_create"]
            .into_iter()
            .map(String::from)
            .collect();
        let filtered = filter_tools_by_depth(&tools, 3, 5);
        assert_eq!(filtered.len(), 2);
    }
}
