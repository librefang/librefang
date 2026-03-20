import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { getUsageSummary, listUsageByAgent, listUsageByModel, getUsageDaily, getBudgetStatus, updateBudget } from "../api";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { BarChart3, DollarSign, Shield, Save, Loader2, RefreshCw, Cpu, Users, Zap, TrendingUp, Clock } from "lucide-react";

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
  const usageByAgent = usageByAgentQuery.data ?? [];
  const usageByModel = usageByModelQuery.data ?? [];
  const daily = dailyQuery.data ?? null;

  const [budgetForm, setBudgetForm] = useState<Record<string, string>>({});

  const isLoading = usageQuery.isLoading;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      {/* Header */}
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <BarChart3 className="h-4 w-4" />
            {t("analytics.intelligence")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight">{t("analytics.title")}</h1>
          <p className="mt-1 text-text-dim font-medium text-sm">{t("analytics.subtitle")}</p>
        </div>
        <Button variant="secondary" onClick={() => { usageQuery.refetch(); usageByAgentQuery.refetch(); usageByModelQuery.refetch(); dailyQuery.refetch(); }}>
          <RefreshCw className={`h-3.5 w-3.5 ${usageQuery.isFetching ? "animate-spin" : ""}`} />
          {t("common.refresh")}
        </Button>
      </header>

      {isLoading ? (
        <div className="grid gap-4 md:grid-cols-4">
          {[1, 2, 3, 4].map(i => <div key={i} className="h-24 rounded-2xl bg-main animate-pulse" />)}
        </div>
      ) : (
        <>
          {/* KPI Cards */}
          <div className="grid gap-4 md:grid-cols-4">
            {[
              { icon: Zap, label: t("analytics.total_calls"), value: usage?.call_count ?? 0, color: "text-brand" },
              { icon: Cpu, label: t("analytics.total_tokens_label"), value: `${(((usage?.total_input_tokens ?? 0) + (usage?.total_output_tokens ?? 0)) / 1000).toFixed(0)}K`, color: "text-purple-500" },
              { icon: DollarSign, label: t("analytics.total_cost"), value: `$${(usage?.total_cost_usd ?? 0).toFixed(4)}`, color: "text-success" },
              { icon: TrendingUp, label: t("analytics.today_cost"), value: `$${(daily?.today_cost_usd ?? 0).toFixed(4)}`, color: "text-warning" },
            ].map((kpi, i) => (
              <div key={i} className="p-4 rounded-2xl border border-border-subtle bg-surface">
                <div className="flex items-center gap-2 mb-2">
                  <kpi.icon className={`w-4 h-4 ${kpi.color}`} />
                  <span className="text-[10px] font-bold text-text-dim uppercase">{kpi.label}</span>
                </div>
                <p className="text-2xl font-black">{kpi.value}</p>
              </div>
            ))}
          </div>

          {/* Cost by Agent + Cost by Model */}
          <div className="grid gap-6 md:grid-cols-2">
            <Card padding="lg">
              <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
                <Users className="w-4 h-4 text-brand" /> {t("analytics.usage_by_agent")}
              </h2>
              <div className="space-y-2 max-h-64 overflow-y-auto">
                {usageByAgent.length === 0 ? (
                  <p className="text-xs text-text-dim italic text-center py-4">{t("common.no_data")}</p>
                ) : usageByAgent.slice(0, 10).map((u, i) => {
                  const maxCost = Math.max(...usageByAgent.map(x => x.total_cost ?? 0), 0.001);
                  const pct = ((u.total_cost ?? 0) / maxCost) * 100;
                  return (
                    <div key={u.agent_id || i} className="flex items-center gap-3">
                      <span className="text-xs font-bold w-28 truncate shrink-0">{u.name || u.agent_id?.slice(0, 8)}</span>
                      <div className="flex-1 h-2 rounded-full bg-main overflow-hidden">
                        <div className="h-full bg-brand rounded-full" style={{ width: `${pct}%` }} />
                      </div>
                      <span className="text-[10px] font-mono text-text-dim w-16 text-right shrink-0">${(u.total_cost ?? 0).toFixed(4)}</span>
                    </div>
                  );
                })}
              </div>
            </Card>

            <Card padding="lg">
              <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
                <Cpu className="w-4 h-4 text-purple-500" /> {t("analytics.usage_by_model")}
              </h2>
              <div className="space-y-2 max-h-64 overflow-y-auto">
                {usageByModel.length === 0 ? (
                  <p className="text-xs text-text-dim italic text-center py-4">{t("common.no_data")}</p>
                ) : usageByModel.slice(0, 10).map((m, i) => {
                  const maxCost = Math.max(...usageByModel.map(x => x.total_cost_usd ?? 0), 0.001);
                  const pct = ((m.total_cost_usd ?? 0) / maxCost) * 100;
                  return (
                    <div key={m.model || i} className="flex items-center gap-3">
                      <span className="text-xs font-bold w-32 truncate shrink-0">{m.model}</span>
                      <div className="flex-1 h-2 rounded-full bg-main overflow-hidden">
                        <div className="h-full bg-purple-500 rounded-full" style={{ width: `${pct}%` }} />
                      </div>
                      <span className="text-[10px] font-mono text-text-dim w-16 text-right shrink-0">${(m.total_cost_usd ?? 0).toFixed(4)}</span>
                    </div>
                  );
                })}
              </div>
            </Card>
          </div>

          {/* Daily Trend */}
          <Card padding="lg">
            <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
              <TrendingUp className="w-4 h-4 text-warning" /> {t("analytics.daily_trend")}
            </h2>
            {(!daily?.days || daily.days.length === 0) ? (
              <p className="text-xs text-text-dim italic text-center py-4">{t("common.no_data")}</p>
            ) : (
              <div className="grid gap-2" style={{ gridTemplateColumns: `repeat(${Math.min((daily.days || []).length, 14)}, 1fr)` }}>
                {(daily.days || []).slice(-14).map((d, i) => {
                  const maxDay = Math.max(...(daily.days || []).map(x => x.cost_usd || 0), 0.001);
                  const pct = ((d.cost_usd || 0) / maxDay) * 100;
                  return (
                    <div key={d.date || i} className="flex flex-col items-center gap-1">
                      <span className="text-[9px] font-mono text-text-dim">${(d.cost_usd || 0).toFixed(0)}</span>
                      <div className="w-full h-24 rounded-lg bg-main overflow-hidden flex flex-col justify-end">
                        <div className="w-full rounded-lg bg-gradient-to-t from-brand to-brand/40 transition-all"
                          style={{ height: `${Math.max(pct, 5)}%` }}
                          title={`${d.date}: $${(d.cost_usd || 0).toFixed(2)} | ${d.calls || 0} calls | ${((d.tokens || 0) / 1000).toFixed(0)}K tok`} />
                      </div>
                      <span className="text-[8px] text-text-dim/50">{(d.date || "").slice(5)}</span>
                    </div>
                  );
                })}
              </div>
            )}
          </Card>

          {/* Budget */}
          <Card padding="lg">
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
            <div className="grid grid-cols-2 md:grid-cols-5 gap-3">
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
