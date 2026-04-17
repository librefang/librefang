import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listMemories,
  searchMemories,
  getMemoryStats,
  getMemoryConfig,
  type MemoryItem,
} from "../http/client";
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

export const memorySearchOrListQueryOptions = (search: string) =>
  queryOptions<{ memories: MemoryItem[]; total: number }>({
    queryKey: [...memoryKeys.lists(), "searchOrList", search] as const,
    queryFn: async () => {
      if (search.trim()) {
        const items = await searchMemories({ query: search.trim(), limit: 50 });
        return { memories: items, total: items.length };
      }
      const res = await listMemories({ offset: 0, limit: 10000 });
      return { memories: res.memories ?? [], total: res.total ?? 0 };
    },
    staleTime: STALE_MS,
    refetchInterval: REFRESH_MS,
  });

export function useMemorySearchOrList(search: string) {
  return useQuery(memorySearchOrListQueryOptions(search));
}

export function useMemoryStats(agentId?: string) {
  return useQuery(memoryQueries.stats(agentId));
}

export function useMemoryConfig() {
  return useQuery(memoryQueries.config());
}
