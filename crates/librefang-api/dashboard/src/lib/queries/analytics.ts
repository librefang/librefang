import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  getUsageSummary,
  listUsageByAgent,
  listUsageByModel,
  getUsageDaily,
  getUsageByModelPerformance,
  getBudgetStatus,
  getProviderBudgets,
} from "../http/client";
import { usageKeys, budgetKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const REFRESH_MS = 30_000;
const STALE_MS = 20_000;
// #3393: analytics stay live while visible without polling hidden tabs.
const analyticsQueryPolicy = {
  staleTime: STALE_MS,
  refetchInterval: REFRESH_MS,
  refetchIntervalInBackground: false,
} as const;

export const usageQueries = {
  summary: () =>
    queryOptions({
      queryKey: usageKeys.summary(),
      queryFn: getUsageSummary,
      ...analyticsQueryPolicy,
    }),
  byAgent: () =>
    queryOptions({
      queryKey: usageKeys.byAgent(),
      queryFn: listUsageByAgent,
      ...analyticsQueryPolicy,
    }),
  byModel: () =>
    queryOptions({
      queryKey: usageKeys.byModel(),
      queryFn: listUsageByModel,
      ...analyticsQueryPolicy,
    }),
  daily: () =>
    queryOptions({
      queryKey: usageKeys.daily(),
      queryFn: getUsageDaily,
      ...analyticsQueryPolicy,
    }),
  modelPerformance: () =>
    queryOptions({
      queryKey: usageKeys.modelPerformance(),
      queryFn: getUsageByModelPerformance,
      ...analyticsQueryPolicy,
    }),
};

export const budgetQueries = {
  status: () =>
    queryOptions({
      queryKey: budgetKeys.status(),
      queryFn: getBudgetStatus,
      ...analyticsQueryPolicy,
    }),
  // Per-provider spend snapshot (#5650). Same refresh cadence as the
  // global budget query so the dashboard's two budget cards stay in
  // lock-step rather than ping-ponging slightly out of sync.
  providers: () =>
    queryOptions({
      queryKey: budgetKeys.providers(),
      queryFn: getProviderBudgets,
      ...analyticsQueryPolicy,
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

export function useProviderBudgets(options: QueryOverrides = {}) {
  return useQuery(withOverrides(budgetQueries.providers(), options));
}
