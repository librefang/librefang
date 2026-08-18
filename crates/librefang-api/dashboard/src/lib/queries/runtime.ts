import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  getStatus,
  getQueueStatus,
  getHealth,
  getHealthDetail,
  getSecurityStatus,
  listAuditRecent,
  verifyAuditChain,
  listBackups,
  getTaskQueueStatus,
  listTaskQueue,
  listCronJobs,
} from "../http/client";
import { runtimeKeys, auditKeys, cronKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const POLL_INTERVAL_MS = {
  fast: 15_000,
  standard: 30_000,
  slow: 60_000,
  security: 120_000,
} as const;

export const systemStatusQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.status(),
    queryFn: getStatus,
    staleTime: POLL_INTERVAL_MS.standard,
    refetchInterval: POLL_INTERVAL_MS.standard,
    refetchIntervalInBackground: false, // #3393
  });

export function useSystemStatus(options: QueryOverrides = {}) {
  return useQuery(withOverrides(systemStatusQueryOptions(), options));
}

export const queueStatusQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.queueStatus(),
    queryFn: getQueueStatus,
    staleTime: POLL_INTERVAL_MS.fast,
    refetchInterval: POLL_INTERVAL_MS.fast,
    refetchIntervalInBackground: false, // #3393
  });

export function useQueueStatus(options: QueryOverrides = {}) {
  return useQuery(withOverrides(queueStatusQueryOptions(), options));
}

export const healthDetailQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.healthDetail(),
    queryFn: getHealthDetail,
    staleTime: POLL_INTERVAL_MS.standard,
    refetchInterval: POLL_INTERVAL_MS.standard,
    refetchIntervalInBackground: false, // #3393
  });

export function useHealthDetail(options: QueryOverrides = {}) {
  return useQuery(withOverrides(healthDetailQueryOptions(), options));
}

/**
 * Minimal-liveness query for `<OfflineBanner />`. Anchored on `/api/health`
 * (always-public) rather than `/api/health/detail` (auth-required, operational
 * telemetry) — see api.ts:getHealth for the rationale (#4868 review fix).
 */
export const healthLivenessQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.healthLiveness(),
    queryFn: getHealth,
    staleTime: POLL_INTERVAL_MS.standard,
    refetchInterval: POLL_INTERVAL_MS.standard,
    refetchIntervalInBackground: false, // #3393
  });

export function useHealthLiveness(options: QueryOverrides = {}) {
  return useQuery(withOverrides(healthLivenessQueryOptions(), options));
}

export const securityStatusQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.security(),
    queryFn: getSecurityStatus,
    staleTime: POLL_INTERVAL_MS.security,
    refetchInterval: POLL_INTERVAL_MS.security,
    refetchIntervalInBackground: false, // #3393
  });

export function useSecurityStatus(options: QueryOverrides = {}) {
  return useQuery(withOverrides(securityStatusQueryOptions(), options));
}

export const auditRecentQueryOptions = (limit: number) =>
  queryOptions({
    queryKey: auditKeys.recent(limit),
    queryFn: () => listAuditRecent(limit),
    staleTime: POLL_INTERVAL_MS.standard,
    refetchInterval: POLL_INTERVAL_MS.standard,
    refetchIntervalInBackground: false, // #3393
  });

export function useAuditRecent(limit: number, options: QueryOverrides = {}) {
  return useQuery(withOverrides(auditRecentQueryOptions(limit), options));
}

export const auditVerifyQueryOptions = () =>
  queryOptions({
    queryKey: auditKeys.verify(),
    queryFn: verifyAuditChain,
    staleTime: POLL_INTERVAL_MS.slow,
    // No refetchInterval — chain verification is expensive; fetch on mount/focus only.
  });

export function useAuditVerify(options: QueryOverrides = {}) {
  return useQuery(withOverrides(auditVerifyQueryOptions(), options));
}

export const backupsQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.backups(),
    queryFn: listBackups,
    staleTime: POLL_INTERVAL_MS.slow,
    refetchInterval: POLL_INTERVAL_MS.slow,
    refetchIntervalInBackground: false, // #3393
  });

export function useBackups(options: QueryOverrides = {}) {
  return useQuery(withOverrides(backupsQueryOptions(), options));
}

export const taskQueueStatusQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.taskStatus(),
    queryFn: getTaskQueueStatus,
    staleTime: POLL_INTERVAL_MS.fast,
    refetchInterval: POLL_INTERVAL_MS.fast,
    refetchIntervalInBackground: false, // #3393
  });

export function useTaskQueueStatus(options: QueryOverrides = {}) {
  return useQuery(withOverrides(taskQueueStatusQueryOptions(), options));
}

export const taskQueueQueryOptions = (status?: string) =>
  queryOptions({
    queryKey: runtimeKeys.taskList(status),
    queryFn: () => listTaskQueue(status),
    staleTime: POLL_INTERVAL_MS.standard,
    refetchInterval: POLL_INTERVAL_MS.standard,
    refetchIntervalInBackground: false, // #3393
  });

export function useTaskQueue(status?: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(taskQueueQueryOptions(status), options));
}

export const cronJobsQueryOptions = (agentId?: string) =>
  queryOptions({
    queryKey: cronKeys.jobs(agentId),
    queryFn: () => listCronJobs(agentId),
    staleTime: POLL_INTERVAL_MS.standard,
    refetchInterval: POLL_INTERVAL_MS.standard,
    refetchIntervalInBackground: false, // #3393
  });

export function useCronJobs(agentId?: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(cronJobsQueryOptions(agentId), options));
}
