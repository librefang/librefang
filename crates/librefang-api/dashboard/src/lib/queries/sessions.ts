import { queryOptions, useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { listSessions, getSessionDetails } from "../http/client";
import { sessionKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 30_000;

export const sessionQueries = {
  list: () =>
    queryOptions({
      queryKey: sessionKeys.lists(),
      queryFn: listSessions,
      staleTime: STALE_MS,
      refetchInterval: STALE_MS,
    }),
  detail: (sessionId: string) =>
    queryOptions({
      queryKey: sessionKeys.detail(sessionId),
      queryFn: () => getSessionDetails(sessionId),
      enabled: !!sessionId,
    }),
};

export function useSessions(options: QueryOverrides = {}) {
  return useQuery(withOverrides(sessionQueries.list(), options));
}

export function useSessionDetails(sessionId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(sessionQueries.detail(sessionId), options));
}

// ---------------------------------------------------------------------------
// useSessionStream — multi-attach SSE consumer for in-flight session events.
//
// Subscribes to `GET /api/agents/{agentId}/sessions/{sessionId}/stream` so
// that any client (extra browser tab, desktop, CLI) can co-watch the events
// of a session that another client started. The originating client still
// sends its turn over `POST /message/stream` (single-consumer mpsc); this
// hook is for read-only observers.
//
// The hook intentionally does NOT go through TanStack Query's cache: an SSE
// connection is a long-lived stream of imperative events, not cacheable
// snapshot data. This is the same exception called out in
// `dashboard/AGENTS.md` ("Streaming / SSE … may call native browser APIs
// directly").
//
// Behaviour:
// - opens an EventSource when both ids are truthy
// - closes cleanly on unmount, agent change, or session change
// - surfaces parsed events as an in-memory append-only list capped at
//   MAX_EVENTS to bound worst-case memory
// - swallows the "404 because the route isn't deployed yet / the session
//   has no active turn" case silently — `lastError` stays null, no toast.
//   Native EventSource cannot read HTTP status codes, so the heuristic is:
//   an error event that fires before any payload arrived AND closes the
//   connection is treated as a no-op. Mid-stream errors after data has
//   flowed are surfaced via `lastError`.
// ---------------------------------------------------------------------------

const MAX_EVENTS = 500;

/** Discriminated union of SSE event payloads emitted by the backend stream. */
export type SessionStreamEvent =
  | { type: "chunk"; data: { content?: string; [k: string]: unknown }; receivedAt: number }
  | { type: "tool_use"; data: { tool?: string; input?: unknown; id?: string; [k: string]: unknown }; receivedAt: number }
  | { type: "tool_result"; data: { tool?: string; result?: unknown; is_error?: boolean; [k: string]: unknown }; receivedAt: number }
  | { type: "phase"; data: { phase?: string; [k: string]: unknown }; receivedAt: number }
  | { type: "done"; data: Record<string, unknown>; receivedAt: number }
  | { type: "owner_notice"; data: { message?: string; [k: string]: unknown }; receivedAt: number };

export type UseSessionStreamResult = {
  /** Append-only list of parsed events, capped at MAX_EVENTS (oldest dropped). */
  events: SessionStreamEvent[];
  /** True while the EventSource is open and accepting events. */
  isAttached: boolean;
  /** Last surfaced error, or null. 404-on-open is intentionally swallowed. */
  lastError: string | null;
};

export type UseSessionStreamOptions = {
  /**
   * Override the default endpoint builder. Tests inject a stub URL; production
   * callers should not need this.
   */
  buildUrl?: (agentId: string, sessionId: string) => string;
  /**
   * Optional EventSource constructor — defaults to globalThis.EventSource.
   * Tests can pass a fake. If the environment has no EventSource (older
   * browsers, jsdom without polyfill) the hook returns a no-op state.
   */
  eventSourceCtor?: typeof EventSource;
};

const SSE_TYPES = ["chunk", "tool_use", "tool_result", "phase", "done", "owner_notice"] as const;
type SseType = (typeof SSE_TYPES)[number];

function defaultBuildUrl(agentId: string, sessionId: string): string {
  return `/api/agents/${encodeURIComponent(agentId)}/sessions/${encodeURIComponent(sessionId)}/stream`;
}

export function useSessionStream(
  agentId: string | null | undefined,
  sessionId: string | null | undefined,
  options: UseSessionStreamOptions = {},
): UseSessionStreamResult {
  const [events, setEvents] = useState<SessionStreamEvent[]>([]);
  const [isAttached, setIsAttached] = useState(false);
  const [lastError, setLastError] = useState<string | null>(null);
  // Track whether any payload arrived before an error fires. If not, an
  // error+close is almost certainly a 404 / not-deployed / no-active-turn —
  // swallow it.
  const receivedAnyRef = useRef(false);

  useEffect(() => {
    // Reset state when ids change so stale events don't leak across sessions.
    setEvents([]);
    setIsAttached(false);
    setLastError(null);
    receivedAnyRef.current = false;

    if (!agentId || !sessionId) return;
    const Ctor = options.eventSourceCtor ?? (typeof EventSource !== "undefined" ? EventSource : undefined);
    if (!Ctor) return;

    const url = (options.buildUrl ?? defaultBuildUrl)(agentId, sessionId);

    let es: EventSource;
    try {
      es = new Ctor(url, { withCredentials: true });
    } catch {
      // Constructor itself threw — environment doesn't support EventSource
      // for this URL (e.g. invalid scheme in tests). Stay detached, silent.
      return;
    }

    const handlePayload = (type: SseType, raw: string) => {
      receivedAnyRef.current = true;
      let parsed: Record<string, unknown> = {};
      try {
        parsed = raw ? (JSON.parse(raw) as Record<string, unknown>) : {};
      } catch {
        // Non-JSON payload — preserve as raw content so callers can still
        // render it.
        parsed = { content: raw };
      }
      const evt = {
        type,
        data: parsed,
        receivedAt: Date.now(),
      } as SessionStreamEvent;
      setEvents((prev) => {
        const next = prev.length >= MAX_EVENTS ? prev.slice(prev.length - MAX_EVENTS + 1) : prev;
        return [...next, evt];
      });
      // Auto-detach on terminal "done" so consumers can react without polling.
      if (type === "done") {
        setIsAttached(false);
      }
    };

    const listeners = SSE_TYPES.map((type) => {
      const fn = (ev: MessageEvent) => handlePayload(type, typeof ev.data === "string" ? ev.data : "");
      es.addEventListener(type, fn as EventListener);
      return { type, fn };
    });

    const onOpen = () => {
      setIsAttached(true);
      setLastError(null);
    };
    const onError = () => {
      // EventSource doesn't expose HTTP status. If we never received any
      // payload AND the connection closed, assume the route isn't there
      // (deployed in #3078 but not yet on this client's server) or the
      // session has no active turn — both of which are valid no-ops.
      const closed = es.readyState === 2; // EventSource.CLOSED
      if (!receivedAnyRef.current && closed) {
        // Silent no-op — don't surface to the UI.
        setIsAttached(false);
        return;
      }
      if (closed) {
        // Mid-stream drop after data flowed — worth telling the caller, but
        // don't toast: a co-watcher dropping isn't a destructive error.
        setLastError("session stream disconnected");
        setIsAttached(false);
      }
      // Transient errors (readyState === CONNECTING) are EventSource's own
      // automatic reconnect; ignore them.
    };

    es.addEventListener("open", onOpen);
    es.addEventListener("error", onError);

    return () => {
      es.removeEventListener("open", onOpen);
      es.removeEventListener("error", onError);
      for (const { type, fn } of listeners) {
        es.removeEventListener(type, fn as EventListener);
      }
      es.close();
      setIsAttached(false);
    };
    // buildUrl/eventSourceCtor are stable in production; tests pass them
    // once at mount. Excluding them keeps the connection from churning.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentId, sessionId]);

  return { events, isAttached, lastError };
}
