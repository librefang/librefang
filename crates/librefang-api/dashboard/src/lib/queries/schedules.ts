import { queryOptions, useQuery } from "@tanstack/react-query";
import { listSchedules, listTriggers } from "../http/client";
import { scheduleKeys, triggerKeys } from "./keys";

const STALE_MS = 30_000;

export const scheduleQueries = {
  list: () =>
    queryOptions({
      queryKey: scheduleKeys.lists(),
      queryFn: listSchedules,
      staleTime: STALE_MS,
      refetchInterval: STALE_MS,
    }),
  triggers: () =>
    queryOptions({
      queryKey: triggerKeys.lists(),
      queryFn: listTriggers,
      staleTime: STALE_MS,
      refetchInterval: STALE_MS,
    }),
};

type UseSchedulesOptions = {
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?: number | false;
};

export function useSchedules(options: UseSchedulesOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...scheduleQueries.list(),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useTriggers(options: UseSchedulesOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...scheduleQueries.triggers(),
    enabled,
    staleTime,
    refetchInterval,
  });
}
