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

/** Check if a provider auth_status indicates the provider is usable.
 *  Mirrors the Rust AuthStatus::is_available() variants. */
export function isProviderAvailable(status?: string): boolean {
  return status === "configured" || status === "validated_key" || status === "not_required" || status === "configured_cli";
}
