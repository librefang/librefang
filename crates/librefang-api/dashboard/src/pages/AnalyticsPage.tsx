import { formatCompact, formatCost } from "../lib/format";
import { useMemo, useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { useUsageSummary, useUsageByAgent, useUsageByModel, useUsageDaily, useModelPerformance, useBudgetStatus } from "../lib/queries/analytics";
import { useUpdateBudget } from "../lib/mutations/analytics";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { PageHeader } from "../components/ui/PageHeader";
import { EmptyState } from "../components/ui/EmptyState";
import { BarChart3, DollarSign, Shield, Save, Loader2, Cpu, Users, Zap, TrendingUp, Activity, Clock, Gauge, Target, Download } from "lucide-react";
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

export function AnalyticsPage() {
  const { t } = useTranslation();

  const usageQuery = useUsageSummary();
  const usageByAgentQuery = useUsageByAgent();
  const usageByModelQuery = useUsageByModel();
  const dailyQuery = useUsageDaily();
  const budgetQuery = useBudgetStatus();
  const modelPerformanceQuery = useModelPerformance();
  const budgetMutation = useUpdateBudget();

  const usage = usageQuery.data ?? null;
  const usageByAgent = useMemo(() => [...(usageByAgentQuery.data ?? [])].filter((a: any) => !a.is_hand).sort((a: any, b: any) => (b.total_cost_usd ?? 0) - (a.total_cost_usd ?? 0)), [usageByAgentQuery.data]);
  const usageByModel = usageByModelQuery.data ?? [];
  const daily = dailyQuery.data ?? null;
  const modelPerformance = modelPerformanceQuery.data ?? [];

  const agentChartData = useMemo(() => usageByAgent.map(u => ({ name: u.name || u.agent_id?.slice(0, 8), cost: u.cost ?? 0 })), [usageByAgent]);
  const modelChartData = useMemo(() => (usageByModel as any[]).map(m => ({ name: m.model?.slice(0, 20), cost: m.total_cost_usd ?? 0 })), [usageByModel]);
  const dailyChartData = useMemo(() => (daily?.days || []).slice(-30).map((d: any) => ({ ...d, date: (d.date || "").slice(5), cost: d.cost_usd || 0 })), [daily]);

  const [budgetForm, setBudgetForm] = useState<Partial<BudgetForm>>({});

  const isLoading =
    usageQuery.isLoading ||
    usageByAgentQuery.isLoading ||
    usageByModelQuery.isLoading ||
    dailyQuery.isLoading ||
    modelPerformanceQuery.isLoading;

  // Download combined per-agent + per-model usage as a CSV so operators
  // can hand it to their finance/FinOps pipeline without screenshotting.
  const handleExportCsv = () => {
    const escape = (v: unknown) => {
      if (v == null) return "";
      const s = String(v);
      return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
    };
    const lines: string[] = [];
    lines.push("scope,name,identifier,total_cost_usd,total_tokens,calls");
    for (const a of usageByAgent as any[]) {
      lines.push(
        [
          "agent",
          escape(a.name ?? ""),
          escape(a.agent_id ?? ""),
          (a.cost ?? a.total_cost_usd ?? 0).toString(),
          (a.total_tokens ?? 0).toString(),
          (a.call_count ?? a.calls ?? 0).toString(),
        ].join(","),
      );
    }
    for (const m of usageByModel as any[]) {
      lines.push(
        [
          "model",
          escape(m.model ?? ""),
          escape(m.provider ?? ""),
          (m.total_cost_usd ?? 0).toString(),
          (m.total_tokens ?? 0).toString(),
          (m.call_count ?? 0).toString(),
        ].join(","),
      );
    }
    const blob = new Blob([lines.join("\n")], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    const date = new Date().toISOString().slice(0, 10);
    a.href = url;
    a.download = `librefang-usage-${date}.csv`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  const kpis = useMemo(() => [
    { icon: Zap, label: t("analytics.total_calls"), value: formatCompact(usage?.call_count ?? 0), color: "text-primary", bg: "bg-primary/10" },
    { icon: Cpu, label: t("analytics.total_tokens_label"), value: formatCompact((usage?.total_input_tokens ?? 0) + (usage?.total_output_tokens ?? 0)), color: "text-purple-500", bg: "bg-purple-500/10" },
    { icon: DollarSign, label: t("analytics.total_cost"), value: formatCost(usage?.total_cost_usd ?? 0), color: "text-success", bg: "bg-success/10" },
    { icon: TrendingUp, label: t("analytics.today_cost"), value: formatCost(daily?.today_cost_usd ?? 0), color: "text-warning", bg: "bg-warning/10" },
  ], [usage, daily, t]);

  const modelKpis = useMemo(() => {
    if (modelPerformance.length === 0) return null;
    let totalCalls = 0;
    let weightedLatency = 0;
    let totalCost = 0;
    let fastest = modelPerformance[0];
    for (const m of modelPerformance) {
      const callCount = m.call_count ?? 0;
      totalCalls += callCount;
      weightedLatency += (m.avg_latency_ms ?? 0) * callCount;
      totalCost += (m.cost_per_call ?? 0) * callCount;
      if ((m.avg_latency_ms ?? Infinity) < (fastest.avg_latency_ms ?? Infinity)) {
        fastest = m;
      }
    }
    const avgLatency = totalCalls > 0 ? weightedLatency / totalCalls : 0;
    const avgCostPerCall = totalCalls > 0 ? totalCost / totalCalls : 0;
    return [
      { icon: Activity, label: t("analytics.avg_latency") || "Avg Latency", value: `${avgLatency.toFixed(0)}ms`, color: "text-blue-500", bg: "bg-blue-500/10" },
      { icon: Gauge, label: t("analytics.fastest_model") || "Fastest Model", value: fastest?.model?.slice(0, 12) ?? "-", color: "text-success", bg: "bg-success/10" },
      { icon: Target, label: t("analytics.avg_cost_per_call") || "Avg Cost/Call", value: `$${avgCostPerCall.toFixed(4)}`, color: "text-purple-500", bg: "bg-purple-500/10" },
      { icon: Clock, label: t("analytics.total_calls") || "Total Calls", value: totalCalls.toString(), color: "text-warning", bg: "bg-warning/10" },
    ];
  }, [modelPerformance, t]);

  const handleRefresh = useCallback(() => {
    void Promise.all([
      usageQuery.refetch(),
      usageByAgentQuery.refetch(),
      usageByModelQuery.refetch(),
      dailyQuery.refetch(),
      modelPerformanceQuery.refetch(),
      budgetQuery.refetch(),
    ]);
  }, [usageQuery, usageByAgentQuery, usageByModelQuery, dailyQuery, modelPerformanceQuery, budgetQuery]);

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
          (usageByAgent.length > 0 || (usageByModel as any[]).length > 0) ? (
            <button
              onClick={handleExportCsv}
              title={t("analytics.export_csv", { defaultValue: "Export CSV" })}
              className="flex h-8 items-center gap-1.5 rounded-xl border border-border-subtle bg-surface px-3 text-xs font-bold text-text-dim hover:text-primary hover:border-primary/30 hover:shadow-sm transition-colors duration-200"
            >
              <Download className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">CSV</span>
            </button>
          ) : undefined
        }
      />

      {isLoading ? (
        <StaggerList className="grid gap-4 grid-cols-2 md:grid-cols-4">
          {[1, 2, 3, 4].map(i => <CardSkeleton key={i} />)}
        </StaggerList>
      ) : (
        <>
          {/* KPI Cards */}
          <StaggerList className="grid grid-cols-2 gap-2 sm:gap-4 md:grid-cols-4">
            {kpis.map((kpi, i) => (
              <Card key={i} hover padding="md">
                <div className="flex items-center justify-between">
                  <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{kpi.label}</span>
                  <div className={`w-8 h-8 rounded-lg ${kpi.bg} flex items-center justify-center`}><kpi.icon className={`w-4 h-4 ${kpi.color}`} /></div>
                </div>
                <p className={`text-2xl sm:text-3xl font-black tracking-tight mt-1 sm:mt-2 ${kpi.color}`}>{kpi.value}</p>
              </Card>
            ))}
          </StaggerList>

          {/* Cost by Agent + Cost by Model */}
          <div className="grid gap-6 md:grid-cols-2">
            <Card padding="lg" hover>
              <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
                <Users className="w-4 h-4 text-primary" /> {t("analytics.usage_by_agent")}
              </h2>
              {usageByAgent.length === 0 ? (
                <EmptyState icon={<Users />} title={t("common.no_data")} description={t("analytics.no_agent_data")} />
              ) : (
                <ResponsiveContainer width="100%" height={Math.max(usageByAgent.length * 36, 100)}>
                  <BarChart data={agentChartData} layout="vertical" margin={{ left: 0, right: 20 }}>
                    <CartesianGrid strokeDasharray="3 3" opacity={0.2} horizontal={false} />
                    <XAxis type="number" tick={{ fontSize: 10 }} tickFormatter={v => `$${v}`} axisLine={false} tickLine={false} />
                    <YAxis type="category" dataKey="name" tick={{ fontSize: 10 }} width={100} axisLine={false} tickLine={false} />
                    <Tooltip contentStyle={{ borderRadius: 12, fontSize: 12 }} formatter={(v: any) => [formatCost(v), t("analytics.cost")]} />
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
                <ResponsiveContainer width="100%" height={Math.max(usageByModel.length * 36, 100)}>
                  <BarChart data={modelChartData} layout="vertical" margin={{ left: 0, right: 20 }}>
                    <CartesianGrid strokeDasharray="3 3" opacity={0.2} horizontal={false} />
                    <XAxis type="number" tick={{ fontSize: 10 }} tickFormatter={v => `$${v}`} axisLine={false} tickLine={false} />
                    <YAxis type="category" dataKey="name" tick={{ fontSize: 10 }} width={120} axisLine={false} tickLine={false} />
                    <Tooltip contentStyle={{ borderRadius: 12, fontSize: 12 }} formatter={(v: any) => [formatCost(v), t("analytics.cost")]} />
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
                    formatter={(v: any) => [formatCost(v), t("analytics.total_cost")]}
                    labelFormatter={l => `${t("analytics.daily_trend")}: ${l}`}
                  />
                  <Area type="monotone" dataKey="cost" stroke="#3b82f6" strokeWidth={2.5} fill="url(#costGrad)" dot={{ r: 3, fill: "#3b82f6", strokeWidth: 2, stroke: "white" }} activeDot={{ r: 5 }} />
                </AreaChart>
              </ResponsiveContainer>
            )}
          </Card>

          {/* Model Performance Dashboard */}
          {modelPerformance.length > 0 && (
            <>
              {/* KPI Cards for Model Performance */}
              <StaggerList className="grid grid-cols-2 gap-2 sm:gap-4 md:grid-cols-4">
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
                    <Activity className="w-4 h-4 text-blue-500" /> {t("analytics.latency_by_model") || "Latency by Model"}
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
                      <Tooltip contentStyle={{ borderRadius: 12, fontSize: 12 }} formatter={(v: any, name: any) => [`${v}ms`, name]} />
                      <Legend />
                      <Bar dataKey="avg" name={t("analytics.avg")} radius={[0, 4, 4, 0]} fill="#3b82f6" />
                      <Bar dataKey="min" name={t("analytics.min")} radius={[0, 4, 4, 0]} fill="#22c55e" />
                      <Bar dataKey="max" name={t("analytics.max")} radius={[0, 4, 4, 0]} fill="#ef4444" />
                    </BarChart>
                  </ResponsiveContainer>
                </Card>

                <Card padding="lg" hover>
                  <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
                    <DollarSign className="w-4 h-4 text-purple-500" /> {t("analytics.cost_per_call") || "Cost per Call"}
                  </h2>
                  <ResponsiveContainer width="100%" height={Math.max(modelPerformance.slice(0, 8).length * 40, 120)}>
                    <BarChart data={modelPerformance.slice(0, 8).map(m => ({ 
                      name: m.model?.slice(0, 18) ?? t("common.unknown"), 
                      costPerCall: m.cost_per_call ?? 0,
                    }))} layout="vertical" margin={{ left: 0, right: 20 }}>
                      <CartesianGrid strokeDasharray="3 3" opacity={0.2} horizontal={false} />
                      <XAxis type="number" tick={{ fontSize: 10 }} tickFormatter={v => `$${v.toFixed(4)}`} axisLine={false} tickLine={false} />
                      <YAxis type="category" dataKey="name" tick={{ fontSize: 10 }} width={120} axisLine={false} tickLine={false} />
                      <Tooltip contentStyle={{ borderRadius: 12, fontSize: 12 }} formatter={(v: any) => [`$${v.toFixed(4)}`, t("analytics.cost_per_call_label")]} />
                      <Bar dataKey="costPerCall" name={t("analytics.cost_per_call_label")} radius={[0, 4, 4, 0]} fill="#a855f7" />
                    </BarChart>
                  </ResponsiveContainer>
                </Card>
              </div>

              {/* Model Performance Table */}
              <Card padding="lg" hover>
                <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
                  <Cpu className="w-4 h-4 text-primary" /> {t("analytics.model_performance_table") || "Model Performance Details"}
                </h2>
                <div className="overflow-x-auto">
                  <table className="w-full text-xs">
                    <thead>
                      <tr className="border-b border-border-subtle">
                        <th className="text-left py-2 px-3 font-bold text-text-dim/60">{t("analytics.model") || "Model"}</th>
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.calls") || "Calls"}</th>
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.total_cost") || "Total Cost"}</th>
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.cost_call") || "Cost/Call"}</th>
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.avg_latency") || "Avg Latency"}</th>
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.min_max") || "Min/Max"}</th>
                        <th className="text-right py-2 px-3 font-bold text-text-dim/60">{t("analytics.tokens") || "Tokens"}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {modelPerformance.map((m, i) => (
                        <tr key={m.model ?? i} className="border-b border-border-subtle/50 hover:bg-primary/5">
                          <td className="py-2 px-3 font-mono font-medium">{m.model?.slice(0, 25)}</td>
                          <td className="py-2 px-3 text-right">{m.call_count ?? 0}</td>
                          <td className="py-2 px-3 text-right font-mono">${(m.total_cost_usd ?? 0).toFixed(4)}</td>
                          <td className="py-2 px-3 text-right font-mono">${(m.cost_per_call ?? 0).toFixed(4)}</td>
                          <td className="py-2 px-3 text-right font-mono">{(m.avg_latency_ms ?? 0).toFixed(0)}ms</td>
                          <td className="py-2 px-3 text-right font-mono text-text-dim">{(m.min_latency_ms ?? 0)}/{(m.max_latency_ms ?? 0)}ms</td>
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
                <Shield className="w-4 h-4 text-primary" /> {t("analytics.budget_title")}
              </h2>
              <Button variant="primary" size="sm"
                onClick={() => {
                  const payload: Record<string, number> = {};
                  if (budgetForm.hourly) {
                    const parsed = parseFloat(budgetForm.hourly);
                    if (!isNaN(parsed) && parsed >= 0) payload.max_hourly_usd = parsed;
                  }
                  if (budgetForm.daily) {
                    const parsed = parseFloat(budgetForm.daily);
                    if (!isNaN(parsed) && parsed >= 0) payload.max_daily_usd = parsed;
                  }
                  if (budgetForm.monthly) {
                    const parsed = parseFloat(budgetForm.monthly);
                    if (!isNaN(parsed) && parsed >= 0) payload.max_monthly_usd = parsed;
                  }
                  if (budgetForm.tokens) {
                    const parsed = parseInt(budgetForm.tokens);
                    if (!isNaN(parsed) && parsed >= 0) payload.default_max_llm_tokens_per_hour = parsed;
                  }
                  if (budgetForm.alert) {
                    const parsed = parseFloat(budgetForm.alert);
                    if (!isNaN(parsed) && parsed >= 0) payload.alert_threshold = parsed;
                  }
                  budgetMutation.mutate(payload);
                }}
                disabled={budgetMutation.isPending}>
                {budgetMutation.isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin mr-1" /> : <Save className="w-3.5 h-3.5 mr-1" />}
                {t("common.save")}
              </Button>
            </div>
            <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
              {([
                { key: "hourly", label: t("analytics.hourly_limit"), current: budgetQuery.data?.max_hourly_usd, unit: "$/hr" },
                { key: "daily", label: t("analytics.daily_limit"), current: budgetQuery.data?.max_daily_usd, unit: "$/day" },
                { key: "monthly", label: t("analytics.monthly_limit"), current: budgetQuery.data?.max_monthly_usd, unit: "$/mo" },
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
                      className="w-full rounded-lg border border-border-subtle bg-main px-2 py-1.5 text-xs font-mono outline-none focus:border-primary" />
                    <span className="text-[8px] text-text-dim/40 shrink-0">{f.unit}</span>
                  </div>
                </div>
              ))}
            </div>
            {budgetMutation.isSuccess && <p className="text-xs text-success mt-2">{t("analytics.budget_saved")}</p>}
          </Card>
        </>
      )}
    </div>
  );
}
