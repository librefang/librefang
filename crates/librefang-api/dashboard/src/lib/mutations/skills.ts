import { useMutation, useQueryClient } from "@tanstack/react-query";
import { installSkill, uninstallSkill, clawhubInstall, skillhubInstall } from "../http/client";
import { skillKeys, fanghubKeys } from "../queries/keys";

export function useInstallSkill() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, hand }: { name: string; hand?: string }) =>
      installSkill(name, hand),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: skillKeys.all });
      qc.invalidateQueries({ queryKey: fanghubKeys.all });
    },
  });
}

export function useUninstallSkill() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: uninstallSkill,
    onSuccess: () => qc.invalidateQueries({ queryKey: skillKeys.all }),
  });
}

export function useClawHubInstall() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ slug, version, hand }: { slug: string; version?: string; hand?: string }) =>
      clawhubInstall(slug, version, hand),
    onSuccess: () => qc.invalidateQueries({ queryKey: skillKeys.all }),
  });
}

export function useSkillHubInstall() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ slug, hand }: { slug: string; hand?: string }) =>
      skillhubInstall(slug, hand),
    onSuccess: () => qc.invalidateQueries({ queryKey: skillKeys.all }),
  });
}

export function useFangHubInstall() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, hand }: { name: string; hand?: string }) =>
      installSkill(name, hand),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: skillKeys.all });
      qc.invalidateQueries({ queryKey: fanghubKeys.all });
    },
  });
}
