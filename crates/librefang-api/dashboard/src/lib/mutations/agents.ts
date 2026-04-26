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
  patchHandAgentRuntimeConfig,
  clearHandAgentRuntimeConfig,
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
  uploadAgentFile,
} from "../http/client";
import { agentKeys, approvalKeys, handKeys, overviewKeys, sessionKeys } from "../queries/keys";

/**
 * Unified payload type for the two agent-config PATCH endpoints.
 *
 * Both `/agents/{id}/config` (standalone agent) and
 * `/agents/{id}/hand-runtime-config` (hand-role override) accept the same
 * model-tuning subset; hand overrides additionally accept `api_key_env` and
 * `base_url` which are tri-state on the server:
 *   - absent       → leave existing override as-is
 *   - empty string → clear that specific field
 *   - non-empty    → set to the provided value
 *
 * Non-hand callers simply never send `api_key_env` / `base_url`; the backend
 * ignores them on the standalone `/config` route.
 */
export type AgentConfigPatch = {
  max_tokens?: number;
  model?: string;
  provider?: string;
  temperature?: number;
  api_key_env?: string;
  base_url?: string;
  web_search_augmentation?: "off" | "auto" | "always";
};

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

/**
 * PATCH /agents/{id}/config — model-tuning update for a **non-hand** agent.
 *
 * Hand-role agents MUST use `usePatchHandAgentRuntimeConfig` instead; the
 * two backends write to different config slots and invalidation fan-out
 * differs (hand overrides also dirty `handKeys.details()`). Branching on
 * `is_hand` is the caller's job because only the caller knows — from the
 * cached agent detail — whether this id refers to a hand role.
 */
export function usePatchAgentConfig() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      agentId,
      config,
    }: {
      agentId: string;
      config: AgentConfigPatch;
    }) => patchAgentConfig(agentId, config),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.lists() });
      qc.invalidateQueries({ queryKey: agentKeys.detail(variables.agentId) });
    },
  });
}

/**
 * PATCH /agents/{id}/hand-runtime-config — per-agent hand runtime override.
 *
 * Accepts the same model-tuning subset as `usePatchAgentConfig` plus
 * `api_key_env` / `base_url` (tri-state; empty string clears).
 *
 * Invalidates:
 * - `agentKeys.lists()` — the model/provider badge in the agent list row
 *   reads from the live manifest which is what this override feeds into.
 * - `agentKeys.detail(id)` — the config panel bound to this hook reads
 *   the same manifest fields.
 * - `handKeys.details()` — the hand-detail view shows per-role runtime
 *   override state, so any cached hand detail referencing this agent's
 *   role must refetch to stay consistent with
 *   `useClearHandAgentRuntimeConfig`.
 */
export function usePatchHandAgentRuntimeConfig() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      agentId,
      config,
    }: {
      agentId: string;
      config: AgentConfigPatch;
    }) => patchHandAgentRuntimeConfig(agentId, config),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: agentKeys.lists() });
      qc.invalidateQueries({ queryKey: agentKeys.detail(variables.agentId) });
      qc.invalidateQueries({ queryKey: handKeys.details() });
    },
  });
}

/**
 * DELETE the per-agent hand runtime override — restores the live manifest
 * to the HAND.toml defaults on the server side. Invalidates:
 *
 * - `agentKeys.lists()` because the model/provider badge surfaced in the
 *   agent list row comes from the live manifest.
 * - `agentKeys.detail(agentId)` because the config panel bound to this
 *   hook reads the same manifest fields.
 * - `handKeys.details()` because the hand-detail view shows per-role
 *   runtime override state; the coordinator agent's clear is observable
 *   through any cached hand detail that references this agent's role.
 */
export function useClearHandAgentRuntimeConfig() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (agentId: string) => clearHandAgentRuntimeConfig(agentId),
    onSuccess: (_data, agentId) => {
      qc.invalidateQueries({ queryKey: agentKeys.lists() });
      qc.invalidateQueries({ queryKey: agentKeys.detail(agentId) });
      qc.invalidateQueries({ queryKey: handKeys.details() });
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

// Upload a chat attachment for the given agent. Returns the metadata that
// callers must thread back through the next /message or WS frame as
// `attachments[]` — uploads not referenced by a message stay orphaned in
// the registry until the daemon restarts.
//
// Intentionally does NOT call invalidateQueries: the upload only registers
// a file_id server-side, and no React Query cache reads UPLOAD_REGISTRY
// directly. The file becomes visible in the UI only after it's referenced
// in a /message call, which goes through useSendAgentMessage and triggers
// the appropriate session invalidation there.
export function useUploadAgentFile() {
  return useMutation({
    mutationFn: ({ agentId, file }: { agentId: string; file: File }) =>
      uploadAgentFile(agentId, file),
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
