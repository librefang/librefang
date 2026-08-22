//! Model routing profiles — tag-based model selection by task complexity.
//!
//! Each profile maps a set of tags (e.g. "code", "debug") to a
//! specific model/provider. The [`ModelRouter`] in `librefang-kernel`
//! evaluates task complexity and matches against these profiles.
//!
//! Profiles are stored in `model_profiles.toml` and hot-reloaded.

use serde::{Deserialize, Serialize};

/// A named profile mapping task tags to a model/provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelProfile {
    /// Unique name for this profile (e.g. "coder", "architect").
    pub name: String,
    /// Tags that trigger this profile. Matched against task description keywords.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Provider to use.
    pub provider: String,
    /// Model id within the provider.
    pub model: String,
    /// Maximum context window in tokens.
    #[serde(default)]
    pub context_window: Option<u64>,
    /// Cost tier: "cheap", "medium", or "expensive".
    #[serde(default)]
    pub cost_tier: CostTier,
    /// Higher priority wins when multiple profiles match a task.
    #[serde(default)]
    pub priority: u32,
    /// Maximum complexity score (0.0-1.0) this profile can handle.
    /// Tasks above this threshold require a higher-capability profile.
    #[serde(default = "default_max_complexity")]
    pub max_complexity: f32,
    /// Fallback profile name if this model/provider is unavailable.
    #[serde(default)]
    pub fallback: Option<String>,
    /// Optional description shown in the dashboard.
    #[serde(default)]
    pub description: Option<String>,
}

fn default_max_complexity() -> f32 {
    1.0
}

/// Cost tier for a model.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CostTier {
    /// Fast, cheap models (haiku-level).
    Cheap,
    /// Balanced cost/capability (sonnet/deepseek-level).
    #[default]
    Medium,
    /// High-capability, expensive models (opus-level).
    Expensive,
}

/// Router configuration — stored in `config.toml [model_router]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRouterConfig {
    /// Master switch. When false, the router is completely bypassed.
    #[serde(default)]
    pub enabled: bool,
    /// Path to profiles file, relative to ~/.librefang.
    #[serde(default = "default_profiles_path")]
    pub profiles_path: String,
    /// Default profile name used as fallback when routing fails.
    #[serde(default)]
    pub default_profile: Option<String>,
    /// Model alias for the complexity evaluator LLM call.
    /// Should point to a cheap model (e.g. "haiku"). When None, the
    /// evaluator uses heuristics only (no LLM cost).
    #[serde(default)]
    pub evaluator_model: Option<String>,
    /// Complexity threshold below which the LLM evaluator is skipped.
    /// Range 0.0-1.0. Default 0.3.
    #[serde(default = "default_complexity_threshold")]
    pub complexity_threshold: f32,
}

fn default_profiles_path() -> String {
    "model_profiles.toml".into()
}
fn default_complexity_threshold() -> f32 {
    0.3
}

impl Default for ModelRouterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            profiles_path: default_profiles_path(),
            default_profile: None,
            evaluator_model: None,
            complexity_threshold: default_complexity_threshold(),
        }
    }
}

/// Result of a complexity evaluation.
#[derive(Debug, Clone)]
pub struct ComplexityScore {
    /// 0.0 (trivial) to 1.0 (extremely complex).
    pub score: f32,
    /// How the score was determined.
    pub source: ComplexitySource,
    /// Human-readable rationale (from LLM evaluator, if used).
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComplexitySource {
    /// Heuristic only (keyword matching, length analysis).
    Heuristic,
    /// Evaluated by an LLM call.
    LlmEvaluator,
}

/// Per-agent router override in agent.toml `[model]`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentRouterOverride {
    /// When true, this agent bypasses the router — always uses its
    /// hardcoded model. Takes precedence over global router config.
    #[serde(default)]
    pub fixed: bool,
    /// Allowed profile names for this agent. Empty = all profiles allowed.
    #[serde(default)]
    pub allowed_profiles: Vec<String>,
    /// Maximum cost tier this agent can use.
    #[serde(default)]
    pub cost_budget: Option<CostTier>,
    /// Fallback profile when routing fails for this agent.
    #[serde(default)]
    pub default_profile: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_profile_parse() {
        let toml = r#"
name = "coder"
tags = ["code", "debug"]
provider = "deepseek"
model = "deepseek-v4-pro"
context_window = 131072
cost_tier = "medium"
priority = 10
max_complexity = 0.8
fallback = "quick"
"#;
        let p: ModelProfile = toml::from_str(toml).unwrap();
        assert_eq!(p.name, "coder");
        assert_eq!(p.tags.len(), 2);
        assert_eq!(p.provider, "deepseek");
        assert_eq!(p.cost_tier, CostTier::Medium);
        assert_eq!(p.max_complexity, 0.8);
    }

    #[test]
    fn router_config_default_off() {
        let cfg = ModelRouterConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.complexity_threshold, 0.3);
    }

    #[test]
    fn agent_override_default_fixed() {
        let ov = AgentRouterOverride::default();
        assert!(!ov.fixed); // default: not fixed = use router if enabled
        assert!(ov.allowed_profiles.is_empty());
    }
}
