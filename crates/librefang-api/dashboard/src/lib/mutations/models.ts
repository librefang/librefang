import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  addCustomModel,
  removeCustomModel,
  updateModelOverrides,
  deleteModelOverrides,
} from "../http/client";
import { modelKeys } from "../queries/keys";

export function useAddCustomModel() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: addCustomModel,
    onSuccess: () => qc.invalidateQueries({ queryKey: modelKeys.all }),
  });
}

export function useRemoveCustomModel() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: removeCustomModel,
    onSuccess: () => qc.invalidateQueries({ queryKey: modelKeys.all }),
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
      overrides: import("../http/client").ModelOverrides;
    }) => updateModelOverrides(modelKey, overrides),
    onSuccess: () => qc.invalidateQueries({ queryKey: modelKeys.all }),
  });
}

export function useDeleteModelOverrides() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteModelOverrides,
    onSuccess: () => qc.invalidateQueries({ queryKey: modelKeys.all }),
  });
}
