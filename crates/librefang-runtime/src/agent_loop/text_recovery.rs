//! Recover tool calls that arrive as free text instead of through the LLM's
//! native `tool_calls` field. Also covers the action-claim hallucination
//! detection that the loop uses to decide whether to nudge the model.
//!
//! Patterns covered: <function=NAME>{...}</function>, <function>NAME{...}</function>,
//! <tool>NAME{...}</tool>, markdown code blocks, backtick-wrapped calls,
//! [TOOL_CALL]...[/TOOL_CALL] blocks (JSON + arrow syntax), <tool_call>JSON</tool_call>
//! (Qwen3), <function name="..." parameters="..." />, and a bare-JSON fallback.

use librefang_types::message::StopReason;
use librefang_types::tool::{ToolCall, ToolDefinition};
use tracing::{info, warn};

/// Detect when the LLM claims to have performed an action in text without
/// actually calling any tools — a common hallucination pattern.
///
/// Covers three claim families: English present-perfect (`i've|i have` +
/// verb), Italian present-perfect (`ho` + past participle), and impersonal
/// completion claims in either language (`successfully X`, `è stato/stata`,
/// `messaggio inviato`). Bare "fatto" is intentionally absent — too noisy
/// as a substring (matches "non ho fatto in tempo").
pub(super) fn looks_like_hallucinated_action(text: &str) -> bool {
    let lower = text.to_lowercase();
    let action_phrases = [
        // English present-perfect — dev/file flavor.
        "i've created",
        "i've written",
        "i've updated",
        "i've saved",
        "i've modified",
        "i've deleted",
        "i've added",
        "i've removed",
        "i've edited",
        "i've fixed",
        "i've changed",
        "i've installed",
        "i have created",
        "i have written",
        "i have updated",
        "i have saved",
        "i have modified",
        "i have deleted",
        "i have added",
        "i have removed",
        // English present-perfect — transactional/domain flavor.
        "i've sent",
        "i've scheduled",
        "i've booked",
        "i've ordered",
        "i've registered",
        "i've recorded",
        "i've transferred",
        "i've logged",
        "i've notified",
        "i've cancelled",
        "i've canceled",
        "i've reserved",
        "i've submitted",
        "i've forwarded",
        "i've attached",
        "i've published",
        "i've uploaded",
        "i have sent",
        "i have scheduled",
        "i have booked",
        "i have ordered",
        "i have registered",
        "i have recorded",
        "i have transferred",
        "i have logged",
        "i have notified",
        "i have submitted",
        "i have attached",
        // English impersonal completion claims.
        "file has been",
        "changes have been",
        "code has been",
        "message has been sent",
        "transaction has been",
        "appointment has been",
        "booking has been",
        "order has been placed",
        "successfully created",
        "successfully updated",
        "successfully saved",
        "successfully written",
        "successfully modified",
        "successfully sent",
        "successfully scheduled",
        "successfully booked",
        "successfully registered",
        // Italian present-perfect — ho + past participle.
        "ho creato",
        "ho scritto",
        "ho aggiornato",
        "ho salvato",
        "ho modificato",
        "ho cancellato",
        "ho aggiunto",
        "ho rimosso",
        "ho eliminato",
        "ho inviato",
        "ho mandato",
        "ho spedito",
        "ho registrato",
        "ho allegato",
        "ho prenotato",
        "ho ordinato",
        "ho schedulato",
        "ho programmato",
        "ho bonificato",
        "ho trasferito",
        "ho recapitato",
        "ho notificato",
        "ho pubblicato",
        "ho caricato",
        "ho fissato",
        "ho impostato",
        // Italian impersonal completion claims.
        "è stato inviato",
        "è stato registrato",
        "è stato salvato",
        "è stato creato",
        "è stato aggiornato",
        "è stato cancellato",
        "è stato programmato",
        "è stato schedulato",
        "è stato prenotato",
        "è stato bonificato",
        "è stata inviata",
        "è stata creata",
        "è stata aggiornata",
        "è stata salvata",
        "è stata cancellata",
        "è stata registrata",
        "è stata programmata",
        "è stata schedulata",
        "è stata prenotata",
        "è stata recapitata",
        // Italian outcome adjectives — narrow forms that imply a completed
        // operation. Avoid bare "fatto" — too noisy as a substring.
        "messaggio inviato",
        "messaggio recapitato",
        "spesa registrata",
        "transazione registrata",
        "operazione completata",
        "operazione riuscita",
        "ordine effettuato",
        "prenotazione effettuata",
        "bonifico effettuato",
        "trasferimento effettuato",
    ];
    action_phrases.iter().any(|phrase| lower.contains(phrase))
}

/// Detect whether the **user's** message contains explicit action-oriented keywords
/// that imply tool execution is required.  When the LLM responds with only text
/// (no `tool_calls`) despite tools being available and the user clearly requesting
/// an action, we should nudge the model to actually invoke tools.
///
/// This complements `looks_like_hallucinated_action` which checks the LLM's
/// *response* text for claims of completion.  This function checks the *user
/// intent* so we can catch cases where the LLM simply describes a plan or
/// summarises the request without attempting to fulfill it.
pub(super) fn user_message_has_action_intent(user_message: &str) -> bool {
    let lower = user_message.to_lowercase();
    let action_keywords = [
        "send", "execute", "create", "delete", "remove", "write", "publish", "deploy", "install",
        "upload", "download", "forward", "submit", "trigger", "launch", "notify", "schedule",
        "rename", "fetch",
    ];
    // Require the keyword to appear as an exact word — uses split_whitespace()
    // so "running" does NOT match "run", and "recreate" does NOT match "create".
    action_keywords.iter().any(|kw| {
        lower.split_whitespace().any(|word| {
            // Strip common punctuation so "send," or "send!" still match
            let cleaned = word.trim_matches(|c: char| c.is_ascii_punctuation());
            cleaned == *kw
        })
    })
}

/// Whether `recover_text_tool_calls` should even be attempted for this
/// turn's response — shared by the non-streaming and streaming loops so the
/// gate can't drift between them (#8236).
///
/// `forced_tools_stripped_this_turn` must be `false`: the block-stall
/// degrade (#5979) strips `tools` from the *request* for exactly one turn so
/// the model is forced to answer in prose instead of re-issuing the call the
/// loop guard keeps blocking. If recovery still promotes markup text back
/// into a `ToolCall` on that same turn, the degrade never actually breaks
/// the loop — the model calls the tool via text instead of the API field,
/// recovery undoes the degrade, the guard blocks it again next iteration,
/// and the cycle repeats to `max_iterations`. The turn offered no tools, so
/// text that merely resembles a call this turn is not a call the model was
/// invited to make.
///
/// `synthesized_from_thinking` must be `false`: when the driver synthesized
/// the response text from a thinking-only reply (`text_synthesized_from_thinking`
/// on `CompletionResponse`, #8556), that text is a best-effort answer, not a
/// call the model was invited to make. Promoting it back into a `ToolUse`
/// keeps the turn alive — the model answers "in prose", recovery turns the
/// prose into a call, and the loop never reaches a clean end.
pub(super) fn should_attempt_text_recovery(
    forced_tools_stripped_this_turn: bool,
    stop_reason: StopReason,
    tool_calls_empty: bool,
    synthesized_from_thinking: bool,
) -> bool {
    !forced_tools_stripped_this_turn
        && !synthesized_from_thinking
        && matches!(stop_reason, StopReason::EndTurn | StopReason::StopSequence)
        && tool_calls_empty
}

/// Recover tool calls that LLMs output as plain text instead of the proper
/// `tool_calls` API field. Covers Groq/Llama, DeepSeek, Qwen, and Ollama models.
///
/// Supported patterns:
/// 1. `<function=tool_name>{"key":"value"}</function>`
/// 2. `<function>tool_name{"key":"value"}</function>`
/// 3. `<tool>tool_name{"key":"value"}</tool>`
/// 4. Markdown code blocks containing `tool_name {"key":"value"}`
/// 5. Backtick-wrapped `tool_name {"key":"value"}`
/// 6. `[TOOL_CALL]...[/TOOL_CALL]` blocks (JSON or arrow syntax) — issue #354
/// 7. `<tool_call>{"name":"tool","arguments":{...}}</tool_call>` — Qwen3, issue #332
/// 8. Bare JSON `{"name":"tool","arguments":{...}}` objects (last resort, only if no tags found)
/// 9. `<function name="tool" parameters="{...}" />` — XML attribute style (Groq/Llama)
///
/// Validates tool names against available tools and returns synthetic `ToolCall` entries.
pub(super) fn recover_text_tool_calls(
    text: &str,
    available_tools: &[ToolDefinition],
) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    let tool_names: Vec<&str> = available_tools.iter().map(|t| t.name.as_str()).collect();

    // Pattern 1: <function=TOOL_NAME>JSON_BODY</function>
    let mut search_from = 0;
    while let Some(start) = text[search_from..].find("<function=") {
        let abs_start = search_from + start;
        let after_prefix = abs_start + "<function=".len();

        // Extract tool name (ends at '>')
        let Some(name_end) = text[after_prefix..].find('>') else {
            search_from = after_prefix;
            continue;
        };
        let tool_name = &text[after_prefix..after_prefix + name_end];
        let json_start = after_prefix + name_end + 1;

        // Find closing </function>
        let Some(close_offset) = text[json_start..].find("</function>") else {
            search_from = json_start;
            continue;
        };
        let json_body = text[json_start..json_start + close_offset].trim();
        search_from = json_start + close_offset + "</function>".len();

        // Validate: tool name must be in available_tools
        if !tool_names.contains(&tool_name) {
            warn!(
                tool = tool_name,
                "Text-based tool call for unknown tool — skipping"
            );
            continue;
        }

        // Parse the input: a JSON object, or — for the Hermes/Llama-3
        // parameter style some GGUFs emit behind OpenAI-compatible proxies —
        // a sequence of <parameter=NAME>value</parameter> pairs (#8235).
        let input: serde_json::Value = match serde_json::from_str(json_body) {
            Ok(v) => v,
            Err(e) => match parse_parameter_style_body(
                json_body,
                tool_input_schema(tool_name, available_tools),
            ) {
                Some(v) => v,
                None => {
                    warn!(tool = tool_name, error = %e, "Failed to parse text-based tool call JSON — skipping");
                    continue;
                }
            },
        };

        info!(
            tool = tool_name,
            "Recovered text-based tool call → synthetic ToolUse"
        );
        calls.push(ToolCall {
            id: format!("recovered_{}", uuid::Uuid::new_v4()),
            name: tool_name.to_string(),
            input,
        });
    }

    // Pattern 2: <function>TOOL_NAME{JSON_BODY}</function>
    // (Groq/Llama variant — tool name immediately followed by JSON object)
    search_from = 0;
    while let Some(start) = text[search_from..].find("<function>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<function>".len();

        // Find closing </function>
        let Some(close_offset) = text[after_tag..].find("</function>") else {
            search_from = after_tag;
            continue;
        };
        let inner = &text[after_tag..after_tag + close_offset];
        search_from = after_tag + close_offset + "</function>".len();

        // The inner content is normally "tool_name{json}". The Hermes/Llama-3
        // parameter style has no JSON object — the name runs up to the first
        // '<' of the parameter markup instead (#8235), so split at whichever
        // marker comes first.
        let brace_pos = inner.find('{');
        let angle_pos = inner.find('<');
        let split_pos = match (brace_pos, angle_pos) {
            (Some(b), Some(a)) => b.min(a),
            (Some(b), None) => b,
            (None, Some(a)) => a,
            (None, None) => continue,
        };
        let tool_name = inner[..split_pos].trim();
        let body = inner[split_pos..].trim();

        if tool_name.is_empty() {
            continue;
        }

        // Validate: tool name must be in available_tools
        if !tool_names.contains(&tool_name) {
            warn!(
                tool = tool_name,
                "Text-based tool call (variant 2) for unknown tool — skipping"
            );
            continue;
        }

        // Parse JSON input, falling back to the parameter style when the
        // body is not a JSON object (#8235).
        let input: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => match parse_parameter_style_body(
                body,
                tool_input_schema(tool_name, available_tools),
            ) {
                Some(v) => v,
                None => {
                    warn!(tool = tool_name, error = %e, "Failed to parse text-based tool call JSON (variant 2) — skipping");
                    continue;
                }
            },
        };

        // Avoid duplicates if pattern 1 already captured this call
        if calls
            .iter()
            .any(|c| c.name == tool_name && c.input == input)
        {
            continue;
        }

        info!(
            tool = tool_name,
            "Recovered text-based tool call (variant 2) → synthetic ToolUse"
        );
        calls.push(ToolCall {
            id: format!("recovered_{}", uuid::Uuid::new_v4()),
            name: tool_name.to_string(),
            input,
        });
    }

    // Pattern 3: <tool>TOOL_NAME{JSON}</tool>  (Qwen / DeepSeek variant)
    search_from = 0;
    while let Some(start) = text[search_from..].find("<tool>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<tool>".len();

        let Some(close_offset) = text[after_tag..].find("</tool>") else {
            search_from = after_tag;
            continue;
        };
        let inner = &text[after_tag..after_tag + close_offset];
        search_from = after_tag + close_offset + "</tool>".len();

        let Some(brace_pos) = inner.find('{') else {
            continue;
        };
        let tool_name = inner[..brace_pos].trim();
        let json_body = inner[brace_pos..].trim();

        if tool_name.is_empty() || !tool_names.contains(&tool_name) {
            continue;
        }

        let input: serde_json::Value = match serde_json::from_str(json_body) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if calls
            .iter()
            .any(|c| c.name == tool_name && c.input == input)
        {
            continue;
        }

        info!(
            tool = tool_name,
            "Recovered text-based tool call (<tool> variant) → synthetic ToolUse"
        );
        calls.push(ToolCall {
            id: format!("recovered_{}", uuid::Uuid::new_v4()),
            name: tool_name.to_string(),
            input,
        });
    }

    // Pattern 4: Markdown code blocks containing tool_name {JSON}
    // Matches: ```\nexec {"command":"ls"}\n``` or ```bash\nexec {"command":"ls"}\n```
    {
        let mut in_block = false;
        let mut block_content = String::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                if in_block {
                    // End of block — try to extract tool call from content
                    let content = block_content.trim();
                    if let Some(brace_pos) = content.find('{') {
                        let potential_tool = content[..brace_pos].trim();
                        if tool_names.contains(&potential_tool) {
                            if let Ok(input) = serde_json::from_str::<serde_json::Value>(
                                content[brace_pos..].trim(),
                            ) {
                                if !calls
                                    .iter()
                                    .any(|c| c.name == potential_tool && c.input == input)
                                {
                                    info!(
                                        tool = potential_tool,
                                        "Recovered tool call from markdown code block"
                                    );
                                    calls.push(ToolCall {
                                        id: format!("recovered_{}", uuid::Uuid::new_v4()),
                                        name: potential_tool.to_string(),
                                        input,
                                    });
                                }
                            }
                        }
                    }
                    block_content.clear();
                    in_block = false;
                } else {
                    in_block = true;
                    block_content.clear();
                }
            } else if in_block {
                if !block_content.is_empty() {
                    block_content.push('\n');
                }
                block_content.push_str(trimmed);
            }
        }
    }

    // Pattern 5: Backtick-wrapped tool call: `tool_name {"key":"value"}`
    {
        let parts: Vec<&str> = text.split('`').collect();
        // Every odd-indexed element is inside backticks
        for chunk in parts.iter().skip(1).step_by(2) {
            let trimmed = chunk.trim();
            if let Some(brace_pos) = trimmed.find('{') {
                let potential_tool = trimmed[..brace_pos].trim();
                if !potential_tool.is_empty()
                    && !potential_tool.contains(' ')
                    && tool_names.contains(&potential_tool)
                {
                    if let Ok(input) =
                        serde_json::from_str::<serde_json::Value>(trimmed[brace_pos..].trim())
                    {
                        if !calls
                            .iter()
                            .any(|c| c.name == potential_tool && c.input == input)
                        {
                            info!(
                                tool = potential_tool,
                                "Recovered tool call from backtick-wrapped text"
                            );
                            calls.push(ToolCall {
                                id: format!("recovered_{}", uuid::Uuid::new_v4()),
                                name: potential_tool.to_string(),
                                input,
                            });
                        }
                    }
                }
            }
        }
    }

    // Pattern 6: [TOOL_CALL]...[/TOOL_CALL] blocks (Ollama models like Qwen, issue #354)
    // Handles both JSON args and custom `{tool => "name", args => {--key "value"}}` syntax.
    search_from = 0;
    while let Some(start) = text[search_from..].find("[TOOL_CALL]") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "[TOOL_CALL]".len();

        let Some(close_offset) = text[after_tag..].find("[/TOOL_CALL]") else {
            search_from = after_tag;
            continue;
        };
        let inner = text[after_tag..after_tag + close_offset].trim();
        search_from = after_tag + close_offset + "[/TOOL_CALL]".len();

        // Try standard JSON first: {"name":"tool","arguments":{...}}
        if let Some((tool_name, input)) = parse_json_tool_call_object(inner, &tool_names) {
            if !calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from [TOOL_CALL] block (JSON)"
                );
                calls.push(ToolCall {
                    id: format!("recovered_{}", uuid::Uuid::new_v4()),
                    name: tool_name,
                    input,
                });
            }
            continue;
        }

        // Custom arrow syntax: {tool => "name", args => {--key "value"}}
        if let Some((tool_name, input)) = parse_arrow_syntax_tool_call(inner, &tool_names) {
            if !calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from [TOOL_CALL] block (arrow syntax)"
                );
                calls.push(ToolCall {
                    id: format!("recovered_{}", uuid::Uuid::new_v4()),
                    name: tool_name,
                    input,
                });
            }
        }
    }

    // Pattern 7: <tool_call>JSON</tool_call> (Qwen3 models on Ollama, issue #332)
    search_from = 0;
    while let Some(start) = text[search_from..].find("<tool_call>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<tool_call>".len();

        let Some(close_offset) = text[after_tag..].find("</tool_call>") else {
            search_from = after_tag;
            continue;
        };
        let inner = text[after_tag..after_tag + close_offset].trim();
        search_from = after_tag + close_offset + "</tool_call>".len();

        if let Some((tool_name, input)) = parse_json_tool_call_object(inner, &tool_names) {
            if !calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from <tool_call> block"
                );
                calls.push(ToolCall {
                    id: format!("recovered_{}", uuid::Uuid::new_v4()),
                    name: tool_name,
                    input,
                });
            }
        }
    }

    // Pattern 9: <function name="tool" parameters="{...}" /> — XML attribute style
    // Groq/Llama sometimes emit self-closing XML with name/parameters attributes.
    // The parameters value is HTML-entity-escaped JSON (&quot; etc.).
    {
        use regex_lite::Regex;
        // Cached: this parser runs on every LLM response (#3491).
        static FUNCTION_TAG_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
            Regex::new(r#"<function\s+name="([^"]+)"\s+parameters="([^"]*)"[^/]*/?>"#).unwrap()
        });
        let re = &*FUNCTION_TAG_RE;
        for caps in re.captures_iter(text) {
            let tool_name = caps.get(1).unwrap().as_str();
            let raw_params = caps.get(2).unwrap().as_str();

            if !tool_names.contains(&tool_name) {
                warn!(
                    tool = tool_name,
                    "XML-attribute tool call for unknown tool — skipping"
                );
                continue;
            }

            // Unescape HTML entities (&quot; &amp; &lt; &gt; &apos;)
            let unescaped = raw_params
                .replace("&quot;", "\"")
                .replace("&amp;", "&")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&apos;", "'");

            let input: serde_json::Value = match serde_json::from_str(&unescaped) {
                Ok(v) => v,
                Err(e) => {
                    warn!(tool = tool_name, error = %e, "Failed to parse XML-attribute tool call params — skipping");
                    continue;
                }
            };

            if calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                continue;
            }

            info!(
                tool = tool_name,
                "Recovered XML-attribute tool call → synthetic ToolUse"
            );
            calls.push(ToolCall {
                id: format!("recovered_{}", uuid::Uuid::new_v4()),
                name: tool_name.to_string(),
                input,
            });
        }
    }

    // Pattern 8: Bare JSON tool call objects in text (common Ollama fallback)
    // Matches: {"name":"tool_name","arguments":{"key":"value"}} not already inside tags
    // Only try this if no calls were found by tag-based patterns, to avoid false positives.
    if calls.is_empty() {
        // Scan for JSON objects that look like tool calls
        let mut scan_from = 0;
        while let Some(brace_start) = text[scan_from..].find('{') {
            let abs_brace = scan_from + brace_start;
            // Try to parse a JSON object starting here
            if let Some((tool_name, input)) =
                try_parse_bare_json_tool_call(&text[abs_brace..], &tool_names)
            {
                if !calls
                    .iter()
                    .any(|c| c.name == tool_name && c.input == input)
                {
                    info!(
                        tool = tool_name.as_str(),
                        "Recovered tool call from bare JSON object in text"
                    );
                    calls.push(ToolCall {
                        id: format!("recovered_{}", uuid::Uuid::new_v4()),
                        name: tool_name,
                        input,
                    });
                }
            }
            scan_from = abs_brace + 1;
        }
    }

    calls
}

/// A reply that is nothing but tool-call markup the recovery could not parse
/// is replaced by this sentence (#8235). Delivering the raw syntax to the
/// channel is the worst possible answer: it is truthful about nothing and
/// unreadable to the user.
const HONEST_UNRECOVERABLE_REPLY: &str = "I tried to call a tool, but the call could not be parsed, so no action was performed. Please send the request again, or start a fresh session with /reset.";

/// Guard the delivered reply against raw tool-call syntax (#8235).
///
/// When a model answers with nothing but a tool call written as text that
/// recovery could not parse — unknown tool, malformed body — the leftover
/// markup is all the user would receive. A reply that starts with a known
/// tool-call opener and leaves nothing once the markup spans are stripped is
/// replaced by one honest sentence. Anything that starts with prose passes
/// through untouched, even when it contains markup further in.
pub(super) fn replace_unrecoverable_tool_call_reply(text: &str) -> std::borrow::Cow<'_, str> {
    let trimmed = text.trim_start();
    if !TOOL_CALL_OPENERS.iter().any(|o| trimmed.starts_with(o)) {
        return std::borrow::Cow::Borrowed(text);
    }
    if strip_tool_call_spans(trimmed).trim().is_empty() {
        std::borrow::Cow::Owned(HONEST_UNRECOVERABLE_REPLY.to_string())
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// Tool-call markup openers recognized by [`replace_unrecoverable_tool_call_reply`]
/// and, incrementally, by the streaming withholding buffer in
/// `retry::stream_with_retry` (#8236) — shared so both places agree on what
/// counts as "this might be a call".
pub(super) const TOOL_CALL_OPENERS: &[&str] = &[
    "<function=",
    "<function>",
    "<tool>",
    "[TOOL_CALL]",
    "<tool_call>",
    "<|tool_call|>",
];

/// Whether `text` (assumed already left-trimmed) either already is, or with
/// more characters still to arrive could become, one of [`TOOL_CALL_OPENERS`].
/// Used by the streaming buffer to decide whether a delta whose opener is
/// arriving one token at a time must keep being withheld rather than being
/// ruled out on a partial prefix (#8236).
pub(super) fn could_be_tool_call_opener(text: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    TOOL_CALL_OPENERS
        .iter()
        .any(|o| o.starts_with(text) || text.starts_with(o))
}

/// Remove every recognized tool-call markup span from `text`, keeping the
/// prose between them. A span whose closer never appears is dropped to the
/// end of the text — an unterminated call is not something to show either.
fn strip_tool_call_spans(text: &str) -> String {
    const SPANS: &[(&str, &str)] = &[
        ("<function=", "</function>"),
        ("<function>", "</function>"),
        ("<tool>", "</tool>"),
        ("[TOOL_CALL]", "[/TOOL_CALL]"),
        ("<tool_call>", "</tool_call>"),
        ("<|tool_call|>", "<|/tool_call|>"),
    ];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'outer: while !rest.is_empty() {
        // Earliest opener wins, so prose before it is kept in order.
        let mut earliest: Option<(usize, (&str, &str))> = None;
        for (open, close) in SPANS {
            if let Some(pos) = rest.find(open) {
                if earliest.is_none_or(|(p, _)| pos < p) {
                    earliest = Some((pos, (open, close)));
                }
            }
        }
        // No further opener — whatever remains is prose that was never part
        // of a call and must be kept, not silently dropped (#8236).
        let Some((pos, (open, close))) = earliest else {
            out.push_str(rest);
            break 'outer;
        };
        out.push_str(&rest[..pos]);
        let after_open = pos + open.len();
        match rest[after_open..].find(close) {
            Some(closer_off) => {
                rest = &rest[after_open + closer_off + close.len()..];
            }
            None => {
                // Unterminated span — drop the remainder.
                break 'outer;
            }
        }
    }
    out
}

/// Parse the Hermes/Llama-3 parameter style some models emit inside the
/// function tags: a sequence of `<parameter=NAME>value</parameter>` pairs
/// instead of a JSON object (#8235). Values are free text — a `command`
/// parameter routinely spans several lines — and the parser takes each value
/// verbatim, less the single newline the markup convention places right
/// after `>` and right before `</parameter>` (#8236) — a full `.trim()`
/// would also eat a deliberate trailing newline or leading indentation on a
/// content-bearing parameter (`content` for a file-writing tool, say).
///
/// `schema` is the tool's `input_schema` (when the tool is known), consulted
/// so numeric-looking values are only coerced to a number/bool when the
/// schema actually declares that parameter as one — see `coerce_scalar`.
fn parse_parameter_style_body(
    body: &str,
    schema: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    let mut map = serde_json::Map::new();
    let mut search_from = 0;
    let mut saw_any = false;
    while let Some(start) = body[search_from..].find("<parameter=") {
        let after_name = search_from + start + "<parameter=".len();
        let Some(name_end) = body[after_name..].find('>') else {
            break;
        };
        let name = body[after_name..after_name + name_end].trim();
        let value_start = after_name + name_end + 1;
        let Some(close_rel) = body[value_start..].find("</parameter>") else {
            break;
        };
        let value = strip_single_markup_newline(&body[value_start..value_start + close_rel]);
        search_from = value_start + close_rel + "</parameter>".len();
        if name.is_empty() {
            continue;
        }
        saw_any = true;
        map.insert(
            name.to_string(),
            coerce_scalar(value, schema_property_type(schema, name)),
        );
    }
    if !saw_any {
        return None;
    }
    Some(serde_json::Value::Object(map))
}

/// Strip exactly one leading and one trailing newline (`\n` or `\r\n`) —
/// the whitespace the parameter markup convention itself introduces around
/// `>value</parameter>` — leaving every other character, including
/// deliberate indentation or a trailing blank line, untouched (#8236).
fn strip_single_markup_newline(raw: &str) -> &str {
    let raw = raw
        .strip_prefix("\r\n")
        .or_else(|| raw.strip_prefix('\n'))
        .unwrap_or(raw);
    raw.strip_suffix("\r\n")
        .or_else(|| raw.strip_suffix('\n'))
        .unwrap_or(raw)
}

/// Look up `properties.<name>.type` in a tool's JSON `input_schema`, when
/// both the schema and that property are declared.
fn schema_property_type<'a>(schema: Option<&'a serde_json::Value>, name: &str) -> Option<&'a str> {
    schema?.get("properties")?.get(name)?.get("type")?.as_str()
}

/// Find `tool_name`'s `input_schema` among the tools offered this turn. The
/// caller has already checked the name is in `available_tools` before
/// calling this, so the lookup is just borrowing the matching definition.
fn tool_input_schema<'a>(
    tool_name: &str,
    available_tools: &'a [ToolDefinition],
) -> Option<&'a serde_json::Value> {
    available_tools
        .iter()
        .find(|t| t.name == tool_name)
        .map(|t| &t.input_schema)
}

/// Type a raw parameter value the way the tool's input schema says to:
/// numbers and booleans are coerced only when `schema_type` says so;
/// everything else — including a `string`-typed property, and any
/// parameter the schema doesn't describe — stays a string (#8236).
///
/// Guessing the type from the value alone (the pre-#8236 behavior) silently
/// corrupted string parameters that happen to look numeric: a `chat_id`
/// like `"-1001234567890"` became a JSON integer and failed `serde`
/// deserialization on the other side (`invalid type: integer, expected a
/// string`); a `code` of `"01234"` lost its leading zeros; a 19+ digit id
/// lost precision by falling through to `f64`. `NaN`/`inf` parse as `f64` in
/// Rust but have no JSON form, so a `number`-typed property containing them
/// still falls back to a string rather than panicking or dropping the value.
fn coerce_scalar(raw: &str, schema_type: Option<&str>) -> serde_json::Value {
    match schema_type {
        Some("integer") => raw
            .parse::<i64>()
            .map_or_else(|_| serde_json::Value::String(raw.to_string()), Into::into),
        Some("number") => raw
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or_else(
                || serde_json::Value::String(raw.to_string()),
                serde_json::Value::Number,
            ),
        Some("boolean") => match raw {
            "true" => serde_json::Value::Bool(true),
            "false" => serde_json::Value::Bool(false),
            _ => serde_json::Value::String(raw.to_string()),
        },
        // "string", "array", "object", "null", any other schema type, or no
        // schema at all — verbatim string.
        _ => serde_json::Value::String(raw.to_string()),
    }
}

/// Parse a JSON object that represents a tool call.
/// Supports formats:
/// - `{"name":"tool","arguments":{"key":"value"}}`
/// - `{"name":"tool","parameters":{"key":"value"}}`
/// - `{"function":"tool","arguments":{"key":"value"}}`
/// - `{"tool":"tool_name","args":{"key":"value"}}`
pub(super) fn parse_json_tool_call_object(
    text: &str,
    tool_names: &[&str],
) -> Option<(String, serde_json::Value)> {
    let obj: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = obj.as_object()?;

    // Extract tool name from various field names
    let name = obj
        .get("name")
        .or_else(|| obj.get("function"))
        .or_else(|| obj.get("tool"))
        .and_then(|v| v.as_str())?;

    if !tool_names.contains(&name) {
        return None;
    }

    // Extract arguments from various field names
    let args = obj
        .get("arguments")
        .or_else(|| obj.get("parameters"))
        .or_else(|| obj.get("args"))
        .or_else(|| obj.get("input"))
        .cloned()
        .unwrap_or(serde_json::json!({}));

    // If arguments is a string (some models stringify it), try to parse it
    let args = if let Some(s) = args.as_str() {
        serde_json::from_str(s).unwrap_or(serde_json::json!({}))
    } else {
        args
    };

    Some((name.to_string(), args))
}

/// Parse the custom arrow syntax used by some Ollama models:
/// `{tool => "name", args => {--key "value"}}` or `{tool => "name", args => {"key":"value"}}`
fn parse_arrow_syntax_tool_call(
    text: &str,
    tool_names: &[&str],
) -> Option<(String, serde_json::Value)> {
    // Extract tool name: look for `tool => "name"` or `tool=>"name"`
    let tool_marker_pos = text.find("tool =>").or_else(|| text.find("tool=>"))?;
    let after_tool = &text[tool_marker_pos + 4..];
    // Skip whitespace and `=>`
    let after_arrow = after_tool.trim_start();
    let after_arrow = after_arrow.strip_prefix("=>")?;
    let after_arrow = after_arrow.trim_start();

    // Extract quoted tool name
    let tool_name = if let Some(stripped) = after_arrow.strip_prefix('"') {
        let end_quote = stripped.find('"')?;
        &stripped[..end_quote]
    } else {
        // Unquoted: take until comma, whitespace, or '}'
        let end = after_arrow
            .find(|c: char| c == ',' || c == '}' || c.is_whitespace())
            .unwrap_or(after_arrow.len());
        &after_arrow[..end]
    };

    if tool_name.is_empty() || !tool_names.contains(&tool_name) {
        return None;
    }

    // Extract args: look for `args => {` or `args=>{`
    let args_value = if let Some(args_pos) = text.find("args =>").or_else(|| text.find("args=>")) {
        let after_args = &text[args_pos + 4..];
        let after_args = after_args.trim_start();
        let after_args = after_args.strip_prefix("=>")?;
        let after_args = after_args.trim_start();

        if after_args.starts_with('{') {
            // Try standard JSON parse first
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(after_args) {
                v
            } else {
                // Parse `--key "value"` / `--key value` style args
                parse_dash_dash_args(after_args)
            }
        } else {
            serde_json::json!({})
        }
    } else {
        serde_json::json!({})
    };

    Some((tool_name.to_string(), args_value))
}

/// Parse `{--key "value", --flag}` or `{--command "ls -F /"}` style arguments
/// into a JSON object.
pub(super) fn parse_dash_dash_args(text: &str) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    // Strip outer braces — find matching close brace
    let inner = if text.starts_with('{') {
        let mut depth = 0;
        let mut end = text.len();
        for (i, c) in text.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        text[1..end].trim()
    } else {
        text.trim()
    };

    // Parse --key "value" or --key value pairs
    let mut remaining = inner;
    while let Some(dash_pos) = remaining.find("--") {
        remaining = &remaining[dash_pos + 2..];

        // Extract key: runs until whitespace, '=', '"', or end
        let key_end = remaining
            .find(|c: char| c.is_whitespace() || c == '=' || c == '"')
            .unwrap_or(remaining.len());
        let key = &remaining[..key_end];
        if key.is_empty() {
            continue;
        }
        remaining = &remaining[key_end..];
        remaining = remaining.trim_start();

        // Skip optional '='
        if remaining.starts_with('=') {
            remaining = remaining[1..].trim_start();
        }

        // Extract value
        if remaining.starts_with('"') {
            // Quoted value — find closing quote
            if let Some(end_quote) = remaining[1..].find('"') {
                let value = &remaining[1..1 + end_quote];
                map.insert(
                    key.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
                remaining = &remaining[2 + end_quote..];
            } else {
                // Unclosed quote — take rest
                let value = &remaining[1..];
                map.insert(
                    key.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
                break;
            }
        } else {
            // Unquoted value — take until next --, comma, }, or end
            let val_end = remaining
                .find([',', '}'])
                .or_else(|| remaining.find("--"))
                .unwrap_or(remaining.len());
            let value = remaining[..val_end].trim();
            if !value.is_empty() {
                map.insert(
                    key.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
            } else {
                // Flag with no value — set to true
                map.insert(key.to_string(), serde_json::Value::Bool(true));
            }
            remaining = &remaining[val_end..];
        }

        // Skip comma separator
        remaining = remaining.trim_start();
        if remaining.starts_with(',') {
            remaining = remaining[1..].trim_start();
        }
    }

    serde_json::Value::Object(map)
}

pub(super) fn find_json_object_end(text: &str) -> Option<usize> {
    let mut depth = 0u32;
    let mut in_string = false;
    let mut escaped = false;

    for (i, c) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else {
                match c {
                    '\\' => escaped = true,
                    '"' => in_string = false,
                    _ => {}
                }
            }
            continue;
        }

        match c {
            '"' => in_string = true,
            '{' => depth = depth.checked_add(1)?,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }

    None
}

/// Try to parse a bare JSON object as a tool call.
/// The JSON must have a "name"/"function"/"tool" field matching a known tool.
fn try_parse_bare_json_tool_call(
    text: &str,
    tool_names: &[&str],
) -> Option<(String, serde_json::Value)> {
    let end = find_json_object_end(text)?;
    parse_json_tool_call_object(&text[..end], tool_names)
}
