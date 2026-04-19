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

export function useRunWorkflow() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ workflowId, input }: { workflowId: string; input: string }) =>
      runWorkflow(workflowId, input),
    onSuccess: async (_data, { workflowId }) => {
      await Promise.all([
        qc.invalidateQueries({ queryKey: workflowKeys.lists() }),
        qc.invalidateQueries({ queryKey: workflowKeys.runs(workflowId) }),
        qc.invalidateQueries({ queryKey: workflowKeys.runDetails() }),
      ]);
    },
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
    onSuccess: () => qc.invalidateQueries({ queryKey: workflowKeys.all }),
  });
}

export function useCreateWorkflow() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: createWorkflow,
    onSuccess: () => qc.invalidateQueries({ queryKey: workflowKeys.all }),
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
    onSuccess: () => qc.invalidateQueries({ queryKey: workflowKeys.all }),
  });
}

export function useInstantiateTemplate() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, params }: { id: string; params: Record<string, unknown> }) =>
      instantiateTemplate(id, params),
    onSuccess: () => qc.invalidateQueries({ queryKey: workflowKeys.all }),
  });
}

export function useSaveWorkflowAsTemplate() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: saveWorkflowAsTemplate,
    onSuccess: () => qc.invalidateQueries({ queryKey: workflowKeys.templates() }),
  });
}
