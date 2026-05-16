import { useMutation, useQueryClient } from "@tanstack/react-query";
import { setSessionLabel, setSessionModelOverride } from "../http/client";
import { agentKeys, sessionKeys } from "../queries/keys";

// Session switch/delete live in mutations/agents.ts as the canonical hooks
// (useSwitchAgentSession / useDeleteAgentSession) so both ChatPage and
// SessionsPage share one invalidation policy. Only session-scoped metadata
// edits remain here.

export function useSetSessionLabel() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ sessionId, label, agentId: _agentId }: { sessionId: string; label: string; agentId?: string }) =>
      setSessionLabel(sessionId, label),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: sessionKeys.lists() });
      qc.invalidateQueries({ queryKey: sessionKeys.detail(variables.sessionId) });
      if (variables.agentId) {
        qc.invalidateQueries({ queryKey: agentKeys.sessions(variables.agentId) });
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
      agentId?: string;
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
