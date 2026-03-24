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
