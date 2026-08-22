//! Profile router — matches a turn against named [`ModelProfile`]s.
//!
//! Three pieces:
//!
//! 1. [`evaluate_complexity_heuristic`] scores the task text 0.0–1.0 from
//!    keywords and length. No LLM call, no I/O, no cost.
//! 2. [`ProfileCatalog`] loads the builtin profile asset and merges the
//!    operator's `~/.librefang/model_profiles.toml` over it.
//! 3. [`match_profile`] picks the best profile for a scored task, honouring the
//!    per-agent [`AgentRouterOverride`] (allowlist, cost budget, opt-out).
//!
//! This is the *profile* router. It complements the older tier router
//! (`librefang_runtime::routing::ModelRouter`), which maps a scored
//! `CompletionRequest` onto three fixed `simple` / `medium` / `complex` model
//! slots. The tier router asks "how hard is this?"; the profile router also
//! asks "what kind of work is this, and what may this agent afford?".
//!
//! Everything here is a pure function over explicit inputs except the catalog
//! loader, whose only I/O is reading the profiles file.
//!
//! Off by default: [`ModelRouterConfig::enabled`] is `false`, and an agent
//! additionally has to opt in with `mode = "flexible"` in its manifest.

use librefang_types::model_profile::{
    AgentRouterOverride, ComplexityScore, ComplexitySource, CostTier, ModelProfile,
    ModelRouterConfig,
};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use tracing::warn;

/// The builtin profile catalog, compiled into the binary.
const BUILTIN_PROFILES_TOML: &str = include_str!("../assets/model_profiles.toml");

/// Keywords strongly correlated with high-complexity tasks.
const COMPLEX_KEYWORDS: &[&str] = &[
    "analyze",
    "architecture",
    "audit",
    "build",
    "compliance",
    "debug",
    "deploy",
    "design",
    "implement",
    "investigate",
    "migrate",
    "multi-step",
    "optimize",
    "pipeline",
    "refactor",
    "research",
    "security",
];

/// Keywords correlated with low-complexity tasks.
const SIMPLE_KEYWORDS: &[&str] = &[
    "check",
    "classify",
    "count",
    "echo",
    "extract",
    "format",
    "list",
    "ping",
    "summarize",
    "translate",
    "validate",
];

/// Evaluate task complexity from the task text alone. Free and instant.
///
/// Returns 0.0 (trivial) to 1.0 (extremely complex).
pub fn evaluate_complexity_heuristic(task_description: &str) -> ComplexityScore {
    let lower = task_description.to_lowercase();
    let word_count = lower.split_whitespace().count();

    let mut score: f32 = 0.0;

    // Length is the coarsest signal: a long brief is rarely a trivial ask.
    if word_count > 200 {
        score += 0.3;
    } else if word_count > 100 {
        score += 0.2;
    } else if word_count > 50 {
        score += 0.1;
    }

    let complex_hits = COMPLEX_KEYWORDS
        .iter()
        .filter(|k| lower.contains(*k))
        .count();
    score += (complex_hits as f32).min(4.0) * 0.08;

    let simple_hits = SIMPLE_KEYWORDS
        .iter()
        .filter(|k| lower.contains(*k))
        .count();
    score -= (simple_hits as f32).min(3.0) * 0.05;

    // A handful of words cannot describe a hard task.
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

/// Where a loaded [`ProfileCatalog`] came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogSource {
    /// Only the compiled-in asset was used — no override file on disk.
    Builtin,
    /// The builtin asset with an operator override file merged over it.
    BuiltinPlusOverride(PathBuf),
}

/// A resolved set of model profiles, ordered by name.
#[derive(Debug, Clone)]
pub struct ProfileCatalog {
    profiles: Vec<ModelProfile>,
    source: CatalogSource,
}

impl ProfileCatalog {
    /// The compiled-in profiles, with no operator override applied.
    ///
    /// # Panics
    ///
    /// Never in practice: the asset is parsed by
    /// `builtin_profiles_asset_parses` on every test run, so a malformed asset
    /// fails CI rather than a user's daemon.
    pub fn builtin() -> Self {
        let profiles = parse_profiles(BUILTIN_PROFILES_TOML)
            .expect("builtin model_profiles.toml asset must parse");
        Self {
            profiles: sorted_by_name(profiles),
            source: CatalogSource::Builtin,
        }
    }

    /// Load the builtin profiles and merge `<home>/<profiles_path>` over them.
    ///
    /// A profile in the override file **replaces** the builtin of the same
    /// name; a profile with a new name is **added**. A missing override file
    /// is the normal case and yields the builtins unchanged. An unreadable or
    /// malformed override file is logged and ignored — a typo in a hand-edited
    /// TOML must not take the daemon down, and falling back to the builtins
    /// keeps every agent routable.
    pub fn load(home: &Path, config: &ModelRouterConfig) -> Self {
        let path = home.join(&config.profiles_path);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::builtin(),
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "model_profiles.toml unreadable — using builtin profiles"
                );
                return Self::builtin();
            }
        };

        let overrides = match parse_profiles(&raw) {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "model_profiles.toml malformed — using builtin profiles"
                );
                return Self::builtin();
            }
        };

        let mut merged = Self::builtin().profiles;
        for profile in overrides {
            match merged.iter_mut().find(|p| p.name == profile.name) {
                Some(slot) => *slot = profile,
                None => merged.push(profile),
            }
        }

        Self {
            profiles: sorted_by_name(merged),
            source: CatalogSource::BuiltinPlusOverride(path),
        }
    }

    /// [`Self::load`] with a process-wide cache keyed on the override file's
    /// path, modification time and length.
    ///
    /// The routing path runs per turn, so re-parsing the TOML every time would
    /// be wasteful; but caching the parse forever would mean an operator's edit
    /// to `model_profiles.toml` needed a daemon restart to take effect. Both
    /// are avoided by re-reading only when the file's mtime or length moved,
    /// which costs one `stat` per routed turn — negligible next to the LLM call
    /// it is about to make.
    pub fn load_cached(home: &Path, config: &ModelRouterConfig) -> Self {
        static CACHE: RwLock<Option<(CacheKey, ProfileCatalog)>> = RwLock::new(None);

        let path = home.join(&config.profiles_path);
        let key = CacheKey::probe(&path);

        if let Ok(guard) = CACHE.read() {
            if let Some((cached_key, catalog)) = guard.as_ref() {
                if *cached_key == key {
                    return catalog.clone();
                }
            }
        }

        let catalog = Self::load(home, config);
        if let Ok(mut guard) = CACHE.write() {
            *guard = Some((key, catalog.clone()));
        }
        catalog
    }

    /// The profiles, ordered by name.
    pub fn profiles(&self) -> &[ModelProfile] {
        &self.profiles
    }

    /// Profile names, ordered.
    ///
    /// Deterministic by construction (#3298): the catalog is name-sorted at
    /// load, so this list is byte-identical across processes regardless of the
    /// order profiles appeared in the builtin asset or the override file.
    pub fn names(&self) -> Vec<String> {
        self.profiles.iter().map(|p| p.name.clone()).collect()
    }

    /// Look a profile up by exact name.
    pub fn get(&self, name: &str) -> Option<&ModelProfile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    /// Where these profiles came from.
    pub fn source(&self) -> &CatalogSource {
        &self.source
    }
}

/// Cache identity for the override file: absent, or present with this
/// (mtime, len). Comparing both catches an edit that preserved the length.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheKey {
    path: PathBuf,
    stamp: Option<(std::time::SystemTime, u64)>,
}

impl CacheKey {
    fn probe(path: &Path) -> Self {
        let stamp = std::fs::metadata(path)
            .ok()
            .and_then(|m| Some((m.modified().ok()?, m.len())));
        Self {
            path: path.to_path_buf(),
            stamp,
        }
    }
}

/// TOML wrapper for the `[[profiles]]` array.
#[derive(serde::Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    profiles: Vec<ModelProfile>,
}

fn parse_profiles(raw: &str) -> Result<Vec<ModelProfile>, toml::de::Error> {
    Ok(toml::from_str::<ProfilesFile>(raw)?.profiles)
}

fn sorted_by_name(mut profiles: Vec<ModelProfile>) -> Vec<ModelProfile> {
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    profiles
}

/// Why the router chose (or declined to choose) a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingDecision {
    /// The router is off kernel-wide, or the agent opted out with
    /// `router_override.fixed = true`.
    Bypassed,
    /// A profile matched the task's tags.
    Matched,
    /// Nothing matched; a configured default profile was used.
    Fellback,
    /// Nothing matched and no permitted fallback existed. The agent keeps the
    /// provider/model from its own manifest.
    NoCandidate,
}

/// Match a task against the catalog and return the best profile.
///
/// Order of operations:
///
/// 1. Bail out when the router is disabled kernel-wide, or the agent set
///    `router_override.fixed`.
/// 2. Keep only profiles the agent is permitted to use — present in
///    `allowed_profiles` (when that list is non-empty) and at or below
///    `cost_budget`.
/// 3. Drop profiles whose `max_complexity` is below the task's score, and —
///    for tasks scoring under [`ModelRouterConfig::complexity_threshold`] —
///    everything above the cheapest permitted tier.
/// 4. Rank the survivors by tag overlap, then `priority`, then name.
/// 5. When nothing survives, fall back to the agent's `default_profile` and
///    then the kernel-wide one, **re-checked against the same permissions** so
///    a fallback can never spend past an agent's cost budget.
pub fn match_profile<'a>(
    task: &str,
    complexity: &ComplexityScore,
    profiles: &'a [ModelProfile],
    config: &ModelRouterConfig,
    agent_override: Option<&AgentRouterOverride>,
) -> (Option<&'a ModelProfile>, RoutingDecision) {
    if !config.enabled || agent_override.is_some_and(|o| o.fixed) {
        return (None, RoutingDecision::Bypassed);
    }
    if profiles.is_empty() {
        return (None, RoutingDecision::NoCandidate);
    }

    let permitted: Vec<&ModelProfile> = profiles
        .iter()
        .filter(|p| is_permitted(p, agent_override))
        .collect();
    if permitted.is_empty() {
        return (None, RoutingDecision::NoCandidate);
    }

    // A task below the threshold is not worth paying up for: cap it at the
    // cheapest tier this agent can actually reach, so a "cheapest" tier that
    // the allowlist or the budget excludes does not silently disable the cap.
    let cheapest_permitted = permitted
        .iter()
        .map(|p| p.cost_tier)
        .min()
        .unwrap_or(CostTier::Cheap);
    let tier_ceiling = if complexity.score < config.complexity_threshold {
        cheapest_permitted
    } else {
        CostTier::Expensive
    };

    let lower = task.to_lowercase();
    let mut candidates: Vec<(&'a ModelProfile, u32)> = permitted
        .iter()
        .copied()
        .filter(|p| complexity.score <= p.max_complexity && p.cost_tier <= tier_ceiling)
        .map(|p| {
            let tag_hits = p.tags.iter().filter(|t| lower.contains(t.as_str())).count() as u32;
            (p, tag_hits)
        })
        .collect();

    // Rank: most tag hits, then highest priority, then name. The name is the
    // final tie-break so the choice is reproducible across processes even when
    // two profiles are otherwise indistinguishable (#3298).
    candidates.sort_by(|(pa, ha), (pb, hb)| {
        hb.cmp(ha)
            .then_with(|| pb.priority.cmp(&pa.priority))
            .then_with(|| pa.name.cmp(&pb.name))
    });

    // Only treat it as a match when the task actually mentioned one of the
    // profile's tags. A zero-hit "best" candidate is not a match, it is just
    // the first row of an arbitrary ranking — route those through the explicit
    // fallback chain instead.
    if let Some((best, hits)) = candidates.first() {
        if *hits > 0 {
            return (Some(best), RoutingDecision::Matched);
        }
    }

    let fallback_name = agent_override
        .and_then(|o| o.default_profile.as_deref())
        .or(config.default_profile.as_deref());
    if let Some(name) = fallback_name {
        // Re-check permissions: a `default_profile` naming an expensive
        // profile must not become a way around the agent's cost budget.
        if let Some(p) = permitted.iter().copied().find(|p| p.name == name) {
            return (Some(p), RoutingDecision::Fellback);
        }
        warn!(
            profile = %name,
            "model_router default_profile is unknown or not permitted for this agent — keeping the agent's own model"
        );
    }

    (None, RoutingDecision::NoCandidate)
}

/// Whether an agent's override permits this profile at all.
fn is_permitted(profile: &ModelProfile, agent_override: Option<&AgentRouterOverride>) -> bool {
    let Some(ov) = agent_override else {
        return true;
    };
    if !ov.allowed_profiles.is_empty() && !ov.allowed_profiles.contains(&profile.name) {
        return false;
    }
    if let Some(budget) = ov.cost_budget {
        if profile.cost_tier > budget {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn tags(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn profile(
        name: &str,
        tag_list: &[&str],
        tier: CostTier,
        priority: u32,
        max: f32,
    ) -> ModelProfile {
        ModelProfile {
            name: name.to_string(),
            tags: tags(tag_list),
            provider: "anthropic".to_string(),
            model: format!("model-{name}"),
            context_window: None,
            cost_tier: tier,
            priority,
            max_complexity: max,
            description: None,
        }
    }

    fn test_profiles() -> Vec<ModelProfile> {
        sorted_by_name(vec![
            profile("quick", &["summarize", "classify"], CostTier::Cheap, 1, 0.3),
            profile(
                "coder",
                &["code", "debug", "refactor", "implement"],
                CostTier::Medium,
                10,
                0.8,
            ),
            profile(
                "architect",
                &["design", "architecture", "research"],
                CostTier::Expensive,
                20,
                1.0,
            ),
        ])
    }

    fn enabled_config() -> ModelRouterConfig {
        ModelRouterConfig {
            enabled: true,
            // Neutralise the cheap-tier cap unless a test opts into it, so the
            // tag-matching assertions below test only tag matching.
            complexity_threshold: 0.0,
            ..Default::default()
        }
    }

    fn score(value: f32) -> ComplexityScore {
        ComplexityScore {
            score: value,
            source: ComplexitySource::Heuristic,
            rationale: None,
        }
    }

    // ---- complexity heuristic ------------------------------------------

    #[test]
    fn heuristic_scores_a_trivial_task_low() {
        let s = evaluate_complexity_heuristic("echo hello world");
        assert!(s.score < 0.3, "got {}", s.score);
        assert_eq!(s.source, ComplexitySource::Heuristic);
    }

    #[test]
    fn heuristic_scores_a_dense_task_high() {
        let s = evaluate_complexity_heuristic(
            "research the architecture of the system, analyze security compliance, \
             design a migration pipeline, audit the database schema, and implement \
             the new authentication layer with refactoring and optimization",
        );
        assert!(s.score > 0.3, "got {}", s.score);
    }

    #[test]
    fn heuristic_stays_within_bounds() {
        let empty = evaluate_complexity_heuristic("");
        assert!((0.0..=1.0).contains(&empty.score), "got {}", empty.score);

        let piled_on =
            "architecture security compliance migrate refactor optimize debug ".repeat(60);
        let high = evaluate_complexity_heuristic(&piled_on);
        assert!((0.0..=1.0).contains(&high.score), "got {}", high.score);
    }

    // ---- threshold boundaries ------------------------------------------

    #[test]
    fn profile_accepts_complexity_exactly_at_its_ceiling() {
        let profiles = test_profiles();
        let cfg = enabled_config();
        // "quick" has max_complexity 0.3 and is the only profile tagged
        // "summarize"; a task scoring exactly 0.3 must still reach it.
        let (matched, decision) =
            match_profile("summarize this", &score(0.3), &profiles, &cfg, None);
        assert_eq!(decision, RoutingDecision::Matched);
        assert_eq!(matched.unwrap().name, "quick");
    }

    #[test]
    fn profile_is_dropped_just_above_its_ceiling() {
        let profiles = test_profiles();
        let cfg = enabled_config();
        // 0.31 exceeds quick's 0.3 ceiling. No other profile is tagged
        // "summarize", so there is no match and no configured fallback.
        let (matched, decision) =
            match_profile("summarize this", &score(0.31), &profiles, &cfg, None);
        assert_eq!(decision, RoutingDecision::NoCandidate);
        assert!(matched.is_none());
    }

    #[test]
    fn tasks_below_the_threshold_are_capped_at_the_cheapest_tier() {
        let profiles = test_profiles();
        let cfg = ModelRouterConfig {
            enabled: true,
            complexity_threshold: 0.5,
            ..Default::default()
        };
        // "design" is an architect tag, but the task scores below the
        // threshold, so the expensive tier is off the table.
        let (matched, decision) = match_profile("design it", &score(0.1), &profiles, &cfg, None);
        assert_eq!(decision, RoutingDecision::NoCandidate);
        assert!(matched.is_none());

        // The same task above the threshold reaches the architect.
        let (matched, decision) = match_profile("design it", &score(0.6), &profiles, &cfg, None);
        assert_eq!(decision, RoutingDecision::Matched);
        assert_eq!(matched.unwrap().name, "architect");
    }

    // ---- tag matching and ranking ---------------------------------------

    #[test]
    fn match_picks_the_profile_whose_tag_the_task_mentions() {
        let profiles = test_profiles();
        let cfg = enabled_config();
        let (matched, decision) = match_profile(
            "implement a new authentication middleware",
            &score(0.4),
            &profiles,
            &cfg,
            None,
        );
        assert_eq!(decision, RoutingDecision::Matched);
        assert_eq!(matched.unwrap().name, "coder");
    }

    #[test]
    fn match_is_independent_of_catalog_order() {
        let cfg = enabled_config();
        let forward = test_profiles();
        let mut reversed = forward.clone();
        reversed.reverse();

        let (a, _) = match_profile("refactor the parser", &score(0.4), &forward, &cfg, None);
        let (b, _) = match_profile("refactor the parser", &score(0.4), &reversed, &cfg, None);
        assert_eq!(a.unwrap().name, b.unwrap().name);
    }

    // ---- per-agent override ---------------------------------------------

    #[test]
    fn fixed_override_bypasses_the_router() {
        let profiles = test_profiles();
        let cfg = enabled_config();
        let ov = AgentRouterOverride {
            fixed: true,
            ..Default::default()
        };
        let (matched, decision) = match_profile(
            "implement a new authentication middleware",
            &score(0.4),
            &profiles,
            &cfg,
            Some(&ov),
        );
        assert_eq!(decision, RoutingDecision::Bypassed);
        assert!(matched.is_none());
    }

    #[test]
    fn allowed_profiles_restricts_the_choice() {
        let profiles = test_profiles();
        let cfg = enabled_config();
        // The task's tag says "coder", but only "quick" is allowed.
        let ov = AgentRouterOverride {
            allowed_profiles: tags(&["quick"]),
            ..Default::default()
        };
        let (matched, decision) = match_profile(
            "refactor and debug the parser",
            &score(0.2),
            &profiles,
            &cfg,
            Some(&ov),
        );
        assert_eq!(decision, RoutingDecision::NoCandidate);
        assert!(matched.is_none());

        // Widening the allowlist lets the same task through to "coder".
        let ov = AgentRouterOverride {
            allowed_profiles: tags(&["quick", "coder"]),
            ..Default::default()
        };
        let (matched, decision) = match_profile(
            "refactor and debug the parser",
            &score(0.2),
            &profiles,
            &cfg,
            Some(&ov),
        );
        assert_eq!(decision, RoutingDecision::Matched);
        assert_eq!(matched.unwrap().name, "coder");
    }

    #[test]
    fn cost_budget_caps_the_tier() {
        let profiles = test_profiles();
        let cfg = enabled_config();
        let ov = AgentRouterOverride {
            cost_budget: Some(CostTier::Medium),
            ..Default::default()
        };
        // "design" and "research" are architect tags, but architect is
        // expensive and the budget is medium.
        let (matched, decision) = match_profile(
            "design and research a distributed architecture",
            &score(0.9),
            &profiles,
            &cfg,
            Some(&ov),
        );
        assert_eq!(decision, RoutingDecision::NoCandidate);
        assert!(matched.is_none());

        // Without the cap, the same task reaches the architect.
        let (matched, decision) = match_profile(
            "design and research a distributed architecture",
            &score(0.9),
            &profiles,
            &cfg,
            None,
        );
        assert_eq!(decision, RoutingDecision::Matched);
        assert_eq!(matched.unwrap().name, "architect");
    }

    #[test]
    fn cost_budget_still_allows_tiers_at_or_below_it() {
        let profiles = test_profiles();
        let cfg = enabled_config();
        let ov = AgentRouterOverride {
            cost_budget: Some(CostTier::Medium),
            ..Default::default()
        };
        let (matched, decision) = match_profile(
            "refactor the parser",
            &score(0.4),
            &profiles,
            &cfg,
            Some(&ov),
        );
        assert_eq!(decision, RoutingDecision::Matched);
        assert_eq!(matched.unwrap().name, "coder");
    }

    // ---- fallbacks --------------------------------------------------------

    #[test]
    fn falls_back_to_the_configured_default_when_nothing_matches() {
        let profiles = test_profiles();
        let cfg = ModelRouterConfig {
            enabled: true,
            complexity_threshold: 0.0,
            default_profile: Some("quick".to_string()),
            ..Default::default()
        };
        let (matched, decision) = match_profile("do the thing", &score(0.2), &profiles, &cfg, None);
        assert_eq!(decision, RoutingDecision::Fellback);
        assert_eq!(matched.unwrap().name, "quick");
    }

    #[test]
    fn agent_default_profile_wins_over_the_kernel_default() {
        let profiles = test_profiles();
        let cfg = ModelRouterConfig {
            enabled: true,
            complexity_threshold: 0.0,
            default_profile: Some("quick".to_string()),
            ..Default::default()
        };
        let ov = AgentRouterOverride {
            default_profile: Some("coder".to_string()),
            ..Default::default()
        };
        let (matched, decision) =
            match_profile("do the thing", &score(0.5), &profiles, &cfg, Some(&ov));
        assert_eq!(decision, RoutingDecision::Fellback);
        assert_eq!(matched.unwrap().name, "coder");
    }

    #[test]
    fn fallback_cannot_escape_the_cost_budget() {
        let profiles = test_profiles();
        let cfg = ModelRouterConfig {
            enabled: true,
            complexity_threshold: 0.0,
            // An expensive default profile paired with a cheap budget: the
            // budget must win, and the agent keeps its own model.
            default_profile: Some("architect".to_string()),
            ..Default::default()
        };
        let ov = AgentRouterOverride {
            cost_budget: Some(CostTier::Cheap),
            ..Default::default()
        };
        let (matched, decision) =
            match_profile("do the thing", &score(0.2), &profiles, &cfg, Some(&ov));
        assert_eq!(decision, RoutingDecision::NoCandidate);
        assert!(matched.is_none());
    }

    #[test]
    fn fallback_cannot_escape_the_allowlist() {
        let profiles = test_profiles();
        let cfg = ModelRouterConfig {
            enabled: true,
            complexity_threshold: 0.0,
            default_profile: Some("architect".to_string()),
            ..Default::default()
        };
        let ov = AgentRouterOverride {
            allowed_profiles: tags(&["quick"]),
            ..Default::default()
        };
        let (matched, decision) =
            match_profile("do the thing", &score(0.2), &profiles, &cfg, Some(&ov));
        assert_eq!(decision, RoutingDecision::NoCandidate);
        assert!(matched.is_none());
    }

    #[test]
    fn disabled_router_returns_bypassed() {
        let profiles = test_profiles();
        let cfg = ModelRouterConfig::default();
        let (matched, decision) =
            match_profile("refactor the parser", &score(0.5), &profiles, &cfg, None);
        assert_eq!(decision, RoutingDecision::Bypassed);
        assert!(matched.is_none());
    }

    #[test]
    fn empty_catalog_yields_no_candidate() {
        let cfg = enabled_config();
        let (matched, decision) = match_profile("refactor", &score(0.5), &[], &cfg, None);
        assert_eq!(decision, RoutingDecision::NoCandidate);
        assert!(matched.is_none());
    }

    // ---- catalog loading --------------------------------------------------

    #[test]
    fn builtin_profiles_asset_parses() {
        let catalog = ProfileCatalog::builtin();
        assert!(!catalog.profiles().is_empty());
        assert_eq!(catalog.source(), &CatalogSource::Builtin);
        for expected in ["architect", "coder", "quick", "researcher"] {
            assert!(
                catalog.get(expected).is_some(),
                "builtin catalog is missing '{expected}'"
            );
        }
    }

    #[test]
    fn builtin_catalog_is_name_sorted() {
        let names = ProfileCatalog::builtin().names();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "catalog must be deterministic (#3298)");
    }

    #[test]
    fn missing_override_file_yields_the_builtins() {
        let home = tempfile::tempdir().unwrap();
        let catalog = ProfileCatalog::load(home.path(), &ModelRouterConfig::default());
        assert_eq!(catalog.source(), &CatalogSource::Builtin);
        assert_eq!(catalog.names(), ProfileCatalog::builtin().names());
    }

    #[test]
    fn override_file_replaces_a_builtin_profile_of_the_same_name() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("model_profiles.toml"),
            r#"
[[profiles]]
name = "coder"
tags = ["code"]
provider = "ollama"
model = "qwen3.5-coder"
cost_tier = "cheap"
priority = 99
"#,
        )
        .unwrap();

        let catalog = ProfileCatalog::load(home.path(), &ModelRouterConfig::default());
        let coder = catalog.get("coder").expect("coder must still exist");
        assert_eq!(coder.provider, "ollama");
        assert_eq!(coder.model, "qwen3.5-coder");
        assert_eq!(coder.cost_tier, CostTier::Cheap);
        assert_eq!(coder.priority, 99);

        // Untouched builtins survive, and the source records the override.
        assert!(catalog.get("architect").is_some());
        assert!(matches!(
            catalog.source(),
            CatalogSource::BuiltinPlusOverride(_)
        ));
    }

    #[test]
    fn override_file_adds_new_profiles_alongside_the_builtins() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("model_profiles.toml"),
            r#"
[[profiles]]
name = "local"
tags = ["offline"]
provider = "ollama"
model = "qwen3.5"
"#,
        )
        .unwrap();

        let catalog = ProfileCatalog::load(home.path(), &ModelRouterConfig::default());
        assert!(catalog.get("local").is_some());
        assert!(catalog.get("coder").is_some());
        assert_eq!(
            catalog.profiles().len(),
            ProfileCatalog::builtin().profiles().len() + 1
        );
    }

    #[test]
    fn override_file_honours_a_custom_profiles_path() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("custom.toml"),
            r#"
[[profiles]]
name = "custom-only"
provider = "ollama"
model = "qwen3.5"
"#,
        )
        .unwrap();

        let config = ModelRouterConfig {
            profiles_path: "custom.toml".to_string(),
            ..Default::default()
        };
        let catalog = ProfileCatalog::load(home.path(), &config);
        assert!(catalog.get("custom-only").is_some());
    }

    #[test]
    fn malformed_override_file_falls_back_to_the_builtins() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("model_profiles.toml"),
            "this is not valid toml [[[",
        )
        .unwrap();

        let catalog = ProfileCatalog::load(home.path(), &ModelRouterConfig::default());
        assert_eq!(catalog.source(), &CatalogSource::Builtin);
        assert_eq!(catalog.names(), ProfileCatalog::builtin().names());
    }

    #[test]
    fn overridden_profile_is_what_the_router_actually_selects() {
        // End-to-end over the two halves of this module: an operator's
        // override file is what reaches the routing decision, not the builtin.
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("model_profiles.toml"),
            r#"
[[profiles]]
name = "coder"
tags = ["refactor"]
provider = "ollama"
model = "qwen3.5-coder"
cost_tier = "cheap"
"#,
        )
        .unwrap();

        let config = ModelRouterConfig {
            enabled: true,
            complexity_threshold: 0.0,
            ..Default::default()
        };
        let catalog = ProfileCatalog::load(home.path(), &config);
        let (matched, decision) = match_profile(
            "refactor the parser",
            &score(0.4),
            catalog.profiles(),
            &config,
            None,
        );
        assert_eq!(decision, RoutingDecision::Matched);
        let chosen = matched.unwrap();
        assert_eq!(chosen.name, "coder");
        assert_eq!(chosen.model, "qwen3.5-coder");
        assert_eq!(chosen.provider, "ollama");
    }

    #[test]
    fn cached_load_picks_up_an_edited_override_file() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("model_profiles.toml");
        let config = ModelRouterConfig::default();

        std::fs::write(
            &path,
            "[[profiles]]\nname = \"coder\"\nprovider = \"ollama\"\nmodel = \"first\"\n",
        )
        .unwrap();
        assert_eq!(
            ProfileCatalog::load_cached(home.path(), &config)
                .get("coder")
                .unwrap()
                .model,
            "first"
        );

        // Rewrite with a different length so the (mtime, len) key moves even
        // on a filesystem with coarse timestamp granularity.
        std::fs::write(
            &path,
            "[[profiles]]\nname = \"coder\"\nprovider = \"ollama\"\nmodel = \"second-model\"\n",
        )
        .unwrap();
        assert_eq!(
            ProfileCatalog::load_cached(home.path(), &config)
                .get("coder")
                .unwrap()
                .model,
            "second-model",
            "an edited profiles file must take effect without a daemon restart"
        );
    }
}
