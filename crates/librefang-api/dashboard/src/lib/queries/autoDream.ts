import { queryOptions, useQuery } from "@tanstack/react-query";
import { getAutoDreamStatus } from "../http/client";
import { autoDreamKeys } from "./keys";

// Polling at 15s matches the cron scheduler cadence — short enough that a
// dream fired via the manual trigger becomes visible quickly, long enough
// that an idle dashboard doesn't hammer the endpoint.
const REFRESH_MS = 15_000;
const STALE_MS = 10_000;

export const autoDreamQueries = {
  status: () =>
    queryOptions({
      queryKey: autoDreamKeys.status(),
      queryFn: getAutoDreamStatus,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
};

export function useAutoDreamStatus() {
  return useQuery(autoDreamQueries.status());
}
