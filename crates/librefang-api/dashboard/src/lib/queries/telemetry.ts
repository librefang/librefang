import { queryOptions, useQuery } from "@tanstack/react-query";
import { getMetricsText } from "../http/client";
import { telemetryKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 5_000;
const REFRESH_MS = 10_000; // live metrics: 10 s refetch (stale at 5 s to catch tab-switch refreshes)

export const telemetryQueryOptions = () =>
  queryOptions({
    queryKey: telemetryKeys.metrics(),
    queryFn: getMetricsText,
    staleTime: STALE_MS,
    refetchInterval: REFRESH_MS,
    refetchIntervalInBackground: false, // #3393
  });

export function useTelemetryMetrics(options: QueryOverrides = {}) {
  return useQuery(withOverrides(telemetryQueryOptions(), options));
}
