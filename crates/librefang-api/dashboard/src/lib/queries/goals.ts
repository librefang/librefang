import { queryOptions, useQuery } from "@tanstack/react-query";
import { listGoals, listGoalTemplates } from "../http/client";
import { goalKeys } from "./keys";

const STALE_MS = 30_000;
const TEMPLATE_STALE_MS = 300_000;

type UseGoalOptions = {
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?: number | false;
};

export const goalQueries = {
  list: () =>
    queryOptions({
      queryKey: goalKeys.lists(),
      queryFn: listGoals,
      staleTime: STALE_MS,
    }),
  templates: () =>
    queryOptions({
      queryKey: goalKeys.templates(),
      queryFn: listGoalTemplates,
      staleTime: TEMPLATE_STALE_MS,
    }),
};

export function useGoals(options: UseGoalOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...goalQueries.list(),
    enabled,
    staleTime,
    refetchInterval: refetchInterval ?? STALE_MS,
  });
}

export function useGoalTemplates(options: UseGoalOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...goalQueries.templates(),
    enabled,
    staleTime,
    refetchInterval,
  });
}
