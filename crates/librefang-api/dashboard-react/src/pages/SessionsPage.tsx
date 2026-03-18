import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FormEvent, useEffect, useMemo, useState } from "react";
import {
  deleteSession,
  getSessionDetails,
  listAgents,
  listSessions,
  setSessionLabel,
  type SessionListItem
} from "../api";
import { asText, normalizeRole } from "../lib/chat";

const REFRESH_MS = 30000;

interface ActionFeedback {
  type: "ok" | "error";
  text: string;
}

function dateText(value?: string): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function roleClass(role: string): string {
  if (role === "assistant") return "border-sky-700 bg-sky-700/15 text-sky-100";
  if (role === "user") return "border-emerald-700 bg-emerald-700/15 text-emerald-100";
  return "border-slate-700 bg-slate-800/60 text-slate-100";
}

export function SessionsPage() {
  const queryClient = useQueryClient();
  const [searchFilter, setSearchFilter] = useState("");
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [labelDraft, setLabelDraft] = useState("");
  const [feedback, setFeedback] = useState<ActionFeedback | null>(null);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);

  const sessionsQuery = useQuery({
    queryKey: ["sessions", "list"],
    queryFn: listSessions,
    refetchInterval: REFRESH_MS
  });

  const agentsQuery = useQuery({
    queryKey: ["agents", "list", "sessions-helper"],
    queryFn: listAgents,
    refetchInterval: REFRESH_MS
  });

  const sessionDetailQuery = useQuery({
    queryKey: ["sessions", "detail", selectedSessionId],
    queryFn: () => getSessionDetails(selectedSessionId ?? ""),
    enabled: Boolean(selectedSessionId)
  });

  const deleteMutation = useMutation({
    mutationFn: deleteSession
  });

  const labelMutation = useMutation({
    mutationFn: ({ sessionId, label }: { sessionId: string; label: string | null }) =>
      setSessionLabel(sessionId, label)
  });

  const sessions = sessionsQuery.data ?? [];
  const agents = agentsQuery.data ?? [];

  useEffect(() => {
    if (!sessions.length) {
      setSelectedSessionId(null);
      return;
    }
    setSelectedSessionId((current) => {
      if (current && sessions.some((session) => session.session_id === current)) return current;
      return sessions[0].session_id;
    });
  }, [sessions]);

  useEffect(() => {
    const label = sessionDetailQuery.data?.label;
    setLabelDraft(typeof label === "string" ? label : "");
  }, [sessionDetailQuery.data?.label, selectedSessionId]);

  const filteredSessions = useMemo(() => {
    const keyword = searchFilter.trim().toLowerCase();
    if (!keyword) return sessions;
    return sessions.filter((session) => {
      const agentId = (session.agent_id ?? "").toLowerCase();
      const label = (session.label ?? "").toLowerCase();
      const id = session.session_id.toLowerCase();
      return agentId.includes(keyword) || label.includes(keyword) || id.includes(keyword);
    });
  }, [searchFilter, sessions]);

  const agentNameById = useMemo(() => {
    const map = new Map<string, string>();
    for (const agent of agents) {
      map.set(agent.id, agent.name);
    }
    return map;
  }, [agents]);

  const selectedSession = useMemo(() => {
    if (!selectedSessionId) return null;
    return sessions.find((session) => session.session_id === selectedSessionId) ?? null;
  }, [selectedSessionId, sessions]);

  const sessionsError = sessionsQuery.error instanceof Error ? sessionsQuery.error.message : "";
  const detailError = sessionDetailQuery.error instanceof Error ? sessionDetailQuery.error.message : "";

  async function refreshAll() {
    await queryClient.invalidateQueries({ queryKey: ["sessions"] });
    await sessionsQuery.refetch();
    if (selectedSessionId) {
      await sessionDetailQuery.refetch();
    }
  }

  async function handleDelete(session: SessionListItem) {
    if (deleteMutation.isPending) return;
    if (!window.confirm(`Delete session ${session.session_id}?`)) return;

    setPendingDeleteId(session.session_id);
    try {
      const result = await deleteMutation.mutateAsync(session.session_id);
      setFeedback({
        type: "ok",
        text:
          typeof result.status === "string"
            ? result.status
            : `session ${session.session_id.slice(0, 8)} deleted`
      });
      if (selectedSessionId === session.session_id) {
        setSelectedSessionId(null);
      }
      await refreshAll();
    } catch (error) {
      setFeedback({
        type: "error",
        text: error instanceof Error ? error.message : "Failed to delete session."
      });
    } finally {
      setPendingDeleteId(null);
    }
  }

  async function handleSaveLabel(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selectedSessionId || labelMutation.isPending) return;
    try {
      const trimmed = labelDraft.trim();
      await labelMutation.mutateAsync({
        sessionId: selectedSessionId,
        label: trimmed.length > 0 ? trimmed : null
      });
      setFeedback({ type: "ok", text: "Session label updated." });
      await refreshAll();
    } catch (error) {
      setFeedback({
        type: "error",
        text: error instanceof Error ? error.message : "Failed to update session label."
      });
    }
  }

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Sessions</h1>
          <p className="text-sm text-slate-400">Conversation session index, detail inspection, and cleanup.</p>
        </div>
        <button
          className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
          onClick={() => void refreshAll()}
          disabled={sessionsQuery.isFetching}
        >
          Refresh
        </button>
      </header>

      {feedback ? (
        <div
          className={`rounded-xl border p-3 text-sm ${
            feedback.type === "ok"
              ? "border-emerald-700 bg-emerald-700/10 text-emerald-200"
              : "border-rose-700 bg-rose-700/10 text-rose-200"
          }`}
        >
          {feedback.text}
        </div>
      ) : null}
      {sessionsError ? (
        <div className="rounded-xl border border-rose-700 bg-rose-700/15 p-4 text-rose-200">{sessionsError}</div>
      ) : null}

      <div className="grid gap-3 xl:grid-cols-[340px_1fr]">
        <aside className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <input
            value={searchFilter}
            onChange={(event) => setSearchFilter(event.target.value)}
            placeholder="Search by session ID / agent ID / label"
            className="w-full rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
          />
          <div className="mt-3 text-xs text-slate-400">
            {filteredSessions.length}/{sessions.length} sessions
          </div>

          {sessionsQuery.isLoading ? (
            <p className="mt-3 text-sm text-slate-400">Loading sessions...</p>
          ) : filteredSessions.length === 0 ? (
            <p className="mt-3 text-sm text-slate-400">No sessions found.</p>
          ) : (
            <ul className="mt-3 flex max-h-[520px] list-none flex-col gap-2 overflow-y-auto p-0">
              {filteredSessions.map((session) => {
                const active = selectedSessionId === session.session_id;
                const agentName = session.agent_id
                  ? agentNameById.get(session.agent_id) ?? session.agent_id
                  : "-";
                return (
                  <li key={session.session_id}>
                    <button
                      type="button"
                      className={`w-full rounded-lg border px-3 py-2 text-left transition ${
                        active
                          ? "border-sky-500 bg-sky-600/15"
                          : "border-slate-800 bg-slate-950/70 hover:border-slate-600"
                      }`}
                      onClick={() => setSelectedSessionId(session.session_id)}
                    >
                      <p className="m-0 truncate text-sm font-medium">{session.label ?? "(no label)"}</p>
                      <p className="m-0 mt-1 text-xs text-slate-400">
                        {agentName} · {session.message_count ?? 0} msg
                      </p>
                      <p className="m-0 mt-1 font-mono text-[11px] text-slate-500">
                        {session.session_id.slice(0, 12)}...
                      </p>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </aside>

        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          {!selectedSession ? (
            <p className="text-sm text-slate-400">Select a session to view details.</p>
          ) : (
            <>
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div>
                  <h2 className="m-0 text-base font-semibold">{selectedSession.label ?? "(no label)"}</h2>
                  <p className="m-0 mt-1 font-mono text-xs text-slate-500">{selectedSession.session_id}</p>
                </div>
                <button
                  className="rounded-lg border border-rose-700 bg-rose-700/10 px-3 py-2 text-xs font-medium text-rose-200 transition hover:bg-rose-700/20 disabled:cursor-not-allowed disabled:opacity-60"
                  onClick={() => void handleDelete(selectedSession)}
                  disabled={pendingDeleteId === selectedSession.session_id}
                >
                  Delete Session
                </button>
              </div>

              <dl className="mt-3 grid grid-cols-[120px_1fr] gap-y-2 text-sm">
                <dt className="text-slate-400">Agent</dt>
                <dd>{selectedSession.agent_id ? agentNameById.get(selectedSession.agent_id) ?? selectedSession.agent_id : "-"}</dd>
                <dt className="text-slate-400">Created</dt>
                <dd>{dateText(selectedSession.created_at)}</dd>
                <dt className="text-slate-400">Messages</dt>
                <dd>{selectedSession.message_count ?? 0}</dd>
              </dl>

              <form className="mt-3 flex gap-2" onSubmit={handleSaveLabel}>
                <input
                  value={labelDraft}
                  onChange={(event) => setLabelDraft(event.target.value)}
                  placeholder="Session label"
                  className="min-w-0 flex-1 rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
                />
                <button
                  type="submit"
                  className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-xs font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
                  disabled={labelMutation.isPending}
                >
                  Save Label
                </button>
              </form>

              {detailError ? (
                <div className="mt-3 rounded-lg border border-rose-700 bg-rose-700/10 p-3 text-sm text-rose-200">
                  {detailError}
                </div>
              ) : sessionDetailQuery.isLoading ? (
                <p className="mt-3 text-sm text-slate-400">Loading session messages...</p>
              ) : (
                <ul className="mt-3 flex max-h-[420px] list-none flex-col gap-2 overflow-y-auto p-0">
                  {(sessionDetailQuery.data?.messages ?? []).map((message, index) => {
                    const role = normalizeRole(message.role);
                    return (
                      <li
                        key={`${selectedSession.session_id}-message-${index}`}
                        className={`rounded-lg border p-3 ${roleClass(role)}`}
                      >
                        <p className="m-0 text-xs uppercase tracking-wide text-slate-300">{role}</p>
                        <p className="m-0 mt-1 whitespace-pre-wrap break-words text-sm">{asText(message.content)}</p>
                      </li>
                    );
                  })}
                </ul>
              )}
            </>
          )}
        </article>
      </div>
    </section>
  );
}
