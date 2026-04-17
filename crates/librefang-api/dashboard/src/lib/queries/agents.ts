import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listAgents,
  getAgentDetail,
  listAgentSessions,
  listAgentTemplates,
  listPromptVersions,
  listExperiments,
  getExperimentMetrics,
} from "../http/client";
import { agentKeys } from "./keys";

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
    }),
  sessions: (agentId: string) =>
    queryOptions({
      queryKey: agentKeys.sessions(agentId),
      queryFn: () => listAgentSessions(agentId),
      enabled: !!agentId,
      staleTime: 10_000,
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

export function useAgents(opts: { includeHands?: boolean } = {}) {
  return useQuery(agentQueries.list(opts));
}

export function useAgentDetail(agentId: string) {
  return useQuery(agentQueries.detail(agentId));
}

export function useAgentSessions(agentId: string) {
  return useQuery(agentQueries.sessions(agentId));
}

export function useAgentTemplates(options: { enabled?: boolean } = {}) {
  return useQuery({ ...agentQueries.templates(), enabled: options.enabled });
}

export function usePromptVersions(agentId: string) {
  return useQuery(agentQueries.promptVersions(agentId));
}

export function useExperiments(agentId: string) {
  return useQuery(agentQueries.experiments(agentId));
}

export function useExperimentMetrics(experimentId: string) {
  return useQuery(agentQueries.experimentMetrics(experimentId));
}
