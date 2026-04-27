import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  testProvider,
  setProviderKey,
  deleteProviderKey,
  setProviderUrl,
  setDefaultProvider,
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
