# [Medium] `parse_tool_args` brace counter doesn't handle JSON `\\` or `\u…` escapes correctly

**Severity:** Medium · **Domain:** LLM driver & MCP · **Source:** `audit-08-llm-mcp.md`
**Verification (re-audit 2026-05-18): DISPUTED.** `crates/librefang-llm-drivers/src/drivers/openai.rs:2410-2443` actually does maintain an `in_string` state that toggles on unescaped `"` and advances past one character after `\`. It only counts `{`/`}` when `!in_string`. Both audit-named failure cases — `{"text":"use {literal braces}"}` and `"A"`-style escapes — parse correctly under the current implementation. The "audit-named failure cases" do not actually break the parser. Switching to `serde_json::Deserializer` streaming mode is still a defensible refactor for robustness against future unknown corner cases, but it's a nicety, not a bug fix.

## Location
`crates/librefang-llm-drivers/src/drivers/openai.rs:2410-2443`

## Problem
The custom brace counter treats every `{` and `}` as a depth marker. JSON strings containing literal `{` / `}` inside quoted strings, or escaped `\\`/`{` sequences, throw the depth off. The fast path masks most cases (well-formed responses), but adversarially crafted tool args or rare provider quirks parse incorrectly.

## Fix
Use the standard JSON parser's streaming mode:
```rust
let mut de = serde_json::Deserializer::from_str(&buffer).into_iter::<Value>();
if let Some(Ok(value)) = de.next() {
    // bytes_consumed = de.byte_offset()
    return Some((value, de.byte_offset()));
}
```

## Tests
- Tool arg containing `"text": "use {literal braces}"` parses correctly.
- Tool arg with `{` escapes parses correctly.
