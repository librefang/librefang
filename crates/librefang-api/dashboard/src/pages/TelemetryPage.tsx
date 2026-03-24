import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { CardSkeleton } from "../components/ui/Skeleton";
import { Badge } from "../components/ui/Badge";
import { AnimatedNumber } from "../components/ui/AnimatedNumber";
import {
  Activity, BarChart3, Clock, Globe, TrendingUp, Zap, CheckCircle2,
  ExternalLink, Cpu, DollarSign, Bot, Wrench, MessageSquare, AlertTriangle, RotateCcw, Users,
} from "lucide-react";

const REFRESH_MS = 5000;

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

function parseGauge(lines: string[], name: string): number {
  for (const line of lines) {
    if (line.startsWith(name + " ") || line.startsWith(name + "{")) {
      // Simple gauge without labels: "metric_name 123"
      if (line.startsWith(name + " ")) {
        return parseFloat(line.slice(name.length + 1)) || 0;
      }
    }
  }
  return 0;
}

function parseMetrics(text: string): ParsedMetrics {
  const requests: HttpMetric[] = [];
  const agentMap = new Map<string, AgentTokenMetric>();
  const lines = text.split("\n");

  // HTTP request metrics
  for (const line of lines) {
    if (line.startsWith("librefang_http_requests_total{")) {
      const match = line.match(
        /librefang_http_requests_total\{method="([^"]+)",path="([^"]+)",status="([^"]+)"\}\s+(\d+)/
      );
      if (match) {
        requests.push({ method: match[1], path: match[2], status: match[3], count: parseInt(match[4], 10) });
      }
    }

    // Per-agent token metrics: librefang_tokens{agent="...",provider="...",model="..."} 123
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
      if (match && agentMap.has(match[1])) {
        agentMap.get(match[1])!.inputTokens = parseFloat(match[2]);
      }
    }
    if (line.startsWith("librefang_tokens_output{")) {
      const match = line.match(/librefang_tokens_output\{agent="([^"]+)"[^}]*\}\s+([\d.]+)/);
      if (match && agentMap.has(match[1])) {
        agentMap.get(match[1])!.outputTokens = parseFloat(match[2]);
      }
    }
    if (line.startsWith("librefang_tool_calls{")) {
      const match = line.match(/librefang_tool_calls\{agent="([^"]+)"[^}]*\}\s+([\d.]+)/);
      if (match && agentMap.has(match[1])) {
        agentMap.get(match[1])!.toolCalls = parseFloat(match[2]);
      }
    }
    if (line.startsWith("librefang_llm_calls{")) {
      const match = line.match(/librefang_llm_calls\{agent="([^"]+)"[^}]*\}\s+([\d.]+)/);
      if (match && agentMap.has(match[1])) {
        agentMap.get(match[1])!.llmCalls = parseFloat(match[2]);
      }
    }
  }

  // Version from librefang_info{version="x.y.z"} 1
  let version = "";
  for (const line of lines) {
    if (line.startsWith("librefang_info{")) {
      const match = line.match(/version="([^"]+)"/);
      if (match) version = match[1];
    }
  }

  const system: SystemMetrics = {
    uptime: parseGauge(lines, "librefang_uptime_seconds"),
    agentsActive: parseGauge(lines, "librefang_agents_active"),
    agentsTotal: parseGauge(lines, "librefang_agents_total"),
    activeSessions: parseGauge(lines, "librefang_active_sessions"),
    costToday: parseGauge(lines, "librefang_cost_usd_today"),
    panics: parseGauge(lines, "librefang_panics_total"),
    restarts: parseGauge(lines, "librefang_restarts_total"),
    version,
  };

  return { requests, agents: Array.from(agentMap.values()), system };
}

// ── Helpers ──────────────────────────────────────────────────────────

function formatUptime(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
  return `${Math.floor(seconds / 86400)}d ${Math.floor((seconds % 86400) / 3600)}h`;
}

function formatCompact(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toLocaleString();
}

// ── Component ────────────────────────────────────────────────────────

async function fetchMetrics(): Promise<string> {
  const res = await fetch("/api/metrics");
  if (!res.ok) throw new Error("Failed to fetch metrics");
  return res.text();
}

export function TelemetryPage() {
  const { t } = useTranslation();
  const metricsQuery = useQuery({
    queryKey: ["telemetry", "metrics"],
    queryFn: fetchMetrics,
    refetchInterval: REFRESH_MS,
  });

  const parsed = metricsQuery.data
    ? parseMetrics(metricsQuery.data)
    : { requests: [], agents: [], system: { uptime: 0, agentsActive: 0, agentsTotal: 0, activeSessions: 0, costToday: 0, panics: 0, restarts: 0, version: "" } };

  const totalRequests = parsed.requests.reduce((sum, r) => sum + r.count, 0);
  const totalTokens = parsed.agents.reduce((sum, a) => sum + a.tokens, 0);
  const totalInput = parsed.agents.reduce((sum, a) => sum + a.inputTokens, 0);
  const totalOutput = parsed.agents.reduce((sum, a) => sum + a.outputTokens, 0);
  const totalLlmCalls = parsed.agents.reduce((sum, a) => sum + a.llmCalls, 0);
  const totalToolCalls = parsed.agents.reduce((sum, a) => sum + a.toolCalls, 0);

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("telemetry.badge")}
        title={t("telemetry.title")}
        subtitle={t("telemetry.subtitle")}
        isFetching={metricsQuery.isFetching}
        onRefresh={() => void metricsQuery.refetch()}
        icon={<Activity className="h-4 w-4" />}
      />

      {metricsQuery.isLoading ? (
        <div className="grid gap-4 md:grid-cols-4">
          <CardSkeleton /><CardSkeleton /><CardSkeleton /><CardSkeleton />
        </div>
      ) : (
        <>
          {/* ── System Health ── */}
          <div className="flex items-center gap-2 mt-2">
            <Cpu className="h-4 w-4 text-brand" />
            <h2 className="text-sm font-black tracking-tight uppercase">{t("telemetry.system_health")}</h2>
            {parsed.system.version && (
              <Badge variant="brand" className="ml-2">v{parsed.system.version}</Badge>
            )}
          </div>
          <div className="grid grid-cols-2 gap-3 sm:gap-4 lg:grid-cols-4 xl:grid-cols-8 stagger-children">
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.uptime")}</span>
                <div className="w-7 h-7 rounded-lg bg-success/10 flex items-center justify-center"><Clock className="w-3.5 h-3.5 text-success" /></div>
              </div>
              <p className="text-xl font-black tracking-tight mt-1">{formatUptime(parsed.system.uptime)}</p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.agents_active")}</span>
                <div className="w-7 h-7 rounded-lg bg-brand/10 flex items-center justify-center"><Bot className="w-3.5 h-3.5 text-brand" /></div>
              </div>
              <p className="text-xl font-black tracking-tight mt-1">
                <AnimatedNumber value={parsed.system.agentsActive} />
                <span className="text-sm font-normal text-text-dim"> / {parsed.system.agentsTotal}</span>
              </p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.active_sessions")}</span>
                <div className="w-7 h-7 rounded-lg bg-accent/10 flex items-center justify-center"><Users className="w-3.5 h-3.5 text-accent" /></div>
              </div>
              <p className="text-xl font-black tracking-tight mt-1"><AnimatedNumber value={parsed.system.activeSessions} /></p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.cost_today")}</span>
                <div className="w-7 h-7 rounded-lg bg-warning/10 flex items-center justify-center"><DollarSign className="w-3.5 h-3.5 text-warning" /></div>
              </div>
              <p className="text-xl font-black tracking-tight mt-1">
                <AnimatedNumber value={parsed.system.costToday} prefix="$" decimals={4} />
              </p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.total_requests")}</span>
                <div className="w-7 h-7 rounded-lg bg-brand/10 flex items-center justify-center"><BarChart3 className="w-3.5 h-3.5 text-brand" /></div>
              </div>
              <p className="text-xl font-black tracking-tight mt-1" title={totalRequests.toLocaleString()}>{formatCompact(totalRequests)}</p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.panics")}</span>
                <div className="w-7 h-7 rounded-lg bg-error/10 flex items-center justify-center"><AlertTriangle className="w-3.5 h-3.5 text-error" /></div>
              </div>
              <p className="text-xl font-black tracking-tight mt-1"><AnimatedNumber value={parsed.system.panics} /></p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.restarts")}</span>
                <div className="w-7 h-7 rounded-lg bg-warning/10 flex items-center justify-center"><RotateCcw className="w-3.5 h-3.5 text-warning" /></div>
              </div>
              <p className="text-xl font-black tracking-tight mt-1"><AnimatedNumber value={parsed.system.restarts} /></p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.status")}</span>
                <div className="w-7 h-7 rounded-lg bg-success/10 flex items-center justify-center"><CheckCircle2 className="w-3.5 h-3.5 text-success" /></div>
              </div>
              <div className="mt-1 flex items-center gap-2">
                <span className="relative flex h-2 w-2">
                  <span className="absolute inline-flex h-full w-full rounded-full bg-success opacity-75 animate-ping" />
                  <span className="relative inline-flex rounded-full h-2 w-2 bg-success" />
                </span>
                <Badge variant="success">{t("telemetry.collecting")}</Badge>
              </div>
            </Card>
          </div>

          {/* ── LLM & Token Usage ── */}
          <div className="flex items-center gap-2 mt-4">
            <Zap className="h-4 w-4 text-warning" />
            <h2 className="text-sm font-black tracking-tight uppercase">{t("telemetry.llm_usage")}</h2>
            <Badge variant="default" className="ml-2">{t("telemetry.tokens_1h")}</Badge>
          </div>
          <div className="grid grid-cols-2 gap-3 sm:gap-4 lg:grid-cols-5 stagger-children">
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.total_tokens")}</span>
                <div className="w-7 h-7 rounded-lg bg-brand/10 flex items-center justify-center"><BarChart3 className="w-3.5 h-3.5 text-brand" /></div>
              </div>
              <p className="text-2xl font-black tracking-tight mt-1 text-brand" title={totalTokens.toLocaleString()}>{formatCompact(totalTokens)}</p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.input_tokens")}</span>
                <div className="w-7 h-7 rounded-lg bg-success/10 flex items-center justify-center"><TrendingUp className="w-3.5 h-3.5 text-success" /></div>
              </div>
              <p className="text-2xl font-black tracking-tight mt-1 text-success" title={totalInput.toLocaleString()}>{formatCompact(totalInput)}</p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.output_tokens")}</span>
                <div className="w-7 h-7 rounded-lg bg-warning/10 flex items-center justify-center"><TrendingUp className="w-3.5 h-3.5 text-warning" /></div>
              </div>
              <p className="text-2xl font-black tracking-tight mt-1 text-warning" title={totalOutput.toLocaleString()}>{formatCompact(totalOutput)}</p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.llm_calls")}</span>
                <div className="w-7 h-7 rounded-lg bg-accent/10 flex items-center justify-center"><MessageSquare className="w-3.5 h-3.5 text-accent" /></div>
              </div>
              <p className="text-2xl font-black tracking-tight mt-1" title={totalLlmCalls.toLocaleString()}>{formatCompact(totalLlmCalls)}</p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.tool_calls")}</span>
                <div className="w-7 h-7 rounded-lg bg-brand/10 flex items-center justify-center"><Wrench className="w-3.5 h-3.5 text-brand" /></div>
              </div>
              <p className="text-2xl font-black tracking-tight mt-1" title={totalToolCalls.toLocaleString()}>{formatCompact(totalToolCalls)}</p>
            </Card>
          </div>

          {/* ── Per-Agent Table + HTTP Endpoints ── */}
          <div className="grid gap-6 lg:grid-cols-2 stagger-children">
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
                  {parsed.agents
                    .sort((a, b) => b.tokens - a.tokens)
                    .map((a, i) => (
                      <div key={i} className="flex items-center gap-3">
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
                  {parsed.requests
                    .sort((a, b) => b.count - a.count)
                    .slice(0, 10)
                    .map((r, i) => (
                      <div key={i} className="flex items-center gap-3">
                        <Badge variant="default" className="font-mono text-xs w-16 justify-center">{r.method}</Badge>
                        <span className="text-sm font-mono flex-1 truncate">{r.path}</span>
                        <Badge variant={r.status.startsWith("2") ? "success" : r.status.startsWith("4") ? "warning" : "error"} className="w-12 justify-center">
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
          </Card>
        </>
      )}
    </div>
  );
}
