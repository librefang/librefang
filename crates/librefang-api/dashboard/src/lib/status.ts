import type { BadgeVariant } from "../components/ui/Badge";

/**
 * Map an agent/task status string to a Badge variant.
 */
export function getStatusVariant(status?: string): BadgeVariant {
  const value = (status ?? "").toLowerCase();
  if (value === "running") return "success";
  if (value === "suspended" || value === "idle") return "warning";
  if (value === "error" || value === "crashed") return "error";
  return "default";
}
