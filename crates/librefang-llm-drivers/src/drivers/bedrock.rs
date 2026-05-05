//! AWS Bedrock Converse API driver.
//!
//! Authenticates via Bedrock API Keys (`AWS_BEARER_TOKEN_BEDROCK`) — Bearer token.
//!
//! Note on trace-header placement: this driver uses simple Bearer token auth
//! (not SigV4 request signing). The `Authorization: Bearer` header is set
//! directly on the `reqwest::RequestBuilder`; the `x-librefang-*` trace
//! headers are appended via `.headers(map)` on the same builder, which means
//! they travel alongside the Bearer token with no signing scope involved.
//! If this driver is ever migrated to full SigV4 signing (where the signer
//! inspects the `HeaderMap` before hashing), trace headers would need to be
//! added AFTER signing — but that migration has not happened.

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
use async_trait::async_trait;
use librefang_types::message::{ContentBlock, MessageContent, Role, StopReason, TokenUsage};
use librefang_types::tool::{ToolCall, ToolDefinition};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tracing::{debug, warn};
use zeroize::Zeroizing;

// ── Driver ───────────────────────────────────────────────────────────────────

/// AWS Bedrock Converse API driver (bearer token auth).
pub struct BedrockDriver {
    api_key: Zeroizing<String>,
    region: String,
    client: reqwest::Client,
    /// Whether to emit the three `x-librefang-{agent,session,step}-id` trace
    /// headers on outbound requests. Mirrors
    /// `KernelConfig.telemetry.emit_caller_trace_headers`; when `false`, no
    /// trace headers are emitted regardless of whether `CompletionRequest`'s
    /// caller-id fields are populated.
    emit_caller_trace_headers: bool,
    /// When set, replaces `https://bedrock-runtime.{region}.amazonaws.com` as
    /// the URL base. Used only in tests to redirect requests to a mock server.
    base_url_override: Option<String>,
}

impl BedrockDriver {
    /// Create a driver using a Bedrock bearer token.
    ///
    /// Resolves from `bedrock_api_key` argument first, then `AWS_BEARER_TOKEN_BEDROCK` env var.
    /// Returns an error if neither is set.
    pub fn new_with_credentials(
        bedrock_api_key: Option<String>,
        region: Option<String>,
    ) -> Result<Self, LlmError> {
        let api_key = bedrock_api_key
            .filter(|k| !k.is_empty())
            .or_else(|| std::env::var("AWS_BEARER_TOKEN_BEDROCK").ok())
            .ok_or_else(|| LlmError::MissingApiKey("Set AWS_BEARER_TOKEN_BEDROCK".to_string()))?;

        let resolved_region = region
            .filter(|r| !r.is_empty())
            .or_else(|| std::env::var("AWS_REGION").ok())
            .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok())
            .unwrap_or_else(|| "us-east-1".to_string());

        Ok(Self {
            api_key: Zeroizing::new(api_key),
            region: resolved_region,
            client: librefang_http::proxied_client(),
            emit_caller_trace_headers: true,
            base_url_override: None,
        })
    }

    /// Override the trace-header emission flag (mirrors
    /// `KernelConfig.telemetry.emit_caller_trace_headers`). Default is `true`,
    /// meaning the three `x-librefang-{agent,session,step}-id` headers are
    /// emitted on every request that has those fields populated. Pass `false`
    /// to suppress them entirely.
    pub fn with_emit_caller_trace_headers(mut self, emit: bool) -> Self {
        self.emit_caller_trace_headers = emit;
        self
    }

    fn build_endpoint(&self, model: &str) -> String {
        if let Some(ref base) = self.base_url_override {
            return format!("{}/model/{}/converse", base, model);
        }
        format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/converse",
            self.region, model
        )
    }

    /// Test-only constructor that points the driver at a custom base URL
    /// (e.g. a `wiremock::MockServer`) instead of the real AWS endpoint.
    /// Requests go to `{base_url}/model/{model}/converse`, which wiremock
    /// intercepts via `path_regex(r"^/model/.*/converse$")`.
    #[doc(hidden)]
    pub fn new_for_test(api_key: String, base_url: String) -> Self {
        Self {
            api_key: Zeroizing::new(api_key),
            region: "us-east-1".to_string(),
            client: librefang_http::proxied_client(),
            emit_caller_trace_headers: true,
            base_url_override: Some(base_url),
        }
    }
}

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConverseRequest {
    messages: Vec<BedrockMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<BedrockTextBlock>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inference_config: Option<InferenceConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_config: Option<BedrockToolConfig>,
}

#[derive(Debug, Serialize)]
struct BedrockMessage {
    role: String,
    content: Vec<BedrockContentBlock>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum BedrockContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        #[serde(rename = "toolUse")]
        tool_use: BedrockToolUse,
    },
    ToolResult {
        #[serde(rename = "toolResult")]
        tool_result: BedrockToolResult,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BedrockToolUse {
    tool_use_id: String,
    name: String,
    input: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BedrockToolResult {
    tool_use_id: String,
    content: Vec<BedrockTextBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BedrockTextBlock {
    text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InferenceConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BedrockToolConfig {
    tools: Vec<BedrockToolDef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct BedrockToolDef {
    #[serde(rename = "toolSpec")]
    tool_spec: BedrockToolSpec,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BedrockToolSpec {
    name: String,
    description: String,
    input_schema: BedrockInputSchema,
}

#[derive(Debug, Serialize)]
struct BedrockInputSchema {
    json: serde_json::Value,
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConverseResponse {
    output: ConverseOutput,
    stop_reason: String,
    usage: BedrockUsage,
}

#[derive(Debug, Deserialize)]
struct ConverseOutput {
    message: BedrockResponseMessage,
}

#[derive(Debug, Deserialize)]
struct BedrockResponseMessage {
    #[allow(dead_code)]
    role: String,
    content: Vec<BedrockResponseContent>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum BedrockResponseContent {
    ToolUse {
        #[serde(rename = "toolUse")]
        tool_use: BedrockResponseToolUse,
    },
    Text {
        text: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BedrockResponseToolUse {
    tool_use_id: String,
    name: String,
    input: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BedrockUsage {
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct BedrockErrorResponse {
    message: String,
}

// ── Conversion helpers ────────────────────────────────────────────────────────

fn convert_messages(
    messages: &[librefang_types::message::Message],
    system: &Option<String>,
) -> (Vec<BedrockMessage>, Option<Vec<BedrockTextBlock>>) {
    let system_blocks = extract_system(messages, system);
    let mut bedrock_messages = Vec::new();

    for msg in messages {
        if msg.role == Role::System {
            continue;
        }
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => continue,
        };
        let content = convert_message_content(&msg.content);
        if !content.is_empty() {
            bedrock_messages.push(BedrockMessage {
                role: role.to_string(),
                content,
            });
        }
    }

    validate_bedrock_tool_pairing(&mut bedrock_messages);
    (bedrock_messages, system_blocks)
}

fn extract_system(
    messages: &[librefang_types::message::Message],
    system: &Option<String>,
) -> Option<Vec<BedrockTextBlock>> {
    let text = system.clone().or_else(|| {
        messages.iter().find_map(|m| {
            if m.role == Role::System {
                match &m.content {
                    MessageContent::Text(t) => Some(t.clone()),
                    MessageContent::Blocks(blocks) => blocks.iter().find_map(|b| {
                        if let ContentBlock::Text { text, .. } = b {
                            Some(text.clone())
                        } else {
                            None
                        }
                    }),
                }
            } else {
                None
            }
        })
    })?;
    Some(vec![BedrockTextBlock { text }])
}

fn convert_message_content(content: &MessageContent) -> Vec<BedrockContentBlock> {
    match content {
        MessageContent::Text(text) => vec![BedrockContentBlock::Text { text: text.clone() }],
        MessageContent::Blocks(blocks) => blocks.iter().filter_map(convert_content_block).collect(),
    }
}

fn convert_content_block(block: &ContentBlock) -> Option<BedrockContentBlock> {
    match block {
        ContentBlock::Text { text, .. } => Some(BedrockContentBlock::Text { text: text.clone() }),
        ContentBlock::ToolUse {
            id, name, input, ..
        } => Some(BedrockContentBlock::ToolUse {
            tool_use: BedrockToolUse {
                tool_use_id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            },
        }),
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
            ..
        } => Some(BedrockContentBlock::ToolResult {
            tool_result: BedrockToolResult {
                tool_use_id: tool_use_id.clone(),
                content: vec![BedrockTextBlock {
                    text: content.clone(),
                }],
                status: if *is_error {
                    Some("error".to_string())
                } else {
                    None
                },
            },
        }),
        // Image, ImageFile, Thinking, and Unknown are not supported — silently drop.
        ContentBlock::Image { .. }
        | ContentBlock::ImageFile { .. }
        | ContentBlock::Thinking { .. }
        | ContentBlock::Unknown => None,
    }
}

fn convert_tools(tools: &[ToolDefinition]) -> Option<BedrockToolConfig> {
    if tools.is_empty() {
        return None;
    }
    let bedrock_tools = tools
        .iter()
        .map(|t| BedrockToolDef {
            tool_spec: BedrockToolSpec {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: BedrockInputSchema {
                    json: t.input_schema.clone(),
                },
            },
        })
        .collect();
    Some(BedrockToolConfig {
        tools: bedrock_tools,
        tool_choice: Some(serde_json::json!({"auto": {}})),
    })
}

/// Returns the set of `tool_use_id`s present in the assistant message immediately
/// before `messages[j]`, or an empty set when `j == 0` or the preceding message is
/// not an assistant.
fn preceding_tool_use_ids(messages: &[BedrockMessage], j: usize) -> HashSet<String> {
    if j > 0 && messages[j - 1].role == "assistant" {
        messages[j - 1]
            .content
            .iter()
            .filter_map(|b| {
                if let BedrockContentBlock::ToolUse { tool_use } = b {
                    Some(tool_use.tool_use_id.clone())
                } else {
                    None
                }
            })
            .collect()
    } else {
        HashSet::new()
    }
}

/// Move toolResult blocks that ended up in the wrong user message back to the
/// correct position.
///
/// This happens when `session_repair` removes a blank assistant message (Phase 2e)
/// between two consecutive user messages and then merges them (Phase 3), which can
/// strand toolResult blocks from one assistant turn inside a user message that is
/// now adjacent to a *different* assistant.
///
/// For each stray toolResult (one whose `tool_use_id` is present in the global
/// toolUse map but NOT in the immediately-preceding assistant's toolUse set), the
/// block is extracted from its current user message and prepended to the user
/// message right after the assistant that owns the matching toolUse.  If no user
/// message exists at that position a new one is inserted.
///
/// After relocation any now-empty source user messages receive a placeholder text
/// block so Bedrock does not reject an empty content array.
///
/// Results whose `tool_use_id` does not appear in *any* assistant are truly orphaned
/// and are left in place for the subsequent cleanup pass to remove.
fn relocate_stray_tool_results(messages: &mut Vec<BedrockMessage>) {
    // Build map: tool_use_id -> index of the assistant message that owns it.
    let mut id_to_asst: HashMap<String, usize> = HashMap::new();
    for (i, msg) in messages.iter().enumerate() {
        if msg.role == "assistant" {
            for block in &msg.content {
                if let BedrockContentBlock::ToolUse { tool_use } = block {
                    id_to_asst.insert(tool_use.tool_use_id.clone(), i);
                }
            }
        }
    }

    // Find every toolResult that is in the wrong user message.
    // A result is "stray" when its tool_use_id IS in id_to_asst but does NOT
    // belong to the immediately-preceding assistant (assistant at j-1).
    let mut stray: Vec<(
        usize,  /* from_j */
        String, /* id */
        usize,  /* target asst k */
    )> = Vec::new();
    for j in 0..messages.len() {
        if messages[j].role != "user" {
            continue;
        }
        let preceding_ids = preceding_tool_use_ids(messages, j);

        for block in &messages[j].content {
            if let BedrockContentBlock::ToolResult { tool_result } = block {
                let id = &tool_result.tool_use_id;
                if !preceding_ids.contains(id) {
                    if let Some(&asst_k) = id_to_asst.get(id) {
                        stray.push((j, id.clone(), asst_k));
                    }
                    // else: no matching toolUse anywhere → truly orphaned; cleanup pass removes it
                }
            }
        }
    }

    if stray.is_empty() {
        return;
    }

    warn!(
        count = stray.len(),
        "Bedrock: relocating stray toolResult blocks to correct assistant turn"
    );

    // Extract stray blocks from their source user messages.
    let stray_from: HashSet<(usize, String)> =
        stray.iter().map(|(j, id, _)| (*j, id.clone())).collect();

    let mut extracted: HashMap<String, BedrockContentBlock> = HashMap::new();
    for (j, msg) in messages.iter_mut().enumerate() {
        if msg.role != "user" {
            continue;
        }
        let mut remaining = Vec::new();
        for block in msg.content.drain(..) {
            let stray_id = if let BedrockContentBlock::ToolResult { ref tool_result } = block {
                if stray_from.contains(&(j, tool_result.tool_use_id.clone())) {
                    Some(tool_result.tool_use_id.clone())
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(id) = stray_id {
                // Keep only the first occurrence of each id (second pass dedup handles rest).
                extracted.entry(id).or_insert(block);
            } else {
                remaining.push(block);
            }
        }
        msg.content = remaining;
    }

    // Group extracted blocks by their target assistant index.
    let mut for_asst: HashMap<usize, Vec<BedrockContentBlock>> = HashMap::new();
    for (_, id, asst_k) in stray {
        if let Some(block) = extracted.remove(&id) {
            for_asst.entry(asst_k).or_default().push(block);
        }
        // Duplicate id in stray vec (same id from multiple user messages): block
        // already consumed by remove() above — skip silently.
    }

    // Insert relocated blocks in reverse order of assistant index so that earlier
    // insertions do not shift the indices of later assistants that still need processing.
    let mut targets: Vec<usize> = for_asst.keys().cloned().collect();
    targets.sort_unstable_by(|a, b| b.cmp(a));

    for asst_k in targets {
        let blocks = for_asst.remove(&asst_k).unwrap();
        let target = asst_k + 1;

        if target < messages.len() && messages[target].role == "user" {
            // Prepend tool results before any existing content (results come first).
            let existing: Vec<BedrockContentBlock> = messages[target].content.drain(..).collect();
            messages[target].content = blocks;
            messages[target].content.extend(existing);
        } else {
            // Insert a new user message immediately after the assistant.
            messages.insert(
                target,
                BedrockMessage {
                    role: "user".to_string(),
                    content: blocks,
                },
            );
        }
    }

    // Replace any now-empty source user messages with a placeholder so Bedrock
    // does not reject an empty content array.
    for msg in messages.iter_mut() {
        if msg.role == "user" && msg.content.is_empty() {
            warn!("Bedrock: user message empty after toolResult relocation; inserting placeholder");
            msg.content.push(BedrockContentBlock::Text {
                text: "[prior tool results relocated]".to_string(),
            });
        }
    }
}

/// Enforce Bedrock's strict toolUse/toolResult pairing requirement.
///
/// Bedrock's Converse API requires that for each assistant message with N toolUse
/// blocks, the immediately following user message contains **exactly** N toolResult
/// blocks — one per toolUse ID. Mismatches cause a 400 error. The fix iterates
/// over every user message and enforces four invariants:
///
/// 0. Relocate stray toolResult blocks to their correct position first.
/// 1. Remove toolResult blocks whose ID is not in the preceding assistant's toolUse set.
/// 2. Deduplicate toolResult blocks — keep only the first occurrence of each ID
///    (duplicate IDs would inflate the count even though all IDs match).
/// 3. Insert a synthetic error result for any toolUse ID that has no matching result.
/// 4. If the user message becomes empty (e.g. it contained only stray results that
///    were removed), replace it with a placeholder text block so Bedrock does not
///    reject an empty content array or a conversation ending with an assistant message.
fn validate_bedrock_tool_pairing(messages: &mut Vec<BedrockMessage>) {
    // Phase 0: move any toolResult blocks that ended up next to the wrong assistant.
    relocate_stray_tool_results(messages);

    let n = messages.len();
    for j in 0..n {
        if messages[j].role != "user" {
            continue;
        }

        // Only process user messages that actually contain toolResult blocks.
        let has_results = messages[j]
            .content
            .iter()
            .any(|b| matches!(b, BedrockContentBlock::ToolResult { .. }));
        if !has_results {
            continue;
        }

        // toolUse IDs the immediately-preceding assistant expects results for.
        let tool_use_ids = preceding_tool_use_ids(messages, j);

        // Step 1: remove toolResult blocks whose ID is not in the assistant's toolUse set.
        let mut removed_count = 0usize;
        messages[j].content.retain(|b| match b {
            BedrockContentBlock::ToolResult { tool_result } => {
                if tool_use_ids.contains(&tool_result.tool_use_id) {
                    true
                } else {
                    removed_count += 1;
                    false
                }
            }
            _ => true,
        });
        if removed_count > 0 {
            warn!(
                removed = removed_count,
                user_idx = j,
                "Bedrock: removed toolResult blocks not matching preceding assistant toolUse"
            );
        }

        // Step 2: deduplicate — keep only the first toolResult block per tool_use_id.
        // After this retain, `seen` holds exactly the surviving result IDs.
        let mut seen: HashSet<String> = HashSet::new();
        let mut dupes_removed = 0usize;
        messages[j].content.retain(|b| match b {
            BedrockContentBlock::ToolResult { tool_result } => {
                if seen.insert(tool_result.tool_use_id.clone()) {
                    true
                } else {
                    dupes_removed += 1;
                    false
                }
            }
            _ => true,
        });
        if dupes_removed > 0 {
            warn!(
                duplicates_removed = dupes_removed,
                user_idx = j,
                "Bedrock: deduplicated toolResult blocks with repeated tool_use_id"
            );
        }

        // Step 3: insert a synthetic error result for any toolUse ID with no result.
        // `seen` already holds all surviving result IDs — no extra scan needed.
        for id in &tool_use_ids {
            if !seen.contains(id) {
                warn!(
                    tool_use_id = %id,
                    user_idx = j,
                    "Bedrock: inserting synthetic result for toolUse with no matching result"
                );
                messages[j].content.push(BedrockContentBlock::ToolResult {
                    tool_result: BedrockToolResult {
                        tool_use_id: id.clone(),
                        content: vec![BedrockTextBlock {
                            text: "[Tool execution was interrupted or lost]".to_string(),
                        }],
                        status: Some("error".to_string()),
                    },
                });
            }
        }

        // Step 4: if all blocks were removed and nothing was inserted, the message is
        // empty.  Replace with a placeholder so Bedrock does not reject an empty
        // content array and so the conversation does not appear to end with an
        // assistant message.
        if messages[j].content.is_empty() {
            warn!(
                user_idx = j,
                "Bedrock: user message empty after toolResult cleanup; inserting placeholder"
            );
            messages[j].content.push(BedrockContentBlock::Text {
                text: "[prior tool results removed]".to_string(),
            });
        }
    }
}

fn convert_response(resp: ConverseResponse) -> Result<CompletionResponse, LlmError> {
    let mut content = Vec::new();
    let mut tool_calls = Vec::new();

    for block in resp.output.message.content {
        match block {
            BedrockResponseContent::Text { text } => {
                content.push(ContentBlock::Text {
                    text,
                    provider_metadata: None,
                });
            }
            BedrockResponseContent::ToolUse { tool_use } => {
                content.push(ContentBlock::ToolUse {
                    id: tool_use.tool_use_id.clone(),
                    name: tool_use.name.clone(),
                    input: tool_use.input.clone(),
                    provider_metadata: None,
                });
                tool_calls.push(ToolCall {
                    id: tool_use.tool_use_id,
                    name: tool_use.name,
                    input: tool_use.input,
                });
            }
        }
    }

    let stop_reason = match resp.stop_reason.as_str() {
        "end_turn" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        "tool_use" => StopReason::ToolUse,
        // Bedrock Converse: guardrail-triggered refusals (#3450).
        // `guardrail_intervened` is the documented value; `content_filtered`
        // is included for forward-compat with future Bedrock surfaces or
        // adapters that mirror the OpenAI/Azure naming. Either way, route
        // to ContentFiltered so the agent loop stops instead of treating
        // the empty turn as a successful EndTurn.
        "guardrail_intervened" | "content_filtered" => StopReason::ContentFiltered,
        _ if !tool_calls.is_empty() => StopReason::ToolUse,
        _ => StopReason::EndTurn,
    };

    Ok(CompletionResponse {
        content,
        stop_reason,
        tool_calls,
        usage: TokenUsage {
            input_tokens: resp.usage.input_tokens,
            output_tokens: resp.usage.output_tokens,
            // Bedrock Converse API does not yet expose prompt-cache token
            // counters separately, so report zero — the agent loop treats
            // missing cache stats the same as no caching.
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        },
    })
}

// ── Status classification ─────────────────────────────────────────────────────

/// What the dispatch loop should do for a given HTTP status.
///
/// Pure function so the policy can be unit-tested without spinning up an
/// HTTP mock — the live dispatch in `complete()` mirrors this exactly.
#[derive(Debug, PartialEq, Eq)]
enum StatusAction {
    /// 2xx — proceed to parse body.
    Success,
    /// 429/502/503/504 — retry until budget exhausted, then fail.
    Retry,
    /// 401/403 — surface as AuthenticationFailed.
    Auth,
    /// Other 4xx/5xx — surface as Api { status, message }.
    Fail,
}

fn classify_response_status(status: u16) -> StatusAction {
    if (200..300).contains(&status) {
        StatusAction::Success
    } else if status == 429 || status == 502 || status == 503 || status == 504 {
        StatusAction::Retry
    } else if status == 401 || status == 403 {
        StatusAction::Auth
    } else {
        StatusAction::Fail
    }
}

// ── LlmDriver impl ────────────────────────────────────────────────────────────

#[async_trait]
impl LlmDriver for BedrockDriver {
    #[tracing::instrument(
        name = "llm.complete",
        skip_all,
        fields(provider = "bedrock", model = %request.model)
    )]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let (messages, system) = convert_messages(&request.messages, &request.system);

        let converse_request = ConverseRequest {
            messages,
            system,
            inference_config: Some(InferenceConfig {
                max_tokens: Some(request.max_tokens),
                temperature: Some(request.temperature),
            }),
            tool_config: convert_tools(&request.tools),
        };

        let body =
            serde_json::to_vec(&converse_request).map_err(|e| LlmError::Parse(e.to_string()))?;

        let url = self.build_endpoint(&request.model);
        debug!(url = %url, "Sending Bedrock Converse request");

        let max_retries = 3u32;
        for attempt in 0..=max_retries {
            let request_builder = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key.as_str()))
                .header("Content-Type", "application/json")
                // Trace headers are appended after the Bearer token header.
                // This driver uses simple bearer-token auth (not SigV4), so
                // there is no canonical-request hash to protect — the headers
                // are safe to add here without signing-scope concerns.
                .headers(super::trace_headers::build_trace_header_map(
                    &[],
                    &request,
                    self.emit_caller_trace_headers,
                ))
                .body(body.clone());

            let resp = request_builder
                .send()
                .await
                .map_err(|e| LlmError::Http(e.to_string()))?;

            let status = resp.status().as_u16();

            match classify_response_status(status) {
                StatusAction::Success => {}
                StatusAction::Retry => {
                    // Honor any server-supplied Retry-After header; fall
                    // back to a 5 s default when missing/invalid.
                    let retry_after_ms =
                        crate::retry_after::parse_retry_after_ms(resp.headers(), 5000);
                    if attempt < max_retries {
                        let retry_ms = (attempt + 1) as u64 * 2000;
                        // Wait at least the server's Retry-After, but
                        // never less than the in-loop exponential
                        // schedule.
                        let wait_ms = retry_ms.max(retry_after_ms);
                        tracing::warn!(
                            status,
                            wait_ms,
                            attempt,
                            "Bedrock transient failure, retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
                        continue;
                    }
                    return Err(if status == 429 {
                        LlmError::RateLimited {
                            retry_after_ms,
                            message: None,
                        }
                    } else {
                        LlmError::Overloaded { retry_after_ms }
                    });
                }
                StatusAction::Auth => {
                    let body_text = resp.text().await.unwrap_or_default();
                    return Err(LlmError::AuthenticationFailed(body_text));
                }
                StatusAction::Fail => {
                    let body_text = resp.text().await.unwrap_or_default();
                    let message = serde_json::from_str::<BedrockErrorResponse>(&body_text)
                        .map(|e| e.message)
                        .unwrap_or(body_text);
                    return Err(LlmError::Api {
                        status,
                        message,
                        code: None,
                    });
                }
            }

            let body_text = resp
                .text()
                .await
                .map_err(|e| LlmError::Http(e.to_string()))?;
            let converse_response: ConverseResponse =
                serde_json::from_str(&body_text).map_err(|e| {
                    // Use char-based truncation to avoid panics on multi-byte UTF-8
                    // boundaries (Bedrock error bodies may contain non-ASCII).
                    let snippet: String = body_text.chars().take(200).collect();
                    LlmError::Parse(format!("{}: {}", e, snippet))
                })?;

            return convert_response(converse_response);
        }

        Err(LlmError::Api {
            status: 0,
            message: "Max retries exceeded".to_string(),
            code: None,
        })
    }
    // stream() uses the default wrapper from LlmDriver trait — no override needed
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::message::{Message, MessageContent, Role};
    use librefang_types::tool::ToolDefinition;

    // ── Endpoint building ──────────────────────────────────────────────────────

    #[test]
    fn test_build_endpoint() {
        let driver = BedrockDriver::new_with_credentials(
            Some("test-key".to_string()),
            Some("eu-west-1".to_string()),
        )
        .unwrap();
        assert_eq!(
            driver.build_endpoint("anthropic.canonical-model-one"),
            "https://bedrock-runtime.eu-west-1.amazonaws.com/model/anthropic.canonical-model-one/converse"
        );
        assert_eq!(
            driver.build_endpoint("eu.anthropic.canonical-model-one"),
            "https://bedrock-runtime.eu-west-1.amazonaws.com/model/eu.anthropic.canonical-model-one/converse"
        );
    }

    // ── Message conversion ─────────────────────────────────────────────────────

    /// Build a test Message with default values for fields the bedrock
    /// converter doesn't care about (pinned/timestamp).
    fn test_message(role: Role, text: &str) -> Message {
        Message {
            role,
            content: MessageContent::Text(text.to_string()),
            pinned: false,
            timestamp: None,
        }
    }

    #[test]
    fn test_convert_text_message() {
        let messages = vec![test_message(Role::User, "Hello")];
        let (bedrock_msgs, system) = convert_messages(&messages, &None);
        assert_eq!(bedrock_msgs.len(), 1);
        assert_eq!(bedrock_msgs[0].role, "user");
        assert!(system.is_none());
    }

    #[test]
    fn test_system_prompt_from_message() {
        let messages = vec![test_message(Role::System, "Be helpful")];
        let (bedrock_msgs, system) = convert_messages(&messages, &None);
        assert!(bedrock_msgs.is_empty());
        assert!(system.is_some());
        assert_eq!(system.unwrap()[0].text, "Be helpful");
    }

    #[test]
    fn test_system_prompt_from_field() {
        let messages = vec![test_message(Role::User, "Hi")];
        let (_, system) = convert_messages(&messages, &Some("You are an AI".to_string()));
        assert!(system.is_some());
        assert_eq!(system.unwrap()[0].text, "You are an AI");
    }

    // ── Tool conversion ────────────────────────────────────────────────────────

    #[test]
    fn test_convert_tools_empty() {
        let result = convert_tools(&[]);
        assert!(result.is_none());
    }

    #[test]
    fn test_convert_tools_nonempty() {
        let tools = vec![ToolDefinition {
            name: "search".to_string(),
            description: "Search the web".to_string(),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
        }];
        let result = convert_tools(&tools);
        assert!(result.is_some());
        let config = result.unwrap();
        assert_eq!(config.tools.len(), 1);
        assert_eq!(config.tools[0].tool_spec.name, "search");
        assert_eq!(config.tools[0].tool_spec.description, "Search the web");
    }

    // ── Response conversion ────────────────────────────────────────────────────

    #[test]
    fn test_convert_response_text() {
        let resp = ConverseResponse {
            output: ConverseOutput {
                message: BedrockResponseMessage {
                    role: "assistant".to_string(),
                    content: vec![BedrockResponseContent::Text {
                        text: "Hello!".to_string(),
                    }],
                },
            },
            stop_reason: "end_turn".to_string(),
            usage: BedrockUsage {
                input_tokens: 10,
                output_tokens: 5,
            },
        };
        let result = convert_response(resp).unwrap();
        assert_eq!(result.text(), "Hello!");
        assert_eq!(result.usage.input_tokens, 10);
        assert_eq!(result.usage.output_tokens, 5);
        assert!(matches!(result.stop_reason, StopReason::EndTurn));
    }

    #[test]
    fn test_convert_response_tool_use() {
        let resp = ConverseResponse {
            output: ConverseOutput {
                message: BedrockResponseMessage {
                    role: "assistant".to_string(),
                    content: vec![BedrockResponseContent::ToolUse {
                        tool_use: BedrockResponseToolUse {
                            tool_use_id: "call_123".to_string(),
                            name: "search".to_string(),
                            input: serde_json::json!({"query": "rust"}),
                        },
                    }],
                },
            },
            stop_reason: "tool_use".to_string(),
            usage: BedrockUsage {
                input_tokens: 15,
                output_tokens: 8,
            },
        };
        let result = convert_response(resp).unwrap();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "search");
        assert_eq!(result.tool_calls[0].id, "call_123");
        assert!(matches!(result.stop_reason, StopReason::ToolUse));
        // ToolUse must also be in content so the agent loop saves it to the session
        assert_eq!(result.content.len(), 1);
        assert!(matches!(&result.content[0], ContentBlock::ToolUse { id, .. } if id == "call_123"));
    }

    // ── Request serialization ─────────────────────────────────────────────────

    #[test]
    fn test_converse_request_serialization() {
        let req = ConverseRequest {
            messages: vec![BedrockMessage {
                role: "user".to_string(),
                content: vec![BedrockContentBlock::Text {
                    text: "Hi".to_string(),
                }],
            }],
            system: Some(vec![BedrockTextBlock {
                text: "Be helpful".to_string(),
            }]),
            inference_config: Some(InferenceConfig {
                max_tokens: Some(1024),
                temperature: Some(0.7),
            }),
            tool_config: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["messages"][0]["role"], "user");
        assert_eq!(json["system"][0]["text"], "Be helpful");
        // camelCase keys from #[serde(rename_all = "camelCase")]
        assert_eq!(json["inferenceConfig"]["maxTokens"], 1024);
        // None fields should be absent
        assert!(json.get("toolConfig").is_none());
    }

    // ── validate_bedrock_tool_pairing ─────────────────────────────────────────

    fn make_asst_with_uses(ids: &[&str]) -> BedrockMessage {
        BedrockMessage {
            role: "assistant".to_string(),
            content: ids
                .iter()
                .map(|id| BedrockContentBlock::ToolUse {
                    tool_use: BedrockToolUse {
                        tool_use_id: id.to_string(),
                        name: "tool".to_string(),
                        input: serde_json::json!({}),
                    },
                })
                .collect(),
        }
    }

    fn make_user_with_results(ids: &[&str]) -> BedrockMessage {
        BedrockMessage {
            role: "user".to_string(),
            content: ids
                .iter()
                .map(|id| BedrockContentBlock::ToolResult {
                    tool_result: BedrockToolResult {
                        tool_use_id: id.to_string(),
                        content: vec![BedrockTextBlock {
                            text: "ok".to_string(),
                        }],
                        status: None,
                    },
                })
                .collect(),
        }
    }

    fn result_ids(msg: &BedrockMessage) -> Vec<String> {
        msg.content
            .iter()
            .filter_map(|b| {
                if let BedrockContentBlock::ToolResult { tool_result } = b {
                    Some(tool_result.tool_use_id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn test_validate_tool_pairing_removes_extra_result() {
        // assistant has 1 toolUse (A); user has 2 toolResults (A, B) → B should be removed
        let mut messages = vec![
            make_asst_with_uses(&["A"]),
            make_user_with_results(&["A", "B"]),
        ];
        validate_bedrock_tool_pairing(&mut messages);
        let ids = result_ids(&messages[1]);
        assert_eq!(ids, vec!["A"]);
    }

    #[test]
    fn test_validate_tool_pairing_inserts_synthetic() {
        // assistant has 2 toolUses (A, B); user has 1 toolResult (A) → synthetic B inserted
        let mut messages = vec![
            make_asst_with_uses(&["A", "B"]),
            make_user_with_results(&["A"]),
        ];
        validate_bedrock_tool_pairing(&mut messages);
        let ids = result_ids(&messages[1]);
        assert!(ids.contains(&"A".to_string()));
        assert!(ids.contains(&"B".to_string()));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn test_validate_tool_pairing_text_only_asst_cleans_stray_results() {
        // text-only assistant (0 toolUses) followed by user with only ToolResult blocks
        // → results removed, empty message replaced with placeholder text, NOT dropped
        let mut messages = vec![
            BedrockMessage {
                role: "assistant".to_string(),
                content: vec![BedrockContentBlock::Text {
                    text: "Done!".to_string(),
                }],
            },
            make_user_with_results(&["orphan"]),
        ];
        validate_bedrock_tool_pairing(&mut messages);
        // Message array length preserved — user message replaced with placeholder
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].role, "user");
        // No ToolResult blocks remain
        assert_eq!(result_ids(&messages[1]).len(), 0);
        // Has exactly one non-empty placeholder text block
        assert_eq!(messages[1].content.len(), 1);
        if let BedrockContentBlock::Text { text } = &messages[1].content[0] {
            assert!(!text.is_empty());
        } else {
            panic!("expected Text block");
        }
    }

    #[test]
    fn test_validate_tool_pairing_deduplicates_result_ids() {
        // assistant has 1 toolUse (A); user has 2 toolResult blocks both with ID A
        // → second duplicate removed, count is now 1=1
        let mut messages = vec![
            make_asst_with_uses(&["A"]),
            make_user_with_results(&["A", "A"]),
        ];
        validate_bedrock_tool_pairing(&mut messages);
        let ids = result_ids(&messages[1]);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "A");
    }

    #[test]
    fn test_validate_tool_pairing_last_message_not_dropped() {
        // Ensure conversation does not end with an assistant message after cleanup.
        // text-only assistant as last-but-one, pure-ToolResult user as last message.
        let mut messages = vec![
            BedrockMessage {
                role: "user".to_string(),
                content: vec![BedrockContentBlock::Text {
                    text: "hi".to_string(),
                }],
            },
            BedrockMessage {
                role: "assistant".to_string(),
                content: vec![BedrockContentBlock::Text {
                    text: "hello".to_string(),
                }],
            },
            make_user_with_results(&["stray"]),
        ];
        validate_bedrock_tool_pairing(&mut messages);
        // Last message must still be a user message (not assistant)
        assert_eq!(messages.last().unwrap().role, "user");
    }

    #[test]
    fn test_validate_tool_pairing_relocates_stray_results() {
        // Scenario mirroring the production bug:
        // session_repair merged a user message so the tool results from asst[0]
        // (which has 3 toolUse blocks) ended up in user[3], adjacent to a text-only
        // asst[2] that has 0 toolUse blocks.
        //
        // Expected: results are *relocated* to user[1] (prepended before its text),
        // and user[3] (now empty) gets a placeholder — no data is lost.
        let mut messages = vec![
            make_asst_with_uses(&["A", "B", "C"]), // [0] asst: 3 toolUse
            BedrockMessage {
                // [1] user: text only (no results)
                role: "user".to_string(),
                content: vec![BedrockContentBlock::Text {
                    text: "continue".to_string(),
                }],
            },
            BedrockMessage {
                // [2] text-only asst (0 toolUse)
                role: "assistant".to_string(),
                content: vec![BedrockContentBlock::Text {
                    text: "Sure".to_string(),
                }],
            },
            make_user_with_results(&["A", "B", "C"]), // [3] STRAY — belongs to asst[0]
        ];
        validate_bedrock_tool_pairing(&mut messages);

        // All 3 results must now be in user[1], paired with asst[0].
        let ids_at_1 = result_ids(&messages[1]);
        assert_eq!(ids_at_1.len(), 3);
        assert!(ids_at_1.contains(&"A".to_string()));
        assert!(ids_at_1.contains(&"B".to_string()));
        assert!(ids_at_1.contains(&"C".to_string()));

        // user[1] still retains its original text block.
        let text_at_1 = messages[1]
            .content
            .iter()
            .filter(|b| matches!(b, BedrockContentBlock::Text { .. }))
            .count();
        assert_eq!(text_at_1, 1);

        // user[3] is now a placeholder (no toolResult blocks), conversation still ends
        // with a user message.
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[3].role, "user");
        assert_eq!(result_ids(&messages[3]).len(), 0);
        // Has at least one text block (the placeholder).
        let text_at_3 = messages[3]
            .content
            .iter()
            .filter(|b| matches!(b, BedrockContentBlock::Text { .. }))
            .count();
        assert!(text_at_3 >= 1);
    }

    // ── Error response handling ───────────────────────────────────────────────

    #[test]
    fn test_classify_status_success() {
        assert_eq!(classify_response_status(200), StatusAction::Success);
        assert_eq!(classify_response_status(204), StatusAction::Success);
    }

    #[test]
    fn test_classify_status_auth_failures() {
        // 401 Unauthorized and 403 Forbidden must map to AuthenticationFailed
        // so the agent loop does not retry them as transient.
        assert_eq!(classify_response_status(401), StatusAction::Auth);
        assert_eq!(classify_response_status(403), StatusAction::Auth);
    }

    #[test]
    fn test_classify_status_rate_limited() {
        // 429 must be retried (not surface immediately).
        assert_eq!(classify_response_status(429), StatusAction::Retry);
    }

    #[test]
    fn test_classify_status_transient_5xx_retries() {
        // 502, 503, 504 are transient gateway/service errors — retry them.
        assert_eq!(classify_response_status(502), StatusAction::Retry);
        assert_eq!(classify_response_status(503), StatusAction::Retry);
        assert_eq!(classify_response_status(504), StatusAction::Retry);
    }

    #[test]
    fn test_classify_status_permanent_5xx_does_not_retry() {
        // 500, 501, 505 are permanent / malformed-request 5xx — do NOT retry.
        assert_eq!(classify_response_status(500), StatusAction::Fail);
        assert_eq!(classify_response_status(501), StatusAction::Fail);
        assert_eq!(classify_response_status(505), StatusAction::Fail);
    }

    #[test]
    fn test_classify_status_other_4xx_fails() {
        // 400 / 404 / 422 surface as Api error, not Auth, not Retry.
        assert_eq!(classify_response_status(400), StatusAction::Fail);
        assert_eq!(classify_response_status(404), StatusAction::Fail);
        assert_eq!(classify_response_status(422), StatusAction::Fail);
    }

    #[test]
    fn test_bedrock_error_response_parses_message() {
        // Real Bedrock 4xx body shape: {"message": "..."}
        let body = r#"{"message":"The security token included in the request is invalid."}"#;
        let parsed: BedrockErrorResponse = serde_json::from_str(body).unwrap();
        assert_eq!(
            parsed.message,
            "The security token included in the request is invalid."
        );
    }

    #[test]
    fn test_error_body_truncation_safe_on_multibyte_boundary() {
        // Regression: previous slice `&body[..body.len().min(200)]` panicked when
        // byte 200 fell inside a multi-byte UTF-8 sequence. The fix uses
        // chars().take(200) which is codepoint-aware.
        //
        // Build a body whose 200th byte falls inside a 3-byte UTF-8 char (中文).
        // Each Chinese char is 3 bytes, so 70 chars = 210 bytes. Truncating to
        // 200 bytes via byte-slice would land mid-codepoint. chars().take(200)
        // takes 200 codepoints (well within the 70 we have) without panic.
        let body: String = "中".repeat(70); // 210 bytes, 70 chars
        assert!(body.len() > 200);
        let snippet: String = body.chars().take(200).collect();
        // 70 chars < 200 chars, so we get all of them back.
        assert_eq!(snippet.chars().count(), 70);
        // And byte length is whatever 70 Chinese chars occupy — never panics.
        assert_eq!(snippet.len(), 210);
    }

    #[test]
    fn test_error_body_truncation_caps_codepoints() {
        // When the body is longer than 200 codepoints, only the first 200 are kept.
        let body: String = "a".repeat(500);
        let snippet: String = body.chars().take(200).collect();
        assert_eq!(snippet.chars().count(), 200);
    }

    #[test]
    fn test_validate_tool_pairing_noop_on_correct() {
        // already correct 2-for-2 → no change
        let mut messages = vec![
            make_asst_with_uses(&["A", "B"]),
            make_user_with_results(&["A", "B"]),
        ];
        validate_bedrock_tool_pairing(&mut messages);
        let ids = result_ids(&messages[1]);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"A".to_string()));
        assert!(ids.contains(&"B".to_string()));
    }
}
