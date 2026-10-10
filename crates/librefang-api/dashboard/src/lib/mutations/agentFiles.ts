import { useMutation, useQueryClient } from "@tanstack/react-query";

import { setAgentFile } from "../http/client";
import { agentFileKeys } from "../queries/keys";

/**
 * Writes one workspace identity file.
 *
 * `PUT /api/agents/{id}/files/{filename}` replaces the file's bytes, so the
 * cached read of *that* file is stale — and so is the agent's file listing,
 * whose `exists` and `size_bytes` columns are projections of what just changed.
 * Nothing under `agentKeys` is touched: this endpoint writes a workspace file,
 * not the agent manifest, and invalidating the agent detail here would refetch
 * the whole detail subtree on every save.
 *
 * `content` is the complete file, not a patch — the caller is responsible for
 * having preserved everything the editor does not own (see
 * `lib/identityFrontMatter.ts`).
 */
export function useSetAgentFile(agentId: string, filename: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (content: string) => setAgentFile(agentId, filename, content),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentFileKeys.detail(agentId, filename) });
      qc.invalidateQueries({ queryKey: agentFileKeys.lists() });
    },
  });
}
