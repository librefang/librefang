// Audit-trail viewer (RBAC M5 + M6).
//
// Admin-only. Filters narrow the in-memory window (server hard cap 5000
// rows, default 200) — for deeper history use the export button which hits
// /api/audit/export with the same filter set.

import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useSearch } from "@tanstack/react-router";
import {
  ScrollText,
  Download,
  AlertTriangle,
  Search,
  ShieldOff,
  ShieldAlert,
  Wrench,
  Terminal,
  LogIn,
  Users,
  DollarSign,
  Settings,
  Plus,
  X as XIcon,
  MessageCircle,
  Brain,
  FileText,
  Globe,
  Key,
  Plug,
  Moon,
  Scissors,
  ShieldCheck,
  Activity,
  Clock,
  RotateCcw,
  Filter,
  ChevronDown,
  ChevronUp,
  Copy,
  Hash,
  Link2,
  FileJson,
} from "lucide-react";

import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge, type BadgeVariant } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { Select } from "../components/ui/Select";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Modal } from "../components/ui/Modal";
import { useAuditQuery } from "../lib/queries/audit";
import { ApiError } from "../lib/http/errors";
import { formatRelativeTime } from "../lib/datetime";
import type { AuditQueryFilters } from "../lib/http/client";
import type { AuditQueryEntry } from "../api";
import { useUIStore } from "../lib/store";

// `<input type="datetime-local">` produces "YYYY-MM-DDTHH:MM" with no
// timezone. The server parses `from` / `to` as RFC-3339 (offset
// required), so we must normalise to ISO-8601 with `Z` before sending
// — otherwise the server returns 400 and the filter silently fails.
// Treats the input as the user's local time (matches what the picker
// displays) and converts to UTC.
function toRfc3339(local: string | undefined): string | undefined {
  if (!local) return undefined;
  const d = new Date(local);
  if (Number.isNaN(d.getTime())) return undefined;
  return d.toISOString();
}

function normaliseFilters(filters: AuditQueryFilters): AuditQueryFilters {
  return {
    ...filters,
    from: toRfc3339(filters.from),
    to: toRfc3339(filters.to),
  };
}

function buildExportUrl(
  filters: AuditQueryFilters,
  format: "csv" | "json",
): string {
  const normalised = normaliseFilters(filters);
  const params = new URLSearchParams({ format });
  for (const [k, v] of Object.entries(normalised)) {
    if (v === undefined || v === null || v === "") continue;
    params.set(k, String(v));
  }
  return `/api/audit/export?${params.toString()}`;
}

// Authenticated download: dashboard auth is Bearer-in-header, but
// `<a download>` triggers a navigation that drops custom headers, so
// the browser would download the daemon's 401 / login HTML as
// `audit.csv`. Fetch with the Bearer header, materialise the body as
// a Blob, then programmatically click an object-URL anchor.
async function downloadExport(
  filters: AuditQueryFilters,
  format: "csv" | "json",
): Promise<void> {
  const url = buildExportUrl(filters, format);
  const token = localStorage.getItem("librefang-api-key") || "";
  const headers: Record<string, string> = {};
  if (token) headers["Authorization"] = `Bearer ${token}`;
  const resp = await fetch(url, { headers });
  if (!resp.ok) {
    throw await ApiError.fromResponse(resp);
  }
  const blob = await resp.blob();
  const objectUrl = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = objectUrl;
  a.download = `audit.${format}`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  // Defer revoke so the browser has a chance to start the save dialog.
  setTimeout(() => URL.revokeObjectURL(objectUrl), 1000);
}

// Action enum identifiers — these are the literal `AuditAction` variant
// names the server expects in the URL query, so the *values* are not
// translatable. The `(any)` label for the empty option *is* — see the
// `actionOptions` memo inside the component.
const ACTION_VALUES = [
  "ToolInvoke",
  "ShellExec",
  "UserLogin",
  "RoleChange",
  "PermissionDenied",
  "BudgetExceeded",
  "ConfigChange",
  "AgentSpawn",
  "AgentKill",
  "AgentMessage",
  "MemoryAccess",
  "FileAccess",
  "NetworkAccess",
  "AuthAttempt",
  "WireConnect",
  "CapabilityCheck",
  "DreamConsolidation",
  "RetentionTrim",
] as const;

// Visual mapping for the action column. Keep this exhaustive on the
// known variants — the server's `AuditAction` enum is append-only and a
// missing variant falls through to `Activity` so a new server-side
// action shows up generically rather than crashing the row.
function actionIcon(action: string): ReactNode {
  switch (action) {
    case "ToolInvoke":
      return <Wrench className="h-3.5 w-3.5" />;
    case "ShellExec":
      return <Terminal className="h-3.5 w-3.5" />;
    case "UserLogin":
      return <LogIn className="h-3.5 w-3.5" />;
    case "RoleChange":
      return <Users className="h-3.5 w-3.5" />;
    case "PermissionDenied":
      return <ShieldOff className="h-3.5 w-3.5" />;
    case "BudgetExceeded":
      return <DollarSign className="h-3.5 w-3.5" />;
    case "ConfigChange":
      return <Settings className="h-3.5 w-3.5" />;
    case "AgentSpawn":
      return <Plus className="h-3.5 w-3.5" />;
    case "AgentKill":
      return <XIcon className="h-3.5 w-3.5" />;
    case "AgentMessage":
      return <MessageCircle className="h-3.5 w-3.5" />;
    case "MemoryAccess":
      return <Brain className="h-3.5 w-3.5" />;
    case "FileAccess":
      return <FileText className="h-3.5 w-3.5" />;
    case "NetworkAccess":
      return <Globe className="h-3.5 w-3.5" />;
    case "AuthAttempt":
      return <Key className="h-3.5 w-3.5" />;
    case "WireConnect":
      return <Plug className="h-3.5 w-3.5" />;
    case "CapabilityCheck":
      return <ShieldCheck className="h-3.5 w-3.5" />;
    case "DreamConsolidation":
      return <Moon className="h-3.5 w-3.5" />;
    case "RetentionTrim":
      return <Scissors className="h-3.5 w-3.5" />;
    default:
      return <Activity className="h-3.5 w-3.5" />;
  }
}

function outcomeVariant(outcome: string): BadgeVariant {
  if (outcome === "ok") return "success";
  if (outcome === "denied") return "error";
  if (outcome === "error") return "warning";
  return "default";
}

// Dim/accent the action chip itself based on outcome — denied actions
// read red even before the eye reaches the outcome badge on the right.
function actionChipClass(outcome: string): string {
  if (outcome === "denied") return "bg-error/10 text-error border-error/20";
  if (outcome === "error") return "bg-warning/10 text-warning border-warning/20";
  return "bg-brand/10 text-brand border-brand/20";
}

// UserId / agent_id are full UUIDs — `f47ac10b-58cc-4372-a567-0e02b2c3d479`
// is 36 chars and dominates the secondary metadata line. Render as
// first 8 + last 4, which keeps the entropy operators actually use to
// disambiguate while halving the visual weight.
function truncateUuid(s: string): string {
  if (s.length <= 16) return s;
  return `${s.slice(0, 8)}…${s.slice(-4)}`;
}

// Bucket label for grouping rows under a date header. "Today" /
// "Yesterday" use the local clock; older days use the locale's date
// short format. Pure function of the row's RFC-3339 timestamp; falls
// back to a localised "Unknown" if parsing fails (kept as its own
// bucket so the operator notices a corrupt timestamp instead of silent
// absorption into Today). `t` is passed through so the function stays
// pure / testable rather than reaching into a hook from the helper.
type Translator = (key: string) => string;
function dateBucketLabel(timestamp: string, t: Translator): string {
  const d = new Date(timestamp);
  if (Number.isNaN(d.getTime())) return t("audit.unknown_date");
  const now = new Date();
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const startOfYesterday = new Date(startOfToday.getTime() - 86_400_000);
  if (d >= startOfToday) return t("audit.today");
  if (d >= startOfYesterday) return t("audit.yesterday");
  return d.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

// Group rows into [bucket, entries[]] pairs while preserving the
// server-side ordering (newest first). Stable: a contiguous run of
// rows for the same bucket becomes one group; we never reorder across
// buckets, so the visual reads top-down chronologically.
function groupByDate(
  entries: AuditQueryEntry[],
  t: Translator,
): { label: string; rows: AuditQueryEntry[] }[] {
  const groups: { label: string; rows: AuditQueryEntry[] }[] = [];
  for (const e of entries) {
    const label = dateBucketLabel(e.timestamp, t);
    const last = groups[groups.length - 1];
    if (last && last.label === label) {
      last.rows.push(e);
    } else {
      groups.push({ label, rows: [e] });
    }
  }
  return groups;
}

// Per-group outcome tally for the date-header strip. Rendered as
// coloured dot+count chips so the operator can see "this day was 12
// ok / 3 denied" without scrolling the whole bucket. Intentionally
// flat counts only — finer-grained breakdowns (per-action) live behind
// the existing filter chips.
function outcomeBreakdown(rows: AuditQueryEntry[]): {
  ok: number;
  denied: number;
  error: number;
  other: number;
} {
  const tally = { ok: 0, denied: 0, error: 0, other: 0 };
  for (const r of rows) {
    if (r.outcome === "ok") tally.ok += 1;
    else if (r.outcome === "denied") tally.denied += 1;
    else if (r.outcome === "error") tally.error += 1;
    else tally.other += 1;
  }
  return tally;
}

// Long-detail heuristic: any string that exceeds DETAIL_CLAMP_CHARS or
// contains a newline gets rendered with line-clamp-2 + "Show more" so
// a 4KB JSON blob doesn't push the next 30 rows below the fold. The
// threshold is conservative enough that one-line file paths and
// "denied: …" strings stay fully visible.
const DETAIL_CLAMP_CHARS = 200;
function shouldClampDetail(detail: string): boolean {
  return detail.length > DETAIL_CLAMP_CHARS || detail.includes("\n");
}

// Quick-pick presets for the `from` filter. Apply via a click — sets
// `draft.from` to `now − N` and clears `draft.to`, so the operator
// gets "everything in the last hour / today / last 24h / last 7 days"
// in one tap instead of fighting the native datetime-local picker.
// The label is shown verbatim; `since` returns the wall-clock instant
// to feed into a `<input type="datetime-local">` value (local-time
// string, no timezone — `toRfc3339` normalises before send).
interface DatePreset {
  key: string;
  label: string;
  since: () => string;
}
function localDatetimeInput(d: Date): string {
  // YYYY-MM-DDTHH:MM in local time (the format datetime-local expects).
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
const DATE_PRESETS: DatePreset[] = [
  {
    key: "1h",
    label: "1h",
    since: () => localDatetimeInput(new Date(Date.now() - 3_600_000)),
  },
  {
    key: "24h",
    label: "24h",
    since: () => localDatetimeInput(new Date(Date.now() - 86_400_000)),
  },
  {
    key: "today",
    // Translated at render via the `audit.today` key — the literal
    // here is only the fallback / readable identifier for the preset.
    label: "Today",
    since: () => {
      const d = new Date();
      d.setHours(0, 0, 0, 0);
      return localDatetimeInput(d);
    },
  },
  {
    key: "7d",
    label: "7d",
    since: () => localDatetimeInput(new Date(Date.now() - 7 * 86_400_000)),
  },
  {
    key: "30d",
    label: "30d",
    since: () => localDatetimeInput(new Date(Date.now() - 30 * 86_400_000)),
  },
];

// Active-filter chips for the collapsed-but-active state: shows what the
// operator is currently filtering by without forcing the form open. Each
// chip strips its own field on click.
interface ActiveChipProps {
  label: string;
  value: string;
  onClear: () => void;
}
function ActiveChip({ label, value, onClear }: ActiveChipProps) {
  return (
    <button
      type="button"
      onClick={onClear}
      className="group inline-flex items-center gap-1.5 rounded-lg border border-brand/20 bg-brand/5 px-2 py-0.5 text-[10px] font-bold text-brand hover:border-error/30 hover:bg-error/10 hover:text-error transition-colors"
    >
      <span className="uppercase tracking-wider text-text-dim group-hover:text-error/70">
        {label}
      </span>
      <span className="font-mono normal-case tracking-normal">{value}</span>
      <XIcon className="h-3 w-3 opacity-50 group-hover:opacity-100" />
    </button>
  );
}

export function AuditPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  // URL search params drive the initial filter state and the optional
  // `?seq=N` deep-link to a specific entry's detail modal. The page
  // writes back to the URL whenever `active` changes so a bookmark /
  // shared link round-trips.
  const search = useSearch({ from: "/audit" }) as {
    user?: string;
    action?: string;
    agent?: string;
    channel?: string;
    from?: string;
    to?: string;
    limit?: number;
    seq?: number;
  };
  const initialFilters: AuditQueryFilters = useMemo(
    () => ({
      user: search.user,
      action: search.action,
      agent: search.agent,
      channel: search.channel,
      from: search.from,
      to: search.to,
      limit: search.limit ?? 200,
    }),
    // Initial only — subsequent URL changes flow OUT (active → URL),
    // not in. Reading `search` reactively here would create a loop with
    // the sync effect below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );
  const [draft, setDraft] = useState<AuditQueryFilters>(initialFilters);
  const [active, setActive] = useState<AuditQueryFilters>(initialFilters);
  const [exportError, setExportError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const [filtersOpen, setFiltersOpen] = useState(false);
  // The audit entry currently shown in the detail modal. Lifted above
  // the row map so the `?seq=` URL deep-link can pre-open it without
  // racing with the row click handler.
  const [detailEntry, setDetailEntry] = useState<AuditQueryEntry | null>(null);
  const [copiedField, setCopiedField] = useState<string | null>(null);
  const addToast = useUIStore((s) => s.addToast);
  // Per-row detail-expansion. Keyed by `${seq}-${hash}` (same as row key).
  // We default to "clamped" for any row with shouldClampDetail(detail);
  // Show more flips it for that one row only.
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const toggleExpanded = (key: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  // Normalise from/to so the server's RFC-3339 parser doesn't 400 on
  // the bare datetime-local format. Same for export URL.
  // NOTE: must be declared before any hook that reads `query.data` —
  // `useMemo` bodies run synchronously on first render and would hit
  // a TDZ ReferenceError if `query` were declared below them.
  const query = useAuditQuery(normaliseFilters(active));

  // Action options for the Select — the empty-value "(any)" gets the
  // localised label; the rest are pinned to their server-side enum
  // names. Memo'd because Select shallow-compares its `options` prop
  // and we don't want to re-render the children every keystroke.
  const actionOptions = useMemo(
    () => [
      { value: "", label: t("audit.any") },
      ...ACTION_VALUES.map((v) => ({ value: v, label: v })),
    ],
    [t],
  );

  // Channel Select options: a fixed seed of well-known adapter names +
  // any other channel value actually present in the current result set,
  // so the operator can both pick from common ones up-front AND drill
  // into a one-off channel that showed up in the log (e.g. a webhook
  // adapter the seed doesn't list). "(any)" stays the empty-value
  // first option; "Custom…" reveals a free-text input for channels
  // that haven't been recorded yet.
  const channelOptions = useMemo(() => {
    const seed = ["api", "dashboard", "cli", "telegram", "discord", "slack", "matrix", "feishu"];
    const seen = new Set<string>(seed);
    for (const e of query.data?.entries ?? []) {
      if (e.channel) seen.add(e.channel);
    }
    const list = Array.from(seen).sort();
    return [
      { value: "", label: t("audit.any") },
      ...list.map((c) => ({ value: c, label: c })),
      { value: "__custom__", label: t("audit.range_custom") },
    ];
  }, [query.data?.entries, t]);

  // True when the active channel filter doesn't match any known option
  // (operator typed something custom, or filtered via row-click for a
  // channel not in the seed). The Select snaps to "Custom…" and the
  // free-text input below stays visible.
  const channelIsCustom = useMemo(() => {
    if (!draft.channel) return false;
    return !channelOptions.some((o) => o.value === draft.channel);
  }, [draft.channel, channelOptions]);

  const onChannelChange = (value: string) => {
    if (value === "__custom__") {
      // Stay on whatever was typed; if blank, just open the input.
      setDraft((d) => ({ ...d, channel: d.channel ?? "" }));
      return;
    }
    setDraft((d) => ({ ...d, channel: value || undefined }));
  };

  const onApply = (e: React.FormEvent) => {
    e.preventDefault();
    setActive(draft);
  };

  const onClearAll = () => {
    const reset: AuditQueryFilters = { limit: 200 };
    setDraft(reset);
    setActive(reset);
  };

  // Active-filter → URL sync. `replace: true` so each filter tweak
  // doesn't pollute browser history (back button feels broken
  // otherwise — every chip click would be its own entry). `seq` is
  // preserved so an open detail modal stays in the URL while the
  // filters change underneath.
  useEffect(() => {
    const next: Record<string, string | number | undefined> = {
      user: active.user || undefined,
      action: active.action || undefined,
      agent: active.agent || undefined,
      channel: active.channel || undefined,
      from: active.from || undefined,
      to: active.to || undefined,
      // Don't bake the default 200 into the URL — keeps share links
      // clean for the common case.
      limit: active.limit && active.limit !== 200 ? active.limit : undefined,
      seq: detailEntry?.seq,
    };
    navigate({
      to: "/audit",
      // TanStack Router strips undefined keys, so omitted filters
      // round-trip as missing — not as `?user=undefined`.
      search: next as Record<string, unknown>,
      replace: true,
    });
  }, [active, detailEntry?.seq, navigate]);

  // `?seq=N` deep-link: when the page boots with a seq in the URL,
  // wait for the row data and auto-open the matching detail modal.
  // We only consult `search.seq` once (initial value) — subsequent
  // URL writes from the modal-open/close path manage themselves.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const initialSeq = useMemo(() => search.seq, []);
  useEffect(() => {
    if (initialSeq == null || detailEntry) return;
    const match = query.data?.entries.find((e) => e.seq === initialSeq);
    if (match) setDetailEntry(match);
  }, [initialSeq, query.data, detailEntry]);

  // Copy helpers + transient "Copied" affordance. The keyed state lets
  // multiple buttons in the modal each show their own check briefly
  // without tripping over each other.
  const copyToClipboard = async (text: string, fieldKey: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedField(fieldKey);
      setTimeout(() => setCopiedField((cur) => (cur === fieldKey ? null : cur)), 1500);
    } catch (err) {
      addToast(
        err instanceof Error ? err.message : t("audit.error_title"),
        "error",
      );
    }
  };

  const buildPermalink = (entry: AuditQueryEntry): string => {
    const params = new URLSearchParams();
    if (active.user) params.set("user", active.user);
    if (active.action) params.set("action", active.action);
    if (active.agent) params.set("agent", active.agent);
    if (active.channel) params.set("channel", active.channel);
    if (active.from) params.set("from", active.from);
    if (active.to) params.set("to", active.to);
    if (active.limit && active.limit !== 200) params.set("limit", String(active.limit));
    params.set("seq", String(entry.seq));
    const qs = params.toString();
    // Use the dashboard's basepath; fall back to current location host so
    // the copied link is absolute (better UX when pasting into chat).
    return `${window.location.origin}/dashboard/audit${qs ? `?${qs}` : ""}`;
  };

  // Click-to-filter from inside a row. The chip handlers feed this so
  // an operator chasing a thread (`who's the user behind this denial?`,
  // `what else did this agent touch?`) can refine without retyping.
  // Mirrors the active-filter chip semantics — the drilled-in value
  // becomes both the active filter (so the next refetch applies it)
  // and the draft (so the form, when expanded, reflects reality).
  const drillFilter = (key: keyof AuditQueryFilters, value: string) => {
    const next = { ...active, [key]: value };
    setActive(next);
    setDraft(next);
  };

  const onExport = async (format: "csv" | "json") => {
    setExportError(null);
    setExporting(true);
    try {
      await downloadExport(active, format);
    } catch (err) {
      setExportError(
        err instanceof ApiError
          ? `${err.status}: ${err.message}`
          : err instanceof Error
            ? err.message
            : String(err),
      );
    } finally {
      setExporting(false);
    }
  };

  // Status-code check, not text-matching the message: the server's
  // forbidden body is "Admin role required for audit access" today
  // but a future copy edit shouldn't silently regress this banner.
  const isForbidden = query.error instanceof ApiError && query.error.status === 403;

  // What's actually filtering today — drives the chip row + the count
  // badge on the "Filters" toggle. `limit` is excluded because the
  // operator never sees it as a "filter" semantically (it's a page
  // size).
  const activeFilterEntries = useMemo(() => {
    const entries: { key: keyof AuditQueryFilters; label: string; value: string }[] = [];
    if (active.user) entries.push({ key: "user", label: t("audit.f_user"), value: active.user });
    if (active.action) entries.push({ key: "action", label: t("audit.f_action"), value: active.action });
    if (active.agent) entries.push({ key: "agent", label: t("audit.f_agent"), value: active.agent });
    if (active.channel) entries.push({ key: "channel", label: t("audit.f_channel"), value: active.channel });
    if (active.from) entries.push({ key: "from", label: t("audit.f_from"), value: active.from });
    if (active.to) entries.push({ key: "to", label: t("audit.f_to"), value: active.to });
    return entries;
  }, [active, t]);

  const dropFilter = (key: keyof AuditQueryFilters) => {
    const next = { ...active, [key]: undefined };
    setActive(next);
    setDraft(next);
  };

  const totalLimit = query.data?.limit ?? active.limit ?? 200;
  const totalCount = query.data?.count ?? 0;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        icon={<ScrollText className="h-4 w-4" />}
        title={t("audit.title")}
        subtitle={t(
          "audit.subtitle",
          "Searchable, filterable audit log across users / actions / agents.",
        )}
        isFetching={query.isFetching}
        onRefresh={() => void query.refetch()}
        helpText={t("audit.help")}
        actions={
          <div className="flex items-center gap-2">
            {query.data && (
              <Badge variant="brand" dot>
                {totalCount} / {totalLimit}
              </Badge>
            )}
            {/* Split export: CSV (Excel / pandas) and JSON (jq / log
                pipelines). Same filter set, same auth flow — both call
                /api/audit/export with the same params, only `?format=`
                differs. */}
            <div className="inline-flex items-stretch divide-x divide-border-subtle rounded-xl border border-border-subtle overflow-hidden">
              <button
                type="button"
                onClick={() => onExport("csv")}
                disabled={exporting || isForbidden}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-bold text-text-main bg-surface hover:bg-surface-hover hover:text-brand disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                title={t("audit.export_csv")}
              >
                <Download className="h-3.5 w-3.5" />
                {exporting
                  ? t("audit.exporting")
                  : t("audit.export_csv_short")}
              </button>
              <button
                type="button"
                onClick={() => onExport("json")}
                disabled={exporting || isForbidden}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-bold text-text-main bg-surface hover:bg-surface-hover hover:text-brand disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                title={t("audit.export_json")}
              >
                <FileJson className="h-3.5 w-3.5" />
                {t("audit.export_json")}
              </button>
            </div>
          </div>
        }
      />

      {exportError && (
        <Card padding="md">
          <div className="flex items-start gap-3 text-sm text-error">
            <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" />
            <div className="flex-1 min-w-0">
              <p className="font-bold text-xs uppercase tracking-wider">
                {t("audit.export_error_title")}
              </p>
              <p className="mt-1 text-xs font-mono break-all">{exportError}</p>
            </div>
            <button
              type="button"
              onClick={() => setExportError(null)}
              className="text-text-dim hover:text-text-main transition-colors"
              aria-label={t("common.close", { defaultValue: "Close" })}
            >
              <XIcon className="h-4 w-4" />
            </button>
          </div>
        </Card>
      )}

      {/* Filter bar — collapsible; chips show what's active when closed.
          Sticks to the top of the scroll container so it stays usable
          when a long log is open. */}
      <div className="sticky top-0 z-10 -mx-1 px-1 py-1 backdrop-blur supports-[backdrop-filter]:bg-surface/60">
      <Card padding="md">
        <div className="flex items-center gap-3 flex-wrap">
          <button
            type="button"
            onClick={() => setFiltersOpen((v) => !v)}
            className="inline-flex items-center gap-1.5 rounded-xl border border-border-subtle bg-main/40 px-3 py-1.5 text-xs font-bold text-text-main hover:border-brand/30 hover:text-brand transition-colors"
            aria-expanded={filtersOpen}
          >
            <Filter className="h-3.5 w-3.5" />
            {t("audit.filters")}
            {activeFilterEntries.length > 0 && (
              <span className="ml-1 inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-brand px-1 text-[9px] font-black text-white">
                {activeFilterEntries.length}
              </span>
            )}
          </button>
          {activeFilterEntries.length > 0 && !filtersOpen && (
            <div className="flex items-center gap-2 flex-wrap flex-1 min-w-0">
              {activeFilterEntries.map((e) => (
                <ActiveChip
                  key={e.key as string}
                  label={e.label}
                  value={e.value}
                  onClear={() => dropFilter(e.key)}
                />
              ))}
            </div>
          )}
          {activeFilterEntries.length > 0 && (
            <button
              type="button"
              onClick={onClearAll}
              className="inline-flex items-center gap-1 text-[10px] font-bold uppercase tracking-wider text-text-dim hover:text-error transition-colors"
            >
              <RotateCcw className="h-3 w-3" />
              {t("audit.clear_all")}
            </button>
          )}
        </div>

        {filtersOpen && (
          <>
            {/* Datetime quick-picks — apply immediately, save the
                operator from fighting the native datetime-local picker
                for the common "everything in the last hour/day" case. */}
            <div className="flex items-center gap-2 flex-wrap mt-4 pb-3 border-b border-border-subtle">
              <span className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                {t("audit.quick_range")}
              </span>
              {DATE_PRESETS.map((p) => (
                <button
                  key={p.key}
                  type="button"
                  onClick={() => applyDatePreset(p)}
                  className="inline-flex items-center gap-1 rounded-lg border border-border-subtle bg-main/40 px-2 py-1 text-[10px] font-bold text-text-main hover:border-brand/30 hover:text-brand transition-colors"
                >
                  <Clock className="h-3 w-3" />
                  {p.key === "today" ? t("audit.today") : p.label}
                </button>
              ))}
            </div>
          <form onSubmit={onApply} className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3 mt-4">
            <Input
              label={t("audit.f_user")}
              value={draft.user ?? ""}
              onChange={(e) =>
                setDraft((d) => ({ ...d, user: e.target.value || undefined }))
              }
              placeholder={t("audit.f_user_placeholder")}
              leftIcon={<Users className="h-3.5 w-3.5" />}
            />
            <Select
              label={t("audit.f_action")}
              value={draft.action ?? ""}
              onChange={(e) =>
                setDraft((d) => ({
                  ...d,
                  action: e.target.value || undefined,
                }))
              }
              options={actionOptions}
            />
            <Input
              label={t("audit.f_agent")}
              value={draft.agent ?? ""}
              onChange={(e) =>
                setDraft((d) => ({ ...d, agent: e.target.value || undefined }))
              }
              placeholder={t("audit.f_agent_placeholder")}
              leftIcon={<Activity className="h-3.5 w-3.5" />}
            />
            <div className="flex flex-col gap-1.5">
              <Select
                label={t("audit.f_channel")}
                value={channelIsCustom ? "__custom__" : (draft.channel ?? "")}
                onChange={(e) => onChannelChange(e.target.value)}
                options={channelOptions}
              />
              {channelIsCustom && (
                <Input
                  value={draft.channel ?? ""}
                  onChange={(e) =>
                    setDraft((d) => ({
                      ...d,
                      channel: e.target.value || undefined,
                    }))
                  }
                  placeholder={t("audit.f_channel_placeholder")}
                  leftIcon={<Plug className="h-3.5 w-3.5" />}
                />
              )}
            </div>
            <Input
              label={t("audit.f_from")}
              type="datetime-local"
              value={draft.from ?? ""}
              onChange={(e) =>
                setDraft((d) => ({
                  ...d,
                  from: e.target.value || undefined,
                }))
              }
              leftIcon={<Clock className="h-3.5 w-3.5" />}
            />
            <Input
              label={t("audit.f_to")}
              type="datetime-local"
              value={draft.to ?? ""}
              onChange={(e) =>
                setDraft((d) => ({ ...d, to: e.target.value || undefined }))
              }
              leftIcon={<Clock className="h-3.5 w-3.5" />}
            />
            <div className="sm:col-span-2 lg:col-span-3 flex items-center justify-end gap-2 pt-1">
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={onClearAll}
                disabled={activeFilterEntries.length === 0}
              >
                {t("audit.reset")}
              </Button>
              <Button type="submit" size="sm" leftIcon={<Search className="h-3.5 w-3.5" />}>
                {t("audit.apply")}
              </Button>
            </div>
          </form>
          </>
        )}
      </Card>
      </div>

      {isForbidden && (
        <Card padding="lg">
          <div className="flex items-start gap-3">
            <div className="rounded-xl bg-error/10 text-error p-2 shrink-0">
              <ShieldAlert className="h-5 w-5" />
            </div>
            <div className="flex-1 min-w-0">
              <p className="text-sm font-black tracking-tight">
                {t("audit.forbidden_title")}
              </p>
              <p className="mt-1 text-xs text-text-dim leading-relaxed">
                {t(
                  "audit.forbidden_body",
                  "/api/audit/query is admin-only. Sign in with an Admin or Owner api_key.",
                )}
              </p>
            </div>
          </div>
        </Card>
      )}

      {!isForbidden && query.error && (
        <Card padding="lg">
          <div className="flex items-start gap-3">
            <div className="rounded-xl bg-error/10 text-error p-2 shrink-0">
              <AlertTriangle className="h-5 w-5" />
            </div>
            <div className="flex-1 min-w-0">
              <p className="text-sm font-black tracking-tight">
                {t("audit.error_title")}
              </p>
              <p className="mt-1 text-xs text-text-dim font-mono break-all">
                {String(query.error)}
              </p>
            </div>
          </div>
        </Card>
      )}

      {query.isLoading ? (
        <ListSkeleton rows={5} />
      ) : query.data && query.data.entries.length === 0 ? (
        <EmptyState
          icon={<ScrollText className="h-7 w-7" />}
          title={t("audit.empty_title")}
          description={
            activeFilterEntries.length > 0
              ? t(
                  "audit.empty_filtered",
                  "Try widening the filters, or clear them to see the most recent rows.",
                )
              : t(
                  "audit.empty_unfiltered",
                  "Nothing recorded yet. As soon as agents take privileged actions they appear here.",
                )
          }
          action={
            activeFilterEntries.length > 0 ? (
              <Button variant="secondary" size="sm" leftIcon={<RotateCcw className="h-3.5 w-3.5" />} onClick={onClearAll}>
                {t("audit.clear_all")}
              </Button>
            ) : undefined
          }
        />
      ) : query.data ? (
        <div className="flex flex-col gap-4">
          {groupByDate(query.data.entries, t).map((group) => {
            const tally = outcomeBreakdown(group.rows);
            return (
            <section key={group.label} className="flex flex-col gap-2">
              <div className="flex items-center gap-3 px-1">
                <h2 className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                  {group.label}
                </h2>
                <div className="flex-1 h-px bg-border-subtle/60" />
                <div className="flex items-center gap-2 text-[10px] font-bold">
                  {tally.ok > 0 && (
                    <span className="inline-flex items-center gap-1 text-success">
                      <span className="w-1.5 h-1.5 rounded-full bg-success" />
                      {tally.ok}
                    </span>
                  )}
                  {tally.denied > 0 && (
                    <span className="inline-flex items-center gap-1 text-error">
                      <span className="w-1.5 h-1.5 rounded-full bg-error" />
                      {tally.denied}
                    </span>
                  )}
                  {tally.error > 0 && (
                    <span className="inline-flex items-center gap-1 text-warning">
                      <span className="w-1.5 h-1.5 rounded-full bg-warning" />
                      {tally.error}
                    </span>
                  )}
                  {tally.other > 0 && (
                    <span className="inline-flex items-center gap-1 text-text-dim">
                      <span className="w-1.5 h-1.5 rounded-full bg-text-dim/40" />
                      {tally.other}
                    </span>
                  )}
                  <span className="text-text-dim/50 ml-1">·</span>
                  <span className="text-text-dim/70">{group.rows.length}</span>
                </div>
              </div>
              <div className="space-y-2 stagger-children">
                {group.rows.map((e) => {
                  const variant = outcomeVariant(e.outcome);
                  const fullTimestamp = e.timestamp;
                  const relTime = formatRelativeTime(e.timestamp);
                  return (
                    <div
                      key={`${e.seq}-${e.hash}`}
                      role="button"
                      tabIndex={0}
                      onClick={() => setDetailEntry(e)}
                      onKeyDown={(ev) => {
                        if (ev.key === "Enter" || ev.key === " ") {
                          ev.preventDefault();
                          setDetailEntry(e);
                        }
                      }}
                      aria-label={t("audit.open_detail")}
                      className="flex items-start gap-3 p-3 sm:p-4 rounded-xl sm:rounded-2xl border border-border-subtle bg-surface hover:border-brand/30 hover:-translate-y-0.5 transition-all duration-200 shadow-sm cursor-pointer focus:outline-none focus:ring-2 focus:ring-brand/30"
                    >
                      {/* Action chip — click filters by this action.
                          stopPropagation so the row's open-detail click
                          doesn't fire on top of it. */}
                      <button
                        type="button"
                        onClick={(ev) => {
                          ev.stopPropagation();
                          drillFilter("action", e.action);
                        }}
                        className={`shrink-0 inline-flex items-center gap-1.5 rounded-lg border px-2 py-1 text-[10px] font-black uppercase tracking-wider hover:opacity-80 transition-opacity ${actionChipClass(e.outcome)}`}
                        title={t("audit.filter_by_action", { action: e.action })}
                      >
                        {actionIcon(e.action)}
                        <span className="hidden sm:inline">{e.action}</span>
                      </button>

                      {/* Body */}
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2 flex-wrap">
                          <span className="sm:hidden text-xs font-bold">{e.action}</span>
                          <Badge variant={variant} dot>
                            {e.outcome}
                          </Badge>
                          {e.user_id && (
                            <button
                              type="button"
                              onClick={(ev) => {
                                ev.stopPropagation();
                                drillFilter("user", e.user_id!);
                              }}
                              className="inline-flex items-center gap-1 text-[10px] text-text-dim hover:text-brand transition-colors"
                              title={t("audit.filter_by_user")}
                            >
                              <Users className="h-3 w-3" />
                              <span className="font-mono">{truncateUuid(e.user_id)}</span>
                            </button>
                          )}
                          {e.channel && (
                            <button
                              type="button"
                              onClick={(ev) => {
                                ev.stopPropagation();
                                drillFilter("channel", e.channel!);
                              }}
                              className="inline-flex items-center gap-1 text-[10px] text-text-dim hover:text-brand transition-colors"
                              title={t("audit.filter_by_channel")}
                            >
                              <Plug className="h-3 w-3" />
                              {e.channel}
                            </button>
                          )}
                          {e.agent_id && e.agent_id !== "system" && (
                            <button
                              type="button"
                              onClick={(ev) => {
                                ev.stopPropagation();
                                drillFilter("agent", e.agent_id);
                              }}
                              className="inline-flex items-center gap-1 text-[10px] text-text-dim hover:text-brand transition-colors"
                              title={t("audit.filter_by_agent")}
                            >
                              <Activity className="h-3 w-3" />
                              <span className="font-mono">{truncateUuid(e.agent_id)}</span>
                            </button>
                          )}
                          <span
                            className="inline-flex items-center gap-1 text-[10px] text-text-dim/70 font-mono"
                            title={t("audit.hash_tooltip", { hash: e.hash })}
                          >
                            #{e.seq}
                          </span>
                        </div>
                        {e.detail && (() => {
                          const rowKey = `${e.seq}-${e.hash}`;
                          const clamp = shouldClampDetail(e.detail);
                          const isExpanded = expanded.has(rowKey);
                          if (!clamp) {
                            return (
                              <p className="mt-1 text-xs text-text-main/90 break-words leading-relaxed">
                                {e.detail}
                              </p>
                            );
                          }
                          return (
                            <div className="mt-1">
                              <p
                                className={`text-xs text-text-main/90 break-words leading-relaxed whitespace-pre-wrap ${isExpanded ? "" : "line-clamp-2"}`}
                              >
                                {e.detail}
                              </p>
                              <button
                                type="button"
                                onClick={(ev) => {
                                  ev.stopPropagation();
                                  toggleExpanded(rowKey);
                                }}
                                className="mt-1 inline-flex items-center gap-1 text-[10px] font-bold text-brand hover:text-brand/80 transition-colors"
                              >
                                {isExpanded ? (
                                  <>
                                    <ChevronUp className="h-3 w-3" />
                                    {t("audit.show_less")}
                                  </>
                                ) : (
                                  <>
                                    <ChevronDown className="h-3 w-3" />
                                    {t("audit.show_more")}
                                  </>
                                )}
                              </button>
                            </div>
                          );
                        })()}
                      </div>

                      {/* Timestamp */}
                      <div
                        className="shrink-0 flex items-center gap-1 text-[10px] text-text-dim font-mono"
                        title={fullTimestamp}
                      >
                        <Clock className="h-3 w-3" />
                        {relTime}
                      </div>
                    </div>
                  );
                })}
              </div>
            </section>
            );
          })}
        </div>
      ) : null}

      {/* Detail modal — opens on row click and on `?seq=N` deep-link.
          Carries everything the in-line row hides: full RFC-3339
          timestamp, full UUID values, prev/curr hash for chain
          verification, and the unclamped detail payload. */}
      <Modal
        isOpen={detailEntry !== null}
        onClose={() => setDetailEntry(null)}
        title={t("audit.detail_title")}
        size="2xl"
      >
        {detailEntry && (
          <div className="p-5 flex flex-col gap-4">
            {/* Header strip — action chip + outcome badge so the modal
                opens with the same visual identity as the row. */}
            <div className="flex items-center gap-3 flex-wrap">
              <div
                className={`inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-black uppercase tracking-wider ${actionChipClass(detailEntry.outcome)}`}
              >
                {actionIcon(detailEntry.action)}
                {detailEntry.action}
              </div>
              <Badge variant={outcomeVariant(detailEntry.outcome)} dot>
                {detailEntry.outcome}
              </Badge>
              <span className="text-[10px] font-mono text-text-dim/70">
                #{detailEntry.seq}
              </span>
              <div className="ml-auto flex items-center gap-2">
                <button
                  type="button"
                  onClick={() =>
                    copyToClipboard(buildPermalink(detailEntry), "permalink")
                  }
                  className="inline-flex items-center gap-1 rounded-lg border border-border-subtle bg-surface px-2 py-1 text-[10px] font-bold text-text-dim hover:text-brand hover:border-brand/30 transition-colors"
                  title={t("audit.detail_copy_link")}
                >
                  <Link2 className="h-3 w-3" />
                  {copiedField === "permalink"
                    ? t("audit.detail_copied")
                    : t("audit.detail_copy_link")}
                </button>
                <button
                  type="button"
                  onClick={() =>
                    copyToClipboard(
                      JSON.stringify(detailEntry, null, 2),
                      "json",
                    )
                  }
                  className="inline-flex items-center gap-1 rounded-lg border border-border-subtle bg-surface px-2 py-1 text-[10px] font-bold text-text-dim hover:text-brand hover:border-brand/30 transition-colors"
                  title={t("audit.detail_copy_json")}
                >
                  <Copy className="h-3 w-3" />
                  {copiedField === "json"
                    ? t("audit.detail_copied")
                    : t("audit.detail_copy_json")}
                </button>
              </div>
            </div>

            {/* Field grid */}
            <dl className="grid grid-cols-1 sm:grid-cols-[max-content_1fr] gap-x-4 gap-y-2 text-xs">
              <DetailRow label={t("audit.detail_timestamp")}>
                <code className="font-mono text-text-main">
                  {detailEntry.timestamp}
                </code>
                <span className="ml-2 text-text-dim/70">
                  ({formatRelativeTime(detailEntry.timestamp)})
                </span>
              </DetailRow>
              <DetailRow label={t("audit.detail_agent")}>
                <code className="font-mono text-text-main break-all">
                  {detailEntry.agent_id}
                </code>
              </DetailRow>
              {detailEntry.user_id && (
                <DetailRow label={t("audit.detail_user")}>
                  <code className="font-mono text-text-main break-all">
                    {detailEntry.user_id}
                  </code>
                </DetailRow>
              )}
              {detailEntry.channel && (
                <DetailRow label={t("audit.detail_channel")}>
                  <code className="font-mono text-text-main">
                    {detailEntry.channel}
                  </code>
                </DetailRow>
              )}
              <DetailRow label={t("audit.detail_hash")}>
                <div className="flex items-center gap-2 min-w-0">
                  <code className="font-mono text-text-main text-[10px] break-all">
                    {detailEntry.hash}
                  </code>
                  <button
                    type="button"
                    onClick={() => copyToClipboard(detailEntry.hash, "hash")}
                    className="shrink-0 text-text-dim hover:text-brand transition-colors"
                    title={t("audit.detail_copy_hash")}
                  >
                    {copiedField === "hash" ? (
                      <span className="text-[10px] font-bold text-success">
                        {t("audit.detail_copied")}
                      </span>
                    ) : (
                      <Hash className="h-3.5 w-3.5" />
                    )}
                  </button>
                </div>
              </DetailRow>
            </dl>

            {/* Detail payload — preformatted, no clamp. JSON-looking
                strings get a darker code-block treatment so the
                operator sees structure even without syntax highlighting. */}
            {detailEntry.detail && (
              <div className="flex flex-col gap-1.5">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                  {t("audit.detail_payload")}
                </span>
                <pre className="rounded-lg border border-border-subtle bg-main/40 p-3 text-xs text-text-main font-mono whitespace-pre-wrap break-words max-h-72 overflow-y-auto scrollbar-thin">
                  {detailEntry.detail}
                </pre>
              </div>
            )}
          </div>
        )}
      </Modal>
    </div>
  );
}

// Tiny presentational helper for the field grid inside the detail
// modal. Keeps each row to a definition-list pair without repeating
// the dt/dd boilerplate eight times.
function DetailRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-[10px] font-black uppercase tracking-widest text-text-dim pt-0.5">
        {label}
      </dt>
      <dd className="min-w-0">{children}</dd>
    </>
  );
}
