import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createAgentType,
  updateAgentType,
  deleteAgentType,
  spawnEphemeral,
  type AgentTypeInput,
  type EphemeralSpawnRequest,
} from "../http/client";
import { agentTypeKeys } from "../queries/keys";

export function useCreateAgentType() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: AgentTypeInput) => createAgentType(body),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentTypeKeys.all });
    },
  });
}

export function useUpdateAgentType() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, body }: { name: string; body: AgentTypeInput }) =>
      updateAgentType(name, body),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentTypeKeys.all });
    },
  });
}

export function useDeleteAgentType() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => deleteAgentType(name),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentTypeKeys.all });
    },
  });
}

// A one-shot ephemeral run creates and tears down a transient worker with no
// registry entry, so nothing in the agent-type list changes — no invalidation
// needed. The result is returned to the caller for display.
export function useSpawnEphemeral() {
  return useMutation({
    mutationFn: (body: EphemeralSpawnRequest) => spawnEphemeral(body),
  });
}
