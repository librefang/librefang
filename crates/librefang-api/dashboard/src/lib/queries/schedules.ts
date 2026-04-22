import { queryOptions, useQuery } from "@tanstack/react-query";
import { listSchedules, listTriggers } from "../http/client";
import { scheduleKeys, triggerKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

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

export function useSchedules(options: QueryOverrides = {}) {
  return useQuery(withOverrides(scheduleQueries.list(), options));
}

export function useTriggers(options: QueryOverrides = {}) {
  return useQuery(withOverrides(scheduleQueries.triggers(), options));
}
