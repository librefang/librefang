import { useEffect, useState } from "react";
import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  agentAvatarPath,
  fetchAuthenticatedImage,
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
  getAgentChannels,
} from "../http/client";
import { agentKeys, toolKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 30_000;
const REFRESH_MS = 30_000;
const LIVE_STALE_MS = 10_000;
const STATS_STALE_MS = 15_000;
const LIVE_REFRESH_MS = 15_000;
const AVATAR_STALE_MS = 300_000;

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
  agentChannels: (agentId: string) =>
    queryOptions({
      queryKey: agentKeys.channels(agentId),
      queryFn: () => getAgentChannels(agentId),
      enabled: !!agentId,
    }),
  toolsList: () =>
    queryOptions({
      queryKey: toolKeys.list(),
      queryFn: listTools,
      staleTime: STALE_MS,
    }),
  // The avatar image as a Blob (#8339). `GET /api/agents/{id}/avatar` is
  // authenticated, so an `<img src>` pointed at it sends no bearer token and
  // gets a 401; the bytes have to be fetched and handed to the tag as an
  // object URL instead.
  //
  // `enabled` is the caller's "this agent has one" — asking otherwise buys a
  // guaranteed 404 per agent per render. The long `staleTime` leans on the
  // route's `ETag` + `no-cache`: a revalidation that finds nothing changed is
  // a bodiless 304, and a re-upload is picked up by the mutations invalidating
  // this key rather than by polling for it.
  avatar: (agentId: string, enabled: boolean) =>
    queryOptions({
      queryKey: agentKeys.avatar(agentId),
      queryFn: () => fetchAuthenticatedImage(agentAvatarPath(agentId)),
      enabled: !!agentId && enabled,
      staleTime: AVATAR_STALE_MS,
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

export function useAgentChannels(agentId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(agentQueries.agentChannels(agentId), options));
}

/**
 * An agent's avatar as an object URL, ready for an `<img src>` (#8339).
 *
 * Two things are being kept apart here. The query caches the *Blob*, which is
 * shared and lives as long as the cache entry does; this hook owns the *object
 * URL*, which is a document-scoped handle that leaks until revoked. So the URL
 * is minted in an effect keyed on the Blob and revoked in that effect's
 * cleanup — on unmount, and on every switch to another agent, which is the
 * case a drawer that stays mounted while the selection changes would otherwise
 * leak on.
 *
 * `hasAvatar` is the caller's answer to "is `identity.avatar_url` set", and it
 * gates the request: an agent without one would otherwise cost a 404 on every
 * render of the row that shows its initials.
 *
 * Returns `undefined` while loading and when there is nothing to show, which is
 * exactly what `Avatar`'s `src` wants — it falls back to the initials on its
 * own, so there is no separate loading state to thread through the UI.
 */
export function useAgentAvatarUrl(agentId: string, hasAvatar: boolean): string | undefined {
  const { data: blob } = useQuery(agentQueries.avatar(agentId, hasAvatar));
  const [objectUrl, setObjectUrl] = useState<string | undefined>(undefined);

  useEffect(() => {
    if (!blob) {
      setObjectUrl(undefined);
      return;
    }
    const url = URL.createObjectURL(blob);
    setObjectUrl(url);
    return () => {
      URL.revokeObjectURL(url);
      // Without this the next paint still points an `<img>` at a URL that has
      // just been revoked, which renders as a broken image rather than as the
      // initials the fallback is there to give.
      setObjectUrl(undefined);
    };
  }, [blob]);

  return objectUrl;
}
