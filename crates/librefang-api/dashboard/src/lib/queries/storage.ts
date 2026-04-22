import { useQuery } from "@tanstack/react-query";
import { getStorageConfig, getStorageStatus } from "../http/client";
import { storageKeys } from "./keys";

/** Current storage configuration (`GET /api/storage/config`). */
export function useStorageConfig() {
  return useQuery({
    queryKey: storageKeys.config(),
    queryFn: getStorageConfig,
    staleTime: 30_000,
  });
}

/**
 * Real-time storage status (`GET /api/storage/status`).
 *
 * Includes backend kind, connection health, table counts, migration
 * availability, and UAR link state. Refreshed every 30 s.
 */
export function useStorageStatus() {
  return useQuery({
    queryKey: storageKeys.status(),
    queryFn: getStorageStatus,
    staleTime: 30_000,
    refetchInterval: 60_000,
  });
}
