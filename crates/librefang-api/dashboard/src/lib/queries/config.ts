import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  getFullConfig,
  getConfigSchema,
  fetchRegistrySchema,
  getRawConfigToml,
} from "../http/client";
import { configKeys, registryKeys } from "./keys";

const STALE_MS = 60_000;
const SCHEMA_STALE_MS = 300_000;
const RAW_STALE_MS = 5_000;

export const configQueries = {
  full: () =>
    queryOptions({
      queryKey: configKeys.full(),
      queryFn: getFullConfig,
      staleTime: STALE_MS,
    }),
  schema: () =>
    queryOptions({
      queryKey: configKeys.schema(),
      queryFn: getConfigSchema,
      staleTime: SCHEMA_STALE_MS,
    }),
  registrySchema: (contentType: string) =>
    queryOptions({
      queryKey: registryKeys.schema(contentType),
      queryFn: () => fetchRegistrySchema(contentType),
      enabled: !!contentType,
      staleTime: SCHEMA_STALE_MS,
      retry: 1,
    }),
  rawToml: (enabled: boolean) =>
    queryOptions({
      queryKey: configKeys.rawToml(),
      queryFn: getRawConfigToml,
      enabled,
      staleTime: RAW_STALE_MS,
    }),
};

type UseConfigOptions = {
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?: number | false;
};

export function useFullConfig(options: UseConfigOptions = {}) {
  return useQuery({ ...configQueries.full(), ...options });
}

export function useConfigSchema(options: UseConfigOptions = {}) {
  return useQuery({ ...configQueries.schema(), ...options });
}

export function useRegistrySchema(contentType: string, options: UseConfigOptions = {}) {
  // Empty contentType disables query (enabled gate in configQueries)
  return useQuery({ ...configQueries.registrySchema(contentType), ...options });
}

// Raw config.toml as text. Disabled by default — caller passes
// `enabled: true` only when the viewer modal is open. Short staleTime
// so re-opening shortly after a save reflects the change.
export function useRawConfigToml(enabled: boolean) {
  return useQuery(configQueries.rawToml(enabled));
}
