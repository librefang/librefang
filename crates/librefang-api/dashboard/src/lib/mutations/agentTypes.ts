import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createAgentType,
  updateAgentType,
  deleteAgentType,
  spawnEphemeral,
} from "../http/client";
import type { AgentTypeSpec, SpawnEphemeralRequest } from "../../api";
import { agentTypeKeys, budgetKeys, usageKeys } from "../queries/keys";

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

/**
 * Run one ephemeral worker and return what it produced (#6699).
 *
 * The worker leaves nothing behind — no registry entry, no session, no
 * workspace — so there is no agent list to refresh afterwards. What it does
 * leave is spend on the *parent's* ledger, which is why usage and budget are
 * the two domains invalidated here: a Quick Run that silently cost money and
 * left the budget widget showing the pre-run figure is the exact surprise this
 * feature must not produce.
 */
export function useSpawnEphemeral() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: SpawnEphemeralRequest) => spawnEphemeral(body),
    onSettled: () => {
      qc.invalidateQueries({ queryKey: usageKeys.all });
      qc.invalidateQueries({ queryKey: budgetKeys.all });
    },
  });
}
