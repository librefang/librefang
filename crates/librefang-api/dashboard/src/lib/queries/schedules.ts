import { queryOptions, useQuery } from "@tanstack/react-query";
import { listSchedules, listTriggers } from "../http/client";
import { scheduleKeys, triggerKeys } from "./keys";

const STALE_MS = 30_000;
const REFRESH_MS = 30_000;

export const scheduleQueries = {
  list: () =>
    queryOptions({
      queryKey: scheduleKeys.lists(),
      queryFn: listSchedules,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
  triggers: () =>
    queryOptions({
      queryKey: triggerKeys.lists(),
      queryFn: listTriggers,
      staleTime: STALE_MS,
    }),
};

export function useSchedules() {
  return useQuery(scheduleQueries.list());
}

export function useTriggers() {
  return useQuery(scheduleQueries.triggers());
}
