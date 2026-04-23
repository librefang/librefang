import { queryOptions, useQuery } from "@tanstack/react-query";
import { loadDashboardSnapshot, getVersionInfo } from "../http/client";
import { overviewKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

export const dashboardSnapshotQueryOptions = () =>
  queryOptions({
    queryKey: overviewKeys.snapshot(),
    queryFn: loadDashboardSnapshot,
    staleTime: 5_000,
    refetchInterval: 5_000,
  });

export const versionInfoQueryOptions = () =>
  queryOptions({
    queryKey: overviewKeys.version(),
    queryFn: getVersionInfo,
    staleTime: Infinity,
    gcTime: Infinity,
  });

export function useDashboardSnapshot(options: QueryOverrides = {}) {
  return useQuery(withOverrides(dashboardSnapshotQueryOptions(), options));
}

export function useVersionInfo(options: QueryOverrides = {}) {
  return useQuery(withOverrides(versionInfoQueryOptions(), options));
}
