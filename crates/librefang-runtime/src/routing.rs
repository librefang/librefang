//! Model routing — auto-selects cheap/mid/expensive models by query complexity.
//!
//! The router scores each `CompletionRequest` based on heuristics (token count,
//! code markers, conversation depth) and picks the cheapest model that can
//! handle the task.
//!
//! Every input is a property of the *request*.
//! Properties of the agent — how many tools it exposes, how long its system prompt is — were scored here until #7952 and are not any more: they are identical on every turn, so they shifted the whole distribution instead of separating easy turns from hard ones.
//! An agent with ~50 MCP tools scored +1000 from tool count alone, past any usable `complex_threshold`, so routing degenerated into pinned-complex and saved nothing.

use crate::llm_driver::CompletionRequest;
use librefang_types::agent::ModelRoutingConfig;

/// Task complexity tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskComplexity {
    /// Quick lookup, greetings, simple Q&A — use the cheapest model.
    Simple,
    /// Standard conversational task — use a mid-tier model.
    Medium,
    /// Multi-step reasoning, code generation, complex analysis — use the best model.
    Complex,
}

impl std::fmt::Display for TaskComplexity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskComplexity::Simple => write!(f, "simple"),
            TaskComplexity::Medium => write!(f, "medium"),
            TaskComplexity::Complex => write!(f, "complex"),
        }
    }
}

/// Model router that selects the appropriate model based on query complexity.
#[derive(Debug, Clone)]
pub struct ModelRouter {
    config: ModelRoutingConfig,
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn add_weighted_score(score: u64, count: u64, weight: u64) -> u64 {
    score.saturating_add(count.saturating_mul(weight))
}

impl ModelRouter {
    /// Create a new model router with the given routing configuration.
    pub fn new(config: ModelRoutingConfig) -> Self {
        Self { config }
    }

    /// Score a completion request and determine its complexity tier.
    ///
    /// Heuristics:
    /// - **Token count**: total characters in messages as a proxy for tokens
    /// - **Code markers**: backticks, `fn`, `def`, `class`, etc.
    /// - **Conversation depth**: more messages = more context = harder reasoning
    ///
    /// Tool count and system-prompt length are deliberately not scored — see the module docs (#7952).
    /// The decision is invariant under both.
    pub fn score(&self, request: &CompletionRequest) -> TaskComplexity {
        // 1. Total message content length (rough token proxy: ~4 chars per token)
        let total_chars = request.messages.iter().fold(0u64, |total, message| {
            total.saturating_add(usize_to_u64_saturating(message.content.text_length()))
        });
        let mut score = total_chars / 4;

        // 2. Code markers in the last user message
        if let Some(last_msg) = request.messages.last() {
            let text = last_msg.content.text_content();
            let text_lower = text.to_lowercase();
            let code_markers = [
                "```",
                "fn ",
                "def ",
                "class ",
                "import ",
                "function ",
                "async ",
                "await ",
                "struct ",
                "impl ",
                "return ",
            ];
            let code_score = usize_to_u64_saturating(
                code_markers
                    .iter()
                    .filter(|marker| text_lower.contains(*marker))
                    .count(),
            );
            score = add_weighted_score(score, code_score, 30);
        }

        // 3. Conversation depth
        let msg_count = usize_to_u64_saturating(request.messages.len());
        if msg_count > 10 {
            score = add_weighted_score(score, msg_count - 10, 15);
        }

        // Classify
        if score < u64::from(self.config.simple_threshold) {
            TaskComplexity::Simple
        } else if score >= u64::from(self.config.complex_threshold) {
            TaskComplexity::Complex
        } else {
            TaskComplexity::Medium
        }
    }

    /// Select the model name for a given complexity tier.
    pub fn model_for_complexity(&self, complexity: TaskComplexity) -> &str {
        match complexity {
            TaskComplexity::Simple => &self.config.simple_model,
            TaskComplexity::Medium => &self.config.medium_model,
            TaskComplexity::Complex => &self.config.complex_model,
        }
    }

    /// Score a request and return the selected model name + complexity.
    pub fn select_model(&self, request: &CompletionRequest) -> (TaskComplexity, String) {
        let complexity = self.score(request);
        let model = self.model_for_complexity(complexity).to_string();
        (complexity, model)
    }

    /// Validate that all configured models exist in the catalog.
    ///
    /// Returns a list of warning messages for models not found in the catalog.
    pub fn validate_models(&self, catalog: &crate::model_catalog::ModelCatalog) -> Vec<String> {
        let mut warnings = vec![];
        for model in [
            &self.config.simple_model,
            &self.config.medium_model,
            &self.config.complex_model,
        ] {
            if catalog.find_model(model).is_none() {
                warnings.push(format!("Model '{}' not found in catalog", model));
            }
        }
        warnings
    }

    /// Resolve aliases in the routing config using the catalog.
    ///
    /// For example, if "sonnet" is configured, resolves to "claude-sonnet-4-6".
    pub fn resolve_aliases(&mut self, catalog: &crate::model_catalog::ModelCatalog) {
        if let Some(resolved) = catalog.resolve_alias(&self.config.simple_model) {
            self.config.simple_model = resolved.to_string();
        }
        if let Some(resolved) = catalog.resolve_alias(&self.config.medium_model) {
            self.config.medium_model = resolved.to_string();
        }
        if let Some(resolved) = catalog.resolve_alias(&self.config.complex_model) {
            self.config.complex_model = resolved.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::message::{Message, MessageContent, Role};
    use librefang_types::tool::ToolDefinition;

    fn test_catalog() -> crate::model_catalog::ModelCatalog {
        let home = crate::registry_sync::resolve_home_dir_for_tests();
        crate::model_catalog::ModelCatalog::new(&home)
    }

    fn default_config() -> ModelRoutingConfig {
        ModelRoutingConfig {
            simple_model: "llama-3.3-70b-versatile".to_string(),
            medium_model: "sonnet".to_string(),
            complex_model: "opus".to_string(),
            simple_threshold: 200,
            complex_threshold: 800,
        }
    }

    fn make_request(messages: Vec<Message>, tools: Vec<ToolDefinition>) -> CompletionRequest {
        CompletionRequest {
            model: "placeholder".to_string(),
            messages: std::sync::Arc::new(messages),
            tools: std::sync::Arc::new(tools),
            max_tokens: 4096,
            temperature: 0.7,
            system: None,
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            prompt_cache_strategy: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
            reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),

            ..Default::default()
        }
    }

    #[test]
    fn test_simple_greeting_routes_to_simple() {
        let router = ModelRouter::new(default_config());
        let request = make_request(
            vec![Message {
                role: Role::User,
                content: MessageContent::text("Hello!"),
                pinned: false,
                timestamp: None,
            }],
            vec![],
        );
        let (complexity, model) = router.select_model(&request);
        assert_eq!(complexity, TaskComplexity::Simple);
        assert_eq!(model, "llama-3.3-70b-versatile");
    }

    #[test]
    fn test_code_markers_increase_complexity() {
        let router = ModelRouter::new(default_config());
        let request = make_request(
            vec![Message {
                role: Role::User,
                content: MessageContent::text(
                    "Write a function that implements async file reading with struct and impl blocks:\n\
                     ```rust\nfn main() { }\n```"
                ),
                pinned: false,
                timestamp: None,
            }],
            vec![],
        );
        let complexity = router.score(&request);
        // Should be at least Medium due to code markers
        assert_ne!(complexity, TaskComplexity::Simple);
    }

    fn n_tools(n: usize) -> Vec<ToolDefinition> {
        (0..n)
            .map(|i| ToolDefinition {
                name: format!("tool_{i}"),
                description: "A test tool".to_string(),
                input_schema: serde_json::json!({}),
            })
            .collect()
    }

    /// #7952: the routing decision must be a property of the request, not of the agent's shape.
    /// A tool-rich agent (50 MCP tools was the reported case) scored +20 per tool, so every turn — including "hi" — landed above `complex_threshold` and the router degenerated into pinned-complex.
    #[test]
    fn tool_count_does_not_change_the_tier() {
        let router = ModelRouter::new(default_config());
        let message = Message {
            role: Role::User,
            content: MessageContent::text("Use the available tools to solve this problem."),
            pinned: false,
            timestamp: None,
        };

        let none = router.score(&make_request(vec![message.clone()], vec![]));
        let some = router.score(&make_request(vec![message.clone()], n_tools(15)));
        let many = router.score(&make_request(vec![message], n_tools(200)));

        assert_eq!(none, some, "15 tools must not move the tier");
        assert_eq!(none, many, "200 tools must not move the tier");
    }

    /// The same turn on a tool-rich agent must still be routed cheaply.
    /// Asserting invariance alone would pass if every tier collapsed to Complex, which is the bug.
    #[test]
    fn a_trivial_message_stays_simple_on_a_tool_rich_agent() {
        let router = ModelRouter::new(default_config());
        let request = make_request(
            vec![Message {
                role: Role::User,
                content: MessageContent::text("hi"),
                pinned: false,
                timestamp: None,
            }],
            n_tools(200),
        );
        assert_eq!(router.score(&request), TaskComplexity::Simple);
    }

    #[test]
    fn test_complexity_score_arithmetic_saturates() {
        assert_eq!(add_weighted_score(u64::MAX - 5, u64::MAX, 20), u64::MAX);
    }

    #[test]
    fn test_long_conversation_routes_higher() {
        let router = ModelRouter::new(default_config());
        // 20 messages with moderate content
        let messages: Vec<Message> = (0..20)
            .map(|i| Message {
                role: if i % 2 == 0 { Role::User } else { Role::Assistant },
                content: MessageContent::text(format!(
                    "This is message {} with enough content to add some token weight to the conversation.",
                    i
                )),
                pinned: false,
                timestamp: None,
            })
            .collect();
        let request = make_request(messages, vec![]);
        let complexity = router.score(&request);
        // Long conversation should be Medium or Complex
        assert_ne!(complexity, TaskComplexity::Simple);
    }

    #[test]
    fn test_model_for_complexity() {
        // Asserts the unresolved-router behaviour: model_for_complexity()
        // returns the raw field value from ModelRoutingConfig without
        // consulting the catalog. The default_config() above ships
        // aliases (`sonnet`, `opus`) on purpose so production picks up
        // whichever canonical Sonnet / Opus the catalog currently
        // points to — this test just confirms the router doesn't
        // resolve them prematurely.
        let router = ModelRouter::new(default_config());
        assert_eq!(
            router.model_for_complexity(TaskComplexity::Simple),
            "llama-3.3-70b-versatile"
        );
        assert_eq!(
            router.model_for_complexity(TaskComplexity::Medium),
            "sonnet"
        );
        assert_eq!(router.model_for_complexity(TaskComplexity::Complex), "opus");
    }

    #[test]
    fn test_complexity_display() {
        assert_eq!(TaskComplexity::Simple.to_string(), "simple");
        assert_eq!(TaskComplexity::Medium.to_string(), "medium");
        assert_eq!(TaskComplexity::Complex.to_string(), "complex");
    }

    #[test]
    fn test_validate_models_all_found() {
        let catalog = test_catalog();
        let config = ModelRoutingConfig {
            simple_model: "llama-3.3-70b-versatile".to_string(),
            medium_model: "claude-sonnet-4-6".to_string(),
            complex_model: "claude-opus-4-6".to_string(),
            simple_threshold: 200,
            complex_threshold: 800,
        };
        let router = ModelRouter::new(config);
        let warnings = router.validate_models(&catalog);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_validate_models_unknown() {
        let catalog = test_catalog();
        let config = ModelRoutingConfig {
            simple_model: "unknown-model".to_string(),
            medium_model: "claude-sonnet-4-6".to_string(),
            complex_model: "claude-opus-4-6".to_string(),
            simple_threshold: 200,
            complex_threshold: 800,
        };
        let router = ModelRouter::new(config);
        let warnings = router.validate_models(&catalog);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("unknown-model"));
    }

    #[test]
    fn test_resolve_aliases() {
        let catalog = test_catalog();
        let config = ModelRoutingConfig {
            simple_model: "llama".to_string(),
            medium_model: "sonnet".to_string(),
            complex_model: "opus".to_string(),
            simple_threshold: 200,
            complex_threshold: 800,
        };
        let mut router = ModelRouter::new(config);
        router.resolve_aliases(&catalog);
        assert_eq!(
            router.model_for_complexity(TaskComplexity::Simple),
            "llama-3.3-70b-versatile"
        );
        assert_eq!(
            router.model_for_complexity(TaskComplexity::Medium),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            router.model_for_complexity(TaskComplexity::Complex),
            "claude-opus-4-7"
        );
    }

    /// #7952: system-prompt length is the same on every turn of an agent, so scoring it moved the whole distribution rather than separating turns.
    /// A long SOUL.md must not make "Hi" a complex request.
    #[test]
    fn system_prompt_length_does_not_change_the_tier() {
        let router = ModelRouter::new(default_config());
        let message = Message {
            role: Role::User,
            content: MessageContent::text("Hi"),
            pinned: false,
            timestamp: None,
        };

        let mut long = make_request(vec![message.clone()], vec![]);
        long.system = Some("A".repeat(20_000));

        let mut short = make_request(vec![message], vec![]);
        short.system = Some("Be helpful.".to_string());

        assert_eq!(router.score(&long), router.score(&short));
        assert_eq!(router.score(&long), TaskComplexity::Simple);
    }
}
