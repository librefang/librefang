import { queryOptions, useQuery } from "@tanstack/react-query";
import { listMcpServers, listAvailableIntegrations } from "../http/client";
import { mcpKeys } from "./keys";

const STALE_MS = 30_000;
const REFRESH_MS = 30_000;
const INTEGRATIONS_STALE_MS = 300_000;

export const mcpQueries = {
  servers: () =>
    queryOptions({
      queryKey: mcpKeys.servers(),
      queryFn: listMcpServers,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
  integrations: (opts: { enabled?: boolean } = {}) =>
    queryOptions({
      queryKey: mcpKeys.integrations(),
      queryFn: listAvailableIntegrations,
      staleTime: INTEGRATIONS_STALE_MS,
      enabled: opts.enabled,
    }),
};

export function useMcpServers() {
  return useQuery(mcpQueries.servers());
}

export function useAvailableIntegrations(opts: { enabled?: boolean } = {}) {
  return useQuery(mcpQueries.integrations(opts));
}
