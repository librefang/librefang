import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  testProvider,
  setProviderKey,
  deleteProviderKey,
  enableProvider,
  setProviderUrl,
  setDefaultProvider,
  createRegistryContent,
} from "../http/client";
import { modelKeys, providerKeys, runtimeKeys } from "../queries/keys";

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
 * POST /registry/content/{contentType} — generic registry content creation.
 *
 * Today the only call site is the "Add provider" wizard on ProvidersPage,
 * which writes a `provider` content entry. We invalidate `providerKeys.all`
 * (list refresh) and `modelKeys.lists()` (a new provider may surface new
 * models on the next list fetch) for that case. Other content types are
 * accepted but currently invalidate the same scoped slices because no other
 * caller exists yet — extend here when a non-provider call site lands.
 */
export function useCreateRegistryContent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      contentType,
      values,
    }: {
      contentType: string;
      values: Record<string, unknown>;
    }) => createRegistryContent(contentType, values),
    onSuccess: (_data, variables) => {
      if (variables.contentType === "provider") {
        qc.invalidateQueries({ queryKey: providerKeys.all });
        qc.invalidateQueries({ queryKey: modelKeys.lists() });
      }
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
      await createRegistryContent("provider", {
        id: EVERYAPI_PROVIDER.id,
        display_name: EVERYAPI_PROVIDER.displayName,
        api_key_env: EVERYAPI_PROVIDER.apiKeyEnv,
        base_url: trimmedBase || EVERYAPI_PROVIDER.defaultBaseUrl,
        key_required: true,
        models: [],
      });
      await setProviderKey(EVERYAPI_PROVIDER.id, trimmedKey);
    },
    onSettled: () => {
      qc.invalidateQueries({ queryKey: providerKeys.all });
      qc.invalidateQueries({ queryKey: modelKeys.lists() });
    },
  });
}

const TEST_SUCCESS_STATUSES = new Set(["ok", "success"]);

export function useValidateProviderKey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async ({
      providerId,
      apiKey,
      requiresKey,
    }: {
      providerId: string;
      apiKey: string;
      requiresKey: boolean;
    }) => {
      if (!providerId) throw new Error("no_provider");
      if (requiresKey && apiKey.trim()) {
        await setProviderKey(providerId, apiKey.trim());
      }
      const test = await testProvider(providerId);
      if (!TEST_SUCCESS_STATUSES.has(test.status ?? "")) {
        throw new Error(test.message || "test_failed");
      }
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: providerKeys.all });
      qc.invalidateQueries({ queryKey: modelKeys.lists() });
    },
  });
}
