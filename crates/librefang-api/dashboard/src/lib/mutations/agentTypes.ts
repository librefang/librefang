import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createAgentType,
  updateAgentType,
  deleteAgentType,
  promoteAgentType,
  restoreTemplateVersion,
} from "../http/client";
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

// The `useSpawnEphemeral` hook lived here. Its only caller was the agent-types
// page's Quick Run modal, and #8384 replaced that control with one that
// instantiates the type instead, so the hook had no consumer left.
//
// The endpoint it wrapped (`POST /api/agents/spawn-ephemeral`) is untouched and
// still reachable — over HTTP directly, and in-turn through the `agent_spawn`
// tool with `ephemeral: true`. The CLI is not on that list: no subcommand
// forwards to it.
//
// `spawnEphemeral` itself stays in `api.ts`, which is the client for the
// daemon's published endpoints rather than UI glue. It has no caller today.
