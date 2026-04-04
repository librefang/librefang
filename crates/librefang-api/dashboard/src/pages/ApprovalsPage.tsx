import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  approveApproval,
  rejectApproval,
  listApprovals,
  batchResolveApprovals,
  modifyAndRetryApproval,
  queryApprovalAudit,
  type ApprovalAuditEntry,
} from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { useUIStore } from "../lib/store";
import { CheckCircle, XCircle, Clock, MessageSquare, ChevronLeft, ChevronRight } from "lucide-react";

const REFRESH_MS = 15000;
const AUDIT_PAGE_SIZE = 20;

type Tab = "pending" | "audit";

function statusBadge(status: string | undefined, t: (key: string) => string) {
  switch (status) {
    case "approved":
      return <Badge variant="success">{t("approvals.status.approved")}</Badge>;
    case "rejected":
      return <Badge variant="danger">{t("approvals.status.rejected")}</Badge>;
    case "expired":
      return <Badge variant="neutral">{t("approvals.status.expired")}</Badge>;
    default:
      return <Badge variant="warning">{t("approvals.pending_review")}</Badge>;
  }
}

function statusIcon(status: string | undefined) {
  switch (status) {
    case "approved":
      return <CheckCircle className="w-5 h-5 text-success" />;
    case "rejected":
      return <XCircle className="w-5 h-5 text-danger" />;
    case "expired":
      return <Clock className="w-5 h-5 text-text-dim" />;
    default:
      return <CheckCircle className="w-5 h-5 text-warning" />;
  }
}

function statusIconBg(status: string | undefined) {
  switch (status) {
    case "approved":
      return "bg-success/10";
    case "rejected":
      return "bg-danger/10";
    case "expired":
      return "bg-surface-2";
    default:
      return "bg-warning/10";
  }
}

function decisionBadge(decision: string, t: (key: string) => string) {
  switch (decision) {
    case "approved":
      return <Badge variant="success">{t("approvals.status.approved")}</Badge>;
    case "rejected":
      return <Badge variant="error">{t("approvals.status.rejected")}</Badge>;
    case "modified":
      return <Badge variant="info">{t("approvals.modify")}</Badge>;
    default:
      return <Badge variant="default">{decision}</Badge>;
  }
}

/* ------------------------------------------------------------------ */
/*  Modify & Retry inline form                                        */
/* ------------------------------------------------------------------ */

function ModifyForm({
  id,
  onDone,
}: {
  id: string;
  onDone: () => void;
}) {
  const { t } = useTranslation();
  const [feedback, setFeedback] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);

  async function handleSubmit() {
    if (!feedback.trim()) return;
    setSubmitting(true);
    try {
      await modifyAndRetryApproval(id, feedback.trim());
      addToast(t("approvals.modifiedToast"), "success");
      queryClient.invalidateQueries({ queryKey: ["approvals"] });
      onDone();
    } catch (e: any) {
      addToast(e.message || t("common.error"), "error");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="mt-3 flex flex-col gap-2">
      <label className="text-xs font-bold text-text-dim">{t("approvals.modifyTitle")}</label>
      <textarea
        value={feedback}
        onChange={(e) => setFeedback(e.target.value)}
        placeholder={t("approvals.modifyPlaceholder")}
        rows={3}
        className="w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none transition-colors resize-none"
      />
      <div className="flex gap-2 justify-end">
        <Button variant="ghost" size="sm" onClick={onDone}>
          {t("common.cancel", "Cancel")}
        </Button>
        <Button
          variant="primary"
          size="sm"
          onClick={handleSubmit}
          disabled={submitting || !feedback.trim()}
          isLoading={submitting}
        >
          {t("approvals.modifySubmit")}
        </Button>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  Audit Log Tab                                                      */
/* ------------------------------------------------------------------ */

function AuditLogTab() {
  const { t } = useTranslation();
  const [offset, setOffset] = useState(0);

  const auditQuery = useQuery({
    queryKey: ["approvals", "audit", offset],
    queryFn: () => queryApprovalAudit({ limit: AUDIT_PAGE_SIZE, offset }),
  });

  const entries: ApprovalAuditEntry[] = auditQuery.data?.entries ?? [];
  const total = auditQuery.data?.total ?? 0;
  const from = total === 0 ? 0 : offset + 1;
  const to = Math.min(offset + AUDIT_PAGE_SIZE, total);

  if (auditQuery.isLoading) {
    return <ListSkeleton rows={5} />;
  }

  if (entries.length === 0) {
    return (
      <div className="flex flex-col items-center py-20">
        <div className="w-20 h-20 rounded-3xl bg-surface-hover flex items-center justify-center mb-6">
          <Clock className="h-10 w-10 text-text-dim" />
        </div>
        <h3 className="text-xl font-black tracking-tight">{t("approvals.auditLog.noEntries")}</h3>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      {/* Table */}
      <div className="overflow-x-auto rounded-xl border border-border-subtle">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border-subtle bg-surface-hover/50">
              <th className="px-4 py-3 text-left text-xs font-bold uppercase tracking-wider text-text-dim">
                {t("approvals.auditLog.tool")}
              </th>
              <th className="px-4 py-3 text-left text-xs font-bold uppercase tracking-wider text-text-dim">
                {t("approvals.auditLog.agent")}
              </th>
              <th className="px-4 py-3 text-left text-xs font-bold uppercase tracking-wider text-text-dim">
                {t("approvals.auditLog.decision")}
              </th>
              <th className="px-4 py-3 text-left text-xs font-bold uppercase tracking-wider text-text-dim">
                {t("approvals.auditLog.decidedBy")}
              </th>
              <th className="px-4 py-3 text-left text-xs font-bold uppercase tracking-wider text-text-dim">
                {t("approvals.auditLog.decidedAt")}
              </th>
              <th className="px-4 py-3 text-left text-xs font-bold uppercase tracking-wider text-text-dim">
                {t("approvals.auditLog.feedback")}
              </th>
            </tr>
          </thead>
          <tbody>
            {entries.map((entry) => (
              <tr key={entry.id} className="border-b last:border-0 border-border-subtle hover:bg-surface-hover/30 transition-colors">
                <td className="px-4 py-3 font-medium">{entry.tool_name}</td>
                <td className="px-4 py-3 text-text-dim">{entry.agent_id}</td>
                <td className="px-4 py-3">{decisionBadge(entry.decision, t)}</td>
                <td className="px-4 py-3 text-text-dim">{entry.decided_by ?? "-"}</td>
                <td className="px-4 py-3 text-text-dim text-xs">
                  {entry.decided_at ? new Date(entry.decided_at).toLocaleString() : "-"}
                </td>
                <td className="px-4 py-3 text-text-dim text-xs max-w-48 truncate">
                  {entry.feedback ?? "-"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {/* Pagination */}
      <div className="flex items-center justify-between text-sm text-text-dim">
        <span>
          {t("approvals.auditLog.showing", { from, to, total })}
        </span>
        <div className="flex gap-2">
          <Button
            variant="secondary"
            size="sm"
            disabled={offset === 0}
            onClick={() => setOffset(Math.max(0, offset - AUDIT_PAGE_SIZE))}
            leftIcon={<ChevronLeft className="h-4 w-4" />}
          >
            {t("common.previous", "Previous")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            disabled={offset + AUDIT_PAGE_SIZE >= total}
            onClick={() => setOffset(offset + AUDIT_PAGE_SIZE)}
            rightIcon={<ChevronRight className="h-4 w-4" />}
          >
            {t("common.next", "Next")}
          </Button>
        </div>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  Main Page                                                          */
/* ------------------------------------------------------------------ */

export function ApprovalsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [modifyingId, setModifyingId] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<Tab>("pending");
  const addToast = useUIStore((s) => s.addToast);

  const approvalsQuery = useQuery({ queryKey: ["approvals", "list"], queryFn: listApprovals, refetchInterval: REFRESH_MS });

  const approveMutation = useMutation({
    mutationFn: (id: string) => approveApproval(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["approvals"] }),
  });
  const rejectMutation = useMutation({
    mutationFn: (id: string) => rejectApproval(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["approvals"] }),
  });

  const approvals = approvalsQuery.data ?? [];
  const pendingApprovals = approvals.filter((a) => !a.status || a.status === "pending");

  async function handleDecision(id: string, decision: "approve" | "reject") {
    setPendingId(id);
    try {
      if (decision === "approve") {
        await approveMutation.mutateAsync(id);
        addToast(t("approvals.approvedToast"), "success");
      } else {
        await rejectMutation.mutateAsync(id);
        addToast(t("approvals.rejectedToast"), "success");
      }
      // Remove from selection after resolution
      setSelected((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    } catch (e: any) {
      addToast(e.message || t("common.error"), "error");
    } finally {
      setPendingId(null);
    }
  }

  async function handleBatchAction(decision: "approve" | "reject") {
    if (selected.size === 0) return;
    const ids = Array.from(selected);
    setPendingId("batch");
    try {
      await batchResolveApprovals(ids, decision);
      addToast(t("approvals.batchSuccess"), "success");
      setSelected(new Set());
      queryClient.invalidateQueries({ queryKey: ["approvals"] });
    } catch (e: any) {
      addToast(e.message || t("common.error"), "error");
    } finally {
      setPendingId(null);
    }
  }

  function toggleSelect(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function toggleSelectAll() {
    if (selected.size === pendingApprovals.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(pendingApprovals.map((a) => a.id)));
    }
  }

  const tabClass = (tab: Tab) =>
    `px-4 py-2 text-sm font-bold rounded-lg transition-colors ${
      activeTab === tab
        ? "bg-brand/10 text-brand border border-brand/20"
        : "text-text-dim hover:text-text-main hover:bg-surface-hover border border-transparent"
    }`;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("nav.approvals")}
        title={t("approvals.title")}
        subtitle={t("approvals.subtitle")}
        isFetching={approvalsQuery.isFetching}
        onRefresh={() => void approvalsQuery.refetch()}
        icon={<CheckCircle className="h-4 w-4" />}
        helpText={t("approvals.help")}
      />

      {/* Tab toggle */}
      <div className="flex gap-2">
        <button className={tabClass("pending")} onClick={() => setActiveTab("pending")}>
          {t("approvals.tabPending")}
          {pendingApprovals.length > 0 && (
            <span className="ml-2 inline-flex h-5 min-w-5 items-center justify-center rounded-full bg-warning/20 px-1.5 text-[10px] font-bold text-warning">
              {pendingApprovals.length}
            </span>
          )}
        </button>
        <button className={tabClass("audit")} onClick={() => setActiveTab("audit")}>
          {t("approvals.tabAuditLog")}
        </button>
      </div>

      {activeTab === "audit" ? (
        <AuditLogTab />
      ) : (
        <>
          {/* Batch action bar */}
          {pendingApprovals.length > 0 && (
            <div className="flex items-center gap-3 flex-wrap">
              <label className="flex items-center gap-2 text-sm text-text-dim cursor-pointer select-none">
                <input
                  type="checkbox"
                  checked={selected.size === pendingApprovals.length && pendingApprovals.length > 0}
                  onChange={toggleSelectAll}
                  className="h-4 w-4 rounded border-border-subtle text-brand focus:ring-brand/30 accent-[var(--color-brand)]"
                />
                {t("approvals.selectAll")}
              </label>
              {selected.size > 0 && (
                <>
                  <span className="text-xs text-text-dim">
                    {t("approvals.selected", { count: selected.size })}
                  </span>
                  <Button
                    variant="success"
                    size="sm"
                    onClick={() => handleBatchAction("approve")}
                    disabled={pendingId === "batch"}
                    isLoading={pendingId === "batch"}
                  >
                    {t("approvals.approveSelected")}
                  </Button>
                  <Button
                    variant="danger"
                    size="sm"
                    onClick={() => handleBatchAction("reject")}
                    disabled={pendingId === "batch"}
                    isLoading={pendingId === "batch"}
                  >
                    {t("approvals.rejectSelected")}
                  </Button>
                </>
              )}
            </div>
          )}

          {approvalsQuery.isLoading ? (
            <ListSkeleton rows={3} />
          ) : approvalsQuery.isError ? (
            <div className="flex flex-col items-center py-20">
              <div className="w-20 h-20 rounded-3xl bg-error/10 flex items-center justify-center mb-6">
                <XCircle className="h-10 w-10 text-error" />
              </div>
              <h3 className="text-xl font-black tracking-tight">{t("common.error", "Error")}</h3>
              <p className="text-sm text-text-dim mt-2 max-w-xs text-center">{t("approvals.loadError", "Failed to load approvals. Check your connection.")}</p>
              <Button variant="secondary" size="sm" className="mt-4" onClick={() => approvalsQuery.refetch()}>
                {t("common.retry", "Retry")}
              </Button>
            </div>
          ) : approvals.length === 0 ? (
            <div className="flex flex-col items-center py-20">
              <div className="relative mb-6">
                <div className="w-20 h-20 rounded-3xl bg-success/10 flex items-center justify-center">
                  <CheckCircle className="h-10 w-10 text-success" />
                </div>
                <span className="absolute inset-0 rounded-3xl bg-success/5 animate-pulse" style={{ animationDuration: "3s" }} />
              </div>
              <h3 className="text-xl font-black tracking-tight">{t("approvals.queue_clear")}</h3>
              <p className="text-sm text-text-dim mt-2 max-w-xs text-center">{t("approvals.queue_clear_desc")}</p>
            </div>
          ) : (
            <div className="grid gap-4">
              {approvals.map((a) => {
                const isPending = !a.status || a.status === "pending";
                return (
                  <Card key={a.id} hover padding="lg">
                    <div className="flex flex-col md:flex-row md:items-center justify-between gap-6">
                      <div className="min-w-0 flex-1 flex items-center gap-3">
                        {/* Checkbox for pending items */}
                        {isPending && (
                          <input
                            type="checkbox"
                            checked={selected.has(a.id)}
                            onChange={() => toggleSelect(a.id)}
                            className="h-4 w-4 rounded border-border-subtle text-brand focus:ring-brand/30 shrink-0 accent-[var(--color-brand)]"
                          />
                        )}
                        <div className={`w-10 h-10 rounded-xl flex items-center justify-center shrink-0 ${statusIconBg(a.status)}`}>
                          {statusIcon(a.status)}
                        </div>
                        <div>
                          {statusBadge(a.status, t)}
                          <p className="mt-1 text-sm font-medium leading-relaxed">{a.action_summary || a.description || t("common.actions")}</p>
                        </div>
                      </div>
                      {isPending ? (
                        <div className="flex gap-2 shrink-0">
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => setModifyingId(modifyingId === a.id ? null : a.id)}
                            leftIcon={<MessageSquare className="h-3.5 w-3.5" />}
                          >
                            {t("approvals.modify")}
                          </Button>
                          <Button variant="danger" size="sm" onClick={() => handleDecision(a.id, "reject")} disabled={pendingId === a.id}>
                            {t("approvals.reject")}
                          </Button>
                          <Button variant="success" size="sm" onClick={() => handleDecision(a.id, "approve")} disabled={pendingId === a.id}>
                            {t("approvals.approve")}
                          </Button>
                        </div>
                      ) : (
                        <div className="text-sm text-text-dim shrink-0">
                          {t(`approvals.status.${a.status}`)}
                        </div>
                      )}
                    </div>
                    {/* Modify form */}
                    {modifyingId === a.id && isPending && (
                      <ModifyForm id={a.id} onDone={() => setModifyingId(null)} />
                    )}
                  </Card>
                );
              })}
            </div>
          )}
        </>
      )}
    </div>
  );
}
