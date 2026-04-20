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
  catalog: (opts: { enabled?: boolean } = {}) =>
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
  authStatus: (id: string, opts: { enabled?: boolean } = {}) =>
    queryOptions({
      queryKey: mcpKeys.authStatus(id),
      queryFn: () => getMcpAuthStatus(id),
      // Auth polling needs a fresh read on each fetchQuery call.
      staleTime: 0,
      enabled: opts.enabled ?? Boolean(id),
    }),
};

export function useMcpServers() {
  return useQuery(mcpQueries.servers());
}

export function useMcpServer(id: string) {
  return useQuery(mcpQueries.server(id));
}

export function useMcpCatalog(opts: { enabled?: boolean } = {}) {
  return useQuery(mcpQueries.catalog(opts));
}

export function useMcpCatalogEntry(id: string) {
  return useQuery(mcpQueries.catalogEntry(id));
}

export function useMcpHealth() {
  return useQuery(mcpQueries.health());
}

export function useMcpAuthStatus(id: string, options: { enabled?: boolean } = {}) {
  return useQuery({
    ...mcpQueries.authStatus(id, options),
    enabled: options.enabled ?? Boolean(id),
  });
}
