import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listMemories,
  searchMemories,
  getMemoryStats,
  getMemoryConfig,
  getAgentKvMemory,
  type MemoryItem,
  type AgentKvPair,
} from "../http/client";
import { healthDetailQueryOptions } from "./runtime";
import { memoryKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

// Records are the active workspace surface, so they refresh every 30 seconds.
// Aggregate count cards tolerate a slower 60-second cadence. Both remain fresh
// for 30 seconds so focus/remount does not duplicate an interval fetch.
const RECORD_REFRESH_MS = 30_000;
const STATS_REFRESH_MS = 60_000;
const MEMORY_STALE_MS = 30_000;
const CONFIG_STALE_MS = 300_000;
const KV_STALE_MS = 30_000;
export const MEMORY_PAGE_SIZE = 50;
const MEMORY_SEARCH_LIMIT = 50;

export const memoryQueries = {

  stats: (agentId?: string) =>
    queryOptions({
      queryKey: memoryKeys.stats(agentId),
      queryFn: () => getMemoryStats(agentId),
      staleTime: MEMORY_STALE_MS,
      refetchInterval: STATS_REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  config: () =>
    queryOptions({
      queryKey: memoryKeys.config(),
      queryFn: getMemoryConfig,
      staleTime: CONFIG_STALE_MS,
    }),
};



// List mode uses the server's real offset/limit contract. Search has no
// pagination or total today, so it deliberately returns a named bounded set.
export interface MemorySearchOrListParams {
  search: string;
  agentId?: string;
  level?: string;
  offset?: number;
  limit?: number;
}

export const memorySearchOrListQueryOptions = ({
  search,
  agentId,
  level,
  offset = 0,
  limit = MEMORY_PAGE_SIZE,
}: MemorySearchOrListParams) =>
  queryOptions<{
    memories: MemoryItem[];
    total: number;
    proactive_enabled?: boolean;
  }>({
    queryKey: memoryKeys.searchOrList({ search, agentId, level, offset, limit }),
    queryFn: async () => {
      if (search.trim()) {
        // Search has no offset/total contract. Keep the server's bounded result
        // explicit until that endpoint supports real pagination.
        const items = await searchMemories({
          query: search.trim(),
          agentId,
          level,
          limit: MEMORY_SEARCH_LIMIT,
        });
        return { memories: items, total: items.length };
      }
      const res = await listMemories({ agentId, level, offset, limit });
      return {
        memories: res.memories ?? [],
        total: res.total ?? 0,
        proactive_enabled: res.proactive_enabled,
      };
    },
    staleTime: MEMORY_STALE_MS,
    refetchInterval: RECORD_REFRESH_MS,
    refetchIntervalInBackground: false, // #3393
  });

export function useMemorySearchOrList(
  params: MemorySearchOrListParams,
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(memorySearchOrListQueryOptions(params), options));
}

// Per-agent KV memory store. Independent of proactive memory — works even
// when `[proactive_memory] enabled = false`. Returns `kv_pairs` directly
// (server already returns `{kv_pairs: [...]}`); we normalize undefined to
// an empty array so consumers can iterate without null checks.
export const agentKvMemoryQueryOptions = (agentId: string) =>
  queryOptions<AgentKvPair[]>({
    queryKey: memoryKeys.agentKv(agentId),
    queryFn: async () => {
      const res = await getAgentKvMemory(agentId);
      return res.kv_pairs ?? [];
    },
    enabled: !!agentId,
    staleTime: KV_STALE_MS,
  });

export function useAgentKvMemory(agentId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentKvMemoryQueryOptions(agentId), options));
}

export function useMemoryStats(agentId?: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(memoryQueries.stats(agentId), options));
}

export function useMemoryConfig(options: QueryOverrides = {}) {
  return useQuery(withOverrides(memoryQueries.config(), options));
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
