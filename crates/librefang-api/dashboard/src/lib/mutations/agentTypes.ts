import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createAgentTypeFromToml,
  deleteAgentType,
  promoteAgentType,
  restoreTemplateVersion,
  spawnEphemeral,
  putAgentTemplateToml,
} from "../http/client";
import type { AgentTypeDetail, SpawnEphemeralRequest } from "../../api";
import { agentTypeKeys, budgetKeys, usageKeys } from "../queries/keys";

/**
 * Report a save's `unknown_keys` back to the caller (#8028).
 *
 * The server drops any top-level key the submitted TOML carried that
 * `AgentManifest` doesn't recognize, and says so in the response body — but
 * a mutation's `onSuccess` runs before the caller sees the result, so this
 * is the one place shared by both write paths that can turn it into
 * something the operator actually sees instead of a fact only the network
 * tab knows.
 */
export const unknownKeysWarning = (detail: AgentTypeDetail): string | null =>
  detail.unknown_keys && detail.unknown_keys.length > 0
    ? detail.unknown_keys.join(", ")
    : null;

/** Create a new agent type from a complete manifest, in one atomic write (#8028). */
export function useCreateAgentTypeFromToml() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, toml }: { name: string; toml: string }) =>
      createAgentTypeFromToml(name, toml),
    onSuccess: () => qc.invalidateQueries({ queryKey: agentTypeKeys.all }),
  });
}

export function useUpdateAgentTypeToml() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, toml }: { name: string; toml: string }) =>
      putAgentTemplateToml(name, toml),
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
 * Promote an agent type to the public registry as a GitHub PR.
 *
 * Read-only with respect to local state — it forks the registry repo,
 * pushes the sanitized manifest, and opens a PR. No local invalidation
 * needed since the agent type itself is unchanged.
 */
export function usePromoteAgentType() {
  return useMutation({
    mutationFn: (name: string) => promoteAgentType(name),
  });
}

/**
 * Restore a template to a prior version from its history.
 *
 * Invalidates the detail and history caches so both the editor and the
 * history tab reflect the restored content immediately.
 */
export function useRestoreTemplateVersion() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, versionId }: { name: string; versionId: number }) =>
      restoreTemplateVersion(name, versionId),
    onSuccess: (_data, { name }) => {
      qc.invalidateQueries({ queryKey: agentTypeKeys.detail(name) });
      qc.invalidateQueries({ queryKey: agentTypeKeys.history(name) });
      qc.invalidateQueries({ queryKey: agentTypeKeys.lists() });
    },
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
