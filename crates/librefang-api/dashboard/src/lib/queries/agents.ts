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
      staleTime: 30_000,
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

type UseAgentOptions = {
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?: number | false;
};

export function useAgents(
  opts: { includeHands?: boolean } = {},
  options: UseAgentOptions = {},
) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...agentQueries.list(opts),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useAgentDetail(agentId: string, options: UseAgentOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...agentQueries.detail(agentId),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useAgentSessions(agentId: string, options: UseAgentOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...agentQueries.sessions(agentId),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useAgentTemplates(options: UseAgentOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...agentQueries.templates(),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function usePromptVersions(agentId: string, options: UseAgentOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...agentQueries.promptVersions(agentId),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useExperiments(agentId: string, options: UseAgentOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...agentQueries.experiments(agentId),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useExperimentMetrics(experimentId: string, options: UseAgentOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...agentQueries.experimentMetrics(experimentId),
    enabled,
    staleTime,
    refetchInterval,
  });
}
