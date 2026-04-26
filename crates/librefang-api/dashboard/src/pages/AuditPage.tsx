// Audit-trail viewer (RBAC M5 + M6).
//
// Admin-only. Filters narrow the in-memory window (server hard cap 5000
// rows, default 200) — for deeper history use the export button which hits
// /api/audit/export with the same filter set.

import { useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
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
} from "lucide-react";

import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge, type BadgeVariant } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { Select } from "../components/ui/Select";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { useAuditQuery } from "../lib/queries/audit";
import { ApiError } from "../lib/http/errors";
import { formatRelativeTime } from "../lib/datetime";
import type { AuditQueryFilters } from "../lib/http/client";

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

const ACTION_OPTIONS: { value: string; label: string }[] = [
  { value: "", label: "(any)" },
  { value: "ToolInvoke", label: "ToolInvoke" },
  { value: "ShellExec", label: "ShellExec" },
  { value: "UserLogin", label: "UserLogin" },
  { value: "RoleChange", label: "RoleChange" },
  { value: "PermissionDenied", label: "PermissionDenied" },
  { value: "BudgetExceeded", label: "BudgetExceeded" },
  { value: "ConfigChange", label: "ConfigChange" },
  { value: "AgentSpawn", label: "AgentSpawn" },
  { value: "AgentKill", label: "AgentKill" },
  { value: "AgentMessage", label: "AgentMessage" },
  { value: "MemoryAccess", label: "MemoryAccess" },
  { value: "FileAccess", label: "FileAccess" },
  { value: "NetworkAccess", label: "NetworkAccess" },
  { value: "AuthAttempt", label: "AuthAttempt" },
  { value: "WireConnect", label: "WireConnect" },
  { value: "CapabilityCheck", label: "CapabilityCheck" },
  { value: "DreamConsolidation", label: "DreamConsolidation" },
  { value: "RetentionTrim", label: "RetentionTrim" },
];

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
  const [draft, setDraft] = useState<AuditQueryFilters>({ limit: 200 });
  const [active, setActive] = useState<AuditQueryFilters>({ limit: 200 });
  const [exportError, setExportError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const [filtersOpen, setFiltersOpen] = useState(false);

  // Normalise from/to so the server's RFC-3339 parser doesn't 400 on
  // the bare datetime-local format. Same for export URL.
  const query = useAuditQuery(normaliseFilters(active));

  const onApply = (e: React.FormEvent) => {
    e.preventDefault();
    setActive(draft);
  };

  const onClearAll = () => {
    const reset: AuditQueryFilters = { limit: 200 };
    setDraft(reset);
    setActive(reset);
  };

  const onExport = async () => {
    setExportError(null);
    setExporting(true);
    try {
      await downloadExport(active, "csv");
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
    if (active.user) entries.push({ key: "user", label: t("audit.f_user", "User"), value: active.user });
    if (active.action) entries.push({ key: "action", label: t("audit.f_action", "Action"), value: active.action });
    if (active.agent) entries.push({ key: "agent", label: t("audit.f_agent", "Agent"), value: active.agent });
    if (active.channel) entries.push({ key: "channel", label: t("audit.f_channel", "Channel"), value: active.channel });
    if (active.from) entries.push({ key: "from", label: t("audit.f_from", "From"), value: active.from });
    if (active.to) entries.push({ key: "to", label: t("audit.f_to", "To"), value: active.to });
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
        title={t("audit.title", "Audit trail")}
        subtitle={t(
          "audit.subtitle",
          "Searchable, filterable audit log across users / actions / agents.",
        )}
        isFetching={query.isFetching}
        onRefresh={() => void query.refetch()}
        helpText={t(
          "audit.help",
          "Hash-chained tamper-evident log of every privileged action. Filters narrow the in-memory window (server hard cap 5000). Use Export for deeper history or to take the chain offline for verification.",
        )}
        actions={
          <div className="flex items-center gap-2">
            {query.data && (
              <Badge variant="brand" dot>
                {totalCount} / {totalLimit}
              </Badge>
            )}
            <Button
              variant="secondary"
              size="sm"
              leftIcon={<Download className="h-3.5 w-3.5" />}
              onClick={onExport}
              disabled={exporting || isForbidden}
            >
              {exporting
                ? t("audit.exporting", "Exporting…")
                : t("audit.export_csv", "Export CSV")}
            </Button>
          </div>
        }
      />

      {exportError && (
        <Card padding="md">
          <div className="flex items-start gap-3 text-sm text-error">
            <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" />
            <div className="flex-1 min-w-0">
              <p className="font-bold text-xs uppercase tracking-wider">
                {t("audit.export_error_title", "Export failed")}
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

      {/* Filter bar — collapsible; chips show what's active when closed */}
      <Card padding="md">
        <div className="flex items-center gap-3 flex-wrap">
          <button
            type="button"
            onClick={() => setFiltersOpen((v) => !v)}
            className="inline-flex items-center gap-1.5 rounded-xl border border-border-subtle bg-main/40 px-3 py-1.5 text-xs font-bold text-text-main hover:border-brand/30 hover:text-brand transition-colors"
            aria-expanded={filtersOpen}
          >
            <Filter className="h-3.5 w-3.5" />
            {t("audit.filters", "Filters")}
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
              {t("audit.clear_all", "Clear all")}
            </button>
          )}
        </div>

        {filtersOpen && (
          <form onSubmit={onApply} className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3 mt-4">
            <Input
              label={t("audit.f_user", "User")}
              value={draft.user ?? ""}
              onChange={(e) =>
                setDraft((d) => ({ ...d, user: e.target.value || undefined }))
              }
              placeholder={t("audit.f_user_placeholder", "UUID or name")}
              leftIcon={<Users className="h-3.5 w-3.5" />}
            />
            <Select
              label={t("audit.f_action", "Action")}
              value={draft.action ?? ""}
              onChange={(e) =>
                setDraft((d) => ({
                  ...d,
                  action: e.target.value || undefined,
                }))
              }
              options={ACTION_OPTIONS}
            />
            <Input
              label={t("audit.f_agent", "Agent")}
              value={draft.agent ?? ""}
              onChange={(e) =>
                setDraft((d) => ({ ...d, agent: e.target.value || undefined }))
              }
              placeholder={t("audit.f_agent_placeholder", "agent id")}
              leftIcon={<Activity className="h-3.5 w-3.5" />}
            />
            <Input
              label={t("audit.f_channel", "Channel")}
              value={draft.channel ?? ""}
              onChange={(e) =>
                setDraft((d) => ({
                  ...d,
                  channel: e.target.value || undefined,
                }))
              }
              placeholder={t("audit.f_channel_placeholder", "api / telegram / …")}
              leftIcon={<Plug className="h-3.5 w-3.5" />}
            />
            <Input
              label={t("audit.f_from", "From")}
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
              label={t("audit.f_to", "To")}
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
                {t("audit.reset", "Reset")}
              </Button>
              <Button type="submit" size="sm" leftIcon={<Search className="h-3.5 w-3.5" />}>
                {t("audit.apply", "Apply filters")}
              </Button>
            </div>
          </form>
        )}
      </Card>

      {isForbidden && (
        <Card padding="lg">
          <div className="flex items-start gap-3">
            <div className="rounded-xl bg-error/10 text-error p-2 shrink-0">
              <ShieldAlert className="h-5 w-5" />
            </div>
            <div className="flex-1 min-w-0">
              <p className="text-sm font-black tracking-tight">
                {t("audit.forbidden_title", "Admin role required")}
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
                {t("audit.error_title", "Failed to load audit log")}
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
          title={t("audit.empty_title", "No matching audit entries")}
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
                {t("audit.clear_all", "Clear all")}
              </Button>
            ) : undefined
          }
        />
      ) : query.data ? (
        <div className="space-y-2 stagger-children">
          {query.data.entries.map((e) => {
            const variant = outcomeVariant(e.outcome);
            const fullTimestamp = e.timestamp;
            const relTime = formatRelativeTime(e.timestamp);
            return (
              <div
                key={`${e.seq}-${e.hash}`}
                className="flex items-start gap-3 p-3 sm:p-4 rounded-xl sm:rounded-2xl border border-border-subtle bg-surface hover:border-brand/30 hover:-translate-y-0.5 transition-all duration-200 shadow-sm"
              >
                {/* Action chip */}
                <div
                  className={`shrink-0 inline-flex items-center gap-1.5 rounded-lg border px-2 py-1 text-[10px] font-black uppercase tracking-wider ${actionChipClass(e.outcome)}`}
                  title={e.action}
                >
                  {actionIcon(e.action)}
                  <span className="hidden sm:inline">{e.action}</span>
                </div>

                {/* Body */}
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="sm:hidden text-xs font-bold">{e.action}</span>
                    <Badge variant={variant} dot>
                      {e.outcome}
                    </Badge>
                    {e.user_id && (
                      <span className="inline-flex items-center gap-1 text-[10px] text-text-dim">
                        <Users className="h-3 w-3" />
                        <span className="font-mono">{e.user_id}</span>
                      </span>
                    )}
                    {e.channel && (
                      <span className="inline-flex items-center gap-1 text-[10px] text-text-dim">
                        <Plug className="h-3 w-3" />
                        {e.channel}
                      </span>
                    )}
                    {e.agent_id && e.agent_id !== "system" && (
                      <span className="inline-flex items-center gap-1 text-[10px] text-text-dim">
                        <Activity className="h-3 w-3" />
                        <span className="font-mono">{e.agent_id}</span>
                      </span>
                    )}
                    <span className="inline-flex items-center gap-1 text-[10px] text-text-dim/70 font-mono">
                      #{e.seq}
                    </span>
                  </div>
                  {e.detail && (
                    <p className="mt-1 text-xs text-text-main/90 break-words leading-relaxed">
                      {e.detail}
                    </p>
                  )}
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
      ) : null}
    </div>
  );
}
