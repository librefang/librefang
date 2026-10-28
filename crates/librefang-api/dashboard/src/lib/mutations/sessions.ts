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

/** The only field a reasoning pin changes on the cached session record. */
function withReasoningEffort(previous: unknown, value: string | null): unknown {
  if (!previous || typeof previous !== "object") return previous;
  return { ...(previous as Record<string, unknown>), reasoning_effort: value };
}

/**
 * Pin a reasoning effort to one session, or clear it with `null`.
 *
 * Pinning changes one scalar, not the transcript. Refetching either full
 * session read (`sessionKeys.detail` and the agent-scoped history snapshot)
 * after every click would re-download the conversation just to learn the level
 * the caller already knows. The cached scalar is patched in place instead,
 * optimistically before the PATCH and with the server's echoed value on
 * success — the route validates and normalizes the token (it does not coerce
 * it to a level the model supports), so the echo is authoritative about the
 * spelling. Only the cheap session list is invalidated.
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
    onMutate: (variables) => {
      const key = sessionKeys.detail(variables.sessionId);
      const previous = qc.getQueryData(key);
      if (previous !== undefined) {
        qc.setQueryData(key, withReasoningEffort(previous, variables.reasoningEffort));
      }
      return { previous };
    },
    onError: (_error, variables, context) => {
      if (context?.previous !== undefined) {
        qc.setQueryData(sessionKeys.detail(variables.sessionId), context.previous);
      }
    },
    onSuccess: (data, variables) => {
      const key = sessionKeys.detail(variables.sessionId);
      const echoed = data.reasoning_effort;
      const accepted = typeof echoed === "string" ? echoed : variables.reasoningEffort;
      const cached = qc.getQueryData(key);
      if (cached !== undefined) {
        qc.setQueryData(key, withReasoningEffort(cached, accepted));
      }
      qc.invalidateQueries({ queryKey: sessionKeys.lists() });
      if (variables.agentId) {
        qc.invalidateQueries({ queryKey: agentKeys.sessions(variables.agentId) });
      }
    },
  });
}
