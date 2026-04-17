import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  testProvider,
  setProviderKey,
  deleteProviderKey,
  setProviderUrl,
  setDefaultProvider,
} from "../../api";
import { providerKeys } from "../queries/keys";

export function useTestProvider() {
  return useMutation({
    mutationFn: testProvider,
  });
}

export function useSetProviderKey() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, key }: { id: string; key: string }) =>
      setProviderKey(id, key),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: providerKeys.all });
    },
  });
}

export function useDeleteProviderKey() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => deleteProviderKey(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: providerKeys.all });
    },
  });
}

export function useSetProviderUrl() {
  const queryClient = useQueryClient();
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
      queryClient.invalidateQueries({ queryKey: providerKeys.all });
    },
  });
}

export function useSetDefaultProvider() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, model }: { id: string; model?: string }) =>
      setDefaultProvider(id, model),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: providerKeys.all });
    },
  });
}
