import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { CardSkeleton } from "../components/ui/Skeleton";
import { Badge } from "../components/ui/Badge";
import { Activity, BarChart3, Clock, Globe, TrendingUp, Zap, CheckCircle2, ExternalLink } from "lucide-react";

const REFRESH_MS = 5000;

interface HttpMetric {
  method: string;
  path: string;
  status: string;
  count: number;
}

interface LatencyMetric {
  method: string;
  path: string;
  sum: number;
  count: number;
  p50: number;
  p90: number;
  p99: number;
}

function parseMetrics(text: string): { requests: HttpMetric[]; latencies: LatencyMetric[] } {
  const requests: HttpMetric[] = [];
  const latencies: LatencyMetric[] = [];
  
  const lines = text.split('\n');
  for (const line of lines) {
    if (line.startsWith('librefang_http_requests_total{')) {
      const match = line.match(/librefang_http_requests_total\{method="([^"]+)",path="([^"]+)",status="([^"]+)"\} (\d+)/);
      if (match) {
        requests.push({
          method: match[1],
          path: match[2],
          status: match[3],
          count: parseInt(match[4], 10),
        });
      }
    } else if (line.startsWith('librefang_http_request_duration_ms_sum{')) {
      const match = line.match(/librefang_http_request_duration_ms_sum\{method="([^"]+)",path="([^"]+)"\} (\d+)/);
      if (match) {
        latencies.push({
          method: match[1],
          path: match[2],
          sum: parseInt(match[3], 10),
          count: 0,
          p50: 0,
          p90: 0,
          p99: 0,
        });
      }
    } else if (line.startsWith('librefang_http_request_duration_ms_count{')) {
      const match = line.match(/librefang_http_request_duration_ms_count\{method="([^"]+)",path="([^"]+)"\} (\d+)/);
      if (match) {
        const existing = latencies.find(l => l.method === match[1] && l.path === match[2]);
        if (existing) existing.count = parseInt(match[3], 10);
      }
    } else if (line.startsWith('librefang_http_request_duration_ms_p50{')) {
      const match = line.match(/librefang_http_request_duration_ms_p50\{method="([^"]+)",path="([^"]+)"\} (\d+)/);
      if (match) {
        const existing = latencies.find(l => l.method === match[1] && l.path === match[2]);
        if (existing) existing.p50 = parseInt(match[3], 10);
      }
    } else if (line.startsWith('librefang_http_request_duration_ms_p90{')) {
      const match = line.match(/librefang_http_request_duration_ms_p90\{method="([^"]+)",path="([^"]+)"\} (\d+)/);
      if (match) {
        const existing = latencies.find(l => l.method === match[1] && l.path === match[2]);
        if (existing) existing.p90 = parseInt(match[3], 10);
      }
    } else if (line.startsWith('librefang_http_request_duration_ms_p99{')) {
      const match = line.match(/librefang_http_request_duration_ms_p99\{method="([^"]+)",path="([^"]+)"\} (\d+)/);
      if (match) {
        const existing = latencies.find(l => l.method === match[1] && l.path === match[2]);
        if (existing) existing.p99 = parseInt(match[3], 10);
      }
    }
  }
  
  return { requests, latencies };
}

async function fetchMetrics(): Promise<string> {
  const res = await fetch('/api/metrics');
  if (!res.ok) throw new Error('Failed to fetch metrics');
  return res.text();
}

export function TelemetryPage() {
  const { t } = useTranslation();
  const metricsQuery = useQuery({
    queryKey: ["telemetry", "metrics"],
    queryFn: fetchMetrics,
    refetchInterval: REFRESH_MS,
  });

  const parsed = metricsQuery.data ? parseMetrics(metricsQuery.data) : { requests: [], latencies: [] };
  const totalRequests = parsed.requests.reduce((sum, r) => sum + r.count, 0);

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
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4 stagger-children">
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.total_requests")}</span>
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><BarChart3 className="w-4 h-4 text-brand" /></div>
              </div>
              <p className="text-3xl font-black tracking-tight mt-2 text-brand">{totalRequests.toLocaleString()}</p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.endpoints")}</span>
                <div className="w-8 h-8 rounded-lg bg-success/10 flex items-center justify-center"><Globe className="w-4 h-4 text-success" /></div>
              </div>
              <p className="text-3xl font-black tracking-tight mt-2">{parsed.requests.length}</p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.avg_latency")}</span>
                <div className="w-8 h-8 rounded-lg bg-warning/10 flex items-center justify-center"><Clock className="w-4 h-4 text-warning" /></div>
              </div>
              <p className="text-3xl font-black tracking-tight mt-2">
                {parsed.latencies.length > 0 
                  ? Math.round(parsed.latencies.reduce((s, l) => s + (l.sum / Math.max(l.count, 1)), 0) / parsed.latencies.length)
                  : 0}ms
              </p>
            </Card>
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{t("telemetry.status")}</span>
                <div className="w-8 h-8 rounded-lg bg-success/10 flex items-center justify-center"><CheckCircle2 className="w-4 h-4 text-success" /></div>
              </div>
              <div className="mt-2 flex items-center gap-2">
                <span className="relative flex h-2.5 w-2.5">
                  <span className="absolute inline-flex h-full w-full rounded-full bg-success opacity-75 animate-ping" />
                  <span className="relative inline-flex rounded-full h-2.5 w-2.5 bg-success" />
                </span>
                <Badge variant="success">{t("telemetry.collecting")}</Badge>
              </div>
            </Card>
          </div>

          <div className="grid gap-6 lg:grid-cols-2 stagger-children">
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><TrendingUp className="h-4 w-4 text-brand" /></div>
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
                        <Badge variant="outline" className="font-mono text-xs w-16 justify-center">{r.method}</Badge>
                        <span className="text-sm font-mono flex-1 truncate">{r.path}</span>
                        <Badge variant={r.status.startsWith("2") ? "success" : r.status.startsWith("4") ? "warning" : "error"} className="w-12 justify-center">
                          {r.status}
                        </Badge>
                        <span className="text-sm font-black text-brand w-16 text-right">{r.count.toLocaleString()}</span>
                      </div>
                    ))}
                </div>
              )}
            </Card>

            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-warning/10 flex items-center justify-center"><Zap className="h-4 w-4 text-warning" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("telemetry.latency")}</h2>
              </div>
              {parsed.latencies.length === 0 ? (
                <p className="text-sm text-text-dim text-center py-8">{t("telemetry.no_data")}</p>
              ) : (
                <div className="space-y-3">
                  {parsed.latencies
                    .sort((a, b) => b.count - a.count)
                    .slice(0, 10)
                    .map((l, i) => (
                      <div key={i} className="space-y-1">
                        <div className="flex items-center gap-2 text-xs">
                          <Badge variant="outline" className="font-mono w-16 justify-center">{l.method}</Badge>
                          <span className="flex-1 truncate font-mono">{l.path}</span>
                        </div>
                        <div className="flex items-center gap-4 text-xs text-text-dim">
                          <span>p50: <span className="font-black text-success">{l.p50}ms</span></span>
                          <span>p90: <span className="font-black text-warning">{l.p90}ms</span></span>
                          <span>p99: <span className="font-black text-error">{l.p99}ms</span></span>
                        </div>
                      </div>
                    ))}
                </div>
              )}
            </Card>
          </div>

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
              {metricsQuery.data?.slice(0, 3000) || ""}
            </pre>
          </Card>
        </>
      )}
    </div>
  );
}
