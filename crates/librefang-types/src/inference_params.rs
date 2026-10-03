//! Resolution of the inference parameters that reach the wire, and the
//! save-time check that guards them against a model's declared limits.
//!
//! Two categories live here and they follow opposite rules.
//!
//! **Preferences** — `temperature`, `top_p`, `max_tokens`, `frequency_penalty`, `presence_penalty`, `top_k`, `min_p`, `repeat_penalty` — are what the operator wants this agent to sound like.
//! The specific setting beats the general one: agent manifest, then per-model override, then — for `max_tokens` alone — the model's effective ceiling (its registry maximum, or the operator's per-model correction of it), and only then the system default.
//! An unspecified output budget asks the endpoint for as much as it will give, and the registry is the only thing here that knows how much that is.
//! That ordering is the whole point of the module: two instances of one agent type must be able to run the same model at different temperatures, and before this the per-model override overwrote both of them with one value.
//!
//! **Endpoint facts** — `reasoning_effort` and the `use_max_completion_tokens` / `force_max_tokens` / `no_system_role` transport flags — are not preferences.
//! A gateway that rejects `reasoning_effort` rejects every turn that carries it (#7770), so the model level has to be able to say "never send this here" and win.
//! These resolve model-first and an agent-level `extra_params` entry cannot raise them.
//!
//! **Limits** — `context_window`, `max_output_tokens` — describe what the endpoint can accept.
//! They never clamp: [`check_output_limit`] and [`check_context_limit`] report an over-limit request to the operator and the request goes out as asked.
//! A silently truncated value leaves the operator worse off than an explicit provider error when the catalog figure is the thing that is wrong, which on a gateway-discovered model it frequently is (#7780).
//! And a limit that was never measured is not a ceiling: see [`KnownLimit`].

use crate::agent::{ModelConfig, DEFAULT_MODEL_MAX_TOKENS, DEFAULT_MODEL_TEMPERATURE};
use crate::model_catalog::{EffectiveLimits, LimitSource as CatalogLimitSource, ModelOverrides};

/// Preset context-window sizes offered by the editors, smallest first.
///
/// A ladder rather than a free slider because the useful values are an order-of-magnitude
/// sequence, not a continuum — dragging a slider to land on exactly 131072 is a chore, and the
/// numbers in between mean nothing to any provider.
/// Every editor also offers a custom entry for the value that is not on the ladder.
///
/// The dashboard mirrors this list in
/// `crates/librefang-api/dashboard/src/lib/modelParamLadders.ts`; the two are
/// small and stable, and keeping the TUI free of a round-trip to fetch them is
/// worth one duplicated constant. Change both together.
pub const CONTEXT_WINDOW_LADDER: &[u64] = &[
    8_192, 32_768, 131_072, 262_144, 524_288, 1_048_576, 2_097_152,
];

/// Preset maximum-output-token sizes offered by the editors, smallest first.
///
/// Deliberately a different ladder from [`CONTEXT_WINDOW_LADDER`], and it stops at 128K.
/// Output tokens are not context tokens: Gemini's 1M/2M figures are how much it can *read*, and no
/// model generates a million tokens of reply.
/// Offering 1M here would state that the value is valid, which is worse than a slider — it invites
/// a setting that the provider will refuse.
pub const MAX_OUTPUT_TOKENS_LADDER: &[u32] =
    &[1_024, 4_096, 8_192, 16_384, 32_768, 65_536, 131_072];

/// A model limit that some identifiable source actually asserted.
///
/// Constructing one is the assertion that the number came from an operator, the shipped registry, or a gateway that reported it — never from a placeholder.
/// [`crate::model_catalog::ModelCatalogEntry::limits_known`] is the flag that keeps the two apart: `merge_discovered_models` stamps a discovered entry with the `131_072` / `16_384` literals it has no source for and marks it unknown, so nothing downstream can mistake the placeholder for a measurement.
///
/// Code that cannot tell the difference must pass `None` rather than guess.
/// Warning an operator that 200000 exceeds a ceiling that was itself invented is noise, and noise trains people to ignore the real warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownLimit {
    /// The limit in tokens. Always non-zero — [`KnownLimit::new`] rejects `0`,
    /// which the catalog uses to mean "not applicable / absent".
    pub tokens: u64,
    /// Where the number came from. Carried so surfaces can say which.
    pub source: LimitSource,
}

impl KnownLimit {
    /// Build a known limit, or `None` when the value is absent (`0`).
    pub fn new(tokens: u64, source: LimitSource) -> Option<Self> {
        (tokens > 0).then_some(Self { tokens, source })
    }

    /// The output ceiling resolved for a model by the catalog's
    /// `ModelCatalog::effective_limits` / `effective_limits_for_manifest`, as a
    /// [`KnownLimit`].
    ///
    /// The catalog has already ranked the operator's `model_overrides.json`
    /// correction above the entry's own declared value (#7774); this maps the
    /// paired [`crate::model_catalog::LimitSource`] onto the vocabulary warnings
    /// speak, so a ceiling the operator set is not later reported back to that
    /// operator as a registry fact.
    ///
    /// `None` when neither layer carried a value — the caller applies its own
    /// fallback. `Unknown` is unreachable while `max_output_tokens` is `Some`
    /// (the catalog pairs the two in one expression) and maps to
    /// [`LimitSource::Registry`] to keep this total without an unreachable
    /// panic.
    pub fn from_effective_limits(limits: &EffectiveLimits) -> Option<Self> {
        let source = match limits.max_output_tokens_source {
            CatalogLimitSource::Override => LimitSource::Operator,
            CatalogLimitSource::Catalog | CatalogLimitSource::Unknown => LimitSource::Registry,
        };
        limits
            .max_output_tokens
            .and_then(|tokens| Self::new(tokens, source))
    }
}

/// Where a [`KnownLimit`] was asserted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitSource {
    /// Hand-set by the operator — `agent.toml: [model] context_window` / `max_output_tokens`.
    Operator,
    /// Shipped in the model registry / curated catalog entry.
    Registry,
    /// Reported by the provider or gateway during discovery.
    Gateway,
}

impl LimitSource {
    /// Stable identifier for logs and API payloads.
    pub fn as_str(self) -> &'static str {
        match self {
            LimitSource::Operator => "operator",
            LimitSource::Registry => "registry",
            LimitSource::Gateway => "gateway",
        }
    }
}

/// Which limit a [`LimitWarning`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    /// Requested `max_tokens` against the model's maximum output tokens.
    OutputTokens,
    /// Requested context window against the model's context window.
    ContextWindow,
}

impl LimitKind {
    /// Stable identifier for logs and API payloads.
    pub fn as_str(self) -> &'static str {
        match self {
            LimitKind::OutputTokens => "max_tokens",
            LimitKind::ContextWindow => "context_window",
        }
    }
}

/// A request that exceeds a limit we can actually vouch for.
///
/// Advisory by construction: producing one never changes the value that is
/// sent. Callers surface it (API response, log line, red field in the editor)
/// and dispatch the request unmodified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitWarning {
    pub kind: LimitKind,
    /// What the operator asked for.
    pub requested: u64,
    /// The limit that was exceeded.
    pub limit: u64,
    /// Where that limit came from.
    pub source: LimitSource,
}

impl LimitWarning {
    /// One-line English message for logs and the API payload.
    ///
    /// Says explicitly that the value is sent anyway, because a warning that
    /// reads like a rejection sends operators hunting for a failure that did
    /// not happen.
    pub fn message(&self) -> String {
        format!(
            "{} {} exceeds the {} limit of {} reported by the {}; sending the requested value unchanged — the provider may reject it",
            self.kind.as_str(),
            self.requested,
            self.kind.as_str(),
            self.limit,
            self.source.as_str(),
        )
    }
}

/// Warn when `requested` output tokens exceed a limit we can vouch for.
///
/// `None` when the request fits, or when `limit` is `None` — an unknown limit
/// is not a ceiling. Never clamps.
pub fn check_output_limit(requested: u32, limit: Option<KnownLimit>) -> Option<LimitWarning> {
    check(LimitKind::OutputTokens, u64::from(requested), limit)
}

/// Warn when a requested context window exceeds a limit we can vouch for.
/// See [`check_output_limit`].
pub fn check_context_limit(requested: u64, limit: Option<KnownLimit>) -> Option<LimitWarning> {
    check(LimitKind::ContextWindow, requested, limit)
}

fn check(kind: LimitKind, requested: u64, limit: Option<KnownLimit>) -> Option<LimitWarning> {
    let known = limit?;
    // `then_some` rather than `then`: every field is already computed, so there
    // is nothing to defer.
    (requested > known.tokens).then_some(LimitWarning {
        kind,
        requested,
        limit: known.tokens,
        source: known.source,
    })
}

/// The inference parameters for one turn, after the precedence chain has run.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedInferenceParams {
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub presence_penalty: Option<f32>,
    pub top_k: Option<u32>,
    pub min_p: Option<f32>,
    pub repeat_penalty: Option<f32>,
    /// Endpoint fact, not a preference — model level only. See the module docs.
    pub reasoning_effort: Option<String>,
    /// Transport flags carried straight through from the per-model override.
    pub use_max_completion_tokens: bool,
    pub force_max_tokens: bool,
}

/// Resolve the parameters for one turn from the agent manifest and the
/// per-model override that matches the final (post-routing) model.
///
/// Preferences take the agent's value first; endpoint facts take the model's.
/// Pure — call it with the override and the registry limit already looked up so
/// it stays testable without a catalog.
///
/// `known_max_output` is the matched model's *effective* output ceiling as a
/// [`KnownLimit`] — the operator's per-model `max_output_tokens` correction if
/// one exists, otherwise the registry entry's own figure (see
/// [`KnownLimit::from_effective_limits`]) — or `None` when neither vouched for
/// one. It decides `max_tokens` only when neither the agent nor the override
/// named one — an agent type that pins its own budget still wins, which is what
/// keeps two instances of one type free to differ.
pub fn resolve_inference_params(
    agent: &ModelConfig,
    model: Option<&ModelOverrides>,
    known_max_output: Option<KnownLimit>,
) -> ResolvedInferenceParams {
    ResolvedInferenceParams {
        max_tokens: agent
            .max_tokens
            .or_else(|| model.and_then(|m| m.max_tokens))
            .unwrap_or_else(|| default_max_tokens(known_max_output)),
        temperature: agent
            .temperature
            .or_else(|| model.and_then(|m| m.temperature))
            .unwrap_or(DEFAULT_MODEL_TEMPERATURE),
        top_p: agent.top_p.or_else(|| model.and_then(|m| m.top_p)),
        frequency_penalty: agent
            .frequency_penalty
            .or_else(|| model.and_then(|m| m.frequency_penalty)),
        presence_penalty: agent
            .presence_penalty
            .or_else(|| model.and_then(|m| m.presence_penalty)),
        top_k: agent.top_k.or_else(|| model.and_then(|m| m.top_k)),
        min_p: agent.min_p.or_else(|| model.and_then(|m| m.min_p)),
        repeat_penalty: agent
            .repeat_penalty
            .or_else(|| model.and_then(|m| m.repeat_penalty)),
        // Model-level only, and it wins: see the module docs on #7770.
        reasoning_effort: model.and_then(|m| m.reasoning_effort.clone()),
        use_max_completion_tokens: model
            .and_then(|m| m.use_max_completion_tokens)
            .unwrap_or(false),
        force_max_tokens: model.and_then(|m| m.force_max_tokens).unwrap_or(false),
    }
}

/// The output budget to send when neither the agent nor the per-model override
/// named one.
///
/// An unspecified `max_tokens` asks the endpoint for as much as it will give,
/// and the catalog is the only thing here that knows how much that is: its
/// `max_output_tokens` is what the provider documents as the model's ceiling,
/// so requesting it cannot overshoot by construction.
/// The fixed 4096 it replaces sat below the documented maximum of most entries,
/// which handicapped precisely the models with room to spare.
/// It is not a merely conservative default either: a reasoning model has to fit
/// its thinking *and* its reply inside the budget, so one that runs out before
/// emitting any text produces no answer at all rather than a shorter one, and
/// the loop then surfaces that absence as a placeholder reply.
///
/// A [`KnownLimit`] is used as-is: whoever looked the ceiling up is responsible
/// for it having a source, and the fallback is [`DEFAULT_MODEL_MAX_TOKENS`] when
/// there is none rather than a number nobody vouched for.
fn default_max_tokens(known: Option<KnownLimit>) -> u32 {
    known
        .and_then(|limit| u32::try_from(limit.tokens).ok())
        .unwrap_or(DEFAULT_MODEL_MAX_TOKENS)
}

impl ResolvedInferenceParams {
    /// Write the resolved values back onto the manifest the runtime reads.
    ///
    /// Every preference lands on its own typed field, and the agent loop copies those onto the typed fields of `CompletionRequest`.
    /// `top_p` / `frequency_penalty` / `presence_penalty` used to be written into `extra_params` instead (#8290), which reached the wire only on the drivers that merge that map, unfiltered on one and at the wrong nesting level on another.
    /// Any copy of those keys, or of the `top_k` / `min_p` / `repeat_penalty` added beside them, left in `extra_params` is removed, so the typed value is the only one a driver sees and the per-driver gate cannot be bypassed by a stale entry.
    /// The endpoint facts below still travel in `extra_params`.
    ///
    /// `reasoning_effort` is *removed* when the model level does not set it, not
    /// merely left alone. A stale `extra_params["reasoning_effort"]` on the
    /// manifest would otherwise survive onto a model whose endpoint rejects the
    /// parameter, which is exactly the failure #7770 fixed — the model level has
    /// to be able to say "never send this here" and have that stick.
    pub fn apply_to(&self, model: &mut ModelConfig) {
        model.max_tokens = Some(self.max_tokens);
        model.temperature = Some(self.temperature);
        model.top_p = self.top_p;
        model.frequency_penalty = self.frequency_penalty;
        model.presence_penalty = self.presence_penalty;
        model.top_k = self.top_k;
        model.min_p = self.min_p;
        model.repeat_penalty = self.repeat_penalty;
        let ep = &mut model.extra_params;
        for key in TYPED_SAMPLING_KEYS {
            ep.remove(key);
        }
        set_or_clear(
            ep,
            "reasoning_effort",
            self.reasoning_effort.as_ref().map(|v| serde_json::json!(v)),
        );
        if self.use_max_completion_tokens {
            ep.insert(
                "use_max_completion_tokens".to_string(),
                serde_json::json!(true),
            );
        }
        if self.force_max_tokens {
            ep.insert("force_max_tokens".to_string(), serde_json::json!(true));
        }
    }
}

/// The sampling preferences that travel as typed `CompletionRequest` fields and must therefore never also appear in `extra_params`.
const TYPED_SAMPLING_KEYS: [&str; 6] = [
    "top_p",
    "frequency_penalty",
    "presence_penalty",
    "top_k",
    "min_p",
    "repeat_penalty",
];

fn set_or_clear(
    map: &mut std::collections::BTreeMap<String, serde_json::Value>,
    key: &str,
    value: Option<serde_json::Value>,
) {
    match value {
        Some(v) => {
            map.insert(key.to_string(), v);
        }
        None => {
            map.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_catalog::ModelCatalogEntry;

    fn agent_with(temperature: Option<f32>, max_tokens: Option<u32>) -> ModelConfig {
        ModelConfig {
            temperature,
            max_tokens,
            ..Default::default()
        }
    }

    /// The bug this module exists for: the writer instance that set its own
    /// temperature must keep it when someone tunes the shared model.
    #[test]
    fn agent_preference_beats_model_override() {
        let agent = agent_with(Some(0.2), Some(8192));
        let model = ModelOverrides {
            temperature: Some(1.4),
            max_tokens: Some(1024),
            ..Default::default()
        };
        let r = resolve_inference_params(&agent, Some(&model), None);
        assert!((r.temperature - 0.2).abs() < f32::EPSILON);
        assert_eq!(r.max_tokens, 8192);
    }

    /// Two instances of one agent type, one model, two temperatures — the
    /// case the user reported.
    #[test]
    fn two_agents_on_one_model_keep_distinct_temperatures() {
        let model = ModelOverrides {
            temperature: Some(0.7),
            ..Default::default()
        };
        let creative = resolve_inference_params(&agent_with(Some(1.2), None), Some(&model), None);
        let academic = resolve_inference_params(&agent_with(Some(0.1), None), Some(&model), None);
        assert!((creative.temperature - 1.2).abs() < f32::EPSILON);
        assert!((academic.temperature - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn inheriting_agent_takes_the_model_override() {
        let model = ModelOverrides {
            temperature: Some(1.4),
            max_tokens: Some(1024),
            top_p: Some(0.5),
            frequency_penalty: Some(0.25),
            presence_penalty: Some(-0.5),
            ..Default::default()
        };
        let r = resolve_inference_params(&agent_with(None, None), Some(&model), None);
        assert!((r.temperature - 1.4).abs() < f32::EPSILON);
        assert_eq!(r.max_tokens, 1024);
        assert_eq!(r.top_p, Some(0.5));
        assert_eq!(r.frequency_penalty, Some(0.25));
        assert_eq!(r.presence_penalty, Some(-0.5));
    }

    #[test]
    fn nothing_set_anywhere_falls_back_to_system_defaults() {
        let r = resolve_inference_params(&agent_with(None, None), None, None);
        assert_eq!(r.max_tokens, DEFAULT_MODEL_MAX_TOKENS);
        assert!((r.temperature - DEFAULT_MODEL_TEMPERATURE).abs() < f32::EPSILON);
        assert_eq!(r.top_p, None);
    }

    /// An unset output budget asks the endpoint for as much as the endpoint says
    /// it can give, which is the rung this module gained.
    ///
    /// 4096 was smaller than the documented maximum of most registry entries, so
    /// the old default handicapped exactly the models with room to spare — and for
    /// a reasoning model, which must fit its thinking *and* its reply inside the
    /// budget, running out before any text is emitted yields no answer at all
    /// rather than a shorter one.
    #[test]
    fn unset_max_tokens_takes_the_registry_ceiling() {
        let limit = KnownLimit::new(65_536, LimitSource::Registry);
        let r = resolve_inference_params(&agent_with(None, None), None, limit);
        assert_eq!(r.max_tokens, 65_536);
    }

    /// A registry entry's declared ceiling, walked through the entry-level
    /// hand-off that the warning surfaces use rather than from a hand-built
    /// `KnownLimit`: with no override anywhere, a registry-declared
    /// `max_output_tokens` answers an unset budget.
    ///
    /// The per-turn rung reads the *effective* limits instead, so an operator
    /// override can correct the entry (see [`KnownLimit::from_effective_limits`]);
    /// this pins the entry-side contract — the number comes from
    /// [`crate::model_catalog::ModelCatalogEntry::known_max_output_tokens`], and
    /// `limits_known` keeps a discovery placeholder from supplying one — that
    /// the warning path still relies on.
    #[test]
    fn catalog_declared_ceiling_answers_an_unset_budget() {
        let entry = ModelCatalogEntry {
            id: "known-model".into(),
            provider: "ollama".into(),
            context_window: 200_000,
            max_output_tokens: 65_536,
            limits_known: true,
            ..Default::default()
        };
        let r = resolve_inference_params(
            &agent_with(None, None),
            None,
            entry.known_max_output_tokens(),
        );
        assert_eq!(r.max_tokens, 65_536);

        // The same entry as a gateway discovery leaves it: the placeholders are
        // not a measurement, so the system default stands in rather than a
        // number nobody vouched for.
        let placeholder = ModelCatalogEntry {
            limits_known: false,
            ..entry
        };
        let r = resolve_inference_params(
            &agent_with(None, None),
            None,
            placeholder.known_max_output_tokens(),
        );
        assert_eq!(r.max_tokens, DEFAULT_MODEL_MAX_TOKENS);
    }

    /// An agent type that pinned its own budget is making a decision, not
    /// forgetting one, so it keeps it — the ceiling only fills a silence.
    #[test]
    fn agent_pinned_budget_beats_the_registry_ceiling() {
        let limit = KnownLimit::new(65_536, LimitSource::Registry);
        let r = resolve_inference_params(&agent_with(None, Some(4_096)), None, limit);
        assert_eq!(r.max_tokens, 4_096);
    }

    /// Same for the per-model override: it is a decision the operator made about
    /// this endpoint, which is more specific than the registry's figure.
    #[test]
    fn model_override_budget_beats_the_registry_ceiling() {
        let limit = KnownLimit::new(65_536, LimitSource::Registry);
        let model = ModelOverrides {
            max_tokens: Some(1_024),
            ..Default::default()
        };
        let r = resolve_inference_params(&agent_with(None, None), Some(&model), limit);
        assert_eq!(r.max_tokens, 1_024);
    }

    /// The source travels with the value. The catalog ranks the operator's
    /// override above the entry's own figure before this rung sees it (#7774),
    /// so the conversion has to map the winner's provenance onto the warning
    /// vocabulary: an operator-corrected ceiling reported as a registry fact
    /// would misattribute the number back to the layer it corrected.
    #[test]
    fn an_effective_ceiling_keeps_the_layer_that_asserted_it() {
        let overridden = EffectiveLimits {
            max_output_tokens: Some(8_192),
            max_output_tokens_source: CatalogLimitSource::Override,
            ..Default::default()
        };
        assert_eq!(
            KnownLimit::from_effective_limits(&overridden),
            KnownLimit::new(8_192, LimitSource::Operator)
        );

        let registry = EffectiveLimits {
            max_output_tokens: Some(65_536),
            max_output_tokens_source: CatalogLimitSource::Catalog,
            ..Default::default()
        };
        assert_eq!(
            KnownLimit::from_effective_limits(&registry),
            KnownLimit::new(65_536, LimitSource::Registry)
        );

        // No layer asserted a ceiling: no limit, and nothing to attribute.
        assert_eq!(
            KnownLimit::from_effective_limits(&EffectiveLimits::default()),
            None
        );
    }

    /// The precedence this rung promises, in the only place it can be walked
    /// without a catalog: the effective ceiling is what the operator said, so
    /// an unset budget takes their correction rather than the entry's figure.
    #[test]
    fn unset_budget_takes_the_operator_corrected_ceiling() {
        let limits = EffectiveLimits {
            max_output_tokens: Some(8_192),
            max_output_tokens_source: CatalogLimitSource::Override,
            ..Default::default()
        };
        let r = resolve_inference_params(
            &agent_with(None, None),
            None,
            KnownLimit::from_effective_limits(&limits),
        );
        assert_eq!(
            r.max_tokens, 8_192,
            "the correction, not the catalog figure"
        );
    }

    /// Pins the figure itself, because the value is the fix: a reasoning model
    /// needs room for its thinking before it writes anything, and dropping this
    /// back under 32K would reintroduce the empty reply this guards. A
    /// compile-time assertion, so it cannot be folded into a runtime `assert!`
    /// that clippy rightly rejects as an assertion on a constant.
    const _: () = assert!(DEFAULT_MODEL_MAX_TOKENS >= 32_768);

    /// The counterexample (#7770): a gateway that rejects `reasoning_effort`
    /// must be able to keep it off the wire, so an agent cannot force it on.
    #[test]
    fn reasoning_effort_stays_model_level() {
        let mut agent = agent_with(None, None);
        agent
            .extra_params
            .insert("reasoning_effort".into(), serde_json::json!("high"));

        // Model sets it → model wins.
        let model = ModelOverrides {
            reasoning_effort: Some("low".into()),
            ..Default::default()
        };
        let r = resolve_inference_params(&agent, Some(&model), None);
        assert_eq!(r.reasoning_effort.as_deref(), Some("low"));
        let mut applied = agent.clone();
        r.apply_to(&mut applied);
        assert_eq!(
            applied.extra_params.get("reasoning_effort"),
            Some(&serde_json::json!("low"))
        );

        // Model leaves it unset → the parameter is dropped, not inherited from
        // the agent. An endpoint that rejects it never sees it.
        let r = resolve_inference_params(&agent, Some(&ModelOverrides::default()), None);
        assert_eq!(r.reasoning_effort, None);
        let mut applied = agent.clone();
        r.apply_to(&mut applied);
        assert!(!applied.extra_params.contains_key("reasoning_effort"));
    }

    #[test]
    fn apply_to_writes_preferences_onto_the_manifest() {
        let agent = ModelConfig {
            temperature: Some(0.2),
            top_p: Some(0.8),
            ..Default::default()
        };
        let mut applied = agent.clone();
        resolve_inference_params(&agent, None, None).apply_to(&mut applied);
        assert_eq!(applied.temperature, Some(0.2));
        assert_eq!(applied.max_tokens, Some(DEFAULT_MODEL_MAX_TOKENS));
        assert_eq!(applied.top_p, Some(0.8));
        assert_eq!(applied.presence_penalty, None);
    }

    /// #8290: the three sampling preferences travel on typed fields, never through `extra_params`.
    /// That map reached the wire only on the drivers that merge it, so a value placed there was silently dropped on Anthropic and Gemini and sent unfiltered to every OpenAI-compatible endpoint.
    #[test]
    fn apply_to_keeps_sampling_preferences_out_of_extra_params() {
        let agent = ModelConfig {
            top_p: Some(0.8),
            frequency_penalty: Some(0.25),
            ..Default::default()
        };
        let model = ModelOverrides {
            presence_penalty: Some(-0.5),
            ..Default::default()
        };
        let mut applied = agent.clone();
        // A stale copy from before the typed fields existed must not survive to override the gate.
        applied
            .extra_params
            .insert("top_p".into(), serde_json::json!(0.1));
        applied
            .extra_params
            .insert("enable_memory".into(), serde_json::json!(true));
        resolve_inference_params(&agent, Some(&model), None).apply_to(&mut applied);

        assert_eq!(applied.top_p, Some(0.8));
        assert_eq!(applied.frequency_penalty, Some(0.25));
        assert_eq!(applied.presence_penalty, Some(-0.5));
        for key in TYPED_SAMPLING_KEYS {
            assert!(
                !applied.extra_params.contains_key(key),
                "{key} must not be duplicated into extra_params"
            );
        }
        // The escape hatch keeps carrying everything else.
        assert_eq!(
            applied.extra_params.get("enable_memory"),
            Some(&serde_json::json!(true))
        );
    }

    /// #8290 part 2: `top_k` / `min_p` / `repeat_penalty` follow the same precedence as the other preferences — agent first, then the per-model override — and land on the typed fields, not in `extra_params`.
    #[test]
    fn local_model_samplers_resolve_agent_first_and_stay_typed() {
        let agent = ModelConfig {
            top_k: Some(40),
            ..Default::default()
        };
        let model = ModelOverrides {
            top_k: Some(20),
            min_p: Some(0.05),
            repeat_penalty: Some(1.1),
            ..Default::default()
        };
        let r = resolve_inference_params(&agent, Some(&model), None);
        assert_eq!(r.top_k, Some(40), "the agent's own value wins");
        assert_eq!(r.min_p, Some(0.05), "inherited from the model override");
        assert_eq!(r.repeat_penalty, Some(1.1));

        let mut applied = agent.clone();
        // Values an operator put in the escape hatch before these fields existed must not reach a driver past its gate.
        for key in ["top_k", "min_p", "repeat_penalty"] {
            applied
                .extra_params
                .insert(key.into(), serde_json::json!(1));
        }
        r.apply_to(&mut applied);
        assert_eq!(applied.top_k, Some(40));
        assert_eq!(applied.min_p, Some(0.05));
        assert_eq!(applied.repeat_penalty, Some(1.1));
        assert!(
            applied.extra_params.is_empty(),
            "{:?}",
            applied.extra_params
        );

        // Nothing set anywhere: nothing is sent.
        let r = resolve_inference_params(&ModelConfig::default(), None, None);
        assert_eq!((r.top_k, r.min_p, r.repeat_penalty), (None, None, None));
    }

    #[test]
    fn known_limit_warns_but_never_clamps() {
        let limit = KnownLimit::new(16_384, LimitSource::Registry);
        let w = check_output_limit(65_536, limit).expect("over a known limit must warn");
        assert_eq!(w.kind, LimitKind::OutputTokens);
        assert_eq!(w.requested, 65_536);
        assert_eq!(w.limit, 16_384);
        assert_eq!(w.source, LimitSource::Registry);
        assert!(w.message().contains("unchanged"));

        // The check is advisory: it returns a warning, never a corrected value.
        assert!(check_output_limit(16_384, limit).is_none());
        assert!(check_output_limit(1, limit).is_none());
    }

    /// An inferred ceiling is a guess. Warning against a guess is noise, and
    /// noise is what makes operators stop reading warnings.
    #[test]
    fn inferred_limit_never_warns() {
        assert!(check_output_limit(999_999, None).is_none());
        assert!(check_context_limit(4_000_000, None).is_none());
        // `0` in the catalog means "absent", so it cannot produce a limit.
        assert!(KnownLimit::new(0, LimitSource::Registry).is_none());
    }

    /// The two ladders are not interchangeable. Output tokens are what a model
    /// generates; context tokens are what it reads. Gemini's 1M/2M are context
    /// figures, and putting them on the output ladder would advertise a setting
    /// no provider will honour.
    #[test]
    fn the_output_ladder_stops_well_below_the_context_ladder() {
        assert!(CONTEXT_WINDOW_LADDER.windows(2).all(|w| w[0] < w[1]));
        assert!(MAX_OUTPUT_TOKENS_LADDER.windows(2).all(|w| w[0] < w[1]));
        let top_output = u64::from(*MAX_OUTPUT_TOKENS_LADDER.last().unwrap());
        let top_context = *CONTEXT_WINDOW_LADDER.last().unwrap();
        assert!(
            top_output < top_context,
            "an output ladder reaching the context ladder's top would offer impossible values"
        );
        assert_eq!(top_context, 2_097_152);
        assert_eq!(top_output, 131_072);
    }

    #[test]
    fn context_limit_warns_independently_of_output_limit() {
        let w = check_context_limit(300_000, KnownLimit::new(200_000, LimitSource::Operator))
            .expect("over a known context window must warn");
        assert_eq!(w.kind, LimitKind::ContextWindow);
        assert_eq!(w.source, LimitSource::Operator);
    }
}
