//! Web search augmentation — pre-LLM context injection for models that lack
//! native tool support.
//!
//! For agents whose `web_search_augmentation` mode is `Always` (or `Auto`
//! and the model catalog reports `supports_tools = false`), the loop asks
//! a side-channel LLM call to extract 1-3 search queries from the recent
//! conversation, runs each against the configured `WebToolsContext`, and
//! returns the concatenated results for the main loop to splice into the
//! system prompt.

use crate::llm_driver::{CompletionRequest, LlmDriver};
use crate::web_search::WebToolsContext;
use librefang_types::agent::AgentManifest;
use librefang_types::config::ResponseFormat;
use librefang_types::message::{Message, Role};
use tracing::{debug, warn};

use super::strip_provider_prefix;
use super::text_recovery::find_json_object_end;

const MAX_SEARCH_QUERIES: usize = 3;
const MAX_SEARCH_RESULT_CHARS_PER_QUERY: usize = 4_000;
const MAX_WEB_SEARCH_AUGMENTATION_CHARS: usize = 10_000;
const SEARCH_RESULTS_TRUNCATION_MARKER: &str = "\n[Search results truncated]\n";

fn external_content_closing_boundary(content: &str) -> Option<&str> {
    let boundary = content.rsplit_once('\n')?.1;
    (boundary.starts_with("<<</EXTCONTENT_") && boundary.ends_with(">>>")).then_some(boundary)
}

fn truncate_search_results(content: &str, max_chars: usize) -> String {
    let total_chars = content.chars().count();
    if total_chars <= max_chars {
        return content.to_string();
    }

    let marker_chars = SEARCH_RESULTS_TRUNCATION_MARKER.chars().count();
    let closing_boundary = external_content_closing_boundary(content);
    let closing_chars = closing_boundary.map_or(0, |boundary| boundary.chars().count());
    let reserved_chars = marker_chars.saturating_add(closing_chars);

    if reserved_chars >= max_chars {
        return content.chars().take(max_chars).collect();
    }

    let prefix: String = content.chars().take(max_chars - reserved_chars).collect();
    match closing_boundary {
        Some(boundary) => format!("{prefix}{SEARCH_RESULTS_TRUNCATION_MARKER}{boundary}"),
        None => format!("{prefix}{SEARCH_RESULTS_TRUNCATION_MARKER}"),
    }
}

fn append_search_results(output: &mut String, results: &str) -> bool {
    let output_chars = output.chars().count();
    let separator_chars = usize::from(!output.is_empty());
    let Some(remaining_chars) =
        MAX_WEB_SEARCH_AUGMENTATION_CHARS.checked_sub(output_chars.saturating_add(separator_chars))
    else {
        return false;
    };
    if remaining_chars == 0 {
        return false;
    }

    if separator_chars != 0 {
        output.push('\n');
    }
    let query_limit = remaining_chars.min(MAX_SEARCH_RESULT_CHARS_PER_QUERY);
    output.push_str(&truncate_search_results(results, query_limit));
    true
}

/// Check if web search augmentation should be performed for this agent.
pub(super) fn should_augment_web_search(manifest: &AgentManifest) -> bool {
    use librefang_types::agent::WebSearchAugmentationMode;
    match manifest.web_search_augmentation {
        WebSearchAugmentationMode::Off => false,
        WebSearchAugmentationMode::Always => true,
        WebSearchAugmentationMode::Auto => {
            // Auto: augment when model catalog says supports_tools == false.
            // If model is not in catalog (None), assume tools are supported (conservative).
            let supports = manifest
                .metadata
                .get("model_supports_tools")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            !supports
        }
    }
}

/// System prompt for LLM-based search query generation.
/// Designed to work with small local models (Gemma, Llama, Qwen, etc.).
pub(super) const SEARCH_QUERY_GEN_PROMPT: &str = r#"You are a search query generator. Analyze the conversation and generate 1-3 concise, diverse web search queries that would help answer the user's latest message.

Rules:
- Respond ONLY with a JSON object: {"queries": ["query1", "query2"]}
- Each query should be concise (3-8 words) and search-engine-friendly
- Generate queries in the same language as the user's message
- If the question is purely conversational (greetings, thanks, etc.), return: {"queries": []}
- Prioritize queries that retrieve factual, up-to-date information
- Today's date: "#;

/// Use the LLM to generate focused search queries from the conversation history.
/// Falls back to `None` on any failure (caller uses raw user message instead).
async fn generate_search_queries(
    driver: &dyn LlmDriver,
    manifest: &AgentManifest,
    session_messages: &[Message],
    user_message: &str,
    reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy,
) -> Option<Vec<String>> {
    // Build a compact conversation summary from the last few messages
    let recent: Vec<&Message> = session_messages
        .iter()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let mut history = String::new();
    for msg in &recent {
        let role = match msg.role {
            Role::System => continue,
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        let text = msg.content.text_content();
        if !text.is_empty() {
            history.push_str(&format!("{role}: {text}\n"));
        }
    }

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let system = format!("{SEARCH_QUERY_GEN_PROMPT}{today}");

    let request = CompletionRequest {
        model: strip_provider_prefix(&manifest.model.model, &manifest.model.provider),
        messages: std::sync::Arc::new(vec![Message::user(format!(
            "{history}\nUser: {user_message}"
        ))]),
        tools: std::sync::Arc::new(vec![]),
        max_tokens: 200,
        temperature: 0.0,
        system: Some(system),
        thinking: None,
        prompt_caching: false,
        cache_ttl: None,
        prompt_cache_strategy: None,
        // Request structured JSON output — `SEARCH_QUERY_GEN_PROMPT`
        // explicitly tells the LLM "Respond ONLY with a JSON object:
        // {"queries": [...]}", so the same `response_format` flag
        // history_fold needs (#5287) applies here. Without it,
        // DeepSeek / OpenAI / Mistral / Gemini are free to emit
        // free-form prose that `parse_search_queries` will reject,
        // causing `generate_search_queries` to return None and the
        // augment path to fall back to the raw user message (the
        // existing failure mode is silent, just degraded relevance).
        // The prompt already contains the word "JSON" (DeepSeek's
        // requirement for json_object mode). Providers that don't
        // honour the flag ignore it without error.
        response_format: Some(ResponseFormat::Json),
        timeout_secs: Some(15),
        extra_body: None,
        agent_id: None,
        session_id: None,
        step_id: None,
        reasoning_echo_policy,

        ..Default::default()
    };

    let response =
        match tokio::time::timeout(std::time::Duration::from_secs(15), driver.complete(request))
            .await
        {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                debug!("Search query generation LLM error: {e}");
                return None;
            }
            Err(_) => {
                debug!("Search query generation timed out");
                return None;
            }
        };

    let text = response.text();
    let max_attempts = 20;
    let mut scan = 0;
    let mut attempt = 0;
    let parsed: serde_json::Value = loop {
        if scan >= text.len() || attempt >= max_attempts {
            return None;
        }
        let start = match text[scan..].find('{') {
            Some(i) => scan + i,
            None => return None,
        };
        let Some(end) = find_json_object_end(&text[start..]).map(|end| start + end) else {
            scan = start + 1;
            attempt += 1;
            continue;
        };
        let candidate = &text[start..end];
        match serde_json::from_str::<serde_json::Value>(candidate) {
            Ok(value) => break value,
            Err(_) => {
                scan = start + 1;
                attempt += 1;
                continue;
            }
        }
    };
    let queries: Vec<String> = parsed["queries"]
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .take(MAX_SEARCH_QUERIES)
        .collect();

    if queries.is_empty() {
        debug!("LLM determined no search needed for this message");
        // Return empty vec to signal "no search needed" (distinct from None = "generation failed")
        Some(Vec::new())
    } else {
        debug!(
            count = queries.len(),
            "Generated search queries: {:?}", queries
        );
        Some(queries)
    }
}

/// Perform web search augmentation — optionally generate queries via LLM,
/// search the web, and return formatted results for context injection.
pub(super) async fn web_search_augment(
    manifest: &AgentManifest,
    user_message: &str,
    web_ctx: Option<&WebToolsContext>,
    driver: &dyn LlmDriver,
    session_messages: &[Message],
    reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy,
) -> Option<String> {
    if !should_augment_web_search(manifest) {
        return None;
    }
    let ctx = web_ctx?;

    // Try LLM-based query generation.
    // Some(vec![...]) = generated queries, Some(vec![]) = no search needed, None = generation failed
    let queries = match generate_search_queries(
        driver,
        manifest,
        session_messages,
        user_message,
        reasoning_echo_policy,
    )
    .await
    {
        Some(q) if q.is_empty() => return None, // LLM says no search needed
        Some(q) => q,
        None => {
            // Query-generation LLM failed — non-JSON response, network
            // error, or the response_format=Json pin we ship in this
            // module is ignored by the provider. Falling back to a single
            // verbatim-user-message search keeps the feature working but
            // is observably worse than a well-formed multi-query expansion;
            // surface it so operators can spot a degraded provider.
            tracing::debug!(
                user_message_chars = user_message.chars().count(),
                "web_search_augment: LLM query generation returned no parseable queries; \
                 falling back to verbatim user message as the single search query",
            );
            vec![user_message.to_string()]
        }
    };

    // Search with each query and collect results
    let mut all_results = String::new();
    for query in &queries {
        match ctx.search.search(query, 3).await {
            Ok(results) if !results.trim().is_empty() => {
                let original_chars = results.chars().count();
                let before_chars = all_results.chars().count();
                if !append_search_results(&mut all_results, &results) {
                    debug!("Web search augmentation reached its context budget");
                    break;
                }
                let added_chars = all_results
                    .chars()
                    .count()
                    .saturating_sub(before_chars)
                    .saturating_sub(usize::from(before_chars != 0));
                if added_chars < original_chars {
                    debug!(
                        %query,
                        original_chars,
                        kept_chars = added_chars,
                        "Truncated web search augmentation results"
                    );
                }
            }
            Ok(_) => {}
            Err(e) => {
                warn!(%query, "Web search augmentation query failed: {e}");
            }
        }
    }

    if all_results.trim().is_empty() {
        None
    } else {
        debug!("Web search augmentation: injecting search results");
        Some(all_results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_driver::{CompletionResponse, LlmError};
    use librefang_types::agent::ModelConfig;
    use librefang_types::message::{ContentBlock, StopReason, TokenUsage};
    use std::sync::{Arc, Mutex};

    /// Driver that records the `response_format` flag on every
    /// request, then returns a benign `{"queries": []}` so
    /// `generate_search_queries` resolves cleanly. Same shape as
    /// `history_fold::tests::ResponseFormatRecordingDriver`.
    struct ResponseFormatRecordingDriver {
        observed: Arc<Mutex<Vec<Option<ResponseFormat>>>>,
    }

    struct QueryResponseDriver {
        response: String,
    }

    impl ResponseFormatRecordingDriver {
        fn new() -> (Self, Arc<Mutex<Vec<Option<ResponseFormat>>>>) {
            let observed = Arc::new(Mutex::new(Vec::new()));
            (
                ResponseFormatRecordingDriver {
                    observed: Arc::clone(&observed),
                },
                observed,
            )
        }
    }

    #[async_trait::async_trait]
    impl LlmDriver for ResponseFormatRecordingDriver {
        async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            self.observed
                .lock()
                .unwrap()
                .push(req.response_format.clone());
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: r#"{"queries": []}"#.to_string(),
                    provider_metadata: None,
                }],
                tool_calls: vec![],
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage::default(),
                actual_provider: None,
                actual_model: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl LlmDriver for QueryResponseDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: self.response.clone(),
                    provider_metadata: None,
                }],
                tool_calls: vec![],
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage::default(),
                actual_provider: None,
                actual_model: None,
            })
        }
    }

    fn test_manifest() -> AgentManifest {
        AgentManifest {
            model: ModelConfig {
                provider: "test".to_string(),
                model: "test-model".to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Regression guard for #5287 (web_augment branch) —
    /// `generate_search_queries` must pin
    /// `response_format = Some(ResponseFormat::Json)` on the aux
    /// request. `SEARCH_QUERY_GEN_PROMPT` says "Respond ONLY with a
    /// JSON object: {"queries": [...]}", so strict-output providers
    /// (DeepSeek / OpenAI / Mistral / Gemini) need the flag set or
    /// they're free to emit prose. The downstream `text.find('{')?`
    /// then returns None and the entire augmentation path silently
    /// degrades to falling back to the raw user message — same
    /// silent-degradation class as history_fold's bulk_summary.
    #[tokio::test]
    async fn search_query_request_pins_response_format_json() {
        let (rec, observed) = ResponseFormatRecordingDriver::new();
        let driver: Box<dyn LlmDriver> = Box::new(rec);
        let manifest = test_manifest();

        let _ = generate_search_queries(
            driver.as_ref(),
            &manifest,
            &[],
            "what's the weather in tokyo?",
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        let observed = observed.lock().unwrap().clone();
        assert_eq!(
            observed.len(),
            1,
            "expected exactly one aux request for query generation"
        );
        assert_eq!(
            observed[0],
            Some(ResponseFormat::Json),
            "generate_search_queries must pin response_format = Json — \
             the SEARCH_QUERY_GEN_PROMPT explicitly asks for a JSON \
             object and strict-output providers need the flag set"
        );
    }

    #[tokio::test]
    async fn search_query_parser_allows_closing_brace_in_json_string() {
        let driver = QueryResponseDriver {
            response: r#"prefix {"queries": ["rust } parser"]} suffix"#.to_string(),
        };
        let queries = generate_search_queries(
            &driver,
            &test_manifest(),
            &[],
            "rust parser",
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        assert_eq!(queries, Some(vec!["rust } parser".to_string()]));
    }

    #[tokio::test]
    async fn search_query_parser_allows_opening_brace_in_json_string() {
        let driver = QueryResponseDriver {
            response: r#"prefix {"queries": ["rust { parser"]} suffix"#.to_string(),
        };
        let queries = generate_search_queries(
            &driver,
            &test_manifest(),
            &[],
            "rust parser",
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        assert_eq!(queries, Some(vec!["rust { parser".to_string()]));
    }

    #[tokio::test]
    async fn search_query_parser_allows_escaped_quote_before_brace() {
        let driver = QueryResponseDriver {
            response: r#"prefix {"queries": ["rust \"}\" parser"]} suffix"#.to_string(),
        };
        let queries = generate_search_queries(
            &driver,
            &test_manifest(),
            &[],
            "rust parser",
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        assert_eq!(queries, Some(vec!["rust \"}\" parser".to_string()]));
    }

    #[tokio::test]
    async fn search_query_parser_limits_provider_output() {
        let driver = QueryResponseDriver {
            response: r#"{"queries":["one","two","three","four"]}"#.to_string(),
        };
        let queries = generate_search_queries(
            &driver,
            &test_manifest(),
            &[],
            "rust parser",
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        assert_eq!(
            queries,
            Some(vec![
                "one".to_string(),
                "two".to_string(),
                "three".to_string()
            ])
        );
    }

    #[test]
    fn truncation_is_unicode_safe_and_preserves_external_boundary() {
        let wrapped = crate::web_content::wrap_external_content(
            "test-search",
            &"検索結果🙂".repeat(MAX_SEARCH_RESULT_CHARS_PER_QUERY),
        );
        let truncated = truncate_search_results(&wrapped, MAX_SEARCH_RESULT_CHARS_PER_QUERY);

        assert_eq!(truncated.chars().count(), MAX_SEARCH_RESULT_CHARS_PER_QUERY);
        assert!(truncated.contains(SEARCH_RESULTS_TRUNCATION_MARKER));
        let boundary = crate::web_content::content_boundary("test-search");
        assert!(truncated.ends_with(&format!("<<</{boundary}>>>")));
    }

    #[test]
    fn combined_search_results_obey_per_query_and_total_budgets() {
        let results = crate::web_content::wrap_external_content(
            "test-search",
            &"x".repeat(MAX_SEARCH_RESULT_CHARS_PER_QUERY * 2),
        );
        let mut combined = String::new();

        assert!(append_search_results(&mut combined, &results));
        assert!(append_search_results(&mut combined, &results));
        assert!(append_search_results(&mut combined, &results));
        assert_eq!(combined.chars().count(), MAX_WEB_SEARCH_AUGMENTATION_CHARS);
        assert_eq!(
            combined.matches(SEARCH_RESULTS_TRUNCATION_MARKER).count(),
            3
        );
        assert!(!append_search_results(&mut combined, &results));
    }
}
