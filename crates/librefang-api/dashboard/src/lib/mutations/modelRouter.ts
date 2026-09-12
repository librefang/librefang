import { useMutation, useQueryClient } from "@tanstack/react-query";
import { updateAgentModelRouting, type AgentModelRouting } from "../http/client";
import { agentKeys, modelRouterKeys } from "../queries/keys";

/**
 * Persist an agent's model selection mode and router override.
 *
 * The server echoes the stored settings back (normalised: the profile
 * allowlist comes back deduplicated and sorted, and a fixed-mode write clears
 * the override), so the detail cache is seeded from the response rather than
 * from the request body — otherwise the UI would briefly show what the user
 * typed instead of what was actually saved.
 *
 * `agentKeys.detail` is invalidated too because the agent detail payload
 * carries the model block this write can change.
 */
export function useUpdateAgentModelRouting() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      agentId,
      routing,
    }: {
      agentId: string;
      routing: AgentModelRouting;
    }) => updateAgentModelRouting(agentId, routing),
    onSuccess: (data, variables) => {
      qc.setQueryData(modelRouterKeys.agent(variables.agentId), data);
      qc.invalidateQueries({ queryKey: modelRouterKeys.all });
      qc.invalidateQueries({ queryKey: agentKeys.detail(variables.agentId) });
    },
  });
}
