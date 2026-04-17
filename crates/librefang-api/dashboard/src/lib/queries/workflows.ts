import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listWorkflows,
  listWorkflowRuns,
  getWorkflowRun,
  listWorkflowTemplates,
} from "../http/client";
import { workflowKeys } from "./keys";

const STALE_MS = 30_000;
const REFRESH_MS = 30_000;
const RUN_STALE_MS = 10_000;
const RUN_REFETCH_MS = 30_000;
const TEMPLATE_STALE_MS = 300_000;

export const workflowQueries = {
  list: () =>
    queryOptions({
      queryKey: workflowKeys.lists(),
      queryFn: listWorkflows,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
  runs: (workflowId: string) =>
    queryOptions({
      queryKey: workflowKeys.runs(workflowId),
      queryFn: () => listWorkflowRuns(workflowId),
      enabled: !!workflowId,
      staleTime: RUN_STALE_MS,
      refetchInterval: RUN_REFETCH_MS,
    }),
  runDetail: (runId: string) =>
    queryOptions({
      queryKey: workflowKeys.runDetail(runId),
      queryFn: () => getWorkflowRun(runId),
      enabled: !!runId,
    }),
  templates: (q?: string, category?: string) =>
    queryOptions({
      queryKey: workflowKeys.templates({ q, category }),
      queryFn: () => listWorkflowTemplates(q, category),
      staleTime: TEMPLATE_STALE_MS,
    }),
};

export function useWorkflows() {
  return useQuery(workflowQueries.list());
}

export function useWorkflowRuns(workflowId: string) {
  return useQuery(workflowQueries.runs(workflowId));
}

export function useWorkflowRunDetail(runId: string) {
  return useQuery(workflowQueries.runDetail(runId));
}

export function useWorkflowTemplates(q?: string, category?: string) {
  return useQuery(workflowQueries.templates(q, category));
}
