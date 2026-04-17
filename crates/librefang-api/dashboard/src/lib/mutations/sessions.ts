import { useMutation, useQueryClient } from "@tanstack/react-query";
import { switchAgentSession, deleteSession, setSessionLabel } from "../http/client";
import { sessionKeys } from "../queries/keys";

export function useSwitchSession() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ agentId, sessionId }: { agentId: string; sessionId: string }) =>
      switchAgentSession(agentId, sessionId),
    onSuccess: () => qc.invalidateQueries({ queryKey: sessionKeys.all }),
  });
}

export function useDeleteSession() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteSession,
    onSuccess: () => qc.invalidateQueries({ queryKey: sessionKeys.all }),
  });
}

export function useSetSessionLabel() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ sessionId, label }: { sessionId: string; label: string }) =>
      setSessionLabel(sessionId, label),
    onSuccess: () => qc.invalidateQueries({ queryKey: sessionKeys.all }),
  });
}
