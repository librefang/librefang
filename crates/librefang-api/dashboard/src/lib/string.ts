/**
 * Truncate a UUID or ID string with ellipsis.
 * e.g. truncateId("550e8400-e29b-41d4-a716-446655440000", 8) → "550e8400…"
 */
export function truncateId(id: string | undefined | null, length = 8): string {
  if (!id) return "-";
  if (id.length <= length) return id;
  return `${id.slice(0, length)}…`;
}

/**
 * Truncate a string with ellipsis if it exceeds maxLength.
 */
export function truncate(str: string | undefined | null, maxLength: number): string {
  if (!str) return "-";
  if (str.length <= maxLength) return str;
  return `${str.slice(0, maxLength)}…`;
}

/**
 * Convert a snake_case / kebab-case / dotted tool ID into a human-readable
 * display name. Mirrors the channel progress prettifier so tool names look
 * the same in chat replies and the dashboard's ToolCallCard header.
 *
 * Words already containing uppercase letters keep their case after the first
 * char (so "MCP_call" → "MCP Call", not "Mcp Call").
 */
export function prettifyToolName(name: string | null | undefined): string {
  if (!name) return "tool";
  return name
    .split(/[_\-.]/)
    .filter(Boolean)
    .map(word => word[0].toUpperCase() + word.slice(1))
    .join(" ");
}
