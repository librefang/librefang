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

type UseAnalyticsOptions = {
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?: number | false;
};

const REFRESH_MS = 30_000;
const STALE_MS = 20_000;

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

export function useUsageSummary(options: UseAnalyticsOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...usageQueries.summary(),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useUsageByAgent(options: UseAnalyticsOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...usageQueries.byAgent(),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useUsageByModel(options: UseAnalyticsOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...usageQueries.byModel(),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useUsageDaily(options: UseAnalyticsOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...usageQueries.daily(),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useModelPerformance(options: UseAnalyticsOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...usageQueries.modelPerformance(),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useBudgetStatus(options: UseAnalyticsOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...budgetQueries.status(),
    enabled,
    staleTime,
    refetchInterval,
  });
}
