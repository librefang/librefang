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
import type { UsageRangeParams } from "../../api";
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

// #8062 — every usage query takes the selected reporting window, and the window is part of its key, so switching presets refetches instead of re-rendering stale numbers under a new caption.
// The default `{}` is the unbounded window, which reproduces the pre-#8062 request byte for byte.
export const usageQueries = {
  summary: (range: UsageRangeParams = {}) =>
    queryOptions({
      queryKey: usageKeys.summary(range),
      queryFn: () => getUsageSummary(range),
      ...analyticsQueryPolicy,
    }),
  byAgent: (range: UsageRangeParams = {}) =>
    queryOptions({
      queryKey: usageKeys.byAgent(range),
      queryFn: () => listUsageByAgent(range),
      ...analyticsQueryPolicy,
    }),
  byModel: (range: UsageRangeParams = {}) =>
    queryOptions({
      queryKey: usageKeys.byModel(range),
      queryFn: () => listUsageByModel(range),
      ...analyticsQueryPolicy,
    }),
  // `days` is only meaningful for the unbounded window — the endpoint answers 400 when it arrives alongside a range.
  // See `dailyDaysFor`.
  daily: (range: UsageRangeParams = {}, days?: number) =>
    queryOptions({
      queryKey: usageKeys.daily(range, days),
      queryFn: () => getUsageDaily(range, days),
      ...analyticsQueryPolicy,
    }),
  modelPerformance: (range: UsageRangeParams = {}) =>
    queryOptions({
      queryKey: usageKeys.modelPerformance(range),
      queryFn: () => getUsageByModelPerformance(range),
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
  //
  // Deliberately NOT range-filtered: `/api/budget/providers` reports live hourly / daily / monthly rollups against the configured caps, which are "right now" facts.
  // Scoping them to a historical window would render a cap bar that looks breached (or clear) for a month nobody is spending in.
  providers: () =>
    queryOptions({
      queryKey: budgetKeys.providers(),
      queryFn: getProviderBudgets,
      ...analyticsQueryPolicy,
    }),
};

export function useUsageSummary(
  range: UsageRangeParams = {},
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(usageQueries.summary(range), options));
}

export function useUsageByAgent(
  range: UsageRangeParams = {},
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(usageQueries.byAgent(range), options));
}

export function useUsageByModel(
  range: UsageRangeParams = {},
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(usageQueries.byModel(range), options));
}

export function useUsageDaily(
  range: UsageRangeParams = {},
  days?: number,
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(usageQueries.daily(range, days), options));
}

export function useModelPerformance(
  range: UsageRangeParams = {},
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(usageQueries.modelPerformance(range), options));
}

export function useBudgetStatus(options: QueryOverrides = {}) {
  return useQuery(withOverrides(budgetQueries.status(), options));
}

export function useProviderBudgets(options: QueryOverrides = {}) {
  return useQuery(withOverrides(budgetQueries.providers(), options));
}
