//! Proactive Memory integration for the runtime.
//!
//! Provides `init_proactive_memory` to create a `ProactiveMemoryStore` for the
//! kernel. The actual `auto_retrieve` and `auto_memorize` calls happen directly
//! in `agent_loop.rs` rather than through fire-and-forget hooks, ensuring
//! results are properly consumed and peer-scoped.

use librefang_memory::{ProactiveMemoryConfig, ProactiveMemoryStore};
use librefang_types::config::ResponseFormat;
use librefang_types::error::LibreFangError;
use librefang_types::memory::{
    ExtractionResult, MemoryAction, MemoryExtractor, MemoryFragment, MemoryItem, MemoryLevel,
};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// EmbeddingDriver → EmbeddingFn bridge
// ---------------------------------------------------------------------------

/// Wraps the runtime's `EmbeddingDriver` to implement `EmbeddingFn` (from librefang-memory).
/// This avoids a circular dependency between librefang-memory and librefang-runtime.
struct EmbeddingBridge(Arc<dyn crate::embedding::EmbeddingDriver + Send + Sync>);

#[async_trait::async_trait]
impl librefang_memory::proactive::EmbeddingFn for EmbeddingBridge {
    async fn embed_one(&self, text: &str) -> librefang_types::error::LibreFangResult<Vec<f32>> {
        self.0
            .embed_one(text)
            .await
            .map_err(|e| LibreFangError::Internal(format!("Embedding failed: {e}")))
    }
}

/// Initialize proactive memory system.
///
/// Creates a `ProactiveMemoryStore` if either auto_retrieve or auto_memorize is enabled.
/// The store is used directly by `agent_loop` — no hook registration needed since
/// the loop calls `auto_retrieve`/`auto_memorize` inline for proper result consumption.
///
/// Returns `None` if both features are disabled.
pub fn init_proactive_memory(
    memory: Arc<librefang_memory::MemorySubstrate>,
    config: ProactiveMemoryConfig,
) -> Option<Arc<ProactiveMemoryStore>> {
    init_proactive_memory_full(memory, config, None, None)
}

/// Initialize proactive memory with an LLM-powered extractor.
///
/// When a driver is provided, memory extraction uses the LLM for high-quality
/// results. Falls back to `init_proactive_memory` (rule-based) if no driver.
pub fn init_proactive_memory_with_llm(
    memory: Arc<librefang_memory::MemorySubstrate>,
    config: ProactiveMemoryConfig,
    driver: Arc<dyn crate::llm_driver::LlmDriver>,
    model: String,
) -> Option<Arc<ProactiveMemoryStore>> {
    init_proactive_memory_full(memory, config, Some((driver, model)), None)
}

/// Initialize proactive memory with an embedding driver for vector search.
pub fn init_proactive_memory_with_embedding(
    memory: Arc<librefang_memory::MemorySubstrate>,
    config: ProactiveMemoryConfig,
    llm: Option<(Arc<dyn crate::llm_driver::LlmDriver>, String)>,
    embedding: Arc<dyn crate::embedding::EmbeddingDriver + Send + Sync>,
) -> Option<Arc<ProactiveMemoryStore>> {
    init_proactive_memory_full(memory, config, llm, Some(embedding))
}

/// Full initialization: LLM extractor + embedding driver (both optional).
pub fn init_proactive_memory_full(
    memory: Arc<librefang_memory::MemorySubstrate>,
    config: ProactiveMemoryConfig,
    llm: Option<(Arc<dyn crate::llm_driver::LlmDriver>, String)>,
    embedding: Option<Arc<dyn crate::embedding::EmbeddingDriver + Send + Sync>>,
) -> Option<Arc<ProactiveMemoryStore>> {
    // Legacy callers (tests, external) can't pass prompt_caching — default
    // to true to match the behaviour shipped before the global-toggle fix.
    let (store, _extractor) =
        init_proactive_memory_full_with_extractor(memory, config, llm, embedding, true)?;
    Some(store)
}

/// Like [`init_proactive_memory_full`] but also returns the concrete
/// `LlmMemoryExtractor` (when one was configured). The kernel needs the
/// concrete handle so it can call `install_kernel_handle` once
/// `Arc<LibreFangKernel>` exists — the fork-based extraction path needs
/// a `Weak<dyn KernelHandle>` which can't be formed before the kernel
/// is in an Arc.
///
/// `prompt_caching` controls whether the extractor's fallback
/// `driver.complete()` path stamps `cache_control` markers. Should be
/// threaded from `KernelConfig.prompt_caching` so operators who disable
/// caching globally see proactive memory also skip it. The fork path
/// inherits caching from the agent's own manifest metadata, which the
/// kernel derives from the same global — so this flag only gates the
/// fallback.
pub fn init_proactive_memory_full_with_extractor(
    memory: Arc<librefang_memory::MemorySubstrate>,
    config: ProactiveMemoryConfig,
    llm: Option<(Arc<dyn crate::llm_driver::LlmDriver>, String)>,
    embedding: Option<Arc<dyn crate::embedding::EmbeddingDriver + Send + Sync>>,
    prompt_caching: bool,
) -> Option<(Arc<ProactiveMemoryStore>, Option<Arc<LlmMemoryExtractor>>)> {
    if !config.auto_retrieve && !config.auto_memorize {
        tracing::debug!("Proactive memory is disabled");
        return None;
    }

    let (mut store, llm_extractor): (_, Option<Arc<LlmMemoryExtractor>>) =
        if let Some((driver, model)) = llm {
            // Hold two handles to the same extractor: one as the concrete
            // type (so the kernel can install its weak self-ref on it
            // later), one as the trait object (so the store can invoke it
            // via `MemoryExtractor`).
            let extractor_concrete = Arc::new(LlmMemoryExtractor::with_prompt_caching(
                driver,
                model,
                prompt_caching,
            ));
            let extractor_dyn: Arc<dyn librefang_types::memory::MemoryExtractor> =
                Arc::clone(&extractor_concrete) as _;
            (
                ProactiveMemoryStore::with_extractor(memory, config, extractor_dyn),
                Some(extractor_concrete),
            )
        } else {
            (ProactiveMemoryStore::new(memory, config), None)
        };

    if let Some(emb) = embedding {
        store = store.with_embedding(Arc::new(EmbeddingBridge(emb)));
        tracing::info!("Proactive memory system initialized (with embeddings)");
    } else {
        tracing::info!("Proactive memory system initialized (text search fallback)");
    }

    Some((Arc::new(store), llm_extractor))
}

/// Initialize proactive memory with default configuration.
pub fn init_proactive_memory_with_defaults(
    memory: Arc<librefang_memory::MemorySubstrate>,
) -> Option<Arc<ProactiveMemoryStore>> {
    init_proactive_memory(memory, ProactiveMemoryConfig::default())
}

// ---------------------------------------------------------------------------
// LLM-powered Memory Extractor
// ---------------------------------------------------------------------------

const MAX_MEMORY_CONTENT_LENGTH: usize = 2000;

fn build_extraction_prompt(categories: &[String]) -> String {
    let categories_list = if categories.is_empty() {
        "any relevant category".to_string()
    } else {
        categories.join(", ")
    };
    format!(
        r#"You are a memory extraction system. Your goal: help a future assistant feel like it truly knows this person — their style, preferences, expertise, and what matters to them.

Extract ONLY clearly stated or strongly demonstrated facts. Do NOT infer personality traits from single messages. Prioritize what would most change how you interact with someone.

## What to extract (in priority order)

1. **Communication style & language**: Concise vs. detailed? Formal vs. casual? Do they write in a specific language (e.g., Chinese, English)? Do they prefer code-heavy answers or conceptual explanations?
2. **Frustrations & pet peeves**: What annoys them? What mistakes should be avoided? These are the most actionable memories — they prevent you from doing things the person hates.
3. **Preferences & opinions**: Tools, languages, frameworks, themes, workflows they like or dislike. Strong opinions about how things should be done.
4. **Work style & autonomy**: Do they want you to just do it, or discuss first? Step-by-step or big-picture? Do they review diffs or trust you?
5. **Technical background**: Expertise level, technologies they work with, role, domain. What they know well vs. what they're learning.
6. **Project context**: Key projects, architectures, recurring tasks, decisions made and why.
7. **Personal details**: Name, timezone, team, anything they voluntarily shared.

## How to write memories

Write each memory as a natural observation that captures nuance — not as a flat database entry.

GOOD: "Prefers concise, direct answers — skips caveats and gets to the point"
BAD: "User prefers concise communication"

GOOD: "Gets frustrated when code suggestions don't compile — always verify before suggesting"
BAD: "User dislikes compilation errors"

GOOD: "Communicates in Chinese; switch to Chinese unless they write in English first"
BAD: "User language: Chinese"

GOOD: "Highly autonomous — wants changes made, not discussed. Just do it and explain after."
BAD: "User prefers autonomous execution"

## Response format

Respond with a JSON object containing two arrays:

1. "memories" - Facts and preferences to remember:
   - "content": the extracted memory (concise, one natural sentence with actionable nuance)
   - "category": one of: {categories_list}
   - "level": "user" for personal/preference info, "session" for current task context, "agent" for agent-specific learnings

2. "relations" - Entity relationships (knowledge graph triples):
   - "subject": entity name (e.g., "Alice")
   - "subject_type": person, organization, project, concept, location, tool
   - "relation": works_at, uses, prefers, knows_about, located_in, part_of, depends_on, dislikes, experienced_with
   - "object": related entity name (e.g., "Acme Corp")
   - "object_type": same types as subject_type

Example:
{{
  "memories": [
    {{"content": "Experienced Rust developer who works on the LibreFang project — treat as expert, skip beginner explanations", "category": "{first_cat}", "level": "user"}},
    {{"content": "Prefers concise code reviews — skip obvious comments, focus on logic and correctness issues only", "category": "{second_cat}", "level": "user"}}
  ],
  "relations": [
    {{"subject": "User", "subject_type": "person", "relation": "experienced_with", "object": "Rust", "object_type": "tool"}}
  ]
}}

If nothing worth extracting: {{"memories": [], "relations": []}}"#,
        categories_list = categories_list,
        first_cat = categories
            .first()
            .map(|s| s.as_str())
            .unwrap_or("preference"),
        second_cat = categories
            .get(1)
            .map(|s| s.as_str())
            .unwrap_or("preference"),
    )
}

const DECISION_SYSTEM_PROMPT: &str = r#"You are a memory conflict resolution system. Given a NEW memory and a list of EXISTING memories, decide what action to take.

Actions:
- "ADD": The new memory is genuinely new information. No existing memory covers this.
- "UPDATE": The new memory updates/supersedes an existing memory (e.g., user changed preference, corrected a fact). Return the ID of the memory to replace.
- "NOOP": The new memory is a duplicate or already covered by an existing memory. Skip it.

Guidelines:
- If existing memory says "User prefers Python" and new says "User prefers Rust" → UPDATE (preference changed)
- If existing memory says "User's name is John" and new says "User's name is John" → NOOP (duplicate)
- If existing memory says "User works at Acme" and new says "User works at Google now" → UPDATE (fact changed)
- If no existing memory is related → ADD

Respond with a single JSON object:
{"action": "ADD"} or {"action": "UPDATE", "existing_id": "<id>"} or {"action": "NOOP"}

If nothing matches, default to ADD."#;

/// LLM-powered memory extractor that uses a language model to identify
/// important information from conversations.
///
/// When `kernel_handle` is set and `extract_memories_with_agent_id` is
/// called (auto_memorize's path), extraction runs through
/// `KernelHandle::run_forked_agent_oneshot`. The fork shares the parent
/// agent's `(system + tools + messages)` prefix, so Anthropic's prompt
/// cache hits on the full conversation history instead of being rebuilt
/// for every auto_memorize call. The extraction-specific system prompt
/// (`EXTRACTION_SYSTEM_PROMPT`) is embedded into the fork's user message
/// rather than replacing the agent's system — keeping the cache key
/// intact at the cost of a few hundred tokens added to the (uncached)
/// fork user message. Net positive when the agent's own system prompt is
/// large enough that caching it outweighs the extra user-message tokens.
///
/// Without `kernel_handle` (or when the fork call fails), falls back to
/// the original standalone `driver.complete()` path with
/// `prompt_caching = true` so at least the extractor's own system prompt
/// caches across back-to-back calls.
pub struct LlmMemoryExtractor {
    driver: Arc<dyn crate::llm_driver::LlmDriver>,
    model: String,
    /// Whether to stamp `prompt_caching = true` on the extraction
    /// `driver.complete()` request. Mirrors the global
    /// `KernelConfig.prompt_caching` toggle — operators who disable
    /// caching at the kernel level (compatibility, cost accounting,
    /// debugging) should see proactive-memory requests also skip
    /// `cache_control`.
    prompt_caching: bool,
    /// Weak reference to the kernel handle, used to look up the
    /// catalog-driven `reasoning_echo_policy` for the extractor's model
    /// (#4842). `None` when no handle has been installed (e.g. tests, or
    /// callers that build the extractor without `install_kernel_handle`);
    /// in that case the OpenAI driver's substring fallback resolves the
    /// policy by model name.
    kernel_handle:
        std::sync::Mutex<Option<std::sync::Weak<dyn crate::kernel_handle::KernelHandle>>>,
}

impl LlmMemoryExtractor {
    pub fn new(driver: Arc<dyn crate::llm_driver::LlmDriver>, model: String) -> Self {
        Self::with_prompt_caching(driver, model, true)
    }

    /// Explicit variant for callers that want to control the
    /// extraction `prompt_caching` flag — typically the kernel,
    /// which passes `KernelConfig.prompt_caching` through so the
    /// extractor honours the same global toggle as the main loop.
    pub fn with_prompt_caching(
        driver: Arc<dyn crate::llm_driver::LlmDriver>,
        model: String,
        prompt_caching: bool,
    ) -> Self {
        Self {
            driver,
            model,
            prompt_caching,
            kernel_handle: std::sync::Mutex::new(None),
        }
    }

    /// Store a weak handle to the kernel so the extractor can look up
    /// catalog-driven metadata at request-build time (currently the
    /// `reasoning_echo_policy` for #4842). Idempotent — overwrites any
    /// previously installed handle.
    ///
    /// **History**: this method existed as a no-op before #4842 (kept for
    /// backwards compatibility with kernel init which still calls it on
    /// every extractor it constructs — the previous fork-based extraction
    /// pathway it once supported was removed because it bypassed JSON
    /// mode; see `extract_memories_with_agent_id` for details). #4842
    /// repurposed the slot to actually store the handle.
    pub fn install_kernel_handle(
        &self,
        handle: std::sync::Weak<dyn crate::kernel_handle::KernelHandle>,
    ) {
        if let Ok(mut slot) = self.kernel_handle.lock() {
            *slot = Some(handle);
        }
    }

    /// Resolve the `reasoning_echo_policy` for [`Self::model`] via the
    /// installed kernel handle. Returns `None` (the safe default) when no
    /// handle is installed, the kernel has been dropped, or the model
    /// isn't in the catalog — the driver's substring fallback handles
    /// those cases.
    fn echo_policy(&self) -> librefang_types::model_catalog::ReasoningEchoPolicy {
        self.kernel_handle
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref()?.upgrade())
            .map(|k| k.reasoning_echo_policy_for(&self.model))
            .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl MemoryExtractor for LlmMemoryExtractor {
    async fn extract_memories(
        &self,
        messages: &[serde_json::Value],
        categories: &[String],
    ) -> librefang_types::error::LibreFangResult<ExtractionResult> {
        // Build a condensed version of the conversation for the LLM.
        // Skip system messages — only include user and assistant roles.
        // Cap total text to ~8000 chars to avoid exceeding extraction model context.
        const MAX_EXTRACTION_CHARS: usize = 8000;
        let mut conversation_text = String::new();
        for msg in messages {
            let role = msg
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            if role == "system" {
                continue;
            }
            if role == "unknown" {
                tracing::debug!(message = ?msg, "Skipping proactive memory message with unknown role");
                continue;
            }
            let content = match msg.get("content") {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(serde_json::Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| {
                        if let Some(s) = v.get("text").and_then(|t| t.as_str()) {
                            Some(s.to_string())
                        } else {
                            v.as_str().map(|s| s.to_string())
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            if !content.is_empty() {
                conversation_text.push_str(&format!("{role}: {content}\n"));
                if conversation_text.len() > MAX_EXTRACTION_CHARS {
                    // Truncate at last complete message (last newline within limit)
                    if let Some(last_newline) =
                        conversation_text[..MAX_EXTRACTION_CHARS].rfind('\n')
                    {
                        conversation_text.truncate(last_newline);
                    } else {
                        // No newline within limit — truncate at char boundary
                        let mut safe = MAX_EXTRACTION_CHARS;
                        while safe > 0 && !conversation_text.is_char_boundary(safe) {
                            safe -= 1;
                        }
                        conversation_text.truncate(safe);
                    }
                    break;
                }
            }
        }

        if conversation_text.is_empty() {
            return Ok(ExtractionResult {
                has_content: false,
                memories: Vec::new(),
                relations: Vec::new(),
                trigger: "llm_extractor".to_string(),
                conflicts: Vec::new(),
            });
        }

        // NOTE: the fork-based path lives in `extract_memories_with_agent_id`
        // — `extract_memories` has no agent_id and therefore can't target
        // a fork. When auto_memorize wants the fork benefits it must call
        // the _with_agent_id variant (it does, via the trait).
        //
        // Build the LLM request. `prompt_caching: true` lets Anthropic
        // cache the ~1KB `EXTRACTION_SYSTEM_PROMPT` across back-to-back
        // auto_memorize calls — the user message (conversation text)
        // differs every call, but the system prompt is stable, so the
        // driver stamps a `cache_control` marker on the system block and
        // subsequent calls within the 5-min TTL hit cache. Non-Anthropic
        // providers ignore the flag (OpenAI caches automatically; others
        // no-op), so enabling it is safe cross-provider.
        //
        // NOTE: this does NOT share cache with the main agent's turn —
        // LlmMemoryExtractor deliberately uses its own `EXTRACTION_SYSTEM_PROMPT`
        // (not the agent's system prompt) for better extraction quality.
        // Cross-call parent-child cache sharing would require rewriting
        // the extractor to use the forkedAgent pattern + tool calls
        // (libre-code's `extractMemories` shape); that's a separate PR.
        let request = crate::llm_driver::CompletionRequest {
            model: self.model.clone(),
            messages: std::sync::Arc::new(vec![librefang_types::message::Message::user(format!(
                "Extract memories from this conversation:\n\n{conversation_text}"
            ))]),
            tools: std::sync::Arc::new(Vec::new()),
            max_tokens: 1024,
            temperature: 0.1,
            system: Some(build_extraction_prompt(categories)),
            thinking: None,
            prompt_caching: self.prompt_caching,
            cache_ttl: None,
            prompt_cache_strategy: None,
            response_format: Some(ResponseFormat::Json),
            timeout_secs: Some(30),
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
            reasoning_echo_policy: self.echo_policy(),
        };

        let response = self.driver.complete(request).await.map_err(|e| {
            tracing::error!("LLM extraction failed: {e}");
            LibreFangError::Internal(format!("LLM extraction failed: {e}"))
        })?;

        let text = response.text();
        parse_llm_extraction_response(&text)
    }

    /// Fork-aware extraction path. When a kernel handle is configured,
    /// routes the extraction LLM call through `run_forked_agent_oneshot`
    /// so it shares the parent agent's cache key. Falls back to the
    /// standalone `extract_memories` path on any fork failure or when
    /// no kernel handle was wired.
    async fn extract_memories_with_agent_id(
        &self,
        messages: &[serde_json::Value],
        agent_id: &str,
        categories: &[String],
    ) -> librefang_types::error::LibreFangResult<ExtractionResult> {
        // Re-run the conversation-text builder from `extract_memories`
        // so the fork prompt gets the same truncation / role filtering.
        // Duplicating ~40 lines here keeps each method independently
        // auditable — factoring into a helper would tangle the fork
        // path with the direct path's truncation quirks (the LLM's
        // ~8000-char cap is specific to the standalone extractor's
        // model context; a forked agent inherits its own limits from
        // the parent manifest).
        const MAX_EXTRACTION_CHARS: usize = 8000;
        let mut conversation_text = String::new();
        for msg in messages {
            let role = msg
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            if role == "system" || role == "unknown" {
                continue;
            }
            let content = match msg.get("content") {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(serde_json::Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| {
                        v.get("text")
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string())
                            .or_else(|| v.as_str().map(|s| s.to_string()))
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            if !content.is_empty() {
                conversation_text.push_str(&format!("{role}: {content}\n"));
                if conversation_text.len() > MAX_EXTRACTION_CHARS {
                    if let Some(last_newline) =
                        conversation_text[..MAX_EXTRACTION_CHARS].rfind('\n')
                    {
                        conversation_text.truncate(last_newline);
                    } else {
                        let mut safe = MAX_EXTRACTION_CHARS;
                        while safe > 0 && !conversation_text.is_char_boundary(safe) {
                            safe -= 1;
                        }
                        conversation_text.truncate(safe);
                    }
                    break;
                }
            }
        }

        if conversation_text.is_empty() {
            return Ok(ExtractionResult {
                has_content: false,
                memories: Vec::new(),
                relations: Vec::new(),
                trigger: "llm_extractor_forked_empty".to_string(),
                conflicts: Vec::new(),
            });
        }

        // Always use the standalone path. The fork path cannot thread
        // `response_format: json_object` through `run_forked_agent_oneshot`,
        // so providers that honour JSON mode (ollama, OpenAI-compat,
        // Anthropic with json schemas, …) lose that guarantee and weak
        // models reply in prose, causing `Failed to parse extraction
        // response JSON` warnings with no memory extracted.
        //
        // Standalone gives us JSON mode + dedicated EXTRACTION_SYSTEM_PROMPT
        // + per-call system-block caching (`prompt_caching = true`). The
        // cross-call parent-child cache sharing that fork was supposed to
        // enable was never fully wired (see the "separate PR" note in
        // `extract_memories`), so there is no real regression from
        // skipping fork here.
        let _ = agent_id; // kept in signature for forward compat
        let _ = conversation_text;
        self.extract_memories(messages, categories).await
    }

    /// LLM-powered conflict resolution: decide ADD/UPDATE/NOOP.
    ///
    /// Sends the new memory and existing candidates to the LLM for reasoning.
    /// Falls back to the default heuristic if the LLM call fails.
    async fn decide_action(
        &self,
        new_memory: &MemoryItem,
        existing_memories: &[MemoryFragment],
    ) -> librefang_types::error::LibreFangResult<MemoryAction> {
        // If no existing memories, always ADD
        if existing_memories.is_empty() {
            return Ok(MemoryAction::Add);
        }

        // Build the context for the LLM
        let mut existing_text = String::new();
        for (i, mem) in existing_memories.iter().enumerate() {
            existing_text.push_str(&format!(
                "{}. [ID: {}] \"{}\"\n",
                i + 1,
                mem.id,
                mem.content
            ));
        }

        let user_msg = format!(
            "NEW MEMORY: \"{}\"\n\nEXISTING MEMORIES:\n{}",
            new_memory.content, existing_text
        );

        // Same caching rationale as `extract_memories` above — the
        // `DECISION_SYSTEM_PROMPT` is stable across calls, so enabling
        // prompt caching lets Anthropic cache the system block. The user
        // message (existing memories + new candidate) varies every call,
        // so message-level caching doesn't help here. System-only cache
        // is still a real saving on active agents where `decide_action`
        // fires dozens of times per session.
        let request = crate::llm_driver::CompletionRequest {
            model: self.model.clone(),
            messages: std::sync::Arc::new(vec![librefang_types::message::Message::user(user_msg)]),
            tools: std::sync::Arc::new(Vec::new()),
            max_tokens: 256,
            temperature: 0.0,
            system: Some(DECISION_SYSTEM_PROMPT.to_string()),
            thinking: None,
            prompt_caching: self.prompt_caching,
            cache_ttl: None,
            prompt_cache_strategy: None,
            // DECISION_SYSTEM_PROMPT asks for `{"action": "...", "existing_id": "..."}`
            // — tell JSON-mode-capable providers to honour it so weak models
            // can't drift into prose.
            response_format: Some(ResponseFormat::Json),
            timeout_secs: Some(15),
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
            reasoning_echo_policy: self.echo_policy(),
        };

        match self.driver.complete(request).await {
            Ok(response) => {
                let text = response.text();
                parse_decision_response(&text, existing_memories)
            }
            Err(e) => {
                tracing::warn!("LLM decision call failed, falling back to heuristic: {e}");
                // Fall back to default heuristic
                let default_extractor = librefang_types::memory::DefaultMemoryExtractor;
                default_extractor
                    .decide_action(new_memory, existing_memories)
                    .await
            }
        }
    }

    fn format_context(&self, memories: &[MemoryItem]) -> String {
        if memories.is_empty() {
            return String::new();
        }

        let mut context = String::from(
            "You have the following understanding of this person from previous conversations. \
             This is knowledge you have — not a list to recite. Let it naturally shape how you \
             respond:\n\
             \n\
             - Reference relevant context when it helps (\"since you're working in Rust...\", \
             \"keeping it concise like you prefer...\") but only when it genuinely adds value.\n\
             - Let remembered preferences silently guide your style, format, and depth — you \
             don't need to announce that you're doing so.\n\
             - NEVER say \"based on my memory\", \"according to my records\", \"I recall that you...\", \
             or mechanically list what you know. A friend doesn't preface every remark with \
             \"I remember you told me...\".\n\
             - If a memory is clearly outdated or the user contradicts it, trust the current \
             conversation over stored context.\n\n",
        );
        for mem in memories {
            context.push_str(&format!("- {}\n", mem.content));
        }
        context
    }
}

/// Strip markdown code blocks from LLM output.
///
/// Handles case-insensitive language tags (```json, ```JSON, ```Json, etc.),
/// leading text before the code block, and extracts the content between the
/// first ``` and last ```.
fn strip_code_block(text: &str) -> &str {
    let trimmed = text.trim();
    // Find first ``` and last ```, extract content between them
    if let Some(start) = trimmed.find("```") {
        let after_start = &trimmed[start + 3..];
        // Skip language tag: find newline, or skip to first `[` or `{` if no newline
        let content_start = if let Some(newline_pos) = after_start.find('\n') {
            newline_pos + 1
        } else {
            after_start.find(['[', '{']).unwrap_or(0)
        };
        let content = &after_start[content_start..];
        if let Some(end) = content.rfind("```") {
            return content[..end].trim();
        }
    }
    trimmed
}

/// Parse the LLM's decision response into a MemoryAction.
fn parse_decision_response(
    text: &str,
    existing_memories: &[MemoryFragment],
) -> librefang_types::error::LibreFangResult<MemoryAction> {
    // Strip markdown code blocks (case-insensitive, handles leading text)
    let json_str = strip_code_block(text);

    let parsed: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(val) => val,
        Err(e) => {
            // Weak / local models routinely reply in prose instead of JSON.
            // The caller already falls back to the default ADD action, so
            // this is expected fallback behavior and should not warn on
            // every post-turn memory decision.
            tracing::debug!("Failed to parse decision response JSON: {e}, input: {json_str}");
            serde_json::Value::Null
        }
    };

    let action_str = parsed
        .get("action")
        .and_then(|v| v.as_str())
        // Missing/non-string action falls through to default ADD below.
        .unwrap_or("")
        .to_uppercase();

    match action_str.as_str() {
        "NOOP" => Ok(MemoryAction::Noop),
        "ADD" => Ok(MemoryAction::Add),
        "UPDATE" => {
            // Read existing_id as string OR number (LLM may return either)
            let existing_id = parsed.get("existing_id").and_then(|v| {
                v.as_str()
                    .map(String::from)
                    .or_else(|| v.as_u64().map(|n| n.to_string()))
            });

            // Validate the ID exists in our candidates (UUID match)
            if let Some(ref id) = existing_id {
                let valid = existing_memories.iter().any(|m| m.id.to_string() == *id);
                if valid {
                    return Ok(MemoryAction::Update {
                        existing_id: id.clone(),
                    });
                }
            }

            // Try interpreting as a 1-based index (LLM may return "1" instead of the UUID)
            if let Some(ref id_str) = existing_id {
                if let Ok(index) = id_str.parse::<usize>() {
                    if index >= 1 && index <= existing_memories.len() {
                        return Ok(MemoryAction::Update {
                            existing_id: existing_memories[index - 1].id.to_string(),
                        });
                    }
                }
            }

            // If ID is invalid/missing, fall back to ADD rather than blindly
            // updating the first candidate — let consolidation merge later.
            Ok(MemoryAction::Add)
        }
        // Unparseable or unknown action — default to ADD (safe: may duplicate,
        // but at least new information is not silently dropped)
        _ => Ok(MemoryAction::Add),
    }
}

/// Parse the LLM's JSON response into an ExtractionResult.
///
/// Handles two formats:
/// - New: `{"memories": [...], "relations": [...]}`
/// - Legacy: `[...]` (array of memory items, no relations)
fn parse_llm_extraction_response(
    text: &str,
) -> librefang_types::error::LibreFangResult<ExtractionResult> {
    use librefang_types::memory::RelationTriple;

    // Strip markdown code blocks (case-insensitive, handles leading text)
    let json_str = strip_code_block(text);

    let parsed: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(val) => val,
        Err(e) => {
            // Weak / local models routinely reply in prose instead of JSON.
            // Extraction is best-effort — falling back to Null just skips
            // this turn's memory/relation updates rather than failing the
            // conversation, so WARN is overkill.
            tracing::debug!("Failed to parse extraction response JSON: {e}, input: {json_str}");
            serde_json::Value::Null
        }
    };

    // Extract memories (from object or legacy array)
    let memory_items = if let Some(arr) = parsed.get("memories").and_then(|v| v.as_array()) {
        arr.clone()
    } else if let Some(arr) = parsed.as_array() {
        arr.clone()
    } else {
        Vec::new()
    };

    let memories: Vec<MemoryItem> = memory_items
        .into_iter()
        .filter_map(|item| {
            let content = item.get("content")?.as_str()?;
            let content = if content.len() > MAX_MEMORY_CONTENT_LENGTH {
                tracing::warn!(
                    "Memory content too long ({} chars), truncating to {}",
                    content.len(),
                    MAX_MEMORY_CONTENT_LENGTH
                );
                let cutoff = content
                    .char_indices()
                    .nth(MAX_MEMORY_CONTENT_LENGTH)
                    .map(|(i, _)| i)
                    .unwrap_or(content.len());
                &content[..cutoff]
            } else {
                content
            };
            let content = content.to_string();
            let category = item
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("general")
                .to_string();
            let level = match item.get("level").and_then(|v| v.as_str()) {
                Some("user") => MemoryLevel::User,
                Some("agent") => MemoryLevel::Agent,
                _ => MemoryLevel::Session,
            };

            let mut metadata = std::collections::HashMap::new();
            metadata.insert("extracted_by".to_string(), serde_json::json!("llm"));

            Some(MemoryItem {
                id: uuid::Uuid::new_v4().to_string(),
                content,
                level,
                category: Some(category),
                metadata,
                created_at: chrono::Utc::now(),
                source: None,
                confidence: None,
                accessed_at: None,
                access_count: None,
                agent_id: None,
            })
        })
        .collect();

    // Extract relations (knowledge graph triples)
    let relations: Vec<RelationTriple> = parsed
        .get("relations")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    Some(RelationTriple {
                        subject: item.get("subject")?.as_str()?.to_string(),
                        subject_type: item
                            .get("subject_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("concept")
                            .to_string(),
                        relation: item.get("relation")?.as_str()?.to_string(),
                        object: item.get("object")?.as_str()?.to_string(),
                        object_type: item
                            .get("object_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("concept")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ExtractionResult {
        has_content: !memories.is_empty() || !relations.is_empty(),
        memories,
        relations,
        trigger: "llm_extractor".to_string(),
        conflicts: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockEmbeddingDriver {
        result: Result<Vec<f32>, crate::embedding::EmbeddingError>,
    }

    #[async_trait::async_trait]
    impl crate::embedding::EmbeddingDriver for MockEmbeddingDriver {
        async fn embed(
            &self,
            _texts: &[&str],
        ) -> Result<Vec<Vec<f32>>, crate::embedding::EmbeddingError> {
            match &self.result {
                Ok(v) => Ok(vec![v.clone()]),
                Err(e) => Err(crate::embedding::EmbeddingError::Api {
                    status: 500,
                    message: e.to_string(),
                }),
            }
        }
        fn dimensions(&self) -> usize {
            3
        }
    }

    struct AlwaysFailingLlmDriver;

    #[async_trait::async_trait]
    impl crate::llm_driver::LlmDriver for AlwaysFailingLlmDriver {
        async fn complete(
            &self,
            _request: crate::llm_driver::CompletionRequest,
        ) -> Result<crate::llm_driver::CompletionResponse, crate::llm_driver::LlmError> {
            Err(crate::llm_driver::LlmError::Api {
                status: 500,
                message: "mock failure".into(),
                code: None,
            })
        }
        fn is_configured(&self) -> bool {
            false
        }
    }

    struct CannedLlmDriver {
        response: String,
    }

    #[async_trait::async_trait]
    impl crate::llm_driver::LlmDriver for CannedLlmDriver {
        async fn complete(
            &self,
            _request: crate::llm_driver::CompletionRequest,
        ) -> Result<crate::llm_driver::CompletionResponse, crate::llm_driver::LlmError> {
            use librefang_types::message::{ContentBlock, StopReason, TokenUsage};
            Ok(crate::llm_driver::CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: self.response.clone(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                },
                actual_provider: None,
            })
        }
        fn is_configured(&self) -> bool {
            true
        }
    }

    fn make_memory_item(content: &str) -> MemoryItem {
        MemoryItem {
            id: uuid::Uuid::new_v4().to_string(),
            content: content.to_string(),
            level: MemoryLevel::User,
            category: Some("test".to_string()),
            metadata: std::collections::HashMap::new(),
            created_at: chrono::Utc::now(),
            source: None,
            confidence: None,
            accessed_at: None,
            access_count: None,
            agent_id: None,
        }
    }

    fn make_fragment(
        id: librefang_types::memory::MemoryId,
    ) -> librefang_types::memory::MemoryFragment {
        use librefang_types::memory::MemorySource;
        librefang_types::memory::MemoryFragment {
            id,
            agent_id: librefang_types::agent::AgentId::new(),
            content: "test content".to_string(),
            embedding: None,
            metadata: std::collections::HashMap::new(),
            source: MemorySource::Conversation,
            confidence: 1.0,
            created_at: chrono::Utc::now(),
            accessed_at: chrono::Utc::now(),
            access_count: 0,
            scope: "user_memory".to_string(),
            image_url: None,
            image_embedding: None,
            modality: Default::default(),
        }
    }

    #[test]
    fn test_disabled_when_both_off() {
        let substrate = librefang_memory::MemorySubstrate::open_in_memory(0.1).unwrap();
        let config = ProactiveMemoryConfig {
            auto_memorize: false,
            auto_retrieve: false,
            ..Default::default()
        };
        assert!(init_proactive_memory(Arc::new(substrate), config).is_none());
    }

    #[test]
    fn test_enabled_by_default() {
        let substrate = librefang_memory::MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = init_proactive_memory_with_defaults(Arc::new(substrate));
        assert!(store.is_some());
    }

    #[test]
    fn test_parse_llm_extraction_json() {
        let json =
            r#"[{"content": "User prefers Rust", "category": "user_preference", "level": "user"}]"#;
        let result = parse_llm_extraction_response(json).unwrap();
        assert!(result.has_content);
        assert_eq!(result.memories.len(), 1);
        assert_eq!(result.memories[0].content, "User prefers Rust");
        assert_eq!(
            result.memories[0].category,
            Some("user_preference".to_string())
        );
        assert_eq!(result.memories[0].level, MemoryLevel::User);
    }

    #[test]
    fn test_parse_llm_extraction_code_block() {
        let json = "```json\n[{\"content\": \"Works at Acme\", \"category\": \"important_fact\", \"level\": \"user\"}]\n```";
        let result = parse_llm_extraction_response(json).unwrap();
        assert!(result.has_content);
        assert_eq!(result.memories.len(), 1);
        assert_eq!(result.memories[0].content, "Works at Acme");
    }

    #[test]
    fn test_parse_llm_extraction_empty() {
        let result = parse_llm_extraction_response("[]").unwrap();
        assert!(!result.has_content);
        assert!(result.memories.is_empty());
    }

    #[test]
    fn test_parse_llm_extraction_invalid() {
        let result = parse_llm_extraction_response("not json at all").unwrap();
        assert!(!result.has_content);
        assert!(result.memories.is_empty());
    }

    #[test]
    fn test_parse_llm_extraction_levels() {
        let json = r#"[
            {"content": "a", "level": "user"},
            {"content": "b", "level": "session"},
            {"content": "c", "level": "agent"},
            {"content": "d"}
        ]"#;
        let result = parse_llm_extraction_response(json).unwrap();
        assert_eq!(result.memories.len(), 4);
        assert_eq!(result.memories[0].level, MemoryLevel::User);
        assert_eq!(result.memories[1].level, MemoryLevel::Session);
        assert_eq!(result.memories[2].level, MemoryLevel::Agent);
        assert_eq!(result.memories[3].level, MemoryLevel::Session); // default
    }

    #[test]
    fn test_parse_llm_extraction_new_format_with_relations() {
        let json = r#"{
            "memories": [
                {"content": "User prefers Rust", "category": "user_preference", "level": "user"}
            ],
            "relations": [
                {"subject": "User", "subject_type": "person", "relation": "prefers", "object": "Rust", "object_type": "tool"}
            ]
        }"#;
        let result = parse_llm_extraction_response(json).unwrap();
        assert!(result.has_content);
        assert_eq!(result.memories.len(), 1);
        assert_eq!(result.memories[0].content, "User prefers Rust");
        assert_eq!(result.relations.len(), 1);
        assert_eq!(result.relations[0].subject, "User");
        assert_eq!(result.relations[0].relation, "prefers");
        assert_eq!(result.relations[0].object, "Rust");
        assert_eq!(result.relations[0].object_type, "tool");
    }

    #[test]
    fn test_parse_llm_extraction_relations_only() {
        let json = r#"{
            "memories": [],
            "relations": [
                {"subject": "Alice", "subject_type": "person", "relation": "works_at", "object": "Google", "object_type": "organization"}
            ]
        }"#;
        let result = parse_llm_extraction_response(json).unwrap();
        assert!(result.has_content); // relations count as content
        assert!(result.memories.is_empty());
        assert_eq!(result.relations.len(), 1);
    }

    #[test]
    fn test_parse_decision_response_add() {
        let fragments = vec![];
        let result = parse_decision_response(r#"{"action": "ADD"}"#, &fragments).unwrap();
        assert_eq!(result, MemoryAction::Add);
    }

    #[test]
    fn test_parse_decision_response_noop() {
        let fragments = vec![];
        let result = parse_decision_response(r#"{"action": "NOOP"}"#, &fragments).unwrap();
        assert_eq!(result, MemoryAction::Noop);
    }

    #[test]
    fn test_parse_decision_response_update() {
        use librefang_types::memory::{MemoryFragment, MemoryId, MemorySource};
        let mem_id = MemoryId::new();
        let fragments = vec![MemoryFragment {
            id: mem_id,
            agent_id: librefang_types::agent::AgentId::new(),
            content: "Old content".to_string(),
            embedding: None,
            metadata: std::collections::HashMap::new(),
            source: MemorySource::Conversation,
            confidence: 1.0,
            created_at: chrono::Utc::now(),
            accessed_at: chrono::Utc::now(),
            access_count: 0,
            scope: "user_memory".to_string(),
            image_url: None,
            image_embedding: None,
            modality: Default::default(),
        }];
        let json = format!(r#"{{"action": "UPDATE", "existing_id": "{}"}}"#, mem_id);
        let result = parse_decision_response(&json, &fragments).unwrap();
        assert_eq!(
            result,
            MemoryAction::Update {
                existing_id: mem_id.to_string()
            }
        );
    }

    #[test]
    fn test_parse_decision_response_invalid_defaults_to_add() {
        let fragments = vec![];
        let result = parse_decision_response("garbage", &fragments).unwrap();
        assert_eq!(result, MemoryAction::Add);
    }

    #[test]
    fn test_parse_decision_response_add_case_insensitive() {
        let fragments = vec![];
        for action in &["ADD", "add", "Add"] {
            let input = format!(r#"{{"action": "{}"}}"#, action);
            let result = parse_decision_response(&input, &fragments).unwrap();
            assert_eq!(result, MemoryAction::Add);
        }
    }

    #[test]
    fn test_strip_code_block_plain_returns_unchanged() {
        assert_eq!(
            strip_code_block(r#"{"action":"ADD"}"#),
            r#"{"action":"ADD"}"#
        );
    }

    #[test]
    fn test_strip_code_block_case_insensitive_tags() {
        for tag in &["JSON", "Json", "jsonc", "Jsonc"] {
            let input = format!("```{}\n{{}}\n```", tag);
            assert_eq!(strip_code_block(&input), "{}");
        }
    }

    #[test]
    fn test_strip_code_block_leading_text() {
        let input = "Here is the result:\n```json\n{\"action\":\"ADD\"}\n```";
        assert_eq!(strip_code_block(input), "{\"action\":\"ADD\"}");
    }

    #[test]
    fn test_strip_code_block_no_newline_after_tag() {
        let input = "```json{\"a\":1}```";
        assert_eq!(strip_code_block(input), r#"{"a":1}"#);
    }

    #[test]
    fn test_strip_code_block_empty() {
        assert_eq!(strip_code_block(""), "");
    }

    #[test]
    fn test_strip_code_block_nested_fences() {
        let input = "```json\n{\"nested\": \"```inside```\"}\n```";
        let result = strip_code_block(input);
        assert!(result.contains("inside"));
    }

    #[test]
    fn test_parse_decision_update_1based_index() {
        use librefang_types::memory::MemoryId;
        let id1 = MemoryId::new();
        let id2 = MemoryId::new();
        let fragments = vec![make_fragment(id1), make_fragment(id2)];
        let input = r#"{"action": "UPDATE", "existing_id": "2"}"#;
        let result = parse_decision_response(input, &fragments).unwrap();
        assert_eq!(
            result,
            MemoryAction::Update {
                existing_id: id2.to_string()
            }
        );
    }

    #[test]
    fn test_parse_decision_update_nonexistent_uuid_falls_to_add() {
        use librefang_types::memory::MemoryId;
        let fragments = vec![make_fragment(MemoryId::new())];
        let input =
            r#"{"action": "UPDATE", "existing_id": "00000000-0000-0000-0000-000000000000"}"#;
        let result = parse_decision_response(input, &fragments).unwrap();
        assert_eq!(result, MemoryAction::Add);
    }

    #[test]
    fn test_parse_decision_update_missing_existing_id_falls_to_add() {
        use librefang_types::memory::MemoryId;
        let fragments = vec![make_fragment(MemoryId::new())];
        let input = r#"{"action": "UPDATE"}"#;
        let result = parse_decision_response(input, &fragments).unwrap();
        assert_eq!(result, MemoryAction::Add);
    }

    #[test]
    fn test_parse_decision_update_in_code_block() {
        use librefang_types::memory::MemoryId;
        let id = MemoryId::new();
        let fragments = vec![make_fragment(id)];
        let input = format!(
            "```json\n{{\"action\": \"UPDATE\", \"existing_id\": \"{}\"}}\n```",
            id
        );
        let result = parse_decision_response(&input, &fragments).unwrap();
        assert_eq!(
            result,
            MemoryAction::Update {
                existing_id: id.to_string()
            }
        );
    }

    #[test]
    fn test_parse_decision_update_numeric_existing_id() {
        use librefang_types::memory::MemoryId;
        let id = MemoryId::new();
        let fragments = vec![make_fragment(id)];
        let input = r#"{"action": "UPDATE", "existing_id": 1}"#;
        let result = parse_decision_response(input, &fragments).unwrap();
        assert_eq!(
            result,
            MemoryAction::Update {
                existing_id: id.to_string()
            }
        );
    }

    #[test]
    fn test_parse_decision_update_index_out_of_bounds_falls_to_add() {
        use librefang_types::memory::MemoryId;
        let fragments = vec![
            make_fragment(MemoryId::new()),
            make_fragment(MemoryId::new()),
        ];
        for idx in &["0", "5", "999"] {
            let input = format!(r#"{{"action": "UPDATE", "existing_id": "{}"}}"#, idx);
            let result = parse_decision_response(&input, &fragments).unwrap();
            assert_eq!(
                result,
                MemoryAction::Add,
                "index {} should fall back to ADD",
                idx
            );
        }
    }

    #[test]
    fn test_parse_decision_unknown_action_defaults_to_add() {
        let fragments = vec![];
        for action in &["DELETE", "SKIP", "MERGE", ""] {
            let input = format!(r#"{{"action": "{}"}}"#, action);
            let result = parse_decision_response(&input, &fragments).unwrap();
            assert_eq!(
                result,
                MemoryAction::Add,
                "action '{}' should default to ADD",
                action
            );
        }
    }

    #[test]
    fn test_parse_decision_empty_object_defaults_to_add() {
        let fragments = vec![];
        let result = parse_decision_response("{}", &fragments).unwrap();
        assert_eq!(result, MemoryAction::Add);
    }

    #[test]
    fn test_parse_decision_noop_in_code_block() {
        let fragments = vec![];
        let input = "```json\n{\"action\": \"NOOP\"}\n```";
        let result = parse_decision_response(input, &fragments).unwrap();
        assert_eq!(result, MemoryAction::Noop);
    }

    #[test]
    fn test_parse_extraction_content_truncation_over_2000() {
        let long_content = "A".repeat(3000);
        let json = format!(r#"[{{"content": "{}", "level": "user"}}]"#, long_content);
        let result = parse_llm_extraction_response(&json).unwrap();
        assert_eq!(result.memories.len(), 1);
        assert_eq!(result.memories[0].content.len(), MAX_MEMORY_CONTENT_LENGTH);
    }

    #[test]
    fn test_parse_extraction_content_exactly_2000_not_truncated() {
        let content = "A".repeat(2000);
        let json = format!(r#"[{{"content": "{}", "level": "user"}}]"#, content);
        let result = parse_llm_extraction_response(&json).unwrap();
        assert_eq!(result.memories[0].content.len(), 2000);
        assert_eq!(result.memories[0].content, content);
    }

    #[test]
    fn test_parse_extraction_content_truncation_utf8_boundary() {
        let content = "ą".repeat(2500);
        let json = format!(r#"[{{"content": "{}", "level": "user"}}]"#, content);
        let result = parse_llm_extraction_response(&json).unwrap();
        assert!(result.memories[0].content.chars().count() <= MAX_MEMORY_CONTENT_LENGTH);
        // Verify valid UTF-8 — no panics
        assert!(std::str::from_utf8(result.memories[0].content.as_bytes()).is_ok());
    }

    #[test]
    fn test_parse_extraction_default_category() {
        let json = r#"[{"content": "test", "level": "user"}]"#;
        let result = parse_llm_extraction_response(json).unwrap();
        assert_eq!(result.memories[0].category, Some("general".to_string()));
    }

    #[test]
    fn test_parse_extraction_relation_default_types() {
        let json = r#"{
            "memories": [],
            "relations": [
                {"subject": "X", "relation": "relates_to", "object": "Y"}
            ]
        }"#;
        let result = parse_llm_extraction_response(json).unwrap();
        assert_eq!(result.relations[0].subject_type, "concept");
        assert_eq!(result.relations[0].object_type, "concept");
    }

    #[test]
    fn test_parse_extraction_relation_missing_required_field_skipped() {
        let json = r#"{
            "memories": [],
            "relations": [
                {"subject": "A", "object": "B"},
                {"subject": "B", "relation": "knows", "object": "C"}
            ]
        }"#;
        let result = parse_llm_extraction_response(json).unwrap();
        assert_eq!(result.relations.len(), 1);
        assert_eq!(result.relations[0].subject, "B");
    }

    #[test]
    fn test_parse_extraction_memory_missing_content_skipped() {
        let json = r#"[{"category": "x", "level": "user"}, {"content": "valid", "level": "user"}]"#;
        let result = parse_llm_extraction_response(json).unwrap();
        assert_eq!(result.memories.len(), 1);
        assert_eq!(result.memories[0].content, "valid");
    }

    #[test]
    fn test_parse_extraction_new_format_in_code_block() {
        let input = r#"```json
{
    "memories": [{"content": "test", "level": "user"}],
    "relations": [{"subject": "A", "relation": "r", "object": "B"}]
}
```"#;
        let result = parse_llm_extraction_response(input).unwrap();
        assert_eq!(result.memories.len(), 1);
        assert_eq!(result.relations.len(), 1);
    }

    #[test]
    fn test_parse_extraction_empty_string() {
        let result = parse_llm_extraction_response("").unwrap();
        assert!(!result.has_content);
        assert!(result.memories.is_empty());
        assert!(result.relations.is_empty());
    }

    // --- format_context tests ---

    #[test]
    fn test_format_context_empty() {
        let extractor = LlmMemoryExtractor::new(
            Arc::new(CannedLlmDriver {
                response: String::new(),
            }),
            "test".to_string(),
        );
        assert!(extractor.format_context(&[]).is_empty());
    }

    #[test]
    fn test_format_context_single_memory() {
        let extractor = LlmMemoryExtractor::new(
            Arc::new(CannedLlmDriver {
                response: String::new(),
            }),
            "test".to_string(),
        );
        let ctx = extractor.format_context(&[make_memory_item("Prefers Rust")]);
        assert!(ctx.contains("- Prefers Rust"));
        assert!(ctx.contains("understanding of this person"));
    }

    #[test]
    fn test_format_context_multiple_memories() {
        let extractor = LlmMemoryExtractor::new(
            Arc::new(CannedLlmDriver {
                response: String::new(),
            }),
            "test".to_string(),
        );
        let items = vec![
            make_memory_item("First"),
            make_memory_item("Second"),
            make_memory_item("Third"),
        ];
        let ctx = extractor.format_context(&items);
        assert!(ctx.contains("- First"));
        assert!(ctx.contains("- Second"));
        assert!(ctx.contains("- Third"));
    }

    #[test]
    fn test_format_context_no_recite_phrases() {
        let extractor = LlmMemoryExtractor::new(
            Arc::new(CannedLlmDriver {
                response: String::new(),
            }),
            "test".to_string(),
        );
        let ctx = extractor.format_context(&[make_memory_item("test")]);
        // Template mentions these as things NOT to do — verify the instruction is present
        assert!(ctx.contains("NEVER say"));
        assert!(ctx.contains("based on my memory"));
        // But the memory content itself should appear as a bullet, not as a recitation
        assert!(ctx.contains("- test"));
    }

    // --- EmbeddingBridge tests ---

    #[test]
    fn test_embedding_bridge_passes_through() {
        use librefang_memory::proactive::EmbeddingFn;
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let driver = Arc::new(MockEmbeddingDriver {
                result: Ok(vec![0.1, 0.2, 0.3]),
            });
            let bridge = EmbeddingBridge(driver);
            let result: Vec<f32> = bridge.embed_one("hello").await.unwrap();
            assert_eq!(result, vec![0.1, 0.2, 0.3]);
        });
    }

    #[test]
    fn test_embedding_bridge_maps_error() {
        use librefang_memory::proactive::EmbeddingFn;
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let driver = Arc::new(MockEmbeddingDriver {
                result: Err(crate::embedding::EmbeddingError::Parse("fail".into())),
            });
            let bridge = EmbeddingBridge(driver);
            let result = bridge.embed_one("hello").await;
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.to_string().contains("Embedding failed"));
        });
    }

    // --- init_proactive_memory_full tests ---

    #[test]
    fn test_init_full_with_llm_driver() {
        let substrate = librefang_memory::MemorySubstrate::open_in_memory(0.1).unwrap();
        let config = ProactiveMemoryConfig {
            auto_retrieve: true,
            auto_memorize: false,
            ..Default::default()
        };
        let llm = Arc::new(CannedLlmDriver {
            response: r#"{"memories":[],"relations":[]}"#.into(),
        });
        let result = init_proactive_memory_full(
            Arc::new(substrate),
            config,
            Some((
                llm as Arc<dyn crate::llm_driver::LlmDriver>,
                "test-model".to_string(),
            )),
            None,
        );
        assert!(result.is_some());
    }

    #[test]
    fn test_init_full_with_embedding_driver() {
        let substrate = librefang_memory::MemorySubstrate::open_in_memory(0.1).unwrap();
        let config = ProactiveMemoryConfig {
            auto_retrieve: false,
            auto_memorize: true,
            ..Default::default()
        };
        let emb = Arc::new(MockEmbeddingDriver {
            result: Ok(vec![0.1, 0.2, 0.3]),
        });
        let result = init_proactive_memory_full(
            Arc::new(substrate),
            config,
            None,
            Some(emb as Arc<dyn crate::embedding::EmbeddingDriver + Send + Sync>),
        );
        assert!(result.is_some());
    }

    #[test]
    fn test_init_full_with_both_llm_and_embedding() {
        let substrate = librefang_memory::MemorySubstrate::open_in_memory(0.1).unwrap();
        let config = ProactiveMemoryConfig {
            auto_retrieve: true,
            auto_memorize: true,
            ..Default::default()
        };
        let llm = Arc::new(CannedLlmDriver {
            response: r#"{"memories":[],"relations":[]}"#.into(),
        });
        let emb = Arc::new(MockEmbeddingDriver {
            result: Ok(vec![0.1, 0.2, 0.3]),
        });
        let result = init_proactive_memory_full(
            Arc::new(substrate),
            config,
            Some((
                llm as Arc<dyn crate::llm_driver::LlmDriver>,
                "test-model".to_string(),
            )),
            Some(emb as Arc<dyn crate::embedding::EmbeddingDriver + Send + Sync>),
        );
        assert!(result.is_some());
    }

    // --- decide_action edge case ---

    #[test]
    fn test_parse_decision_update_numeric_id_fallback_to_add() {
        use librefang_types::memory::MemoryId;
        let fragments = vec![make_fragment(MemoryId::new())];
        let input = r#"{"action": "UPDATE", "existing_id": 999}"#;
        let result = parse_decision_response(input, &fragments).unwrap();
        assert_eq!(result, MemoryAction::Add);
    }

    // --- LLM failure path tests ---

    #[test]
    fn test_decide_action_llm_failure_falls_back_to_heuristic() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // decide_action catches LLM errors and falls back to DefaultMemoryExtractor
            // heuristic rather than bubbling up — verifies graceful degradation.
            let extractor =
                LlmMemoryExtractor::new(Arc::new(AlwaysFailingLlmDriver), "test-model".to_string());
            let new_mem = make_memory_item("new fact");
            let existing = vec![make_fragment(librefang_types::memory::MemoryId::new())];
            let result = extractor.decide_action(&new_mem, &existing).await;
            assert!(result.is_ok(), "LLM failure should fall back, not error");
        });
    }

    #[test]
    fn test_decide_action_empty_existing_returns_add_without_llm_call() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // AlwaysFailingLlmDriver would error if called — proves the short-circuit works
            let extractor =
                LlmMemoryExtractor::new(Arc::new(AlwaysFailingLlmDriver), "test-model".to_string());
            let new_mem = make_memory_item("first fact");
            let result = extractor.decide_action(&new_mem, &[]).await.unwrap();
            assert_eq!(result, MemoryAction::Add);
        });
    }
}
