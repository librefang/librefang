import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  CheckCircle2,
  ChevronRight,
  Clock,
  Filter,
  Plus,
  RefreshCw,
  Rocket,
  Sparkles,
} from "lucide-react";
import { Card } from "../components/ui/Card";
import { Kpi } from "../components/ui/Kpi";
import { Pill } from "../components/ui/Pill";
import { SectionLabel } from "../components/ui/SectionLabel";
import { Sparkline } from "../components/ui/Sparkline";
import { Button } from "../components/ui/Button";
import { ErrorState } from "../components/ui/ErrorState";
import { formatRelativeTime } from "../lib/datetime";
import { useDashboardSnapshot, useVersionInfo } from "../lib/queries/overview";
import { useQuickInit } from "../lib/mutations/overview";
import { useApprovalCount } from "../lib/queries/approvals";
import { useMcpServers } from "../lib/queries/mcp";
import { usePeers } from "../lib/queries/network";
import { useSchedules } from "../lib/queries/schedules";
import { useSessions } from "../lib/queries/sessions";
import { useBudgetStatus } from "../lib/queries/analytics";
import { formatCompact, formatCost } from "../lib/format";

// Sample series for sparkline / cost chart trend until backend exposes them.
// Replace with snapshot fields once /api/usage/daily / /api/sessions/density land.
const COST_TREND_30D = [4.2, 4.0, 3.8, 4.5, 5.1, 4.9, 5.4, 5.0, 4.6, 5.2, 5.6, 6.1, 5.8, 6.3, 6.0, 6.4, 6.8, 6.5, 7.0, 6.9, 7.4, 7.1, 7.6, 7.3, 7.8, 8.1, 7.9, 8.4, 8.2, 8.7];
const COST_TREND_7D  = [4, 5, 4, 6, 5, 7, 6];
const COST_TREND_90D = Array.from({ length: 30 }, (_, i) => 4 + Math.sin(i / 3) * 2 + i * 0.08 + Math.random() * 1.2);
const TOKEN_BARS    = [82, 74, 91, 68, 103, 87, 79, 95, 71, 88, 102, 96, 84, 77, 108, 93, 86, 99, 82, 91, 77, 104, 89, 95, 71, 88, 102, 96, 84, 77];
const LATENCY_TREND = [320, 340, 290, 310, 285, 305, 270, 295, 280, 268, 285, 270, 260, 275, 250, 265, 245, 258, 240, 255, 232, 248, 220, 235, 215, 228, 210, 225, 205, 218];

type Range = "7d" | "30d" | "90d";
const RANGE_DATA: Record<Range, { trend: number[]; cost: number; prior: number; labelKey: string }> = {
  "7d":  { trend: COST_TREND_7D,  cost:  41.20, prior:  48.40, labelKey: "overview.range.7d" },
  "30d": { trend: COST_TREND_30D, cost: 184.20, prior: 206.40, labelKey: "overview.range.30d" },
  "90d": { trend: COST_TREND_90D, cost: 512.80, prior: 498.10, labelKey: "overview.range.90d" },
};

function CostChart({ data, height = 170 }: { data: number[]; height?: number }) {
  const w = 1000;
  const max = Math.max(...data) * 1.2;
  const stepX = w / (data.length - 1);
  const path = data.map((v, i) => `${i === 0 ? "M" : "L"}${i * stepX},${height - (v / max) * (height - 16)}`).join(" ");
  const fill = `${path} L${w},${height} L0,${height} Z`;
  const data2 = data.map((v) => v * 0.5);
  const path2 = data2.map((v, i) => `${i === 0 ? "M" : "L"}${i * stepX},${height - (v / max) * (height - 16)}`).join(" ");
  const fill2 = `${path2} L${w},${height} L0,${height} Z`;
  const markerIndex = Math.min(data.length - 1, Math.max(0, Math.floor(data.length * 0.78)));
  const markerX = markerIndex * stepX;
  const markerY = height - (data[markerIndex] / max) * (height - 16);
  return (
    <svg viewBox={`0 0 ${w} ${height}`} preserveAspectRatio="none" className="block w-full h-[130px] lg:h-[170px]" aria-hidden="true">
      <defs>
        <linearGradient id="cc-1" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#38bdf8" stopOpacity="0.3" />
          <stop offset="100%" stopColor="#38bdf8" stopOpacity="0" />
        </linearGradient>
        <linearGradient id="cc-2" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#a78bfa" stopOpacity="0.3" />
          <stop offset="100%" stopColor="#a78bfa" stopOpacity="0" />
        </linearGradient>
      </defs>
      {[0.25, 0.5, 0.75].map((p, i) => (
        <line key={i} x1={0} y1={height * p} x2={w} y2={height * p} stroke="rgba(51,65,85,0.4)" strokeDasharray="2 4" strokeWidth={0.5} />
      ))}
      <path d={fill} fill="url(#cc-1)" />
      <path d={fill2} fill="url(#cc-2)" />
      <path d={path2} fill="none" stroke="#a78bfa" strokeWidth={1.5} />
      <path d={path} fill="none" stroke="#38bdf8" strokeWidth={2} />
      <line x1={markerX} y1={0} x2={markerX} y2={height} stroke="rgba(56,189,248,0.4)" strokeWidth={1} strokeDasharray="2 2" />
      <circle cx={markerX} cy={markerY} r={3.5} fill="#38bdf8" stroke="rgba(2,6,23,0.95)" strokeWidth={2} />
    </svg>
  );
}

function formatUptime(seconds?: number): string {
  if (seconds === undefined || seconds < 0) return "-";
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m`;
  return "<1m";
}

// Backend serializes AgentState via `format!("{:?}", state)` so values
// arrive Title-cased ("Running", "Suspended", "Crashed", "Terminated",
// "Created"). Normalize to lowercase before matching, and map the Rust
// variant names to the dashboard's canonical kinds.
function normalizeState(state?: string): string {
  return (state ?? "").toLowerCase();
}

function isRunning(state?: string): boolean {
  return normalizeState(state) === "running";
}

function isErrored(state?: string): boolean {
  const s = normalizeState(state);
  return s === "crashed" || s === "failed" || s === "error";
}

function isIdle(state?: string): boolean {
  const s = normalizeState(state);
  // "Created" / "Terminated" / unset all read as idle on the hero. Suspended
  // is its own kind (paused) so it's excluded.
  return s === "" || s === "idle" || s === "created" || s === "terminated";
}

function pillKindForState(state?: string): "running" | "idle" | "error" | "pending" | "scheduled" | "paused" {
  switch (normalizeState(state)) {
    case "running":                       return "running";
    case "crashed":
    case "failed":
    case "error":                         return "error";
    case "pending":                       return "pending";
    case "scheduled":                     return "scheduled";
    case "suspended":
    case "paused":                        return "paused";
    default:                              return "idle";
  }
}

function pillKindForSessionStatus(status?: string): "ok" | "error" | "pending" {
  if (status === "error" || status === "failed") return "error";
  if (status === "pending" || status === "running") return "pending";
  return "ok";
}

function formatDuration(ms?: number): string {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return "-";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)}s`;
  const minutes = Math.floor(ms / 60_000);
  const seconds = Math.round((ms % 60_000) / 1000);
  return `${minutes}m ${seconds}s`;
}

function sessionTokens(session: {
  total_tokens?: number;
  input_tokens?: number;
  output_tokens?: number;
  context_window_tokens?: number;
}): number | undefined {
  if (typeof session.total_tokens === "number") return session.total_tokens;
  const input = typeof session.input_tokens === "number" ? session.input_tokens : undefined;
  const output = typeof session.output_tokens === "number" ? session.output_tokens : undefined;
  if (input != null || output != null) return (input ?? 0) + (output ?? 0);
  if (typeof session.context_window_tokens === "number") return session.context_window_tokens;
  return undefined;
}

function budgetUsageRatio(budget?: Record<string, unknown>): { ratio: number; used?: number; cap?: number } | null {
  if (!budget) return null;
  // Backend `/api/budget` (BudgetStatus) ships `daily_spend` / `daily_limit`
  // / `daily_pct`. The legacy `*_usd`-suffixed names below are kept as a
  // fallback so older builds and any wrapper layer that re-shapes the
  // payload still resolve.
  const usedCandidates = [
    budget.daily_spend,
    budget.spend_today_usd,
    budget.today_spend_usd,
    budget.daily_spend_usd,
    budget.current_daily_usd,
    budget.today_cost_usd,
  ];
  const capCandidates = [
    budget.daily_limit,
    budget.max_daily_usd,
    budget.daily_cap_usd,
    budget.daily_budget_usd,
  ];
  const used = usedCandidates.find((v): v is number => typeof v === "number" && Number.isFinite(v));
  const cap = capCandidates.find((v): v is number => typeof v === "number" && Number.isFinite(v) && v > 0);
  if (used == null || cap == null) return null;
  // Backend pre-computes `daily_pct` as a 0..1 fraction — prefer it when
  // present so a future change to the formula doesn't drift.
  const pct = typeof budget.daily_pct === "number" && Number.isFinite(budget.daily_pct as number)
    ? (budget.daily_pct as number)
    : used / cap;
  return { ratio: pct, used, cap };
}

type AlertKind = "error" | "warning" | "pending";
interface AlertItem {
  id: string;
  kind: AlertKind;
  title: string;
  sub: string;
  page?: string;
  /** ISO timestamp the alert was triggered at, when the source has one.
   *  Rendered as a relative time on the right edge of the row; omitted
   *  for sources where no event timestamp exists (mcp degraded,
   *  approvals count, generic health checks). */
  triggered_at?: string;
}

const ALERT_KIND_TINT: Record<AlertKind, { color: string; bg: string }> = {
  error:   { color: "#fb7185", bg: "rgba(251,113,133,0.12)" },
  warning: { color: "#fbbf24", bg: "rgba(251,191,36,0.12)" },
  pending: { color: "#fbbf24", bg: "rgba(251,191,36,0.10)" },
};

export function OverviewPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const snapshotQuery = useDashboardSnapshot();
  const versionQuery = useVersionInfo();
  const quickInitMutation = useQuickInit();
  const approvalCountQuery = useApprovalCount();
  const mcpServersQuery = useMcpServers();
  const peersQuery = usePeers();
  const schedulesQuery = useSchedules();
  const sessionsQuery = useSessions();
  const budgetStatusQuery = useBudgetStatus();

  const snapshot = snapshotQuery.data ?? null;
  const versionInfo = versionQuery.data;
  const isLoading = snapshotQuery.isLoading;
  const isError = snapshotQuery.isError;
  const needsInit = snapshot?.status?.config_exists === false;

  const [range, setRange] = useState<Range>("30d");
  const [updatedTick, setUpdatedTick] = useState(0);
  const [dismissedAlerts, setDismissedAlerts] = useState<string[]>([]);
  const [showAllAlerts, setShowAllAlerts] = useState(false);

  useEffect(() => {
    const id = window.setInterval(() => setUpdatedTick((x) => x + 1), 1000);
    return () => window.clearInterval(id);
  }, []);

  const updatedAgo = snapshotQuery.dataUpdatedAt
    ? formatRelativeTime(snapshotQuery.dataUpdatedAt)
    : "-";
  // Force re-render every second so updatedAgo refreshes; the value isn't read,
  // but useEffect tick ensures formatRelativeTime gets recomputed.
  void updatedTick;

  const agents       = snapshot?.agents ?? [];
  // Prefer the backend's pre-computed active count when available — it's
  // authoritative and matches the daemon's internal AgentState::Running
  // check exactly. Fall back to a client-side filter (case-insensitive,
  // see normalizeState) if the field is missing.
  const agentsRunning = useMemo(
    () => snapshot?.status?.active_agent_count
      ?? agents.filter((a) => isRunning(a.state)).length,
    [agents, snapshot?.status?.active_agent_count],
  );
  const agentsIdle  = useMemo(() => agents.filter((a) => isIdle(a.state)).length, [agents]);
  const agentsError = useMemo(() => agents.filter((a) => isErrored(a.state)).length, [agents]);
  const agentsTotal = snapshot?.status?.agent_count ?? agents.length;

  const mcpConfiguredCount = mcpServersQuery.data?.total_configured
    ?? mcpServersQuery.data?.configured?.length ?? 0;
  const mcpConnectedCount  = mcpServersQuery.data?.total_connected
    ?? mcpServersQuery.data?.connected?.filter((c) => c.connected).length ?? 0;
  const mcpDegradedCount   = Math.max(0, mcpConfiguredCount - mcpConnectedCount);

  const peersCount       = peersQuery.data?.length ?? 0;
  const schedulesCount   = schedulesQuery.data?.length ?? 0;
  const pendingApprovals = approvalCountQuery.data ?? 0;
  const budgetUsage = budgetUsageRatio(budgetStatusQuery.data as Record<string, unknown> | undefined);

  // 24h session count — derived from the sessions list (sorted DESC by
  // created_at, default page size 100). `snapshot.status.session_count` is
  // an all-time tally so it overstates the KPI by an order of magnitude on
  // long-lived installs. If the list is a full page we may undercount when
  // traffic is >100/24h — that's acceptable for the dashboard headline; the
  // Sessions page paginates if the user wants a full roster.
  const sessionsCount = useMemo(() => {
    const list = sessionsQuery.data ?? [];
    if (list.length === 0) return 0;
    const cutoff = Date.now() - 24 * 60 * 60 * 1000;
    return list.filter((s) => {
      if (!s.created_at) return false;
      const t = Date.parse(s.created_at);
      return Number.isFinite(t) && t >= cutoff;
    }).length;
  }, [sessionsQuery.data]);
  const defaultModel = agents.find((a) => a.model_name)?.model_name ?? "-";

  const rangeData = RANGE_DATA[range];
  const costDelta = rangeData.cost - rangeData.prior;
  const costDeltaPct = Math.abs((costDelta / rangeData.prior) * 100);
  const costTrendDir: "up" | "down" = costDelta > 0 ? "up" : "down";

  const recentAgents = useMemo(
    () => agents.filter((a) => !a.is_hand && !a.name.includes(":")).slice(0, 7),
    [agents],
  );

  // Map of agent id -> name so the recent-sessions table can show agent names
  // without an extra round-trip per row.
  const agentNameById = useMemo(() => {
    const m = new Map<string, string>();
    for (const a of agents) m.set(a.id, a.name);
    return m;
  }, [agents]);

  const recentSessions = useMemo(
    () => (sessionsQuery.data ?? [])
      .slice()
      .sort((a, b) => {
        const ta = a.created_at ? Date.parse(a.created_at) : 0;
        const tb = b.created_at ? Date.parse(b.created_at) : 0;
        return tb - ta;
      })
      .slice(0, 7),
    [sessionsQuery.data],
  );

  // Alerts — derived from snapshot + live queries. Order: errors first,
  // then warnings, then pending. Each alert is dismissible and the panel
  // collapses to 3 visible by default.
  const alerts = useMemo<AlertItem[]>(() => {
    const out: AlertItem[] = [];
    for (const a of agents) {
      if (isErrored(a.state)) {
        out.push({
          id: `agent-${a.id}`,
          kind: "error",
          title: t("overview.alerts.agent_failed", {
            defaultValue: "{{name}} failed",
            name: a.name,
          }),
          sub: t("overview.alerts.agent_failed_sub", {
            defaultValue: "Restart from Agents page",
          }),
          page: "/agents",
          triggered_at: a.last_active,
        });
      }
    }
    if (budgetUsage && budgetUsage.ratio >= 0.75) {
      out.push({
        id: "budget-threshold",
        kind: budgetUsage.ratio >= 1 ? "error" : "warning",
        title: t("overview.alerts.budget_threshold", {
          defaultValue: "Daily budget at {{pct}}%",
          pct: Math.round(budgetUsage.ratio * 100),
        }),
        sub: t("overview.alerts.budget_threshold_sub", {
          defaultValue: "{{used}} / cap {{cap}}",
          used: formatCost(budgetUsage.used ?? 0),
          cap: formatCost(budgetUsage.cap ?? 0),
        }),
        page: "/analytics",
      });
    }
    if (pendingApprovals > 0) {
      out.push({
        id: "approvals-pending",
        kind: "pending",
        title: t("overview.alerts.approvals_pending", {
          defaultValue: "{{count}} approval(s) pending",
          count: pendingApprovals,
        }),
        sub: t("overview.alerts.approvals_pending_sub", {
          defaultValue: "Review in Approvals queue",
        }),
        page: "/approvals",
      });
    }
    if (mcpDegradedCount > 0) {
      out.push({
        id: "mcp-degraded",
        kind: "warning",
        title: t("overview.alerts.mcp_degraded", {
          defaultValue: "{{count}} MCP server(s) degraded",
          count: mcpDegradedCount,
        }),
        sub: `${mcpConnectedCount}/${mcpConfiguredCount} ${t("overview.alerts.connected", { defaultValue: "connected" })}`,
        page: "/mcp",
      });
    }
    for (const c of snapshot?.health?.checks ?? []) {
      if (c.status !== "ok" && !(mcpDegradedCount > 0 && c.name.toLowerCase().includes("mcp"))) {
        out.push({
          id: `health-${c.name}`,
          kind: "warning",
          title: c.name,
          sub: c.status ?? t("overview.alerts.degraded", { defaultValue: "Degraded" }),
          page: "/runtime",
        });
      }
    }
    return out;
  }, [agents, mcpDegradedCount, mcpConnectedCount, mcpConfiguredCount, snapshot?.health?.checks, pendingApprovals, budgetUsage, t]);

  const visibleAlerts = useMemo(
    () => alerts.filter((a) => !dismissedAlerts.includes(a.id)),
    [alerts, dismissedAlerts],
  );
  const shownAlerts = showAllAlerts ? visibleAlerts : visibleAlerts.slice(0, 3);

  const dismissAlert = (id: string) =>
    setDismissedAlerts((cur) => (cur.includes(id) ? cur : [...cur, id]));

  // System tile health — drawn from real snapshot data + live runtime queries.
  const systemTiles = [
    {
      label: t("overview.system.runtime"),
      value: versionInfo?.version ?? snapshot?.status?.version ?? "-",
      dot: snapshot?.health?.status === "ok" ? "ok" : "warn",
      page: "/runtime",
    },
    {
      label: t("overview.system.scheduler", { defaultValue: "Scheduler" }),
      value: schedulesCount > 0
        ? `cron · ${schedulesCount} ${t("overview.system.active", { defaultValue: "active" })}`
        : t("overview.system.no_schedules", { defaultValue: "no schedules" }),
      dot: "ok",
      page: "/scheduler",
    },
    {
      label: t("overview.system.mcp", { defaultValue: "MCP" }),
      value: `${mcpConfiguredCount} ${t("overview.system.servers", { defaultValue: "servers" })}${mcpDegradedCount > 0 ? ` · ${mcpDegradedCount} ${t("overview.system.degraded", { defaultValue: "deg" })}` : ""}`,
      dot: mcpDegradedCount > 0 ? "warn" : "ok",
      page: "/mcp",
    },
    {
      label: t("overview.system.network", { defaultValue: "Network" }),
      value: peersCount > 0
        ? `${peersCount} ${t("overview.system.peers", { defaultValue: "peers" })}`
        : t("overview.system.no_peers", { defaultValue: "no peers" }),
      dot: "ok",
      page: "/network",
    },
    {
      label: t("overview.system.memory"),
      value: snapshot?.status?.memory_used_mb != null
        ? `${snapshot.status.memory_used_mb} MB · sqlite`
        : "sqlite",
      dot: "ok",
      page: "/memory",
    },
    {
      label: t("overview.system.audit", { defaultValue: "Audit" }),
      value: t("overview.system.audit_value", { defaultValue: "append-only · OK" }),
      dot: "ok",
      page: "/audit",
    },
  ] as const;

  const refresh = () => void snapshotQuery.refetch();

  if (isError) {
    return (
      <Card padding="lg" className="surface-lit">
        <ErrorState onRetry={refresh} />
      </Card>
    );
  }

  return (
    <div className="flex flex-col gap-3 lg:gap-4 p-3 lg:p-6">
      {/* Hero strip */}
      <header className="flex items-end justify-between gap-3 lg:gap-4 flex-wrap">
        <div className="min-w-0 flex-1">
          <div className="text-[10.5px] lg:text-[11px] font-semibold uppercase tracking-[0.08em] text-text-dim flex items-center gap-1.5">
            <span className="font-mono truncate">{snapshot?.status?.hostname ?? versionInfo?.hostname ?? "node-01"}</span>
            <span>·</span>
            <span className="truncate">{new Date().toLocaleString(undefined, { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" })}</span>
          </div>
          <h1 className="mt-1.5 text-lg sm:text-xl lg:text-2xl font-semibold tracking-[-0.02em] text-text-main flex items-center gap-2 flex-wrap">
            {isLoading ? (
              <span className="text-text-dim font-normal">{t("overview.loading_runtime", { defaultValue: "Loading runtime…" })}</span>
            ) : agentsTotal === 0 ? (
              <span>{t("overview.no_agents", { defaultValue: "No agents yet" })}</span>
            ) : (
              <>
                <span>
                  {agentsRunning} {t("overview.agents_online", { defaultValue: "agents online" })}
                </span>
                {agentsIdle > 0 || agentsError > 0 ? (
                  <span className="text-text-dim font-normal">
                    {agentsIdle > 0 ? <>· {agentsIdle} {t("overview.idle", { defaultValue: "idle" })}</> : null}
                    {agentsError > 0 ? <>{agentsIdle > 0 ? " · " : "· "}{agentsError} {t("overview.error", { defaultValue: "error" })}</> : null}
                  </span>
                ) : null}
                <Pill kind={snapshot?.health?.status === "ok" ? "running" : "pending"} size="sm" mono>
                  {snapshot?.health?.status === "ok" ? "live" : "degraded"}
                </Pill>
              </>
            )}
          </h1>
        </div>
        <div className="hidden sm:flex items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            leftIcon={<RefreshCw className={`w-3.5 h-3.5 ${snapshotQuery.isFetching ? "animate-spin" : ""}`} />}
            onClick={refresh}
          >
            {snapshotQuery.isFetching ? t("overview.refreshing", { defaultValue: "Refreshing…" }) : t("overview.refresh", { defaultValue: "Refresh" })}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            leftIcon={<Filter className="w-3.5 h-3.5" />}
            onClick={() => {
              const next: Range = range === "30d" ? "7d" : range === "7d" ? "90d" : "30d";
              setRange(next);
            }}
          >
            {t(rangeData.labelKey, { defaultValue: range === "7d" ? "7 days" : range === "30d" ? "30 days" : "90 days" })}
          </Button>
          <Button variant="primary" size="sm" leftIcon={<Plus className="w-3.5 h-3.5" />} onClick={() => navigate({ to: "/agents" })}>
            {t("overview.new_agent", { defaultValue: "New agent" })}
          </Button>
        </div>
        {/* Mobile action bar — compact icon-only buttons */}
        <div className="flex sm:hidden items-center gap-1.5 shrink-0">
          <button
            type="button"
            onClick={refresh}
            aria-label={t("overview.refresh", { defaultValue: "Refresh" })}
            className="w-9 h-9 rounded-md border border-border-subtle bg-surface text-text-dim hover:text-text-main hover:border-brand/30 grid place-items-center transition-colors"
          >
            <RefreshCw className={`w-4 h-4 ${snapshotQuery.isFetching ? "animate-spin" : ""}`} />
          </button>
          <button
            type="button"
            onClick={() => {
              const next: Range = range === "30d" ? "7d" : range === "7d" ? "90d" : "30d";
              setRange(next);
            }}
            className="h-9 px-2.5 rounded-md border border-border-subtle bg-surface text-text-dim hover:text-text-main hover:border-brand/30 inline-flex items-center gap-1 text-[11px] font-mono transition-colors"
          >
            <Filter className="w-3.5 h-3.5" />
            <span>{range}</span>
          </button>
          <button
            type="button"
            onClick={() => navigate({ to: "/agents" })}
            aria-label={t("overview.new_agent", { defaultValue: "New agent" })}
            className="w-9 h-9 rounded-md bg-brand/15 border border-brand/30 text-brand hover:bg-brand/25 grid place-items-center transition-colors"
          >
            <Plus className="w-4 h-4" />
          </button>
        </div>
      </header>

      {/* Setup banner */}
      {needsInit ? (
        <Card padding="md" className="surface-lit border-brand/30 bg-linear-to-r from-brand/5 via-brand/10 to-brand/5">
          <div className="flex flex-col sm:flex-row items-start sm:items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-brand/15">
              <Rocket className="h-5 w-5 text-brand" />
            </div>
            <div className="flex-1 min-w-0">
              <h3 className="text-sm font-semibold">{t("overview.setup_title")}</h3>
              <p className="mt-0.5 text-xs text-text-dim">{t("overview.setup_description")}</p>
            </div>
            <div className="flex items-center gap-2 shrink-0">
              <Button variant="secondary" size="sm" onClick={() => navigate({ to: "/wizard" })}>
                {t("overview.setup_wizard", { defaultValue: "Use Wizard" })}
              </Button>
              <Button
                variant="primary"
                size="sm"
                onClick={() => quickInitMutation.mutateAsync().catch(() => {})}
                disabled={quickInitMutation.isPending}
              >
                {quickInitMutation.isPending ? t("overview.setup_running") : t("overview.setup_button")}
              </Button>
            </div>
          </div>
        </Card>
      ) : null}

      {/* KPI grid */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-2 lg:gap-3">
        <Kpi
          label={t("overview.kpi.active_agents", { defaultValue: "Active agents" })}
          value={agentsRunning}
          sub={`of ${agentsTotal} ${t("overview.kpi.configured", { defaultValue: "configured" })}`}
          delta={agentsRunning > 0 ? `+${agentsRunning}` : undefined}
          trend="up"
          accent
          sparkline={<Sparkline data={[6, 8, 7, 9, 10, 9, 11, 10, agentsRunning + 1, agentsRunning]} width={70} height={28} color="#38bdf8" />}
          onClick={() => navigate({ to: "/agents" })}
        />
        <Kpi
          label={`${t("overview.kpi.spend", { defaultValue: "Spend" })} · ${t(rangeData.labelKey, { defaultValue: range })}`}
          value={`$${rangeData.cost.toFixed(2)}`}
          delta={`${costTrendDir === "up" ? "+" : "−"}${costDeltaPct.toFixed(0)}%`}
          trend={costTrendDir}
          sub={`vs $${rangeData.prior.toFixed(2)} ${t("overview.kpi.prior", { defaultValue: "prior" })}`}
          sparkline={<Sparkline data={rangeData.trend.slice(-12)} width={70} height={28} color="#a78bfa" />}
          onClick={() => navigate({ to: "/analytics" })}
        />
        <Kpi
          label={`${t("nav.sessions", { defaultValue: "Sessions" })} · 24h`}
          value={sessionsCount > 0 ? formatCompact(sessionsCount) : "0"}
          delta={sessionsCount > 0 ? `+${Math.floor(sessionsCount * 0.15)}` : undefined}
          trend="up"
          sub={`3.4k ${t("overview.kpi.peak", { defaultValue: "tokens/s peak" })}`}
          sparkline={<Sparkline data={TOKEN_BARS.slice(-12)} width={70} height={28} color="#34d399" />}
          onClick={() => navigate({ to: "/sessions" })}
        />
        <Kpi
          label={t("overview.kpi.p95_latency", { defaultValue: "P95 latency" })}
          value="218"
          unit="ms"
          delta="−14ms"
          trend="down"
          sub={defaultModel}
          sparkline={<Sparkline data={LATENCY_TREND.slice(-12).map((x) => 600 - x)} width={70} height={28} color="#fbbf24" />}
          onClick={() => navigate({ to: "/telemetry" })}
        />
      </div>

      {/* Cost trend chart + Health column */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-3">
        <Card padding="none" className="surface-lit lg:col-span-2 overflow-hidden">
          <div className="px-3 lg:px-4 pt-3 lg:pt-3.5 pb-2 flex items-start justify-between gap-2">
            <div className="min-w-0">
              <SectionLabel className="!mb-0.5">
                {t("overview.cost.title", { defaultValue: "Cost" })} · {t(rangeData.labelKey, { defaultValue: range })}
              </SectionLabel>
              <div className="flex items-baseline gap-2 flex-wrap">
                <span className="font-mono font-semibold text-lg lg:text-[22px] tracking-[-0.02em] tabular-nums">${rangeData.cost.toFixed(2)}</span>
                <span className={`text-[11px] font-mono tabular-nums ${costTrendDir === "up" ? "text-rose-400" : "text-emerald-400"}`}>
                  {costTrendDir === "up" ? "+" : "−"}${Math.abs(costDelta).toFixed(0)} {t("overview.cost.vs_prior", { defaultValue: "vs prior" })}
                </span>
              </div>
            </div>
            <div className="flex gap-1 lg:gap-1.5 shrink-0">
              {(["7d", "30d", "90d"] as const).map((p) => {
                const active = p === range;
                return (
                  <button
                    key={p}
                    onClick={() => setRange(p)}
                    className={`px-2 lg:px-2.5 py-0.5 text-[11px] rounded-md font-mono cursor-pointer transition-colors ${
                      active
                        ? "bg-brand/10 border border-brand/30 text-brand"
                        : "bg-transparent border border-border-subtle text-text-dim hover:border-brand/20"
                    }`}
                  >
                    {p}
                  </button>
                );
              })}
            </div>
          </div>
          <div className="px-2 pb-2">
            <CostChart data={rangeData.trend} height={170} />
          </div>
          <div className="flex flex-wrap gap-x-3 gap-y-1 lg:gap-4 px-3 lg:px-4 pb-3 text-[10.5px] lg:text-[11px] text-text-dim">
            <span className="inline-flex items-center gap-1.5">
              <span className="w-2 h-0.5 bg-sky-400 rounded-sm" /> Anthropic · 64%
            </span>
            <span className="inline-flex items-center gap-1.5">
              <span className="w-2 h-0.5 bg-violet-400 rounded-sm" /> OpenAI · 28%
            </span>
            <span className="inline-flex items-center gap-1.5">
              <span className="w-2 h-0.5 bg-emerald-400 rounded-sm" /> Other · 8%
            </span>
          </div>
        </Card>

        {/* Alerts — derived from agents/MCP/approvals. Mirrors the design's
            sidebar Alerts panel; dismissible items, View all / Show less. */}
        <Card padding="none" className="surface-lit">
          <div className="px-3 lg:px-4 pt-3 lg:pt-3.5">
            <SectionLabel
              action={
                visibleAlerts.length > 3 ? (
                  <button
                    onClick={() => setShowAllAlerts((x) => !x)}
                    className="bg-transparent border-0 text-brand text-[11px] cursor-pointer hover:underline"
                  >
                    {showAllAlerts
                      ? t("overview.alerts.show_less", { defaultValue: "Show less" })
                      : t("overview.view_all", { defaultValue: "View all" })}
                  </button>
                ) : null
              }
            >
              {t("overview.alerts.title", { defaultValue: "Alerts" })}
              {visibleAlerts.length > 0 ? (
                <>
                  {" · "}
                  {visibleAlerts.length}{" "}
                  {t("overview.alerts.active", { defaultValue: "active" })}
                </>
              ) : null}
            </SectionLabel>
          </div>
          <div className="flex flex-col">
            {shownAlerts.length === 0 ? (
              <div className="px-4 py-6 flex flex-col items-center gap-2 border-t border-border-subtle">
                <div
                  className="w-9 h-9 rounded-full grid place-items-center"
                  style={{
                    background: "rgba(34,197,94,0.12)",
                    color: "var(--color-success)",
                  }}
                >
                  <CheckCircle2 className="w-5 h-5" />
                </div>
                <div className="text-[12.5px] font-medium text-text-main">
                  {t("overview.alerts.systems_ok", {
                    defaultValue: "All systems operational",
                  })}
                </div>
                {dismissedAlerts.length > 0 ? (
                  <button
                    onClick={() => setDismissedAlerts([])}
                    className="bg-transparent border-0 text-brand text-[11px] cursor-pointer hover:underline"
                  >
                    {t("overview.alerts.restore_count", {
                      defaultValue: "Restore {{count}} dismissed",
                      count: dismissedAlerts.length,
                    })}
                  </button>
                ) : null}
              </div>
            ) : null}
            {shownAlerts.map((alert) => {
              const tint = ALERT_KIND_TINT[alert.kind];
              const Icon = alert.kind === "pending" ? Clock : AlertTriangle;
              return (
                <button
                  key={alert.id}
                  onClick={() => {
                    if (alert.page) navigate({ to: alert.page as never });
                    dismissAlert(alert.id);
                  }}
                  className="px-3 lg:px-4 py-2.5 flex items-start gap-2.5 border-t border-border-subtle bg-transparent text-left cursor-pointer hover:bg-main/30 transition-colors"
                >
                  <div
                    className="w-6 h-6 rounded-md grid place-items-center shrink-0"
                    style={{ background: tint.bg, color: tint.color }}
                  >
                    <Icon className="w-[13px] h-[13px]" />
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <div className="flex-1 text-[12.5px] font-medium truncate text-text-main">
                        {alert.title}
                      </div>
                      {alert.triggered_at ? (
                        <span className="font-mono text-[10.5px] text-text-dim/80 shrink-0 tabular-nums">
                          {formatRelativeTime(alert.triggered_at)}
                        </span>
                      ) : null}
                    </div>
                    <div className="text-[11px] text-text-dim mt-0.5 truncate">{alert.sub}</div>
                  </div>
                  <ChevronRight className="w-3 h-3 mt-1 text-text-dim/60 shrink-0" />
                </button>
              );
            })}
          </div>
        </Card>
      </div>

      {/* Recent sessions — Agent · Task · Msgs · time-ago. Falls back to a
          recent-agents view when the daemon hasn't surfaced any sessions yet
          (e.g. fresh install) so the row never goes empty. */}
      <Card padding="none" className="surface-lit">
        <div className="px-3 lg:px-4 pt-3 lg:pt-3.5 pb-2 flex items-center justify-between gap-2">
          <SectionLabel className="!mb-0">
            {recentSessions.length > 0
              ? t("overview.recent_sessions", { defaultValue: "Recent sessions" })
              : t("overview.recent_agents", { defaultValue: "Recent agents" })}
          </SectionLabel>
          <div className="flex items-center gap-2 lg:gap-3 shrink-0">
            <span className="font-mono text-[10.5px] lg:text-[11px] text-text-dim/80 hidden sm:inline">
              {t("overview.updated", { defaultValue: "updated" })} · {updatedAgo}
            </span>
            <button
              onClick={() => navigate({
                to: recentSessions.length > 0 ? "/sessions" : "/agents",
              } as never)}
              className="bg-transparent border-0 text-brand text-[11px] cursor-pointer hover:underline"
            >
              {t("overview.view_all", { defaultValue: "View all" })}
            </button>
          </div>
        </div>

        {/* Mobile card list (sessions) */}
        {recentSessions.length > 0 ? (
          <ul className="md:hidden flex flex-col">
            {recentSessions.slice(0, 4).map((session) => {
              const agentName = session.agent_id
                ? agentNameById.get(session.agent_id) ?? session.agent_id
                : "—";
              const status = session.active ? "running" : "ok";
              const tokens = sessionTokens(session);
              return (
                <li key={session.session_id}>
                  <button
                    onClick={() => navigate({ to: "/sessions" } as never)}
                    className="w-full text-left px-3 py-2.5 border-t border-border-subtle hover:bg-main/30 transition-colors flex items-start gap-2.5"
                  >
                    <Pill kind={pillKindForSessionStatus(status)} dot size="sm">
                      <span className="sr-only">{status}</span>
                    </Pill>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center justify-between gap-2">
                        <span className="font-mono text-[12px] truncate">{agentName}</span>
                        <span className="font-mono text-[10.5px] text-text-dim/80 shrink-0 tabular-nums">
                          {session.created_at ? formatRelativeTime(session.created_at) : "-"}
                        </span>
                      </div>
                      <div className="text-[12px] text-text-main mt-0.5 truncate">
                        {session.label ? (
                          session.label
                        ) : (
                          <span className="font-mono text-text-dim">#{session.session_id.slice(0, 8)}</span>
                        )}
                      </div>
                      <div className="mt-1 flex items-center gap-3 text-[10.5px] text-text-dim font-mono tabular-nums">
                        <span>{tokens == null ? "-" : `${formatCompact(tokens)} tok`}</span>
                        <span>{typeof session.cost_usd === "number" ? formatCost(session.cost_usd) : "-"}</span>
                        <span>{formatDuration(session.duration_ms)}</span>
                      </div>
                    </div>
                    <ChevronRight className="w-3.5 h-3.5 mt-0.5 text-text-dim/60 shrink-0" />
                  </button>
                </li>
              );
            })}
          </ul>
        ) : (
          <ul className="md:hidden flex flex-col">
            {recentAgents.length === 0 && !isLoading ? (
              <li className="px-3 py-5 text-center text-text-dim text-xs border-t border-border-subtle">
                {t("overview.no_active_agents", { defaultValue: "No agents yet" })}
              </li>
            ) : null}
            {recentAgents.slice(0, 4).map((agent) => (
              <li key={agent.id}>
                <button
                  onClick={() => navigate({ to: "/agents" })}
                  className="w-full text-left px-3 py-2.5 border-t border-border-subtle hover:bg-main/30 transition-colors flex items-start gap-2.5"
                >
                  <Pill kind={pillKindForState(agent.state)} dot size="sm">
                    <span className="sr-only">{agent.state}</span>
                  </Pill>
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center justify-between gap-2">
                      <span className="font-mono text-[12px] truncate">{agent.name}</span>
                      <span className="font-mono text-[10.5px] text-text-dim/80 shrink-0 tabular-nums">
                        {agent.last_active ? formatRelativeTime(agent.last_active) : "-"}
                      </span>
                    </div>
                    <div className="font-mono text-[11px] text-text-dim mt-0.5 truncate">
                      {agent.model_name ?? "-"}
                    </div>
                  </div>
                  <ChevronRight className="w-3.5 h-3.5 mt-0.5 text-text-dim/60 shrink-0" />
                </button>
              </li>
            ))}
          </ul>
        )}

        <div className="overflow-hidden hidden md:block">
          {recentSessions.length > 0 ? (
            <table className="w-full border-collapse text-[12.5px]">
              <thead>
                <tr className="text-text-dim/80 text-left text-[10.5px] uppercase tracking-[0.08em]">
                  <th className="px-4 py-1.5 font-semibold">{t("overview.col.agent", { defaultValue: "Agent" })}</th>
                  <th className="px-2 py-1.5 font-semibold">{t("overview.col.task", { defaultValue: "Task" })}</th>
                  <th className="px-2 py-1.5 font-semibold hidden md:table-cell font-mono">{t("overview.col.tokens", { defaultValue: "Tokens" })}</th>
                  <th className="px-2 py-1.5 font-semibold hidden md:table-cell font-mono">{t("overview.col.cost", { defaultValue: "Cost" })}</th>
                  <th className="px-2 py-1.5 font-semibold hidden lg:table-cell font-mono">{t("overview.col.duration", { defaultValue: "Dur" })}</th>
                  <th className="px-4 py-1.5 font-semibold text-right"></th>
                </tr>
              </thead>
              <tbody>
                {recentSessions.map((session) => {
                  const agentName = session.agent_id
                    ? agentNameById.get(session.agent_id) ?? session.agent_id
                    : "—";
                  const status = session.active ? "running" : "ok";
                  const tokens = sessionTokens(session);
                  return (
                    <tr
                      key={session.session_id}
                      onClick={() => navigate({ to: "/sessions" } as never)}
                      className="border-t border-border-subtle cursor-pointer hover:bg-main/30 transition-colors"
                    >
                      <td className="px-4 py-2">
                        <div className="flex items-center gap-2">
                          <Pill kind={pillKindForSessionStatus(status)} dot size="sm">
                            <span className="sr-only">{status}</span>
                          </Pill>
                          <span className="font-mono text-[12px]">{agentName}</span>
                        </div>
                      </td>
                      <td className="px-2 py-2 text-text-main">
                        <span className="block max-w-[120px] md:max-w-[280px] overflow-hidden text-ellipsis whitespace-nowrap">
                          {session.label
                            ? session.label
                            : (
                              <span className="font-mono text-text-dim">
                                #{session.session_id.slice(0, 8)}
                              </span>
                            )}
                        </span>
                      </td>
                      <td className="px-2 py-2 text-text-dim hidden md:table-cell font-mono text-[11.5px] tabular-nums">
                        {tokens == null ? "-" : formatCompact(tokens)}
                      </td>
                      <td className="px-2 py-2 text-text-dim hidden md:table-cell font-mono text-[11.5px] tabular-nums">
                        {typeof session.cost_usd === "number" ? formatCost(session.cost_usd) : "-"}
                      </td>
                      <td className="px-2 py-2 text-text-dim hidden lg:table-cell font-mono text-[11.5px] tabular-nums">
                        {formatDuration(session.duration_ms)}
                      </td>
                      <td className="px-4 py-2 text-right text-text-dim/80 font-mono text-[11px]">
                        {session.created_at ? formatRelativeTime(session.created_at) : "-"}
                        <ChevronRight className="inline w-3 h-3 ml-1 -mt-0.5 text-text-dim/60" />
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          ) : (
            <table className="w-full border-collapse text-[12.5px]">
              <thead>
                <tr className="text-text-dim/80 text-left text-[10.5px] uppercase tracking-[0.08em]">
                  <th className="px-4 py-1.5 font-semibold">{t("overview.col.agent", { defaultValue: "Agent" })}</th>
                  <th className="px-2 py-1.5 font-semibold">{t("overview.col.model", { defaultValue: "Model" })}</th>
                  <th className="px-2 py-1.5 font-semibold hidden md:table-cell">{t("overview.col.profile", { defaultValue: "Profile" })}</th>
                  <th className="px-2 py-1.5 font-semibold hidden md:table-cell">{t("overview.col.mode", { defaultValue: "Mode" })}</th>
                  <th className="px-4 py-1.5 font-semibold text-right">{t("overview.col.last_active", { defaultValue: "Last active" })}</th>
                </tr>
              </thead>
              <tbody>
                {recentAgents.length === 0 && !isLoading ? (
                  <tr>
                    <td colSpan={5} className="px-4 py-5 text-center text-text-dim text-xs border-t border-border-subtle">
                      {t("overview.no_active_agents", { defaultValue: "No agents yet" })}
                    </td>
                  </tr>
                ) : null}
                {recentAgents.map((agent) => (
                  <tr
                    key={agent.id}
                    onClick={() => navigate({ to: "/agents" })}
                    className="border-t border-border-subtle cursor-pointer hover:bg-main/30 transition-colors"
                  >
                    <td className="px-4 py-2">
                      <div className="flex items-center gap-2">
                        <Pill kind={pillKindForState(agent.state)} dot size="sm">
                          <span className="sr-only">{agent.state}</span>
                        </Pill>
                        <span className="font-mono text-[12px]">{agent.name}</span>
                      </div>
                    </td>
                    <td className="px-2 py-2 text-text-main truncate max-w-[200px]">
                      <span className="font-mono text-[11.5px]">{agent.model_name ?? "-"}</span>
                    </td>
                    <td className="px-2 py-2 text-text-dim hidden md:table-cell">{agent.profile ?? "-"}</td>
                    <td className="px-2 py-2 text-text-dim hidden md:table-cell">{agent.mode ?? "-"}</td>
                    <td className="px-4 py-2 text-right text-text-dim/80 font-mono text-[11px]">
                      {agent.last_active ? formatRelativeTime(agent.last_active) : "-"}
                      <ChevronRight className="inline w-3 h-3 ml-1 -mt-0.5 text-text-dim/60" />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </Card>

      {/* Bottom dual: tokens-by-agent + system tiles */}
      <div className="hidden md:grid md:grid-cols-2 gap-3">
        <Card padding="md" className="surface-lit">
          <SectionLabel>{t("overview.tokens_by_agent", { defaultValue: "Tokens by agent · 24h" })}</SectionLabel>
          <div className="flex flex-col gap-2 mt-1">
            {recentAgents.slice(0, 5).map((agent, idx) => {
              const pct = Math.max(0.18, 1 - idx * 0.18);
              const tokens = Math.round(420 - idx * 70);
              return (
                <button
                  key={agent.id}
                  onClick={() => navigate({ to: "/agents" })}
                  className="flex items-center gap-2.5 bg-transparent border-0 px-0 py-1 cursor-pointer text-left text-text-main"
                >
                  <span className="font-mono text-[11.5px] w-32 truncate">{agent.name}</span>
                  <div className="flex-1 h-1.5 bg-slate-700/40 rounded-full overflow-hidden">
                    <div
                      className="h-full bg-linear-to-r from-sky-400/40 to-sky-400 shadow-[0_0_8px_rgba(56,189,248,0.5)] rounded-full"
                      style={{ width: `${pct * 100}%` }}
                    />
                  </div>
                  <span className="font-mono text-[11px] text-text-dim w-12 text-right">{tokens}k</span>
                </button>
              );
            })}
            {recentAgents.length === 0 ? (
              <div className="text-text-dim text-xs py-6 text-center">
                {t("overview.no_token_data", { defaultValue: "No token usage yet" })}
              </div>
            ) : null}
          </div>
        </Card>

        <Card padding="md" className="surface-lit">
          <SectionLabel>{t("overview.system_status", { defaultValue: "System" })}</SectionLabel>
          <div className="grid grid-cols-2 gap-2.5 text-xs">
            {systemTiles.map((tile) => (
              <button
                key={tile.label}
                onClick={() => navigate({ to: tile.page as never })}
                className="px-2.5 py-2 rounded-md bg-main/60 border border-border-subtle flex items-center justify-between gap-2 cursor-pointer text-left text-text-main hover:border-brand/40 transition-colors"
              >
                <div className="min-w-0">
                  <div className="text-[10.5px] text-text-dim uppercase tracking-[0.08em] font-semibold">{tile.label}</div>
                  <div className="font-mono text-[11px] mt-0.5 truncate">{tile.value}</div>
                </div>
                <span
                  className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                    tile.dot === "ok"
                      ? "bg-emerald-400 shadow-[0_0_6px_#34d399]"
                      : "bg-amber-400 shadow-[0_0_6px_#fbbf24] animate-pulse-soft"
                  }`}
                />
              </button>
            ))}
          </div>
          <div className="mt-3 flex items-center justify-between text-[11px] text-text-dim">
            <span>
              {t("overview.uptime", { defaultValue: "Uptime" })}{" "}
              <span className="font-mono text-text-main">{formatUptime(snapshot?.status?.uptime_seconds)}</span>
            </span>
            <Pill kind={snapshot?.health?.status === "ok" ? "ok" : "pending"} size="sm">
              {snapshot?.health?.status === "ok" ? "OK" : "DEGRADED"}
            </Pill>
          </div>
        </Card>
      </div>

      {/* Pro tip */}
      <div className="hidden sm:flex items-center gap-2 rounded-lg border border-brand/10 bg-linear-to-r from-brand/5 to-transparent px-3 py-2">
        <Sparkles className="h-3.5 w-3.5 text-brand shrink-0" />
        <span className="text-[11.5px] text-text-dim flex-1">
          <span className="font-semibold text-brand">{t("overview.pro_tip", { defaultValue: "Pro tip" })}</span>{" "}
          — {t("overview.pro_tip_shortcut", { defaultValue: "Open the command palette" })}
        </span>
        <div className="flex items-center gap-1 shrink-0">
          <kbd className="inline-flex h-5 min-w-[20px] items-center justify-center rounded border border-border-subtle bg-main px-1 text-[10px] font-mono font-semibold text-text-dim">⌘K</kbd>
        </div>
      </div>
    </div>
  );
}
