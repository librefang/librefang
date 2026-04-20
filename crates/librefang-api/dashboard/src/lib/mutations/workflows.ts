import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  runWorkflow,
  dryRunWorkflow,
  deleteWorkflow,
  createWorkflow,
  updateWorkflow,
  instantiateTemplate,
  saveWorkflowAsTemplate,
} from "../http/client";
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
    mutationFn: ({ workflowId, input }: { workflowId: string; input: string }) =>
      runWorkflow(workflowId, input),
    onSuccess: (_data, variables) => Promise.all([
      invalidateWorkflowLists(qc),
      invalidateWorkflowRecord(qc, variables.workflowId),
    ]),
  });
}

export function useDryRunWorkflow() {
  return useMutation({
    mutationFn: ({ workflowId, input }: { workflowId: string; input: string }) =>
      dryRunWorkflow(workflowId, input),
  });
}

export function useDeleteWorkflow() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteWorkflow,
    onSuccess: (_data, workflowId) => Promise.all([
      invalidateWorkflowLists(qc),
      invalidateWorkflowRecord(qc, workflowId),
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
    onSuccess: (_data, variables) => Promise.all([
      invalidateWorkflowLists(qc),
      invalidateWorkflowRecord(qc, variables.workflowId),
    ]),
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
