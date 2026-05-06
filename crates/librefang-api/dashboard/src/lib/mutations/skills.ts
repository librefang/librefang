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
import {
  skillKeys,
  fanghubKeys,
  clawhubKeys,
  clawhubCnKeys,
  skillhubKeys,
} from "../queries/keys";

// Install/uninstall flips `is_installed` on every hub's browse / search /
// detail responses (the daemon computes it from the local skills directory),
// so any successful mutation must invalidate _all_ hub query domains in
// addition to the installed-skills list. Otherwise the source-of-skill grid
// keeps showing stale "Install" buttons until the next refetchInterval — see
// #4689 (FangHub Installed-tab gap, SkillHub / ClawHub / ClawHub-CN button
// not flipping post-install).
function invalidateAllSkillSurfaces(qc: ReturnType<typeof useQueryClient>) {
  qc.invalidateQueries({ queryKey: skillKeys.all });
  qc.invalidateQueries({ queryKey: fanghubKeys.all });
  qc.invalidateQueries({ queryKey: clawhubKeys.all });
  qc.invalidateQueries({ queryKey: clawhubCnKeys.all });
  qc.invalidateQueries({ queryKey: skillhubKeys.all });
}

export function useInstallSkill() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, hand }: { name: string; hand?: string }) =>
      installSkill(name, hand),
    onSuccess: () => invalidateAllSkillSurfaces(qc),
  });
}

export function useUninstallSkill() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: uninstallSkill,
    onSuccess: () => invalidateAllSkillSurfaces(qc),
  });
}

export function useClawHubInstall() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ slug, version, hand }: { slug: string; version?: string; hand?: string }) =>
      clawhubInstall(slug, version, hand),
    onSuccess: () => invalidateAllSkillSurfaces(qc),
  });
}

export function useClawHubCnInstall() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ slug, version, hand }: { slug: string; version?: string; hand?: string }) =>
      clawhubCnInstall(slug, version, hand),
    onSuccess: () => invalidateAllSkillSurfaces(qc),
  });
}

export function useSkillHubInstall() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ slug, hand }: { slug: string; hand?: string }) =>
      skillhubInstall(slug, hand),
    onSuccess: () => invalidateAllSkillSurfaces(qc),
  });
}

export function useFangHubInstall() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, hand }: { name: string; hand?: string }) =>
      installSkill(name, hand),
    onSuccess: () => invalidateAllSkillSurfaces(qc),
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
    // Reload re-reads every skill manifest from disk; any hub's browse cache
    // could now show a different `is_installed`, so invalidate every surface.
    onSuccess: () => invalidateAllSkillSurfaces(qc),
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
