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
import { QueryOverrides, withOverrides } from "./options";

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
  catalog: (opts: QueryOverrides = {}) =>
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
  authStatus: (id: string, opts: QueryOverrides = {}) =>
    queryOptions({
      queryKey: mcpKeys.authStatus(id),
      queryFn: () => getMcpAuthStatus(id),
      // 2s staleTime balances OAuth polling freshness with request deduplication during rapid refetch cycles.
      staleTime: 2_000,
      enabled: opts.enabled ?? Boolean(id),
    }),
};

export function useMcpServers(options: QueryOverrides = {}) {
  return useQuery(withOverrides(mcpQueries.servers(), options));
}

export function useMcpServer(id: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(mcpQueries.server(id), options));
}

export function useMcpCatalog(options: QueryOverrides = {}) {
  return useQuery(withOverrides(mcpQueries.catalog(), options));
}

export function useMcpCatalogEntry(id: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(mcpQueries.catalogEntry(id), options));
}

export function useMcpHealth(options: QueryOverrides = {}) {
  return useQuery(withOverrides(mcpQueries.health(), options));
}

export function useMcpAuthStatus(id: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(mcpQueries.authStatus(id), options));
}
