//! MCP system-prompt summary rendering. The boundary where the MCP server
//! registry crosses into the LLM prompt — determinism is load-bearing here
//! (see issue #3298). Kept out of `mod.rs` so the unit tests can exercise
//! it without instantiating a full kernel.

/// Build a deterministic cache key for the per-agent MCP allowlist; sorts and joins with `\x1f` so insertion-order variants share one entry.
pub(super) fn mcp_summary_cache_key(mcp_allowlist: &[String]) -> String {
    if mcp_allowlist.is_empty() {
        return String::from("*");
    }
    let mut sorted = mcp_allowlist.to_vec();
    sorted.sort();
    sorted.join("\x1f")
}

/// Render the MCP-server tool summary that lands in the system prompt.
///
/// Pulled out of [`Kernel::build_mcp_summary`] so it can be unit-tested
/// without instantiating a full kernel. Determinism is load-bearing:
///
/// - Servers are grouped in a `BTreeMap` so the outer iteration order is
///   lexicographic, not HashMap-random across processes.
/// - Each server's tool list is sorted before joining — `tools_in` carries
///   MCP-server-connect order which varies run-to-run and would otherwise
///   defeat provider prompt caching even when the underlying tool set is
///   identical.
///
/// See issue #3298 and the regression test
/// `tests::mcp_summary_is_byte_identical_across_input_orders` below.
pub(super) fn render_mcp_summary(
    tools_in: &[String],
    configured_servers: &[String],
    mcp_allowlist: &[String],
) -> String {
    if tools_in.is_empty() {
        return String::new();
    }

    let normalized: Vec<String> = mcp_allowlist
        .iter()
        .map(|s| librefang_runtime::mcp::normalize_name(s))
        .collect();

    let mut servers: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut tool_count = 0usize;
    for tool_name in tools_in {
        if let Some(server_name) = librefang_runtime::mcp::resolve_mcp_server_from_known(
            tool_name,
            configured_servers.iter().map(String::as_str),
        ) {
            let normalized_server = librefang_runtime::mcp::normalize_name(server_name);
            if !mcp_allowlist.is_empty() && !normalized.iter().any(|n| n == &normalized_server) {
                continue;
            }
            if let Some(raw_tool_name) =
                tool_name.strip_prefix(&format!("mcp_{}_", normalized_server))
            {
                servers
                    .entry(normalized_server)
                    .or_default()
                    .push(raw_tool_name.to_string());
            } else {
                servers
                    .entry(normalized_server)
                    .or_default()
                    .push(tool_name.clone());
            }
        } else {
            servers
                .entry("unknown".to_string())
                .or_default()
                .push(tool_name.clone());
        }
        tool_count += 1;
    }
    if tool_count == 0 {
        return String::new();
    }
    // Sort each server's tool list so the rendered summary is byte-stable
    // across processes — see function-level docs.
    for tool_names in servers.values_mut() {
        tool_names.sort();
    }
    let mut summary = format!("\n\n--- Connected MCP Servers ({} tools) ---\n", tool_count);
    for (server, tool_names) in &servers {
        summary.push_str(&format!(
            "- {server}: {} tools ({})\n",
            tool_names.len(),
            tool_names.join(", ")
        ));
    }
    summary.push_str("MCP tools are prefixed with mcp_{server}_ and work like regular tools.\n");
    let has_filesystem = servers.keys().any(|s| s.contains("filesystem"));
    if has_filesystem {
        summary.push_str(
            "IMPORTANT: For accessing files OUTSIDE your workspace directory, you MUST use \
             the MCP filesystem tools (e.g. mcp_filesystem_read_file, mcp_filesystem_list_directory) \
             instead of the built-in file_read/file_list/file_write tools, which are restricted to \
             the workspace. The MCP filesystem server has been granted access to specific directories \
             by the user.",
        );
    }
    summary
}
