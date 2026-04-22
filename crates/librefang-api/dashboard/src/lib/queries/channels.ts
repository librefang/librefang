import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listChannels,
  getCommsTopology,
  listCommsEvents,
} from "../http/client";
import { channelKeys, commsKeys } from "./keys";

const STALE_MS = 30_000;
const REFRESH_MS = 30_000;
const TOPOLOGY_REFRESH_MS = 60_000;
const EVENTS_STALE_MS = 10_000;

export const channelQueries = {
  list: () =>
    queryOptions({
      queryKey: channelKeys.lists(),
      queryFn: listChannels,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
};

export const commsQueries = {
  topology: () =>
    queryOptions({
      queryKey: commsKeys.topology(),
      queryFn: getCommsTopology,
      staleTime: STALE_MS,
      refetchInterval: TOPOLOGY_REFRESH_MS,
    }),
  events: (limit = 200) =>
    queryOptions({
      queryKey: commsKeys.events(limit),
      queryFn: () => listCommsEvents(limit),
      staleTime: EVENTS_STALE_MS,
    }),
};

type UseChannelsOptions = { enabled?: boolean; staleTime?: number; refetchInterval?: number | false };

export function useChannels(options: UseChannelsOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...channelQueries.list(),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useCommsTopology(options: UseChannelsOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...commsQueries.topology(),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useCommsEvents(
  limit = 200,
  options: { enabled?: boolean; refetchInterval?: number | false } = {},
) {
  return useQuery({
    ...commsQueries.events(limit),
    enabled: options.enabled,
    refetchInterval: options.refetchInterval ?? REFRESH_MS,
  });
}
