import { queryOptions, useQuery } from "@tanstack/react-query";
import { getTerminalHealth, listTerminalWindows } from "../http/client";
import { terminalKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const REFRESH_MS = 10_000;

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
      staleTime: REFRESH_MS,
      refetchInterval: REFRESH_MS,
    }),
};

export function useTerminalHealth(options: QueryOverrides = {}) {
  return useQuery(withOverrides(terminalQueries.health(), options));
}

export function useTerminalWindows(options: QueryOverrides = {}) {
  return useQuery(withOverrides(terminalQueries.windows(), options));
}
