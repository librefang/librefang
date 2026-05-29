import { queryOptions, useQuery } from "@tanstack/react-query";
import { listGoals, listGoalTemplates, getGoalRun } from "../http/client";
import { goalKeys } from "./keys";
import { withOverrides, QueryOverrides } from "./options";

const STALE_MS = 30_000;
const TEMPLATE_STALE_MS = 300_000;
const RUN_POLL_MS = 4_000;

export const goalQueries = {
  list: () =>
    queryOptions({
      queryKey: goalKeys.lists(),
      queryFn: listGoals,
      staleTime: STALE_MS,
      refetchInterval: STALE_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  templates: () =>
    queryOptions({
      queryKey: goalKeys.templates(),
      queryFn: listGoalTemplates,
      staleTime: TEMPLATE_STALE_MS,
    }),
  run: (id: string) =>
    queryOptions({
      queryKey: goalKeys.run(id),
      queryFn: () => getGoalRun(id),
      // Poll while a run is active so the dashboard reflects live progress.
      refetchInterval: RUN_POLL_MS,
      refetchIntervalInBackground: false,
    }),
};

export function useGoals(options: QueryOverrides = {}) {
  return useQuery(withOverrides(goalQueries.list(), options));
}

export function useGoalTemplates(options: QueryOverrides = {}) {
  return useQuery(withOverrides(goalQueries.templates(), options));
}

export function useGoalRun(id: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(goalQueries.run(id), options));
}
