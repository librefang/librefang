import { queryOptions, useQuery } from "@tanstack/react-query";
import { listSessions, getSessionDetails } from "../http/client";
import { sessionKeys } from "./keys";

const REFRESH_MS = 30_000;
const STALE_MS = 30_000;

export const sessionQueries = {
  list: () =>
    queryOptions({
      queryKey: sessionKeys.lists(),
      queryFn: listSessions,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
  detail: (sessionId: string) =>
    queryOptions({
      queryKey: sessionKeys.detail(sessionId),
      queryFn: () => getSessionDetails(sessionId),
      enabled: !!sessionId,
    }),
};

export function useSessions() {
  return useQuery(sessionQueries.list());
}

export function useSessionDetails(sessionId: string) {
  return useQuery(sessionQueries.detail(sessionId));
}
