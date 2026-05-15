//! Model-identifier normalization and per-session override application.
//!
//! Pulls the `provider/model` parsing rules, the qualified-id fallback for
//! providers that require `org/model` form (OpenRouter, Together, Fireworks,
//! Replicate, HuggingFace), the per-session `model_override` apply path
//! (#4898), and the context-window defaults / `stable_prefix_mode` flag
//! lookup out of `agent_loop/mod.rs`. None of these helpers touch the
//! agent loop state — they only mutate or query an `AgentManifest`, so
//! they belong in their own focused module.

use librefang_types::agent::{AgentManifest, STABLE_PREFIX_MODE_METADATA_KEY};
use librefang_types::error::LibreFangResult;
use tracing::warn;

/// Strip a provider prefix from a model ID before sending to the API.
///
/// Many models are stored as `provider/org/model` (e.g. `openrouter/google/gemini-2.5-flash`)
/// but the upstream API expects just `org/model` (e.g. `google/gemini-2.5-flash`).
///
/// For providers that require qualified `org/model` format (OpenRouter, Together, Fireworks,
/// Replicate, Chutes), bare model names like `gemini-2.5-flash` are normalized to their
/// fully-qualified form (e.g. `google/gemini-2.5-flash`) to prevent 400 errors.
pub fn strip_provider_prefix(model: &str, provider: &str) -> String {
    let slash_prefix = format!("{}/", provider);
    let colon_prefix = format!("{}:", provider);
    let stripped = if model.starts_with(&slash_prefix) {
        model[slash_prefix.len()..].to_string()
    } else if model.starts_with(&colon_prefix) {
        model[colon_prefix.len()..].to_string()
    } else {
        model.to_string()
    };

    // Providers that require org/model format — normalize bare model names.
    if needs_qualified_model_id(provider) && !stripped.contains('/') {
        if let Some(qualified) = normalize_bare_model_id(&stripped) {
            warn!(
                provider,
                bare_model = %stripped,
                qualified_model = %qualified,
                "Normalized bare model ID to qualified format for provider"
            );
            return qualified;
        }
        warn!(
            provider,
            model = %stripped,
            "Model ID has no org/ prefix which is required by this provider. \
             This may cause API errors. Use the format 'org/model-name' \
             (e.g. 'google/gemini-2.5-flash' for OpenRouter)."
        );
    }

    stripped
}

/// Apply a per-session model override string (#4898).
///
/// Format: `"<provider>/<model>"` (sets both provider and model — provider
/// is the first `/`-delimited segment, model is everything after it, so
/// qualified identifiers like `meta-llama/Llama-3.3-70B` are handled
/// correctly) or `"<model>"` (model only, provider stays as the manifest
/// default). Returns `Err(LibreFangError::InvalidInput)` for obviously
/// invalid inputs (empty string, missing provider or model component).
///
/// Exposed as `pub` so `kernel::agent_execution::execute_llm_agent` can
/// call it at the dispatch site (before billing/router) without duplicating
/// the logic.
pub fn apply_session_model_override_to_manifest(
    manifest: &mut AgentManifest,
    override_str: &str,
) -> LibreFangResult<()> {
    use librefang_types::error::LibreFangError;
    if override_str.is_empty() {
        return Err(LibreFangError::InvalidInput(
            "model_override must not be empty".to_string(),
        ));
    }
    // Use splitn(2, '/') so qualified model IDs like
    // `meta-llama/Llama-3.3-70B` don't get mis-split on the second `/`.
    let mut parts = override_str.splitn(2, '/');
    let first = parts.next().unwrap_or("");
    match parts.next() {
        Some(model) => {
            // provider/model form
            if first.is_empty() {
                return Err(LibreFangError::InvalidInput(
                    "model_override provider must not be empty (got '/model' form)".to_string(),
                ));
            }
            if model.is_empty() {
                return Err(LibreFangError::InvalidInput(
                    "model_override model must not be empty (got 'provider/' form)".to_string(),
                ));
            }
            manifest.model.provider = first.to_string();
            manifest.model.model = model.to_string();
        }
        None => {
            // model-only form — provider stays as manifest default
            manifest.model.model = first.to_string();
        }
    }
    Ok(())
}

/// Providers that require model IDs in `org/model` format.
pub(super) fn needs_qualified_model_id(provider: &str) -> bool {
    matches!(
        provider,
        "openrouter" | "together" | "fireworks" | "replicate" | "huggingface"
    )
}

/// Try to resolve a bare model name to a fully-qualified `org/model` identifier.
///
/// This covers common model names that users might enter without the org prefix.
/// Returns `None` if the model name is not recognized.
fn normalize_bare_model_id(bare_model: &str) -> Option<String> {
    // Normalize to lowercase for matching, preserve `:suffix` (e.g. `:free`)
    let (base, suffix) = match bare_model.split_once(':') {
        Some((b, s)) => (b, format!(":{s}")),
        None => (bare_model, String::new()),
    };
    let lower = base.to_lowercase();

    let qualified = match lower.as_str() {
        // Google models
        m if m.starts_with("gemini-") || m.starts_with("gemma-") => {
            format!("google/{base}{suffix}")
        }
        // Anthropic models
        m if m.starts_with("claude-") => format!("anthropic/{base}{suffix}"),
        // OpenAI models
        m if m.starts_with("gpt-")
            || m.starts_with("o1")
            || m.starts_with("o3")
            || m.starts_with("o4") =>
        {
            format!("openai/{base}{suffix}")
        }
        // Meta Llama models
        m if m.starts_with("llama-") => format!("meta-llama/{base}{suffix}"),
        // DeepSeek models
        m if m.starts_with("deepseek-") => format!("deepseek/{base}{suffix}"),
        // Mistral models
        m if m.starts_with("mistral-")
            || m.starts_with("mixtral-")
            || m.starts_with("codestral") =>
        {
            format!("mistralai/{base}{suffix}")
        }
        // Qwen models
        m if m.starts_with("qwen-") || m.starts_with("qwq") => {
            format!("qwen/{base}{suffix}")
        }
        // Cohere models
        m if m.starts_with("command-") => format!("cohere/{base}{suffix}"),
        // Not recognized — return None so the caller can warn
        _ => return None,
    };

    Some(qualified)
}

/// Default context window size (tokens) for token-based trimming when the
/// model is in the catalog but its `context_window` was unset. Referenced by
/// `docs/architecture/message-history-trimming.md` and
/// `docs/src/app/configuration/core/page.mdx` so kept as the authoritative
/// value even when no runtime path currently reads it.
#[allow(dead_code)]
pub(super) const DEFAULT_CONTEXT_WINDOW: usize = 200_000;

/// Conservative fallback for **unknown** models — i.e. the catalog had no
/// entry for this model name. 200K silently assumes a Claude-class window;
/// for a small open-source model that actually supports 8K, an oversized
/// prompt only fails at the provider with HTTP 400 *after* tokens are
/// already metered. 8192 is the smallest window any modern provider ships
/// (gpt-3.5, llama-2-base, …), so this errs on the side of trimming early
/// rather than burning tokens (#3349). Operators with larger windows must
/// set `agent.toml: model.context_window` (or the equivalent provider
/// catalog entry) explicitly.
pub(super) const UNKNOWN_MODEL_CONTEXT_WINDOW: usize = 8192;

/// Check if stable_prefix_mode is enabled via manifest metadata.
pub(super) fn stable_prefix_mode_enabled(manifest: &AgentManifest) -> bool {
    manifest
        .metadata
        .get(STABLE_PREFIX_MODE_METADATA_KEY)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}
#[cfg(test)]
mod session_model_override_tests {
    use super::*;
    use librefang_types::agent::ModelConfig;

    fn manifest_with(provider: &str, model: &str) -> AgentManifest {
        AgentManifest {
            model: ModelConfig {
                provider: provider.to_string(),
                model: model.to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn override_provider_and_model_when_slash_present() {
        let mut m = manifest_with("anthropic", "claude-sonnet-4-6");
        apply_session_model_override_to_manifest(&mut m, "groq/llama-3.3-70b").unwrap();
        assert_eq!(m.model.provider, "groq");
        assert_eq!(m.model.model, "llama-3.3-70b");
    }

    #[test]
    fn override_model_only_when_no_slash() {
        let mut m = manifest_with("anthropic", "claude-sonnet-4-6");
        apply_session_model_override_to_manifest(&mut m, "claude-haiku-4-5").unwrap();
        assert_eq!(
            m.model.provider, "anthropic",
            "provider must stay as manifest default when override has no slash"
        );
        assert_eq!(m.model.model, "claude-haiku-4-5");
    }

    #[test]
    fn override_preserves_other_manifest_fields() {
        let mut m = manifest_with("anthropic", "claude-sonnet-4-6");
        m.name = "agent-foo".to_string();
        m.description = "test agent".to_string();
        apply_session_model_override_to_manifest(&mut m, "groq/llama-3.3").unwrap();
        assert_eq!(m.name, "agent-foo");
        assert_eq!(m.description, "test agent");
    }

    #[test]
    fn qualified_model_id_with_multiple_slashes_uses_splitn() {
        // "meta-llama/Llama-3.3-70B" — provider is "meta-llama", model is
        // "Llama-3.3-70B". A naive split_once('/') would behave the same
        // here but splitn(2) ensures a future "org/family/variant" can't
        // inadvertently truncate the model name.
        let mut m = manifest_with("openai", "gpt-4o");
        apply_session_model_override_to_manifest(&mut m, "meta-llama/Llama-3.3-70B").unwrap();
        assert_eq!(m.model.provider, "meta-llama");
        assert_eq!(m.model.model, "Llama-3.3-70B");
    }

    #[test]
    fn empty_override_is_rejected() {
        let mut m = manifest_with("anthropic", "claude-sonnet-4-6");
        assert!(apply_session_model_override_to_manifest(&mut m, "").is_err());
    }

    #[test]
    fn slash_only_provider_form_is_rejected() {
        // "/model" → empty provider — invalid
        let mut m = manifest_with("anthropic", "claude-sonnet-4-6");
        assert!(apply_session_model_override_to_manifest(&mut m, "/llama-3.3-70b").is_err());
    }

    #[test]
    fn trailing_slash_form_is_rejected() {
        // "groq/" → empty model — invalid
        let mut m = manifest_with("anthropic", "claude-sonnet-4-6");
        assert!(apply_session_model_override_to_manifest(&mut m, "groq/").is_err());
    }
}
