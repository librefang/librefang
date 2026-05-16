//! Meta-tools that operate on the tool catalog itself: `tool_load` looks
//! up a tool's full schema by name, `tool_search` does keyword search.
//!
//! Both consult the per-agent `available_tools` slice when available so
//! lazy-loadable non-builtin tools (MCP, skills) stay reachable after the
//! eager schema trim (#3044). When the slice is `None` they fall back to
//! `super::builtin_tool_definitions()` to keep legacy callers working.

use super::builtin_tool_definitions;
use librefang_types::tool::{ToolDefinition, ToolExecutionStatus, ToolResult};

/// Resolve the lookup pool for `tool_load` / `tool_search`.
///
/// - `Some(list)` — caller threaded the agent's granted tool list through
///   `ToolExecContext.available_tools`. Trust it as the source of truth
///   even when `list.is_empty()`. An empty slice means the agent has
///   been granted nothing; the meta-tools must report "not found" rather
///   than expose the full builtin catalog.
/// - `None` — caller didn't thread the granted list through (legacy
///   `execute_tool` paths: REST/MCP bridges, approval resume, unit tests).
///   Fall back to the builtin catalog so these code paths keep working.
fn meta_lookup_pool(available: Option<&[ToolDefinition]>) -> Vec<ToolDefinition> {
    match available {
        Some(list) => list.to_vec(),
        None => builtin_tool_definitions(),
    }
}

/// Meta-tool: load a tool's full schema by name (issue #3044). The returned
/// schema is both printed into `content` for the LLM to read AND attached as
/// `ToolResult.loaded_tool` so the agent loop can register it in the session's
/// lazy-load cache — making the tool callable on the next turn.
pub(super) fn tool_meta_load(
    input: &serde_json::Value,
    available_tools: Option<&[ToolDefinition]>,
) -> ToolResult {
    let name = input
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        return ToolResult::error(
            "".to_string(),
            "tool_load requires a 'name' string".to_string(),
        );
    }
    let pool = meta_lookup_pool(available_tools);
    match pool.into_iter().find(|t| t.name == name) {
        Some(def) => {
            let schema = serde_json::json!({
                "name": def.name,
                "description": def.description,
                "input_schema": def.input_schema,
            });
            let content = format!(
                "Loaded tool '{}'. Schema:\n{}\n\nYou can call this tool on your next turn.",
                def.name,
                serde_json::to_string_pretty(&schema).unwrap_or_else(|_| schema.to_string()),
            );
            ToolResult {
                tool_use_id: String::new(),
                content,
                is_error: false,
                status: ToolExecutionStatus::Completed,
                loaded_tool: Some(def),
                ..Default::default()
            }
        }
        None => ToolResult::error(
            String::new(),
            format!(
                "Unknown tool '{}'. Call tool_search(query) to find available tools.",
                name
            ),
        ),
    }
}

/// Meta-tool: search the tool catalog by keyword (issue #3044). Returns a
/// short list of matching tool names and one-line hints sourced from the
/// prompt_builder catalog.
pub(super) fn tool_meta_search(
    input: &serde_json::Value,
    available_tools: Option<&[ToolDefinition]>,
) -> ToolResult {
    let query = input
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if query.is_empty() {
        return ToolResult::error(
            String::new(),
            "tool_search requires a non-empty 'query' string".to_string(),
        );
    }
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .clamp(1, 50) as usize;

    // Tokenize query — any token in the tool name, description, or hint makes a hit.
    let tokens: Vec<&str> = query.split_whitespace().collect();
    let mut matches: Vec<(usize, String, String)> = Vec::new();
    for def in meta_lookup_pool(available_tools) {
        let name_lc = def.name.to_lowercase();
        let desc_lc = def.description.to_lowercase();
        let hint = crate::prompt_builder::tool_hint(&def.name);
        let hint_lc = hint.to_lowercase();
        let score = tokens.iter().fold(0usize, |acc, tok| {
            let tok = tok.trim();
            if tok.is_empty() {
                return acc;
            }
            acc + (name_lc.contains(tok) as usize) * 3
                + (hint_lc.contains(tok) as usize) * 2
                + (desc_lc.contains(tok) as usize)
        });
        if score > 0 {
            matches.push((score, def.name, hint.to_string()));
        }
    }
    matches.sort_by_key(|m| std::cmp::Reverse(m.0));
    matches.truncate(limit);

    if matches.is_empty() {
        return ToolResult::ok(
            String::new(),
            format!(
                "No tools matched '{}'. Browse the tool catalog in the system prompt.",
                query
            ),
        );
    }
    let lines: Vec<String> = matches
        .into_iter()
        .map(|(_, name, hint)| {
            if hint.is_empty() {
                name
            } else {
                format!("{name}: {hint}")
            }
        })
        .collect();
    ToolResult::ok(
        String::new(),
        format!(
            "Matches for '{}' (call tool_load(name) to get a tool's schema):\n{}",
            query,
            lines.join("\n")
        ),
    )
}
