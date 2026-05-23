import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  runWorkflow,
  dryRunWorkflow,
  deleteWorkflow,
  createWorkflow,
  updateWorkflow,
  instantiateTemplate,
  saveWorkflowAsTemplate,
  resolveOperatorStep,
} from "../http/client";
import type {
  WorkflowItem,
  WorkflowRunInput,
  OperatorActionVerb,
} from "../../api";
import { workflowKeys } from "../queries/keys";

function invalidateWorkflowLists(qc: ReturnType<typeof useQueryClient>) {
  return qc.invalidateQueries({ queryKey: workflowKeys.lists() });
}

function invalidateWorkflowRecord(
  qc: ReturnType<typeof useQueryClient>,
  workflowId: string,
) {
  return Promise.all([
    qc.invalidateQueries({ queryKey: workflowKeys.detail(workflowId) }),
    qc.invalidateQueries({ queryKey: workflowKeys.runs(workflowId) }),
  ]);
}

export function useRunWorkflow() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ workflowId, input }: { workflowId: string; input: WorkflowRunInput }) =>
      runWorkflow(workflowId, input),
    onSuccess: (data, variables) => {
      const invalidations: Array<Promise<unknown>> = [
        invalidateWorkflowLists(qc),
        qc.invalidateQueries({ queryKey: workflowKeys.runs(variables.workflowId) }),
      ];
      const runId = typeof data.run_id === "string" ? data.run_id : undefined;

      if (runId) {
        invalidations.push(
          qc.invalidateQueries({ queryKey: workflowKeys.runDetail(runId) }),
        );
      }

      return Promise.all(invalidations);
    },
  });
}

export function useDryRunWorkflow() {
  return useMutation({
    mutationFn: ({ workflowId, input }: { workflowId: string; input: WorkflowRunInput }) =>
      dryRunWorkflow(workflowId, input),
  });
}

export function useDeleteWorkflow() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteWorkflow,
    onSuccess: (_data, workflowId) => Promise.all([
      invalidateWorkflowLists(qc),
      qc.removeQueries({ queryKey: workflowKeys.detail(workflowId) }),
      qc.invalidateQueries({ queryKey: workflowKeys.runs(workflowId) }),
    ]),
  });
}

export function useCreateWorkflow() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: createWorkflow,
    onSuccess: () => invalidateWorkflowLists(qc),
  });
}

export function useUpdateWorkflow() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      workflowId,
      payload,
    }: {
      workflowId: string;
      payload: Parameters<typeof updateWorkflow>[1];
    }) => updateWorkflow(workflowId, payload),
    onSuccess: (data, variables) => {
      // Patch the cached workflow detail in place using the post-mutation
      // entity returned by the handler (#3832). Falls through to invalidate
      // as a belt-and-suspenders guard, and to cover the narrow race where
      // the handler returned a stale fallback body. List rows can preserve
      // shared fields (name, description, last_run, success_rate); we still
      // invalidate the lists for safety since they may include aggregates.
      const hasEntity =
        data && typeof data === "object" && "id" in data && (data as WorkflowItem).id;
      if (hasEntity) {
        qc.setQueryData<WorkflowItem>(
          workflowKeys.detail(variables.workflowId),
          data as WorkflowItem,
        );
      }
      return Promise.all([
        invalidateWorkflowLists(qc),
        invalidateWorkflowRecord(qc, variables.workflowId),
      ]);
    },
  });
}

export function useInstantiateTemplate() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, params }: { id: string; params: Record<string, unknown> }) =>
      instantiateTemplate(id, params),
    onSuccess: () => invalidateWorkflowLists(qc),
  });
}

export function useSaveWorkflowAsTemplate() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: saveWorkflowAsTemplate,
    onSuccess: () => qc.invalidateQueries({ queryKey: workflowKeys.templates() }),
  });
}

/** HITL operator-step resolution (#4977). Posts `{action, payload?, field?}`
 *  to `POST /api/workflows/runs/:run_id/operator`. The endpoint replies
 *  200 immediately and the run resumes asynchronously, so on success we
 *  invalidate the operator-pause inspector + the worklist + the run
 *  detail + the workflow's run list so every surface re-fetches the new
 *  state. */
export function useResolveOperatorStep() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      runId,
      action,
      payload,
      field,
    }: {
      runId: string;
      action: OperatorActionVerb;
      payload?: string;
      field?: string;
    }) => resolveOperatorStep(runId, { action, payload, field }),
    onSuccess: (_data, variables) => {
      // Scoped invalidation — only the surfaces that actually change
      // when an operator pause resolves. Previously this invalidated
      // `workflowKeys.all`, which cascaded a refetch of every
      // workflow list / template / unrelated run on the page; in a
      // large install with dozens of workflows on screen that's a
      // visible jank and an unnecessary load on the API.
      return Promise.all([
        // The inspected pause is resolved — drop it so the action bar
        // disappears.
        qc.invalidateQueries({
          queryKey: workflowKeys.operatorPause(variables.runId),
        }),
        // The whole operator worklist (pendingOperator + any other
        // operator-scoped subkeys) shifts: at minimum this run leaves
        // the pending list. Using `operatorAll()` rather than
        // `pendingOperator()` so any future operator-scoped queries
        // get invalidated together for free.
        qc.invalidateQueries({ queryKey: workflowKeys.operatorAll() }),
        // The run itself transitions Paused → Running (and shortly to
        // Completed / Failed), so the detail panel needs a refresh.
        qc.invalidateQueries({
          queryKey: workflowKeys.runDetail(variables.runId),
        }),
      ]);
    },
  });
}
