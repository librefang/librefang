import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  spawnAgent,
  cloneAgent,
  stopAgent,
  suspendAgent,
  resumeAgent,
  deleteAgent,
  patchAgent,
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
      qc.invalidateQueries({ queryKey: agentKeys.lists() });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}

export function useCloneAgent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: cloneAgent,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentKeys.lists() });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}

// Abort an in-flight agent run. The backend aborts the kernel task; the UI
// side separately reconciles streaming state (see ChatPage.stopMessage), so
// this hook intentionally doesn't invalidate queries — agent list state is
// unchanged by a stop.
export function useStopAgent() {
  return useMutation({
    mutationFn: (agentId: string) => stopAgent(agentId),
  });
}

export function useSuspendAgent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: suspendAgent,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentKeys.lists() });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}

export function useDeleteAgent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteAgent,
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.lists() });
      qc.removeQueries({ queryKey: agentKeys.detail(variables) });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}

export function useResumeAgent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: resumeAgent,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentKeys.lists() });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}

/**
 * Manifest-level partial update: name, description, system_prompt,
 * mcp_servers, model. Distinct from `usePatchAgentConfig` which targets
 * `/agents/{id}/config` (model-tuning only).
 */
export function usePatchAgent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      agentId,
      body,
    }: {
      agentId: string;
      body: {
        name?: string;
        description?: string;
        system_prompt?: string;
        model?: string;
        provider?: string;
        mcp_servers?: string[];
      };
    }) => patchAgent(agentId, body),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.lists() });
      qc.invalidateQueries({ queryKey: agentKeys.detail(variables.agentId) });
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
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.sessions(variables.agentId) });
      qc.invalidateQueries({ queryKey: agentKeys.detail(variables.agentId) });
      qc.invalidateQueries({ queryKey: sessionKeys.lists() });
    },
  });
}

// Canonical session-switch hook. Invalidates both cache slices so ChatPage
// (agent-scoped sessions list) and SessionsPage (global sessions list) stay
// in sync regardless of which page triggered the switch.
export function useSwitchAgentSession() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ agentId, sessionId }: { agentId: string; sessionId: string }) =>
      switchAgentSession(agentId, sessionId),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.detail(variables.agentId) });
      qc.invalidateQueries({ queryKey: agentKeys.sessions(variables.agentId) });
      qc.invalidateQueries({ queryKey: sessionKeys.lists() });
    },
  });
}

// Canonical session-delete hook. Caller supplies `agentId` when known so the
// agent-scoped sessions list can be narrowly invalidated; otherwise we fall
// back to invalidating the full agents cache. Always invalidates the global
// sessions list so SessionsPage stays fresh.
export function useDeleteAgentSession() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ sessionId }: { sessionId: string; agentId?: string }) =>
      deleteSession(sessionId),
    onSuccess: (_data, variables) => {
      if (variables.agentId) {
        qc.invalidateQueries({ queryKey: agentKeys.sessions(variables.agentId) });
        qc.invalidateQueries({ queryKey: agentKeys.detail(variables.agentId) });
      } else {
        qc.invalidateQueries({ queryKey: agentKeys.all });
      }
      qc.invalidateQueries({ queryKey: sessionKeys.lists() });
    },
  });
}

export function useDeletePromptVersion() {
  const qc = useQueryClient();
  return useMutation({
    // agentId aliased to _agentId so it's available as variables.agentId in
    // onSuccess for targeted invalidation, but not passed to the API call.
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
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.promptVersions(variables.agentId) });
      // Active version may be surfaced on the agent detail view.
      qc.invalidateQueries({ queryKey: agentKeys.detail(variables.agentId) });
    },
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
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.promptVersions(variables.agentId) });
    },
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
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.experiments(variables.agentId) });
    },
  });
}

export function useStartExperiment() {
  const qc = useQueryClient();
  return useMutation({
    // agentId aliased to _agentId so it's available as variables.agentId in
    // onSuccess for targeted invalidation, but not passed to the API call.
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
    // agentId aliased to _agentId so it's available as variables.agentId in
    // onSuccess for targeted invalidation, but not passed to the API call.
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
    // agentId aliased to _agentId so it's available as variables.agentId in
    // onSuccess for targeted invalidation, but not passed to the API call.
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
