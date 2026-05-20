import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listWorkflows,
  getWorkflow,
  listWorkflowRuns,
  getWorkflowRun,
  listWorkflowTemplates,
  inspectOperatorPause,
  listPendingOperatorRuns,
  ApiError,
} from "../http/client";
import { workflowKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

/** Stale/refetch timing constants.
 *  STALE_MS / REFRESH_MS — workflow list: 30 s stale, 30 s poll.
 *  RUN_STALE_MS / RUN_REFETCH_MS — run list: 10 s stale (fast-changing), 30 s poll.
 *  RUN_DETAIL_STALE_MS — single run detail: 30 s stale, no background poll (fetch-on-focus only).
 *  TEMPLATE_STALE_MS — templates change rarely: 5 min stale, no poll.
 *  OPERATOR_PENDING_STALE_MS / OPERATOR_PENDING_REFETCH_MS — HITL worklist:
 *    5 s stale, 15 s poll (matches the cadence the human operator needs to
 *    see new pauses arrive without thrashing the API).
 */
const STALE_MS = 30_000;
const REFRESH_MS = 30_000;
const RUN_STALE_MS = 10_000;
const RUN_REFETCH_MS = 30_000;
const RUN_DETAIL_STALE_MS = 30_000;
const TEMPLATE_STALE_MS = 300_000;
const OPERATOR_PENDING_STALE_MS = 5_000;
const OPERATOR_PENDING_REFETCH_MS = 15_000;

export const workflowQueries = {
  list: () =>
    queryOptions({
      queryKey: workflowKeys.lists(),
      queryFn: listWorkflows,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  detail: (workflowId: string) =>
    queryOptions({
      queryKey: workflowKeys.detail(workflowId),
      queryFn: () => getWorkflow(workflowId),
      enabled: !!workflowId,
      staleTime: STALE_MS,
    }),
  runs: (workflowId: string) =>
    queryOptions({
      queryKey: workflowKeys.runs(workflowId),
      queryFn: () => listWorkflowRuns(workflowId),
      enabled: !!workflowId,
      staleTime: RUN_STALE_MS,
      refetchInterval: RUN_REFETCH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  runDetail: (runId: string) =>
    queryOptions({
      queryKey: workflowKeys.runDetail(runId),
      queryFn: () => getWorkflowRun(runId),
      enabled: !!runId,
      staleTime: RUN_DETAIL_STALE_MS,
    }),
  templates: (q?: string, category?: string) =>
    queryOptions({
      queryKey: workflowKeys.templates({ q, category }),
      queryFn: () => listWorkflowTemplates(q, category),
      staleTime: TEMPLATE_STALE_MS,
    }),
  // HITL operator-step worklist (#4977). Polls every 15 s so the
  // human operator sees newly-arrived pauses without manually refetching.
  pendingOperator: () =>
    queryOptions({
      queryKey: workflowKeys.pendingOperator(),
      queryFn: listPendingOperatorRuns,
      staleTime: OPERATOR_PENDING_STALE_MS,
      refetchInterval: OPERATOR_PENDING_REFETCH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  // Single-run operator-pause inspector. The 409 ("not_operator_pause")
  // response is a legitimate not-pending state — the consumer reads it
  // by branching on `ApiError.status === 409` rather than retrying.
  operatorPause: (runId: string) =>
    queryOptions({
      queryKey: workflowKeys.operatorPause(runId),
      queryFn: () => inspectOperatorPause(runId),
      enabled: !!runId,
      staleTime: OPERATOR_PENDING_STALE_MS,
      retry: (failureCount, error) => {
        // Don't retry "this run isn't paused at an operator step" — that's
        // a stable state, not a transient failure.
        if (error instanceof ApiError && (error.status === 404 || error.status === 409)) {
          return false;
        }
        return failureCount < 2;
      },
    }),
};

export function useWorkflows(options: QueryOverrides = {}) {
  return useQuery(withOverrides(workflowQueries.list(), options));
}

export function useWorkflowDetail(workflowId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(workflowQueries.detail(workflowId), options));
}

export function useWorkflowRuns(workflowId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(workflowQueries.runs(workflowId), options));
}

export function useWorkflowRunDetail(runId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(workflowQueries.runDetail(runId), options));
}

export function useWorkflowTemplates(q?: string, category?: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(workflowQueries.templates(q, category), options));
}

/** HITL operator-step worklist (#4977). Used by the WorkflowsPage banner
 *  + the Approvals page section that surface pending operator reviews. */
export function usePendingOperatorRuns(options: QueryOverrides = {}) {
  return useQuery(withOverrides(workflowQueries.pendingOperator(), options));
}

/** Single-run operator-pause inspector. Returns the artifact + the
 *  allowed actions for one paused run; the consumer renders the action
 *  bar from `data.actions`. Caller is expected to branch on the 404 /
 *  409 (`error.status` from `ApiError`) to render "this run isn't
 *  paused at an operator step" instead of the action bar. */
export function useWorkflowOperatorPause(runId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(workflowQueries.operatorPause(runId), options));
}
