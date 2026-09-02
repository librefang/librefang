import { formatCompact, formatCost } from "../lib/format";
import { useMemo, useState, useCallback, useRef, useEffect } from "react";
import { useTranslation } from "react-i18next";
import type { UsageByAgentItem, UsageByModelItem, UsageDailyItem } from "../api";
import { useUsageSummary, useUsageByAgent, useUsageByModel, useUsageDaily, useModelPerformance, useBudgetStatus, useProviderBudgets } from "../lib/queries/analytics";
import { useUpdateBudget, useUpdateProviderBudget } from "../lib/mutations/analytics";
import type { ProviderBudgetRow } from "../api";
import { useUIStore } from "../lib/store";
import { toastErr } from "../lib/errors";
import {
  USAGE_RANGE_PRESETS,
  dailyDaysFor,
  isInvertedRange,
  normalizeUsageRange,
  rangeSpanDays,
  resolveUsageRange,
  type UsageRange,
  type UsageRangePreset,
} from "../lib/usageRange";

// `is_hand`, `total_cost_usd`, `input_tokens` / `output_tokens` and `call_count` are now declared on `UsageByAgentItem` in `api.ts` (#8062) — the page reads them as real columns rather than as a local widening.
// `calls` stays local: the handler does not emit it, and the fallback exists only so a hand-written or proxied payload using that name still renders.
type AnalyticsAgentRow = UsageByAgentItem & {
  calls?: number;
};
type AnalyticsModelRow = UsageByModelItem & {
  provider?: string;
  total_tokens?: number;
};
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { PageHeader } from "../components/ui/PageHeader";
import { EmptyState } from "../components/ui/EmptyState";
import { BarChart3, DollarSign, Shield, Save, Loader2, Cpu, Users, Zap, TrendingUp, Activity, Clock, Gauge, Target, Download, CalendarRange, Wrench, Coins, Database, Sigma } from "lucide-react";
import { CardSkeleton } from "../components/ui/Skeleton";
import { AreaChart, Area, BarChart, Bar, XAxis, YAxis, Tooltip, ResponsiveContainer, CartesianGrid, Legend } from "recharts";
import { StaggerList } from "../components/ui/StaggerList";

interface BudgetForm {
  hourly?: string;
  daily?: string;
  monthly?: string;
  tokens?: string;
  alert?: string;
}

const GLOBAL_BUDGET_FIELDS: readonly {
  formKey: keyof BudgetForm;
  payloadKey: string;
  labelKey: string;
  integer?: boolean;
  max?: number;
}[] = [
  { formKey: "hourly", payloadKey: "max_hourly_usd", labelKey: "analytics.hourly_limit" },
  { formKey: "daily", payloadKey: "max_daily_usd", labelKey: "analytics.daily_limit" },
  { formKey: "monthly", payloadKey: "max_monthly_usd", labelKey: "analytics.monthly_limit" },
  {
    formKey: "tokens",
    payloadKey: "default_max_llm_tokens_per_hour",
    labelKey: "analytics.token_limit",
    integer: true,
  },
  {
    formKey: "alert",
    payloadKey: "alert_threshold",
    labelKey: "analytics.alert_threshold",
    max: 1,
  },
];

/**
 * Widest daily window that still gets a dot per point (#8062).
 *
 * Dropping `.slice(-30)` was right — the chart must plot the window the caption claims — but the per-point dot it inherited was sized for a 30-point series.
 * At 366 points on a 200px-tall chart the dots are roughly a radius apart, so their 2px white strokes merge into an opaque band that hides the area fill and the trend underneath it, and the series costs 366 extra SVG nodes to draw.
 * `activeDot` is unaffected, so hovering any day still marks it; only the always-on dots go away.
 *
 * 31 is the cutoff because the longest calendar-month preset is 31 days: every preset the picker offers up to `last_month` keeps its dots, and only the genuinely wide windows lose them.
 */
export const DAILY_CHART_DOT_LIMIT = 31;

function providerCapTone(pct: number, alertThreshold: number): string {
  if (pct >= alertThreshold) return "bg-error shadow-[0_0_6px_rgba(239,68,68,0.45)]";
  if (pct >= alertThreshold * 0.6) return "bg-warning";
  return "bg-brand";
}

function parseNonNegative(raw: string, integer = false): number | null {
  if (raw.trim() === "") return null;
  const value = Number(raw);
  if (!Number.isFinite(value) || value < 0 || (integer && !Number.isSafeInteger(value))) return null;
  return value;
}

/**
 * Filename fragment describing the exported window (#8062 item 11).
 *
 * Mirrors the naming `GET /api/usage/export` already uses server-side, so a folder holding both kinds of export sorts and reads consistently.
 */
export function rangeFileLabel(range: UsageRange): string {
  const { start_date: start, end_date: end } = range;
  if (start && end) return start === end ? start : `${start}-to-${end}`;
  if (start) return `from-${start}`;
  if (end) return `through-${end}`;
  return "all";
}

export function escapeCsvField(value: unknown): string {
  if (value == null) return "";
  const raw = String(value);
  const safe = /^[=+\-@\t\r\n]/.test(raw) ? `'${raw}` : raw;
  return /[",\n]/.test(safe) ? `"${safe.replace(/"/g, '""')}"` : safe;
}

// Render a single percent / progress-bar pair with green/yellow/red coloring
// driven by the global `alert_threshold` echoed on `/api/budget/providers`.
// 0-cap means "unlimited" — the bar collapses to a single em-dash so the
// operator can tell at a glance there's no gate on that window.
function ProviderCapBar({
  spend,
  cap,
  alertThreshold,
}: {
  spend: number;
  cap: number;
  alertThreshold: number;
}) {
  if (cap <= 0) {
    return <span className="text-[10px] text-text-dim/60 font-mono">—</span>;
  }
  const pct = Math.min(1, spend / cap);
  const breached = pct >= alertThreshold;
  const tone = providerCapTone(pct, alertThreshold);
  return (
    <div className="flex items-center gap-1.5 min-w-[80px]">
      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-main/60">
        <div
          className={`h-full rounded-full transition-all duration-500 ${tone}`}
          style={{ width: `${(pct * 100).toFixed(1)}%` }}
        />
      </div>
      <span className={`text-[9px] font-mono ${breached ? "text-error font-bold" : "text-text-dim"}`}>
        {(pct * 100).toFixed(0)}%
      </span>
    </div>
  );
}

// #5650 — Per-provider budget table. Read surface (caps + current spend +
// exhaustion state) plus inline edit form on each row that maps onto
// `PUT /api/budget/providers/{provider_id}`. Pulled out of the main
// component so the editor's `useState<editingProvider>` doesn't churn
// the analytics page on every keystroke.
function ProviderBudgetsCard({
  rows,
  alertThreshold,
  isLoading,
  mutation,
}: {
  rows: ProviderBudgetRow[];
  alertThreshold: number;
  isLoading: boolean;
  mutation: ReturnType<typeof useUpdateProviderBudget>;
}) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const [editingProvider, setEditingProvider] = useState<string | null>(null);
  const [editForm, setEditForm] = useState<{
    max_cost_per_hour_usd: string;
    max_cost_per_day_usd: string;
    max_cost_per_month_usd: string;
    max_tokens_per_hour: string;
  }>({
    max_cost_per_hour_usd: "0",
    max_cost_per_day_usd: "0",
    max_cost_per_month_usd: "0",
    max_tokens_per_hour: "0",
  });

  const startEditing = (row: ProviderBudgetRow) => {
    setEditingProvider(row.provider);
    setEditForm({
      max_cost_per_hour_usd: String(row.cap_hourly_usd ?? 0),
      max_cost_per_day_usd: String(row.cap_daily_usd ?? 0),
      max_cost_per_month_usd: String(row.cap_monthly_usd ?? 0),
      max_tokens_per_hour: String(row.cap_tokens_per_hour ?? 0),
    });
  };

  const submitEdit = (providerId: string) => {
    const parsed = {
      max_cost_per_hour_usd: parseNonNegative(editForm.max_cost_per_hour_usd),
      max_cost_per_day_usd: parseNonNegative(editForm.max_cost_per_day_usd),
      max_cost_per_month_usd: parseNonNegative(editForm.max_cost_per_month_usd),
      max_tokens_per_hour: parseNonNegative(editForm.max_tokens_per_hour, true),
    };
    for (const [field, value] of Object.entries(parsed)) {
      if (value === null) {
        addToast(
          t("analytics.provider_budgets.bad_input", "{{field}} must be a non-negative number", {
            field,
          }),
          "error",
        );
        return;
      }
    }
    const payload = parsed as Record<keyof typeof parsed, number>;
    mutation.mutate(
      { providerId, payload },
      {
        onSuccess: () => {
          setEditingProvider(null);
          addToast(
            t("analytics.provider_budgets.saved", "Per-provider caps saved"),
            "success",
          );
        },
        onError: (err) =>
          addToast(
            toastErr(
              err,
              t("analytics.provider_budgets.save_failed", "Failed to save per-provider caps"),
            ),
            "error",
          ),
      },
    );
  };

  return (
    <Card padding="lg" hover>
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-sm font-bold flex items-center gap-2">
          <Shield className="w-4 h-4 text-brand" />
          {t("analytics.provider_budgets.title", "Per-provider caps & spend")}
        </h2>
        <span className="text-[10px] uppercase tracking-wider text-text-dim font-mono">
          {t("analytics.provider_budgets.row_count", "{{n}} providers", { n: rows.length })}
        </span>
      </div>
      <p className="text-xs text-text-dim mb-4 leading-relaxed">
        {t(
          "analytics.provider_budgets.help",
          "Each provider with a [budget.providers.<id>] entry or recent spend appears here. A cap of 0 means unlimited. Rows the LLM fallback chain is currently skipping (exhausted) carry a red badge.",
        )}
      </p>
      {isLoading && rows.length === 0 ? (
        <p className="text-xs text-text-dim italic">
          {t("analytics.provider_budgets.loading", "Loading provider spend…")}
        </p>
      ) : rows.length === 0 ? (
        <p className="text-xs text-text-dim italic">
          {t(
            "analytics.provider_budgets.empty",
            "No providers configured and no recent spend recorded.",
          )}
        </p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-xs">
            <thead>
              <tr className="text-text-dim text-[10px] uppercase tracking-wider border-b border-border-subtle">
                <th className="text-left py-2 px-2 font-semibold">{t("analytics.provider_budgets.col_provider", "Provider")}</th>
                <th className="text-right py-2 px-2 font-semibold">{t("analytics.provider_budgets.col_hourly", "Hourly")}</th>
                <th className="text-right py-2 px-2 font-semibold">{t("analytics.provider_budgets.col_daily", "Daily")}</th>
                <th className="text-right py-2 px-2 font-semibold">{t("analytics.provider_budgets.col_monthly", "Monthly")}</th>
                <th className="text-right py-2 px-2 font-semibold">{t("analytics.provider_budgets.col_tokens", "Tokens/hr")}</th>
                <th className="text-right py-2 px-2 font-semibold">{t("analytics.provider_budgets.col_state", "State")}</th>
                <th className="text-right py-2 px-2 font-semibold">{t("analytics.provider_budgets.col_actions", "")}</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => {
                const isEditing = editingProvider === row.provider;
                return (
                  <tr
                    key={row.provider}
                    className="border-b border-border-subtle/50 hover:bg-brand/5 align-top"
                  >
                    <td className="py-2 px-2 font-mono font-medium">
                      <div className="flex flex-col">
                        <span>{row.provider}</span>
                        {row.unconfigured && (
                          <span className="text-[9px] uppercase tracking-wider text-warning font-bold">
                            {t("analytics.provider_budgets.unconfigured", "set a cap")}
                          </span>
                        )}
                      </div>
                    </td>
                    {/* Spend / cap pairs — one cell per window. */}
                    {([
                      ["max_cost_per_hour_usd", row.spend_hourly_usd, row.cap_hourly_usd, "$"] as const,
                      ["max_cost_per_day_usd", row.spend_daily_usd, row.cap_daily_usd, "$"] as const,
                      ["max_cost_per_month_usd", row.spend_monthly_usd, row.cap_monthly_usd, "$"] as const,
                    ]).map(([fieldKey, spend, cap, unit]) => (
                      <td key={fieldKey} className="py-2 px-2 text-right font-mono">
                        <div className="flex flex-col items-end gap-1">
                          <span>
                            {unit}{spend.toFixed(4)}
                            <span className="text-text-dim/60"> / {cap > 0 ? `${unit}${cap.toFixed(2)}` : "∞"}</span>
                          </span>
                          <ProviderCapBar spend={spend} cap={cap} alertThreshold={alertThreshold} />
                          {isEditing && (
                            <input
                              type="number"
                              step="0.01"
                              min="0"
                              value={editForm[fieldKey]}
                              onChange={(e) =>
                                setEditForm((f) => ({ ...f, [fieldKey]: e.target.value }))
                              }
                              className="w-20 rounded-md border border-border-subtle bg-main px-1.5 py-0.5 text-[10px] font-mono outline-none focus:border-brand"
                            />
                          )}
                        </div>
                      </td>
                    ))}
                    <td className="py-2 px-2 text-right font-mono">
                      <div className="flex flex-col items-end gap-1">
                        <span>
                          {row.tokens_this_hour.toLocaleString()}
                          <span className="text-text-dim/60">
                            {" / "}
                            {row.cap_tokens_per_hour > 0 ? row.cap_tokens_per_hour.toLocaleString() : "∞"}
                          </span>
                        </span>
                        <ProviderCapBar
                          spend={row.tokens_this_hour}
                          cap={row.cap_tokens_per_hour}
                          alertThreshold={alertThreshold}
                        />
                        {isEditing && (
                          <input
                            type="number"
                            step="1"
                            min="0"
                            value={editForm.max_tokens_per_hour}
                            onChange={(e) =>
                              setEditForm((f) => ({ ...f, max_tokens_per_hour: e.target.value }))
                            }
                            className="w-20 rounded-md border border-border-subtle bg-main px-1.5 py-0.5 text-[10px] font-mono outline-none focus:border-brand"
                          />
                        )}
                      </div>
                    </td>
                    <td className="py-2 px-2 text-right">
                      {row.is_exhausted ? (
                        <span className="inline-flex items-center gap-1 rounded-full bg-error/10 px-2 py-0.5 text-[10px] font-bold uppercase tracking-wider text-error">
                          {row.exhaustion_reason ?? "exhausted"}
                        </span>
                      ) : (
                        <span className="text-[10px] text-text-dim/60 uppercase tracking-wider">
                          {t("analytics.provider_budgets.healthy", "healthy")}
                        </span>
                      )}
                    </td>
                    <td className="py-2 px-2 text-right">
                      {isEditing ? (
                        <div className="flex justify-end gap-1">
                          <Button
                            size="sm"
                            variant="primary"
                            disabled={mutation.isPending}
                            onClick={() => submitEdit(row.provider)}
                          >
                            {mutation.isPending ? (
                              <Loader2 className="w-3 h-3 animate-spin" />
                            ) : (
                              t("common.save", "Save")
                            )}
                          </Button>
                          <Button size="sm" variant="ghost" onClick={() => setEditingProvider(null)}>
                            {t("common.cancel", "Cancel")}
                          </Button>
                        </div>
                      ) : (
                        <Button size="sm" variant="ghost" onClick={() => startEditing(row)}>
                          {t("analytics.provider_budgets.edit", "Edit caps")}
                        </Button>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </Card>
  );
}

const PRESET_LABELS: Record<UsageRangePreset, string> = {
  today: "Today",
  yesterday: "Yesterday",
  "7d": "7 days",
  "14d": "14 days",
  "30d": "30 days",
  this_month: "This month",
  last_month: "Last month",
  all: "All",
};

/**
 * Reporting-window picker (#8062 item 1).
 *
 * Presets on the left, explicit from/to on the right. Picking a preset fills the two inputs so the chosen window is always visible as concrete dates rather than as a highlighted button whose meaning the reader has to infer, and editing either input switches the selection to `custom`.
 *
 * The "UTC" label is load-bearing, not decoration: the server filters on UTC calendar days, so for a viewer east or west of Greenwich "today" here is not their local today.
 * Saying so is cheaper than silently reporting a window shifted by the local offset.
 */
function UsageRangeBar({
  preset,
  range,
  onPreset,
  onRangeField,
  firstEventDate,
  retentionDays,
}: {
  preset: UsageRangePreset | "custom";
  range: UsageRange;
  onPreset: (p: UsageRangePreset) => void;
  onRangeField: (field: "start_date" | "end_date", value: string) => void;
  firstEventDate?: string | null;
  retentionDays?: number;
}) {
  const { t } = useTranslation();
  const inverted = isInvertedRange(range);
  // `first_event_date` is MIN(timestamp), so it arrives as a full RFC 3339 instant despite the field name — the day is the first 10 characters.
  const firstDay = firstEventDate ? firstEventDate.slice(0, 10) : null;

  return (
    <Card padding="md" hover>
      <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
        <div className="flex flex-wrap items-center gap-1.5">
          <CalendarRange className="mr-1 h-4 w-4 text-brand" />
          {USAGE_RANGE_PRESETS.map((p) => (
            <button
              key={p}
              type="button"
              aria-pressed={preset === p}
              onClick={() => onPreset(p)}
              className={`rounded-lg px-2.5 py-1 text-[11px] font-bold transition-colors duration-200 ${
                preset === p
                  ? "bg-brand text-white"
                  : "border border-border-subtle bg-surface text-text-dim hover:border-brand/30 hover:text-brand"
              }`}
            >
              {t(`analytics.range.${p}`, { defaultValue: PRESET_LABELS[p] })}
            </button>
          ))}
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <label className="flex items-center gap-1.5 text-[10px] font-bold uppercase tracking-wider text-text-dim">
            {t("analytics.range.from", { defaultValue: "From" })}
            <input
              type="date"
              aria-label={t("analytics.range.from", { defaultValue: "From" })}
              value={range.start_date ?? ""}
              onChange={(e) => onRangeField("start_date", e.target.value)}
              className="rounded-lg border border-border-subtle bg-main px-2 py-1 font-mono text-xs normal-case tracking-normal outline-none focus:border-brand"
            />
          </label>
          <label className="flex items-center gap-1.5 text-[10px] font-bold uppercase tracking-wider text-text-dim">
            {t("analytics.range.to", { defaultValue: "To" })}
            <input
              type="date"
              aria-label={t("analytics.range.to", { defaultValue: "To" })}
              value={range.end_date ?? ""}
              onChange={(e) => onRangeField("end_date", e.target.value)}
              className="rounded-lg border border-border-subtle bg-main px-2 py-1 font-mono text-xs normal-case tracking-normal outline-none focus:border-brand"
            />
          </label>
          <span className="rounded-md bg-main/60 px-1.5 py-0.5 font-mono text-[9px] font-bold text-text-dim/70">
            UTC
          </span>
        </div>
      </div>

      {inverted && (
        <p className="mt-2 text-xs text-error">
          {t("analytics.range.inverted", {
            defaultValue:
              "The end date is before the start date — showing the unfiltered window until the range is valid.",
          })}
        </p>
      )}

      {/* Retention indicator (#8062 item 10). `first_event_date` alone cannot
          distinguish a young deployment from one whose older rows the retention
          sweep already deleted, so both facts are stated together. */}
      <p className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10px] text-text-dim/70">
        <span className="flex items-center gap-1">
          <Database className="h-3 w-3" />
          {firstDay
            ? t("analytics.range.data_since", {
                defaultValue: "Stored usage data starts {{date}}",
                date: firstDay,
              })
            : t("analytics.range.no_data_stored", {
                defaultValue: "No usage events stored yet",
              })}
        </span>
        {retentionDays !== undefined && (
          <span>
            {retentionDays > 0
              ? t("analytics.range.retention", {
                  defaultValue: "retention {{days}} days — older events are pruned",
                  days: retentionDays,
                })
              : t("analytics.range.retention_disabled", {
                  defaultValue: "retention disabled — nothing is pruned",
                })}
          </span>
        )}
      </p>
    </Card>
  );
}

export function AnalyticsPage() {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);

  // #8062 — one reporting window drives every usage query on the page.
  // The default is `7d` rather than `all`, which is the window the page effectively showed before (the daily endpoint's own default) now stated explicitly and applied to the KPI tiles and tables too, not just the trend chart.
  const [preset, setPreset] = useState<UsageRangePreset | "custom">("7d");
  const [rawRange, setRawRange] = useState<UsageRange>(() =>
    resolveUsageRange("7d"),
  );

  // Only well-formed bounds reach the query key, so a half-typed custom date does not fire a request the server answers with 400.
  const range = useMemo(() => normalizeUsageRange(rawRange), [rawRange]);
  const dailyDays = useMemo(() => dailyDaysFor(range), [range]);

  const applyPreset = useCallback((p: UsageRangePreset) => {
    setPreset(p);
    setRawRange(resolveUsageRange(p));
  }, []);

  const setRangeField = useCallback(
    (field: "start_date" | "end_date", value: string) => {
      setPreset("custom");
      setRawRange((prev) => ({ ...prev, [field]: value }));
    },
    [],
  );

  const usageQuery = useUsageSummary(range);
  const usageByAgentQuery = useUsageByAgent(range);
  const usageByModelQuery = useUsageByModel(range);
  const dailyQuery = useUsageDaily(range, dailyDays);
  const budgetQuery = useBudgetStatus();
  const modelPerformanceQuery = useModelPerformance(range);
  const budgetMutation = useUpdateBudget();
  // #5650 — per-provider snapshot + cap mutation. Lives alongside the
  // global budget query so the two refresh in lock-step at 30s.
  const providerBudgetsQuery = useProviderBudgets();
  const providerBudgetMutation = useUpdateProviderBudget();

  const usage = usageQuery.data ?? null;
  // Every registry entry, hands included, sorted by spend. `/api/usage` filters each row on `agent_id` (never `billed_agent_id`), so the rows partition the events disjointly and a hand's spend is NOT also counted on its parent — the detail table below can therefore show hands without double-counting.
  const allAgentRows = useMemo<AnalyticsAgentRow[]>(
    () => [...((usageByAgentQuery.data ?? []) as AnalyticsAgentRow[])]
      .sort((a, b) => (b.total_cost_usd ?? b.cost ?? 0) - (a.total_cost_usd ?? a.cost ?? 0)),
    [usageByAgentQuery.data],
  );
  // The cost-by-agent chart stays hands-free: a fleet has many short-lived hands and they crowd out the agents the chart exists to compare.
  // The per-agent table (#8062 item 7) is where hands are surfaced, tagged.
  const usageByAgent = useMemo<AnalyticsAgentRow[]>(
    () => allAgentRows.filter(a => !a.is_hand),
    [allAgentRows],
  );
  const usageByModel = useMemo<AnalyticsModelRow[]>(
    () => (usageByModelQuery.data ?? []) as AnalyticsModelRow[],
    [usageByModelQuery.data],
  );
  const daily = dailyQuery.data ?? null;
  const modelPerformance = useMemo(
    () => modelPerformanceQuery.data ?? [],
    [modelPerformanceQuery.data],
  );

  const agentChartData = useMemo(() => usageByAgent.map(u => ({ name: u.name || u.agent_id?.slice(0, 8), cost: u.cost ?? 0 })), [usageByAgent]);
  const modelChartData = useMemo(() => usageByModel.map(m => ({ name: m.model?.slice(0, 20), cost: m.total_cost_usd ?? 0 })), [usageByModel]);
  // No `.slice(-30)` any more (#8062): the window is now whatever the picker selected, and truncating the tail while the range caption above claims the full span is exactly the sort of quiet disagreement this page is being fixed for.
  // `fullDate` is kept alongside the axis label because `MM-DD` repeats once a window crosses a year boundary.
  const dailyChartData = useMemo(
    () => (daily?.days || []).map((d: UsageDailyItem) => ({
      ...d,
      fullDate: d.date || "",
      date: (d.date || "").slice(5),
      cost: d.cost_usd || 0,
    })),
    [daily],
  );

  const [budgetForm, setBudgetForm] = useState<Partial<BudgetForm>>({});
  const budgetResetTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => { if (budgetResetTimerRef.current) clearTimeout(budgetResetTimerRef.current); }, []);

  const isLoading =
    usageQuery.isLoading ||
    usageByAgentQuery.isLoading ||
    usageByModelQuery.isLoading ||
    dailyQuery.isLoading ||
    modelPerformanceQuery.isLoading;

  // Sum of the daily rows the chart draws (#8062 item 9).
  // Summed over `dailyChartData` — the array the chart is handed — rather than over `daily.days` a second time, so the caption cannot describe a different set of rows than the ones plotted above it.
  // That is not hypothetical: the `.slice(-30)` this page carried until #8062 sat between the payload and the chart but not between the payload and any caption, which is exactly how a 366-day window would have been captioned over a 30-day picture.
  //
  // Distinct from the KPI row, which reads `/api/usage/summary` for the same window: agreement between the two is the reader's own check that the range is being applied consistently, so they are computed from different endpoints on purpose.
  const rangeTotals = useMemo(() => {
    let cost = 0;
    let tokens = 0;
    let calls = 0;
    for (const d of dailyChartData) {
      cost += d.cost_usd ?? 0;
      tokens += d.tokens ?? 0;
      calls += d.calls ?? 0;
    }
    return { cost, tokens, calls, dayCount: dailyChartData.length };
  }, [dailyChartData]);

  // Inclusive width of the selected window; `null` for the unbounded `all` preset, where "N of M days" has no M to report.
  const spanDays = useMemo(() => rangeSpanDays(range), [range]);

  // Download combined per-agent + per-model usage as a CSV so operators
  // can hand it to their finance/FinOps pipeline without screenshotting.
  //
  // Client-side from the JSON already fetched, so the export always matches the window on screen.
  // `GET /api/usage/export` (#7891) is the other half of this story — it streams raw per-event rows for archival; this one exports the rollups the page is showing, which is the shape a monthly report wants.
  const handleExportCsv = () => {
    const lines: string[] = [];
    lines.push(
      "scope,name,identifier,is_hand,total_cost_usd,input_tokens,output_tokens,total_tokens,calls,tool_calls",
    );
    // Hands are included here even though the chart omits them — a finance export that silently drops part of the spend is worse than a busy one.
    for (const a of allAgentRows) {
      lines.push(
        [
          "agent",
          escapeCsvField(a.name ?? ""),
          escapeCsvField(a.agent_id ?? ""),
          a.is_hand ? "true" : "false",
          (a.total_cost_usd ?? a.cost ?? 0).toString(),
          (a.input_tokens ?? 0).toString(),
          (a.output_tokens ?? 0).toString(),
          (a.total_tokens ?? 0).toString(),
          (a.call_count ?? a.calls ?? 0).toString(),
          (a.tool_calls ?? 0).toString(),
        ].join(","),
      );
    }
    for (const m of usageByModel) {
      lines.push(
        [
          "model",
          escapeCsvField(m.model ?? ""),
          escapeCsvField(m.provider ?? ""),
          "",
          (m.total_cost_usd ?? 0).toString(),
          (m.total_input_tokens ?? 0).toString(),
          (m.total_output_tokens ?? 0).toString(),
          (
            m.total_tokens ??
            (m.total_input_tokens ?? 0) + (m.total_output_tokens ?? 0)
          ).toString(),
          (m.call_count ?? 0).toString(),
          "",
        ].join(","),
      );
    }
    const blob = new Blob([lines.join("\n")], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    // Name the file after the window it covers, not the day it was downloaded — three exports of three different months used to land as one filename.
    a.href = url;
    a.download = `librefang-usage-${rangeFileLabel(range)}.csv`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  const kpis = useMemo(() => {
    const calls = usage?.call_count ?? 0;
    const cost = usage?.total_cost_usd ?? 0;
    return [
      { icon: Zap, label: t("analytics.total_calls"), value: formatCompact(calls), color: "text-brand", bg: "bg-brand/10" },
      // #8062 item 1 — tool calls were the one summary field the page never showed.
      { icon: Wrench, label: t("analytics.total_tool_calls", { defaultValue: "Tool Calls" }), value: formatCompact(usage?.total_tool_calls ?? 0), color: "text-blue-500", bg: "bg-blue-500/10" },
      { icon: Cpu, label: t("analytics.total_tokens_label"), value: formatCompact((usage?.total_input_tokens ?? 0) + (usage?.total_output_tokens ?? 0)), color: "text-purple-500", bg: "bg-purple-500/10" },
      { icon: DollarSign, label: t("analytics.total_cost"), value: formatCost(cost), color: "text-success", bg: "bg-success/10" },
      // #8062 item 2 — derived, not served: the summary carries no per-call cost.
      // Zero calls renders $0.0000 rather than NaN.
      { icon: Coins, label: t("analytics.cost_per_call_kpi", { defaultValue: "Cost / Call" }), value: `$${(calls > 0 ? cost / calls : 0).toFixed(4)}`, color: "text-pink-500", bg: "bg-pink-500/10" },
      // #8062 item 3. Deliberately NOT range-scoped — `today_cost_usd` is a fixed "since UTC midnight" rollup, so it stays put while the rest of the row follows the picker.
      // The label says so.
      { icon: TrendingUp, label: t("analytics.today_cost"), value: formatCost(daily?.today_cost_usd ?? 0), color: "text-warning", bg: "bg-warning/10" },
    ];
  }, [usage, daily, t]);

  const modelKpis = useMemo(() => {
    if (modelPerformance.length === 0) return null;
    let totalCalls = 0;
    let weightedLatency = 0;
    let totalCost = 0;
    let worstP95 = 0;
    let fastest = modelPerformance[0];
    for (const m of modelPerformance) {
      const callCount = m.call_count ?? 0;
      totalCalls += callCount;
      weightedLatency += (m.avg_latency_ms ?? 0) * callCount;
      totalCost += (m.cost_per_call ?? 0) * callCount;
      if ((m.avg_latency_ms ?? Infinity) < (fastest.avg_latency_ms ?? Infinity)) {
        fastest = m;
      }
      // #8062 item 8 — the worst per-model P95 in the window.
      // Deliberately the max rather than a call-weighted mean: an SLO is breached by the slowest model in the chain, and averaging percentiles across models is not a percentile of anything.
      if ((m.p95_latency_ms ?? 0) > worstP95) {
        worstP95 = m.p95_latency_ms ?? 0;
      }
    }
    const avgLatency = totalCalls > 0 ? weightedLatency / totalCalls : 0;
    const avgCostPerCall = totalCalls > 0 ? totalCost / totalCalls : 0;
    return [
      { icon: Activity, label: t("analytics.avg_latency", { defaultValue: "Avg Latency" }), value: `${avgLatency.toFixed(0)}ms`, color: "text-blue-500", bg: "bg-blue-500/10" },
      { icon: Sigma, label: t("analytics.worst_p95_latency", { defaultValue: "Worst P95 Latency" }), value: `${worstP95}ms`, color: "text-orange-500", bg: "bg-orange-500/10" },
      { icon: Gauge, label: t("analytics.fastest_model", { defaultValue: "Fastest Model" }), value: fastest?.model?.slice(0, 12) ?? "-", color: "text-success", bg: "bg-success/10" },
      { icon: Target, label: t("analytics.avg_cost_per_call", { defaultValue: "Avg Cost/Call" }), value: `$${avgCostPerCall.toFixed(4)}`, color: "text-purple-500", bg: "bg-purple-500/10" },
      { icon: Clock, label: t("analytics.total_calls", { defaultValue: "Total Calls" }), value: totalCalls.toString(), color: "text-warning", bg: "bg-warning/10" },
    ];
  }, [modelPerformance, t]);

  const handleRefresh = useCallback(() => {
    Promise.all([
      usageQuery.refetch(),
      usageByAgentQuery.refetch(),
      usageByModelQuery.refetch(),
      dailyQuery.refetch(),
      modelPerformanceQuery.refetch(),
      budgetQuery.refetch(),
      providerBudgetsQuery.refetch(),
    ]).catch((e) => {
      // Match NetworkPage's pattern (#4718 review L1) — surface refresh
      // failures as a toast rather than silently swallowing them.
      addToast(toastErr(e, t("common.error")), "error");
    });
  }, [usageQuery, usageByAgentQuery, usageByModelQuery, dailyQuery, modelPerformanceQuery, budgetQuery, providerBudgetsQuery, addToast, t]);

  return (
    <div className="flex flex-col gap-4 sm:gap-6 transition-colors duration-300">
      {/* Header */}
      <PageHeader
        icon={<BarChart3 className="h-4 w-4" />}
        badge={t("analytics.intelligence")}
        title={t("analytics.title")}
        subtitle={t("analytics.subtitle")}
        isFetching={usageQuery.isFetching}
        onRefresh={handleRefresh}
        helpText={t("analytics.help")}
        actions={
          (allAgentRows.length > 0 || usageByModel.length > 0) ? (
            <button
              onClick={handleExportCsv}
              title={t("analytics.export_csv", { defaultValue: "Export CSV" })}
              className="flex h-8 items-center gap-1.5 rounded-xl border border-border-subtle bg-surface px-3 text-xs font-bold text-text-dim hover:text-brand hover:border-brand/30 hover:shadow-sm transition-colors duration-200"
            >
              <Download className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">CSV</span>
            </button>
          ) : undefined
        }
      />

      {/* #8062 — the reporting window. Outside the `isLoading` branch so the
          picker stays usable while the queries it drives are in flight; a
          control that vanishes on every range change is unusable for the
          "try last month, then the month before" reading pattern. */}
      <UsageRangeBar
        preset={preset}
        range={rawRange}
        onPreset={applyPreset}
        onRangeField={setRangeField}
        firstEventDate={daily?.first_event_date}
        retentionDays={daily?.retention_days}
      />

      {isLoading ? (
        <StaggerList className="grid gap-4 grid-cols-2 md:grid-cols-4">
          {[1, 2, 3, 4].map(i => <CardSkeleton key={i} />)}
        </StaggerList>
      ) : (
        <>
          {/* KPI Cards */}
          <StaggerList className="grid grid-cols-2 gap-2 sm:gap-4 md:grid-cols-3 lg:grid-cols-6">
            {kpis.map((kpi, i) => (
              <Card key={i} hover padding="md">
                <div className="flex items-center justify-between">
                  <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{kpi.label}</span>
                  <div className={`w-8 h-8 rounded-lg ${kpi.bg} flex items-center justify-center`}><kpi.icon className={`w-4 h-4 ${kpi.color}`} /></div>
                </div>
                {/* Six tiles per row instead of four (#8062), so the value
                    steps down one size to keep a $0.0000 cost-per-call on one
                    line at the lg breakpoint. */}
                <p className={`text-xl sm:text-2xl font-black tracking-tight mt-1 sm:mt-2 ${kpi.color}`}>{kpi.value}</p>
              </Card>
            ))}
          </StaggerList>

          {/* Cost by Agent + Cost by Model */}
          <div className="grid gap-6 md:grid-cols-2">
            <Card padding="lg" hover>
              <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
                <Users className="w-4 h-4 text-brand" /> {t("analytics.usage_by_agent")}
              </h2>
              {usageByAgent.length === 0 ? (
                <EmptyState icon={<Users />} title={t("common.no_data")} description={t("analytics.no_agent_data")} />
              ) : (
                <ResponsiveContainer width="100%" height={Math.min(Math.max(usageByAgent.length * 36, 100), 600)}>
                  <BarChart data={agentChartData} layout="vertical" margin={{ left: 0, right: 20 }}>
                    <CartesianGrid strokeDasharray="3 3" opacity={0.2} horizontal={false} />
                    <XAxis type="number" tick={{ fontSize: 10 }} tickFormatter={v => `$${v}`} axisLine={false} tickLine={false} />
                    <YAxis type="category" dataKey="name" tick={{ fontSize: 10 }} width={100} axisLine={false} tickLine={false} />
                    <Tooltip contentStyle={{ borderRadius: 12, fontSize: 12 }} formatter={(v) => [formatCost(typeof v === "number" ? v : Number(v ?? 0)), t("analytics.cost")]} />
                    <Bar dataKey="cost" radius={[0, 6, 6, 0]} fill="#3b82f6" />
                  </BarChart>
                </ResponsiveContainer>
              )}
            </Card>

            <Card padding="lg" hover>
              <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
                <Cpu className="w-4 h-4 text-purple-500" /> {t("analytics.usage_by_model")}
              </h2>
              {usageByModel.length === 0 ? (
                <EmptyState icon={<Cpu />} title={t("common.no_data")} description={t("analytics.no_model_data")} />
              ) : (
                <ResponsiveContainer width="100%" height={Math.min(Math.max(usageByModel.length * 36, 100), 600)}>
                  <BarChart data={modelChartData} layout="vertical" margin={{ left: 0, right: 20 }}>
                    <CartesianGrid strokeDasharray="3 3" opacity={0.2} horizontal={false} />
                    <XAxis type="number" tick={{ fontSize: 10 }} tickFormatter={v => `$${v}`} axisLine={false} tickLine={false} />
                    <YAxis type="category" dataKey="name" tick={{ fontSize: 10 }} width={120} axisLine={false} tickLine={false} />
                    <Tooltip contentStyle={{ borderRadius: 12, fontSize: 12 }} formatter={(v) => [formatCost(typeof v === "number" ? v : Number(v ?? 0)), t("analytics.cost")]} />
                    <Bar dataKey="cost" radius={[0, 6, 6, 0]} fill="#a855f7" />
                  </BarChart>
                </ResponsiveContainer>
              )}
            </Card>
          </div>

          {/* Daily Trend */}
          <Card padding="lg" hover>
            <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
              <TrendingUp className="w-4 h-4 text-warning" /> {t("analytics.daily_trend")}
            </h2>
            {(!daily?.days || daily.days.length === 0) ? (
              <EmptyState icon={<TrendingUp />} title={t("common.no_data")} description={t("analytics.no_trend_data")} />
            ) : (
              <ResponsiveContainer width="100%" height={200}>
                <AreaChart data={dailyChartData}>
                  <defs>
                    <linearGradient id="costGrad" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="5%" stopColor="#3b82f6" stopOpacity={0.3} />
                      <stop offset="95%" stopColor="#3b82f6" stopOpacity={0} />
                    </linearGradient>
                  </defs>
                  <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" opacity={0.3} />
                  <XAxis dataKey="date" tick={{ fontSize: 10 }} tickLine={false} axisLine={false} />
                  <YAxis tick={{ fontSize: 10 }} tickLine={false} axisLine={false} tickFormatter={v => `$${v}`} width={50} />
                  <Tooltip
                    contentStyle={{ borderRadius: 12, border: "1px solid #e5e7eb", fontSize: 12, boxShadow: "0 4px 12px rgba(0,0,0,0.1)" }}
                    formatter={(v) => [formatCost(typeof v === "number" ? v : Number(v ?? 0)), t("analytics.total_cost")]}
                    // The unambiguous full date, not the MM-DD axis label — a window can span a year boundary now that it is up to 366 days wide.
                    labelFormatter={(_l, payload) =>
                      payload?.[0]?.payload?.fullDate ?? String(_l)
                    }
                  />
                  {/* Single series, so no legend — the card title names it. */}
                  <Area type="monotone" dataKey="cost" stroke="#3b82f6" strokeWidth={2.5} fill="url(#costGrad)" dot={dailyChartData.length <= DAILY_CHART_DOT_LIMIT ? { r: 3, fill: "#3b82f6", strokeWidth: 2, stroke: "white" } : false} activeDot={{ r: 5 }} />
                </AreaChart>
              </ResponsiveContainer>
            )}

            {/* Range summary (#8062 item 9) — the totals of the days plotted
                above, so the chart's shape and its magnitude are read together
                instead of the reader eyeballing an area. */}
            {rangeTotals.dayCount > 0 && (
              <div className="mt-4 grid grid-cols-2 gap-3 border-t border-border-subtle pt-3 sm:grid-cols-4">
                {([
                  { label: t("analytics.range.summary_days", { defaultValue: "Days with data" }), value: String(rangeTotals.dayCount) },
                  { label: t("analytics.range.summary_calls", { defaultValue: "Calls in range" }), value: formatCompact(rangeTotals.calls) },
                  { label: t("analytics.range.summary_tokens", { defaultValue: "Tokens in range" }), value: formatCompact(rangeTotals.tokens) },
                  { label: t("analytics.range.summary_cost", { defaultValue: "Cost in range" }), value: formatCost(rangeTotals.cost) },
                ]).map((cell) => (
                  <div key={cell.label}>
                    <p className="text-[9px] font-black uppercase tracking-widest text-text-dim/60">{cell.label}</p>
                    <p className="mt-0.5 font-mono text-sm font-bold">{cell.value}</p>
                  </div>
                ))}
                {spanDays !== null && (
                  <p className="col-span-2 text-[10px] text-text-dim/60 sm:col-span-4">
                    {t("analytics.range.summary_span", {
                      defaultValue:
                        "{{withData}} of {{span}} days in the selected window recorded usage.",
                      withData: rangeTotals.dayCount,
                      span: spanDays,
                    })}
                  </p>
                )}
              </div>
            )}
          </Card>

          {/* Per-agent detail (#8062 item 7). The cost-by-agent chart above
              compares magnitudes; this answers "what is that agent actually
              doing" — tool calls and the prompt/completion split — and is the
              only place a hand's spend is visible, tagged as such. */}
          {allAgentRows.length > 0 && (
            <Card padding="lg" hover>
              <div className="mb-4 flex items-center justify-between">
                <h2 className="flex items-center gap-2 text-sm font-bold">
                  <Users className="h-4 w-4 text-brand" />
                  {t("analytics.agent_detail_table", { defaultValue: "Per-agent Details" })}
                </h2>
                <span className="font-mono text-[10px] uppercase tracking-wider text-text-dim">
                  {t("analytics.agent_detail_count", {
                    defaultValue: "{{n}} agents",
                    n: allAgentRows.length,
                  })}
                </span>
              </div>
              <div className="overflow-x-auto">
                <table className="w-full text-xs">
                  <thead>
                    <tr className="border-b border-border-subtle">
                      <th className="px-3 py-2 text-left font-bold text-text-dim/60">{t("analytics.agent", { defaultValue: "Agent" })}</th>
                      <th className="px-3 py-2 text-right font-bold text-text-dim/60">{t("analytics.calls", { defaultValue: "Calls" })}</th>
                      <th className="px-3 py-2 text-right font-bold text-text-dim/60">{t("analytics.total_tool_calls", { defaultValue: "Tool Calls" })}</th>
                      <th className="px-3 py-2 text-right font-bold text-text-dim/60">{t("analytics.input_tokens", { defaultValue: "Input" })}</th>
                      <th className="px-3 py-2 text-right font-bold text-text-dim/60">{t("analytics.output_tokens", { defaultValue: "Output" })}</th>
                      <th className="px-3 py-2 text-right font-bold text-text-dim/60">{t("analytics.tokens", { defaultValue: "Tokens" })}</th>
                      <th className="px-3 py-2 text-right font-bold text-text-dim/60">{t("analytics.total_cost", { defaultValue: "Total Cost" })}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {allAgentRows.map((a, i) => (
                      <tr key={a.agent_id ?? i} className="border-b border-border-subtle/50 hover:bg-brand/5">
                        <td className="px-3 py-2 font-mono font-medium">
                          <span className="flex items-center gap-1.5">
                            {a.name || a.agent_id?.slice(0, 8) || t("common.unknown")}
                            {a.is_hand && (
                              <span className="rounded bg-warning/10 px-1 py-0.5 text-[9px] font-bold uppercase tracking-wider text-warning">
                                {t("analytics.hand_tag", { defaultValue: "hand" })}
                              </span>
                            )}
                          </span>
                        </td>
                        <td className="px-3 py-2 text-right">{a.call_count ?? a.calls ?? 0}</td>
                        <td className="px-3 py-2 text-right">{a.tool_calls ?? 0}</td>
                        <td className="px-3 py-2 text-right font-mono text-text-dim">{(a.input_tokens ?? 0).toLocaleString()}</td>
                        <td className="px-3 py-2 text-right font-mono text-text-dim">{(a.output_tokens ?? 0).toLocaleString()}</td>
                        <td className="px-3 py-2 text-right font-mono">{(a.total_tokens ?? 0).toLocaleString()}</td>
                        <td className="px-3 py-2 text-right font-mono">{formatCost(a.total_cost_usd ?? a.cost ?? 0)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </Card>
          )}

          {/* Model Performance Dashboard */}
          {modelPerformance.length > 0 && (
            <>
              {/* KPI Cards for Model Performance — five tiles since #8062 added
                  the worst-P95 SLO tile. */}
              <StaggerList className="grid grid-cols-2 gap-2 sm:gap-4 md:grid-cols-3 lg:grid-cols-5">
                {modelKpis?.map((kpi, i) => (
                  <Card key={i} hover padding="md">
                    <div className="flex items-center justify-between">
                      <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{kpi.label}</span>
                      <div className={`w-8 h-8 rounded-lg ${kpi.bg} flex items-center justify-center`}><kpi.icon className={`w-4 h-4 ${kpi.color}`} /></div>
                    </div>
                    <p className={`text-xl sm:text-2xl font-black tracking-tight mt-1 sm:mt-2 ${kpi.color}`}>{kpi.value}</p>
                  </Card>
                ))}
              </StaggerList>

              {/* Latency Comparison + Cost Comparison */}
              <div className="grid gap-6 md:grid-cols-2">
                <Card padding="lg" hover>
                  <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
                    <Activity className="w-4 h-4 text-blue-500" /> {t("analytics.latency_by_model", { defaultValue: "Latency by Model" })}
                  </h2>
                  <ResponsiveContainer width="100%" height={Math.max(modelPerformance.slice(0, 8).length * 40, 120)}>
                    <BarChart data={modelPerformance.slice(0, 8).map(m => ({ 
                      name: m.model?.slice(0, 18) ?? t("common.unknown"), 
                      avg: m.avg_latency_ms ?? 0,
                      min: m.min_latency_ms ?? 0,
                      max: m.max_latency_ms ?? 0,
                    }))} layout="vertical" margin={{ left: 0, right: 20 }}>
                      <CartesianGrid strokeDasharray="3 3" opacity={0.2} horizontal={false} />
                      <XAxis type="number" tick={{ fontSize: 10 }} tickFormatter={v => `${v}ms`} axisLine={false} tickLine={false} />
                      <YAxis type="category" dataKey="name" tick={{ fontSize: 10 }} width={120} axisLine={false} tickLine={false} />
                      <Tooltip contentStyle={{ borderRadius: 12, fontSize: 12 }} formatter={(v, name) => [`${v}ms`, name ?? ""]} />
                      <Legend />
                      <Bar dataKey="avg" name={t("analytics.avg")} radius={[0, 4, 4, 0]} fill="#3b82f6" />
                      <Bar dataKey="min" name={t("analytics.min")} radius={[0, 4, 4, 0]} fill="#22c55e" />
                      <Bar dataKey="max" name={t("analytics.max")} radius={[0, 4, 4, 0]} fill="#ef4444" />
                    </BarChart>
                  </ResponsiveContainer>
                </Card>

                <Card padding="lg" hover>
                  <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
                    <DollarSign className="w-4 h-4 text-purple-500" /> {t("analytics.cost_per_call", { defaultValue: "Cost per Call" })}
                  </h2>
                  <ResponsiveContainer width="100%" height={Math.max(modelPerformance.slice(0, 8).length * 40, 120)}>
                    <BarChart data={modelPerformance.slice(0, 8).map(m => ({ 
                      name: m.model?.slice(0, 18) ?? t("common.unknown"), 
                      costPerCall: m.cost_per_call ?? 0,
                    }))} layout="vertical" margin={{ left: 0, right: 20 }}>
                      <CartesianGrid strokeDasharray="3 3" opacity={0.2} horizontal={false} />
                      <XAxis type="number" tick={{ fontSize: 10 }} tickFormatter={v => `$${v.toFixed(4)}`} axisLine={false} tickLine={false} />
                      <YAxis type="category" dataKey="name" tick={{ fontSize: 10 }} width={120} axisLine={false} tickLine={false} />
                      <Tooltip contentStyle={{ borderRadius: 12, fontSize: 12 }} formatter={(v) => [`$${(typeof v === "number" ? v : Number(v ?? 0)).toFixed(4)}`, t("analytics.cost_per_call_label")]} />
                      <Bar dataKey="costPerCall" name={t("analytics.cost_per_call_label")} radius={[0, 4, 4, 0]} fill="#a855f7" />
                    </BarChart>
                  </ResponsiveContainer>
                </Card>
              </div>

              {/* Model Performance Table */}
              <Card padding="lg" hover>
                <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
                  <Cpu className="w-4 h-4 text-brand" /> {t("analytics.model_performance_table", { defaultValue: "Model Performance Details" })}
                </h2>
                <div className="overflow-x-auto">
                  <table className="w-full text-xs">
                    <thead>
                      <tr className="border-b border-border-subtle">
                        <th className="text-left py-2 px-3 font-bold text-text-dim/60">{t("analytics.model", { defaultValue: "Model" })}</th>
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.calls", { defaultValue: "Calls" })}</th>
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.total_cost", { defaultValue: "Total Cost" })}</th>
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.cost_call", { defaultValue: "Cost/Call" })}</th>
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.avg_latency", { defaultValue: "Avg Latency" })}</th>
                        {/* #8062 item 8 — the SLO column. */}
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.p95_latency", { defaultValue: "P95" })}</th>
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.min_max", { defaultValue: "Min/Max" })}</th>
                        {/* #8062 item 6 — prompt vs completion, which is what
                            makes two models comparable on price. The combined
                            total is kept so the column the page already had
                            does not disappear. */}
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.input_tokens", { defaultValue: "Input" })}</th>
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.output_tokens", { defaultValue: "Output" })}</th>
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.tokens", { defaultValue: "Tokens" })}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {modelPerformance.map((m, i) => (
                        <tr key={m.model ?? i} className="border-b border-border-subtle/50 hover:bg-brand/5">
                          <td className="py-2 px-3 font-mono font-medium">{m.model?.slice(0, 25)}</td>
                          <td className="py-2 px-3 text-right">{m.call_count ?? 0}</td>
                          <td className="py-2 px-3 text-right font-mono">${(m.total_cost_usd ?? 0).toFixed(4)}</td>
                          <td className="py-2 px-3 text-right font-mono">${(m.cost_per_call ?? 0).toFixed(4)}</td>
                          <td className="py-2 px-3 text-right font-mono">{(m.avg_latency_ms ?? 0).toFixed(0)}ms</td>
                          <td className="py-2 px-3 text-right font-mono">{m.p95_latency_ms ?? 0}ms</td>
                          <td className="py-2 px-3 text-right font-mono text-text-dim">{(m.min_latency_ms ?? 0)}/{(m.max_latency_ms ?? 0)}ms</td>
                          <td className="py-2 px-3 text-right font-mono text-text-dim">{(m.total_input_tokens ?? 0).toLocaleString()}</td>
                          <td className="py-2 px-3 text-right font-mono text-text-dim">{(m.total_output_tokens ?? 0).toLocaleString()}</td>
                          <td className="py-2 px-3 text-right font-mono">{((m.total_input_tokens ?? 0) + (m.total_output_tokens ?? 0)).toLocaleString()}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </Card>
            </>
          )}

          {/* Budget */}
          <Card padding="lg" hover>
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-sm font-bold flex items-center gap-2">
                <Shield className="w-4 h-4 text-brand" /> {t("analytics.budget_title")}
              </h2>
              <Button variant="primary" size="sm"
                onClick={() => {
                  const payload: Record<string, number> = {};
                  for (const field of GLOBAL_BUDGET_FIELDS) {
                    const raw = budgetForm[field.formKey];
                    if (raw === undefined || raw.trim() === "") continue;
                    const parsed = parseNonNegative(raw, field.integer ?? false);
                    if (parsed === null || (field.max !== undefined && parsed > field.max)) {
                      addToast(
                        t(
                          "analytics.budget_bad_input",
                          "{{field}} has an invalid value. Use a finite non-negative number; token caps must be integers and alert threshold must be 0–1.",
                          { field: t(field.labelKey) },
                        ),
                        "error",
                      );
                      return;
                    }
                    payload[field.payloadKey] = parsed;
                  }
                  if (Object.keys(payload).length === 0) {
                    addToast(
                      t("analytics.budget_no_changes", "Enter at least one valid budget value before saving."),
                      "info",
                    );
                    return;
                  }
                  budgetMutation.mutate(payload, {
                    onSuccess: () => {
                      setBudgetForm({});
                      if (budgetResetTimerRef.current) clearTimeout(budgetResetTimerRef.current);
                      budgetResetTimerRef.current = setTimeout(() => budgetMutation.reset(), 2000);
                    },
                  });
                }}
                disabled={budgetMutation.isPending}>
                {budgetMutation.isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin mr-1" /> : <Save className="w-3.5 h-3.5 mr-1" />}
                {t("common.save")}
              </Button>
            </div>
            <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
              {([
                // `GET /api/budget` returns the kernel-side `BudgetStatus`
                // shape (`*_limit` / `*_spend` / `*_pct`), NOT the on-disk
                // `BudgetConfig` field names — issue #4797 was a typo here
                // that always rendered "-" for configured caps because
                // `max_hourly_usd` is undefined on the response payload.
                { key: "hourly", label: t("analytics.hourly_limit"), current: budgetQuery.data?.hourly_limit, unit: "$/hr" },
                { key: "daily", label: t("analytics.daily_limit"), current: budgetQuery.data?.daily_limit, unit: "$/day" },
                { key: "monthly", label: t("analytics.monthly_limit"), current: budgetQuery.data?.monthly_limit, unit: "$/mo" },
                { key: "tokens", label: t("analytics.token_limit"), current: budgetQuery.data?.default_max_llm_tokens_per_hour, unit: "tok/hr" },
                { key: "alert", label: t("analytics.alert_threshold"), current: budgetQuery.data?.alert_threshold, unit: "0-1" },
              ] as { key: keyof BudgetForm; label: string; current: number | undefined; unit: string }[]).map(f => (
                <div key={f.key}>
                  <label className="text-[9px] font-bold text-text-dim uppercase">{f.label}</label>
                  <div className="flex items-center gap-1 mt-1">
                    <input type="number" step="any"
                      value={budgetForm[f.key] ?? (f.current !== undefined ? String(f.current) : "")}
                      onChange={e => setBudgetForm(prev => ({ ...prev, [f.key]: e.target.value }))}
                      placeholder={f.current !== undefined ? String(f.current) : "-"}
                      className="w-full rounded-lg border border-border-subtle bg-main px-2 py-1.5 text-xs font-mono outline-none focus:border-brand" />
                    <span className="text-[8px] text-text-dim/40 shrink-0">{f.unit}</span>
                  </div>
                </div>
              ))}
            </div>
            {budgetMutation.isSuccess && <p className="text-xs text-success mt-2">{t("analytics.budget_saved")}</p>}
          </Card>

          {/* #5650 — Per-provider caps + spend (the [budget.providers] surface). */}
          <ProviderBudgetsCard
            rows={providerBudgetsQuery.data?.providers ?? []}
            alertThreshold={
              providerBudgetsQuery.data?.alert_threshold ?? budgetQuery.data?.alert_threshold ?? 0.8
            }
            isLoading={providerBudgetsQuery.isLoading}
            mutation={providerBudgetMutation}
          />
        </>
      )}
    </div>
  );
}
