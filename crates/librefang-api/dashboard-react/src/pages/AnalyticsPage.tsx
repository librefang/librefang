import { useQuery } from "@tanstack/react-query";
import {
  getUsageDaily,
  getUsageSummary,
  listUsageByAgent,
  listUsageByModel,
  type UsageByAgentItem,
  type UsageByModelItem,
  type UsageDailyItem,
  type UsageSummaryResponse
} from "../api";

const REFRESH_MS = 30000;

interface AnalyticsSnapshot {
  summary: UsageSummaryResponse;
  byModel: UsageByModelItem[];
  byAgent: UsageByAgentItem[];
  daily: UsageDailyItem[];
  todayCost: number;
}

function formatTokens(value?: number): string {
  if (!value) return "0";
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(2)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(value);
}

function formatCost(value?: number): string {
  if (!value) return "$0.00";
  if (value < 0.01) return `$${value.toFixed(4)}`;
  return `$${value.toFixed(2)}`;
}

function dayText(value?: string): string {
  if (!value) return "-";
  const date = new Date(`${value}T12:00:00`);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

async function loadAnalyticsSnapshot(): Promise<AnalyticsSnapshot> {
  const [summary, byModel, byAgent, daily] = await Promise.all([
    getUsageSummary(),
    listUsageByModel(),
    listUsageByAgent(),
    getUsageDaily()
  ]);

  return {
    summary,
    byModel,
    byAgent,
    daily: daily.days ?? [],
    todayCost: daily.today_cost_usd ?? 0
  };
}

export function AnalyticsPage() {
  const analyticsQuery = useQuery({
    queryKey: ["analytics", "snapshot"],
    queryFn: loadAnalyticsSnapshot,
    refetchInterval: REFRESH_MS
  });

  const snapshot = analyticsQuery.data ?? null;
  const error = analyticsQuery.error instanceof Error ? analyticsQuery.error.message : "";
  const loading = analyticsQuery.isLoading;

  const models = [...(snapshot?.byModel ?? [])].sort(
    (a, b) => (b.total_cost_usd ?? 0) - (a.total_cost_usd ?? 0)
  );
  const agents = [...(snapshot?.byAgent ?? [])].sort(
    (a, b) => (b.total_tokens ?? 0) - (a.total_tokens ?? 0)
  );
  const days = snapshot?.daily ?? [];
  const maxDayCost = Math.max(1, ...days.map((day) => day.cost_usd ?? 0));

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Analytics</h1>
          <p className="text-sm text-slate-400">Usage, cost, model distribution, and per-agent breakdown.</p>
        </div>
        <button
          className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
          onClick={() => void analyticsQuery.refetch()}
          disabled={analyticsQuery.isFetching}
        >
          Refresh
        </button>
      </header>

      {loading ? (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">Loading analytics...</div>
      ) : null}
      {error ? (
        <div className="rounded-xl border border-rose-700 bg-rose-700/15 p-4 text-rose-200">{error}</div>
      ) : null}

      {snapshot ? (
        <>
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-5">
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <span className="text-sm text-slate-400">Calls</span>
              <strong className="mt-1 block text-2xl">{snapshot.summary.call_count ?? 0}</strong>
            </article>
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <span className="text-sm text-slate-400">Input Tokens</span>
              <strong className="mt-1 block text-2xl">{formatTokens(snapshot.summary.total_input_tokens)}</strong>
            </article>
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <span className="text-sm text-slate-400">Output Tokens</span>
              <strong className="mt-1 block text-2xl">{formatTokens(snapshot.summary.total_output_tokens)}</strong>
            </article>
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <span className="text-sm text-slate-400">Total Cost</span>
              <strong className="mt-1 block text-2xl">{formatCost(snapshot.summary.total_cost_usd)}</strong>
            </article>
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <span className="text-sm text-slate-400">Today Cost</span>
              <strong className="mt-1 block text-2xl">{formatCost(snapshot.todayCost)}</strong>
            </article>
          </div>

          <div className="grid gap-3 xl:grid-cols-2">
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <h2 className="m-0 text-base font-semibold">By Model</h2>
              {models.length === 0 ? (
                <p className="mt-2 text-sm text-slate-400">No model usage yet.</p>
              ) : (
                <ul className="mt-3 flex max-h-[420px] list-none flex-col gap-2 overflow-y-auto p-0">
                  {models.map((model) => (
                    <li key={model.model ?? "unknown"} className="rounded-lg border border-slate-800 bg-slate-950/70 p-3">
                      <div className="flex items-center justify-between gap-3">
                        <p className="m-0 truncate text-sm font-medium">{model.model ?? "unknown"}</p>
                        <span className="text-sm text-slate-200">{formatCost(model.total_cost_usd)}</span>
                      </div>
                      <div className="mt-2 h-2 overflow-hidden rounded-full bg-slate-800">
                        <div
                          className="h-full rounded-full bg-sky-500"
                          style={{
                            width: `${Math.max(
                              2,
                              Math.round(
                                ((model.total_cost_usd ?? 0) / Math.max(1, models[0]?.total_cost_usd ?? 1)) * 100
                              )
                            )}%`
                          }}
                        />
                      </div>
                      <p className="m-0 mt-2 text-xs text-slate-400">
                        {formatTokens((model.total_input_tokens ?? 0) + (model.total_output_tokens ?? 0))} tokens ·{" "}
                        {model.call_count ?? 0} calls
                      </p>
                    </li>
                  ))}
                </ul>
              )}
            </article>

            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <h2 className="m-0 text-base font-semibold">By Agent</h2>
              {agents.length === 0 ? (
                <p className="mt-2 text-sm text-slate-400">No agent usage yet.</p>
              ) : (
                <ul className="mt-3 flex max-h-[420px] list-none flex-col gap-2 overflow-y-auto p-0">
                  {agents.map((agent) => (
                    <li
                      key={agent.agent_id ?? agent.name ?? "unknown-agent"}
                      className="rounded-lg border border-slate-800 bg-slate-950/70 p-3"
                    >
                      <div className="flex items-center justify-between gap-3">
                        <p className="m-0 truncate text-sm font-medium">{agent.name ?? agent.agent_id ?? "unknown"}</p>
                        <span className="text-xs text-slate-400">{formatTokens(agent.total_tokens)} tokens</span>
                      </div>
                      <p className="m-0 mt-1 text-xs text-slate-500">
                        tool calls: {agent.tool_calls ?? 0} · {agent.agent_id ?? "-"}
                      </p>
                    </li>
                  ))}
                </ul>
              )}
            </article>
          </div>

          <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
            <h2 className="m-0 text-base font-semibold">Daily Cost (7d)</h2>
            {days.length === 0 ? (
              <p className="mt-2 text-sm text-slate-400">No daily cost data yet.</p>
            ) : (
              <ul className="mt-3 flex list-none flex-col gap-2 p-0">
                {days.map((day, index) => (
                  <li
                    key={day.date ?? `day-${index}`}
                    className="grid grid-cols-[90px_1fr_auto] items-center gap-3"
                  >
                    <span className="text-xs text-slate-400">{dayText(day.date)}</span>
                    <div className="h-2 overflow-hidden rounded-full bg-slate-800">
                      <div
                        className="h-full rounded-full bg-emerald-500"
                        style={{
                          width: `${Math.max(2, Math.round(((day.cost_usd ?? 0) / maxDayCost) * 100))}%`
                        }}
                      />
                    </div>
                    <span className="text-xs text-slate-300">{formatCost(day.cost_usd)}</span>
                  </li>
                ))}
              </ul>
            )}
          </article>
        </>
      ) : null}
    </section>
  );
}
