import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  getFullConfig,
  getConfigSchema,
  fetchRegistrySchema,
  getRawConfigToml,
} from "../http/client";
import { configKeys, registryKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

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
      // Schemas are optional per content type and selected interactively;
      // surface an unavailable schema promptly instead of keeping the form
      // behind the default multi-retry delay.
      retry: 1,
    }),
  rawToml: () =>
    queryOptions({
      queryKey: configKeys.rawToml(),
      queryFn: getRawConfigToml,
      staleTime: RAW_STALE_MS,
    }),
};



export function useFullConfig(options: QueryOverrides = {}) {
  return useQuery(withOverrides(configQueries.full(), options));
}

export function useConfigSchema(options: QueryOverrides = {}) {
  return useQuery(withOverrides(configQueries.schema(), options));
}

export function useRegistrySchema(contentType: string, options: QueryOverrides = {}) {
  // Empty contentType disables query (enabled gate in configQueries)
  return useQuery(withOverrides(configQueries.registrySchema(contentType), options));
}

// Raw config.toml as text. Disabled by default — caller passes
// `enabled: true` only when the viewer modal is open. Short staleTime
// so re-opening shortly after a save reflects the change.
export function useRawConfigToml(
  enabled: boolean,
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(configQueries.rawToml(), {
    ...options,
    enabled: enabled && options.enabled !== false,
  }));
}
