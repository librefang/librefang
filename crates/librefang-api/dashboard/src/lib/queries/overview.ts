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
    // #3393: every mounted page using `useDashboardSnapshot` would otherwise
    // refetch every 5 s while the tab is backgrounded. The QueryClient
    // default in `main.tsx` also pins this to false, but we set it
    // explicitly per-query so the visibility gate sits next to the poll
    // interval and survives any future change to the global default.
    refetchIntervalInBackground: false,
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
