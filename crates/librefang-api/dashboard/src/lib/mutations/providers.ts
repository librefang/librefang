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
