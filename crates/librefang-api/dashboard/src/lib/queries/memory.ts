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

const REFRESH_MS = 30_000;
const STALE_MS = 30_000;
const CONFIG_STALE_MS = 300_000;
const KV_STALE_MS = 30_000;

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



// Propagates `proactive_enabled` from the list endpoint so the page can
// decide whether to render proactive sections without making a second
// request. The search endpoint does not expose this flag today; in search
// mode we leave it `undefined` and let the page rely on the list-mode
// response (search is hidden when proactive is disabled, so this never
// becomes ambiguous in practice).
export const memorySearchOrListQueryOptions = (search: string) =>
  queryOptions<{
    memories: MemoryItem[];
    total: number;
    proactive_enabled?: boolean;
  }>({
    queryKey: memoryKeys.searchOrList(search),
    queryFn: async () => {
      if (search.trim()) {
        const items = await searchMemories({ query: search.trim(), limit: 50 });
        return { memories: items, total: items.length };
      }
      const res = await listMemories({ offset: 0, limit: 10000 });
      return {
        memories: res.memories ?? [],
        total: res.total ?? 0,
        proactive_enabled: res.proactive_enabled,
      };
    },
    staleTime: STALE_MS,
    refetchInterval: REFRESH_MS,
  });

export function useMemorySearchOrList(search: string) {
  return useQuery(memorySearchOrListQueryOptions(search));
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
