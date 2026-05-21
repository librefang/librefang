import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listChannels,
  listChannelInstances,
  getCommsTopology,
  listCommsEvents,
} from "../http/client";
import { channelKeys, commsKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

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
      refetchIntervalInBackground: false, // #3393
    }),
  // Per-instance list (#4837). No background refresh — the dashboard
  // re-reads via the standard `invalidateQueries` pattern after every
  // create/update/delete mutation, and the form drawer is short-lived
  // so a periodic refetch would be wasted load.
  instances: (name: string) =>
    queryOptions({
      queryKey: channelKeys.instances(name),
      queryFn: () => listChannelInstances(name),
      staleTime: STALE_MS,
    }),
};

export const commsQueries = {
  topology: () =>
    queryOptions({
      queryKey: commsKeys.topology(),
      queryFn: getCommsTopology,
      staleTime: STALE_MS,
      refetchInterval: TOPOLOGY_REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  events: (limit = 200) =>
    queryOptions({
      queryKey: commsKeys.events(limit),
      queryFn: () => listCommsEvents(limit),
      staleTime: EVENTS_STALE_MS,
    }),
};

export function useChannels(options: QueryOverrides = {}) {
  return useQuery(withOverrides(channelQueries.list(), options));
}

export function useChannelInstances(
  name: string,
  options: QueryOverrides & { enabled?: boolean } = {},
) {
  const base = channelQueries.instances(name);
  return useQuery({
    ...withOverrides(base, options),
    enabled: options.enabled ?? Boolean(name),
  });
}

export function useCommsTopology(options: QueryOverrides = {}) {
  return useQuery(withOverrides(commsQueries.topology(), options));
}

export function useCommsEvents(
  limit = 200,
  options: { enabled?: boolean; refetchInterval?: number | false } = {},
) {
  return useQuery({
    ...commsQueries.events(limit),
    enabled: options.enabled,
    refetchInterval: options.refetchInterval ?? REFRESH_MS,
    refetchIntervalInBackground: false, // #3393
  });
}
