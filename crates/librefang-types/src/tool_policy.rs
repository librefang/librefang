//! Tool policy data types for deny-wins, glob-pattern based tool access control.
//!
//! These types are defined in `librefang-types` so they can be used in
//! [`KernelConfig`](crate::config::KernelConfig) without circular dependencies.
//! The resolution logic lives in `librefang-runtime::tool_policy`.

use serde::{Deserialize, Serialize};

/// Effect of a policy rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyEffect {
    /// Allow the tool.
    Allow,
    /// Deny the tool.
    Deny,
}

/// A single tool policy rule with glob pattern support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPolicyRule {
    /// Glob pattern to match tool names (e.g., "shell_*", "web_*", "mcp_github_*").
    pub pattern: String,
    /// Whether to allow or deny matching tools.
    pub effect: PolicyEffect,
}

/// Tool group — named collection of tool patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolGroup {
    /// Group name (e.g., "web_tools", "code_tools").
    pub name: String,
    /// Tool name patterns in this group.
    pub tools: Vec<String>,
}

/// Complete tool policy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolPolicy {
    /// Agent-level rules (highest priority, checked first).
    pub agent_rules: Vec<ToolPolicyRule>,
    /// Global rules (checked after agent rules).
    pub global_rules: Vec<ToolPolicyRule>,
    /// Named tool groups for grouping patterns.
    pub groups: Vec<ToolGroup>,
    /// Maximum subagent nesting depth. Default: 10.
    #[serde(default = "default_subagent_max_depth")]
    pub subagent_max_depth: u32,
    /// Maximum concurrent subagents. Default: 5.
    #[serde(default = "default_subagent_max_concurrent")]
    pub subagent_max_concurrent: u32,
}

fn default_subagent_max_depth() -> u32 {
    10
}

fn default_subagent_max_concurrent() -> u32 {
    5
}

impl Default for ToolPolicy {
    fn default() -> Self {
        Self {
            agent_rules: Vec::new(),
            global_rules: Vec::new(),
            groups: Vec::new(),
            subagent_max_depth: default_subagent_max_depth(),
            subagent_max_concurrent: default_subagent_max_concurrent(),
        }
    }
}

impl ToolPolicy {
    /// Check if any rules are configured.
    pub fn is_empty(&self) -> bool {
        self.agent_rules.is_empty() && self.global_rules.is_empty()
    }
}
