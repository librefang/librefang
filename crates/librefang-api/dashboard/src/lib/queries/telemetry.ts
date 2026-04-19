import { queryOptions, useQuery } from "@tanstack/react-query";
import { getMetricsText } from "../http/client";
import { telemetryKeys } from "./keys";

const STALE_MS = 5_000;
const REFRESH_MS = 5_000;

type UseTelemetryMetricsOptions = {
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?: number | false;
};

export const telemetryQueryOptions = () =>
  queryOptions({
    queryKey: telemetryKeys.metrics(),
    queryFn: getMetricsText,
    staleTime: STALE_MS,
    refetchInterval: REFRESH_MS,
  });

export function useTelemetryMetrics(
  options: UseTelemetryMetricsOptions = {},
) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...telemetryQueryOptions(),
    enabled,
    staleTime,
    refetchInterval,
  });
}
