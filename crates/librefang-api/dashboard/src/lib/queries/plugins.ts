import { queryOptions, useQuery } from "@tanstack/react-query";
import { listPlugins, listPluginRegistries } from "../http/client";
import { pluginKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 60_000;

export const pluginQueries = {
  list: () =>
    queryOptions({
      queryKey: pluginKeys.lists(),
      queryFn: listPlugins,
      staleTime: STALE_MS,
      refetchInterval: STALE_MS,
    }),
  registries: () =>
    queryOptions({
      queryKey: pluginKeys.registries(),
      queryFn: listPluginRegistries,
      staleTime: 300_000,
      refetchInterval: 300_000,
    }),
};

export function usePlugins(options: QueryOverrides = {}) {
  return useQuery(withOverrides(pluginQueries.list(), options));
}

export function usePluginRegistries(options: QueryOverrides = {}) {
  return useQuery(withOverrides(pluginQueries.registries(), options));
}
