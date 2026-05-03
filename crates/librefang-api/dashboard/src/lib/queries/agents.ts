import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listAgents,
  getAgentDetail,
  getAgentStats,
  listAgentEvents,
  listAgentSessions,
  listAgentTemplates,
  listPromptVersions,
  listExperiments,
  getExperimentMetrics,
  loadAgentSession,
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
      refetchIntervalInBackground: false, // #3393
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
      refetchIntervalInBackground: false, // #3393
    }),
  events: (agentId: string, limit = 30) =>
    queryOptions({
      queryKey: agentKeys.events(agentId, limit),
      queryFn: () => listAgentEvents(agentId, limit),
      enabled: !!agentId,
      staleTime: 10_000,
      refetchInterval: 15_000,
      refetchIntervalInBackground: false, // #3393
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
  // Snapshot of the (agent, session) chat history. ChatPage hydrates from
  // this on first navigation and on session switch; subsequent turns are
  // applied locally rather than refetched. Cache survives back/forward
  // navigation so returning to a previously viewed agent is instant — the
  // long staleTime keeps that cached payload from being refetched on focus.
  session: (agentId: string, sessionId?: string | null) =>
    queryOptions({
      queryKey: agentKeys.session(agentId, sessionId ?? null),
      queryFn: () => loadAgentSession(agentId, sessionId ?? null),
      enabled: !!agentId,
      staleTime: 5 * 60_000,
      refetchOnWindowFocus: false,
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

export function useAgentEvents(
  agentId: string,
  limit = 30,
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(agentQueries.events(agentId, limit), options));
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
