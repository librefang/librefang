import { queryOptions, useQuery } from "@tanstack/react-query";
import { listGoals, listGoalTemplates } from "../http/client";
import { goalKeys } from "./keys";

const REFRESH_MS = 30_000;
const STALE_MS = 30_000;
const TEMPLATE_STALE_MS = 300_000;

export const goalQueries = {
  list: () =>
    queryOptions({
      queryKey: goalKeys.list(),
      queryFn: listGoals,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
  templates: () =>
    queryOptions({
      queryKey: goalKeys.templates(),
      queryFn: listGoalTemplates,
      staleTime: TEMPLATE_STALE_MS,
    }),
};

export function useGoals() {
  return useQuery(goalQueries.list());
}

export function useGoalTemplates() {
  return useQuery(goalQueries.templates());
}
