import { queryOptions, useQuery } from "@tanstack/react-query";
import { getTerminalHealth, listTerminalWindows } from "../http/client";
import { terminalKeys } from "./keys";

const REFRESH_MS = 10_000;

type UseTerminalQueryOptions = {
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?: number | false;
};

export const terminalQueries = {
  health: () =>
    queryOptions({
      queryKey: terminalKeys.health(),
      queryFn: getTerminalHealth,
      staleTime: 60_000,
    }),
  windows: () =>
    queryOptions({
      queryKey: terminalKeys.windows(),
      queryFn: listTerminalWindows,
      refetchInterval: REFRESH_MS,
    }),
};

export function useTerminalHealth(options: UseTerminalQueryOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...terminalQueries.health(),
    enabled,
    ...(staleTime !== undefined ? { staleTime } : {}),
    ...(refetchInterval !== undefined ? { refetchInterval } : {}),
  });
}

export function useTerminalWindows(options: UseTerminalQueryOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...terminalQueries.windows(),
    enabled,
    ...(staleTime !== undefined ? { staleTime } : {}),
    ...(refetchInterval !== undefined ? { refetchInterval } : {}),
  });
}
