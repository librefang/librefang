import { queryOptions, useQuery } from "@tanstack/react-query";
import { listPromptsOverview } from "../http/client";
import { promptsKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

// The prompt repository page is a management surface, not a live dashboard:
// poll lazily so an idle tab does not hammer the cross-agent aggregation.
const STALE_MS = 30_000;
const REFRESH_MS = 60_000;

export const promptQueries = {
  // Fleet-wide repository overview: one summary row per non-hand agent.
  overview: () =>
    queryOptions({
      queryKey: promptsKeys.list(),
      queryFn: () => listPromptsOverview(),
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false,
    }),
};

export function usePromptsOverview(options: QueryOverrides = {}) {
  return useQuery(withOverrides(promptQueries.overview(), options));
}

// Per-agent version history reuses the existing agent-scoped read
// (`agentKeys.promptVersions` / `GET /agents/{id}/prompts/versions`) so the
// repository page and the per-agent prompt modal share one cache entry. Pull
// it through `usePromptVersions` from `./agents` directly at the call site.
export { usePromptVersions } from "./agents";
