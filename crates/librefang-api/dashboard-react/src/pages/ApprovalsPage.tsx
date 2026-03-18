import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { approveApproval, listApprovals, type ApprovalItem } from "../api";

const REFRESH_MS = 15000;

export function ApprovalsPage() {
  const queryClient = useQueryClient();
  const [pendingId, setPendingId] = useState<string | null>(null);

  const approvalsQuery = useQuery({
    queryKey: ["approvals", "list"],
    queryFn: listApprovals,
    refetchInterval: REFRESH_MS
  });

  const approveMutation = useMutation({
    mutationFn: ({ id, decision }: { id: string; decision: "approve" | "reject" }) =>
      approveApproval(id, decision === "approve")
  });

  const approvals = approvalsQuery.data ?? [];

  async function handleDecision(id: string, decision: "approve" | "reject") {
    setPendingId(id);
    try {
      await approveMutation.mutateAsync({ id, decision });
      await queryClient.invalidateQueries({ queryKey: ["approvals"] });
    } catch (e) {
      console.error("Approval action failed", e);
    } finally {
      setPendingId(null);
    }
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" /><polyline points="22 4 12 14.01 9 11.01" />
            </svg>
            Human-in-the-loop
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">Approvals</h1>
          <p className="mt-1 text-text-dim font-medium max-w-2xl">Review and authorize critical agent actions before execution.</p>
        </div>
        <button
          className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm"
          onClick={() => void approvalsQuery.refetch()}
        >
          <svg className={`h-3.5 w-3.5 ${approvalsQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
            <path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
          </svg>
          Refresh
        </button>
      </header>

      <div className="grid gap-4">
        {approvals.map((a: ApprovalItem) => (
          <article key={a.id} className="group rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm transition-all hover:border-brand/30 ring-1 ring-black/5 dark:ring-white/5">
            <div className="flex flex-col md:flex-row md:items-center justify-between gap-6">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-3 mb-2">
                  <span className="px-2 py-0.5 rounded-lg bg-warning/10 border border-warning/20 text-[10px] font-black text-warning uppercase tracking-widest">Pending Review</span>
                  <h2 className="text-sm font-black truncate text-slate-700 dark:text-slate-200">Task Authorization: {a.id.slice(0, 8)}</h2>
                </div>
                <p className="text-sm font-medium text-slate-900 dark:text-white leading-relaxed mb-4">{a.prompt || "The agent is requesting permission to perform an action."}</p>
                <div className="flex flex-wrap gap-4 text-[10px] font-bold text-text-dim uppercase tracking-wider">
                  <div className="flex items-center gap-1">
                    <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z" /></svg>
                    Agent: <span className="text-brand">{a.agent_id || "System"}</span>
                  </div>
                  <div className="flex items-center gap-1">
                    <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path d="M12 8v4l3 2" /></svg>
                    Created: {new Date(a.created_at || "").toLocaleString()}
                  </div>
                </div>
              </div>
              
              <div className="flex gap-3 shrink-0">
                <button
                  onClick={() => void handleDecision(a.id, "reject")}
                  disabled={pendingId === a.id}
                  className="px-6 py-2.5 rounded-xl border border-error/20 bg-error/5 text-error text-xs font-black hover:bg-error/10 transition-all disabled:opacity-50"
                >
                  Reject
                </button>
                <button
                  onClick={() => void handleDecision(a.id, "approve")}
                  disabled={pendingId === a.id}
                  className="px-8 py-2.5 rounded-xl bg-success text-white text-xs font-black shadow-lg shadow-success/20 hover:opacity-90 transition-all disabled:opacity-50"
                >
                  Approve Execution
                </button>
              </div>
            </div>
          </article>
        ))}

        {approvals.length === 0 && !approvalsQuery.isLoading && (
          <div className="py-24 text-center border border-dashed border-border-subtle rounded-3xl bg-surface/30">
            <div className="mx-auto h-12 w-12 rounded-full bg-success/10 flex items-center justify-center text-success mb-4">
              <svg className="h-6 w-6" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2"><path d="M5 13l4 4L19 7" /></svg>
            </div>
            <h3 className="text-lg font-black tracking-tight">Queue Clear</h3>
            <p className="text-sm text-text-dim mt-1">No pending actions require your approval.</p>
          </div>
        )}
      </div>
    </div>
  );
}
