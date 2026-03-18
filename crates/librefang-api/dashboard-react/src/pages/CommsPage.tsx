import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FormEvent, useMemo, useState } from "react";
import {
  getCommsTopology,
  listCommsEvents,
  postCommsTask,
  sendCommsMessage,
  type ApiActionResponse,
  type CommsEventItem,
  type CommsNode
} from "../api";

const REFRESH_MS = 10000;

interface ActionFeedback {
  type: "ok" | "error";
  text: string;
}

function actionText(action: ApiActionResponse): string {
  if (typeof action.message === "string" && action.message.trim().length > 0) return action.message;
  if (typeof action.status === "string" && action.status.trim().length > 0) return action.status;
  if (typeof action.error === "string" && action.error.trim().length > 0) return action.error;
  return JSON.stringify(action);
}

function dateText(value?: string): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function eventKindText(kind?: string): string {
  if (!kind) return "event";
  return kind.replace(/_/g, " ");
}

function stateClass(state?: string): string {
  const value = (state ?? "").toLowerCase();
  if (value.includes("running")) return "border-emerald-700 bg-emerald-700/15 text-emerald-100";
  if (value.includes("suspended") || value.includes("paused")) {
    return "border-amber-700 bg-amber-700/15 text-amber-100";
  }
  if (value.includes("terminated") || value.includes("crashed")) {
    return "border-rose-700 bg-rose-700/15 text-rose-100";
  }
  return "border-slate-700 bg-slate-800/60 text-slate-100";
}

function nodeName(node: CommsNode): string {
  return node.name ?? node.id;
}

export function CommsPage() {
  const queryClient = useQueryClient();
  const [feedback, setFeedback] = useState<ActionFeedback | null>(null);
  const [sendFrom, setSendFrom] = useState("");
  const [sendTo, setSendTo] = useState("");
  const [sendMessage, setSendMessage] = useState("");
  const [taskTitle, setTaskTitle] = useState("");
  const [taskDescription, setTaskDescription] = useState("");
  const [taskAssignTo, setTaskAssignTo] = useState("");

  const topologyQuery = useQuery({
    queryKey: ["comms", "topology"],
    queryFn: getCommsTopology,
    refetchInterval: REFRESH_MS
  });
  const eventsQuery = useQuery({
    queryKey: ["comms", "events", 200],
    queryFn: () => listCommsEvents(200),
    refetchInterval: REFRESH_MS
  });

  const sendMutation = useMutation({
    mutationFn: sendCommsMessage
  });
  const taskMutation = useMutation({
    mutationFn: postCommsTask
  });

  const nodes = topologyQuery.data?.nodes ?? [];
  const edges = topologyQuery.data?.edges ?? [];
  const events = eventsQuery.data ?? [];

  const peerEdgeCount = useMemo(
    () => edges.filter((edge) => (edge.kind ?? "").toLowerCase() === "peer").length,
    [edges]
  );
  const parentChildEdgeCount = useMemo(
    () =>
      edges.filter((edge) => {
        const normalized = (edge.kind ?? "").toLowerCase().replace(/_/g, "");
        return normalized === "parentchild";
      }).length,
    [edges]
  );
  const nodeById = useMemo(() => {
    const map = new Map<string, CommsNode>();
    for (const node of nodes) {
      map.set(node.id, node);
    }
    return map;
  }, [nodes]);

  const error = (() => {
    if (topologyQuery.error instanceof Error) return topologyQuery.error.message;
    if (eventsQuery.error instanceof Error) return eventsQuery.error.message;
    return "";
  })();

  async function refreshAll() {
    await queryClient.invalidateQueries({ queryKey: ["comms"] });
    await Promise.all([topologyQuery.refetch(), eventsQuery.refetch()]);
  }

  async function handleSend(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!sendFrom || !sendTo || !sendMessage.trim() || sendMutation.isPending) return;

    try {
      const result = await sendMutation.mutateAsync({
        from_agent_id: sendFrom,
        to_agent_id: sendTo,
        message: sendMessage.trim()
      });
      setFeedback({ type: "ok", text: actionText(result) });
      setSendMessage("");
      await eventsQuery.refetch();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Failed to send message."
      });
    }
  }

  async function handlePostTask(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!taskTitle.trim() || taskMutation.isPending) return;
    try {
      const result = await taskMutation.mutateAsync({
        title: taskTitle.trim(),
        description: taskDescription.trim(),
        ...(taskAssignTo ? { assigned_to: taskAssignTo } : {})
      });
      setFeedback({ type: "ok", text: actionText(result) });
      setTaskTitle("");
      setTaskDescription("");
      await eventsQuery.refetch();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Failed to post task."
      });
    }
  }

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Comms</h1>
          <p className="text-sm text-slate-400">Inter-agent topology, recent communication events, and task dispatch.</p>
        </div>
        <button
          className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
          onClick={() => void refreshAll()}
          disabled={topologyQuery.isFetching || eventsQuery.isFetching}
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
      {error ? (
        <div className="rounded-xl border border-rose-700 bg-rose-700/15 p-4 text-rose-200">{error}</div>
      ) : null}

      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">Agents</span>
          <strong className="mt-1 block text-2xl">{nodes.length}</strong>
        </article>
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">Peer Links</span>
          <strong className="mt-1 block text-2xl">{peerEdgeCount}</strong>
        </article>
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">Hierarchy Links</span>
          <strong className="mt-1 block text-2xl">{parentChildEdgeCount}</strong>
        </article>
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">Events</span>
          <strong className="mt-1 block text-2xl">{events.length}</strong>
        </article>
      </div>

      <div className="grid gap-3 xl:grid-cols-2">
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="m-0 text-base font-semibold">Send Agent Message</h2>
          <form className="mt-3 flex flex-col gap-2" onSubmit={handleSend}>
            <select
              value={sendFrom}
              onChange={(event) => setSendFrom(event.target.value)}
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={sendMutation.isPending}
            >
              <option value="">From agent</option>
              {nodes.map((node) => (
                <option key={node.id} value={node.id}>
                  {nodeName(node)}
                </option>
              ))}
            </select>
            <select
              value={sendTo}
              onChange={(event) => setSendTo(event.target.value)}
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={sendMutation.isPending}
            >
              <option value="">To agent</option>
              {nodes.map((node) => (
                <option key={node.id} value={node.id}>
                  {nodeName(node)}
                </option>
              ))}
            </select>
            <textarea
              value={sendMessage}
              onChange={(event) => setSendMessage(event.target.value)}
              rows={3}
              placeholder="Message"
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={sendMutation.isPending}
            />
            <button
              type="submit"
              className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
              disabled={!sendFrom || !sendTo || sendMessage.trim().length === 0 || sendMutation.isPending}
            >
              Send
            </button>
          </form>
        </article>

        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="m-0 text-base font-semibold">Post Task</h2>
          <form className="mt-3 flex flex-col gap-2" onSubmit={handlePostTask}>
            <input
              value={taskTitle}
              onChange={(event) => setTaskTitle(event.target.value)}
              placeholder="Task title"
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={taskMutation.isPending}
            />
            <textarea
              value={taskDescription}
              onChange={(event) => setTaskDescription(event.target.value)}
              rows={3}
              placeholder="Task description"
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={taskMutation.isPending}
            />
            <select
              value={taskAssignTo}
              onChange={(event) => setTaskAssignTo(event.target.value)}
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={taskMutation.isPending}
            >
              <option value="">Assign to (optional)</option>
              {nodes.map((node) => (
                <option key={node.id} value={node.id}>
                  {nodeName(node)}
                </option>
              ))}
            </select>
            <button
              type="submit"
              className="rounded-lg border border-emerald-600 bg-emerald-700/20 px-3 py-2 text-sm font-medium text-emerald-200 transition hover:bg-emerald-700/30 disabled:cursor-not-allowed disabled:opacity-60"
              disabled={taskTitle.trim().length === 0 || taskMutation.isPending}
            >
              Post Task
            </button>
          </form>
        </article>
      </div>

      <div className="grid gap-3 xl:grid-cols-[320px_1fr]">
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="m-0 text-base font-semibold">Topology</h2>
          {topologyQuery.isLoading ? (
            <p className="mt-2 text-sm text-slate-400">Loading topology...</p>
          ) : nodes.length === 0 ? (
            <p className="mt-2 text-sm text-slate-400">No agents in topology.</p>
          ) : (
            <ul className="mt-3 flex max-h-[460px] list-none flex-col gap-2 overflow-y-auto p-0">
              {nodes.map((node) => (
                <li key={node.id} className="rounded-lg border border-slate-800 bg-slate-950/70 p-3">
                  <div className="flex items-center justify-between gap-2">
                    <strong className="truncate text-sm">{nodeName(node)}</strong>
                    <span className={`rounded-full border px-2 py-1 text-xs ${stateClass(node.state)}`}>
                      {node.state ?? "unknown"}
                    </span>
                  </div>
                  <p className="m-0 mt-1 text-xs text-slate-500">{node.model ?? "-"}</p>
                </li>
              ))}
            </ul>
          )}
        </article>

        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="m-0 text-base font-semibold">Events</h2>
          {eventsQuery.isLoading ? (
            <p className="mt-2 text-sm text-slate-400">Loading events...</p>
          ) : events.length === 0 ? (
            <p className="mt-2 text-sm text-slate-400">No comms events.</p>
          ) : (
            <ul className="mt-3 flex max-h-[460px] list-none flex-col gap-2 overflow-y-auto p-0">
              {events.map((eventItem: CommsEventItem) => (
                <li
                  key={eventItem.id ?? `${eventItem.timestamp}-${eventItem.kind}-${eventItem.source_id}`}
                  className="rounded-lg border border-slate-800 bg-slate-950/70 p-3"
                >
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <span className="rounded-full border border-slate-700 bg-slate-800/60 px-2 py-1 text-xs text-slate-200">
                      {eventKindText(eventItem.kind)}
                    </span>
                    <span className="text-xs text-slate-500">{dateText(eventItem.timestamp)}</span>
                  </div>
                  <p className="m-0 mt-2 text-sm text-slate-100">
                    {(eventItem.source_name ?? nodeById.get(eventItem.source_id ?? "")?.name ?? eventItem.source_id ?? "-")}
                    {" -> "}
                    {(eventItem.target_name ?? nodeById.get(eventItem.target_id ?? "")?.name ?? eventItem.target_id ?? "-")}
                  </p>
                  <p className="m-0 mt-1 break-words text-xs text-slate-400">{eventItem.detail ?? "-"}</p>
                </li>
              ))}
            </ul>
          )}
        </article>
      </div>
    </section>
  );
}
