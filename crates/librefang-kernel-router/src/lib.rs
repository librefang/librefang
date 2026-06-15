use librefang_types::agent::AgentManifest;
use regex_lite::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const ROUTING_EXCLUDED_TEMPLATES: &[&str] = &["assistant"];
const GENERIC_ENGLISH_WORDS: &[&str] = &[
    "a",
    "agent",
    "an",
    "analysis",
    "and",
    "assistant",
    "checking",
    "create",
    "dedicated",
    "default",
    "drafting",
    "expert",
    "for",
    "friendly",
    "general",
    "general-purpose",
    "help",
    "helper",
    "helpful",
    "management",
    "multi-language",
    "multilingual",
    "of",
    "or",
    "planning",
    "preparation",
    "professional",
    "productivity",
    "research",
    "review",
    "senior",
    "specialist",
    "suggestions",
    "support",
    "system",
    "task",
    "template",
    "the",
    "tool",
    "to",
    "with",
    "workflow",
    "writing",
];

/// A template routing rule, owned so it can be loaded from TOML at runtime
/// rather than baked into the binary as `&'static`. `strong` / `weak` are
/// `(label, regex)` pairs; a strong hit scores [`EXPLICIT_ALIAS_WEIGHT`], a
/// weak hit [`WEAK_PHRASE_WEIGHT`].
#[derive(Debug, Clone)]
struct RouteRule {
    target: String,
    strong: Vec<(String, String)>,
    weak: Vec<(String, String)>,
}

/// Embedded default routing rules — the single source of truth for the
/// built-in specialist routes. Operators override per-target via
/// `$LIBREFANG_HOME/registry/templates/routing.toml` (see [`build_template_rules`]).
const DEFAULT_ROUTING_TOML: &str = include_str!("../default_routing.toml");

/// Serde shape for one `[[template]]` entry in a routing.toml.
#[derive(Debug, Clone, Deserialize)]
struct RouteRuleToml {
    target: String,
    #[serde(default)]
    strong: Vec<LabeledPattern>,
    #[serde(default)]
    weak: Vec<LabeledPattern>,
    /// `enabled = false` removes a default rule of the same `target` from
    /// routing (the only way to delete a bundled rule via an override file).
    #[serde(default = "default_enabled")]
    enabled: bool,
}

/// Serde shape for one `{ label, regex }` pattern.
#[derive(Debug, Clone, Deserialize)]
struct LabeledPattern {
    label: String,
    regex: String,
}

/// Serde shape for a whole routing.toml file.
#[derive(Debug, Clone, Deserialize, Default)]
struct RoutingTomlFile {
    #[serde(default)]
    template: Vec<RouteRuleToml>,
}

fn default_enabled() -> bool {
    true
}

impl RouteRuleToml {
    /// Flatten the serde shape into the owned `RouteRule` used by routing.
    fn into_rule(self) -> RouteRule {
        let map = |patterns: Vec<LabeledPattern>| {
            patterns
                .into_iter()
                .map(|p| (p.label, p.regex))
                .collect::<Vec<_>>()
        };
        RouteRule {
            target: self.target,
            strong: map(self.strong),
            weak: map(self.weak),
        }
    }
}

#[derive(Debug, Clone)]
struct HandRouteCandidate {
    hand_id: String,
    strong_phrases: Vec<String>,
    weak_phrases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HandSelection {
    pub hand_id: Option<String>,
    pub reason: String,
    pub score: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateSelection {
    pub template: String,
    pub reason: String,
    pub score: usize,
}

#[derive(Debug, Clone)]
struct ManifestRouteCandidate {
    template: String,
    /// User-configured aliases from [metadata.routing] — highest confidence.
    explicit_aliases: Vec<String>,
    /// Auto-generated phrases from name/description/tags — lower confidence.
    generated_phrases: Vec<String>,
    /// Weak aliases (explicit + generated from template name tokens).
    weak_phrases: Vec<String>,
}

/// Scoring weights for manifest routing.
const EXPLICIT_ALIAS_WEIGHT: usize = 6;
const GENERATED_PHRASE_WEIGHT: usize = 2;
const WEAK_PHRASE_WEIGHT: usize = 1;
/// Maximum semantic bonus points (scaled from 0.0–1.0 similarity).
const MAX_SEMANTIC_BONUS: f32 = 5.0;
/// Minimum semantic similarity to consider a semantic-only match.
const SEMANTIC_ONLY_THRESHOLD: f32 = 0.55;

// ── Hand routing: data-driven from HAND.toml ────────────────────────────

/// Cached hand route candidates built from bundled HAND.toml definitions.
/// Invalidated alongside `MANIFEST_CACHE` on hot-reload.
#[derive(Debug, Clone)]
struct HandRouteCacheEntry {
    home_dir: Option<String>,
    // `Arc` so per-message `hand_route_candidates()` hands out a refcount bump
    // instead of deep-cloning the candidate Vec on every routing call.
    candidates: Arc<Vec<HandRouteCandidate>>,
}

static HAND_ROUTE_CACHE: OnceLock<Mutex<Option<HandRouteCacheEntry>>> = OnceLock::new();
static HAND_ROUTE_HOME_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

/// Set the LibreFang home directory used for hand-route candidate loading.
pub fn set_hand_route_home_dir(home_dir: &Path) {
    let slot = HAND_ROUTE_HOME_DIR.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(home_dir.to_path_buf());
}

/// Invalidate the hand route cache (call alongside `invalidate_manifest_cache`).
pub fn invalidate_hand_route_cache() {
    if let Some(cache) = HAND_ROUTE_CACHE.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = None;
        }
    }
}

fn hand_route_candidates() -> Arc<Vec<HandRouteCandidate>> {
    let home_dir = resolve_hand_route_home_dir();
    let home_dir_key = Some(home_dir.to_string_lossy().to_string());
    let cache = HAND_ROUTE_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref cached) = *guard {
        if cached.home_dir == home_dir_key {
            return Arc::clone(&cached.candidates);
        }
    }

    let candidates = Arc::new(build_hand_route_candidates(Some(&home_dir)));
    *guard = Some(HandRouteCacheEntry {
        home_dir: home_dir_key,
        candidates: Arc::clone(&candidates),
    });
    candidates
}

fn resolve_hand_route_home_dir() -> PathBuf {
    if let Some(slot) = HAND_ROUTE_HOME_DIR.get() {
        let guard = slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(home_dir) = guard.as_ref() {
            return home_dir.clone();
        }
    }

    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return PathBuf::from(home);
    }

    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".librefang")
}

fn build_hand_route_candidates(home_dir: Option<&Path>) -> Vec<HandRouteCandidate> {
    let mut candidates_by_id: HashMap<String, HandRouteCandidate> = HashMap::new();

    if let Some(home_dir) = home_dir {
        for candidate in load_hand_route_candidates(home_dir) {
            candidates_by_id.insert(candidate.hand_id.clone(), candidate);
        }
    }

    let mut candidates: Vec<HandRouteCandidate> = candidates_by_id.into_values().collect();
    candidates.sort_by(|a, b| a.hand_id.cmp(&b.hand_id));
    candidates
}

fn load_hand_route_candidates(home_dir: &Path) -> Vec<HandRouteCandidate> {
    let mut seen = std::collections::HashSet::new();
    let mut candidates = Vec::new();

    let dirs = [home_dir.join("registry").join("hands")];

    // Pass the agents registry alongside HAND.toml parsing so hands that
    // declare `base = "<template>"` for their agents can resolve the
    // template. Without this the hand parser fails the flat path with
    // "requires agents registry directory" and emits a WARN on every
    // routing scan — and routing happens on every inbound message dispatch,
    // so the warning floods the log.
    let agents_dir = home_dir.join("registry").join("agents");
    let agents_dir_arg: Option<&Path> = if agents_dir.is_dir() {
        Some(agents_dir.as_path())
    } else {
        None
    };

    for hands_dir in &dirs {
        let Ok(entries) = fs::read_dir(hands_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let hand_dir = entry.path();
            if !hand_dir.is_dir() {
                continue;
            }
            let name = hand_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if !seen.insert(name.clone()) {
                continue;
            }
            let hand_toml = hand_dir.join("HAND.toml");
            let Ok(toml_content) = fs::read_to_string(&hand_toml) else {
                continue;
            };
            // Surface parse failures at WARN — the previous `let Ok else
            // continue` swallowed the error and the hand was silently
            // dropped from routing, hiding misconfigured HAND.toml files
            // (such as the `base = "<template>"` issue this PR fixes).
            match librefang_hands::registry::parse_hand_toml_with_agents_dir(
                &toml_content,
                "",
                std::collections::HashMap::new(),
                agents_dir_arg,
            ) {
                Ok(def) => candidates.push(hand_route_candidate_from_definition(def)),
                Err(e) => tracing::warn!(
                    hand = %name,
                    error = %e,
                    "Failed to parse HAND.toml for routing — hand will be unreachable",
                ),
            }
        }
    }

    candidates
}

fn hand_route_candidate_from_definition(
    def: librefang_hands::HandDefinition,
) -> HandRouteCandidate {
    // Strong: explicit aliases + description-derived phrases
    let mut strong = def.routing.aliases.clone();
    strong.extend(description_phrases(&def.description));

    // Weak: explicit weak_aliases + id-derived tokens
    let mut weak = def.routing.weak_aliases.clone();
    weak.extend(
        def.id
            .to_lowercase()
            .split(['-', '_'])
            .filter(|token| token.len() >= 3 && !GENERIC_ENGLISH_WORDS.contains(token))
            .map(str::to_string),
    );

    HandRouteCandidate {
        hand_id: def.id,
        strong_phrases: dedupe(strong),
        weak_phrases: dedupe(weak),
    }
}

// ── Template routing: data-driven from default_routing.toml + override ───

/// Cached merged template routing rules. Invalidated alongside
/// `MANIFEST_CACHE` and `HAND_ROUTE_CACHE` on hot-reload.
#[derive(Debug, Clone)]
struct TemplateRuleCacheEntry {
    home_dir: Option<String>,
    // `Arc` so the per-message `template_rules()` hands out a cheap refcount
    // bump instead of deep-cloning ~30 rules (each with several owned Strings)
    // on every inbound routing call.
    rules: Arc<Vec<RouteRule>>,
}

static TEMPLATE_RULE_CACHE: OnceLock<Mutex<Option<TemplateRuleCacheEntry>>> = OnceLock::new();

/// Invalidate the template rule cache (call alongside
/// [`invalidate_manifest_cache`] / [`invalidate_hand_route_cache`]).
pub fn invalidate_template_rule_cache() {
    if let Some(cache) = TEMPLATE_RULE_CACHE.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = None;
        }
    }
}

/// Resolve the active template routing rules: the embedded defaults merged
/// with any operator override at
/// `$LIBREFANG_HOME/registry/templates/routing.toml`. Cached per home dir;
/// rebuilt on hot-reload via [`invalidate_template_rule_cache`].
fn template_rules() -> Arc<Vec<RouteRule>> {
    // Template rules share the router home dir with hand routing.
    let home_dir = resolve_hand_route_home_dir();
    let home_dir_key = Some(home_dir.to_string_lossy().to_string());
    let cache = TEMPLATE_RULE_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref cached) = *guard {
        if cached.home_dir == home_dir_key {
            return Arc::clone(&cached.rules);
        }
    }
    let rules = Arc::new(build_template_rules(Some(&home_dir)));
    *guard = Some(TemplateRuleCacheEntry {
        home_dir: home_dir_key,
        rules: Arc::clone(&rules),
    });
    rules
}

/// Build the merged rule set: embedded defaults first (in file order, which
/// the scoring tie-break relies on), then operator overrides applied by
/// `target` — same target replaces in place (preserving position), a new
/// target appends, `enabled = false` removes. A missing override file leaves
/// the defaults untouched; an unreadable / unparseable file is logged at WARN
/// and the defaults are used, so routing never breaks on a bad override.
fn build_template_rules(home_dir: Option<&Path>) -> Vec<RouteRule> {
    let mut rules: Vec<RouteRule> = parse_routing_toml(DEFAULT_ROUTING_TOML)
        .unwrap_or_else(|e| {
            // The bundled default is a compile-time asset; a parse failure
            // means the binary itself is broken. Surface it loudly but keep
            // routing alive with an empty set rather than panicking the daemon.
            tracing::error!(error = %e, "Embedded default_routing.toml failed to parse");
            Vec::new()
        })
        .into_iter()
        // Honor `enabled = false` in the bundled default too, so the field has
        // one consistent meaning ("drop this rule") on both load paths.
        .filter(|entry| entry.enabled)
        .map(RouteRuleToml::into_rule)
        .collect();

    let Some(home_dir) = home_dir else {
        return rules;
    };
    let path = home_dir
        .join("registry")
        .join("templates")
        .join("routing.toml");
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return rules,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "Failed to read routing.toml override; using bundled default rules",
            );
            return rules;
        }
    };
    let overrides = match parse_routing_toml(&content) {
        Ok(overrides) => overrides,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "Failed to parse routing.toml override; using bundled default rules",
            );
            return rules;
        }
    };

    for entry in overrides {
        match rules.iter().position(|r| r.target == entry.target) {
            Some(pos) if entry.enabled => rules[pos] = entry.into_rule(),
            Some(pos) => {
                rules.remove(pos);
            }
            None if entry.enabled => rules.push(entry.into_rule()),
            None => {} // enabled = false on a non-existent target: no-op
        }
    }
    rules
}

/// Parse a routing.toml document into its `[[template]]` entries.
fn parse_routing_toml(content: &str) -> Result<Vec<RouteRuleToml>, toml::de::Error> {
    toml::from_str::<RoutingTomlFile>(content).map(|f| f.template)
}

/// Minimum score required for a hand match to be considered. A single weak
/// keyword hit (score 1) is too noisy — require at least one strong hit (3)
/// or two weak hits (2) to route to a hand.
const MIN_HAND_SCORE: usize = 2;

/// Select the best hand for a message using keyword matching.
///
/// Keywords are loaded from HAND.toml `[routing]` sections (English-only)
/// and augmented with description-derived phrases. For cross-lingual
/// matching, the caller can provide optional `semantic_scores` computed
/// via embedding cosine similarity against hand descriptions.
pub fn auto_select_hand(
    message: &str,
    semantic_scores: Option<&HashMap<String, f32>>,
) -> HandSelection {
    let mut scored: Vec<(usize, String, Vec<String>)> = Vec::new();

    for candidate in hand_route_candidates().iter() {
        let strong_hits: Vec<String> = candidate
            .strong_phrases
            .iter()
            .filter(|phrase| phrase_matches(message, phrase))
            .cloned()
            .collect();
        let weak_hits: Vec<String> = candidate
            .weak_phrases
            .iter()
            .filter(|phrase| phrase_matches(message, phrase))
            .cloned()
            .collect();
        let mut score =
            strong_hits.len() * EXPLICIT_ALIAS_WEIGHT + weak_hits.len() * WEAK_PHRASE_WEIGHT;

        // Blend semantic similarity when available
        if let Some(scores) = semantic_scores {
            if let Some(&sim) = scores.get(&candidate.hand_id) {
                let bonus = (sim * MAX_SEMANTIC_BONUS).round() as usize;
                score += bonus;
            }
        }

        if score >= MIN_HAND_SCORE {
            let mut hits = strong_hits;
            hits.extend(weak_hits);
            scored.push((score, candidate.hand_id.clone(), hits));
        }
    }

    if scored.is_empty() {
        return HandSelection {
            hand_id: None,
            reason: "no hand match".to_string(),
            score: 0,
        };
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.2.len().cmp(&a.2.len())));
    let (score, hand_id, hits) = scored.remove(0);

    HandSelection {
        hand_id: Some(hand_id.clone()),
        reason: format!("matched {hand_id} via {}", hits.join(", ")),
        score,
    }
}

pub fn auto_select_template(
    message: &str,
    agents_dir: &Path,
    semantic_scores: Option<&HashMap<String, f32>>,
) -> TemplateSelection {
    let normalized = message.to_lowercase();
    let metadata_match = auto_select_template_from_metadata(message, agents_dir, semantic_scores);
    let rules = template_rules();
    let mut scored: Vec<(usize, String, Vec<String>)> = Vec::new();

    for rule in rules.iter() {
        let strong_hits = matched_labels(message, &rule.strong);
        let weak_hits = matched_labels(message, &rule.weak);
        // Template rules are hand-curated (equivalent to explicit aliases)
        let mut score =
            strong_hits.len() * EXPLICIT_ALIAS_WEIGHT + weak_hits.len() * WEAK_PHRASE_WEIGHT;

        // Blend semantic similarity when available
        if let Some(scores) = semantic_scores {
            if let Some(&sim) = scores.get(&rule.target) {
                let bonus = (sim * MAX_SEMANTIC_BONUS).round() as usize;
                score += bonus;
            }
        }

        if score > 0 {
            let mut hits = strong_hits;
            hits.extend(weak_hits);
            scored.push((score, rule.target.clone(), hits));
        }
    }

    // When keyword matching found nothing, try semantic-only candidates from the rule set
    if scored.is_empty() {
        if let Some(scores) = semantic_scores {
            for rule in rules.iter() {
                if let Some(&sim) = scores.get(&rule.target) {
                    if sim >= SEMANTIC_ONLY_THRESHOLD {
                        let bonus = (sim * MAX_SEMANTIC_BONUS).round() as usize;
                        scored.push((bonus, rule.target.clone(), vec![]));
                    }
                }
            }
        }
    }

    if scored.is_empty() {
        return metadata_match.unwrap_or_else(|| TemplateSelection {
            template: "orchestrator".to_string(),
            reason: "no direct specialist match; defaulted to orchestrator".to_string(),
            score: 0,
        });
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.2.len().cmp(&a.2.len())));
    let (best_score, best_template, best_hits) = &scored[0];

    if scored.len() > 1 {
        let (second_score, second_template, _) = &scored[1];
        let multi_domain = ["同时", "分别", "协作", "多个", "multi", "together"]
            .iter()
            .any(|token| normalized.contains(token));
        if *second_score > 0 && best_template != second_template && multi_domain {
            return TemplateSelection {
                template: "orchestrator".to_string(),
                reason: format!(
                    "multiple specialties matched ({best_template}, {second_template}); routed to orchestrator"
                ),
                score: *best_score,
            };
        }
    }

    if let Some(metadata_match) = metadata_match {
        if metadata_match.template != *best_template
            && metadata_match.score > *best_score
            && (*best_score <= 1 || metadata_match.score >= *best_score + 2)
        {
            return metadata_match;
        }
    }

    let hits = best_hits.join(", ");
    TemplateSelection {
        template: best_template.clone(),
        reason: if hits.is_empty() {
            format!("matched {best_template}")
        } else {
            format!("matched {best_template} via {hits}")
        },
        score: *best_score,
    }
}

pub fn load_template_manifest(home_dir: &Path, template: &str) -> Result<AgentManifest, String> {
    load_template_manifest_at(&home_dir.join("workspaces").join("agents"), template)
}

fn auto_select_template_from_metadata(
    message: &str,
    agents_dir: &Path,
    semantic_scores: Option<&HashMap<String, f32>>,
) -> Option<TemplateSelection> {
    let mut scored: Vec<(usize, String, Vec<String>)> = Vec::new();

    for candidate in manifest_route_candidates(agents_dir).iter() {
        let explicit_hits: Vec<String> = candidate
            .explicit_aliases
            .iter()
            .filter(|phrase| phrase_matches(message, phrase))
            .cloned()
            .collect();
        let generated_hits: Vec<String> = candidate
            .generated_phrases
            .iter()
            .filter(|phrase| phrase_matches(message, phrase))
            .cloned()
            .collect();
        let weak_hits: Vec<String> = candidate
            .weak_phrases
            .iter()
            .filter(|phrase| phrase_matches(message, phrase))
            .cloned()
            .collect();
        let mut score = explicit_hits.len() * EXPLICIT_ALIAS_WEIGHT
            + generated_hits.len() * GENERATED_PHRASE_WEIGHT
            + weak_hits.len() * WEAK_PHRASE_WEIGHT;

        // Blend semantic similarity when available
        if let Some(scores) = semantic_scores {
            if let Some(&sim) = scores.get(candidate.template.as_str()) {
                let bonus = (sim * MAX_SEMANTIC_BONUS).round() as usize;
                score += bonus;
            }
        }

        if score > 0 {
            let mut hits = explicit_hits;
            hits.extend(generated_hits);
            hits.extend(weak_hits);
            scored.push((score, candidate.template.clone(), hits));
        }
    }

    // When keyword matching found nothing, try semantic-only candidates
    if scored.is_empty() {
        if let Some(scores) = semantic_scores {
            for candidate in manifest_route_candidates(agents_dir).iter() {
                if let Some(&sim) = scores.get(candidate.template.as_str()) {
                    if sim >= SEMANTIC_ONLY_THRESHOLD {
                        let bonus = (sim * MAX_SEMANTIC_BONUS).round() as usize;
                        scored.push((bonus, candidate.template.clone(), vec![]));
                    }
                }
            }
        }
    }

    if scored.is_empty() {
        return None;
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.2.len().cmp(&a.2.len())));
    let (score, template, hits) = scored.remove(0);

    Some(TemplateSelection {
        template: template.clone(),
        reason: format!(
            "matched {template} via manifest metadata: {}",
            hits.join(", ")
        ),
        score,
    })
}

/// Cached manifest route candidates, keyed by the `agents_dir` path used to
/// build them. Invalidated via `invalidate_manifest_cache()`, which should be
/// called on config hot-reload or agent install/uninstall.
type ManifestCacheEntry = (PathBuf, Arc<Vec<ManifestRouteCandidate>>);
static MANIFEST_CACHE: OnceLock<Mutex<Option<ManifestCacheEntry>>> = OnceLock::new();

/// Invalidate the cached manifest route candidates so they are rebuilt on the
/// next routing call. Call this after config hot-reload or agent changes.
pub fn invalidate_manifest_cache() {
    if let Some(cache) = MANIFEST_CACHE.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = None;
        }
    }
}

/// Returns (template_name, description) pairs for all routable templates.
/// Used by the kernel to build template description embeddings for semantic routing.
pub fn all_template_descriptions(agents_dir: &Path) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for template in all_template_names(agents_dir) {
        if ROUTING_EXCLUDED_TEMPLATES.contains(&template.as_str()) {
            continue;
        }
        if let Ok(manifest) = load_template_manifest_at(agents_dir, &template) {
            if !manifest.description.is_empty() {
                let embed_text = format!(
                    "{}: {}. Tags: {}",
                    manifest.name,
                    manifest.description,
                    manifest.tags.join(", ")
                );
                result.push((template, embed_text));
            }
        }
    }
    result
}

fn manifest_route_candidates(agents_dir: &Path) -> Arc<Vec<ManifestRouteCandidate>> {
    let cache = MANIFEST_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((ref cached_path, ref cached)) = *guard {
        if cached_path == agents_dir {
            return Arc::clone(cached);
        }
    }

    let candidates = Arc::new(build_manifest_route_candidates(agents_dir));
    *guard = Some((agents_dir.to_path_buf(), Arc::clone(&candidates)));
    candidates
}

fn build_manifest_route_candidates(agents_dir: &Path) -> Vec<ManifestRouteCandidate> {
    let mut candidates = Vec::new();

    for template in all_template_names(agents_dir) {
        if ROUTING_EXCLUDED_TEMPLATES.contains(&template.as_str()) {
            continue;
        }

        let Ok(manifest) = load_template_manifest_at(agents_dir, &template) else {
            continue;
        };
        let (routing_aliases, routing_weak_aliases, exclude_generated) =
            manifest_routing_config(&manifest);

        let generated = if exclude_generated {
            Vec::new()
        } else {
            let mut phrases = english_variants(&template);
            phrases.extend(tag_phrases(&manifest.tags));
            phrases.extend(description_phrases(&manifest.description));
            phrases
        };

        let mut weak_source = routing_weak_aliases;
        weak_source.extend(
            template
                .to_lowercase()
                .split(['-', '_'])
                .filter(|token| token.len() >= 3 && !GENERIC_ENGLISH_WORDS.contains(token))
                .map(str::to_string),
        );

        candidates.push(ManifestRouteCandidate {
            template,
            explicit_aliases: dedupe(routing_aliases),
            generated_phrases: dedupe(generated),
            weak_phrases: dedupe(weak_source),
        });
    }

    candidates
}

fn load_template_manifest_at(agents_dir: &Path, template: &str) -> Result<AgentManifest, String> {
    if !is_safe_template_name(template) {
        return Err(format!("invalid template name '{template}'"));
    }

    let manifest_path = agents_dir.join(template).join("agent.toml");
    if manifest_path.exists() {
        let manifest_toml = fs::read_to_string(&manifest_path)
            .map_err(|e| format!("failed to read {}: {e}", manifest_path.display()))?;
        return toml::from_str::<AgentManifest>(&manifest_toml)
            .map_err(|e| format!("failed to parse {}: {e}", manifest_path.display()));
    }

    Err(format!(
        "template '{template}' not found in {}. Run `librefang init` to sync agents from the registry.",
        agents_dir.display()
    ))
}

fn template_names_from_dir(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && path.join("agent.toml").exists() {
            if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
                if is_safe_template_name(name) {
                    names.push(name.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

fn all_template_names(agents_dir: &Path) -> Vec<String> {
    let mut names = template_names_from_dir(agents_dir);
    names.sort();
    names.dedup();
    names
}

fn is_safe_template_name(template: &str) -> bool {
    !template.is_empty()
        && template
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn matched_labels(message: &str, patterns: &[(String, String)]) -> Vec<String> {
    patterns
        .iter()
        .filter(|(_, pattern)| regex_matches(message, pattern))
        .map(|(label, _)| label.clone())
        .collect()
}

/// Upper bound on cached compiled regex patterns. Sized to comfortably
/// hold the largest realistic operator-curated set (per-channel
/// keyword routes, per-agent allowlists) plus headroom; once it's
/// exceeded the bounded eviction kicks in so the cache memory
/// footprint stays predictable even when patterns are
/// agent-or-config-controlled. Each entry is the source `String` +
/// the compiled `Regex`, low single-digit KB at most — so the cap
/// pins the worst case in the low tens of MB, not "unbounded".
/// Audit: regex-cache-unbounded.
const MAX_REGEX_CACHE_ENTRIES: usize = 4096;

/// Bounded compile cache for `regex_matches`. FIFO eviction (oldest
/// pattern out) rather than full LRU — for the router workload
/// (operator-curated route patterns, mostly stable) FIFO is a
/// strictly bounded approximation with no per-hit bookkeeping cost.
/// `entries` is `pattern -> Option<Regex>`: `None` caches a
/// compilation failure so a flood of invalid patterns doesn't
/// recompile every call. `order` records insertion order so eviction
/// is O(1).
struct RegexCache {
    entries: HashMap<String, Option<Regex>>,
    order: VecDeque<String>,
}

impl RegexCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Compile-and-cache: returns the cached compilation outcome for
    /// `pattern` (Some(Regex) on a syntactically valid pattern,
    /// None on a compile error). When inserting a new entry past
    /// the cap, evicts the oldest cached entry first.
    fn get_or_compile(&mut self, pattern: &str) -> Option<&Regex> {
        if !self.entries.contains_key(pattern) {
            // Evict before insert so we never breach the cap, even
            // for one tick. Loop because an operator could in theory
            // lower the cap at compile time and the cache would have
            // backlog above the new ceiling on the first call.
            while self.entries.len() >= MAX_REGEX_CACHE_ENTRIES {
                if let Some(oldest) = self.order.pop_front() {
                    self.entries.remove(&oldest);
                } else {
                    break;
                }
            }
            let compiled = Regex::new(&format!("(?i){pattern}")).ok();
            self.entries.insert(pattern.to_string(), compiled);
            self.order.push_back(pattern.to_string());
        }
        // Unwrap-safe: we just inserted on miss; on hit the
        // contains_key check guards the entry.
        self.entries
            .get(pattern)
            .expect("entry inserted on miss path or guarded by contains_key on hit path")
            .as_ref()
    }
}

/// Global cache of compiled regex patterns. Keyed by the raw pattern
/// string with FIFO eviction at [`MAX_REGEX_CACHE_ENTRIES`] — avoids
/// recompiling the same patterns on every incoming message while
/// preventing the unbounded-growth DoS the audit flagged (an agent /
/// manifest pattern would otherwise live in the cache forever).
static REGEX_CACHE: OnceLock<Mutex<RegexCache>> = OnceLock::new();

fn regex_matches(message: &str, pattern: &str) -> bool {
    let cache = REGEX_CACHE.get_or_init(|| Mutex::new(RegexCache::new()));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    // None == compile error → never matches. Mirrors the historical
    // "never-match sentinel" branch but without the panic-risk of
    // the previous `Regex::new("(?!x)x").unwrap()` (regex_lite
    // doesn't support look-around, so the sentinel would have
    // panicked the first time any invalid pattern reached this
    // path).
    guard
        .get_or_compile(pattern)
        .map(|r| r.is_match(message))
        .unwrap_or(false)
}

fn english_variants(text: &str) -> Vec<String> {
    let normalized = text.trim().to_lowercase();
    if normalized.is_empty() {
        return Vec::new();
    }

    let mut variants = vec![normalized.clone()];
    if normalized.contains('-') || normalized.contains('_') {
        variants.push(normalized.replace(['-', '_'], " "));
        variants.extend(
            normalized
                .split(['-', '_'])
                .filter(|part| part.len() >= 3)
                .map(str::to_string),
        );
    }
    dedupe(variants)
}

fn description_phrases(description: &str) -> Vec<String> {
    let text = description.trim();
    if text.is_empty() {
        return Vec::new();
    }

    let mut phrases = Vec::new();

    for chunk in split_phrase_chunks(text) {
        if chunk.is_empty() {
            continue;
        }

        if is_ascii_phrase(&chunk) {
            phrases.extend(ascii_phrase_candidates(&chunk, 4));
        } else if is_meaningful_unicode_phrase(&chunk) {
            phrases.push(chunk);
        }
    }

    dedupe(phrases)
}

fn tag_phrases(tags: &[String]) -> Vec<String> {
    let mut phrases = Vec::new();

    for tag in tags {
        let normalized = tag.trim();
        if normalized.is_empty() {
            continue;
        }

        if is_ascii_phrase(normalized) {
            phrases.extend(ascii_phrase_candidates(normalized, 3));
        } else if is_meaningful_unicode_phrase(normalized) {
            phrases.push(normalized.to_string());
        }
    }

    dedupe(phrases)
}

fn manifest_routing_config(manifest: &AgentManifest) -> (Vec<String>, Vec<String>, bool) {
    let Some(Value::Object(routing)) = manifest.metadata.get("routing") else {
        return (Vec::new(), Vec::new(), false);
    };

    let mut aliases = json_string_list(routing.get("aliases"));
    aliases.extend(json_string_list(routing.get("strong_aliases")));

    (
        aliases,
        json_string_list(routing.get("weak_aliases")),
        routing
            .get("exclude_generated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
}

fn json_string_list(value: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn phrase_matches(message: &str, phrase: &str) -> bool {
    let candidate = phrase.trim();
    if candidate.is_empty() {
        return false;
    }

    if is_ascii_phrase(candidate) {
        // Reuse the global REGEX_CACHE so the same phrase across many incoming
        // messages compiles only once (#3491). The cache key already includes
        // the same `(?i)` casing applied here, so we avoid double-compilation
        // by using `regex_matches` with a pattern that mirrors that contract.
        let escaped = regex_lite::escape(&candidate.to_lowercase()).replace("\\ ", r"[\s_-]+");
        let pattern = format!(r"(^|[^a-z0-9]){}([^a-z0-9]|$)", escaped);
        return regex_matches(&message.to_lowercase(), &pattern);
    }

    message.to_lowercase().contains(&candidate.to_lowercase())
}

fn split_phrase_chunks(text: &str) -> Vec<String> {
    text.split(is_phrase_separator)
        .filter_map(normalize_phrase_chunk)
        .collect()
}

fn is_phrase_separator(ch: char) -> bool {
    ch == '\n'
        || ch == '\r'
        || ch == '\t'
        || (ch.is_ascii_punctuation() && !matches!(ch, '-' | '_'))
        || matches!(
            ch,
            '\u{3001}' // 、
                | '\u{3002}' // 。
                | '\u{FF0C}' // ，
                | '\u{FF1B}' // ；
                | '\u{FF1A}' // ：
                | '\u{FF08}' // （
                | '\u{FF09}' // ）
                | '\u{2013}' // –
                | '\u{2014}' // —
        )
}

fn normalize_phrase_chunk(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-');
    if trimmed.is_empty() {
        return None;
    }

    if !is_ascii_phrase(trimmed) {
        return Some(trimmed.to_string());
    }

    let words: Vec<&str> = trimmed
        .split([' ', '-', '_'])
        .filter(|word| !word.is_empty())
        .collect();
    let start = words
        .iter()
        .position(|word| !GENERIC_ENGLISH_WORDS.contains(&word.to_ascii_lowercase().as_str()))
        .unwrap_or(words.len());
    let end = words
        .iter()
        .rposition(|word| !GENERIC_ENGLISH_WORDS.contains(&word.to_ascii_lowercase().as_str()))
        .map(|idx| idx + 1)
        .unwrap_or(0);

    if start >= end {
        return None;
    }

    Some(words[start..end].join(" "))
}

fn ascii_phrase_candidates(text: &str, min_len: usize) -> Vec<String> {
    let normalized = text.trim().to_lowercase();
    if normalized.is_empty() {
        return Vec::new();
    }

    let content_words: Vec<String> = normalized
        .split([' ', '-', '_'])
        .filter(|word| word.len() >= min_len && !GENERIC_ENGLISH_WORDS.contains(word))
        .map(str::to_string)
        .collect();
    let mut phrases = Vec::new();

    if normalized.len() >= min_len
        && normalized.split_whitespace().count() <= 4
        && normalized
            .split_whitespace()
            .any(|word| !GENERIC_ENGLISH_WORDS.contains(&word))
    {
        phrases.extend(english_variants(&normalized));
    }

    phrases.extend(content_words.iter().cloned());
    for window in content_words.windows(2) {
        phrases.push(window.join(" "));
    }

    dedupe(phrases)
}

fn is_meaningful_unicode_phrase(text: &str) -> bool {
    (2..=32).contains(&text.chars().count())
}

fn is_ascii_phrase(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '_' | '-'))
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();

    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            ordered.push(trimmed.to_string());
        }
    }

    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ensure_registry() {
        use std::sync::Once;
        static SYNC_ONCE: Once = Once::new();
        SYNC_ONCE.call_once(|| {
            let test_home = librefang_runtime::registry_sync::resolve_home_dir_for_tests();
            set_hand_route_home_dir(&test_home);
            invalidate_hand_route_cache();
        });
    }

    /// Helper: call auto_select_hand without semantic scores.
    fn hand(msg: &str) -> HandSelection {
        ensure_registry();
        auto_select_hand(msg, None)
    }

    fn write_test_hand(home_dir: &Path, hand_id: &str, aliases: &[&str], weak_aliases: &[&str]) {
        let hand_dir = home_dir.join("registry").join("hands").join(hand_id);
        fs::create_dir_all(&hand_dir).unwrap();

        let aliases_toml = aliases
            .iter()
            .map(|alias| format!("\"{alias}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let weak_aliases_toml = weak_aliases
            .iter()
            .map(|alias| format!("\"{alias}\""))
            .collect::<Vec<_>>()
            .join(", ");

        let hand_toml = format!(
            r#"
id = "{hand_id}"
name = "Test {hand_id}"
description = "Custom hand for tests"
category = "data"

[routing]
aliases = [{aliases_toml}]
weak_aliases = [{weak_aliases_toml}]

[agent]
name = "{hand_id}-agent"
description = "Test hand agent"
system_prompt = "Test prompt"
"#
        );

        fs::write(hand_dir.join("HAND.toml"), hand_toml).unwrap();
    }

    #[test]
    fn test_auto_select_hand_prefers_browser_tasks() {
        let selection = hand("open website and navigate to the login page");
        assert_eq!(selection.hand_id, Some("browser".to_string()));
        assert!(selection.score > 0);
    }

    #[test]
    fn test_auto_select_template_prefers_explicit_coder_rule() {
        // Invalidate cache to ensure clean state for subsequent tests
        invalidate_manifest_cache();

        let selection = auto_select_template(
            "请实现一个新的 Rust API 并补丁修复它",
            Path::new("/tmp/does-not-exist"),
            None,
        );
        assert_eq!(selection.template, "coder");
        assert!(selection.score > 0);
    }

    #[test]
    fn test_auto_select_template_can_use_manifest_metadata() {
        // Invalidate manifest cache and force rebuild to ensure fresh scan
        invalidate_manifest_cache();

        let tmp = tempdir().unwrap();
        let agents_dir = tmp.path().join("agents");
        let template_dir = agents_dir.join("release-notes");
        fs::create_dir_all(&template_dir).unwrap();
        fs::write(
            template_dir.join("agent.toml"),
            r#"
name = "release-notes"
description = "Drafts release notes and changelogs."
module = "builtin:chat"
tags = ["release-notes", "changelog"]

[model]
provider = "default"
model = "default"
system_prompt = "unused"

[metadata.routing]
aliases = ["release notes"]
weak_aliases = ["changelog"]
"#,
        )
        .unwrap();

        let selection = auto_select_template(
            "Please draft release notes for version 1.2.3",
            &agents_dir,
            None,
        );
        assert_eq!(selection.template, "release-notes");
        assert!(selection.score > 0);
    }

    #[test]
    fn test_auto_select_template_routes_multi_domain_to_orchestrator() {
        // Invalidate cache to ensure clean state for subsequent tests
        invalidate_manifest_cache();

        let selection = auto_select_template(
            "请同时写代码并做深度调研，然后协作输出方案",
            Path::new("/tmp/does-not-exist"),
            None,
        );
        assert_eq!(selection.template, "orchestrator");
        assert!(selection.score > 0);
    }

    #[test]
    fn test_description_phrases_extract_language_agnostic_keywords() {
        let phrases = description_phrases(
            "Friendly multi-language translation agent for document translation, localization, and cross-cultural communication.",
        );
        assert!(phrases.contains(&"translation".to_string()));
        assert!(phrases.contains(&"document".to_string()));
        assert!(phrases.contains(&"localization".to_string()));
        assert!(phrases.contains(&"cross cultural".to_string()));
        assert!(!phrases.contains(&"friendly".to_string()));
    }

    #[test]
    fn test_tag_phrases_keep_non_ascii_tags_without_language_specific_rules() {
        let phrases = tag_phrases(&["分析".to_string(), "release-notes".to_string()]);
        assert!(phrases.contains(&"分析".to_string()));
        assert!(phrases.contains(&"release notes".to_string()));
    }

    #[test]
    fn test_bundled_template_metadata_routes_common_intents() {
        let cases = [
            (
                "Perform a threat model and vulnerability review for this service",
                "security-auditor",
            ),
            ("Draft a reply email to this customer", "email-assistant"),
            (
                "Create a travel itinerary for Kyoto this weekend",
                "travel-planner",
            ),
            ("Translate this product page into Japanese", "translator"),
            ("Write a test plan for this release", "test-engineer"),
            (
                "Prepare meeting notes and action items",
                "meeting-assistant",
            ),
            (
                "Help me design a system architecture for this service",
                "architect",
            ),
            (
                "Break this project into milestones and dependencies",
                "planner",
            ),
            ("Investigate this bug and find the root cause", "debugger"),
            (
                "Do deep web research and gather sources on this topic",
                "researcher",
            ),
        ];

        for (message, expected) in cases {
            let selection = auto_select_template(message, Path::new("/tmp/does-not-exist"), None);
            assert_eq!(selection.template, expected, "message: {message}");
            assert!(selection.score > 0, "message: {message}");
        }
    }

    // ── Hand routing: all hands coverage (English keywords from HAND.toml) ──

    #[test]
    fn test_auto_select_hand_routes_collector() {
        let sel = hand("please monitor changes on this repo and track updates");
        assert_eq!(sel.hand_id, Some("collector".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_researcher() {
        let sel = hand("do a deep research and systematic review of the landscape");
        assert_eq!(sel.hand_id, Some("researcher".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_clip() {
        let sel = hand("clip video and do subtitle extraction");
        assert_eq!(sel.hand_id, Some("clip".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_predictor() {
        let sel = hand("predict the probability and forecast this outcome");
        assert_eq!(sel.hand_id, Some("predictor".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_trader() {
        let sel = hand("check my portfolio and do market analysis");
        assert_eq!(sel.hand_id, Some("trader".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_lead() {
        let sel = hand("do lead generation and build a prospect list");
        assert_eq!(sel.hand_id, Some("lead".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_analytics() {
        let sel = hand("run data analysis and create a dashboard report");
        assert_eq!(sel.hand_id, Some("analytics".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_apitester() {
        let sel = hand("run an api test and endpoint test on this service");
        assert_eq!(sel.hand_id, Some("apitester".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_devops() {
        let sel = hand("set up ci/cd pipeline and infrastructure monitoring");
        assert_eq!(sel.hand_id, Some("devops".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_strategist() {
        let sel = hand("do a strategic analysis and competitive analysis");
        assert_eq!(sel.hand_id, Some("strategist".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_linkedin() {
        let sel = hand("optimize my linkedin profile optimization strategy");
        assert_eq!(sel.hand_id, Some("linkedin".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_reddit() {
        let sel = hand("post on reddit subreddit and monitor replies");
        assert_eq!(sel.hand_id, Some("reddit".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_twitter() {
        let sel = hand("post a tweet on twitter with scheduled tweet");
        assert_eq!(sel.hand_id, Some("twitter".to_string()));
    }

    // ── MIN_HAND_SCORE threshold ────────────────────────────────────

    #[test]
    fn test_weak_only_single_match_rejected() {
        // Single weak keyword should be below MIN_HAND_SCORE=2
        let sel = hand("help me deploy");
        assert_eq!(sel.hand_id, None, "single weak match should be rejected");
        assert_eq!(sel.score, 0);
    }

    #[test]
    fn test_strong_match_always_passes_threshold() {
        // A single strong match = score 3 >= MIN_HAND_SCORE(2)
        let sel = hand("open website please");
        assert_eq!(sel.hand_id, Some("browser".to_string()));
        assert!(sel.score >= 2);
    }

    // ── Negative / boundary tests ───────────────────────────────────

    #[test]
    fn test_generic_greeting_no_hand_match() {
        let sel = hand("hello, how is the weather today?");
        assert_eq!(sel.hand_id, None);
    }

    #[test]
    fn test_generic_english_no_hand_match() {
        let sel = hand("Hello, how are you today?");
        assert_eq!(sel.hand_id, None);
    }

    #[test]
    fn test_coding_request_no_hand_match() {
        let sel = hand("please write a Rust function for me");
        assert_eq!(sel.hand_id, None);
    }

    #[test]
    fn test_ambiguous_short_message_no_hand_match() {
        let sel = hand("help me look at this");
        assert_eq!(sel.hand_id, None);
    }

    // ── Scoring: strong beats weak ──────────────────────────────────

    #[test]
    fn test_strong_match_scores_higher_than_weak() {
        // "deep research" is a strong alias for researcher (score 3+)
        let strong = hand("do deep research on this topic");
        // "research" alone is weak for researcher (score 1, rejected)
        let weak = hand("research this");
        assert!(strong.score > weak.score);
    }

    #[test]
    fn test_multiple_strong_matches_boost_score() {
        let single = hand("run data analysis");
        let double = hand("run data analysis and build a dashboard with automated report");
        assert!(double.score >= single.score);
    }

    // ── Semantic score blending ─────────────────────────────────────

    #[test]
    fn test_semantic_scores_boost_hand_selection() {
        ensure_registry();
        // Without semantic: generic message should not match any hand
        let without = hand("please help me with this task");
        assert_eq!(without.hand_id, None);

        // With semantic: simulated high similarity to "collector"
        let mut scores = HashMap::new();
        scores.insert("collector".to_string(), 0.9);
        let with = auto_select_hand("please help me with this task", Some(&scores));
        assert_eq!(with.hand_id, Some("collector".to_string()));
        assert!(with.score >= MIN_HAND_SCORE);
    }

    #[test]
    fn test_semantic_fallback_routes_chinese_to_collector() {
        ensure_registry();
        // Chinese input: no English keyword match → score 0 without semantic
        let without = hand("帮我监控这个网站的变更");
        assert_eq!(
            without.hand_id, None,
            "Chinese should not match English keywords"
        );

        // With embedding similarity: "帮我监控这个网站的变更" would be semantically
        // close to collector's description "monitors any target continuously with
        // change detection". Simulated here with a high cosine score.
        let mut scores = HashMap::new();
        scores.insert("collector".to_string(), 0.85);
        scores.insert("browser".to_string(), 0.3);
        let with = auto_select_hand("帮我监控这个网站的变更", Some(&scores));
        assert_eq!(with.hand_id, Some("collector".to_string()));
    }

    #[test]
    fn test_semantic_fallback_routes_japanese_to_trader() {
        ensure_registry();
        // Japanese: "株式取引のポートフォリオを確認して" (check stock trading portfolio)
        let without = hand("株式取引のポートフォリオを確認して");
        assert_eq!(without.hand_id, None);

        let mut scores = HashMap::new();
        scores.insert("trader".to_string(), 0.82);
        scores.insert("analytics".to_string(), 0.25);
        let with = auto_select_hand("株式取引のポートフォリオを確認して", Some(&scores));
        assert_eq!(with.hand_id, Some("trader".to_string()));
    }

    #[test]
    fn test_semantic_fallback_routes_korean_to_researcher() {
        ensure_registry();
        // Korean: "이 주제에 대해 심층 연구를 해주세요" (do deep research on this topic)
        let mut scores = HashMap::new();
        scores.insert("researcher".to_string(), 0.88);
        let with = auto_select_hand("이 주제에 대해 심층 연구를 해주세요", Some(&scores));
        assert_eq!(with.hand_id, Some("researcher".to_string()));
    }

    #[test]
    fn test_semantic_low_similarity_does_not_match() {
        ensure_registry();
        // All scores below threshold: similarity too low to trigger routing
        let mut scores = HashMap::new();
        scores.insert("collector".to_string(), 0.2);
        scores.insert("browser".to_string(), 0.15);
        scores.insert("trader".to_string(), 0.1);
        let sel = auto_select_hand("一些随便的话", Some(&scores));
        // 0.2 * 3 = 0.6, rounds to 1 — below MIN_HAND_SCORE(2)
        assert_eq!(sel.hand_id, None, "low similarity should not match");
    }

    #[test]
    fn test_semantic_plus_keyword_combined_scoring() {
        ensure_registry();
        // English keyword gives partial score, semantic boosts it over threshold
        // "deploy" is a weak alias for devops (score 1, below threshold alone)
        let keyword_only = hand("help me deploy the service");
        // May or may not match depending on whether deploy hits weak alias
        let keyword_score = keyword_only.score;

        // With semantic boost: devops similarity adds bonus points
        let mut scores = HashMap::new();
        scores.insert("devops".to_string(), 0.75);
        let combined = auto_select_hand("help me deploy the service", Some(&scores));
        assert!(
            combined.score > keyword_score,
            "semantic should boost keyword score"
        );
        assert_eq!(combined.hand_id, Some("devops".to_string()));
    }

    #[test]
    fn test_no_embedding_graceful_degradation() {
        ensure_registry();
        // When semantic_scores is None, only keyword matching is used.
        // Non-English input simply gets no match (graceful, not error).
        let sel = auto_select_hand("请帮我做数据分析", None);
        assert_eq!(sel.hand_id, None, "should gracefully return no match");
        assert_eq!(sel.score, 0);
    }

    #[test]
    fn test_semantic_does_not_override_strong_keyword() {
        ensure_registry();
        // If keyword matching strongly matches hand A, but semantic scores
        // favor hand B, keyword should still win (keyword score is higher).
        let mut scores = HashMap::new();
        scores.insert("trader".to_string(), 0.9); // semantic favors trader
                                                  // But message strongly matches browser via keywords
        let sel = auto_select_hand("open website and navigate to the login page", Some(&scores));
        // Browser should win because keyword score (3+) > trader semantic (2-3)
        assert_eq!(sel.hand_id, Some("browser".to_string()));
    }

    // ── Cache consistency ───────────────────────────────────────────

    #[test]
    fn test_hand_route_cache_returns_consistent_results() {
        let r1 = hand("open website and fill form");
        let r2 = hand("open website and fill form");
        assert_eq!(r1.hand_id, r2.hand_id);
        assert_eq!(r1.score, r2.score);
    }

    #[test]
    fn test_build_hand_route_candidates_loads_user_installed_hands() {
        let tmp = tempdir().unwrap();
        write_test_hand(
            tmp.path(),
            "uptime-watcher",
            &["uptime pulse monitor"],
            &["uptime pulse"],
        );

        let candidates = build_hand_route_candidates(Some(tmp.path()));
        let custom = candidates
            .iter()
            .find(|candidate| candidate.hand_id == "uptime-watcher")
            .expect("user-installed hand should be loaded");

        assert!(custom
            .strong_phrases
            .iter()
            .any(|phrase| phrase == "uptime pulse monitor"));
        assert!(custom
            .weak_phrases
            .iter()
            .any(|phrase| phrase == "uptime pulse"));
    }

    #[test]
    fn test_build_hand_route_candidates_ignores_invalid_user_hand_manifests() {
        let tmp = tempdir().unwrap();
        let hand_dir = tmp.path().join("registry").join("hands").join("broken");
        fs::create_dir_all(&hand_dir).unwrap();
        fs::write(hand_dir.join("HAND.toml"), "not = valid = toml").unwrap();

        let candidates = build_hand_route_candidates(Some(tmp.path()));
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.hand_id != "broken"),
            "invalid HAND.toml should be skipped"
        );
    }

    #[test]
    fn test_load_template_manifest_not_found_returns_error() {
        let tmp = tempdir().unwrap();
        assert!(load_template_manifest(tmp.path(), "nonexistent").is_err());
    }

    #[test]
    fn test_load_template_manifest_from_disk() {
        let tmp = tempdir().unwrap();
        let template_dir = tmp
            .path()
            .join("workspaces")
            .join("agents")
            .join("assistant");
        fs::create_dir_all(&template_dir).unwrap();
        fs::write(
            template_dir.join("agent.toml"),
            r#"
name = "assistant"
description = "Local override"
module = "builtin:chat"

[model]
provider = "default"
model = "default"
system_prompt = "override"
"#,
        )
        .unwrap();

        let manifest = load_template_manifest(tmp.path(), "assistant").unwrap();
        assert_eq!(manifest.description, "Local override");
    }

    // NOTE: builtin:router agent was removed. The test
    // `test_builtin_router_spawns_metadata_template_and_cleans_up` was deleted.
    // Assistant now handles routing directly via LLM tools.

    /// Audit: regex-cache-unbounded. The fix replaced the unbounded
    /// `HashMap<String, Regex>` with a FIFO-evicting cache capped at
    /// `MAX_REGEX_CACHE_ENTRIES`. These tests exercise the local
    /// `RegexCache` struct directly (the global static is shared
    /// across the whole test binary; mutating it would flake other
    /// tests).
    #[test]
    fn regex_cache_reuses_compiled_pattern_within_capacity() {
        let mut cache = RegexCache::new();
        let r1 = cache.get_or_compile("hello").unwrap() as *const Regex;
        let r2 = cache.get_or_compile("hello").unwrap() as *const Regex;
        assert!(
            std::ptr::eq(r1, r2),
            "second lookup for the same pattern must return the cached compilation"
        );
        assert_eq!(
            cache.entries.len(),
            1,
            "exactly one entry for a single distinct pattern"
        );
    }

    #[test]
    fn regex_cache_evicts_oldest_when_capacity_exceeded() {
        // Drive the cache past MAX_REGEX_CACHE_ENTRIES with N+2
        // distinct patterns. The eviction policy is FIFO so the
        // first two patterns must be gone; the last MAX must
        // remain.
        let mut cache = RegexCache::new();
        for i in 0..(MAX_REGEX_CACHE_ENTRIES + 2) {
            let pat = format!("pat{i}");
            cache.get_or_compile(&pat);
        }
        assert_eq!(
            cache.entries.len(),
            MAX_REGEX_CACHE_ENTRIES,
            "cache must never grow past MAX_REGEX_CACHE_ENTRIES"
        );
        // The two oldest patterns were evicted; the newest two
        // are still present.
        assert!(!cache.entries.contains_key("pat0"));
        assert!(!cache.entries.contains_key("pat1"));
        let newest = format!("pat{}", MAX_REGEX_CACHE_ENTRIES + 1);
        assert!(cache.entries.contains_key(&newest));
    }

    #[test]
    fn regex_cache_match_behavior_survives_eviction() {
        // After being evicted, the same pattern must still produce
        // the same match outcome — the eviction is a memory bound,
        // not a behavioural one. Re-fetching pays a compile cost
        // but the match result is identical.
        let mut cache = RegexCache::new();
        let matches_before = cache
            .get_or_compile("hello")
            .unwrap()
            .is_match("hello world");
        assert!(matches_before);
        // Flood the cache so "hello" gets evicted.
        for i in 0..(MAX_REGEX_CACHE_ENTRIES + 1) {
            cache.get_or_compile(&format!("flood{i}"));
        }
        assert!(!cache.entries.contains_key("hello"));
        // Re-fetch — must compile fresh and still match.
        let matches_after = cache
            .get_or_compile("hello")
            .unwrap()
            .is_match("hello world");
        assert!(
            matches_after,
            "match outcome must survive an eviction round-trip"
        );
    }

    #[test]
    fn regex_cache_invalid_pattern_caches_compile_failure_as_none() {
        // A syntactically invalid pattern caches `None` (compile
        // failure) so a flood of bad patterns doesn't re-spend the
        // regex compiler on every call AND doesn't panic. Replaces
        // the historical `Regex::new("(?!x)x").unwrap()` sentinel
        // which would have panicked under `regex_lite` (no
        // look-around). The user-visible behaviour
        // (`regex_matches` returns false for invalid patterns) is
        // preserved.
        let mut cache = RegexCache::new();
        let outcome = cache.get_or_compile("[invalid");
        assert!(outcome.is_none(), "invalid pattern must cache as None");
        assert!(
            cache.entries.contains_key("[invalid"),
            "the failure result still occupies a cache slot so a flood of \
             invalid patterns can't recompile on every call"
        );
        // Second call must hit the cached failure, not recompile.
        let second = cache.get_or_compile("[invalid");
        assert!(second.is_none());
        assert_eq!(cache.entries.len(), 1);
    }

    // ── Template rule loading / override merge ───────────────────────────

    fn write_routing_override(home_dir: &Path, body: &str) {
        let dir = home_dir.join("registry").join("templates");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("routing.toml"), body).unwrap();
    }

    #[test]
    fn embedded_default_routing_toml_is_valid() {
        // The include_str! asset must always parse — a broken default would
        // silently empty the rule set in production.
        let parsed =
            parse_routing_toml(DEFAULT_ROUTING_TOML).expect("bundled default routing.toml parses");
        assert_eq!(parsed.len(), 30);
        assert!(parsed.iter().all(|r| !r.strong.is_empty()));
    }

    #[test]
    fn default_rules_preserve_file_order() {
        // No home dir → bundled defaults only. File order is load-bearing for
        // the scoring tie-break, so assert the historical first/last targets.
        let rules = build_template_rules(None);
        assert_eq!(rules.len(), 30);
        assert_eq!(rules.first().unwrap().target, "hello-world");
        assert_eq!(rules.last().unwrap().target, "orchestrator");
    }

    #[test]
    fn missing_override_file_uses_defaults() {
        let tmp = tempdir().unwrap();
        let rules = build_template_rules(Some(tmp.path()));
        assert_eq!(rules.len(), 30);
        assert_eq!(rules.first().unwrap().target, "hello-world");
    }

    #[test]
    fn override_replaces_rule_in_place() {
        let tmp = tempdir().unwrap();
        write_routing_override(
            tmp.path(),
            r#"
[[template]]
target = "coder"
strong = [{ label = "造轮子", regex = "造个轮子|攒一个" }]
"#,
        );
        let rules = build_template_rules(Some(tmp.path()));
        // Count unchanged and coder kept its original index (1) — a same-target
        // override replaces in place, never reorders.
        assert_eq!(rules.len(), 30);
        assert_eq!(rules[1].target, "coder");
        assert_eq!(
            rules[1].strong,
            vec![("造轮子".to_string(), "造个轮子|攒一个".to_string())]
        );
        assert!(
            rules[1].weak.is_empty(),
            "override with no weak clears weak"
        );
    }

    #[test]
    fn override_appends_new_target() {
        let tmp = tempdir().unwrap();
        write_routing_override(
            tmp.path(),
            r#"
[[template]]
target = "my-bot"
strong = [{ label = "bot", regex = "\\bmybot\\b" }]
"#,
        );
        let rules = build_template_rules(Some(tmp.path()));
        assert_eq!(rules.len(), 31);
        assert_eq!(rules.last().unwrap().target, "my-bot");
    }

    #[test]
    fn override_disables_default_target() {
        let tmp = tempdir().unwrap();
        write_routing_override(
            tmp.path(),
            "[[template]]\ntarget = \"recipe-assistant\"\nenabled = false\n",
        );
        let rules = build_template_rules(Some(tmp.path()));
        assert_eq!(rules.len(), 29);
        assert!(rules.iter().all(|r| r.target != "recipe-assistant"));
    }

    #[test]
    fn disable_on_unknown_target_is_noop() {
        let tmp = tempdir().unwrap();
        write_routing_override(
            tmp.path(),
            "[[template]]\ntarget = \"does-not-exist\"\nenabled = false\n",
        );
        let rules = build_template_rules(Some(tmp.path()));
        assert_eq!(rules.len(), 30);
    }

    #[test]
    fn unparseable_override_falls_back_to_defaults() {
        let tmp = tempdir().unwrap();
        write_routing_override(tmp.path(), "this is = = not valid toml");
        // Fail-soft: a broken override must not empty routing.
        let rules = build_template_rules(Some(tmp.path()));
        assert_eq!(rules.len(), 30);
        assert_eq!(rules.first().unwrap().target, "hello-world");
    }

    #[test]
    fn bad_regex_in_override_loads_without_error() {
        // An invalid regex compiles to None in RegexCache (never matches) but
        // must not fail loading/merging.
        let tmp = tempdir().unwrap();
        write_routing_override(
            tmp.path(),
            r#"
[[template]]
target = "coder"
strong = [{ label = "bad", regex = "(unclosed" }]
"#,
        );
        let rules = build_template_rules(Some(tmp.path()));
        assert_eq!(rules.len(), 30);
        let coder = rules.iter().find(|r| r.target == "coder").unwrap();
        assert_eq!(coder.strong[0].0, "bad");
    }

    #[test]
    fn default_targets_are_unique() {
        // A duplicate target in the bundled default would make the second copy
        // un-overridable — the merge loop only ever finds the first by position.
        let rules = build_template_rules(None);
        let mut seen = HashSet::new();
        for r in &rules {
            assert!(
                seen.insert(r.target.clone()),
                "duplicate default target: {}",
                r.target
            );
        }
    }
}
