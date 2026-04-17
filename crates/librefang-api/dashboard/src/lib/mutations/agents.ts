import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  spawnAgent,
  cloneAgent,
  suspendAgent,
  resumeAgent,
  deleteAgent,
  patchAgentConfig,
  createAgentSession,
  switchAgentSession,
  deleteSession,
  deletePromptVersion,
  activatePromptVersion,
  createPromptVersion,
  createExperiment,
  startExperiment,
  pauseExperiment,
  completeExperiment,
  resolveApproval,
} from "../http/client";
import { agentKeys, approvalKeys, overviewKeys, sessionKeys } from "../queries/keys";

export function useSpawnAgent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: spawnAgent,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentKeys.all });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}

export function useCloneAgent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: cloneAgent,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentKeys.all });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}

export function useSuspendAgent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: suspendAgent,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentKeys.all });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}

export function useDeleteAgent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteAgent,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentKeys.all });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}

export function useResumeAgent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: resumeAgent,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentKeys.all });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}

export function usePatchAgentConfig() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      agentId,
      config,
    }: {
      agentId: string;
      config: {
        max_tokens?: number;
        model?: string;
        provider?: string;
        temperature?: number;
        web_search_augmentation?: "off" | "auto" | "always";
      };
    }) => patchAgentConfig(agentId, config),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.lists() });
      qc.invalidateQueries({ queryKey: agentKeys.detail(variables.agentId) });
    },
  });
}

export function useCreateAgentSession() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ agentId, label }: { agentId: string; label?: string }) =>
      createAgentSession(agentId, label),
    onSuccess: () => qc.invalidateQueries({ queryKey: agentKeys.all }),
  });
}

export function useSwitchAgentSession() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ agentId, sessionId }: { agentId: string; sessionId: string }) =>
      switchAgentSession(agentId, sessionId),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.detail(variables.agentId) });
      qc.invalidateQueries({ queryKey: agentKeys.sessions(variables.agentId) });
    },
  });
}

export function useDeleteAgentSession() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteSession,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentKeys.all });
      qc.invalidateQueries({ queryKey: sessionKeys.all });
    },
  });
}

export function useDeletePromptVersion() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ versionId, agentId: _agentId }: { versionId: string; agentId: string }) =>
      deletePromptVersion(versionId),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.promptVersions(variables.agentId) });
    },
  });
}

export function useActivatePromptVersion() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ versionId, agentId }: { versionId: string; agentId: string }) =>
      activatePromptVersion(versionId, agentId),
    onSuccess: () => qc.invalidateQueries({ queryKey: agentKeys.all }),
  });
}

export function useCreatePromptVersion() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      agentId,
      version,
    }: {
      agentId: string;
      version: Parameters<typeof createPromptVersion>[1];
    }) => createPromptVersion(agentId, version),
    onSuccess: () => qc.invalidateQueries({ queryKey: agentKeys.all }),
  });
}

export function useCreateExperiment() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      agentId,
      experiment,
    }: {
      agentId: string;
      experiment: Parameters<typeof createExperiment>[1];
    }) => createExperiment(agentId, experiment),
    onSuccess: () => qc.invalidateQueries({ queryKey: agentKeys.all }),
  });
}

export function useStartExperiment() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ experimentId, agentId: _agentId }: { experimentId: string; agentId: string }) =>
      startExperiment(experimentId),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.experiments(variables.agentId) });
      qc.invalidateQueries({ queryKey: agentKeys.experimentMetrics(variables.experimentId) });
    },
  });
}

export function usePauseExperiment() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ experimentId, agentId: _agentId }: { experimentId: string; agentId: string }) =>
      pauseExperiment(experimentId),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.experiments(variables.agentId) });
      qc.invalidateQueries({ queryKey: agentKeys.experimentMetrics(variables.experimentId) });
    },
  });
}

export function useCompleteExperiment() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ experimentId, agentId: _agentId }: { experimentId: string; agentId: string }) =>
      completeExperiment(experimentId),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.experiments(variables.agentId) });
      qc.invalidateQueries({ queryKey: agentKeys.experimentMetrics(variables.experimentId) });
    },
  });
}

export function useResolveApproval() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, approved }: { id: string; approved: boolean }) =>
      resolveApproval(id, approved),
    onSuccess: () => qc.invalidateQueries({ queryKey: approvalKeys.all }),
  });
}
