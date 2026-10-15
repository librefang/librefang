import { queryOptions, useQuery } from "@tanstack/react-query";

import { getAgentFile } from "../http/client";
import { ApiError } from "../http/errors";
import { agentFileKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

/**
 * An identity file changes only when something writes it — this dashboard's
 * editor, or a shell in the agent's workspace. There is no server-side writer
 * racing the reader, so the mutation below is the only thing that needs to
 * force a refetch and a 30 s window is enough for everything else.
 */
const STALE_MS = 30_000;

export const agentFileQueries = {
  detail: (agentId: string, filename: string) =>
    queryOptions({
      queryKey: agentFileKeys.detail(agentId, filename),
      queryFn: () => getAgentFile(agentId, filename),
      enabled: !!agentId && !!filename,
      staleTime: STALE_MS,
      // A 404 here is not a failure, it is the answer: the daemon reports
      // "not found" both for an absent file and for an unreadable one, and the
      // editor renders either as an empty form the operator can save into.
      // Retrying it only delays the form. Every other status keeps the
      // client-wide one-retry budget.
      retry: (failureCount, error) => {
        if (error instanceof ApiError && error.status === 404) return false;
        return failureCount < 2;
      },
    }),
};

export function useAgentFile(
  agentId: string,
  filename: string,
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(agentFileQueries.detail(agentId, filename), options));
}
