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
} from "../../api";
import { runtimeKeys, auditKeys, cronKeys } from "./keys";

export { useDashboardSnapshot, useVersionInfo } from "./overview";

type UseAuditRecentOptions = {
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?: number | false;
};

export const runtimePageRefreshers = [
  "refetchSnapshot",
  "refetchVersion",
  "refetchQueue",
  "refetchHealthDetail",
  "refetchSecurity",
  "refetchAuditRecent",
  "refetchAuditVerify",
  "refetchBackups",
  "refetchTaskStatus",
  "refetchTaskList",
] as const;

export type RuntimePageRefresherName =
  (typeof runtimePageRefreshers)[number];

export const systemStatusQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.status(),
    queryFn: getStatus,
    staleTime: 30_000,
    refetchInterval: 30_000,
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
  });

export function useHealthDetail() {
  return useQuery(healthDetailQueryOptions());
}

export const securityStatusQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.security(),
    queryFn: getSecurityStatus,
    staleTime: 60_000,
    refetchInterval: 120_000,
  });

export function useSecurityStatus() {
  return useQuery(securityStatusQueryOptions());
}

export const auditRecentQueryOptions = (limit: number) =>
  queryOptions({
    queryKey: auditKeys.recent(limit),
    queryFn: () => listAuditRecent(limit),
    staleTime: 30_000,
    refetchInterval: 30_000,
  });

export function useAuditRecent(
  limit: number,
  options: UseAuditRecentOptions = {},
) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...auditRecentQueryOptions(limit),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export const auditVerifyQueryOptions = () =>
  queryOptions({
    queryKey: auditKeys.verify(),
    queryFn: verifyAuditChain,
    staleTime: 60_000,
    refetchInterval: 120_000,
  });

export function useAuditVerify() {
  return useQuery(auditVerifyQueryOptions());
}

export const backupsQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.backups(),
    queryFn: listBackups,
    staleTime: 60_000,
    refetchInterval: 60_000,
  });

export function useBackups() {
  return useQuery(backupsQueryOptions());
}

export const taskQueueStatusQueryOptions = () =>
  queryOptions({
    queryKey: runtimeKeys.taskStatus(),
    queryFn: getTaskQueueStatus,
    staleTime: 15_000,
    refetchInterval: 15_000,
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
  });

export function useTaskQueue(status?: string) {
  return useQuery(taskQueueQueryOptions(status));
}

export function useCronJobs(agentId?: string) {
  return useQuery({
    queryKey: cronKeys.jobs(agentId),
    queryFn: () => listCronJobs(agentId),
    enabled: !!agentId,
    staleTime: 30_000,
    refetchInterval: 30_000,
  });
}
