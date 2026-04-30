import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  installSkill,
  uninstallSkill,
  clawhubInstall,
  clawhubCnInstall,
  skillhubInstall,
  createSkill,
  reloadSkills,
  evolveUpdateSkill,
  evolvePatchSkill,
  evolveRollbackSkill,
  evolveDeleteSkill,
  evolveWriteFile,
  evolveRemoveFile,
} from "../http/client";
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

export function useClawHubCnInstall() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ slug, version, hand }: { slug: string; version?: string; hand?: string }) =>
      clawhubCnInstall(slug, version, hand),
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

export function useCreateSkill() {
  const qc = useQueryClient();
  return useMutation({
    // Accept an optional signal so callers can cancel on unmount.
    mutationFn: (vars: {
      name: string;
      description: string;
      prompt_context: string;
      tags?: string[];
      signal?: AbortSignal;
    }) => {
      const { signal, ...params } = vars;
      return createSkill(params, signal);
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: skillKeys.lists() }),
  });
}

export function useReloadSkills() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: reloadSkills,
    onSuccess: () => qc.invalidateQueries({ queryKey: skillKeys.all }),
  });
}

export function useEvolveUpdateSkill() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      name,
      params,
    }: {
      name: string;
      params: { prompt_context: string; changelog: string };
    }) => evolveUpdateSkill(name, params),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: skillKeys.detail(variables.name) });
      qc.invalidateQueries({ queryKey: skillKeys.lists() });
    },
  });
}

export function useEvolvePatchSkill() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      name,
      params,
    }: {
      name: string;
      params: {
        old_string: string;
        new_string: string;
        changelog: string;
        replace_all: boolean;
      };
    }) => evolvePatchSkill(name, params),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: skillKeys.detail(variables.name) });
      qc.invalidateQueries({ queryKey: skillKeys.lists() });
    },
  });
}

export function useEvolveRollbackSkill() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name }: { name: string }) => evolveRollbackSkill(name),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: skillKeys.detail(variables.name) });
      qc.invalidateQueries({ queryKey: skillKeys.lists() });
    },
  });
}

export function useEvolveDeleteSkill() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name }: { name: string }) => evolveDeleteSkill(name),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: skillKeys.detail(variables.name) });
      qc.removeQueries({ queryKey: skillKeys.supportingFiles(variables.name) });
      qc.invalidateQueries({ queryKey: skillKeys.lists() });
    },
  });
}

export function useEvolveWriteFile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      name,
      params,
    }: {
      name: string;
      params: { path: string; content: string };
    }) => evolveWriteFile(name, params),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: skillKeys.detail(variables.name) });
      qc.invalidateQueries({ queryKey: skillKeys.supportingFile(variables.name, variables.params.path) });
    },
  });
}

export function useEvolveRemoveFile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, path }: { name: string; path: string }) =>
      evolveRemoveFile(name, path),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: skillKeys.detail(variables.name) });
      qc.removeQueries({ queryKey: skillKeys.supportingFile(variables.name, variables.path) });
    },
  });
}
