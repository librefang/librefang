import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  getFullConfig,
  getConfigSchema,
  getConfigStatus,
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
  // Ownership of config.toml — the deployment's or the dashboard's.
  // Shares `STALE_MS` with `full`: the mode is a deployment fact that changes
  // only across a restart, but it is cheap and the two are always read
  // together, so a shorter-lived cache buys nothing.
  status: () =>
    queryOptions({
      queryKey: configKeys.status(),
      queryFn: getConfigStatus,
      staleTime: STALE_MS,
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



export function useFullConfig(options: QueryOverrides = {}) {
  return useQuery(withOverrides(configQueries.full(), options));
}

export function useConfigSchema(options: QueryOverrides = {}) {
  return useQuery(withOverrides(configQueries.schema(), options));
}

/**
 * Where `config.toml` came from and whether this daemon will accept a write.
 *
 * Read it before rendering a control that persists configuration, so a
 * managed deployment shows the setting as locked instead of offering an
 * editable control that answers `423 config_managed` on save (#6695).
 */
export function useConfigStatus(options: QueryOverrides = {}) {
  return useQuery(withOverrides(configQueries.status(), options));
}

export function useRegistrySchema(contentType: string, options: QueryOverrides = {}) {
  // Empty contentType disables query (enabled gate in configQueries)
  return useQuery(withOverrides(configQueries.registrySchema(contentType), options));
}

// Raw config.toml as text. Disabled by default — caller passes
// `enabled: true` only when the viewer modal is open. Short staleTime
// so re-opening shortly after a save reflects the change.
export function useRawConfigToml(enabled: boolean) {
  return useQuery(configQueries.rawToml(enabled));
}
