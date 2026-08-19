import { queryOptions, useQuery } from "@tanstack/react-query";
import { getTerminalHealth, listTerminalWindows } from "../http/client";
import { terminalKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const HEALTH_STALE_MS = 60_000;
const WINDOW_REFRESH_MS = 10_000;

export const terminalQueries = {
  health: () =>
    queryOptions({
      queryKey: terminalKeys.health(),
      queryFn: getTerminalHealth,
      staleTime: HEALTH_STALE_MS,
    }),
  windows: () =>
    queryOptions({
      queryKey: terminalKeys.windows(),
      queryFn: listTerminalWindows,
      staleTime: WINDOW_REFRESH_MS,
      refetchInterval: WINDOW_REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
};

export function useTerminalHealth(options: QueryOverrides = {}) {
  return useQuery(withOverrides(terminalQueries.health(), options));
}

export function useTerminalWindows(options: QueryOverrides = {}) {
  return useQuery(withOverrides(terminalQueries.windows(), options));
}
