//! Which Messages API request schema a given model id accepts.
//!
//! The request body is not stable across model generations, and the changes are removals rather than additions — a field a model no longer takes is answered with `400 invalid_request_error`, not ignored.
//! `temperature` / `top_p` / `top_k` were removed on Opus 4.7 and on everything released after it, and the `thinking: {"type": "enabled", "budget_tokens": N}` opt-in was removed alongside them in favour of `thinking: {"type": "adaptive"}` with the depth carried in `output_config.effort`.
//! Opus 4.6 and Sonnet 4.6 sit between the two: they still accept the sampling parameters and they already accept the adaptive form.
//!
//! Every driver call site that has to make one of those choices asks this module instead of matching the model string itself, so the id list lives in one place and a new model is one table edit.
//! An unrecognised **Claude** id resolves to the newest contract, because the two failure directions are not symmetric there: sending a removed parameter is a hard 400 that kills the turn, while omitting one the model would have accepted merely falls back to the model's own default.
//!
//! That asymmetry inverts for an id that is not a Claude id at all, and this driver serves several of those.
//! `ApiFormat::Anthropic` also backs the Anthropic-compatible third-party endpoints in `drivers/mod.rs` — `kimi_coding` (`kimi-for-coding`, `kimi-k2.5`) and `byteplus_coding` (`ark-code-latest`, `bytedance-seed-code`, …) — which implement the classic Messages API and have gone nowhere near Anthropic's removals.
//! Those ids therefore resolve to [`AnthropicWireGeneration::Budgeted`], which is byte-for-byte the request shape they were served before this table existed.

/// The generation of the Messages API request schema a model behind this driver speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnthropicWireGeneration {
    /// Haiku 4.5 and every Claude model released before it, plus every non-Claude id on an Anthropic-compatible endpoint.
    /// Sampling parameters are accepted, and `thinking: {"type": "enabled", "budget_tokens": N}` is the only way to ask for extended thinking.
    Budgeted,
    /// Opus 4.6 and Sonnet 4.6 — the transitional pair.
    /// Sampling parameters are still accepted and the adaptive thinking form already works, so the driver sends the current form on a wire that also tolerates the deprecated one.
    AdaptiveWithSampling,
    /// Opus 4.7 and everything after it, plus every unrecognised `claude-*` id.
    /// Sampling parameters and the budgeted thinking form were both removed and answer 400.
    Adaptive,
}

impl AnthropicWireGeneration {
    /// Whether `temperature` / `top_p` / `top_k` are still part of this model's request schema.
    pub(crate) fn accepts_sampling_params(self) -> bool {
        matches!(self, Self::Budgeted | Self::AdaptiveWithSampling)
    }

    /// Whether an extended-thinking opt-in has to be spelled `{"type": "enabled", "budget_tokens": N}` rather than `{"type": "adaptive"}` plus `output_config.effort`.
    pub(crate) fn uses_budgeted_thinking(self) -> bool {
        matches!(self, Self::Budgeted)
    }
}

/// Claude model-id prefixes whose request schema is not the current one.
///
/// Only Claude ids that predate a removal need an entry; any absent `claude-*` id — including every model released after this table was last touched — resolves to [`AnthropicWireGeneration::Adaptive`].
/// The 4.7 and 4.8 entries carry that same default explicitly, because without them the `claude-opus-4` line would claim them as legacy.
/// Matching takes the longest prefix that fits rather than the first, so the table stays correct however its entries are ordered.
const WIRE_GENERATIONS: &[(&str, AnthropicWireGeneration)] = &[
    ("claude-opus-4-8", AnthropicWireGeneration::Adaptive),
    ("claude-opus-4-7", AnthropicWireGeneration::Adaptive),
    (
        "claude-opus-4-6",
        AnthropicWireGeneration::AdaptiveWithSampling,
    ),
    (
        "claude-sonnet-4-6",
        AnthropicWireGeneration::AdaptiveWithSampling,
    ),
    ("claude-opus-4", AnthropicWireGeneration::Budgeted),
    ("claude-sonnet-4", AnthropicWireGeneration::Budgeted),
    ("claude-haiku-4-5", AnthropicWireGeneration::Budgeted),
    ("claude-3-7-sonnet", AnthropicWireGeneration::Budgeted),
    ("claude-3-5-sonnet", AnthropicWireGeneration::Budgeted),
    ("claude-3-5-haiku", AnthropicWireGeneration::Budgeted),
    ("claude-3-opus", AnthropicWireGeneration::Budgeted),
    ("claude-3-sonnet", AnthropicWireGeneration::Budgeted),
    ("claude-3-haiku", AnthropicWireGeneration::Budgeted),
    ("claude-2", AnthropicWireGeneration::Budgeted),
    ("claude-instant", AnthropicWireGeneration::Budgeted),
];

/// Model-id prefixes that both reason unless the request explicitly says not to **and** accept `thinking: {"type": "disabled"}` as the way of saying it.
///
/// Both halves have to hold for the explicit off-switch to be safe, which is why this is a list rather than a property of [`AnthropicWireGeneration`].
/// Opus 5 and Sonnet 5 satisfy both.
/// Fable 5 / 5.1 and the Mythos pair reason unasked too, but answer 400 to that spelling — thinking is always on for them and no request can turn it off — so they are deliberately absent and get no `thinking` field at all.
/// Opus 4.7 and 4.8 are the mirror image: they accept the spelling but do not reason unless asked, so sending it would only add a field that changes nothing.
///
/// Opus 5's acceptance is scoped to `output_config.effort` values of `high` and below, and a request that disables thinking sends no `output_config` at all, so the default effort of `high` applies and the field is accepted.
const REASONS_WITHOUT_BEING_ASKED: &[&str] = &["claude-opus-5", "claude-sonnet-5"];

/// Reduce a configured model id to the part the tables match against, or `None` when it is not a Claude id at all.
///
/// Ids reach the driver in several shapes — bare (`claude-opus-4-6`), snapshot-suffixed (`claude-haiku-4-5-20251001`) and gateway-prefixed (`anthropic/claude-opus-5`) — and only the `claude-…` portion carries the generation.
fn canonical_id(model: &str) -> Option<String> {
    let lower = model.to_ascii_lowercase();
    lower.find("claude-").map(|at| lower[at..].to_string())
}

/// The request schema the model behind `model` accepts.
pub(crate) fn wire_generation(model: &str) -> AnthropicWireGeneration {
    // Not a Claude id, so none of Anthropic's removals apply to it.
    // These are the third-party endpoints that speak the Messages API, and the shape they have always been served is the classic one.
    let Some(id) = canonical_id(model) else {
        return AnthropicWireGeneration::Budgeted;
    };
    WIRE_GENERATIONS
        .iter()
        .filter(|(prefix, _)| id.starts_with(prefix))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, generation)| *generation)
        .unwrap_or(AnthropicWireGeneration::Adaptive)
}

/// Whether this model reasons when the request carries no `thinking` field at all *and* accepts the explicit off-switch, so that a turn asking not to reason can say so on the wire rather than by omission.
pub(crate) fn reasons_without_being_asked(model: &str) -> bool {
    canonical_id(model).is_some_and(|id| {
        REASONS_WITHOUT_BEING_ASKED
            .iter()
            .any(|prefix| id.starts_with(prefix))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The models that answer 400 to `temperature` must not be offered one, and they must be asked for thinking in the adaptive form.
    #[test]
    fn models_that_removed_sampling_parameters_take_the_adaptive_contract() {
        for model in [
            "claude-opus-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-sonnet-5",
            "claude-fable-5",
            "claude-fable-5-1",
            "claude-mythos-5",
            "claude-mythos-5-1",
        ] {
            let generation = wire_generation(model);
            assert_eq!(
                generation,
                AnthropicWireGeneration::Adaptive,
                "{model} removed the sampling parameters and the budgeted thinking form",
            );
            assert!(!generation.accepts_sampling_params(), "{model}");
            assert!(!generation.uses_budgeted_thinking(), "{model}");
        }
    }

    /// The 4.6 pair kept the sampling parameters while gaining the adaptive thinking form, so it is the one generation where both hold.
    #[test]
    fn the_transitional_pair_keeps_sampling_and_takes_adaptive_thinking() {
        for model in ["claude-opus-4-6", "claude-sonnet-4-6"] {
            let generation = wire_generation(model);
            assert_eq!(
                generation,
                AnthropicWireGeneration::AdaptiveWithSampling,
                "{model}",
            );
            assert!(generation.accepts_sampling_params(), "{model}");
            assert!(!generation.uses_budgeted_thinking(), "{model}");
        }
    }

    /// Haiku 4.5 and older still require the budgeted form — sending them `{"type": "adaptive"}` would break thinking on the models where it currently works.
    #[test]
    fn haiku_4_5_and_older_keep_the_budgeted_contract() {
        for model in [
            "claude-haiku-4-5",
            "claude-haiku-4-5-20251001",
            "claude-sonnet-4-5-20250929",
            "claude-opus-4-1-20250805",
            "claude-3-7-sonnet-20250219",
            "claude-3-5-haiku-20241022",
            "claude-3-opus-20240229",
        ] {
            let generation = wire_generation(model);
            assert_eq!(generation, AnthropicWireGeneration::Budgeted, "{model}");
            assert!(generation.accepts_sampling_params(), "{model}");
            assert!(generation.uses_budgeted_thinking(), "{model}");
        }
    }

    /// A Claude model released after this table was written must degrade towards the newest contract, not towards the legacy one — omitting an accepted parameter costs a default, sending a removed one costs the turn.
    #[test]
    fn unrecognised_claude_ids_degrade_to_the_current_contract() {
        for model in ["claude-opus-6", "claude-something-new"] {
            assert_eq!(
                wire_generation(model),
                AnthropicWireGeneration::Adaptive,
                "{model:?} is an unrecognised Claude id and must not be sent removed parameters",
            );
        }
    }

    /// `ApiFormat::Anthropic` also backs `kimi_coding` and `byteplus_coding`, whose models are not Claude models and never lost a parameter.
    /// Degrading them towards the newest Claude contract would strip a `temperature` they honour and hand them an adaptive thinking form they do not implement, so a non-Claude id keeps the classic schema.
    #[test]
    fn non_claude_ids_on_this_driver_keep_the_classic_schema() {
        for model in [
            "kimi-for-coding",
            "kimi-k2.5",
            "ark-code-latest",
            "bytedance-seed-code",
            "glm-5.1",
            "",
        ] {
            let generation = wire_generation(model);
            assert_eq!(
                generation,
                AnthropicWireGeneration::Budgeted,
                "{model:?} is not a Claude id and none of Anthropic's removals apply to it",
            );
            assert!(generation.accepts_sampling_params(), "{model:?}");
            assert!(generation.uses_budgeted_thinking(), "{model:?}");
        }
    }

    /// `claude-opus-4` is a prefix of `claude-opus-4-6`, so first-match ordering would misclassify the newer id; the lookup takes the longest prefix instead.
    #[test]
    fn longest_prefix_wins_over_the_family_prefix() {
        assert_eq!(
            wire_generation("claude-opus-4-6"),
            AnthropicWireGeneration::AdaptiveWithSampling,
        );
        assert_eq!(
            wire_generation("claude-opus-4-7"),
            AnthropicWireGeneration::Adaptive,
        );
        assert_eq!(
            wire_generation("claude-opus-4-20250514"),
            AnthropicWireGeneration::Budgeted,
        );
    }

    /// Gateway prefixes and upper-case spellings must not knock an id off its own row.
    #[test]
    fn gateway_prefixes_and_case_do_not_change_the_contract() {
        for model in [
            "anthropic/claude-opus-4-6",
            "anthropic:claude-opus-4-6",
            "CLAUDE-OPUS-4-6",
        ] {
            assert_eq!(
                wire_generation(model),
                AnthropicWireGeneration::AdaptiveWithSampling,
                "{model}",
            );
        }
    }

    /// The explicit off-switch goes only to the ids that both reason unasked and accept that spelling.
    ///
    /// Opus 5 and Sonnet 5 qualify.
    /// Fable / Mythos reason unasked but answer 400 to `{"type": "disabled"}`; Opus 4.7 / 4.8 accept the spelling but do not reason unasked; everything older does neither.
    #[test]
    fn only_the_models_that_reason_unasked_and_accept_the_switch_are_told_to_stop() {
        for model in [
            "claude-opus-5",
            "claude-sonnet-5",
            "anthropic/claude-opus-5",
            "claude-sonnet-5-20260101",
        ] {
            assert!(
                reasons_without_being_asked(model),
                "{model} reasons on an omitted thinking field, so an agent asking not to reason has to be heard",
            );
        }
        for model in [
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-fable-5",
            "claude-fable-5-1",
            "claude-mythos-5-1",
            "claude-haiku-4-5",
            "claude-opus-6",
            "kimi-for-coding",
        ] {
            assert!(
                !reasons_without_being_asked(model),
                "{model} must be left alone rather than sent a possibly-rejected disable",
            );
        }
    }
}
