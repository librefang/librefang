//! ModelRouter — task-to-profile matching with hybrid complexity evaluation.
//!
//! When `[model_router] enabled = true` in config.toml, this module:
//! 1. Evaluates task complexity (heuristic keywords + optional cheap LLM)
//! 2. Matches the task against configured ModelProfiles by tag overlap
//! 3. Returns the best profile, respecting agent cost_budget and allowed_profiles
//!
//! All entries are pure functions — no side effects, no I/O. The caller
//! (agent_spawn, goal runner, workflow executor) passes in the config.
//!
//! Alpha feature. Disabled by default.

use librefang_types::model_profile::{
    AgentRouterOverride, ComplexityScore, ComplexitySource, CostTier, ModelProfile,
    ModelRouterConfig,
};

/// Keywords strongly correlated with high-complexity tasks.
const COMPLEX_KEYWORDS: &[&str] = &[
    "research",
    "analyze",
    "audit",
    "architecture",
    "design",
    "migrate",
    "refactor",
    "investigate",
    "debug",
    "optimize",
    "implement",
    "build",
    "deploy",
    "security",
    "compliance",
    "multi-step",
    "pipeline",
];

/// Keywords correlated with low-complexity tasks.
const SIMPLE_KEYWORDS: &[&str] = &[
    "echo",
    "ping",
    "list",
    "format",
    "summarize",
    "translate",
    "classify",
    "extract",
    "count",
    "validate",
    "check",
];

/// Evaluate task complexity using heuristics only (free, instant).
/// Returns 0.0 (trivial) to 1.0 (extremely complex).
pub fn evaluate_complexity_heuristic(task_description: &str) -> ComplexityScore {
    let lower = task_description.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let word_count = words.len();

    let mut score: f32 = 0.0;

    // Word count signals
    if word_count > 200 {
        score += 0.3;
    } else if word_count > 100 {
        score += 0.2;
    } else if word_count > 50 {
        score += 0.1;
    }

    // Complex keyword matches
    let complex_hits = COMPLEX_KEYWORDS
        .iter()
        .filter(|k| lower.contains(*k))
        .count();
    score += (complex_hits as f32).min(4.0) * 0.08;

    // Simple keyword matches (dampen score)
    let simple_hits = SIMPLE_KEYWORDS
        .iter()
        .filter(|k| lower.contains(*k))
        .count();
    score -= (simple_hits as f32).min(3.0) * 0.05;

    // Task length relative to reasonable single-action
    if word_count <= 5 {
        score -= 0.1;
    }

    score = score.clamp(0.0, 1.0);

    ComplexityScore {
        score,
        source: ComplexitySource::Heuristic,
        rationale: Some(format!(
            "words={word_count} complex_hits={complex_hits} simple_hits={simple_hits}"
        )),
    }
}

/// Match a task against available profiles, returning the best match.
///
/// Algorithm:
/// 1. Score each profile by tag overlap with task description
/// 2. Filter out profiles not in `override.allowed_profiles` (if set)
/// 3. Filter out profiles exceeding `override.cost_budget` (if set)
/// 4. Filter out profiles where complexity > max_complexity
/// 5. Select highest priority among remaining
/// 6. Fall back to `override.default_profile` or `config.default_profile`
pub fn match_profile<'a>(
    task: &str,
    complexity: &ComplexityScore,
    profiles: &'a [ModelProfile],
    config: &ModelRouterConfig,
    agent_override: Option<&AgentRouterOverride>,
) -> Option<&'a ModelProfile> {
    if profiles.is_empty() || !config.enabled {
        return None;
    }

    let lower = task.to_lowercase();
    let allowed: Vec<&str> = agent_override
        .and_then(|o| {
            if o.allowed_profiles.is_empty() {
                None
            } else {
                Some(o.allowed_profiles.iter().map(|s| s.as_str()).collect())
            }
        })
        .unwrap_or_default();
    let cost_budget = agent_override.and_then(|o| o.cost_budget);
    let fixed = agent_override.map(|o| o.fixed).unwrap_or(false);

    if fixed {
        return None; // Agent is fixed, router bypassed
    }

    // Score profiles: tag overlap × priority weight
    let mut scored: Vec<(&ModelProfile, u32)> = profiles
        .iter()
        .filter(|p| {
            // Allowed profiles filter
            if !allowed.is_empty() && !allowed.iter().any(|a| *a == p.name) {
                return false;
            }
            // Cost budget filter
            if let Some(budget) = cost_budget {
                match (budget, p.cost_tier) {
                    (CostTier::Cheap, CostTier::Medium | CostTier::Expensive) => return false,
                    (CostTier::Medium, CostTier::Expensive) => return false,
                    _ => {}
                }
            }
            // Complexity filter
            if complexity.score > p.max_complexity {
                return false;
            }
            true
        })
        .map(|p| {
            let tag_hits = p.tags.iter().filter(|t| lower.contains(t.as_str())).count() as u32;
            let score = tag_hits.saturating_mul(10).saturating_add(p.priority);
            (p, score)
        })
        .collect();

    scored.sort_by(|a, b| b.1.cmp(&a.1));

    if let Some((best, _)) = scored.first() {
        return Some(best);
    }

    // Fallback: try the override's default_profile, then config's default
    let fallback_name = agent_override
        .and_then(|o| o.default_profile.as_deref())
        .or(config.default_profile.as_deref());

    if let Some(name) = fallback_name {
        if let Some(p) = profiles.iter().find(|p| p.name == name) {
            return Some(p);
        }
    }

    // Last resort: first profile
    profiles.first()
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::model_profile::CostTier;

    fn test_profiles() -> Vec<ModelProfile> {
        vec![
            ModelProfile {
                name: "quick".into(),
                tags: vec!["summarize".into(), "classify".into(), "extract".into()],
                provider: "anthropic".into(),
                model: "claude-haiku-4-5".into(),
                context_window: Some(131072),
                cost_tier: CostTier::Cheap,
                priority: 1,
                max_complexity: 0.3,
                fallback: Some("coder".into()),
                description: None,
            },
            ModelProfile {
                name: "coder".into(),
                tags: vec![
                    "code".into(),
                    "debug".into(),
                    "refactor".into(),
                    "implement".into(),
                    "build".into(),
                    "fix".into(),
                    "develop".into(),
                    "test".into(),
                    "deploy".into(),
                ],
                provider: "deepseek".into(),
                model: "deepseek-v4-pro".into(),
                context_window: Some(131072),
                cost_tier: CostTier::Medium,
                priority: 10,
                max_complexity: 0.8,
                fallback: Some("quick".into()),
                description: None,
            },
            ModelProfile {
                name: "architect".into(),
                tags: vec!["design".into(), "architecture".into(), "research".into()],
                provider: "anthropic".into(),
                model: "claude-opus-4-8".into(),
                context_window: Some(200000),
                cost_tier: CostTier::Expensive,
                priority: 20,
                max_complexity: 1.0,
                fallback: Some("coder".into()),
                description: None,
            },
        ]
    }

    #[test]
    fn heuristic_simple_task_scores_low() {
        let score = evaluate_complexity_heuristic("echo hello world");
        assert!(score.score < 0.3, "got {}", score.score);
        assert_eq!(score.source, ComplexitySource::Heuristic);
    }

    #[test]
    fn heuristic_complex_task_scores_high() {
        let score = evaluate_complexity_heuristic(
            "research the architecture of the system, analyze security compliance, \
             design a migration pipeline, audit the database schema, and implement \
             the new authentication layer with refactoring and optimization",
        );
        assert!(score.score > 0.3, "got {}", score.score);
    }

    #[test]
    fn match_profile_picks_best_by_tags() {
        let profiles = test_profiles();
        let config = ModelRouterConfig {
            enabled: true,
            ..Default::default()
        };
        let score = evaluate_complexity_heuristic("implement a new authentication middleware");
        let matched = match_profile(
            "implement a new authentication middleware",
            &score,
            &profiles,
            &config,
            None,
        );
        assert!(matched.is_some());
        // "code" tag matches "implement" semantics — should pick coder
        assert_eq!(matched.unwrap().name, "coder");
    }

    #[test]
    fn match_profile_respects_cost_budget() {
        let profiles = test_profiles();
        let config = ModelRouterConfig {
            enabled: true,
            ..Default::default()
        };
        let score = evaluate_complexity_heuristic(
            "design and research a new distributed architecture for the system",
        );
        let ov = AgentRouterOverride {
            cost_budget: Some(CostTier::Medium),
            ..Default::default()
        };
        let matched = match_profile(
            "design and research a new distributed architecture for the system",
            &score,
            &profiles,
            &config,
            Some(&ov),
        );
        // Architect is expensive, cost_budget is Medium — should NOT pick architect
        assert!(matched.is_some());
        assert_ne!(matched.unwrap().name, "architect");
        assert_eq!(matched.unwrap().name, "coder"); // next best
    }

    #[test]
    fn match_profile_fallback_when_none_match() {
        let profiles = test_profiles();
        let config = ModelRouterConfig {
            enabled: true,
            default_profile: Some("quick".into()),
            ..Default::default()
        };
        let score = ComplexityScore {
            score: 1.0,
            source: ComplexitySource::Heuristic,
            rationale: None,
        };
        // All profiles have max_complexity < 1.0 except architect, but with
        // cost_budget Cheap, only quick passes the cost filter — and it fails
        // complexity. Should fall back to config.default_profile ("quick").
        let ov = AgentRouterOverride {
            cost_budget: Some(CostTier::Cheap),
            ..Default::default()
        };
        let matched = match_profile("any task", &score, &profiles, &config, Some(&ov));
        assert!(matched.is_some());
        // Falls back to default_profile "quick"
        assert_eq!(matched.unwrap().name, "quick");
    }

    #[test]
    fn router_disabled_returns_none() {
        let profiles = test_profiles();
        let config = ModelRouterConfig::default(); // disabled
        let score = ComplexityScore {
            score: 0.5,
            source: ComplexitySource::Heuristic,
            rationale: None,
        };
        assert!(match_profile("any task", &score, &profiles, &config, None).is_none());
    }
}
