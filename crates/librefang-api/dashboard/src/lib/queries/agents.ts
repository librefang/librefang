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
  getAgentSessionContext,
  listTools,
  getAgentTools,
  getAgentSkills,
  getAgentMcpServers,
} from "../http/client";
import { agentKeys, toolKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 30_000;
const REFRESH_MS = 30_000;
const LIVE_STALE_MS = 10_000;
const STATS_STALE_MS = 15_000;
const LIVE_REFRESH_MS = 15_000;

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
      staleTime: STALE_MS,
    }),
  sessions: (agentId: string) =>
    queryOptions({
      queryKey: agentKeys.sessions(agentId),
      queryFn: () => listAgentSessions(agentId),
      enabled: !!agentId,
      staleTime: LIVE_STALE_MS,
    }),
  stats: (agentId: string) =>
    queryOptions({
      queryKey: agentKeys.stats(agentId),
      queryFn: () => getAgentStats(agentId),
      enabled: !!agentId,
      staleTime: STATS_STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  events: (agentId: string, limit = 30) =>
    queryOptions({
      queryKey: agentKeys.events(agentId, limit),
      queryFn: () => listAgentEvents(agentId, limit),
      enabled: !!agentId,
      staleTime: LIVE_STALE_MS,
      refetchInterval: LIVE_REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  templates: () =>
    queryOptions({
      queryKey: agentKeys.templates(),
      queryFn: listAgentTemplates,
      staleTime: STALE_MS,
    }),
  promptVersions: (agentId: string) =>
    queryOptions({
      queryKey: agentKeys.promptVersions(agentId),
      queryFn: () => listPromptVersions(agentId),
      enabled: !!agentId,
      staleTime: STALE_MS,
    }),
  experiments: (agentId: string) =>
    queryOptions({
      queryKey: agentKeys.experiments(agentId),
      queryFn: () => listExperiments(agentId),
      enabled: !!agentId,
      staleTime: STALE_MS,
    }),
  experimentMetrics: (experimentId: string) =>
    queryOptions({
      queryKey: agentKeys.experimentMetrics(experimentId),
      queryFn: () => getExperimentMetrics(experimentId),
      enabled: !!experimentId,
      staleTime: STALE_MS,
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
  // Context-window usage snapshot — a cheap polled read that backs the chat
  // header fill indicator. Refetched on a modest interval so it stays roughly
  // live without spamming; paused in the background like the other agent
  // polls (#3393). Disabled until both agent and session are known.
  sessionContext: (agentId: string, sessionId?: string | null) =>
    queryOptions({
      queryKey: agentKeys.sessionContext(agentId, sessionId ?? null),
      queryFn: () => getAgentSessionContext(agentId, sessionId ?? null),
      enabled: !!agentId && !!sessionId,
      staleTime: LIVE_STALE_MS,
      refetchInterval: LIVE_REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  agentTools: (agentId: string) =>
    queryOptions({
      queryKey: agentKeys.tools(agentId),
      queryFn: () => getAgentTools(agentId),
      enabled: !!agentId,
      staleTime: STALE_MS,
    }),
  agentSkills: (agentId: string) =>
    queryOptions({
      queryKey: agentKeys.skills(agentId),
      queryFn: () => getAgentSkills(agentId),
      enabled: !!agentId,
      staleTime: STALE_MS,
    }),
  agentMcpServers: (agentId: string) =>
    queryOptions({
      queryKey: agentKeys.mcpServers(agentId),
      queryFn: () => getAgentMcpServers(agentId),
      enabled: !!agentId,
    }),
  toolsList: () =>
    queryOptions({
      queryKey: toolKeys.list(),
      queryFn: listTools,
      staleTime: STALE_MS,
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

export function useTools(options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentQueries.toolsList(), options));
}

export function useAgentTools(agentId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentQueries.agentTools(agentId), options));
}

export function useAgentSkills(agentId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentQueries.agentSkills(agentId), options));
}

export function useAgentMcpServers(agentId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentQueries.agentMcpServers(agentId), options));
}
