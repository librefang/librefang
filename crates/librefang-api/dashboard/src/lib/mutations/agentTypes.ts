import { useMutation, useQueryClient } from "@tanstack/react-query";
import { createAgentType, updateAgentType, deleteAgentType } from "../http/client";
import type { AgentTypeSpec } from "../../api";
import { agentTypeKeys } from "../queries/keys";

export function useCreateAgentType() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (spec: AgentTypeSpec) => createAgentType(spec),
    onSuccess: () => qc.invalidateQueries({ queryKey: agentTypeKeys.all }),
  });
}

/**
 * Save an edit to an existing agent type.
 *
 * `spec` is a patch: the server keeps every manifest field the object does not
 * mention (#7740). Callers should send only what the form actually edits rather
 * than reconstructing a full document, so an operator's `[[triggers]]`,
 * `tool_allowlist`, `[compaction]` and the rest survive the save.
 */
export function useUpdateAgentType() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, spec }: { name: string; spec: AgentTypeSpec }) =>
      updateAgentType(name, spec),
    onSuccess: (_data, { name }) => {
      qc.invalidateQueries({ queryKey: agentTypeKeys.detail(name) });
      qc.invalidateQueries({ queryKey: agentTypeKeys.lists() });
    },
  });
}

export function useDeleteAgentType() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => deleteAgentType(name),
    onSuccess: () => qc.invalidateQueries({ queryKey: agentTypeKeys.all }),
  });
}
