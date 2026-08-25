import { queryOptions, useQuery } from "@tanstack/react-query";
import { listAgentTemplates, getAgentType } from "../http/client";
import { agentTypeKeys } from "./keys";
import { withOverrides, QueryOverrides } from "./options";

// Agent types are operator-authored documents on disk, not live state — nothing
// changes them behind the dashboard's back, so there is no poll interval here.
// The editor's mutations invalidate these keys directly.
const STALE_MS = 60_000;

export const agentTypeQueries = {
  list: () =>
    queryOptions({
      queryKey: agentTypeKeys.list(),
      queryFn: listAgentTemplates,
      staleTime: STALE_MS,
    }),
  detail: (name: string) =>
    queryOptions({
      queryKey: agentTypeKeys.detail(name),
      queryFn: () => getAgentType(name),
      staleTime: STALE_MS,
    }),
};

export function useAgentTypes(options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentTypeQueries.list(), options));
}

export function useAgentType(name: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentTypeQueries.detail(name), options));
}
