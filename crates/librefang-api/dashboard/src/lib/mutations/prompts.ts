import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createPromptVersion,
  deletePromptVersion,
  activatePromptVersion,
  patchAgent,
} from "../http/client";
import type { PromptVersion } from "../../api";
import { agentKeys, promptsKeys } from "../queries/keys";

// Prompt repository mutations (#6160).
//
// These are the repository-page counterparts of the per-agent prompt
// mutations in `mutations/agents.ts`. They wrap the same backend calls but
// additionally invalidate the repository overview (`promptsKeys.list()`) and the
// per-agent repo-detail subtree (`promptsKeys.details()`) so both the fleet-wide
// counts/active-version and any single-agent repo-detail view refetch alongside
// the per-agent version list. `details()` has no subscriber yet, but invalidating
// the whole subtree keeps a future `promptsKeys.detail(agentId)` consumer
// forward-compatible (per the colocate-invalidation rule in CLAUDE.md). Use these
// from the prompt repository page; the agent-detail modal keeps its own hooks.

/**
 * Create a new prompt version for an agent (repository surface).
 *
 * Invalidates both the per-agent version list and the repository overview,
 * since the overview surfaces the agent's version count.
 */
export function useCreatePromptVersionForRepo() {
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
      qc.invalidateQueries({
        queryKey: agentKeys.promptVersions(variables.agentId),
      });
      qc.invalidateQueries({ queryKey: promptsKeys.list() });
      qc.invalidateQueries({ queryKey: promptsKeys.details() });
    },
  });
}

/**
 * Delete an inactive prompt version (repository surface).
 */
export function useDeletePromptVersionForRepo() {
  const qc = useQueryClient();
  return useMutation({
    // agentId is needed for targeted invalidation but not for the API call.
    mutationFn: ({
      versionId,
      agentId: _agentId,
    }: {
      versionId: string;
      agentId: string;
    }) => deletePromptVersion(versionId),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({
        queryKey: agentKeys.promptVersions(variables.agentId),
      });
      qc.invalidateQueries({ queryKey: promptsKeys.list() });
      qc.invalidateQueries({ queryKey: promptsKeys.details() });
    },
  });
}

/**
 * Bind a managed prompt version to an agent.
 *
 * This is the load-bearing repository action. Activating a version on its own
 * only flips the store's `is_active` flag — it does NOT change the prompt the
 * agent actually sends to the LLM, which is read from
 * `manifest.model.system_prompt`. So binding does both, in order:
 *
 *   1. `PATCH /api/agents/{id}` with `system_prompt` = the version's text,
 *      hot-swapping the live prompt used on the next message and persisting
 *      it to the agent manifest (agent.toml + DB).
 *   2. `POST /api/prompts/versions/{id}/activate`, flipping the store flag so
 *      the repository UI shows this version as the active one.
 *
 * Invalidates the per-agent version list, the agent detail, the agent list
 * (the model/prompt badge), the repository overview, and the per-agent
 * repo-detail subtree.
 */
export function useBindPromptVersionToAgent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async ({
      agentId,
      version,
    }: {
      agentId: string;
      version: PromptVersion;
    }) => {
      // 1. Hot-swap the live system prompt onto the manifest.
      await patchAgent(agentId, { system_prompt: version.system_prompt });
      // 2. Flip the store's active flag to match.
      return activatePromptVersion(version.id, agentId);
    },
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({
        queryKey: agentKeys.promptVersions(variables.agentId),
      });
      qc.invalidateQueries({ queryKey: agentKeys.detail(variables.agentId) });
      qc.invalidateQueries({ queryKey: agentKeys.lists() });
      qc.invalidateQueries({ queryKey: promptsKeys.list() });
      qc.invalidateQueries({ queryKey: promptsKeys.details() });
    },
  });
}
