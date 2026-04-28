import { useState, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { type ApprovalAuditEntry } from "../api";
import {
  useApprovals,
  useApprovalAudit,
  useTotpStatus,
} from "../lib/queries/approvals";
import {
  useApproveApproval,
  useRejectApproval,
  useBatchResolveApprovals,
  useModifyAndRetryApproval,
} from "../lib/mutations/approvals";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { ErrorState } from "../components/ui/ErrorState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { ResponsiveTable, type ResponsiveTableColumn } from "../components/ui/ResponsiveTable";
import { Badge } from "../components/ui/Badge";
import { useUIStore } from "../lib/store";
import { CheckCircle, XCircle, Clock, MessageSquare, ChevronLeft, ChevronRight } from "lucide-react";

const AUDIT_PAGE_SIZE = 20;

const TOTP_REGEX = /^\d{6}$/;
const RECOVERY_REGEX = /^\d{4}-\d{4}$/;

function isValidTotpOrRecovery(v: string) {
  return TOTP_REGEX.test(v) || RECOVERY_REGEX.test(v);
}

type Tab = "pending" | "audit";
type PendingTarget = { kind: "item"; id: string } | { kind: "batch" } | null;

function tabClass(_tab: Tab, isActive: boolean) {
  return `px-4 py-2 text-sm font-bold rounded-lg transition-colors ${
    isActive
      ? "bg-primary/10 text-primary border border-primary/20"
      : "text-text-dim hover:text-text-main hover:bg-surface-hover border border-transparent"
  }`;
}

function statusBadge(status: string | undefined, t: (key: string) => string) {
  switch (status) {
    case "approved":
      return <Badge variant="success">{t("approvals.status.approved")}</Badge>;
    case "rejected":
      return <Badge variant="error">{t("approvals.status.rejected")}</Badge>;
    case "expired":
      return <Badge variant="default">{t("approvals.status.expired")}</Badge>;
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
  const addToast = useUIStore((s) => s.addToast);
  const modifyAndRetry = useModifyAndRetryApproval();

  async function handleSubmit() {
    if (!feedback.trim()) return;
    try {
      await modifyAndRetry.mutateAsync({ id, feedback: feedback.trim() });
      addToast(t("approvals.modifiedToast"), "success");
      onDone();
    } catch (e: unknown) {
      addToast(e instanceof Error ? e.message : String(e), "error");
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
        className="w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm focus:border-primary focus:ring-2 focus:ring-primary/10 outline-none transition-colors resize-none"
      />
      <div className="flex gap-2 justify-end">
        <Button variant="ghost" size="sm" onClick={onDone}>
          {t("common.cancel", "Cancel")}
        </Button>
        <Button
          variant="primary"
          size="sm"
          onClick={handleSubmit}
          disabled={modifyAndRetry.isPending || !feedback.trim()}
          isLoading={modifyAndRetry.isPending}
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

  const auditQuery = useApprovalAudit({ limit: AUDIT_PAGE_SIZE, offset });

  const entries: ApprovalAuditEntry[] = auditQuery.data?.entries ?? [];
  const total = auditQuery.data?.total ?? 0;
  const from = total === 0 ? 0 : offset + 1;
  const to = Math.min(offset + AUDIT_PAGE_SIZE, total);

  if (auditQuery.isLoading) {
    return <ListSkeleton rows={5} />;
  }

  if (entries.length === 0) {
    return (
      <EmptyState
        icon={<Clock className="w-7 h-7" />}
        title={t("approvals.auditLog.noEntries")}
      />
    );
  }

  const auditColumns = useMemo<ResponsiveTableColumn<ApprovalAuditEntry>[]>(
    () => [
      {
        key: "tool_name",
        label: t("approvals.auditLog.tool"),
        tdClass: "px-4 py-3 font-medium",
        render: (e) => e.tool_name,
      },
      {
        key: "agent_id",
        label: t("approvals.auditLog.agent"),
        tdClass: "px-4 py-3 text-text-dim",
        render: (e) => e.agent_id,
      },
      {
        key: "decision",
        label: t("approvals.auditLog.decision"),
        tdClass: "px-4 py-3",
        render: (e) => decisionBadge(e.decision, t),
      },
      {
        key: "decided_by",
        label: t("approvals.auditLog.decidedBy"),
        tdClass: "px-4 py-3 text-text-dim",
        render: (e) => e.decided_by ?? "—",
      },
      {
        key: "decided_at",
        label: t("approvals.auditLog.decidedAt"),
        tdClass: "px-4 py-3 text-text-dim text-xs",
        render: (e) => (e.decided_at ? new Date(e.decided_at).toLocaleString() : "—"),
      },
      {
        key: "feedback",
        label: t("approvals.auditLog.feedback"),
        tdClass: "px-4 py-3 text-text-dim text-xs max-w-48 truncate",
        render: (e) => e.feedback ?? "—",
      },
    ],
    [t],
  );

  return (
    <div className="flex flex-col gap-4">
      <ResponsiveTable
        columns={auditColumns}
        rows={entries}
        rowKey={(e) => e.id}
      />

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
  const [pendingTarget, setPendingTarget] = useState<PendingTarget>(null);
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [modifyingId, setModifyingId] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<Tab>("pending");
  const [totpPromptId, setTotpPromptId] = useState<string | null>(null);
  const [totpInput, setTotpInput] = useState("");
  const addToast = useUIStore((s) => s.addToast);

  const approvalsQuery = useApprovals();
  const totpQuery = useTotpStatus();
  const approveMutation = useApproveApproval();
  const rejectMutation = useRejectApproval();
  const batchResolve = useBatchResolveApprovals();

  const totpEnforced = totpQuery.data?.enforced ?? false;

  const approvals = approvalsQuery.data ?? [];
  const pendingApprovals = useMemo(
    () => approvals.filter((a) => !a.status || a.status === "pending"),
    [approvals]
  );

  async function handleDecision(id: string, decision: "approve" | "reject") {
    // If TOTP is enforced and user is approving, prompt for code
    if (decision === "approve" && totpEnforced) {
      setTotpPromptId(id);
      setTotpInput("");
      return;
    }
    await executeDecision(id, decision);
  }

  async function handleTotpSubmit() {
    if (!totpPromptId || !isValidTotpOrRecovery(totpInput)) return;
    await executeDecision(totpPromptId, "approve", totpInput);
    setTotpPromptId(null);
    setTotpInput("");
  }

  async function executeDecision(id: string, decision: "approve" | "reject", totpCode?: string) {
    setPendingTarget({ kind: "item", id });
    try {
      if (decision === "approve") {
        await approveMutation.mutateAsync({ id, totpCode });
        addToast(t("approvals.approvedToast"), "success");
      } else {
        await rejectMutation.mutateAsync(id);
        addToast(t("approvals.rejectedToast"), "success");
      }
      setSelected((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    } catch (e: unknown) {
      addToast(e instanceof Error ? e.message : String(e), "error");
    } finally {
      setPendingTarget(null);
    }
  }

  async function handleBatchAction(decision: "approve" | "reject") {
    if (selected.size === 0) return;
    const ids = Array.from(selected);
    setPendingTarget({ kind: "batch" });
    try {
      await batchResolve.mutateAsync({ ids, decision });
      addToast(t("approvals.batchSuccess"), "success");
      setSelected(new Set());
    } catch (e: unknown) {
      addToast(e instanceof Error ? e.message : String(e), "error");
    } finally {
      setPendingTarget(null);
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
        <button className={tabClass("pending", activeTab === "pending")} onClick={() => setActiveTab("pending")}>
          {t("approvals.tabPending")}
          {pendingApprovals.length > 0 && (
            <span className="ml-2 inline-flex h-5 min-w-5 items-center justify-center rounded-full bg-warning/20 px-1.5 text-[10px] font-bold text-warning">
              {pendingApprovals.length}
            </span>
          )}
        </button>
        <button className={tabClass("audit", activeTab === "audit")} onClick={() => setActiveTab("audit")}>
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
                  className="h-4 w-4 rounded border-border-subtle text-primary focus:ring-primary/30 accent-[var(--color-brand)]"
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
                    disabled={pendingTarget?.kind === "batch" || totpEnforced}
                    isLoading={pendingTarget?.kind === "batch"}
                    title={totpEnforced ? t("approvals.batch_disabled_totp") : undefined}
                  >
                    {t("approvals.approveSelected")}
                  </Button>
                  <Button
                    variant="danger"
                    size="sm"
                    onClick={() => handleBatchAction("reject")}
                    disabled={pendingTarget?.kind === "batch"}
                    isLoading={pendingTarget?.kind === "batch"}
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
            <ErrorState
              message={t("approvals.loadError", "Failed to load approvals. Check your connection.")}
              onRetry={() => approvalsQuery.refetch()}
            />
          ) : approvals.length === 0 ? (
            <EmptyState
              icon={<CheckCircle className="w-7 h-7" />}
              title={t("approvals.queue_clear")}
              description={t("approvals.queue_clear_desc")}
            />
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
                            className="h-4 w-4 rounded border-border-subtle text-primary focus:ring-primary/30 shrink-0 accent-[var(--color-brand)]"
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
                          <Button variant="danger" size="sm" onClick={() => handleDecision(a.id, "reject")} disabled={pendingTarget?.kind === "item" && pendingTarget.id === a.id}>
                            {t("approvals.reject")}
                          </Button>
                          <Button variant="success" size="sm" onClick={() => handleDecision(a.id, "approve")} disabled={pendingTarget?.kind === "item" && pendingTarget.id === a.id}>
                            {t("approvals.approve")}
                          </Button>
                        </div>
                      ) : (
                        <div className="text-sm text-text-dim shrink-0">
                          {t(`approvals.status.${a.status}`)}
                        </div>
                      )}
                    </div>
                    {/* TOTP prompt */}
                    {totpPromptId === a.id && isPending && (
                      <div className="mt-3 flex items-center gap-2">
                        <input
                          type="text"
                          maxLength={9}
                          value={totpInput}
                          onChange={(e) => setTotpInput(e.target.value.replace(/[^0-9-]/g, "").slice(0, 9))}
                          placeholder={t("approvals.totpPlaceholder", { defaultValue: "000000 / 0000-0000" })}
                          className="w-40 rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm font-mono tracking-widest text-center focus:border-primary focus:ring-2 focus:ring-primary/10 outline-none transition-colors"
                          autoFocus
                          onKeyDown={(e) => e.key === "Enter" && handleTotpSubmit()}
                        />
                        <Button
                          variant="success"
                          size="sm"
                          onClick={handleTotpSubmit}
                          disabled={!isValidTotpOrRecovery(totpInput) || (pendingTarget?.kind === "item" && pendingTarget.id === a.id)}
                          isLoading={pendingTarget?.kind === "item" && pendingTarget.id === a.id}
                        >
                          {t("approvals.approve")}
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => { setTotpPromptId(null); setTotpInput(""); }}
                        >
                          {t("common.cancel", "Cancel")}
                        </Button>
                        <span className="text-xs text-text-dim">{t("approvals.totpLabel", { defaultValue: "TOTP" })}</span>
                      </div>
                    )}
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
