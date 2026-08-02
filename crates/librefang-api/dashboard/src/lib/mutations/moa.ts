import { useMutation, useQueryClient } from "@tanstack/react-query";
import { putMoaPreset, deleteMoaPreset, putMoaConfig, type MoaPreset } from "../http/client";
import { moaKeys } from "../queries/keys";

export function usePutMoaPreset() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, preset }: { name: string; preset: MoaPreset }) => putMoaPreset(name, preset),
    onSuccess: () => qc.invalidateQueries({ queryKey: moaKeys.presets() }),
  });
}

export function useDeleteMoaPreset() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteMoaPreset,
    onSuccess: () => qc.invalidateQueries({ queryKey: moaKeys.presets() }),
  });
}

export function usePutMoaConfig() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: putMoaConfig,
    onSuccess: () => qc.invalidateQueries({ queryKey: moaKeys.presets() }),
  });
}
