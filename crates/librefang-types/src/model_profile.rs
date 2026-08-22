//! Model routing profiles — tag-based model selection by task complexity.
//!
//! A [`ModelProfile`] binds a set of task tags (e.g. `"code"`, `"debug"`) to a
//! concrete provider/model pair, a cost tier, and the maximum task complexity
//! that model is expected to handle.
//! `librefang_kernel::model_router` scores the incoming turn and picks the best
//! matching profile.
//!
//! This is the *profile* layer of model routing.
//! It sits alongside the older tier router ([`crate::agent::ModelRoutingConfig`] plus
//! `librefang_runtime::routing::ModelRouter`), which maps a scored request onto one of three
//! fixed `simple` / `medium` / `complex` model slots.
//! The tier router answers "how hard is this?"; the profile router additionally answers "what
//! *kind* of work is this, and what may this agent afford?".
//! An agent opts into the profile router by setting `mode = "flexible"` in its manifest
//! `[model]` block; agents left at the default `mode = "fixed"` are untouched.
//!
//! Builtin profiles ship with the kernel as an asset.
//! Operators override them from `~/.librefang/model_profiles.toml`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A named profile mapping task tags to a model/provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelProfile {
    /// Unique name for this profile (e.g. `"coder"`, `"architect"`).
    pub name: String,
    /// Tags that trigger this profile, matched against the task text.
    ///
    /// `BTreeSet` (not `Vec`) so the tag order is deterministic across
    /// processes and duplicate tags collapse — the resolved profile reaches
    /// the LLM request, and unstable ordering silently invalidates provider
    /// prompt caches (#3298).
    #[serde(default)]
    pub tags: BTreeSet<String>,
    /// Provider to use.
    pub provider: String,
    /// Model id within the provider.
    pub model: String,
    /// Maximum context window in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// Cost tier: `"cheap"`, `"medium"`, or `"expensive"`.
    #[serde(default)]
    pub cost_tier: CostTier,
    /// Higher priority wins when several profiles match a task equally well.
    #[serde(default)]
    pub priority: u32,
    /// Maximum complexity score (0.0–1.0) this profile is expected to handle.
    /// Tasks scoring above it are routed to a higher-capability profile.
    #[serde(default = "default_max_complexity")]
    pub max_complexity: f32,
    /// Optional description shown in the dashboard and the TUI picker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn default_max_complexity() -> f32 {
    1.0
}

/// Cost tier for a model.
///
/// Ordered cheap < medium < expensive so a per-agent
/// [`AgentRouterOverride::cost_budget`] caps the tier with a plain comparison.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default, Hash,
)]
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

impl CostTier {
    /// The wire/config string for this tier, as accepted by `serde`.
    pub fn as_str(self) -> &'static str {
        match self {
            CostTier::Cheap => "cheap",
            CostTier::Medium => "medium",
            CostTier::Expensive => "expensive",
        }
    }

    /// Parse a wire/config string. Returns `None` for anything unrecognised,
    /// including the "no cap" sentinel the UIs send for an unset budget.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cheap" => Some(CostTier::Cheap),
            "medium" => Some(CostTier::Medium),
            "expensive" => Some(CostTier::Expensive),
            _ => None,
        }
    }
}

impl std::fmt::Display for CostTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Router configuration — `config.toml [model_router]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(default)]
pub struct ModelRouterConfig {
    /// Master switch. When false the profile router is bypassed entirely and
    /// every agent keeps the provider/model in its own manifest.
    pub enabled: bool,
    /// Profile catalog filename, resolved relative to the LibreFang home dir.
    /// When the file is absent the builtin profiles are used unchanged.
    pub profiles_path: String,
    /// Profile name used when no profile matches the task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    /// Complexity threshold above which routing is allowed to leave the
    /// cheapest tier. Range 0.0–1.0.
    pub complexity_threshold: f32,
}

fn default_profiles_path() -> String {
    "model_profiles.toml".to_string()
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
            complexity_threshold: default_complexity_threshold(),
        }
    }
}

/// How a [`ComplexityScore`] was determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComplexitySource {
    /// Keyword matching and length analysis. Free, instant, no LLM call.
    Heuristic,
}

/// Result of a complexity evaluation.
#[derive(Debug, Clone)]
pub struct ComplexityScore {
    /// 0.0 (trivial) to 1.0 (extremely complex).
    pub score: f32,
    /// How the score was determined.
    pub source: ComplexitySource,
    /// Human-readable rationale, surfaced in routing logs.
    pub rationale: Option<String>,
}

/// Per-agent router override, from the manifest `[model.router_override]`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct AgentRouterOverride {
    /// When true this agent bypasses the router even in `flexible` mode and
    /// always uses the provider/model in its own manifest.
    pub fixed: bool,
    /// Profile names this agent may use. Empty means "all profiles allowed".
    ///
    /// `BTreeSet` (not `Vec`) so the allowlist is deduplicated and ordered
    /// deterministically — it is echoed back through the API and the TUI, and
    /// it gates which model reaches the LLM request (#3298).
    pub allowed_profiles: BTreeSet<String>,
    /// Highest cost tier this agent may use. `None` means "no cap".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_budget: Option<CostTier>,
    /// Profile to fall back to when nothing matches, before the kernel-wide
    /// [`ModelRouterConfig::default_profile`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_profile_parses_from_toml() {
        let toml = r#"
name = "coder"
tags = ["code", "debug"]
provider = "deepseek"
model = "deepseek-v4-pro"
context_window = 131072
cost_tier = "medium"
priority = 10
max_complexity = 0.8
"#;
        let p: ModelProfile = toml::from_str(toml).unwrap();
        assert_eq!(p.name, "coder");
        assert_eq!(p.provider, "deepseek");
        assert_eq!(p.cost_tier, CostTier::Medium);
        assert_eq!(p.max_complexity, 0.8);
        assert!(p.tags.contains("code"));
        assert!(p.tags.contains("debug"));
    }

    #[test]
    fn model_profile_tags_are_sorted_and_deduplicated() {
        let toml = r#"
name = "coder"
tags = ["refactor", "code", "debug", "code"]
provider = "deepseek"
model = "deepseek-v4-pro"
"#;
        let p: ModelProfile = toml::from_str(toml).unwrap();
        let ordered: Vec<&str> = p.tags.iter().map(|s| s.as_str()).collect();
        assert_eq!(ordered, ["code", "debug", "refactor"]);
    }

    #[test]
    fn model_profile_optional_fields_default() {
        let toml = r#"
name = "bare"
provider = "anthropic"
model = "claude-haiku-4-5"
"#;
        let p: ModelProfile = toml::from_str(toml).unwrap();
        assert!(p.tags.is_empty());
        assert_eq!(p.cost_tier, CostTier::Medium);
        assert_eq!(p.priority, 0);
        assert_eq!(p.max_complexity, 1.0);
        assert!(p.description.is_none());
    }

    #[test]
    fn cost_tier_orders_cheap_below_expensive() {
        assert!(CostTier::Cheap < CostTier::Medium);
        assert!(CostTier::Medium < CostTier::Expensive);
    }

    #[test]
    fn cost_tier_round_trips_through_str() {
        for tier in [CostTier::Cheap, CostTier::Medium, CostTier::Expensive] {
            assert_eq!(CostTier::parse(tier.as_str()), Some(tier));
        }
        assert_eq!(CostTier::parse("default"), None);
        assert_eq!(CostTier::parse(""), None);
    }

    #[test]
    fn router_config_is_off_by_default() {
        let cfg = ModelRouterConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.profiles_path, "model_profiles.toml");
        assert_eq!(cfg.complexity_threshold, 0.3);
        assert!(cfg.default_profile.is_none());
    }

    #[test]
    fn router_config_fills_defaults_for_absent_keys() {
        let cfg: ModelRouterConfig = toml::from_str("enabled = true").unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.profiles_path, "model_profiles.toml");
        assert_eq!(cfg.complexity_threshold, 0.3);
    }

    #[test]
    fn agent_override_defaults_to_unconstrained() {
        let ov = AgentRouterOverride::default();
        assert!(!ov.fixed);
        assert!(ov.allowed_profiles.is_empty());
        assert!(ov.cost_budget.is_none());
        assert!(ov.default_profile.is_none());
    }

    #[test]
    fn agent_override_allowlist_is_deduplicated() {
        let toml = r#"
allowed_profiles = ["coder", "quick", "coder"]
cost_budget = "medium"
"#;
        let ov: AgentRouterOverride = toml::from_str(toml).unwrap();
        let ordered: Vec<&str> = ov.allowed_profiles.iter().map(|s| s.as_str()).collect();
        assert_eq!(ordered, ["coder", "quick"]);
        assert_eq!(ov.cost_budget, Some(CostTier::Medium));
    }
}
