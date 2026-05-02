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
