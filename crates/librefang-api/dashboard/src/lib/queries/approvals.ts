import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listApprovals,
  listPendingApprovals,
  fetchApprovalCount,
  queryApprovalAudit,
  totpStatus,
} from "../../api";
import { approvalKeys, totpKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_APPROVALS = 10_000;
const REFETCH_APPROVALS = 15_000;
const STALE_COUNT = 10_000;
const REFETCH_COUNT = 15_000;
const STALE_PENDING = 3_000;
const REFETCH_PENDING = 5_000;
const STALE_TOTP = 60_000;

export const approvalQueries = {
  list: () =>
    queryOptions({
      queryKey: approvalKeys.lists(),
      queryFn: listApprovals,
      staleTime: STALE_APPROVALS,
      refetchInterval: REFETCH_APPROVALS,
    }),
  count: () =>
    queryOptions({
      queryKey: approvalKeys.count(),
      queryFn: fetchApprovalCount,
      staleTime: STALE_COUNT,
      refetchInterval: REFETCH_COUNT,
    }),
  pending: (agentId?: string) =>
    queryOptions({
      queryKey: approvalKeys.pending(agentId),
      queryFn: () => listPendingApprovals(agentId),
      staleTime: STALE_PENDING,
      refetchInterval: REFETCH_PENDING,
    }),
  audit: (params: {
    limit?: number;
    offset?: number;
    agent_id?: string;
    tool_name?: string;
  } = {}) =>
    queryOptions({
      queryKey: approvalKeys.audit(params),
      queryFn: () => queryApprovalAudit(params),
      staleTime: 30_000,
    }),
  totpStatus: () =>
    queryOptions({
      queryKey: totpKeys.status(),
      queryFn: totpStatus,
      staleTime: STALE_TOTP,
    }),
};

export function useApprovals(options: QueryOverrides = {}) {
  return useQuery(withOverrides(approvalQueries.list(), options));
}

export function useApprovalCount(options: QueryOverrides = {}) {
  return useQuery(withOverrides(approvalQueries.count(), options));
}

export function usePendingApprovals(
  agentId?: string,
  options: QueryOverrides = {},
) {
  return useQuery({
    ...withOverrides(approvalQueries.pending(agentId), options),
    enabled: options.enabled ?? Boolean(agentId),
  });
}

export function useApprovalAudit(
  params: {
    limit?: number;
    offset?: number;
    agent_id?: string;
    tool_name?: string;
  } = {},
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(approvalQueries.audit(params), options));
}

export function useTotpStatus() {
  return useQuery(approvalQueries.totpStatus());
}
