import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  testProvider,
  setProviderKey,
  deleteProviderKey,
  setProviderUrl,
  setDefaultProvider,
} from "../http/client";
import { modelKeys, providerKeys, runtimeKeys } from "../queries/keys";

// Fire-and-forget: one-shot probe, test result returned to caller, no cache to invalidate.
export function useTestProvider() {
  return useMutation({
    mutationFn: testProvider,
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
