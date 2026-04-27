import { formatCompact } from "../lib/format";
import { formatUptime } from "../lib/datetime";
import { useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useTelemetryMetrics } from "../lib/queries/telemetry";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { CardSkeleton } from "../components/ui/Skeleton";
import { Badge } from "../components/ui/Badge";
import { AnimatedNumber } from "../components/ui/AnimatedNumber";
import { ErrorState } from "../components/ui/ErrorState";
import {
  Activity, BarChart3, Clock, Globe, Zap, CheckCircle2,
  ExternalLink, Cpu, DollarSign, Bot, Wrench, MessageSquare, AlertTriangle,
  RotateCcw, Users, ChevronDown, ChevronUp,
} from "lucide-react";
import { StaggerList } from "../components/ui/StaggerList";

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
  const gaugeMap = new Map<string, number>();
  const lines = text.split("\n");
  let version = "";

  for (const line of lines) {
    if (line.startsWith("#")) continue;

    const spaceIdx = line.indexOf(" ");
    if (spaceIdx > 0) {
      const namePart = line.slice(0, spaceIdx);
      if (!namePart.includes("{")) {
        gaugeMap.set(namePart, parseFloat(line.slice(spaceIdx + 1)) || 0);
      }
    }

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

  return {
    requests,
    // Skip rollup rows emitted for namespace-like aggregate metrics.
    agents: Array.from(agentMap.values()).filter(a => !a.agent.includes(":")),
    system,
  };
}

// Roll up `(method, path, status)` rows into one row per `(method, path)`,
// with the status counts collapsed into a per-class summary. Lets the
// endpoints panel show one line per endpoint instead of one line per
// (endpoint, status) tuple — much easier to scan when a single endpoint
// has a mix of 2xx/4xx/5xx hits.
interface EndpointRollup {
  method: string;
  path: string;
  total: number;
  ok: number;       // 2xx
  redirect: number; // 3xx
  client: number;   // 4xx
  server: number;   // 5xx
}

function rollupEndpoints(rows: HttpMetric[]): EndpointRollup[] {
  const map = new Map<string, EndpointRollup>();
  for (const r of rows) {
    const key = `${r.method} ${r.path}`;
    let bucket = map.get(key);
    if (!bucket) {
      bucket = { method: r.method, path: r.path, total: 0, ok: 0, redirect: 0, client: 0, server: 0 };
      map.set(key, bucket);
    }
    bucket.total += r.count;
    const c = r.status.charAt(0);
    if (c === "2") bucket.ok += r.count;
    else if (c === "3") bucket.redirect += r.count;
    else if (c === "4") bucket.client += r.count;
    else if (c === "5") bucket.server += r.count;
  }
  return Array.from(map.values()).sort((a, b) => b.total - a.total);
}

// ── Small atoms ──────────────────────────────────────────────────────

type MetricVariant = "success" | "brand" | "accent" | "warning" | "error";

const VARIANT_BG: Record<MetricVariant, string> = {
  success: "bg-success/10",
  brand: "bg-brand/10",
  accent: "bg-accent/10",
  warning: "bg-warning/10",
  error: "bg-error/10",
};

function MetricCard({ label, icon, value, variant, sub }: {
  label: string;
  icon: ReactNode;
  value: ReactNode;
  variant: MetricVariant;
  sub?: ReactNode;
}) {
  return (
    <Card hover padding="md">
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{label}</span>
        <div className={`w-7 h-7 rounded-lg ${VARIANT_BG[variant]} flex items-center justify-center`}>{icon}</div>
      </div>
      <div className="mt-1.5">{value}</div>
      {sub && <div className="mt-1 text-[10px] font-medium text-text-dim/60">{sub}</div>}
    </Card>
  );
}

function SectionHeader({ icon, label, badge }: { icon: ReactNode; label: string; badge?: ReactNode }) {
  return (
    <div className="flex items-center gap-2 mt-2">
      {icon}
      <h2 className="text-sm font-black tracking-tight uppercase">{label}</h2>
      {badge}
    </div>
  );
}

// Inline status distribution bar — width is proportional inside each
// row so the eye can compare endpoint health without doing arithmetic.
function StatusBar({ ok, redirect, client, server, total }: {
  ok: number; redirect: number; client: number; server: number; total: number;
}) {
  if (total === 0) return null;
  const pct = (n: number) => (n / total) * 100;
  return (
    <div
      className="flex h-1.5 w-24 rounded-full overflow-hidden bg-border-subtle/40 shrink-0"
      title={`${ok} OK · ${redirect} 3xx · ${client} 4xx · ${server} 5xx`}
    >
      {ok > 0 && <div style={{ width: `${pct(ok)}%` }} className="bg-success" />}
      {redirect > 0 && <div style={{ width: `${pct(redirect)}%` }} className="bg-text-dim/40" />}
      {client > 0 && <div style={{ width: `${pct(client)}%` }} className="bg-warning" />}
      {server > 0 && <div style={{ width: `${pct(server)}%` }} className="bg-error" />}
    </div>
  );
}

// ── Component ────────────────────────────────────────────────────────

export function TelemetryPage() {
  const { t } = useTranslation();
  const metricsQuery = useTelemetryMetrics();
  const [rawExpanded, setRawExpanded] = useState(false);

  const parsed = useMemo(
    () =>
      metricsQuery.data
        ? parseMetrics(metricsQuery.data)
        : { requests: [], agents: [], system: { uptime: 0, agentsActive: 0, agentsTotal: 0, activeSessions: 0, costToday: 0, panics: 0, restarts: 0, version: "" } },
    [metricsQuery.data],
  );

  const {
    totalRequests, totalTokens, totalInput, totalOutput, totalLlmCalls, totalToolCalls,
    errorCount,
  } = useMemo(() => {
    let totalRequests = 0;
    let errorCount = 0;
    for (const r of parsed.requests) {
      totalRequests += r.count;
      const c = r.status.charAt(0);
      if (c === "4" || c === "5") errorCount += r.count;
    }
    return {
      totalRequests,
      errorCount,
      totalTokens: parsed.agents.reduce((s, a) => s + a.tokens, 0),
      totalInput: parsed.agents.reduce((s, a) => s + a.inputTokens, 0),
      totalOutput: parsed.agents.reduce((s, a) => s + a.outputTokens, 0),
      totalLlmCalls: parsed.agents.reduce((s, a) => s + a.llmCalls, 0),
      totalToolCalls: parsed.agents.reduce((s, a) => s + a.toolCalls, 0),
    };
  }, [parsed]);

  const errorRate = totalRequests > 0 ? (errorCount / totalRequests) * 100 : 0;

  const agentsByTokens = useMemo(
    () => [...parsed.agents].sort((a, b) => b.tokens - a.tokens),
    [parsed.agents],
  );

  const endpointRollups = useMemo(
    () => rollupEndpoints(parsed.requests).slice(0, 12),
    [parsed.requests],
  );

  const lastUpdated = useMemo(
    () => (metricsQuery.dataUpdatedAt ? new Date(metricsQuery.dataUpdatedAt) : null),
    [metricsQuery.dataUpdatedAt],
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
        actions={
          lastUpdated ? (
            <span className="text-[11px] font-medium text-text-dim/70">
              {t("telemetry.last_updated")} {lastUpdated.toLocaleTimeString()}
            </span>
          ) : undefined
        }
      />

      {metricsQuery.isLoading ? (
        <div className="grid gap-4 md:grid-cols-4">
          <CardSkeleton /><CardSkeleton /><CardSkeleton /><CardSkeleton />
        </div>
      ) : metricsQuery.isError ? (
        <ErrorState onRetry={() => void metricsQuery.refetch()} />
      ) : (
        <>
          {/* ── Health ── */}
          <SectionHeader
            icon={<Cpu className="h-4 w-4 text-brand" />}
            label={t("telemetry.section_health")}
            badge={parsed.system.version ? <Badge variant="brand" className="ml-2">v{parsed.system.version}</Badge> : undefined}
          />
          <StaggerList className="grid grid-cols-2 gap-3 sm:gap-4 md:grid-cols-4">
            <MetricCard
              label={t("telemetry.status")}
              icon={<CheckCircle2 className="w-3.5 h-3.5 text-success" />}
              value={
                <div className="flex items-center gap-2">
                  <span className="relative flex h-2 w-2">
                    <span className="absolute inline-flex h-full w-full rounded-full bg-success opacity-75 animate-pulse" />
                    <span className="relative inline-flex rounded-full h-2 w-2 bg-success" />
                  </span>
                  <span className="text-sm font-bold text-success">{t("telemetry.collecting")}</span>
                </div>
              }
              variant="success"
            />
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
              label={t("telemetry.cost_today")}
              icon={<DollarSign className="w-3.5 h-3.5 text-warning" />}
              value={<span className="text-xl font-black tracking-tight"><AnimatedNumber value={parsed.system.costToday} prefix="$" decimals={4} /></span>}
              variant="warning"
            />
          </StaggerList>

          {/* ── Activity ── */}
          <SectionHeader
            icon={<Zap className="h-4 w-4 text-warning" />}
            label={t("telemetry.section_activity")}
            badge={<Badge variant="default" className="ml-2">{t("telemetry.tokens_1h")}</Badge>}
          />
          <StaggerList className="grid grid-cols-2 gap-3 sm:gap-4 md:grid-cols-4">
            <MetricCard
              label={t("telemetry.active_sessions")}
              icon={<Users className="w-3.5 h-3.5 text-accent" />}
              value={<span className="text-xl font-black tracking-tight"><AnimatedNumber value={parsed.system.activeSessions} /></span>}
              variant="accent"
            />
            <MetricCard
              label={t("telemetry.total_requests")}
              icon={<BarChart3 className="w-3.5 h-3.5 text-brand" />}
              value={<p className="text-xl font-black tracking-tight" title={totalRequests.toLocaleString()}>{formatCompact(totalRequests)}</p>}
              variant="brand"
            />
            <MetricCard
              label={t("telemetry.total_tokens")}
              icon={<BarChart3 className="w-3.5 h-3.5 text-brand" />}
              value={<p className="text-xl font-black tracking-tight text-brand" title={totalTokens.toLocaleString()}>{formatCompact(totalTokens)}</p>}
              variant="brand"
              sub={
                <span className="font-mono">
                  <span className="text-success">{formatCompact(totalInput)}</span>
                  <span className="text-text-dim/50"> {t("telemetry.input_short")} · </span>
                  <span className="text-warning">{formatCompact(totalOutput)}</span>
                  <span className="text-text-dim/50"> {t("telemetry.output_short")}</span>
                </span>
              }
            />
            <MetricCard
              label={t("telemetry.llm_calls")}
              icon={<MessageSquare className="w-3.5 h-3.5 text-accent" />}
              value={<p className="text-xl font-black tracking-tight" title={totalLlmCalls.toLocaleString()}>{formatCompact(totalLlmCalls)}</p>}
              variant="accent"
              sub={
                <span className="font-mono">
                  <Wrench className="inline w-3 h-3 mr-0.5 text-text-dim/60" />
                  {formatCompact(totalToolCalls)} {t("telemetry.tools")}
                </span>
              }
            />
          </StaggerList>

          {/* ── Reliability ── */}
          <SectionHeader
            icon={<AlertTriangle className="h-4 w-4 text-error" />}
            label={t("telemetry.section_reliability")}
          />
          <StaggerList className="grid grid-cols-2 gap-3 sm:gap-4 md:grid-cols-3">
            <MetricCard
              label={t("telemetry.panics")}
              icon={<AlertTriangle className={`w-3.5 h-3.5 ${parsed.system.panics > 0 ? "text-error" : "text-text-dim/40"}`} />}
              value={
                <span className={`text-xl font-black tracking-tight ${parsed.system.panics > 0 ? "text-error" : ""}`}>
                  <AnimatedNumber value={parsed.system.panics} />
                </span>
              }
              variant={parsed.system.panics > 0 ? "error" : "success"}
            />
            <MetricCard
              label={t("telemetry.restarts")}
              icon={<RotateCcw className={`w-3.5 h-3.5 ${parsed.system.restarts > 0 ? "text-warning" : "text-text-dim/40"}`} />}
              value={
                <span className={`text-xl font-black tracking-tight ${parsed.system.restarts > 0 ? "text-warning" : ""}`}>
                  <AnimatedNumber value={parsed.system.restarts} />
                </span>
              }
              variant={parsed.system.restarts > 0 ? "warning" : "success"}
            />
            <MetricCard
              label={t("telemetry.error_rate")}
              icon={<AlertTriangle className={`w-3.5 h-3.5 ${errorRate > 1 ? "text-error" : "text-text-dim/40"}`} />}
              value={
                <p className={`text-xl font-black tracking-tight ${errorRate > 1 ? "text-error" : errorRate > 0 ? "text-warning" : ""}`}>
                  {errorRate.toFixed(errorRate > 0 && errorRate < 0.01 ? 3 : 2)}%
                </p>
              }
              variant={errorRate > 1 ? "error" : errorRate > 0 ? "warning" : "success"}
              sub={totalRequests > 0 ? `${errorCount.toLocaleString()} / ${totalRequests.toLocaleString()}` : undefined}
            />
          </StaggerList>

          {/* ── Per-Agent + HTTP Endpoints ── */}
          <StaggerList className="grid gap-6 lg:grid-cols-2">
            {/* Per-Agent Token Usage */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><Bot className="h-4 w-4 text-brand" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("telemetry.per_agent")}</h2>
                {agentsByTokens.length > 0 && <Badge variant="default" className="ml-auto">{agentsByTokens.length}</Badge>}
              </div>
              {agentsByTokens.length === 0 ? (
                <p className="text-sm text-text-dim text-center py-8">{t("telemetry.no_data")}</p>
              ) : (
                <div className="space-y-4">
                  {agentsByTokens.map((a) => {
                    // Fall back to total if input/output histograms are
                    // empty (older daemon, or pre-instrumentation traffic).
                    const inOut = a.inputTokens + a.outputTokens;
                    const inputPct = inOut > 0 ? (a.inputTokens / inOut) * 100 : 0;
                    return (
                      <div key={a.agent} className="space-y-1.5">
                        <div className="flex items-center gap-3">
                          <span className="text-sm font-semibold flex-1 truncate">{a.agent}</span>
                          {(a.provider || a.model) && (
                            <Badge variant="default" className="font-mono text-[10px]">{a.provider}/{a.model}</Badge>
                          )}
                          <span className="text-sm font-black text-brand text-right tabular-nums" title={a.tokens.toLocaleString()}>
                            {formatCompact(a.tokens)}
                            <span className="text-[10px] font-normal text-text-dim ml-0.5">tok</span>
                          </span>
                        </div>
                        {/* in/out split — green = input, amber = output */}
                        {inOut > 0 && (
                          <div
                            className="flex h-1 w-full rounded-full overflow-hidden bg-border-subtle/30"
                            title={`${a.inputTokens.toLocaleString()} in · ${a.outputTokens.toLocaleString()} out`}
                          >
                            <div style={{ width: `${inputPct}%` }} className="bg-success/60" />
                            <div style={{ width: `${100 - inputPct}%` }} className="bg-warning/60" />
                          </div>
                        )}
                        <div className="flex items-center gap-3 text-[10px] font-mono text-text-dim/70">
                          <span><span className="text-success">{formatCompact(a.inputTokens)}</span> {t("telemetry.input_short")}</span>
                          <span><span className="text-warning">{formatCompact(a.outputTokens)}</span> {t("telemetry.output_short")}</span>
                          <span className="ml-auto">
                            {formatCompact(a.llmCalls)} {t("telemetry.calls")} · {formatCompact(a.toolCalls)} {t("telemetry.tools")}
                          </span>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </Card>

            {/* HTTP Endpoints — rolled up by (method, path) */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><Globe className="h-4 w-4 text-brand" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("telemetry.top_endpoints")}</h2>
                {endpointRollups.length > 0 && <Badge variant="default" className="ml-auto">{endpointRollups.length}</Badge>}
              </div>
              {endpointRollups.length === 0 ? (
                <p className="text-sm text-text-dim text-center py-8">{t("telemetry.no_data")}</p>
              ) : (
                <div className="space-y-2.5">
                  {endpointRollups.map((r) => (
                    <div key={r.method + r.path} className="flex items-center gap-3">
                      <Badge variant="default" className="font-mono text-[10px] w-14 justify-center shrink-0">{r.method}</Badge>
                      <span className="text-xs font-mono flex-1 truncate" title={r.path}>{r.path}</span>
                      <StatusBar ok={r.ok} redirect={r.redirect} client={r.client} server={r.server} total={r.total} />
                      <span className="text-sm font-black text-brand text-right tabular-nums w-14" title={r.total.toLocaleString()}>{formatCompact(r.total)}</span>
                    </div>
                  ))}
                </div>
              )}
            </Card>
          </StaggerList>

          {/* ── Raw Prometheus (collapsible) ── */}
          <Card padding="lg">
            <button
              onClick={() => setRawExpanded(v => !v)}
              className="w-full flex items-center justify-between"
              aria-expanded={rawExpanded}
            >
              <div className="flex items-center gap-2">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><ExternalLink className="h-4 w-4 text-brand" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("telemetry.prometheus_endpoint")}</h2>
              </div>
              <div className="flex items-center gap-3">
                <a
                  href="/api/metrics"
                  target="_blank"
                  rel="noopener noreferrer"
                  onClick={(e) => e.stopPropagation()}
                  className="text-xs text-brand hover:underline"
                >
                  {t("telemetry.view_raw")}
                </a>
                {rawExpanded ? (
                  <ChevronUp className="h-4 w-4 text-text-dim" />
                ) : (
                  <ChevronDown className="h-4 w-4 text-text-dim" />
                )}
              </div>
            </button>
            {rawExpanded && (
              <>
                <pre className="mt-4 text-xs font-mono bg-main rounded-lg p-4 overflow-auto max-h-96 text-text-dim">
                  {metricsQuery.data?.slice(0, 8000) || ""}
                </pre>
                {(metricsQuery.data?.length || 0) > 8000 && (
                  <span className="text-xs text-text-dim mt-2 block">...truncated</span>
                )}
              </>
            )}
          </Card>
        </>
      )}
    </div>
  );
}
