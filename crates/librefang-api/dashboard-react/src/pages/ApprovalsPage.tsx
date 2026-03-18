import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { approveApproval, listApprovals, type ApprovalItem } from "../api";

const REFRESH_MS = 15000;

export function ApprovalsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [pendingId, setPendingId] = useState<string | null>(null);

  const approvalsQuery = useQuery({ queryKey: ["approvals", "list"], queryFn: listApprovals, refetchInterval: REFRESH_MS });
  const approveMutation = useMutation({ mutationFn: ({ id, decision }: any) => approveApproval(id) });

  const approvals = approvalsQuery.data ?? [];

  async function handleDecision(id: string, decision: "approve" | "reject") {
    setPendingId(id);
    try {
      await approveMutation.mutateAsync({ id, decision });
      await queryClient.invalidateQueries({ queryKey: ["approvals"] });
    } finally { setPendingId(null); }
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" /><polyline points="22 4 12 14.01 9 11.01" /></svg>
            {t("nav.approvals")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">{t("approvals.title")}</h1>
          <p className="mt-1 text-text-dim font-medium">{t("approvals.subtitle")}</p>
        </div>
        <button className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm" onClick={() => void approvalsQuery.refetch()}>
          {t("common.refresh")}
        </button>
      </header>

      <div className="grid gap-4">
        {approvals.map((a) => (
          <article key={a.id} className="group rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm transition-all hover:border-brand/30">
            <div className="flex flex-col md:flex-row md:items-center justify-between gap-6">
              <div className="min-w-0 flex-1">
                <span className="px-2 py-0.5 rounded-lg bg-warning/10 border border-warning/20 text-[10px] font-black text-warning uppercase tracking-widest">{t("approvals.pending_review")}</span>
                <p className="mt-4 text-sm font-medium leading-relaxed">{a.prompt || t("common.actions")}</p>
              </div>
              <div className="flex gap-3 shrink-0">
                <button onClick={() => handleDecision(a.id, "reject")} className="px-6 py-2.5 rounded-xl border border-error/20 bg-error/5 text-error text-xs font-black hover:bg-error/10">{t("approvals.reject")}</button>
                <button onClick={() => handleDecision(a.id, "approve")} className="px-8 py-2.5 rounded-xl bg-success text-white text-xs font-black shadow-lg shadow-success/20">{t("approvals.approve")}</button>
              </div>
            </div>
          </article>
        ))}
        {approvals.length === 0 && !approvalsQuery.isLoading && (
          <div className="py-24 text-center border border-dashed border-border-subtle rounded-3xl bg-surface/30">
            <h3 className="text-lg font-black tracking-tight">{t("approvals.queue_clear")}</h3>
            <p className="text-sm text-text-dim mt-1">{t("approvals.queue_clear_desc")}</p>
          </div>
        )}
      </div>
    </div>
  );
}
