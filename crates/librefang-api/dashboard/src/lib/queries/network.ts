import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  getNetworkStatus,
  listPeers,
  listA2AAgents,
} from "../http/client";
import { networkKeys, peerKeys, a2aKeys } from "./keys";

const REFRESH_MS = 15_000;
const STALE_MS = 30_000;

export const networkQueries = {
  status: () =>
    queryOptions({
      queryKey: networkKeys.status(),
      queryFn: getNetworkStatus,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
  peers: () =>
    queryOptions({
      queryKey: peerKeys.lists(),
      queryFn: listPeers,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
  a2aAgents: () =>
    queryOptions({
      queryKey: a2aKeys.agents(),
      queryFn: listA2AAgents,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
};

export function useNetworkStatus() {
  return useQuery(networkQueries.status());
}

export function usePeers() {
  return useQuery(networkQueries.peers());
}

export function useA2AAgents() {
  return useQuery(networkQueries.a2aAgents());
}
