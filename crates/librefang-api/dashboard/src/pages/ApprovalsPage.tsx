import { useState, useMemo, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import { type ApprovalAuditEntry, type ApprovalItem } from "../api";
import {
  useApprovals,
  useApprovalAudit,
  useTotpStatus,
} from "../lib/queries/approvals";
import {
  useApproveApproval,
  useRejectApproval,
  useModifyAndRetryApproval,
} from "../lib/mutations/approvals";
import { useListNav } from "../lib/useListNav";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { ErrorState } from "../components/ui/ErrorState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { useUIStore } from "../lib/store";
import {
  CheckCircle,
  XCircle,
  Clock,
  Shield,
  ShieldCheck,
  Filter,
  RefreshCw,
  Search,
  Lock,
  Edit3,
  History as HistoryIcon,
  Zap,
} from "lucide-react";

const TOTP_REGEX = /^\d{6}$/;
const RECOVERY_REGEX = /^\d{4}-\d{4}$/;

function isValidTotpOrRecovery(v: string) {
  return TOTP_REGEX.test(v) || RECOVERY_REGEX.test(v);
}

type Tab = "pending" | "history";

/* ------------------------------------------------------------------ */
/*  Risk palette — drives gradient/badges/icons                       */
/* ------------------------------------------------------------------ */

type Risk = "high" | "medium" | "low";

function normalizeRisk(r: string | undefined): Risk {
  const v = (r ?? "").toLowerCase();
  if (v === "high" || v === "critical") return "high";
  if (v === "medium" || v === "moderate") return "medium";
  return "low";
}

const riskHex: Record<Risk, string> = {
  high: "var(--color-error)",
  medium: "var(--color-warning)",
  low: "var(--color-success)",
};

/* ------------------------------------------------------------------ */
/*  Helpers                                                           */
/* ------------------------------------------------------------------ */

function timeAgo(iso: string | undefined, now: number): string {
  if (!iso) return "—";
  const t = new Date(iso).getTime();
  if (!Number.isFinite(t)) return "—";
  const sec = Math.max(0, Math.floor((now - t) / 1000));
  if (sec < 60) return `${sec}s`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h`;
  return `${Math.floor(hr / 24)}d`;
}

function useNow(intervalMs = 30_000) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return now;
}

/* ------------------------------------------------------------------ */
/*  Inline edit-and-approve form                                      */
/* ------------------------------------------------------------------ */

function EditAndApproveForm({ id, onDone }: { id: string; onDone: () => void }) {
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
    <div className="mt-4 flex flex-col gap-2 border-t border-border-subtle pt-4">
      <label className="text-[10px] font-bold uppercase tracking-wider text-text-dim">
        {t("approvals.editApproveTitle")}
      </label>
      <textarea
        value={feedback}
        onChange={(e) => setFeedback(e.target.value)}
        placeholder={t("approvals.modifyPlaceholder")}
        rows={3}
        className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none transition-colors resize-none"
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
          {t("approvals.editApproveSubmit")}
        </Button>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  TOTP modal — full overlay, six visual boxes                       */
/* ------------------------------------------------------------------ */

function TotpModal({
  approval,
  onCancel,
  onSubmit,
  pending,
}: {
  approval: ApprovalItem;
  onCancel: () => void;
  onSubmit: (code: string) => void;
  pending: boolean;
}) {
  const { t } = useTranslation();
  const [value, setValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  const isRecovery = value.includes("-");
  const digits = isRecovery ? null : value.padEnd(6, " ").slice(0, 6).split("");
  const cursorIdx = Math.min(value.length, 5);
  const valid = isValidTotpOrRecovery(value);

  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-black/60 backdrop-blur-md p-5"
      onClick={onCancel}
    >
      <div
        className="animate-rise w-full max-w-sm rounded-2xl border border-accent/40 bg-surface p-5 shadow-[0_24px_60px_-12px_rgba(0,0,0,0.7),0_0_60px_-10px_rgba(167,139,250,0.4)]"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-3 mb-4">
          <div className="grid place-items-center w-9 h-9 rounded-lg bg-accent/15 border border-accent/40 text-accent">
            <Lock className="w-4 h-4" />
          </div>
          <div>
            <div className="text-[14px] font-semibold">
              {t("approvals.totp.modalTitle")}
            </div>
            <div className="text-[11.5px] text-text-dim mt-0.5">
              {t("approvals.totp.modalSubtitle")}
            </div>
          </div>
        </div>

        <p className="text-[12.5px] leading-relaxed text-text-dim mb-4">
          <span className="font-mono">
            {approval.agent_name || approval.agent_id || "agent"}
          </span>{" "}
          {t("approvals.totp.modalContext", {
            action:
              approval.action_summary || approval.action || approval.tool_name || "",
          })}
        </p>

        {digits ? (
          <div className="flex gap-1.5 mb-3">
            {digits.map((d, i) => {
              const filled = d.trim().length > 0;
              const isCursor = i === cursorIdx && value.length < 6;
              return (
                <div
                  key={i}
                  className={`flex-1 h-11 grid place-items-center rounded-lg border font-mono text-lg font-semibold transition-colors ${
                    filled
                      ? "border-accent/50 bg-accent/10 text-accent"
                      : "border-border-subtle bg-main/60 text-text-dim"
                  } ${isCursor ? "ring-2 ring-accent/40" : ""}`}
                >
                  {filled ? d : isCursor ? "|" : ""}
                </div>
              );
            })}
          </div>
        ) : (
          <div className="mb-3 px-3 py-2 rounded-lg border border-accent/30 bg-accent/5 font-mono text-sm tracking-widest text-accent text-center">
            {value}
          </div>
        )}

        {/* hidden actual input — captures keystrokes including paste */}
        <input
          ref={inputRef}
          value={value}
          onChange={(e) =>
            setValue(e.target.value.replace(/[^0-9-]/g, "").slice(0, 9))
          }
          onKeyDown={(e) => {
            if (e.key === "Enter" && valid && !pending) onSubmit(value);
          }}
          inputMode="numeric"
          autoComplete="one-time-code"
          maxLength={9}
          className="sr-only"
          aria-label={t("approvals.totpLabel")}
        />

        <div className="flex items-center gap-1.5 mb-4 text-[11.5px] text-text-dim">
          <Clock className="w-3 h-3" />
          <span>{t("approvals.totp.expiresHint")}</span>
        </div>

        <div className="flex gap-2">
          <Button variant="ghost" size="md" className="flex-1 justify-center" onClick={onCancel}>
            {t("common.cancel", "Cancel")}
          </Button>
          <Button
            variant="success"
            size="md"
            className="flex-1 justify-center"
            leftIcon={<ShieldCheck className="w-4 h-4" />}
            disabled={!valid || pending}
            isLoading={pending}
            onClick={() => onSubmit(value)}
          >
            {t("approvals.totp.confirm")}
          </Button>
        </div>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  History tab                                                       */
/* ------------------------------------------------------------------ */

const HISTORY_PAGE_SIZE = 50;

function HistoryTab() {
  const { t } = useTranslation();
  const [offset, setOffset] = useState(0);
  const auditQuery = useApprovalAudit({ limit: HISTORY_PAGE_SIZE, offset });
  // #3842: prefer canonical `items`; fall back to legacy `entries` for older daemons.
  const entries: ApprovalAuditEntry[] =
    auditQuery.data?.items ?? auditQuery.data?.entries ?? [];
  const total = auditQuery.data?.total ?? 0;
  const from = total === 0 ? 0 : offset + 1;
  const to = Math.min(offset + HISTORY_PAGE_SIZE, total);

  if (auditQuery.isLoading) return <ListSkeleton rows={5} />;
  if (auditQuery.isError) {
    return (
      <ErrorState
        message={t("approvals.loadError")}
        onRetry={() => auditQuery.refetch()}
      />
    );
  }
  if (entries.length === 0) {
    return (
      <EmptyState
        icon={<HistoryIcon className="w-7 h-7" />}
        title={t("approvals.history.empty")}
        description={t("approvals.history.emptyDesc")}
      />
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <Card padding="none" className="overflow-hidden">
        {/* Header row — only visible on lg */}
        <div className="hidden lg:grid grid-cols-[100px_140px_1fr_80px_160px_110px] items-center px-4 py-2 border-b border-border-subtle bg-main/40 text-[10px] font-bold uppercase tracking-wider text-text-dim">
          <span>{t("approvals.history.cols.decision")}</span>
          <span>{t("approvals.history.cols.agent")}</span>
          <span>{t("approvals.history.cols.action")}</span>
          <span>{t("approvals.history.cols.risk")}</span>
          <span>{t("approvals.history.cols.resolvedBy")}</span>
          <span className="text-right">{t("approvals.history.cols.when")}</span>
        </div>
        {entries.map((h, i) => {
          const risk = normalizeRisk(h.risk_level);
          const decision = h.decision;
          const isApprove = decision === "approved" || decision === "approve";
          const isDeny = decision === "rejected" || decision === "reject";
          const decisionColor = isApprove
            ? "var(--color-success)"
            : isDeny
              ? "var(--color-error)"
              : "var(--color-warning)";
          const DecisionIcon = isApprove ? CheckCircle : isDeny ? XCircle : Edit3;
          const decisionLabel = isApprove
            ? t("approvals.history.decisions.approved")
            : isDeny
              ? t("approvals.history.decisions.denied")
              : t("approvals.history.decisions.edited");
          const dt = h.decided_at ? new Date(h.decided_at) : null;
          const auto = (h.decided_by ?? "").startsWith("auto");

          return (
            <div
              key={h.id}
              className={`grid grid-cols-[1fr_80px] lg:grid-cols-[100px_140px_1fr_80px_160px_110px] items-center px-4 py-2.5 text-[12.5px] ${
                i < entries.length - 1 ? "border-b border-border-subtle" : ""
              }`}
            >
              <span
                className="inline-flex items-center gap-1.5 text-[11px] font-bold uppercase tracking-wider"
                style={{ color: decisionColor }}
              >
                <DecisionIcon className="w-3 h-3" />
                {decisionLabel}
              </span>

              <span className="hidden lg:inline font-mono text-[12px] truncate pr-2">
                {h.agent_id}
              </span>

              <span className="hidden lg:inline truncate pr-3">
                {h.action_summary || h.tool_name}
              </span>

              <span
                className="hidden lg:inline-flex items-center justify-self-start text-[10px] font-bold uppercase tracking-wider px-1.5 py-0.5 rounded border"
                style={{
                  background: `color-mix(in oklab, ${riskHex[risk]} 15%, transparent)`,
                  borderColor: `color-mix(in oklab, ${riskHex[risk]} 30%, transparent)`,
                  color: riskHex[risk],
                }}
              >
                {risk}
              </span>

              <span className="hidden lg:inline-flex items-center gap-1 font-mono text-[11px] text-text-dim truncate pr-2">
                {auto ? <Zap className="w-2.5 h-2.5 text-accent" /> : null}
                {h.decided_by ?? "—"}
              </span>

              <span className="font-mono text-[11px] text-text-dim text-right">
                {dt
                  ? dt.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
                  : "—"}
              </span>
            </div>
          );
        })}
      </Card>

      <div className="flex items-center justify-between text-sm text-text-dim">
        <span>{t("approvals.auditLog.showing", { from, to, total })}</span>
        <div className="flex gap-2">
          <Button
            variant="secondary"
            size="sm"
            disabled={offset === 0}
            onClick={() => setOffset(Math.max(0, offset - HISTORY_PAGE_SIZE))}
          >
            {t("common.previous", "Previous")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            disabled={offset + HISTORY_PAGE_SIZE >= total}
            onClick={() => setOffset(offset + HISTORY_PAGE_SIZE)}
          >
            {t("common.next", "Next")}
          </Button>
        </div>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  Pending card                                                      */
/* ------------------------------------------------------------------ */

type CardElementProps = {
  ref: (el: HTMLElement | null) => void;
  tabIndex: number;
  "aria-selected": boolean;
  "data-listnav-index": number;
  onMouseEnter: () => void;
  onClick: () => void;
};

function PendingCard({
  approval,
  totpEnforced,
  isPending,
  onApprove,
  onDeny,
  isEditing,
  onToggleEdit,
  navProps,
  selected,
}: {
  approval: ApprovalItem;
  totpEnforced: boolean;
  isPending: boolean;
  onApprove: () => void;
  onDeny: () => void;
  isEditing: boolean;
  onToggleEdit: () => void;
  navProps: CardElementProps;
  selected: boolean;
}) {
  const { t } = useTranslation();
  const now = useNow(30_000);
  const risk = normalizeRisk(approval.risk_level);
  const color = riskHex[risk];
  const ago = timeAgo(approval.requested_at || approval.created_at, now);
  const action = approval.action_summary || approval.action || approval.tool_name || "—";
  const description = approval.description ?? "";
  const tools = approval.tool_name ? [approval.tool_name] : [];

  return (
    <div
      {...navProps}
      role="option"
      className={`outline-none rounded-2xl transition-shadow ${
        selected ? "ring-2 ring-brand/50" : ""
      }`}
    >
    <Card padding="none" className="overflow-hidden">
      {/* Risk-tinted header */}
      <div
        className="flex items-center gap-2.5 px-3.5 py-2.5 border-b border-border-subtle"
        style={{
          background: `linear-gradient(90deg, color-mix(in oklab, ${color} 8%, transparent), transparent)`,
        }}
      >
        <Shield className="w-3.5 h-3.5 shrink-0" style={{ color }} />
        <span className="font-mono text-[12px] truncate">
          {approval.agent_name || approval.agent_id || "agent"}
        </span>
        <span className="text-[11px] text-text-dim hidden sm:inline">
          {t("approvals.requestedAgo", { ago })}
        </span>
        <span className="ml-auto flex items-center gap-1.5">
          <span
            className="text-[10px] font-bold uppercase tracking-wider px-1.5 py-0.5 rounded border"
            style={{
              background: `color-mix(in oklab, ${color} 15%, transparent)`,
              borderColor: `color-mix(in oklab, ${color} 30%, transparent)`,
              color,
            }}
          >
            {t(`approvals.risk.${risk}`)}
          </span>
          {totpEnforced && (
            <span className="font-mono text-[10px] px-1.5 py-0.5 rounded border border-accent/30 bg-accent/10 text-accent">
              TOTP
            </span>
          )}
        </span>
      </div>

      {/* Body */}
      <div className="p-3.5">
        <div className="text-[13.5px] font-medium mb-1.5 break-words">{action}</div>
        {description && (
          <div className="text-[12.5px] text-text-dim leading-relaxed break-words">
            {description}
          </div>
        )}

        {tools.length > 0 && (
          <div className="flex gap-1.5 flex-wrap my-3">
            {tools.map((tName) => (
              <span
                key={tName}
                className="font-mono text-[10.5px] px-1.5 py-0.5 rounded border border-accent/25 bg-accent/10 text-accent"
              >
                {tName}
              </span>
            ))}
          </div>
        )}

        <div className="flex flex-wrap gap-2 mt-3">
          <Button
            variant="success"
            size="md"
            leftIcon={<CheckCircle className="w-4 h-4" />}
            onClick={onApprove}
            disabled={isPending}
            isLoading={isPending}
          >
            {totpEnforced ? t("approvals.approveWithTotp") : t("approvals.approve")}
          </Button>
          <Button variant="secondary" size="md" onClick={onToggleEdit} disabled={isPending}>
            {t("approvals.editApprove")}
          </Button>
          <Button
            variant="ghost"
            size="md"
            leftIcon={<XCircle className="w-4 h-4" />}
            onClick={onDeny}
            disabled={isPending}
          >
            {t("approvals.deny")}
          </Button>
        </div>

        {isEditing && <EditAndApproveForm id={approval.id} onDone={onToggleEdit} />}
      </div>
    </Card>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  Main page                                                         */
/* ------------------------------------------------------------------ */

export function ApprovalsPage() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<Tab>("pending");
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [totpFor, setTotpFor] = useState<ApprovalItem | null>(null);
  const [filter, setFilter] = useState("");
  const [filterOpen, setFilterOpen] = useState(false);
  const addToast = useUIStore((s) => s.addToast);

  const approvalsQuery = useApprovals();
  const totpQuery = useTotpStatus();
  const approveMutation = useApproveApproval();
  const rejectMutation = useRejectApproval();

  const totpEnforced = totpQuery.data?.enforced ?? false;
  const approvals = approvalsQuery.data ?? [];
  const pendingApprovals = useMemo(
    () => approvals.filter((a) => !a.status || a.status === "pending"),
    [approvals],
  );

  const filteredPending = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return pendingApprovals;
    return pendingApprovals.filter((a) =>
      [a.agent_id, a.agent_name, a.tool_name, a.action_summary, a.description]
        .filter(Boolean)
        .some((s) => (s as string).toLowerCase().includes(q)),
    );
  }, [pendingApprovals, filter]);

  // j/k vim-nav over the visible pending list. Esc closes (in priority
  // order) the TOTP modal → the open filter input → clears row selection.
  const nav = useListNav({
    items: filteredPending,
    disabled: activeTab !== "pending",
    onEscape: () => {
      if (totpFor) setTotpFor(null);
      else if (filterOpen) {
        setFilter("");
        setFilterOpen(false);
      }
    },
  });

  async function executeApprove(id: string, totpCode?: string) {
    setPendingId(id);
    try {
      await approveMutation.mutateAsync({ id, totpCode });
      addToast(t("approvals.approvedToast"), "success");
      setTotpFor(null);
    } catch (e: unknown) {
      addToast(e instanceof Error ? e.message : String(e), "error");
    } finally {
      setPendingId(null);
    }
  }

  async function executeReject(id: string) {
    setPendingId(id);
    try {
      await rejectMutation.mutateAsync(id);
      addToast(t("approvals.rejectedToast"), "success");
    } catch (e: unknown) {
      addToast(e instanceof Error ? e.message : String(e), "error");
    } finally {
      setPendingId(null);
    }
  }

  function handleApprove(a: ApprovalItem) {
    if (totpEnforced) {
      setTotpFor(a);
    } else {
      void executeApprove(a.id);
    }
  }

  return (
    <div className="flex flex-col h-full">
      {/* Top bar */}
      <div className="flex items-center gap-2.5 flex-wrap px-4 lg:px-5 py-3 border-b border-border-subtle">
        <h2 className="m-0 text-[15px] font-semibold">{t("approvals.title")}</h2>
        <span
          className={`inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] font-bold ${
            pendingApprovals.length > 0
              ? "bg-warning/15 text-warning"
              : "bg-success/10 text-success"
          }`}
        >
          {t("approvals.pendingCount", { count: pendingApprovals.length })}
        </span>
        <div className="ml-auto flex items-center gap-1.5">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => approvalsQuery.refetch()}
            leftIcon={
              <RefreshCw
                className={`w-3.5 h-3.5 ${approvalsQuery.isFetching ? "animate-spin" : ""}`}
              />
            }
          >
            <span className="hidden sm:inline">{t("common.refresh", "Refresh")}</span>
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setFilterOpen((v) => !v)}
            leftIcon={<Filter className="w-3.5 h-3.5" />}
          >
            <span className="hidden sm:inline">{t("approvals.filter")}</span>
          </Button>
          <Link
            to="/settings"
            className="inline-flex items-center gap-1.5 rounded-lg border border-border-subtle bg-surface px-2.5 h-7 text-xs font-semibold hover:border-brand/30 hover:text-brand transition-colors"
          >
            {t("approvals.autoRules")}
          </Link>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex gap-0 px-4 lg:px-5 border-b border-border-subtle bg-main/30">
        {([
          { id: "pending", label: t("approvals.tabPending"), count: pendingApprovals.length },
          { id: "history", label: t("approvals.tabHistory"), count: undefined },
        ] as const).map((tDef) => {
          const active = activeTab === tDef.id;
          return (
            <button
              key={tDef.id}
              role="tab"
              aria-selected={active}
              onClick={() => setActiveTab(tDef.id)}
              className={`relative inline-flex items-center gap-2 px-3.5 py-2.5 text-[12.5px] transition-colors ${
                active
                  ? "font-semibold border-b-2 border-brand"
                  : "text-text-dim font-medium border-b-2 border-transparent hover:text-current"
              }`}
            >
              {tDef.id === "pending" ? (
                <Clock className="w-3 h-3" />
              ) : (
                <HistoryIcon className="w-3 h-3" />
              )}
              {tDef.label}
              {tDef.count !== undefined && (
                <span
                  className={`font-mono text-[10px] px-1.5 py-px rounded-full ${
                    active ? "bg-brand/15 text-brand" : "bg-text-dim/10 text-text-dim"
                  }`}
                >
                  {tDef.count}
                </span>
              )}
            </button>
          );
        })}
      </div>

      {/* Filter input — collapses */}
      {filterOpen && activeTab === "pending" && (
        <div className="px-4 lg:px-5 py-2.5 border-b border-border-subtle">
          <div className="relative max-w-md">
            <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-text-dim" />
            <input
              type="text"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder={t("approvals.filterPlaceholder")}
              autoFocus
              className="w-full pl-8 pr-3 py-1.5 rounded-lg border border-border-subtle bg-main text-[13px] focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none"
            />
          </div>
        </div>
      )}

      {/* Body */}
      <div className="flex-1 overflow-y-auto p-4 lg:p-5">
        {activeTab === "history" ? (
          <HistoryTab />
        ) : approvalsQuery.isLoading ? (
          <ListSkeleton rows={3} />
        ) : approvalsQuery.isError ? (
          <ErrorState
            message={t("approvals.loadError")}
            onRetry={() => approvalsQuery.refetch()}
          />
        ) : filteredPending.length === 0 ? (
          <EmptyState
            icon={<CheckCircle className="w-7 h-7" />}
            title={t("approvals.queue_clear")}
            description={
              filter ? t("approvals.noFilterMatch") : t("approvals.queue_clear_desc")
            }
          />
        ) : (
          <div className="flex flex-col gap-3" role="listbox" aria-label={t("approvals.tabPending")}>
            {filteredPending.map((a, i) => (
              <PendingCard
                key={a.id}
                approval={a}
                totpEnforced={totpEnforced}
                isPending={pendingId === a.id}
                isEditing={editingId === a.id}
                onApprove={() => handleApprove(a)}
                onDeny={() => void executeReject(a.id)}
                onToggleEdit={() => setEditingId(editingId === a.id ? null : a.id)}
                navProps={nav.getItemProps(i)}
                selected={nav.selectedIndex === i}
              />
            ))}
          </div>
        )}
      </div>

      {/* TOTP modal */}
      {totpFor && (
        <TotpModal
          approval={totpFor}
          pending={pendingId === totpFor.id}
          onCancel={() => setTotpFor(null)}
          onSubmit={(code) => void executeApprove(totpFor.id, code)}
        />
      )}
    </div>
  );
}
