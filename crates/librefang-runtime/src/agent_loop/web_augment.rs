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
use librefang_types::message::{Message, Role};
use tracing::{debug, warn};

use super::strip_provider_prefix;

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
        response_format: None,
        timeout_secs: Some(15),
        extra_body: None,
        agent_id: None,
        session_id: None,
        step_id: None,
        reasoning_echo_policy,
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
    // Extract JSON from response — find the outermost { }
    let start = text.find('{')?;
    let end = text.rfind('}')? + 1;
    let json_str = &text[start..end];

    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let queries: Vec<String> = parsed["queries"]
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
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
        None => vec![user_message.to_string()], // Generation failed, fall back to raw message
    };

    // Search with each query and collect results
    let mut all_results = String::new();
    for query in &queries {
        match ctx.search.search(query, 3).await {
            Ok(results) if !results.trim().is_empty() => {
                all_results.push_str(&results);
                all_results.push('\n');
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
