/** Tool-grant semantics mirrored from the kernel, for read-only display.
 *
 * The kernel grants an agent's tools through two independent paths (`LibreFangKernel::available_tools`, `tools_and_skills.rs`):
 *
 * - **Builtin tools** are filtered by `capabilities.tools` (the dashboard's `capabilities_tools`). An empty list — or one containing `"*"` — means unrestricted.
 * - **MCP tools** are granted by the agent's `mcp_servers` allowlist and are explicitly NOT filtered by `capabilities.tools`: "MCP tool names are dynamic and unknown at agent-definition time. Use tool_blocklist to restrict specific MCP tools if needed." An empty `mcp_servers` grants no server; `["*"]` grants every connected server (#5855).
 *
 * `tool_allowlist` and then `tool_blocklist` filter whatever survived, on both paths, with glob patterns (#6495). Both run in the kernel's Step 4, after MCP tools have joined the candidate set, so a non-empty allowlist naming no `mcp__*` glob strips a granted server outright — the quoted advice above predates #6495 and is no longer the whole story.
 *
 * Deriving MCP group state from `capabilities_tools` (as the Tools tab did before #6565) therefore always reported a whole-server grant as unassigned, even while the agent was actively calling those tools.
 */

/** MCP grant classification emitted by `GET /api/agents/{id}`. */
export type McpGrantMode = "all" | "allowlist" | "none";

/** Mirror of `librefang_runtime::mcp::normalize_name`. */
export function normalizeMcpName(name: string): string {
  return name.toLowerCase().replace(/-/g, "_");
}

/** Mirror of `librefang_types::capability::glob_matches` for tool names.
 *
 * Tool names carry no `/`, `\` or `.`, so only the separator-free branch (`glob_matches_simple`) is reachable for them: `*` matches any run of characters, a bare `*` matches everything, and a pattern with no wildcard must match exactly.
 */
export function toolPatternMatches(pattern: string, value: string): boolean {
  if (pattern === "*") return true;
  if (pattern === value) return true;
  if (!pattern.includes("*")) return false;

  const parts = pattern.split("*");
  // Leading literal must be a prefix.
  const first = parts[0];
  if (first && !value.startsWith(first)) return false;
  // Trailing literal must be a suffix.
  const last = parts[parts.length - 1];
  if (last && !value.endsWith(last)) return false;
  // Interior literals must appear in order, and the prefix/suffix must not overlap — `a*b` should not match `ab` twice over the same characters.
  let cursor = first.length;
  for (const segment of parts.slice(1, -1)) {
    if (!segment) continue;
    const at = value.indexOf(segment, cursor);
    if (at === -1) return false;
    cursor = at + segment.length;
  }
  return cursor + last.length <= value.length;
}

/** Whether `toolName` is filtered out by a `tool_blocklist`. */
export function isToolBlocked(toolName: string, blocklist: readonly string[]): boolean {
  return blocklist.some((pattern) => toolPatternMatches(pattern, toolName));
}

/** Whether `toolName` survives a `tool_allowlist`.
 *
 * An empty list is unrestricted, mirroring the kernel's `if !tool_allowlist.is_empty()` guard (`tools_and_skills.rs`, Step 4).
 * That step runs after MCP tools have been pushed into the candidate set, so the allowlist filters MCP tools too — a non-empty list naming no `mcp__*` glob strips the whole server (#6495).
 */
export function isToolAllowed(toolName: string, allowlist: readonly string[]): boolean {
  return allowlist.length === 0 || allowlist.some((pattern) => toolPatternMatches(pattern, toolName));
}

/** Resolve the MCP grant mode from the raw allowlist when the backend field is absent (older daemon), mirroring `routes::agents::mcp_servers_mode`. */
export function resolveMcpGrantMode(
  mcpServers: readonly string[] | undefined,
  mode: McpGrantMode | undefined,
): McpGrantMode {
  if (mode) return mode;
  const servers = mcpServers ?? [];
  if (servers.length === 0) return "none";
  if (servers.some((s) => s === "*")) return "all";
  return "allowlist";
}

/** Whether one MCP server is granted to the agent. */
export function isMcpServerGranted(
  server: string,
  mcpServers: readonly string[] | undefined,
  mode: McpGrantMode,
): boolean {
  if (mode === "none") return false;
  if (mode === "all") return true;
  const target = normalizeMcpName(server);
  return (mcpServers ?? []).some((s) => normalizeMcpName(s) === target);
}
