import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import {
  approveApproval,
  listApprovals,
  rejectApproval,
  type ApiActionResponse,
  type ApprovalItem
} from "../api";

const REFRESH_MS = 5000;

type ApprovalFilter = "all" | "pending" | "approved" | "rejected";

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

function statusClass(status?: string): string {
  if (status === "approved") return "border-emerald-700 bg-emerald-700/20 text-emerald-200";
  if (status === "rejected") return "border-rose-700 bg-rose-700/20 text-rose-200";
  return "border-amber-700 bg-amber-700/20 text-amber-200";
}

function relativeTime(value?: string): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const diffSec = Math.floor((Date.now() - date.getTime()) / 1000);
  if (diffSec < 5) return "just now";
  if (diffSec < 60) return `${diffSec}s ago`;
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`;
  return `${Math.floor(diffSec / 86400)}d ago`;
}

function timeLeft(item: ApprovalItem): string {
  const created = item.created_at ?? item.requested_at;
  if (!created || !item.timeout_secs) return "-";
  const createdMs = new Date(created).getTime();
  if (Number.isNaN(createdMs)) return "-";
  const remain = createdMs + item.timeout_secs * 1000 - Date.now();
  if (remain <= 0) return "expired";
  const sec = Math.floor(remain / 1000);
  if (sec < 60) return `${sec}s`;
  if (sec < 3600) return `${Math.floor(sec / 60)}m`;
  return `${Math.floor(sec / 3600)}h`;
}

export function ApprovalsPage() {
  const queryClient = useQueryClient();
  const [filterStatus, setFilterStatus] = useState<ApprovalFilter>("all");
  const [feedback, setFeedback] = useState<ActionFeedback | null>(null);
  const [pendingId, setPendingId] = useState<string | null>(null);

  const approvalsQuery = useQuery({
    queryKey: ["approvals", "list"],
    queryFn: listApprovals,
    refetchInterval: REFRESH_MS
  });

  const approveMutation = useMutation({
    mutationFn: approveApproval
  });
  const rejectMutation = useMutation({
    mutationFn: rejectApproval
  });

  const approvals = approvalsQuery.data ?? [];
  const filtered = useMemo(() => {
    if (filterStatus === "all") return approvals;
    return approvals.filter((item) => (item.status ?? "pending") === filterStatus);
  }, [approvals, filterStatus]);
  const pendingCount = approvals.filter((item) => (item.status ?? "pending") === "pending").length;
  const error = approvalsQuery.error instanceof Error ? approvalsQuery.error.message : "";

  async function refreshApprovals() {
    await queryClient.invalidateQueries({ queryKey: ["approvals"] });
    await approvalsQuery.refetch();
  }

  async function handleApprove(id: string) {
    if (approveMutation.isPending || rejectMutation.isPending) return;
    setPendingId(id);
    try {
      const result = await approveMutation.mutateAsync(id);
      setFeedback({ type: "ok", text: actionText(result) });
      await refreshApprovals();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Approve failed."
      });
    } finally {
      setPendingId(null);
    }
  }

  async function handleReject(id: string) {
    if (approveMutation.isPending || rejectMutation.isPending) return;
    if (!window.confirm("Reject this approval request?")) return;
    setPendingId(id);
    try {
      const result = await rejectMutation.mutateAsync(id);
      setFeedback({ type: "ok", text: actionText(result) });
      await refreshApprovals();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Reject failed."
      });
    } finally {
      setPendingId(null);
    }
  }

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Approvals</h1>
          <p className="text-sm text-slate-400">Human-in-the-loop approval queue for sensitive agent actions.</p>
        </div>
        <div className="flex items-center gap-2">
          <span className="rounded-full border border-amber-700 bg-amber-700/20 px-2 py-1 text-xs text-amber-200">
            {pendingCount} pending
          </span>
          <button
            className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
            type="button"
            onClick={() => void refreshApprovals()}
            disabled={approvalsQuery.isFetching}
          >
            Refresh
          </button>
        </div>
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

      <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
        <div className="mb-3 flex flex-wrap gap-2">
          {(["all", "pending", "approved", "rejected"] as const).map((value) => (
            <button
              key={value}
              className={`rounded-lg border px-3 py-2 text-sm capitalize ${
                filterStatus === value
                  ? "border-sky-500 bg-sky-600/20 text-sky-100"
                  : "border-slate-700 bg-slate-950/70 text-slate-300 hover:border-slate-500"
              }`}
              onClick={() => setFilterStatus(value)}
            >
              {value}
            </button>
          ))}
        </div>

        {approvalsQuery.isLoading ? (
          <p className="text-sm text-slate-400">Loading approvals...</p>
        ) : filtered.length === 0 ? (
          <p className="text-sm text-slate-400">No approvals match this filter.</p>
        ) : (
          <ul className="flex list-none flex-col gap-2 p-0">
            {filtered.map((item) => {
              const status = item.status ?? "pending";
              const isPendingAction = pendingId === item.id;
              return (
                <li key={item.id} className="rounded-lg border border-slate-800 bg-slate-950/70 p-3">
                  <div className="flex flex-wrap items-start justify-between gap-2">
                    <div className="min-w-0">
                      <p className="m-0 truncate text-sm font-semibold">{item.action ?? item.action_summary ?? "-"}</p>
                      <p className="m-0 mt-1 break-words text-xs text-slate-300">{item.description ?? "-"}</p>
                      <p className="m-0 mt-1 text-xs text-slate-500">
                        {item.agent_name ?? item.agent_id ?? "-"} · {relativeTime(item.created_at ?? item.requested_at)}
                      </p>
                    </div>
                    <span className={`rounded-full border px-2 py-1 text-xs uppercase ${statusClass(status)}`}>
                      {status}
                    </span>
                  </div>

                  <div className="mt-2 grid gap-1 text-xs text-slate-400 sm:grid-cols-3">
                    <span>Tool: {item.tool_name ?? "-"}</span>
                    <span>Risk: {item.risk_level ?? "-"}</span>
                    <span>Timeout: {timeLeft(item)}</span>
                  </div>

                  <div className="mt-3 flex flex-wrap gap-2">
                    <button
                      className="rounded-lg border border-emerald-600 bg-emerald-700/20 px-3 py-1.5 text-xs font-medium text-emerald-200 transition hover:bg-emerald-700/30 disabled:cursor-not-allowed disabled:opacity-50"
                      onClick={() => void handleApprove(item.id)}
                      disabled={status !== "pending" || isPendingAction}
                    >
                      Approve
                    </button>
                    <button
                      className="rounded-lg border border-rose-600 bg-rose-700/20 px-3 py-1.5 text-xs font-medium text-rose-200 transition hover:bg-rose-700/30 disabled:cursor-not-allowed disabled:opacity-50"
                      onClick={() => void handleReject(item.id)}
                      disabled={status !== "pending" || isPendingAction}
                    >
                      Reject
                    </button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </section>
  );
}
