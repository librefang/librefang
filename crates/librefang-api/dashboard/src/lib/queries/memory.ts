import { queryOptions, useQuery } from "@tanstack/react-query";
import { listMemories, getMemoryStats, getMemoryConfig } from "../http/client";
import { memoryKeys } from "./keys";

const REFRESH_MS = 30_000;
const STALE_MS = 30_000;
const CONFIG_STALE_MS = 300_000;

export const memoryQueries = {
  list: (params?: { agentId?: string; offset?: number; limit?: number; category?: string }) =>
    queryOptions({
      queryKey: memoryKeys.list(params),
      queryFn: () => listMemories(params),
      staleTime: STALE_MS,
    }),
  stats: (agentId?: string) =>
    queryOptions({
      queryKey: memoryKeys.stats(agentId),
      queryFn: () => getMemoryStats(agentId),
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS * 2,
    }),
  config: () =>
    queryOptions({
      queryKey: memoryKeys.config(),
      queryFn: getMemoryConfig,
      staleTime: CONFIG_STALE_MS,
    }),
};

export function useMemories(params?: { agentId?: string; offset?: number; limit?: number; category?: string }) {
  return useQuery(memoryQueries.list(params));
}

export function useMemoryStats(agentId?: string) {
  return useQuery(memoryQueries.stats(agentId));
}

export function useMemoryConfig() {
  return useQuery(memoryQueries.config());
}
