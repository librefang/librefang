import { formatCompact } from "../lib/format";
import { formatUptime } from "../lib/datetime";
import { useMemo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useTelemetryMetrics } from "../lib/queries/telemetry";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { CardSkeleton } from "../components/ui/Skeleton";
import { Badge } from "../components/ui/Badge";
import { AnimatedNumber } from "../components/ui/AnimatedNumber";
import { ErrorState } from "../components/ui/ErrorState";
import {
  Activity, BarChart3, Clock, Globe, TrendingUp, Zap, CheckCircle2,
  ExternalLink, Cpu, DollarSign, Bot, Wrench, MessageSquare, AlertTriangle, RotateCcw, Users,
} from "lucide-react";

// ── Parsed metric types ──────────────────────────────────────────────

interface HttpMetric {
  method: string;
  path: string;
  status: string;
  count: number;
}

interface AgentTokenMetric {
  agent: string;
  provider: string;
  model: string;
  tokens: number;
  inputTokens: number;
  outputTokens: number;
  toolCalls: number;
  llmCalls: number;
}

interface SystemMetrics {
  uptime: number;
  agentsActive: number;
  agentsTotal: number;
  activeSessions: number;
  costToday: number;
  panics: number;
  restarts: number;
  version: string;
}

interface ParsedMetrics {
  requests: HttpMetric[];
  agents: AgentTokenMetric[];
  system: SystemMetrics;
}

// ── Parser ───────────────────────────────────────────────────────────

function ensureAgentEntry(map: Map<string, AgentTokenMetric>, agent: string): AgentTokenMetric {
  if (!map.has(agent)) {
    map.set(agent, {
      agent,
      provider: "",
      model: "",
      tokens: 0,
      inputTokens: 0,
      outputTokens: 0,
      toolCalls: 0,
      llmCalls: 0,
    });
  }
  return map.get(agent)!;
}

function parseMetrics(text: string): ParsedMetrics {
  const requests: HttpMetric[] = [];
  const agentMap = new Map<string, AgentTokenMetric>();
  const lines = text.split("\n");
  let version = "";

  for (const line of lines) {
    if (line.startsWith("librefang_http_requests_total{")) {
      const match = line.match(
        /librefang_http_requests_total\{method="([^"]+)",path="([^"]+)",status="([^"]+)"\}\s+(\d+)/
      );
      if (match) {
        requests.push({ method: match[1], path: match[2], status: match[3], count: parseInt(match[4], 10) });
      }
    }

    if (line.startsWith("librefang_tokens{")) {
      const match = line.match(
        /librefang_tokens\{agent="([^"]+)",provider="([^"]+)",model="([^"]+)"\}\s+([\d.]+)/
      );
      if (match) {
        const key = match[1];
        if (!agentMap.has(key)) {
          agentMap.set(key, {
            agent: match[1], provider: match[2], model: match[3],
            tokens: 0, inputTokens: 0, outputTokens: 0, toolCalls: 0, llmCalls: 0,
          });
        }
        agentMap.get(key)!.tokens = parseFloat(match[4]);
      }
    }
    if (line.startsWith("librefang_tokens_input{")) {
      const match = line.match(/librefang_tokens_input\{agent="([^"]+)"[^}]*\}\s+([\d.]+)/);
      if (match) {
        ensureAgentEntry(agentMap, match[1]).inputTokens = parseFloat(match[2]);
      }
    }
    if (line.startsWith("librefang_tokens_output{")) {
      const match = line.match(/librefang_tokens_output\{agent="([^"]+)"[^}]*\}\s+([\d.]+)/);
      if (match) {
        ensureAgentEntry(agentMap, match[1]).outputTokens = parseFloat(match[2]);
      }
    }
    if (line.startsWith("librefang_tool_calls{")) {
      const match = line.match(/librefang_tool_calls\{agent="([^"]+)"[^}]*\}\s+([\d.]+)/);
      if (match) {
        ensureAgentEntry(agentMap, match[1]).toolCalls = parseFloat(match[2]);
      }
    }
    if (line.startsWith("librefang_llm_calls{")) {
      const match = line.match(/librefang_llm_calls\{agent="([^"]+)"[^}]*\}\s+([\d.]+)/);
      if (match) {
        ensureAgentEntry(agentMap, match[1]).llmCalls = parseFloat(match[2]);
      }
    }

    if (!version && line.startsWith("librefang_info{")) {
      const match = line.match(/version="([^"]+)"/);
      if (match) version = match[1];
    }
  }

  const gaugeMap = new Map<string, number>();
  for (const line of lines) {
    const spaceIdx = line.indexOf(" ");
    if (spaceIdx > 0) {
      const namePart = line.slice(0, spaceIdx);
      if (!namePart.includes("{")) {
        gaugeMap.set(namePart, parseFloat(line.slice(spaceIdx + 1)) || 0);
      }
    }
  }

  const system: SystemMetrics = {
    uptime: gaugeMap.get("librefang_uptime_seconds") || 0,
    agentsActive: gaugeMap.get("librefang_agents_active") || 0,
    agentsTotal: gaugeMap.get("librefang_agents_total") || 0,
    activeSessions: gaugeMap.get("librefang_active_sessions") || 0,
    costToday: gaugeMap.get("librefang_cost_usd_today") || 0,
    panics: gaugeMap.get("librefang_panics_total") || 0,
    restarts: gaugeMap.get("librefang_restarts_total") || 0,
    version,
  };

  return { requests, agents: Array.from(agentMap.values()).filter(a => !a.agent.includes(":")), system };
}

// ── Component ────────────────────────────────────────────────────────

function MetricCard({ label, icon, value, variant, prefix, decimals }: {
  label: string;
  icon: ReactNode;
  value: number | ReactNode;
  variant: "success" | "brand" | "accent" | "warning" | "error";
  prefix?: string;
  decimals?: number;
}) {
  const bgClass = { success: "bg-success/10", brand: "bg-brand/10", accent: "bg-accent/10", warning: "bg-warning/10", error: "bg-error/10" }[variant];
  return (
    <Card hover padding="md">
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{label}</span>
        <div className={`w-7 h-7 rounded-lg ${bgClass} flex items-center justify-center`}>{icon}</div>
      </div>
      <div className="mt-1">
        {typeof value === "number" ? (
          <span className="text-xl font-black tracking-tight"><AnimatedNumber value={value} prefix={prefix} decimals={decimals} /></span>
        ) : (
          value
        )}
      </div>
    </Card>
  );
}

export function TelemetryPage() {
  const { t } = useTranslation();
  const metricsQuery = useTelemetryMetrics();

  const parsed = useMemo(
    () =>
      metricsQuery.data
        ? parseMetrics(metricsQuery.data)
        : { requests: [], agents: [], system: { uptime: 0, agentsActive: 0, agentsTotal: 0, activeSessions: 0, costToday: 0, panics: 0, restarts: 0, version: "" } },
    [metricsQuery.data],
  );

  const { totalRequests, totalTokens, totalInput, totalOutput, totalLlmCalls, totalToolCalls } = useMemo(() => ({
    totalRequests: parsed.requests.reduce((sum, r) => sum + r.count, 0),
    totalTokens: parsed.agents.reduce((sum, a) => sum + a.tokens, 0),
    totalInput: parsed.agents.reduce((sum, a) => sum + a.inputTokens, 0),
    totalOutput: parsed.agents.reduce((sum, a) => sum + a.outputTokens, 0),
    totalLlmCalls: parsed.agents.reduce((sum, a) => sum + a.llmCalls, 0),
    totalToolCalls: parsed.agents.reduce((sum, a) => sum + a.toolCalls, 0),
  }), [parsed]);

  const agentsByTokens = useMemo(
    () => [...parsed.agents].sort((a, b) => b.tokens - a.tokens),
    [parsed.agents],
  );

  const requestsByCount = useMemo(
    () => [...parsed.requests].sort((a, b) => b.count - a.count).slice(0, 10),
    [parsed.requests],
  );

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("telemetry.badge")}
        title={t("telemetry.title")}
        subtitle={t("telemetry.subtitle")}
        isFetching={metricsQuery.isFetching}
        onRefresh={() => void metricsQuery.refetch()}
        icon={<Activity className="h-4 w-4" />}
        helpText={t("telemetry.help")}
      />

      {metricsQuery.isLoading ? (
        <div className="grid gap-4 md:grid-cols-4">
          <CardSkeleton /><CardSkeleton /><CardSkeleton /><CardSkeleton />
        </div>
      ) : metricsQuery.isError ? (
        <ErrorState onRetry={() => void metricsQuery.refetch()} />
      ) : (
        <>
          {/* ── System Health ── */}
          <div className="flex items-center gap-2 mt-2">
            <Cpu className="h-4 w-4 text-brand" />
            <h2 className="text-sm font-black tracking-tight uppercase">{t("telemetry.system_health")}</h2>
            {parsed.system.version ? (
              <Badge variant="brand" className="ml-2">v{parsed.system.version}</Badge>
            ) : null}
          </div>
          <div className="grid grid-cols-2 gap-3 sm:gap-4 md:grid-cols-4 xl:grid-cols-8 stagger-children">
            <MetricCard
              label={t("telemetry.uptime")}
              icon={<Clock className="w-3.5 h-3.5 text-success" />}
              value={<p className="text-xl font-black tracking-tight">{formatUptime(parsed.system.uptime)}</p>}
              variant="success"
            />
            <MetricCard
              label={t("telemetry.agents_active")}
              icon={<Bot className="w-3.5 h-3.5 text-brand" />}
              value={
                <p className="text-xl font-black tracking-tight">
                  <AnimatedNumber value={parsed.system.agentsActive} />
                  <span className="text-sm font-normal text-text-dim"> / {parsed.system.agentsTotal}</span>
                </p>
              }
              variant="brand"
            />
            <MetricCard
              label={t("telemetry.active_sessions")}
              icon={<Users className="w-3.5 h-3.5 text-accent" />}
              value={parsed.system.activeSessions}
              variant="accent"
            />
            <MetricCard
              label={t("telemetry.cost_today")}
              icon={<DollarSign className="w-3.5 h-3.5 text-warning" />}
              value={parsed.system.costToday}
              variant="warning"
              prefix="$"
              decimals={4}
            />
            <MetricCard
              label={t("telemetry.total_requests")}
              icon={<BarChart3 className="w-3.5 h-3.5 text-brand" />}
              value={<p className="text-xl font-black tracking-tight" title={totalRequests.toLocaleString()}>{formatCompact(totalRequests)}</p>}
              variant="brand"
            />
            <MetricCard
              label={t("telemetry.panics")}
              icon={<AlertTriangle className="w-3.5 h-3.5 text-error" />}
              value={parsed.system.panics}
              variant="error"
            />
            <MetricCard
              label={t("telemetry.restarts")}
              icon={<RotateCcw className="w-3.5 h-3.5 text-warning" />}
              value={parsed.system.restarts}
              variant="warning"
            />
            <MetricCard
              label={t("telemetry.status")}
              icon={<CheckCircle2 className="w-3.5 h-3.5 text-success" />}
              value={
                <div className="flex items-center gap-2">
                  <span className="relative flex h-2 w-2">
                    <span className="absolute inline-flex h-full w-full rounded-full bg-success opacity-75 animate-pulse" />
                    <span className="relative inline-flex rounded-full h-2 w-2 bg-success" />
                  </span>
                  <Badge variant="success">{t("telemetry.collecting")}</Badge>
                </div>
              }
              variant="success"
            />
          </div>

          {/* ── LLM & Token Usage ── */}
          <div className="flex items-center gap-2 mt-4">
            <Zap className="h-4 w-4 text-warning" />
            <h2 className="text-sm font-black tracking-tight uppercase">{t("telemetry.llm_usage")}</h2>
            <Badge variant="default" className="ml-2">{t("telemetry.tokens_1h")}</Badge>
          </div>
          <div className="grid grid-cols-2 gap-3 sm:gap-4 md:grid-cols-3 lg:grid-cols-5 stagger-children">
            <MetricCard
              label={t("telemetry.total_tokens")}
              icon={<BarChart3 className="w-3.5 h-3.5 text-brand" />}
              value={<p className="text-2xl font-black tracking-tight text-brand" title={totalTokens.toLocaleString()}>{formatCompact(totalTokens)}</p>}
              variant="brand"
            />
            <MetricCard
              label={t("telemetry.input_tokens")}
              icon={<TrendingUp className="w-3.5 h-3.5 text-success" />}
              value={<p className="text-2xl font-black tracking-tight text-success" title={totalInput.toLocaleString()}>{formatCompact(totalInput)}</p>}
              variant="success"
            />
            <MetricCard
              label={t("telemetry.output_tokens")}
              icon={<TrendingUp className="w-3.5 h-3.5 text-warning" />}
              value={<p className="text-2xl font-black tracking-tight text-warning" title={totalOutput.toLocaleString()}>{formatCompact(totalOutput)}</p>}
              variant="warning"
            />
            <MetricCard
              label={t("telemetry.llm_calls")}
              icon={<MessageSquare className="w-3.5 h-3.5 text-accent" />}
              value={<p className="text-2xl font-black tracking-tight" title={totalLlmCalls.toLocaleString()}>{formatCompact(totalLlmCalls)}</p>}
              variant="accent"
            />
            <MetricCard
              label={t("telemetry.tool_calls")}
              icon={<Wrench className="w-3.5 h-3.5 text-brand" />}
              value={<p className="text-2xl font-black tracking-tight" title={totalToolCalls.toLocaleString()}>{formatCompact(totalToolCalls)}</p>}
              variant="brand"
            />
          </div>

          {/* ── Per-Agent Table + HTTP Endpoints ── */}
          <div className="grid gap-6 md:grid-cols-2 stagger-children">
            {/* Per-Agent Token Usage */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><Bot className="h-4 w-4 text-brand" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("telemetry.per_agent")}</h2>
              </div>
              {parsed.agents.length === 0 ? (
                <p className="text-sm text-text-dim text-center py-8">{t("telemetry.no_data")}</p>
              ) : (
                <div className="space-y-3">
                  {agentsByTokens.map((a) => (
                      <div key={a.agent} className="flex items-center gap-3">
                        <span className="text-sm font-semibold flex-1 truncate">{a.agent}</span>
                        <Badge variant="default" className="font-mono text-xs">{a.provider}/{a.model}</Badge>
                        <span className="text-sm font-black text-brand w-20 text-right" title={a.tokens.toLocaleString()}>{formatCompact(a.tokens)}<span className="text-xs font-normal text-text-dim"> tok</span></span>
                      </div>
                    ))}
                </div>
              )}
            </Card>

            {/* Top HTTP Endpoints */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><Globe className="h-4 w-4 text-brand" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("telemetry.top_endpoints")}</h2>
              </div>
              {parsed.requests.length === 0 ? (
                <p className="text-sm text-text-dim text-center py-8">{t("telemetry.no_data")}</p>
              ) : (
                <div className="space-y-3">
                  {requestsByCount.map((r) => (
                      <div key={r.method + r.path + r.status} className="flex items-center gap-3">
                        <Badge variant="default" className="font-mono text-xs w-16 justify-center">{r.method}</Badge>
                        <span className="text-sm font-mono flex-1 truncate">{r.path}</span>
                        <Badge variant={r.status.startsWith("2") ? "success" : r.status.startsWith("3") ? "default" : r.status.startsWith("4") ? "warning" : "error"} className="w-12 justify-center">
                          {r.status}
                        </Badge>
                        <span className="text-sm font-black text-brand w-16 text-right" title={r.count.toLocaleString()}>{formatCompact(r.count)}</span>
                      </div>
                    ))}
                </div>
              )}
            </Card>
          </div>

          {/* ── Raw Prometheus ── */}
          <Card padding="lg">
            <div className="flex items-center justify-between mb-4">
              <div className="flex items-center gap-2">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><ExternalLink className="h-4 w-4 text-brand" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("telemetry.prometheus_endpoint")}</h2>
              </div>
              <a
                href="/api/metrics"
                target="_blank"
                rel="noopener noreferrer"
                className="text-xs text-brand hover:underline"
              >
                {t("telemetry.view_raw")}
              </a>
            </div>
            <pre className="text-xs font-mono bg-main rounded-lg p-4 overflow-auto max-h-64 text-text-dim">
              {metricsQuery.data?.slice(0, 4000) || ""}
            </pre>
            {(metricsQuery.data?.length || 0) > 4000 && (
              <span className="text-xs text-text-dim mt-2 block">...truncated</span>
            )}
          </Card>
        </>
      )}
    </div>
  );
}
