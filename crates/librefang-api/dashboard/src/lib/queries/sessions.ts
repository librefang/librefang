import { queryOptions, useQuery } from "@tanstack/react-query";
import { listSessions, getSessionDetails } from "../http/client";
import { sessionKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 30_000;

export const sessionQueries = {
  list: () =>
    queryOptions({
      queryKey: sessionKeys.lists(),
      queryFn: listSessions,
      staleTime: STALE_MS,
      refetchInterval: STALE_MS,
    }),
  detail: (sessionId: string) =>
    queryOptions({
      queryKey: sessionKeys.detail(sessionId),
      queryFn: () => getSessionDetails(sessionId),
      enabled: !!sessionId,
    }),
};

export function useSessions(options: QueryOverrides = {}) {
  return useQuery(withOverrides(sessionQueries.list(), options));
}

export function useSessionDetails(sessionId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(sessionQueries.detail(sessionId), options));
}
