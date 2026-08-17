import { queryOptions, useQuery } from "@tanstack/react-query";
import { loadDashboardSnapshot, getVersionInfo } from "../http/client";
import { overviewKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const SNAPSHOT_POLL_INTERVAL_MS = 5_000;
const VERSION_STALE_MS = 5 * 60_000;

export const dashboardSnapshotQueryOptions = () =>
  queryOptions({
    queryKey: overviewKeys.snapshot(),
    queryFn: loadDashboardSnapshot,
    staleTime: SNAPSHOT_POLL_INTERVAL_MS,
    refetchInterval: SNAPSHOT_POLL_INTERVAL_MS,
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
    // A long-lived SPA can survive a backend deploy. Keep the cached value,
    // but allow a focus/remount refresh once it is five minutes old.
    staleTime: VERSION_STALE_MS,
    gcTime: Infinity,
    // The dashboard QueryClient disables focus refetching globally.
    refetchOnWindowFocus: true,
  });

export function useDashboardSnapshot(options: QueryOverrides = {}) {
  return useQuery(withOverrides(dashboardSnapshotQueryOptions(), options));
}

export function useVersionInfo(options: QueryOverrides = {}) {
  return useQuery(withOverrides(versionInfoQueryOptions(), options));
}
