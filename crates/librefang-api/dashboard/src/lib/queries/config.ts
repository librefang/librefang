import { queryOptions, useQuery } from "@tanstack/react-query";
import { getFullConfig, getConfigSchema, fetchRegistrySchema } from "../http/client";
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
