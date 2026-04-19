import { queryOptions, useQuery } from "@tanstack/react-query";
import { listTerminalWindows } from "../http/client";
import { terminalKeys } from "./keys";

const REFRESH_MS = 10_000;

export const terminalQueries = {
  windows: () =>
    queryOptions({
      queryKey: terminalKeys.windows(),
      queryFn: listTerminalWindows,
      refetchInterval: REFRESH_MS,
    }),
};

export function useTerminalWindows(options: { enabled?: boolean } = {}) {
  return useQuery({ ...terminalQueries.windows(), enabled: options.enabled });
}
