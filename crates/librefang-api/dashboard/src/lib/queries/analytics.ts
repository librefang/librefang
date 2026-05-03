import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  getUsageSummary,
  listUsageByAgent,
  listUsageByModel,
  getUsageDaily,
  getUsageByModelPerformance,
  getBudgetStatus,
} from "../http/client";
import { usageKeys, budgetKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const REFRESH_MS = 30_000;
const STALE_MS = 20_000;

export const usageQueries = {
  summary: () =>
    queryOptions({
      queryKey: usageKeys.summary(),
      queryFn: getUsageSummary,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false, // #3393: skip polling when tab is hidden
    }),
  byAgent: () =>
    queryOptions({
      queryKey: usageKeys.byAgent(),
      queryFn: listUsageByAgent,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false,
    }),
  byModel: () =>
    queryOptions({
      queryKey: usageKeys.byModel(),
      queryFn: listUsageByModel,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false,
    }),
  daily: () =>
    queryOptions({
      queryKey: usageKeys.daily(),
      queryFn: getUsageDaily,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false,
    }),
  modelPerformance: () =>
    queryOptions({
      queryKey: usageKeys.modelPerformance(),
      queryFn: getUsageByModelPerformance,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false,
    }),
};

export const budgetQueries = {
  status: () =>
    queryOptions({
      queryKey: budgetKeys.status(),
      queryFn: getBudgetStatus,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
};

export function useUsageSummary(options: QueryOverrides = {}) {
  return useQuery(withOverrides(usageQueries.summary(), options));
}

export function useUsageByAgent(options: QueryOverrides = {}) {
  return useQuery(withOverrides(usageQueries.byAgent(), options));
}

export function useUsageByModel(options: QueryOverrides = {}) {
  return useQuery(withOverrides(usageQueries.byModel(), options));
}

export function useUsageDaily(options: QueryOverrides = {}) {
  return useQuery(withOverrides(usageQueries.daily(), options));
}

export function useModelPerformance(options: QueryOverrides = {}) {
  return useQuery(withOverrides(usageQueries.modelPerformance(), options));
}

export function useBudgetStatus(options: QueryOverrides = {}) {
  return useQuery(withOverrides(budgetQueries.status(), options));
}
