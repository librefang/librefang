import React, { useState, useMemo, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import {
  type ApprovalAuditEntry,
  type ApprovalItem,
  type KnownApprovalDecision,
} from "../api";
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
import { useFullConfig } from "../lib/queries/config";
import { useListNav } from "../lib/useListNav";
import { Skeleton, ListSkeleton } from "../components/ui/Skeleton";
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
  Eye,
  EyeOff,
  HelpCircle,
  History as HistoryIcon,
  Hourglass,
  SkipForward,
  UserCheck,
  Zap,
} from "lucide-react";

const TOTP_REGEX = /^\d{6}$/;
const RECOVERY_REGEX = /^\d{4}-\d{4}$/;

export function isValidTotpOrRecovery(v: string) {
  return TOTP_REGEX.test(v) || RECOVERY_REGEX.test(v);
}

export function sanitizeTotpOrRecovery(value: string): string {
  const filtered = value.replace(/[^0-9-]/g, "");
  const dashIndex = filtered.indexOf("-");
  if (dashIndex < 0) return filtered.slice(0, 6);

  const beforeDash = filtered
    .slice(0, dashIndex)
    .replace(/-/g, "")
    .slice(0, 4);
  const afterDash = filtered
    .slice(dashIndex + 1)
    .replace(/-/g, "")
    .slice(0, 4);
  if (beforeDash.length < 4) return `${beforeDash}${afterDash}`.slice(0, 6);
  return `${beforeDash}-${afterDash}`;
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

function timeAgo(
  iso: string | undefined,
  now: number,
  t: (key: string, opts?: Record<string, unknown>) => string,
): string {
  if (!iso) return "—";
  const ts = new Date(iso).getTime();
  if (!Number.isFinite(ts)) return "—";
  const sec = Math.max(0, Math.floor((now - ts) / 1000));
  if (sec < 60) return t("approvals.timeAgo.seconds", { count: sec });
  const min = Math.floor(sec / 60);
  if (min < 60) return t("approvals.timeAgo.minutes", { count: min });
  const hr = Math.floor(min / 60);
  if (hr < 24) return t("approvals.timeAgo.hours", { count: hr });
  return t("approvals.timeAgo.days", { count: Math.floor(hr / 24) });
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

function EditAndApproveForm({
  isPending,
  onSubmit,
  onCancel,
}: {
  isPending: boolean;
  onSubmit: (feedback: string) => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const [feedback, setFeedback] = useState("");

  function handleSubmit() {
    if (!feedback.trim()) return;
    onSubmit(feedback.trim());
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
        <Button variant="ghost" size="sm" onClick={onCancel} disabled={isPending}>
          {t("common.cancel", "Cancel")}
        </Button>
        <Button
          variant="primary"
          size="sm"
          onClick={handleSubmit}
          disabled={isPending || !feedback.trim()}
          isLoading={isPending}
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
  const [reveal, setReveal] = useState(false);
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
  const maskedRecovery = reveal ? value : "•".repeat(value.length);

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
          <div className="flex gap-1.5 mb-2">
            {digits.map((d, i) => {
              const filled = d.trim().length > 0;
              const isCursor = i === cursorIdx && value.length < 6;
              const display = filled ? (reveal ? d : "•") : isCursor ? "|" : "";
              return (
                <div
                  key={i}
                  className={`flex-1 h-11 grid place-items-center rounded-lg border font-mono text-lg font-semibold transition-colors ${
                    filled
                      ? "border-accent/50 bg-accent/10 text-accent"
                      : "border-border-subtle bg-main/60 text-text-dim"
                  } ${isCursor ? "ring-2 ring-accent/40" : ""}`}
                >
                  {display}
                </div>
              );
            })}
          </div>
        ) : (
          <div className="mb-2 px-3 py-2 rounded-lg border border-accent/30 bg-accent/5 font-mono text-sm tracking-widest text-accent text-center">
            {maskedRecovery}
          </div>
        )}

        <div className="flex justify-end mb-3">
          <button
            type="button"
            onClick={() => setReveal((v) => !v)}
            className="inline-flex items-center gap-1 text-[11px] text-text-dim hover:text-text transition-colors"
            aria-label={reveal ? t("common.hide", "Hide") : t("common.show", "Show")}
            aria-pressed={reveal}
          >
            {reveal ? (
              <>
                <EyeOff className="w-3 h-3" />
                {t("common.hide", "Hide")}
              </>
            ) : (
              <>
                <Eye className="w-3 h-3" />
                {t("common.show", "Show")}
              </>
            )}
          </button>
        </div>

        {/* hidden actual input — captures keystrokes including paste.
            Uses type="password" so OS-level UI (mobile keyboard previews,
            screen readers) treats it as a secret; visual masking is
            handled by the boxes above. */}
        <input
          ref={inputRef}
          type="password"
          value={value}
          onChange={(e) => setValue(sanitizeTotpOrRecovery(e.target.value))}
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
/*  Audit decision presentation                                       */
/* ------------------------------------------------------------------ */

/** Every lucide icon shares one type; anchor to a concrete one. */
type LucideIconComponent = typeof CheckCircle;

type DecisionPresentation = {
  /** i18n key for the visible label. */
  labelKey: string;
  Icon: LucideIconComponent;
  /** Theme variable — resolves per light/dark mode, never a literal hex. */
  color: string;
};

/**
 * How each known `approval_audit.decision` value renders in the History table.
 *
 * Typed as a total `Record` over `KnownApprovalDecision` on purpose (#6607): adding a member to that union without giving it a row here is a compile error, so a newly-emitted backend variant cannot silently inherit another decision's label.
 * Every entry carries its own label text and its own icon — colour is never the sole carrier of the distinction, because an approval audit trail has to be readable without colour perception.
 */
const DECISION_PRESENTATION: Record<KnownApprovalDecision, DecisionPresentation> = {
  approved: {
    labelKey: "approvals.history.decisions.approved",
    Icon: CheckCircle,
    color: "var(--color-success)",
  },
  // Request verb, kept as an alias of `approved` — see `KnownApprovalDecision`.
  approve: {
    labelKey: "approvals.history.decisions.approved",
    Icon: CheckCircle,
    color: "var(--color-success)",
  },
  denied: {
    labelKey: "approvals.history.decisions.denied",
    Icon: XCircle,
    color: "var(--color-error)",
  },
  // `routes/approvals.rs` spelling of `Denied` on sibling shapes.
  rejected: {
    labelKey: "approvals.history.decisions.denied",
    Icon: XCircle,
    color: "var(--color-error)",
  },
  // Request verb, kept as an alias of `denied`.
  reject: {
    labelKey: "approvals.history.decisions.denied",
    Icon: XCircle,
    color: "var(--color-error)",
  },
  // The one decision that genuinely represents an operator edit.
  modify_and_retry: {
    labelKey: "approvals.history.decisions.edited",
    Icon: Edit3,
    color: "var(--color-warning)",
  },
  // Nobody answered before the timeout expired — neutral, not an operator action.
  timed_out: {
    labelKey: "approvals.history.decisions.timedOut",
    Icon: Clock,
    color: "var(--color-text-dim)",
  },
  // Submission row: written before any decision exists, so it is not a completed outcome.
  // `Hourglass` rather than `Loader2` — every other `Loader2` in the dashboard is paired with `animate-spin` to mean "request in flight", and a static spinner glyph repeated down a history table reads as a stalled one.
  pending: {
    labelKey: "approvals.history.decisions.pending",
    Icon: Hourglass,
    color: "var(--color-brand)",
  },
  // Timeout fallback ran the agent on without the tool.
  skipped: {
    labelKey: "approvals.history.decisions.skipped",
    Icon: SkipForward,
    color: "var(--color-accent)",
  },
};

/**
 * Fallback for a value this build does not know.
 *
 * Referenced by identity in `HistoryRow` to decide between the translated label and the raw server value, so it must stay a single shared object.
 */
const UNKNOWN_DECISION: DecisionPresentation = {
  labelKey: "approvals.history.decisions.unknown",
  Icon: HelpCircle,
  color: "var(--color-text-dim)",
};

/**
 * Resolve a raw `decision` string to its presentation.
 *
 * `hasOwnProperty` rather than `in` so a prototype key ("toString", "constructor") arriving from the server cannot resolve to a presentation.
 */
function decisionPresentation(decision: string): DecisionPresentation {
  return Object.prototype.hasOwnProperty.call(DECISION_PRESENTATION, decision)
    ? DECISION_PRESENTATION[decision as KnownApprovalDecision]
    : UNKNOWN_DECISION;
}

/* ------------------------------------------------------------------ */
/*  History row (memoised)                                            */
/* ------------------------------------------------------------------ */

const HistoryRow = React.memo(function HistoryRow({
  h,
  isLast,
  t,
}: {
  h: ApprovalAuditEntry;
  isLast: boolean;
  t: (key: string, opts?: Record<string, unknown>) => string;
}) {
  const risk = normalizeRisk(h.risk_level);
  const decision = h.decision;
  const presentation = decisionPresentation(decision);
  const isKnown = presentation !== UNKNOWN_DECISION;
  const DecisionIcon = presentation.Icon;
  // An unrecognised decision shows the raw server value rather than borrowing another decision's label, so a future backend variant degrades visibly instead of becoming a false record (#6607).
  const decisionLabel = isKnown
    ? t(presentation.labelKey)
    : decision || t(UNKNOWN_DECISION.labelKey);
  const decisionAria = isKnown
    ? t("approvals.history.decisions.aria", { label: t(presentation.labelKey) })
    : t("approvals.history.decisions.unknownAria", { value: decisionLabel });
  const dt = h.decided_at ? new Date(h.decided_at) : null;
  const auto = (h.decided_by ?? "").startsWith("auto");

  return (
    <div
      role="row"
      className={`grid grid-cols-[1fr_80px] lg:grid-cols-[100px_140px_1fr_80px_160px_110px] items-center px-4 py-2.5 text-[12.5px] ${
        isLast ? "" : "border-b border-border-subtle"
      }`}
    >
      <span
        role="cell"
        aria-label={decisionAria}
        title={decisionLabel}
        className="inline-flex min-w-0 items-center gap-1.5 text-[11px] font-bold uppercase tracking-wider"
        style={{ color: presentation.color }}
      >
        <DecisionIcon className="w-3 h-3 shrink-0" aria-hidden="true" />
        <span className="truncate">{decisionLabel}</span>
      </span>

      <span role="cell" className="hidden lg:inline font-mono text-[12px] truncate pr-2">
        {h.agent_id}
      </span>

      <span role="cell" className="hidden lg:inline truncate pr-3">
        {h.action_summary || h.tool_name}
      </span>

      <span
        role="cell"
        className="hidden lg:inline-flex items-center justify-self-start text-[10px] font-bold uppercase tracking-wider px-1.5 py-0.5 rounded border"
        style={{
          background: `color-mix(in oklab, ${riskHex[risk]} 15%, transparent)`,
          borderColor: `color-mix(in oklab, ${riskHex[risk]} 30%, transparent)`,
          color: riskHex[risk],
        }}
      >
        {risk}
      </span>

      <span role="cell" className="hidden lg:inline-flex items-center gap-1 font-mono text-[11px] text-text-dim truncate pr-2">
        {auto ? <Zap className="w-2.5 h-2.5 text-accent" /> : null}
        {h.decided_by ?? "—"}
      </span>

      <span role="cell" className="font-mono text-[11px] text-text-dim text-right">
        {dt
          ? dt.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
          : "—"}
      </span>
    </div>
  );
});

/* ------------------------------------------------------------------ */
/*  History tab                                                       */
/* ------------------------------------------------------------------ */

const HISTORY_PAGE_SIZE = 50;

export function maxHistoryOffset(total: number): number {
  return total > 0
    ? Math.floor((total - 1) / HISTORY_PAGE_SIZE) * HISTORY_PAGE_SIZE
    : 0;
}

function HistoryTab() {
  const { t } = useTranslation();
  const [offset, setOffset] = useState(0);
  const auditQuery = useApprovalAudit({ limit: HISTORY_PAGE_SIZE, offset });
  const entries: ApprovalAuditEntry[] =
    auditQuery.data?.items ?? auditQuery.data?.entries ?? [];
  const total = auditQuery.data?.total ?? 0;
  const from = total === 0 ? 0 : offset + 1;
  const to = Math.min(offset + HISTORY_PAGE_SIZE, total);

  useEffect(() => {
    const maximum = maxHistoryOffset(total);
    if (offset > maximum) setOffset(maximum);
  }, [offset, total]);

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
      <Card padding="none" className="overflow-hidden" role="table" aria-label={t("approvals.tabHistory")}>
        <div role="row" className="hidden lg:grid grid-cols-[100px_140px_1fr_80px_160px_110px] items-center px-4 py-2 border-b border-border-subtle bg-main/40 text-[10px] font-bold uppercase tracking-wider text-text-dim">
          <span role="columnheader">{t("approvals.history.cols.decision")}</span>
          <span role="columnheader">{t("approvals.history.cols.agent")}</span>
          <span role="columnheader">{t("approvals.history.cols.action")}</span>
          <span role="columnheader">{t("approvals.history.cols.risk")}</span>
          <span role="columnheader">{t("approvals.history.cols.resolvedBy")}</span>
          <span role="columnheader" className="text-right">{t("approvals.history.cols.when")}</span>
        </div>
        {entries.map((h, i) => (
          <HistoryRow key={h.id} h={h} isLast={i === entries.length - 1} t={t} />
        ))}
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
  onModifyAndRetry,
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
  onModifyAndRetry: (feedback: string) => void;
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
  const ago = timeAgo(approval.requested_at || approval.created_at, now, t);
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
              {t("approvals.totpBadge")}
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

        {isEditing && (
          <EditAndApproveForm
            isPending={isPending}
            onSubmit={onModifyAndRetry}
            onCancel={onToggleEdit}
          />
        )}
      </div>
    </Card>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  Trusted senders (approval bypass list)                            */
/* ------------------------------------------------------------------ */

/**
 * Narrow the untyped `GET /api/config` body down to `approval.trusted_senders`.
 *
 * The config endpoint is `Record<string, unknown>` by design — it mirrors whatever `KernelConfig` serializes — so the shape is checked here rather than asserted.
 * A malformed section yields an empty list, which renders the same as "nobody is on it"; that is the safe direction to fail, because the alternative is inventing entries that are not configured.
 */
function readTrustedSenders(config: Record<string, unknown> | undefined): string[] {
  if (!config) return [];
  const approval = config["approval"];
  if (typeof approval !== "object" || approval === null) return [];
  const senders = (approval as Record<string, unknown>)["trusted_senders"];
  if (!Array.isArray(senders)) return [];
  return senders.filter((s): s is string => typeof s === "string");
}

/**
 * Read-only audit surface for `approval.trusted_senders` (#6611).
 *
 * A sender on this list skips the approval prompt for every tool the risk classifier does not rank high, so it is the one approval setting whose *populated* state deserves attention — an empty list means every sender goes through the gate, which is the safe configuration and is presented as such.
 * The list is deliberately not editable here: it is excluded from the config write allowlist so that holding an API key is not enough to grant yourself the bypass.
 */
function TrustedSendersCard() {
  const { t } = useTranslation();
  const configQuery = useFullConfig();
  const senders = useMemo(
    () => readTrustedSenders(configQuery.data),
    [configQuery.data],
  );

  return (
    <Card padding="md" className="mb-4">
      <div className="flex items-center gap-2 mb-2">
        <UserCheck className="w-3.5 h-3.5 text-text-dim" aria-hidden="true" />
        <h3 className="m-0 text-[12.5px] font-semibold">
          {t("approvals.trustedSendersTitle", "Trusted senders")}
        </h3>
        {senders.length > 0 && (
          <span className="font-mono text-[10px] px-1.5 py-px rounded-full bg-warning/15 text-warning">
            {senders.length}
          </span>
        )}
      </div>
      <p className="text-[11.5px] text-text-dim mb-2.5">
        {t(
          "approvals.trustedSendersDesc",
          "Senders listed here skip the approval prompt for every tool that is not classified high-risk. Edit the list in config.toml — it is intentionally not writable over the API.",
        )}
      </p>
      {configQuery.isLoading ? (
        <div className="flex gap-2" role="status" aria-busy="true">
          <Skeleton className="h-5 w-24" />
          <Skeleton className="h-5 w-20" />
        </div>
      ) : configQuery.isError ? (
        <ErrorState
          message={t(
            "approvals.trustedSendersLoadError",
            "Could not load the approval configuration.",
          )}
          onRetry={() => configQuery.refetch()}
        />
      ) : senders.length === 0 ? (
        <p className="text-[11.5px] text-success">
          {t(
            "approvals.trustedSendersEmpty",
            "No trusted senders — every sender goes through the approval gate.",
          )}
        </p>
      ) : (
        <ul
          className="flex flex-wrap gap-1.5 list-none m-0 p-0"
          aria-label={t("approvals.trustedSendersTitle", "Trusted senders")}
        >
          {senders.map((sender) => (
            <li key={sender}>
              <code className="inline-block rounded-md border border-warning/20 bg-warning/10 px-1.5 py-0.5 font-mono text-[11px] text-warning break-all">
                {sender}
              </code>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}

/* ------------------------------------------------------------------ */
/*  Main page                                                         */
/* ------------------------------------------------------------------ */

export function ApprovalsPage() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<Tab>("pending");
  const [pendingIds, setPendingIds] = useState<Set<string>>(() => new Set());
  const pendingIdsRef = useRef<Set<string>>(new Set());
  const [editingId, setEditingId] = useState<string | null>(null);
  const [totpFor, setTotpFor] = useState<ApprovalItem | null>(null);
  const [filter, setFilter] = useState("");
  const [filterOpen, setFilterOpen] = useState(false);
  const addToast = useUIStore((s) => s.addToast);

  const approvalsQuery = useApprovals();
  const totpQuery = useTotpStatus();
  const approveMutation = useApproveApproval();
  const rejectMutation = useRejectApproval();
  const modifyAndRetryMutation = useModifyAndRetryApproval();

  const totpEnforced = !totpQuery.isSuccess || totpQuery.data?.enforced !== false;
  const approvals = useMemo(() => approvalsQuery.data ?? [], [approvalsQuery.data]);
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

  function beginDecision(id: string): boolean {
    if (pendingIdsRef.current.has(id)) return false;
    pendingIdsRef.current.add(id);
    setPendingIds(new Set(pendingIdsRef.current));
    return true;
  }

  function finishDecision(id: string) {
    pendingIdsRef.current.delete(id);
    setPendingIds(new Set(pendingIdsRef.current));
  }

  async function executeApprove(id: string, totpCode?: string) {
    if (!beginDecision(id)) return;
    try {
      await approveMutation.mutateAsync({ id, totpCode });
      addToast(t("approvals.approvedToast"), "success");
      setTotpFor(null);
    } catch (e: unknown) {
      addToast(e instanceof Error ? e.message : String(e), "error");
    } finally {
      finishDecision(id);
    }
  }

  async function executeReject(id: string) {
    if (!beginDecision(id)) return;
    try {
      await rejectMutation.mutateAsync(id);
      addToast(t("approvals.rejectedToast"), "success");
    } catch (e: unknown) {
      addToast(e instanceof Error ? e.message : String(e), "error");
    } finally {
      finishDecision(id);
    }
  }

  async function executeModifyAndRetry(id: string, feedback: string) {
    if (!beginDecision(id)) return;
    try {
      await modifyAndRetryMutation.mutateAsync({ id, feedback });
      addToast(t("approvals.modifiedToast"), "success");
      setEditingId(null);
    } catch (e: unknown) {
      addToast(e instanceof Error ? e.message : String(e), "error");
    } finally {
      finishDecision(id);
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
              className="w-full pl-8 pr-3 py-1.5 rounded-lg border border-border-subtle bg-main text-[13px] focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none"
            />
          </div>
        </div>
      )}

      {/* Body */}
      <div className="flex-1 overflow-y-auto p-4 lg:p-5">
        {/* Who currently bypasses this queue.
            Sits above the queue itself because it explains requests that never arrive. */}
        {activeTab === "pending" && <TrustedSendersCard />}
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
                isPending={pendingIds.has(a.id)}
                isEditing={editingId === a.id}
                onApprove={() => handleApprove(a)}
                onDeny={() => void executeReject(a.id)}
                onModifyAndRetry={(feedback) =>
                  void executeModifyAndRetry(a.id, feedback)
                }
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
          pending={pendingIds.has(totpFor.id)}
          onCancel={() => setTotpFor(null)}
          onSubmit={(code) => void executeApprove(totpFor.id, code)}
        />
      )}
    </div>
  );
}
