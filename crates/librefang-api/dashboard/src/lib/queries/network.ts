import { queryOptions, useQuery, type QueryKey } from "@tanstack/react-query";
import {
  getNetworkStatus,
  listPeers,
  listTrustedPeers,
  listA2AAgents,
} from "../http/client";
import { networkKeys, peerKeys, a2aKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const REFRESH_MS = 15_000;
const STALE_MS = 30_000;

const liveNetworkQueryOptions = <T>(
  queryKey: QueryKey,
  queryFn: () => Promise<T>,
) =>
  queryOptions({
    queryKey,
    queryFn,
    staleTime: STALE_MS,
    refetchInterval: REFRESH_MS,
    refetchIntervalInBackground: false, // #3393
  });

export const networkQueries = {
  status: () => liveNetworkQueryOptions(networkKeys.status(), getNetworkStatus),
  peers: () => liveNetworkQueryOptions(peerKeys.lists(), listPeers),
  trustedPeers: () =>
    liveNetworkQueryOptions(networkKeys.trustedPeers(), listTrustedPeers),
  a2aAgents: () => liveNetworkQueryOptions(a2aKeys.agents(), listA2AAgents),
};

export function useNetworkStatus(options: QueryOverrides = {}) {
  return useQuery(withOverrides(networkQueries.status(), options));
}

export function usePeers(options: QueryOverrides = {}) {
  return useQuery(withOverrides(networkQueries.peers(), options));
}

export function useTrustedPeers(options: QueryOverrides = {}) {
  return useQuery(withOverrides(networkQueries.trustedPeers(), options));
}

export function useA2AAgents(options: QueryOverrides = {}) {
  return useQuery(withOverrides(networkQueries.a2aAgents(), options));
}
