import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  testProvider,
  setProviderKey,
  deleteProviderKey,
  enableProvider,
  setProviderUrl,
  setProviderDiscovery,
  setDefaultProvider,
  createRegistryContent,
} from "../http/client";
import { modelKeys, providerKeys, runtimeKeys } from "../queries/keys";

export class ProviderProbeError extends Error {
  constructor(public readonly status: string, message?: string) {
    super(message || "test_failed");
    this.name = "ProviderProbeError";
  }
}

export type EveryApiConnectPhase = "create" | "key_after_create";

export class EveryApiConnectError extends Error {
  public readonly cause: unknown;

  constructor(public readonly phase: EveryApiConnectPhase, cause: unknown) {
    const detail = cause instanceof Error && cause.message ? `: ${cause.message}` : "";
    const summary = phase === "create"
      ? "EveryAPI provider creation failed"
      : "EveryAPI provider was created, but its relay key could not be saved";
    super(`${summary}${detail}`);
    this.name = "EveryApiConnectError";
    this.cause = cause;
  }
}

// Probes the provider and persists `latency_ms` + `last_tested` on the
// kernel side, so callers must refetch the provider list to see the new
// values. Use `onSettled` (not `onSuccess`) because the backend records the
// timestamp even on probe failure (`result.ok === false` with HTTP 200) and
// the dashboard surfaces that "last attempted" timing too.
export function useTestProvider() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: testProvider,
    onSettled: () => {
      qc.invalidateQueries({ queryKey: providerKeys.all });
    },
  });
}

export function useSetProviderKey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, key }: { id: string; key: string }) =>
      setProviderKey(id, key),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: providerKeys.all });
      qc.invalidateQueries({ queryKey: modelKeys.lists() });
    },
  });
}

export function useDeleteProviderKey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => deleteProviderKey(id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: providerKeys.all });
      qc.invalidateQueries({ queryKey: modelKeys.lists() });
    },
  });
}

// Counterpart to `useDeleteProviderKey` — the dashboard's only way back
// for CLI providers (claude-code, codex-cli, gemini-cli, qwen-code) that
// have no key/URL to set. For non-CLI providers, the existing
// set-key/set-url flows already un-suppress, but this hook is the
// one-click "Re-enable" entry point that works uniformly. Invalidates
// the same slices as the delete counterpart so the picker / configured
// grid both refetch.
export function useEnableProvider() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => enableProvider(id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: providerKeys.all });
      qc.invalidateQueries({ queryKey: modelKeys.lists() });
    },
  });
}

export function useSetProviderUrl() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      id,
      baseUrl,
      proxyUrl,
    }: {
      id: string;
      baseUrl: string;
      proxyUrl?: string;
    }) => setProviderUrl(id, baseUrl, proxyUrl),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: providerKeys.all });
      qc.invalidateQueries({ queryKey: modelKeys.lists() });
    },
  });
}

/**
 * PUT /providers/{id}/discovery — opt a provider in or out of live model
 * discovery (#6702).
 *
 * Invalidates the model lists as well as the provider slice: turning discovery
 * on makes the next probe merge the endpoint's `/v1/models` listing into the
 * catalog, and turning it off stops refreshing it — either way the Models page
 * is showing a stale set until it refetches.
 */
export function useSetProviderDiscovery() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, discoverModels }: { id: string; discoverModels: boolean }) =>
      setProviderDiscovery(id, discoverModels),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: providerKeys.all });
      qc.invalidateQueries({ queryKey: modelKeys.lists() });
    },
  });
}

/**
 * POST /registry/content/provider — provider registry creation.
 *
 * This hook is intentionally provider-specific even though the transport API
 * accepts arbitrary registry content. A future content type must define its
 * own cache contract instead of silently inheriting provider invalidation.
 */
export function useCreateRegistryContent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      contentType,
      values,
    }: {
      contentType: "provider";
      values: Record<string, unknown>;
    }) => createRegistryContent(contentType, values),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: providerKeys.all });
      qc.invalidateQueries({ queryKey: modelKeys.lists() });
    },
  });
}

export function useSetDefaultProvider() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, model }: { id: string; model?: string }) =>
      setDefaultProvider(id, model),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: providerKeys.all });
      qc.invalidateQueries({ queryKey: modelKeys.lists() });
      qc.invalidateQueries({ queryKey: runtimeKeys.status() });
    },
  });
}

/**
 * EveryAPI's provider identity, mirroring the constants the CLI writes.
 *
 * These are a cross-language contract, not dashboard-local choices: `librefang models connect everyapi` writes the same values into `providers/everyapi.toml`, and the daemon keys its catalog refresh off the provider id.
 * Changing any of them here without changing `PROVIDER_ID` / `PROVIDER_DISPLAY_NAME` / `API_KEY_ENV` in `crates/librefang-cli/src/commands/everyapi.rs` would make the dashboard register a provider the daemon never refreshes.
 */
export const EVERYAPI_PROVIDER = {
  id: "everyapi",
  displayName: "EveryAPI",
  apiKeyEnv: "EVERYAPI_API_KEY",
  /**
   * The OpenAI-compatible root, `/v1` segment included, because every consumer appends onto it verbatim: the driver adds `/chat/completions` (`crates/librefang-llm-drivers/src/drivers/openai.rs`) and the daemon's catalog refresh adds `/models` (`crates/librefang-api/src/everyapi_catalog.rs`).
   * Nothing appends `/v1` on the way through — `derive_base_url` in `crates/librefang-cli/src/commands/everyapi.rs` bakes it into the value the CLI stores, so the dashboard has to store the same shape or the entry registers with a catalog that can never load.
   */
  defaultBaseUrl: "https://api.everyapi.ai/v1",
} as const;

/**
 * Register the EveryAPI gateway as a provider and store its relay key.
 *
 * EveryAPI is not a built-in provider, so it never appears in the Add picker's catalog until an entry exists — which is why the dashboard needs an explicit connect action rather than the usual "pick from the list, then fill in a key" flow.
 *
 * The entry is registered with an empty `models` array on purpose.
 * The daemon fills the catalog itself: `catalog_needs_initial_refresh` in `crates/librefang-api/src/everyapi_catalog.rs` is true exactly when the provider is configured but has no live models, and `refresh_if_missing_in_background` then fetches `/v1/models` and `/api/pricing` and synthesises the entries.
 * Duplicating that synthesis in the browser would mean a second implementation of the pricing rules to keep in sync, and the gateway is not CORS-open to the dashboard anyway.
 *
 * Ordering matters: the provider entry is written first, because the key endpoint addresses a provider by id and there is nothing to attach a key to until the entry exists.
 *
 * Invalidation is on `onSettled`, not `onSuccess`, because the two writes are not atomic.
 * When `createRegistryContent` succeeds and `setProviderKey` then throws, the mutation reports failure while the daemon already holds a keyless `everyapi` entry.
 * Invalidating only on success would leave the Providers page rendering its cached "not present" state, so it would keep offering "Connect EveryAPI gateway" and a retry would re-`POST` an id that already exists.
 */
export function useConnectEveryApi() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async ({
      relayKey,
      baseUrl,
    }: {
      relayKey: string;
      baseUrl?: string;
    }) => {
      const trimmedKey = relayKey.trim();
      if (!trimmedKey) throw new Error("empty_relay_key");
      const trimmedBase = (baseUrl ?? "").trim();
      try {
        await createRegistryContent("provider", {
          id: EVERYAPI_PROVIDER.id,
          display_name: EVERYAPI_PROVIDER.displayName,
          api_key_env: EVERYAPI_PROVIDER.apiKeyEnv,
          base_url: trimmedBase || EVERYAPI_PROVIDER.defaultBaseUrl,
          key_required: true,
          models: [],
        });
      } catch (error) {
        throw new EveryApiConnectError("create", error);
      }
      try {
        await setProviderKey(EVERYAPI_PROVIDER.id, trimmedKey);
      } catch (error) {
        throw new EveryApiConnectError("key_after_create", error);
      }
    },
    onSettled: () => {
      qc.invalidateQueries({ queryKey: providerKeys.all });
      qc.invalidateQueries({ queryKey: modelKeys.lists() });
    },
  });
}

const TEST_SUCCESS_STATUSES = new Set(["ok", "success"]);

/**
 * Persist a typed key (when there is one) and then probe the provider.
 *
 * The save is gated on the key being non-empty, NOT on the provider declaring
 * `key_required` (#6703). A `key_required: false` provider — every built-in
 * local one — may still sit behind an authenticating server, and the runtime
 * forwards whatever key is stored as `Authorization: Bearer`; dropping the key
 * here because the provider "doesn't need one" silently discarded what the
 * user typed and left the probe 401ing.
 */
export function useValidateProviderKey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async ({
      providerId,
      apiKey,
    }: {
      providerId: string;
      apiKey: string;
    }) => {
      if (!providerId) throw new Error("no_provider");
      if (apiKey.trim()) {
        await setProviderKey(providerId, apiKey.trim());
      }
      const test = await testProvider(providerId);
      if (!TEST_SUCCESS_STATUSES.has(test.status ?? "")) {
        throw new ProviderProbeError(test.status ?? "unknown", test.message);
      }
    },
    onSettled: () => {
      qc.invalidateQueries({ queryKey: providerKeys.all });
      qc.invalidateQueries({ queryKey: modelKeys.lists() });
    },
  });
}
