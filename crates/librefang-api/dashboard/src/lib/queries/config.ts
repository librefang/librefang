import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  getFullConfig,
  getConfigSchema,
  fetchRegistrySchema,
  getRawConfigToml,
} from "../http/client";
import { configKeys, registryKeys } from "./keys";

export const configQueries = {
  full: () =>
    queryOptions({
      queryKey: configKeys.full(),
      queryFn: getFullConfig,
      staleTime: 60_000,
    }),
  schema: () =>
    queryOptions({
      queryKey: configKeys.schema(),
      queryFn: getConfigSchema,
      staleTime: 300_000,
    }),
  registrySchema: (contentType: string) =>
    queryOptions({
      queryKey: registryKeys.schema(contentType),
      queryFn: () => fetchRegistrySchema(contentType),
      enabled: !!contentType,
      staleTime: 300_000,
      retry: 1,
    }),
};

export function useFullConfig() {
  return useQuery(configQueries.full());
}

export function useConfigSchema() {
  return useQuery(configQueries.schema());
}

export function useRegistrySchema(contentType: string) {
  return useQuery(configQueries.registrySchema(contentType));
}

// Raw config.toml as text. Disabled by default — caller passes
// `enabled: true` only when the viewer modal is open. Short staleTime
// so re-opening shortly after a save reflects the change.
export function useRawConfigToml(enabled: boolean) {
  return useQuery({
    queryKey: configKeys.rawToml(),
    queryFn: getRawConfigToml,
    enabled,
    staleTime: 5_000,
  });
}
