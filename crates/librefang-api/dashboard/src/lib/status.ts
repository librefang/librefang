import type { BadgeVariant } from "../components/ui/Badge";
import type { ProviderItem } from "../api";

export const AVAILABLE_PROVIDER_STATUSES = [
  "configured",
  "validated_key",
  "not_required",
  "configured_cli",
  "auto_detected",
] as const;

const AVAILABLE_PROVIDERS = new Set<string>(AVAILABLE_PROVIDER_STATUSES);

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
 *  Keep `AVAILABLE_PROVIDER_STATUSES` synchronized with
 *  `AuthStatus::is_available()` in
 *  `crates/librefang-types/src/model_catalog.rs`; the source-contract test
 *  fails when the Rust variant set changes. */
export function isProviderAvailable(status?: string): boolean {
  return !!status && AVAILABLE_PROVIDERS.has(status.toLowerCase());
}

/** Check whether a provider is a coding-agent CLI passthrough (claude-code,
 *  codex-cli, gemini-cli, qwen-code, codewhale) rather than an HTTP endpoint.
 *
 *  Such a provider spawns a subprocess and has no base URL, so there is nothing
 *  to point a base URL / API key / model-discovery probe at. Both the Providers
 *  page and the onboarding wizard need the distinction, which is why it lives
 *  here next to `isProviderAvailable` rather than in one of them. */
export function isCliProvider(
  provider: Pick<ProviderItem, "auth_status" | "base_url" | "key_required">,
): boolean {
  return provider.auth_status === "configured_cli"
    || provider.auth_status === "cli_not_installed"
    || (!provider.base_url && !provider.key_required);
}
