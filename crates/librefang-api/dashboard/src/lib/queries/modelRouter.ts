import { queryOptions, useQuery } from "@tanstack/react-query";
import { listModelRouterProfiles, getAgentModelRouting } from "../http/client";
import { modelRouterKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

// The profile catalog changes only when an operator edits
// `~/.librefang/model_profiles.toml`, so it is cached generously and not
// polled — a manual refetch or a routing mutation is what brings it back.
const CATALOG_STALE_MS = 60_000;
const AGENT_STALE_MS = 15_000;

export const modelRouterQueries = {
  profiles: () =>
    queryOptions({
      queryKey: modelRouterKeys.profiles(),
      queryFn: () => listModelRouterProfiles(),
      staleTime: CATALOG_STALE_MS,
    }),
  agent: (agentId: string) =>
    queryOptions({
      queryKey: modelRouterKeys.agent(agentId),
      queryFn: () => getAgentModelRouting(agentId),
      enabled: !!agentId,
      staleTime: AGENT_STALE_MS,
    }),
};

/** The resolved model-router profile catalog plus the kernel-wide on/off flag. */
export function useModelRouterProfiles(options: QueryOverrides = {}) {
  return useQuery(withOverrides(modelRouterQueries.profiles(), options));
}

/** One agent's model selection mode and router override. */
export function useAgentModelRouting(agentId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(modelRouterQueries.agent(agentId), options));
}
