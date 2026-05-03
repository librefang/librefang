import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listWorkflows,
  getWorkflow,
  listWorkflowRuns,
  getWorkflowRun,
  listWorkflowTemplates,
} from "../http/client";
import { workflowKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

/** Stale/refetch timing constants.
 *  STALE_MS / REFRESH_MS — workflow list: 30 s stale, 30 s poll.
 *  RUN_STALE_MS / RUN_REFETCH_MS — run list: 10 s stale (fast-changing), 30 s poll.
 *  RUN_DETAIL_STALE_MS — single run detail: 30 s stale, no background poll (fetch-on-focus only).
 *  TEMPLATE_STALE_MS — templates change rarely: 5 min stale, no poll.
 */
const STALE_MS = 30_000;
const REFRESH_MS = 30_000;
const RUN_STALE_MS = 10_000;
const RUN_REFETCH_MS = 30_000;
const RUN_DETAIL_STALE_MS = 30_000;
const TEMPLATE_STALE_MS = 300_000;

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
