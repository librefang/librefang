import { queryOptions, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import {
  listSessions,
  getSessionDetails,
  type ListSessionsResult,
} from "../http/client";
import { buildAuthenticatedWebSocket } from "../../api";
import { sessionKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 30_000;

export const sessionQueries = {
  list: () =>
    queryOptions({
      queryKey: sessionKeys.lists(),
      queryFn: listSessions,
      select: (data: ListSessionsResult) => data.items,
      staleTime: STALE_MS,
      refetchInterval: STALE_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  detail: (sessionId: string) =>
    queryOptions({
      queryKey: sessionKeys.detail(sessionId),
      queryFn: () => getSessionDetails(sessionId),
      enabled: !!sessionId,
    }),
};

export function useSessions(options: QueryOverrides = {}) {
  const qc = useQueryClient();
  const query = useQuery(withOverrides(sessionQueries.list(), options));
  const raw = qc.getQueryData<ListSessionsResult>(sessionKeys.lists());
  return {
    ...query,
    truncated: raw?.truncated ?? false,
  };
}

export function useSessionDetails(sessionId: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(sessionQueries.detail(sessionId), options));
}

// ---------------------------------------------------------------------------
// useSessionStream — multi-attach consumer for in-flight session events.
//
// Subscribes to `GET /api/agents/{agentId}/sessions/{sessionId}/stream` so
// that any client (extra browser tab, desktop, CLI) can co-watch the events
// of a session that another client started. The originating client still
// sends its turn over `POST /message/stream` (single-consumer mpsc); this
// hook is for read-only observers.
//
// Transport: WebSocket (audit `docs/issues/session-sse-withcredentials.md`).
// Previously the hook used `EventSource` with `withCredentials: true`,
// which only carries cookies — but the dashboard's auth is a
// `Authorization: Bearer <token>` header, and `EventSource` does not
// support custom headers. The result was an effectively unauthenticated
// upstream request that only survived because the route returns mock
// data today; once #3078 gates the route on auth, every attach would
// 401 silently. The dashboard's existing authenticated WebSocket pattern
// (#3963, `buildAuthenticatedWebSocket`) carries the bearer via the
// `Sec-WebSocket-Protocol` sub-protocol — the token never reaches URLs,
// proxy access logs, browser history, or `Referer` headers.
//
// SERVER-SIDE NOTE: the route handler at
// `crates/librefang-api/src/routes/agents.rs::attach_session_stream` is
// currently an SSE responder. A coordinated server-side change is
// required to accept the WebSocket upgrade; until that lands the
// connection fails at handshake (and the close code surfaces via
// `lastError`). The audit doc tracks both halves.
//
// The hook intentionally does NOT go through TanStack Query's cache: a
// streaming connection is a long-lived stream of imperative events, not
// cacheable snapshot data. This is the same exception called out in
// `dashboard/AGENTS.md` ("Streaming / SSE … may call native browser APIs
// directly").
//
// Behaviour:
// - opens a WebSocket when both ids are truthy
// - closes cleanly on unmount, agent change, or session change
// - surfaces parsed events as an in-memory append-only list capped at
//   MAX_EVENTS to bound worst-case memory
// - reconnects with exponential backoff (capped at WS_MAX_RETRIES) for
//   transient mid-stream disconnects; auth-error close codes
//   (4401 / 4403) stop reconnect and surface via `lastError`
// - swallows the "route not deployed / session has no active turn" case
//   silently when the first attempt closes before any payload arrived
//   AND the close code is non-auth — `lastError` stays null, matching
//   the prior SSE behaviour the audit explicitly preserves
// ---------------------------------------------------------------------------

const MAX_EVENTS = 500;
const WS_MAX_RETRIES = 5;
const WS_AUTH_ERROR_CODES = new Set([4401, 4403]);
// Normal closure (1000) is a clean shutdown, not a fault.
const WS_NORMAL_CLOSE = 1000;

/** Discriminated union of session event payloads emitted by the backend stream. */
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
  /** True while the WebSocket is open and accepting events. */
  isAttached: boolean;
  /** Last surfaced error, or null. Open-then-close before any data flowed
   *  with a non-auth close code is intentionally swallowed. */
  lastError: string | null;
};

export type UseSessionStreamOptions = {
  /**
   * Override the default endpoint path builder. Tests inject a stub path;
   * production callers should not need this. This returns a PATH (e.g.
   * `/api/...`); the hook prepends scheme + host through
   * `buildAuthenticatedWebSocket`.
   */
  buildPath?: (agentId: string, sessionId: string) => string;
  /**
   * Optional WebSocket constructor — defaults to `globalThis.WebSocket`.
   * Tests pass a fake. If the environment has no WebSocket the hook
   * returns a no-op state.
   */
  webSocketCtor?: typeof WebSocket;
  /**
   * Optional override for the URL builder. Tests use this to inject a
   * deterministic `ws://test/...` URL without depending on
   * `window.location` or stored auth state. Production callers should
   * not need this.
   */
  buildWebSocket?: (path: string) => { url: string; protocols: string[] };
};

const STREAM_TYPES = ["chunk", "tool_use", "tool_result", "phase", "done", "owner_notice"] as const;
type StreamType = (typeof STREAM_TYPES)[number];

function defaultBuildPath(agentId: string, sessionId: string): string {
  return `/api/agents/${encodeURIComponent(agentId)}/sessions/${encodeURIComponent(sessionId)}/stream`;
}

/** Shape of a single parsed envelope from the server. The server emits
 *  one JSON message per event with `{ type, ...data }`; we keep the
 *  parsed payload (minus `type`) under `data` so consumers see the same
 *  shape they saw under SSE. */
type EventEnvelope = { type?: string } & Record<string, unknown>;

export function useSessionStream(
  agentId: string | null | undefined,
  sessionId: string | null | undefined,
  options: UseSessionStreamOptions = {},
): UseSessionStreamResult {
  const [events, setEvents] = useState<SessionStreamEvent[]>([]);
  const [isAttached, setIsAttached] = useState(false);
  const [lastError, setLastError] = useState<string | null>(null);
  // Track whether any payload arrived before close. If not, a close with
  // a non-auth code on the first attempt is almost certainly route-not-
  // deployed / no-active-turn — swallow it (parity with prior SSE).
  const receivedAnyRef = useRef(false);

  useEffect(() => {
    // Reset state when ids change so stale events don't leak across sessions.
    setEvents([]);
    setIsAttached(false);
    setLastError(null);
    receivedAnyRef.current = false;

    if (!agentId || !sessionId) return;
    const Ctor =
      options.webSocketCtor ??
      (typeof WebSocket !== "undefined" ? WebSocket : undefined);
    if (!Ctor) return;

    const path = (options.buildPath ?? defaultBuildPath)(agentId, sessionId);
    const { url, protocols } = (options.buildWebSocket ?? buildAuthenticatedWebSocket)(path);

    let cancelled = false;
    let ws: WebSocket | null = null;
    let retries = 0;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    const appendEvent = (type: StreamType, parsed: Record<string, unknown>) => {
      const evt = {
        type,
        data: parsed,
        receivedAt: Date.now(),
      } as SessionStreamEvent;
      setEvents((prev) => {
        const next =
          prev.length >= MAX_EVENTS
            ? prev.slice(prev.length - MAX_EVENTS + 1)
            : prev;
        return [...next, evt];
      });
      if (type === "done") {
        setIsAttached(false);
      }
    };

    const handleMessage = (raw: string) => {
      let envelope: EventEnvelope = {};
      try {
        envelope = raw ? (JSON.parse(raw) as EventEnvelope) : {};
      } catch {
        // Non-JSON frame — preserve under `content` so callers can still
        // render it. Treat as a `chunk` (the most common payload type
        // and the one that the previous SSE fallback assumed).
        receivedAnyRef.current = true;
        appendEvent("chunk", { content: raw });
        return;
      }
      const type = envelope.type;
      if (typeof type !== "string" || !STREAM_TYPES.includes(type as StreamType)) {
        // Unknown / missing type — surface as a chunk so we don't lose
        // the payload but don't poison the typed consumers.
        receivedAnyRef.current = true;
        appendEvent("chunk", { ...envelope });
        return;
      }
      receivedAnyRef.current = true;
      // Strip `type` from the data envelope so consumers see only the
      // payload fields (matches the previous SSE shape, which delivered
      // `data` separately from the event name).
      const { type: _drop, ...payload } = envelope;
      void _drop;
      appendEvent(type as StreamType, payload);
    };

    function connect() {
      if (cancelled) return;
      let socket: WebSocket;
      try {
        socket = new Ctor!(url, protocols.length > 0 ? protocols : undefined);
      } catch {
        // Constructor itself threw — environment doesn't support
        // WebSocket for this URL (e.g. invalid scheme in tests). Stay
        // detached, silent.
        return;
      }
      ws = socket;

      socket.onopen = () => {
        if (cancelled) return;
        setIsAttached(true);
        setLastError(null);
        retries = 0;
      };

      socket.onmessage = (ev: MessageEvent) => {
        if (cancelled) return;
        if (typeof ev.data === "string") {
          handleMessage(ev.data);
        }
        // Binary frames (Blob / ArrayBuffer) are not part of the
        // protocol; ignore them rather than guess at a decoding.
      };

      socket.onerror = () => {
        // onclose fires after onerror; reconnect / surface decisions
        // happen there so we see the close code.
      };

      socket.onclose = (event: CloseEvent) => {
        if (cancelled) return;
        setIsAttached(false);

        // Auth failure — stop retrying, surface a clear message so the
        // dashboard can prompt the operator to refresh / re-auth.
        if (WS_AUTH_ERROR_CODES.has(event.code)) {
          setLastError("session stream authentication required");
          return;
        }

        // Clean shutdown initiated by the server (1000) — nothing to do.
        if (event.code === WS_NORMAL_CLOSE) {
          return;
        }

        // Open-then-close before any payload arrived AND a non-auth code
        // on the first attempt: most likely the route isn't deployed on
        // this server yet or the session has no active turn. Swallow
        // silently to match the previous SSE-era behaviour the audit
        // explicitly preserves.
        if (!receivedAnyRef.current && retries === 0) {
          return;
        }

        // Mid-stream drop or transient failure — try to reconnect with
        // exponential backoff up to WS_MAX_RETRIES. Surface a soft
        // status only when we give up so the UI can show a real error.
        if (retries >= WS_MAX_RETRIES) {
          setLastError("session stream disconnected");
          return;
        }
        const delay = Math.min(1000 * 2 ** retries, 15000);
        retries += 1;
        reconnectTimer = setTimeout(connect, delay);
      };
    }

    connect();

    return () => {
      cancelled = true;
      if (reconnectTimer !== null) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      if (ws) {
        // Tear down handlers before close so a late onclose can't
        // re-enter React state for an unmounted hook.
        ws.onopen = null;
        ws.onmessage = null;
        ws.onerror = null;
        ws.onclose = null;
        try {
          ws.close(WS_NORMAL_CLOSE, "unmount");
        } catch {
          // ignore — close on an already-closing socket throws in some
          // environments.
        }
        ws = null;
      }
      setIsAttached(false);
    };
    // buildPath / webSocketCtor / buildWebSocket are stable in
    // production; tests pass them once at mount. Excluding them keeps
    // the connection from churning.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentId, sessionId]);

  return { events, isAttached, lastError };
}
