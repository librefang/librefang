import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  addCustomModel,
  removeCustomModel,
  updateModelOverrides,
  deleteModelOverrides,
  type ModelOverrides,
} from "../http/client";
import { modelKeys } from "../queries/keys";

export function useAddCustomModel() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: addCustomModel,
    onSuccess: () => qc.invalidateQueries({ queryKey: modelKeys.lists() }),
  });
}

export function useRemoveCustomModel() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: removeCustomModel,
    onSuccess: () => qc.invalidateQueries({ queryKey: modelKeys.lists() }),
  });
}

export function useUpdateModelOverrides() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      modelKey,
      overrides,
    }: {
      modelKey: string;
      overrides: ModelOverrides;
    }) => updateModelOverrides(modelKey, overrides),
    onSuccess: (data, variables) => {
      // Server now returns the persisted ModelOverrides — seed the detail
      // cache directly so consumers don't need a follow-up GET. (Refs #3832.)
      qc.setQueryData(modelKeys.overrides(variables.modelKey), data);
      qc.invalidateQueries({ queryKey: modelKeys.lists() });
    },
  });
}

export function useDeleteModelOverrides() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteModelOverrides,
    onSuccess: (_data, modelKey) => {
      qc.invalidateQueries({ queryKey: modelKeys.lists() });
      qc.invalidateQueries({ queryKey: modelKeys.overrides(modelKey) });
    },
  });
}
