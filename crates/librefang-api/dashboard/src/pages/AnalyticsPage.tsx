import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { getUsageSummary, listUsageByAgent, listUsageByModel, getUsageDaily, getBudgetStatus, updateBudget } from "../api";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { PageHeader } from "../components/ui/PageHeader";
import { EmptyState } from "../components/ui/EmptyState";
import { BarChart3, DollarSign, Shield, Save, Loader2, Cpu, Users, Zap, TrendingUp } from "lucide-react";
import { CardSkeleton } from "../components/ui/Skeleton";
import { AreaChart, Area, BarChart, Bar, XAxis, YAxis, Tooltip, ResponsiveContainer, CartesianGrid } from "recharts";

function formatNumber(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

const REFRESH_MS = 30000;

export function AnalyticsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const usageQuery = useQuery({ queryKey: ["usage", "summary"], queryFn: getUsageSummary, refetchInterval: REFRESH_MS });
  const usageByAgentQuery = useQuery({ queryKey: ["usage", "byAgent"], queryFn: listUsageByAgent, refetchInterval: REFRESH_MS });
  const usageByModelQuery = useQuery({ queryKey: ["usage", "byModel"], queryFn: listUsageByModel, refetchInterval: REFRESH_MS });
  const dailyQuery = useQuery({ queryKey: ["usage", "daily"], queryFn: getUsageDaily, refetchInterval: REFRESH_MS });
  const budgetQuery = useQuery({ queryKey: ["budget"], queryFn: getBudgetStatus, refetchInterval: REFRESH_MS });
  const budgetMutation = useMutation({ mutationFn: updateBudget, onSuccess: () => queryClient.invalidateQueries({ queryKey: ["budget"] }) });

  const usage = usageQuery.data ?? null;
  const usageByAgent = [...(usageByAgentQuery.data ?? [])].sort((a: any, b: any) => (b.total_cost_usd ?? 0) - (a.total_cost_usd ?? 0));
  const usageByModel = usageByModelQuery.data ?? [];
  const daily = dailyQuery.data ?? null;

  const [budgetForm, setBudgetForm] = useState<Record<string, string>>({});

  const isLoading = usageQuery.isLoading;

  return (
    <div className="flex flex-col gap-4 sm:gap-6 transition-colors duration-300">
      {/* Header */}
      <PageHeader
        icon={<BarChart3 className="h-4 w-4" />}
        badge={t("analytics.intelligence")}
        title={t("analytics.title")}
        subtitle={t("analytics.subtitle")}
        isFetching={usageQuery.isFetching}
        onRefresh={() => { usageQuery.refetch(); usageByAgentQuery.refetch(); usageByModelQuery.refetch(); dailyQuery.refetch(); }}
      />

      {isLoading ? (
        <div className="grid gap-4 grid-cols-2 md:grid-cols-4 stagger-children">
          {[1, 2, 3, 4].map(i => <CardSkeleton key={i} />)}
        </div>
      ) : (
        <>
          {/* KPI Cards */}
          <div className="grid grid-cols-2 gap-2 sm:gap-4 md:grid-cols-4 stagger-children">
            {[
              { icon: Zap, label: t("analytics.total_calls"), value: formatNumber(usage?.call_count ?? 0), color: "text-brand", bg: "bg-brand/10" },
              { icon: Cpu, label: t("analytics.total_tokens_label"), value: formatNumber((usage?.total_input_tokens ?? 0) + (usage?.total_output_tokens ?? 0)), color: "text-purple-500", bg: "bg-purple-500/10" },
              { icon: DollarSign, label: t("analytics.total_cost"), value: `$${(usage?.total_cost_usd ?? 0).toFixed(2)}`, color: "text-success", bg: "bg-success/10" },
              { icon: TrendingUp, label: t("analytics.today_cost"), value: `$${(daily?.today_cost_usd ?? 0).toFixed(2)}`, color: "text-warning", bg: "bg-warning/10" },
            ].map((kpi, i) => (
              <Card key={i} hover padding="md">
                <div className="flex items-center justify-between">
                  <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{kpi.label}</span>
                  <div className={`w-8 h-8 rounded-lg ${kpi.bg} flex items-center justify-center`}><kpi.icon className={`w-4 h-4 ${kpi.color}`} /></div>
                </div>
                <p className={`text-2xl sm:text-3xl font-black tracking-tight mt-1 sm:mt-2 ${kpi.color}`}>{kpi.value}</p>
              </Card>
            ))}
          </div>

          {/* Cost by Agent + Cost by Model */}
          <div className="grid gap-6 md:grid-cols-2">
            <Card padding="lg" hover>
              <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
                <Users className="w-4 h-4 text-brand" /> {t("analytics.usage_by_agent")}
              </h2>
              {usageByAgent.length === 0 ? (
                <EmptyState icon={<Users />} title={t("common.no_data")} description={t("analytics.no_agent_data")} />
              ) : (
                <ResponsiveContainer width="100%" height={Math.max(usageByAgent.slice(0, 8).length * 36, 100)}>
                  <BarChart data={usageByAgent.slice(0, 8).map(u => ({ name: u.name || u.agent_id?.slice(0, 8), cost: u.cost ?? 0 }))} layout="vertical" margin={{ left: 0, right: 20 }}>
                    <CartesianGrid strokeDasharray="3 3" opacity={0.2} horizontal={false} />
                    <XAxis type="number" tick={{ fontSize: 10 }} tickFormatter={v => `$${v}`} axisLine={false} tickLine={false} />
                    <YAxis type="category" dataKey="name" tick={{ fontSize: 10 }} width={100} axisLine={false} tickLine={false} />
                    <Tooltip contentStyle={{ borderRadius: 12, fontSize: 12 }} formatter={(v: any) => [`$${v.toFixed(4)}`, "Cost"]} />
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
                <ResponsiveContainer width="100%" height={Math.max(usageByModel.slice(0, 8).length * 36, 100)}>
                  <BarChart data={usageByModel.slice(0, 8).map(m => ({ name: m.model?.slice(0, 20), cost: m.total_cost_usd ?? 0 }))} layout="vertical" margin={{ left: 0, right: 20 }}>
                    <CartesianGrid strokeDasharray="3 3" opacity={0.2} horizontal={false} />
                    <XAxis type="number" tick={{ fontSize: 10 }} tickFormatter={v => `$${v}`} axisLine={false} tickLine={false} />
                    <YAxis type="category" dataKey="name" tick={{ fontSize: 10 }} width={120} axisLine={false} tickLine={false} />
                    <Tooltip contentStyle={{ borderRadius: 12, fontSize: 12 }} formatter={(v: any) => [`$${v.toFixed(4)}`, "Cost"]} />
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
                <AreaChart data={(daily.days || []).slice(-30).map(d => ({ ...d, date: (d.date || "").slice(5), cost: d.cost_usd || 0 }))}>
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
                    formatter={(v: any) => [`$${v.toFixed(2)}`, t("analytics.total_cost")]}
                    labelFormatter={l => `${t("analytics.daily_trend")}: ${l}`}
                  />
                  <Area type="monotone" dataKey="cost" stroke="#3b82f6" strokeWidth={2.5} fill="url(#costGrad)" dot={{ r: 3, fill: "#3b82f6", strokeWidth: 2, stroke: "white" }} activeDot={{ r: 5 }} />
                </AreaChart>
              </ResponsiveContainer>
            )}
          </Card>

          {/* Budget */}
          <Card padding="lg" hover>
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-sm font-bold flex items-center gap-2">
                <Shield className="w-4 h-4 text-brand" /> {t("analytics.budget_title")}
              </h2>
              <Button variant="primary" size="sm"
                onClick={() => {
                  const payload: Record<string, number> = {};
                  if (budgetForm.hourly) payload.max_hourly_usd = parseFloat(budgetForm.hourly);
                  if (budgetForm.daily) payload.max_daily_usd = parseFloat(budgetForm.daily);
                  if (budgetForm.monthly) payload.max_monthly_usd = parseFloat(budgetForm.monthly);
                  if (budgetForm.tokens) payload.default_max_llm_tokens_per_hour = parseInt(budgetForm.tokens);
                  if (budgetForm.alert) payload.alert_threshold = parseFloat(budgetForm.alert);
                  budgetMutation.mutate(payload);
                }}
                disabled={budgetMutation.isPending}>
                {budgetMutation.isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin mr-1" /> : <Save className="w-3.5 h-3.5 mr-1" />}
                {t("common.save")}
              </Button>
            </div>
            <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
              {[
                { key: "hourly", label: t("analytics.hourly_limit"), current: budgetQuery.data?.max_hourly_usd, unit: "$/hr" },
                { key: "daily", label: t("analytics.daily_limit"), current: budgetQuery.data?.max_daily_usd, unit: "$/day" },
                { key: "monthly", label: t("analytics.monthly_limit"), current: budgetQuery.data?.max_monthly_usd, unit: "$/mo" },
                { key: "tokens", label: t("analytics.token_limit"), current: budgetQuery.data?.default_max_llm_tokens_per_hour, unit: "tok/hr" },
                { key: "alert", label: t("analytics.alert_threshold"), current: budgetQuery.data?.alert_threshold, unit: "0-1" },
              ].map(f => (
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
        </>
      )}
    </div>
  );
}
