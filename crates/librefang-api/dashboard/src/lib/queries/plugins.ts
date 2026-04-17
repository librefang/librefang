import { queryOptions, useQuery } from "@tanstack/react-query";
import { listPlugins, listPluginRegistries } from "../http/client";
import { pluginKeys } from "./keys";

const STALE_MS = 30_000;

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
    }),
};

export function usePlugins() {
  return useQuery(pluginQueries.list());
}

export function usePluginRegistries(enabled?: boolean) {
  return useQuery({ ...pluginQueries.registries(), enabled });
}
