import type { BadgeVariant } from "../components/ui/Badge";

const AVAILABLE_PROVIDERS = new Set([
  "configured",
  "validated_key",
  "not_required",
  "configured_cli",
  "auto_detected",
]);

const STATUS_VARIANT_MAP = new Map<string, BadgeVariant>([
  ["running", "success"],
  ["suspended", "warning"],
  ["idle", "warning"],
  ["error", "error"],
  ["crashed", "error"],
]);

/**
 * Map an agent/task status string to a Badge variant.
 */
export function getStatusVariant(status?: string): BadgeVariant {
  const value = (status ?? "").toLowerCase();
  return STATUS_VARIANT_MAP.get(value) ?? "default";
}

/** Check if a provider auth_status indicates the provider is usable.
 *  Mirrors the Rust AuthStatus::is_available() variants. */
export function isProviderAvailable(status?: string): boolean {
  return !!status && AVAILABLE_PROVIDERS.has(status);
}
