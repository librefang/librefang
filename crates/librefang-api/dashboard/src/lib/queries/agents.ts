import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listAgents,
  getAgentDetail,
  getAgentStats,
  listAgentSessions,
  listAgentTemplates,
  listPromptVersions,
  listExperiments,
  getExperimentMetrics,
} from "../http/client";
import { agentKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 30_000;
const REFRESH_MS = 30_000;

export const agentQueries = {
  list: (opts: { includeHands?: boolean } = {}) =>
    queryOptions({
      queryKey: agentKeys.list(opts),
      queryFn: () => listAgents(opts),
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
  detail: (agentId: string) =>
    queryOptions({
      queryKey: agentKeys.detail(agentId),
      queryFn: () => getAgentDetail(agentId),
      enabled: !!agentId,
      staleTime: 30_000,
    }),
  sessions: (agentId: string) =>
    queryOptions({
      queryKey: agentKeys.sessions(agentId),
      queryFn: () => listAgentSessions(agentId),
      enabled: !!agentId,
      staleTime: 10_000,
    }),
  stats: (agentId: string) =>
    queryOptions({
      queryKey: agentKeys.stats(agentId),
      queryFn: () => getAgentStats(agentId),
      enabled: !!agentId,
      staleTime: 15_000,
      refetchInterval: 30_000,
    }),
  templates: () =>
    queryOptions({
      queryKey: agentKeys.templates(),
      queryFn: listAgentTemplates,
    }),
  promptVersions: (agentId: string) =>
    queryOptions({
      queryKey: agentKeys.promptVersions(agentId),
      queryFn: () => listPromptVersions(agentId),
      enabled: !!agentId,
    }),
  experiments: (agentId: string) =>
    queryOptions({
      queryKey: agentKeys.experiments(agentId),
      queryFn: () => listExperiments(agentId),
      enabled: !!agentId,
    }),
  experimentMetrics: (experimentId: string) =>
    queryOptions({
      queryKey: agentKeys.experimentMetrics(experimentId),
      queryFn: () => getExperimentMetrics(experimentId),
      enabled: !!experimentId,
    }),
};

export function useAgents(
  opts: { includeHands?: boolean } = {},
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(agentQueries.list(opts), options));
}

export function useAgentDetail(agentId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentQueries.detail(agentId), options));
}

export function useAgentSessions(agentId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentQueries.sessions(agentId), options));
}

export function useAgentStats(agentId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentQueries.stats(agentId), options));
}

export function useAgentTemplates(options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentQueries.templates(), options));
}

export function usePromptVersions(agentId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentQueries.promptVersions(agentId), options));
}

export function useExperiments(agentId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentQueries.experiments(agentId), options));
}

export function useExperimentMetrics(experimentId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentQueries.experimentMetrics(experimentId), options));
}
