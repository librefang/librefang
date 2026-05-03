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
      refetchIntervalInBackground: false, // #3393
    }),
  triggers: (agentId?: string) =>
    queryOptions({
      queryKey: triggerKeys.list(agentId),
      queryFn: () => listTriggers(agentId),
      staleTime: STALE_MS,
      refetchInterval: STALE_MS,
      refetchIntervalInBackground: false, // #3393
    }),
};

export function useSchedules(options: QueryOverrides = {}) {
  return useQuery(withOverrides(scheduleQueries.list(), options));
}

export function useTriggers(options: QueryOverrides = {}) {
  return useQuery(withOverrides(scheduleQueries.triggers(), options));
}

/** Per-agent triggers — uses GET /api/triggers?agent_id=… so the agent
 *  detail panel doesn't need to load every trigger and filter clientside. */
export function useAgentTriggers(agentId: string, options: QueryOverrides = {}) {
  return useQuery(
    withOverrides(
      { ...scheduleQueries.triggers(agentId), enabled: !!agentId },
      options,
    ),
  );
}
