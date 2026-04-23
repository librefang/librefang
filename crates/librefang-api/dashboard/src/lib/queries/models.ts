import { queryOptions, useQuery } from "@tanstack/react-query";
import { listModels, getModelOverrides } from "../http/client";
import { modelKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 30_000;
const REFRESH_MS = 60_000;

export const modelQueries = {
  list: (filters: {
    provider?: string;
    tier?: string;
    available?: boolean;
  } = {}) =>
    queryOptions({
      queryKey: modelKeys.list(filters),
      queryFn: () => listModels(filters),
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
  overrides: (modelKey: string) =>
    queryOptions({
      queryKey: modelKeys.overrides(modelKey),
      queryFn: () => getModelOverrides(modelKey),
      enabled: !!modelKey,
      staleTime: 60_000,
    }),
};

export function useModels(
  filters: { provider?: string; tier?: string; available?: boolean } = {},
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(modelQueries.list(filters), options));
}

export function useModelOverrides(modelKey: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(modelQueries.overrides(modelKey), options));
}
