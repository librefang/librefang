import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listMcpServers,
  getMcpServer,
  listMcpCatalog,
  getMcpCatalogEntry,
  getMcpHealth,
  getMcpAuthStatus,
} from "../http/client";
import { mcpKeys } from "./keys";

type UseMcpOptions = {
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?: number | false;
};

const SERVERS_STALE_MS = 30_000;
const SERVERS_REFRESH_MS = 30_000;
const CATALOG_STALE_MS = 300_000;
const HEALTH_STALE_MS = 15_000;

export const mcpQueries = {
  servers: () =>
    queryOptions({
      queryKey: mcpKeys.servers(),
      queryFn: listMcpServers,
      staleTime: SERVERS_STALE_MS,
      refetchInterval: SERVERS_REFRESH_MS,
    }),
  server: (id: string) =>
    queryOptions({
      queryKey: mcpKeys.server(id),
      queryFn: () => getMcpServer(id),
      staleTime: SERVERS_STALE_MS,
      enabled: Boolean(id),
    }),
  catalog: (opts: UseMcpOptions = {}) =>
    queryOptions({
      queryKey: mcpKeys.catalog(),
      queryFn: listMcpCatalog,
      staleTime: CATALOG_STALE_MS,
      enabled: opts.enabled,
    }),
  catalogEntry: (id: string) =>
    queryOptions({
      queryKey: mcpKeys.catalogEntry(id),
      queryFn: () => getMcpCatalogEntry(id),
      staleTime: CATALOG_STALE_MS,
      enabled: Boolean(id),
    }),
  health: () =>
    queryOptions({
      queryKey: mcpKeys.health(),
      queryFn: getMcpHealth,
      staleTime: HEALTH_STALE_MS,
    }),
  authStatus: (id: string, opts: UseMcpOptions = {}) =>
    queryOptions({
      queryKey: mcpKeys.authStatus(id),
      queryFn: () => getMcpAuthStatus(id),
      // Auth polling needs a fresh read on each fetchQuery call.
      staleTime: 2_000,
      enabled: opts.enabled ?? Boolean(id),
    }),
};

export function useMcpServers(options: UseMcpOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...mcpQueries.servers(),
    enabled,
    staleTime: staleTime ?? SERVERS_STALE_MS,
    refetchInterval: refetchInterval ?? SERVERS_REFRESH_MS,
  });
}

export function useMcpServer(id: string, options: UseMcpOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...mcpQueries.server(id),
    enabled: enabled ?? Boolean(id),
    staleTime: staleTime ?? SERVERS_STALE_MS,
    refetchInterval,
  });
}

export function useMcpCatalog(options: UseMcpOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...mcpQueries.catalog(options),
    enabled,
    staleTime: staleTime ?? CATALOG_STALE_MS,
    refetchInterval,
  });
}

export function useMcpCatalogEntry(id: string, options: UseMcpOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...mcpQueries.catalogEntry(id),
    enabled: enabled ?? Boolean(id),
    staleTime: staleTime ?? CATALOG_STALE_MS,
    refetchInterval,
  });
}

export function useMcpHealth(options: UseMcpOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...mcpQueries.health(),
    enabled,
    staleTime: staleTime ?? HEALTH_STALE_MS,
    refetchInterval,
  });
}

export function useMcpAuthStatus(id: string, options: UseMcpOptions = {}) {
  const { staleTime, refetchInterval } = options;
  return useQuery({
    ...mcpQueries.authStatus(id, options),
    staleTime: staleTime ?? 2_000,
    refetchInterval,
  });
}
