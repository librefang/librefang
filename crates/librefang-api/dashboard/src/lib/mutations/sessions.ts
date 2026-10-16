import { useMutation, useQueryClient } from "@tanstack/react-query";
import { setSessionLabel, setSessionModelOverride, setSessionReasoningOverride } from "../http/client";
import { agentKeys, sessionKeys } from "../queries/keys";

// Session switch/delete live in mutations/agents.ts as the canonical hooks
// (useSwitchAgentSession / useDeleteAgentSession) so both ChatPage and
// SessionsPage share one invalidation policy. Only session-scoped metadata
// edits remain here.

export function useSetSessionLabel() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ sessionId, label, agentId: _agentId }: { sessionId: string; label: string; agentId: string | null }) =>
      setSessionLabel(sessionId, label),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: sessionKeys.lists() });
      qc.invalidateQueries({ queryKey: sessionKeys.detail(variables.sessionId) });
      if (variables.agentId) {
        qc.invalidateQueries({ queryKey: agentKeys.sessions(variables.agentId) });
        qc.invalidateQueries({ queryKey: agentKeys.sessionSnapshots(variables.agentId) });
      }
    },
  });
}

export function useSetSessionModelOverride() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      sessionId,
      modelOverride,
    }: {
      sessionId: string;
      modelOverride: string | null;
      agentId: string | null;
    }) => setSessionModelOverride(sessionId, modelOverride),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: sessionKeys.lists() });
      qc.invalidateQueries({ queryKey: sessionKeys.detail(variables.sessionId) });
      if (variables.agentId) {
        qc.invalidateQueries({ queryKey: agentKeys.sessions(variables.agentId) });
        // Use the 3-element snapshot prefix so every cached
        // (agent, sessionId) snapshot — including ones keyed by an explicit
        // sessionId — is invalidated. `agentKeys.session(agentId)` resolves
        // to a 4-element key with `sessionId = null`, which only matches
        // the "no override" slot and leaves explicit-id snapshots stale.
        qc.invalidateQueries({ queryKey: agentKeys.sessionSnapshots(variables.agentId) });
      }
    },
  });
}

/**
 * Pin a reasoning effort to one session, or clear it with `null`.
 *
 * Same invalidation policy as {@link useSetSessionModelOverride} — the session
 * detail carries the pinned level back (the chat composer reads it from
 * there), the session list snapshot may too, and the agent-scoped caches must
 * not keep serving the previous value.
 */
export function useSetSessionReasoning() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      sessionId,
      reasoningEffort,
    }: {
      sessionId: string;
      reasoningEffort: string | null;
      agentId: string | null;
    }) => setSessionReasoningOverride(sessionId, reasoningEffort),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: sessionKeys.lists() });
      qc.invalidateQueries({ queryKey: sessionKeys.detail(variables.sessionId) });
      if (variables.agentId) {
        qc.invalidateQueries({ queryKey: agentKeys.sessions(variables.agentId) });
        qc.invalidateQueries({ queryKey: agentKeys.sessionSnapshots(variables.agentId) });
      }
    },
  });
}
