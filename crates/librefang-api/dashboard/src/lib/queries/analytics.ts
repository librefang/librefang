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

const REFRESH_MS = 30_000;
const STALE_MS = 30_000;

export const usageQueries = {
  summary: () =>
    queryOptions({
      queryKey: usageKeys.summary(),
      queryFn: getUsageSummary,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
  byAgent: () =>
    queryOptions({
      queryKey: usageKeys.byAgent(),
      queryFn: listUsageByAgent,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
  byModel: () =>
    queryOptions({
      queryKey: usageKeys.byModel(),
      queryFn: listUsageByModel,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
  daily: () =>
    queryOptions({
      queryKey: usageKeys.daily(),
      queryFn: getUsageDaily,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
  modelPerformance: () =>
    queryOptions({
      queryKey: usageKeys.modelPerformance(),
      queryFn: getUsageByModelPerformance,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
};

export const budgetQueries = {
  status: () =>
    queryOptions({
      queryKey: budgetKeys.status(),
      queryFn: getBudgetStatus,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
};

export function useUsageSummary() {
  return useQuery(usageQueries.summary());
}

export function useUsageByAgent() {
  return useQuery(usageQueries.byAgent());
}

export function useUsageByModel() {
  return useQuery(usageQueries.byModel());
}

export function useUsageDaily() {
  return useQuery(usageQueries.daily());
}

export function useModelPerformance() {
  return useQuery(usageQueries.modelPerformance());
}

export function useBudgetStatus() {
  return useQuery(budgetQueries.status());
}
