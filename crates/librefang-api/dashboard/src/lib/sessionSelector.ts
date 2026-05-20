import type { SessionListItem } from "../api";

/**
 * Pick the most recently created session id from a per-agent session list.
 *
 * IMPORTANT: callers MUST pass the result of `listAgentSessions(agentId)`
 * (i.e. `useAgentSessions`), NOT the global `listSessions()` payload.
 * The global `/api/sessions` endpoint is paginated to 50 rows server-side,
 * so on busy systems an agent's newest session can fall off the page and
 * this selector would return `undefined` even though sessions exist.
 * The per-agent endpoint `/api/agents/{id}/sessions` is unscoped to that
 * cap and returns the full agent-scoped history. See issue #4294.
 *
 * Returns `undefined` for an empty list. Sessions without `created_at`
 * sort as epoch 0 — a single such session is still returned, but any
 * session with a real timestamp wins over it.
 */
export function pickLatestSessionId(
  sessions: ReadonlyArray<SessionListItem> | undefined,
): string | undefined {
  if (!sessions || sessions.length === 0) return undefined;
  let best: { session_id: string; ts: number } | undefined;
  for (const s of sessions) {
    const ts = s.created_at ? Date.parse(s.created_at) : 0;
    if (!best || ts > best.ts) best = { session_id: s.session_id, ts };
  }
  return best?.session_id;
}

/**
 * Derive the "active" session id for the sessions dropdown from the URL-pinned
 * session id only.
 *
 * When the chat was opened with only `?agentId=` (no `?sessionId=`), the WS
 * connection rides the server-side canonical pointer.  Until the server
 * confirms which session was used (and the URL is pinned via `?sessionId=`),
 * we cannot know which session is actually receiving messages.  Returning
 * `undefined` prevents the dropdown from highlighting a session that may not
 * be the live one — the highlight would imply messages go there, which is
 * only true once the URL is pinned.
 *
 * Pass `urlSessionId` (from the router search params) directly; do NOT pass
 * a fallback derived from the session list.
 */
export function deriveDropdownActiveSessionId(
  urlSessionId: string | null | undefined,
): string | undefined {
  return urlSessionId ?? undefined;
}

/**
 * Should the chat hook auto-pin a server-resolved session id into the URL?
 *
 * Issue #5199 — shared gate for both transport paths:
 *
 *   - WS `response` event (`ChatPage.tsx` WS handler block)
 *   - HTTP `/message` reply (`ChatPage.tsx::sendViaHttp` fallback)
 *
 * The HTTP path was added in the round-2 review of PR #5253 after Codex
 * flagged that the WS-only gate left the fallback transport silently
 * unpinned: a first send before WS connects, or a send that takes the
 * 180s/30s WS-drop fallback timer, would persist to a concrete session
 * but leave `?sessionId=` absent from the URL — the very regression the
 * PR was meant to close. Both paths now share this gate so a future
 * refactor cannot drift them apart.
 *
 * All five guards must hold:
 *
 *   1. `sendAgentId === currentAgentId` — the user is still viewing the
 *      agent that issued the send. If they've navigated to a different
 *      agent, rewriting the URL now would land them somewhere they're
 *      not looking at; the off-screen agent's existing bare-`agentId`
 *      heuristics will still resolve the session on the next navigate-back.
 *   2. `currentSessionId === null` — the user has NOT manually pinned a
 *      session between send and response (e.g. clicked a row in the
 *      sessions dropdown). `ws.close()` is async, and a frame queued in
 *      the browser can still reach the about-to-detach listener before
 *      close() takes effect; the HTTP path has the same window via the
 *      awaited mutation. Overwriting the user's deliberate pin would be
 *      a worse regression than the unpinned-URL bug.
 *   3. `urlSessionId == null` — the request itself was unpinned. We never
 *      override an explicit `?sessionId=` even if (1) and (2) somehow
 *      hold; the server already mirrors this on its side by omitting
 *      `session_id` from the response body when the request supplied one.
 *   4. `resolvedSessionId` is a non-empty string — defensive guard for
 *      malformed server responses; the wire contract is "string or
 *      absent", but a regression that emits `""` should not pin to an
 *      empty id.
 *
 * Pure function so both transport paths can call it identically and the
 * contract is unit-testable in isolation.
 */
export function shouldAutoPinResolvedSession(args: {
  sendAgentId: string;
  currentAgentId: string | null;
  currentSessionId: string | null;
  urlSessionId: string | null | undefined;
  resolvedSessionId: unknown;
}): args is typeof args & { resolvedSessionId: string } {
  if (args.sendAgentId !== args.currentAgentId) return false;
  if (args.currentSessionId !== null) return false;
  if (args.urlSessionId) return false;
  if (typeof args.resolvedSessionId !== "string") return false;
  if (args.resolvedSessionId.length === 0) return false;
  return true;
}

/**
 * Pick the dropdown's truncated label given the resolved active session id and
 * the per-agent session list. Pure function so the contract is unit-testable
 * (the dropdown body is otherwise buried in a large JSX block).
 *
 * Returns `null` for the unpinned case so the caller can render the localized
 * "Unpinned" hint string — keeping i18n at the call site rather than baking
 * English into the selector. When the active session id is present but no
 * matching row is found in `sessions`, falls back to the short 8-char prefix
 * of the id; if even that is empty, returns `null` so the caller can render
 * its own generic placeholder. Issue #5199-C.
 */
export function pickSessionDropdownLabel(
  activeSessionId: string | undefined,
  sessions: ReadonlyArray<{ session_id?: string; label?: string | null }> | undefined,
): string | null {
  if (!activeSessionId) return null;
  const active = sessions?.find((s) => s.session_id === activeSessionId);
  if (active?.label) return active.label;
  const short = activeSessionId.slice(0, 8);
  return short.length > 0 ? short : null;
}
