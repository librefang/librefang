import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listMemories,
  searchMemories,
  getMemoryStats,
  getMemoryConfig,
  type MemoryItem,
} from "../http/client";
import { healthDetailQueryOptions } from "./runtime";
import { memoryKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const REFRESH_MS = 30_000;
const STALE_MS = 30_000;
const CONFIG_STALE_MS = 300_000;

export const memoryQueries = {

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



export const memorySearchOrListQueryOptions = (search: string) =>
  queryOptions<{ memories: MemoryItem[]; total: number }>({
    queryKey: memoryKeys.searchOrList(search),
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

/**
 * Server-side liveness signal for the embedding subsystem.
 *
 * Reads the `memory.embedding_available` field from `/api/health/detail`,
 * which is populated by a server-side probe (validates provider wiring / keys).
 * This is NOT the same as "is a provider configured" — see `useMemoryConfig`
 * for the config-only view. A provider string can be truthy while the server
 * probe still returns `embedding_available: false` (bad key, provider down).
 *
 * Shares cache with `useHealthDetail` via the same `queryKey`; `select`
 * narrows the returned data so consumers of this hook don't re-render on
 * unrelated health field changes.
 */
export function useMemoryHealth(options: QueryOverrides = {}) {
  return useQuery({
    ...withOverrides(healthDetailQueryOptions(), options),
    select: (data): boolean => data.memory?.embedding_available ?? false,
  });
}
