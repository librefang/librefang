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
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: modelKeys.lists() });
      qc.invalidateQueries({ queryKey: modelKeys.overrides(variables.modelKey) });
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
