import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listHands,
  listActiveHands,
  getHandDetail,
  getHandSettings,
  getHandStats,
  getHandSession,
  getHandInstanceStatus,
  getHandManifestToml,
  type HandStatsResponse,
} from "../http/client";
import { handKeys } from "./keys";

const STALE_MS = 30_000;
const REFRESH_MS = 30_000;
const STATS_BATCH_CONCURRENCY = 6;

export class HandStatsBatchError extends Error {
  constructor(
    readonly instanceIds: readonly string[],
    readonly causes: readonly unknown[],
  ) {
    super(`Failed to load stats for ${instanceIds.length} hand instances`);
    this.name = "HandStatsBatchError";
  }
}

async function getHandStatsBatch(instanceIds: readonly string[]) {
  const results: Record<string, HandStatsResponse> = {};
  const failedIds: string[] = [];
  const causes: unknown[] = [];
  let nextIndex = 0;

  const worker = async () => {
    while (nextIndex < instanceIds.length) {
      const id = instanceIds[nextIndex++];
      try {
        results[id] = await getHandStats(id);
      } catch (error) {
        failedIds.push(id);
        causes.push(error);
      }
    }
  };

  await Promise.all(
    Array.from(
      { length: Math.min(STATS_BATCH_CONCURRENCY, instanceIds.length) },
      worker,
    ),
  );

  if (failedIds.length === instanceIds.length && instanceIds.length > 0) {
    throw new HandStatsBatchError(failedIds, causes);
  }

  return results;
}

export const handQueries = {
  list: () =>
    queryOptions({
      queryKey: handKeys.lists(),
      queryFn: listHands,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  active: () =>
    queryOptions({
      queryKey: handKeys.active(),
      queryFn: listActiveHands,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  detail: (handId: string) =>
    queryOptions({
      queryKey: handKeys.detail(handId),
      queryFn: () => getHandDetail(handId),
      enabled: !!handId,
      staleTime: STALE_MS,
    }),
  settings: (handId: string) =>
    queryOptions({
      queryKey: handKeys.settings(handId),
      queryFn: () => getHandSettings(handId),
      enabled: !!handId,
      staleTime: STALE_MS,
    }),
  stats: (instanceId: string) =>
    queryOptions({
      queryKey: handKeys.stats(instanceId),
      queryFn: () => getHandStats(instanceId),
      enabled: !!instanceId,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  statsBatch: (instanceIds: readonly string[]) =>
    queryOptions({
      queryKey: handKeys.statsBatch(instanceIds),
      queryFn: () => getHandStatsBatch(instanceIds),
      enabled: instanceIds.length > 0,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  session: (instanceId: string) =>
    queryOptions({
      queryKey: handKeys.session(instanceId),
      queryFn: () => getHandSession(instanceId),
      enabled: !!instanceId,
      staleTime: STALE_MS,
    }),
  instanceStatus: (instanceId: string) =>
    queryOptions({
      queryKey: handKeys.instanceStatus(instanceId),
      queryFn: () => getHandInstanceStatus(instanceId),
      enabled: !!instanceId,
      staleTime: STALE_MS,
    }),
  manifest: (handId: string, enabled = false) =>
    queryOptions({
      queryKey: handKeys.manifest(handId),
      queryFn: () => getHandManifestToml(handId),
      enabled: enabled && !!handId,
      staleTime: 60_000,
    }),
};

export function useHands() {
  return useQuery(handQueries.list());
}

export function useActiveHands() {
  return useQuery(handQueries.active());
}

export function useActiveHandsWhen(enabled: boolean) {
  return useQuery({
    ...handQueries.active(),
    enabled,
  });
}

export function useHandDetail(handId: string) {
  return useQuery(handQueries.detail(handId));
}

export function useHandSettings(handId: string) {
  return useQuery(handQueries.settings(handId));
}

export function useHandStats(instanceId: string) {
  return useQuery(handQueries.stats(instanceId));
}

export function useHandStatsBatch(instanceIds: readonly string[]) {
  return useQuery(handQueries.statsBatch(instanceIds));
}

export function useHandSession(instanceId: string) {
  return useQuery(handQueries.session(instanceId));
}

export function useHandInstanceStatus(instanceId: string) {
  return useQuery(handQueries.instanceStatus(instanceId));
}

// Lazy-load the raw HAND.toml. Disabled by default — caller passes
// `enabled: true` only when the viewer modal opens, so we don't fetch
// every hand's TOML eagerly.
export function useHandManifestToml(handId: string, enabled: boolean) {
  return useQuery(handQueries.manifest(handId, enabled));
}
