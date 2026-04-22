import { queryOptions, useQuery } from "@tanstack/react-query";
import { listGoals, listGoalTemplates } from "../http/client";
import { goalKeys } from "./keys";
import { withOverrides, QueryOverrides } from "./options";

const STALE_MS = 30_000;
const TEMPLATE_STALE_MS = 300_000;

export const goalQueries = {
  list: () =>
    queryOptions({
      queryKey: goalKeys.lists(),
      queryFn: listGoals,
      staleTime: STALE_MS,
      refetchInterval: STALE_MS,
    }),
  templates: () =>
    queryOptions({
      queryKey: goalKeys.templates(),
      queryFn: listGoalTemplates,
      staleTime: TEMPLATE_STALE_MS,
    }),
};

export function useGoals(options: QueryOverrides = {}) {
  return useQuery(withOverrides(goalQueries.list(), options));
}

export function useGoalTemplates(options: QueryOverrides = {}) {
  return useQuery(withOverrides(goalQueries.templates(), options));
}
