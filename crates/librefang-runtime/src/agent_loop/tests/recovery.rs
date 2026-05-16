use super::integration::{test_manifest, NormalDriver};
use super::*;

// --- Deep edge-case tests for text-to-tool recovery ---

#[test]
fn test_recover_text_tool_calls_nested_json() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search".into(),
        input_schema: serde_json::json!({}),
    }];
    let text =
        r#"<function=web_search>{"query":"rust","filters":{"lang":"en","year":2024}}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input["filters"]["lang"], "en");
}

#[test]
fn test_recover_text_tool_calls_with_surrounding_text() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "Sure, let me search that for you.\n\n<function=web_search>{\"query\":\"rust async programming\"}</function>\n\nI'll get back to you with results.";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input["query"], "rust async programming");
}

#[test]
fn test_recover_text_tool_calls_whitespace_in_json() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search".into(),
        input_schema: serde_json::json!({}),
    }];
    // Some models emit pretty-printed JSON
    let text = "<function=web_search>\n  {\"query\": \"hello world\"}\n</function>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input["query"], "hello world");
}

#[test]
fn test_recover_text_tool_calls_unclosed_tag() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search".into(),
        input_schema: serde_json::json!({}),
    }];
    // Missing </function> — should gracefully skip
    let text = r#"<function=web_search>{"query":"test"}"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty(), "Unclosed tag should be skipped");
}

#[test]
fn test_recover_text_tool_calls_missing_closing_bracket() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search".into(),
        input_schema: serde_json::json!({}),
    }];
    // Missing > after tool name
    let text = r#"<function=web_search{"query":"test"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    // The parser finds > inside JSON, will likely produce invalid tool name
    // or invalid JSON — either way, should not panic
    // (just verifying no panic / no bad behavior)
    let _ = calls;
}

#[test]
fn test_recover_text_tool_calls_empty_json_object() {
    let tools = vec![ToolDefinition {
        name: "list_files".into(),
        description: "List".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = r#"<function=list_files>{}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "list_files");
    assert_eq!(calls[0].input, serde_json::json!({}));
}

#[test]
fn test_recover_text_tool_calls_mixed_valid_invalid() {
    let tools = vec![
        ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        },
        ToolDefinition {
            name: "read_file".into(),
            description: "Read".into(),
            input_schema: serde_json::json!({}),
        },
    ];
    // First: valid, second: unknown tool, third: valid
    let text = r#"<function=web_search>{"q":"a"}</function> <function=unknown>{"x":1}</function> <function=read_file>{"path":"b"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 2, "Should recover 2 valid, skip 1 unknown");
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[1].name, "read_file");
}

// --- Variant 2 pattern tests: <function>NAME{JSON}</function> ---

#[test]
fn test_recover_variant2_basic() {
    let tools = vec![ToolDefinition {
        name: "web_fetch".into(),
        description: "Fetch".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = r#"<function>web_fetch{"url":"https://example.com"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_fetch");
    assert_eq!(calls[0].input["url"], "https://example.com");
}

#[test]
fn test_recover_variant2_unknown_tool() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = r#"<function>unknown_tool{"q":"test"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 0);
}

#[test]
fn test_recover_variant2_with_surrounding_text() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = r#"Let me search for that. <function>web_search{"query":"rust lang"}</function> I'll find the answer."#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
}

#[test]
fn test_recover_both_variants_mixed() {
    let tools = vec![
        ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        },
        ToolDefinition {
            name: "web_fetch".into(),
            description: "Fetch".into(),
            input_schema: serde_json::json!({}),
        },
    ];
    // Mix of variant 1 and variant 2
    let text = r#"<function=web_search>{"q":"a"}</function> <function>web_fetch{"url":"https://x.com"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[1].name, "web_fetch");
}

#[test]
fn test_recover_tool_tag_variant() {
    let tools = vec![ToolDefinition {
        name: "exec".into(),
        description: "Execute".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = r#"I'll run that for you. <tool>exec{"command":"ls -la"}</tool>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "exec");
    assert_eq!(calls[0].input["command"], "ls -la");
}

#[test]
fn test_recover_markdown_code_block() {
    let tools = vec![ToolDefinition {
        name: "exec".into(),
        description: "Execute".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "I'll execute that command:\n```\nexec {\"command\": \"ls -la\"}\n```";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "exec");
    assert_eq!(calls[0].input["command"], "ls -la");
}

#[test]
fn test_recover_markdown_code_block_with_lang() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "```json\nweb_search {\"query\": \"rust\"}\n```";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
}

#[test]
fn test_recover_backtick_wrapped() {
    let tools = vec![ToolDefinition {
        name: "exec".into(),
        description: "Execute".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = r#"Let me run `exec {"command":"pwd"}` for you."#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "exec");
    assert_eq!(calls[0].input["command"], "pwd");
}

#[test]
fn test_recover_backtick_ignores_unknown_tool() {
    let tools = vec![ToolDefinition {
        name: "exec".into(),
        description: "Execute".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = r#"Try `unknown_tool {"key":"val"}` instead."#;
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn test_recover_no_duplicates_across_patterns() {
    let tools = vec![ToolDefinition {
        name: "exec".into(),
        description: "Execute".into(),
        input_schema: serde_json::json!({}),
    }];
    // Same call in both function tag and tool tag — should only appear once
    let text = r#"<function=exec>{"command":"ls"}</function> <tool>exec{"command":"ls"}</tool>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
}

// --- Pattern 6: [TOOL_CALL]...[/TOOL_CALL] tests (issue #354) ---

#[test]
fn test_recover_tool_call_block_json() {
    let tools = vec![ToolDefinition {
        name: "shell_exec".into(),
        description: "Execute shell command".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "[TOOL_CALL]\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls -la\"}}\n[/TOOL_CALL]";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[0].input["command"], "ls -la");
}

#[test]
fn test_recover_tool_call_block_arrow_syntax() {
    let tools = vec![ToolDefinition {
        name: "shell_exec".into(),
        description: "Execute shell command".into(),
        input_schema: serde_json::json!({}),
    }];
    // Exact format from issue #354
    let text =
        "[TOOL_CALL]\n{tool => \"shell_exec\", args => {\n--command \"ls -F /\"\n}}\n[/TOOL_CALL]";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[0].input["command"], "ls -F /");
}

#[test]
fn test_recover_tool_call_block_unknown_tool() {
    let tools = vec![ToolDefinition {
        name: "shell_exec".into(),
        description: "Execute".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "[TOOL_CALL]\n{\"name\": \"hack_system\", \"arguments\": {\"cmd\": \"rm -rf /\"}}\n[/TOOL_CALL]";
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn test_recover_tool_call_block_multiple() {
    let tools = vec![
        ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        },
        ToolDefinition {
            name: "file_read".into(),
            description: "Read".into(),
            input_schema: serde_json::json!({}),
        },
    ];
    let text = "[TOOL_CALL]\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}\n[/TOOL_CALL]\nSome text.\n[TOOL_CALL]\n{\"name\": \"file_read\", \"arguments\": {\"path\": \"/tmp/test.txt\"}}\n[/TOOL_CALL]";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[1].name, "file_read");
}

#[test]
fn test_recover_tool_call_block_unclosed() {
    let tools = vec![ToolDefinition {
        name: "shell_exec".into(),
        description: "Execute".into(),
        input_schema: serde_json::json!({}),
    }];
    // Unclosed [TOOL_CALL] — pattern 6 skips it, but pattern 8 (bare JSON)
    // still finds the valid JSON tool call object.
    let text = "[TOOL_CALL]\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1, "Bare JSON fallback should recover this");
    assert_eq!(calls[0].name, "shell_exec");
}

// --- Pattern 7: <tool_call>JSON</tool_call> tests (Qwen3, issue #332) ---

#[test]
fn test_recover_tool_call_xml_basic() {
    let tools = vec![ToolDefinition {
        name: "shell_exec".into(),
        description: "Execute".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "<tool_call>\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls -la\"}}\n</tool_call>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[0].input["command"], "ls -la");
}

#[test]
fn test_recover_tool_call_xml_with_surrounding_text() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "I'll search for that.\n\n<tool_call>\n{\"name\": \"web_search\", \"arguments\": {\"query\": \"rust async\"}}\n</tool_call>\n\nLet me get results.";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[0].input["query"], "rust async");
}

#[test]
fn test_recover_tool_call_xml_function_field() {
    let tools = vec![ToolDefinition {
        name: "file_read".into(),
        description: "Read".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "<tool_call>{\"function\": \"file_read\", \"arguments\": {\"path\": \"/etc/hosts\"}}</tool_call>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "file_read");
}

#[test]
fn test_recover_tool_call_xml_parameters_field() {
    let tools = vec![ToolDefinition {
        name: "web_fetch".into(),
        description: "Fetch".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "<tool_call>{\"name\": \"web_fetch\", \"parameters\": {\"url\": \"https://example.com\"}}</tool_call>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_fetch");
    assert_eq!(calls[0].input["url"], "https://example.com");
}

#[test]
fn test_recover_tool_call_xml_stringified_args() {
    let tools = vec![ToolDefinition {
        name: "shell_exec".into(),
        description: "Execute".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "<tool_call>{\"name\": \"shell_exec\", \"arguments\": \"{\\\"command\\\": \\\"pwd\\\"}\"}</tool_call>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[0].input["command"], "pwd");
}

#[test]
fn test_recover_tool_call_xml_unknown_tool() {
    let tools = vec![ToolDefinition {
        name: "shell_exec".into(),
        description: "Execute".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "<tool_call>{\"name\": \"hack_system\", \"arguments\": {\"cmd\": \"rm -rf /\"}}</tool_call>";
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn test_recover_tool_call_xml_multiple() {
    let tools = vec![
        ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        },
        ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        },
    ];
    let text = "<tool_call>{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}</tool_call>\n<tool_call>{\"name\": \"web_search\", \"arguments\": {\"query\": \"rust\"}}</tool_call>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[1].name, "web_search");
}

// --- Pattern 8: Bare JSON tool call object tests ---

#[test]
fn test_recover_bare_json_tool_call() {
    let tools = vec![ToolDefinition {
        name: "shell_exec".into(),
        description: "Execute".into(),
        input_schema: serde_json::json!({}),
    }];
    let text =
        "I'll run that: {\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls -la\"}}";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[0].input["command"], "ls -la");
}

#[test]
fn test_recover_bare_json_no_false_positive() {
    let tools = vec![ToolDefinition {
        name: "shell_exec".into(),
        description: "Execute".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "The config looks like {\"debug\": true, \"level\": \"info\"}";
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn test_recover_bare_json_skipped_when_tags_found() {
    let tools = vec![ToolDefinition {
        name: "shell_exec".into(),
        description: "Execute".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "<function=shell_exec>{\"command\":\"ls\"}</function> {\"name\": \"shell_exec\", \"arguments\": {\"command\": \"pwd\"}}";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input["command"], "ls");
}

// --- Pattern 9: XML-attribute style <function name="..." parameters="..." /> ---

#[test]
fn test_recover_xml_attribute_basic() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = r#"<function name="web_search" parameters="{&quot;query&quot;: &quot;best crypto 2024&quot;}" />"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[0].input["query"], "best crypto 2024");
}

#[test]
fn test_recover_xml_attribute_unknown_tool() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = r#"<function name="unknown_tool" parameters="{&quot;x&quot;: 1}" />"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn test_recover_xml_attribute_non_selfclosing() {
    let tools = vec![ToolDefinition {
        name: "shell_exec".into(),
        description: "Execute".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = r#"<function name="shell_exec" parameters="{&quot;command&quot;: &quot;ls&quot;}"></function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell_exec");
}

// --- Helper function tests ---

#[test]
fn test_parse_dash_dash_args_basic() {
    let result = parse_dash_dash_args("{--command \"ls -F /\"}");
    assert_eq!(result["command"], "ls -F /");
}

#[test]
fn test_parse_dash_dash_args_multiple() {
    let result = parse_dash_dash_args("{--file \"test.txt\", --verbose}");
    assert_eq!(result["file"], "test.txt");
    assert_eq!(result["verbose"], true);
}

#[test]
fn test_parse_dash_dash_args_unquoted_value() {
    let result = parse_dash_dash_args("{--count 5}");
    assert_eq!(result["count"], "5");
}

#[test]
fn test_parse_json_tool_call_object_standard() {
    let tool_names = vec!["shell_exec"];
    let result = parse_json_tool_call_object(
        "{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}",
        &tool_names,
    );
    assert!(result.is_some());
    let (name, args) = result.unwrap();
    assert_eq!(name, "shell_exec");
    assert_eq!(args["command"], "ls");
}

#[test]
fn test_parse_json_tool_call_object_function_field() {
    let tool_names = vec!["web_fetch"];
    let result = parse_json_tool_call_object(
        "{\"function\": \"web_fetch\", \"parameters\": {\"url\": \"https://x.com\"}}",
        &tool_names,
    );
    assert!(result.is_some());
    let (name, args) = result.unwrap();
    assert_eq!(name, "web_fetch");
    assert_eq!(args["url"], "https://x.com");
}

#[test]
fn test_parse_json_tool_call_object_unknown_tool() {
    let tool_names = vec!["shell_exec"];
    let result =
        parse_json_tool_call_object("{\"name\": \"unknown\", \"arguments\": {}}", &tool_names);
    assert!(result.is_none());
}

// --- End-to-end integration test: text-as-tool-call recovery through agent loop ---

/// Mock driver that simulates a Groq/Llama model outputting tool calls as text.
/// Call 1: Returns text with `<function=web_search>...</function>` (EndTurn, no tool_calls)
/// Call 2: Returns a normal text response (after tool result is provided)
struct TextToolCallDriver {
    call_count: AtomicU32,
}

impl TextToolCallDriver {
    fn new() -> Self {
        Self {
            call_count: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl LlmDriver for TextToolCallDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let call = self.call_count.fetch_add(1, Ordering::Relaxed);
        if call == 0 {
            // Simulate Groq/Llama: tool call as text, not in tool_calls field
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: r#"Let me search for that. <function=web_search>{"query":"rust async"}</function>"#.to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![], // BUG: no tool_calls!
                usage: TokenUsage {
                    input_tokens: 20,
                    output_tokens: 15,
                    ..Default::default()
                },
            })
        } else {
            // After tool result, return normal response
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "Based on the search results, Rust async is great!".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 30,
                    output_tokens: 12,
                    ..Default::default()
                },
            })
        }
    }
}

#[tokio::test]
async fn test_text_tool_call_recovery_e2e() {
    // This is THE critical test: a model outputs a tool call as text,
    // the recovery code detects it, promotes it to ToolUse, executes the tool,
    // and the agent loop continues to produce a final response.
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(TextToolCallDriver::new());

    // Provide web_search as an available tool so recovery can match it
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search the web".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            }
        }),
    }];

    let result = run_agent_loop(
        &manifest,
        "Search for rust async programming",
        &mut session,
        &memory,
        driver,
        &tools,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None, // on_phase
        None, // media_engine
        None, // media_drivers
        None, // tts_engine
        None, // docker_config
        None, // hooks
        None, // context_window_tokens
        None, // process_manager
        None, // checkpoint_manager
        None, // process_registry
        None, // user_content_blocks
        None, // proactive_memory
        None, // context_engine
        None, // pending_messages
        &LoopOptions::default(),
    )
    .await
    .expect("Agent loop should complete");

    // The response should contain the second call's output, NOT the raw function tag
    assert!(
        !result.response.contains("<function="),
        "Response should not contain raw function tags, got: {:?}",
        result.response
    );
    assert!(
        result.iterations >= 2,
        "Should have at least 2 iterations (tool call + final response), got: {}",
        result.iterations
    );
    // Verify the final text response came through
    assert!(
        result.response.contains("search results") || result.response.contains("Rust async"),
        "Expected final response text, got: {:?}",
        result.response
    );
}

/// Mock driver that returns NO text-based tool calls — just normal text.
/// Verifies recovery does NOT interfere with normal flow.
#[tokio::test]
async fn test_normal_flow_unaffected_by_recovery() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(NormalDriver);

    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search the web".into(),
        input_schema: serde_json::json!({}),
    }];

    let result = run_agent_loop(
        &manifest,
        "Say hello",
        &mut session,
        &memory,
        driver,
        &tools, // tools available but not used
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None,
        None,
        None,
        None,
        None,
        None, // checkpoint_manager
        None, // process_registry
        None, // user_content_blocks
        None, // proactive_memory
        None, // context_engine
        None, // pending_messages
        &LoopOptions::default(),
    )
    .await
    .expect("Normal loop should complete");

    assert_eq!(result.response, "Hello from the agent!");
    assert_eq!(
        result.iterations, 1,
        "Normal response should complete in 1 iteration"
    );
}

// --- Streaming path: text-as-tool-call recovery ---

#[tokio::test]
async fn test_text_tool_call_recovery_streaming_e2e() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(TextToolCallDriver::new());

    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search the web".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            }
        }),
    }];

    let (tx, mut rx) = mpsc::channel(64);

    let result = run_agent_loop_streaming(
        &manifest,
        "Search for rust async programming",
        &mut session,
        &memory,
        driver,
        &tools,
        None,
        tx,
        None,
        None,
        None,
        None,
        None,
        None,
        None, // on_phase
        None, // media_engine
        None, // media_drivers
        None, // tts_engine
        None, // docker_config
        None, // hooks
        None, // context_window_tokens
        None, // process_manager
        None, // checkpoint_manager
        None, // process_registry
        None, // user_content_blocks
        None, // proactive_memory
        None, // context_engine
        None, // pending_messages
        &LoopOptions::default(),
    )
    .await
    .expect("Streaming loop should complete");

    // Same assertions as non-streaming
    assert!(
        !result.response.contains("<function="),
        "Streaming: response should not contain raw function tags, got: {:?}",
        result.response
    );
    assert!(
        result.iterations >= 2,
        "Streaming: should have at least 2 iterations, got: {}",
        result.iterations
    );

    // Drain the stream channel to verify events were sent
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    assert!(!events.is_empty(), "Should have received stream events");
}
