import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  getStatus,
  getQueueStatus,
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

export const systemStatusQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.status(),
    queryFn: getStatus,
    staleTime: 30_000,
    refetchInterval: 30_000,
    refetchIntervalInBackground: false, // #3393
  });

export function useSystemStatus() {
  return useQuery(systemStatusQueryOptions());
}

export const queueStatusQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.queueStatus(),
    queryFn: getQueueStatus,
    staleTime: 15_000,
    refetchInterval: 15_000,
    refetchIntervalInBackground: false, // #3393
  });

export function useQueueStatus() {
  return useQuery(queueStatusQueryOptions());
}

export const healthDetailQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.healthDetail(),
    queryFn: getHealthDetail,
    staleTime: 30_000,
    refetchInterval: 30_000,
    refetchIntervalInBackground: false, // #3393
  });

export function useHealthDetail() {
  return useQuery(healthDetailQueryOptions());
}

export const securityStatusQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.security(),
    queryFn: getSecurityStatus,
    staleTime: 120_000,
    refetchInterval: 120_000,
    refetchIntervalInBackground: false, // #3393
  });

export function useSecurityStatus(options: QueryOverrides = {}) {
  return useQuery(withOverrides(securityStatusQueryOptions(), options));
}

export const auditRecentQueryOptions = (limit: number) =>
  queryOptions({
    queryKey: auditKeys.recent(limit),
    queryFn: () => listAuditRecent(limit),
    staleTime: 30_000,
    refetchInterval: 30_000,
    refetchIntervalInBackground: false, // #3393
  });

export function useAuditRecent(limit: number, options: QueryOverrides = {}) {
  return useQuery(withOverrides(auditRecentQueryOptions(limit), options));
}

export const auditVerifyQueryOptions = () =>
  queryOptions({
    queryKey: auditKeys.verify(),
    queryFn: verifyAuditChain,
    staleTime: 60_000,
    // No refetchInterval — chain verification is expensive; fetch on mount/focus only.
  });

export function useAuditVerify(options: QueryOverrides = {}) {
  return useQuery(withOverrides(auditVerifyQueryOptions(), options));
}

export const backupsQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.backups(),
    queryFn: listBackups,
    staleTime: 60_000,
    refetchInterval: 60_000,
    refetchIntervalInBackground: false, // #3393
  });

export function useBackups(options: QueryOverrides = {}) {
  return useQuery(withOverrides(backupsQueryOptions(), options));
}

export const taskQueueStatusQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.taskStatus(),
    queryFn: getTaskQueueStatus,
    staleTime: 15_000,
    refetchInterval: 15_000,
    refetchIntervalInBackground: false, // #3393
  });

export function useTaskQueueStatus() {
  return useQuery(taskQueueStatusQueryOptions());
}

export const taskQueueQueryOptions = (status?: string) =>
  queryOptions({
    queryKey: runtimeKeys.taskList(status),
    queryFn: () => listTaskQueue(status),
    staleTime: 30_000,
    refetchInterval: 30_000,
    refetchIntervalInBackground: false, // #3393
  });

export function useTaskQueue(status?: string) {
  return useQuery(taskQueueQueryOptions(status));
}

export const cronJobsQueryOptions = (agentId?: string) =>
  queryOptions({
    queryKey: cronKeys.jobs(agentId),
    queryFn: () => listCronJobs(agentId),
    enabled: !!agentId,
    staleTime: 30_000,
    refetchInterval: 30_000,
    refetchIntervalInBackground: false, // #3393
  });

export function useCronJobs(agentId?: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(cronJobsQueryOptions(agentId), options));
}
