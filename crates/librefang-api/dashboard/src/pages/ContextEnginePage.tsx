import { useId, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Brain, ChevronDown, GitBranch, Gauge, ListChecks, ScrollText, ShieldCheck } from "lucide-react";
import type {
  ContextEngineConfigSnapshot,
  ContextEngineHookCoverage,
  ContextEngineHookName,
  ContextEngineMode,
  ContextEngineSandboxPlugin,
  ContextEngineTrace,
} from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { EmptyState } from "../components/ui/EmptyState";
import { ErrorState } from "../components/ui/ErrorState";
import { CardSkeleton } from "../components/ui/Skeleton";
import { StaggerList } from "../components/ui/StaggerList";
import {
  useContextEngineChain,
  useContextEngineConfig,
  useContextEngineHealth,
  useContextEngineMetrics,
  useContextEngineSandboxPolicy,
  useContextEngineTraces,
} from "../lib/queries/contextEngine";

const HOOK_NAMES: ContextEngineHookName[] = [
  "ingest",
  "after_turn",
  "bootstrap",
  "assemble",
  "compact",
  "transform_tool_result",
  "prepare_subagent",
  "merge_subagent",
];

const CHAIN_HOOKS: (keyof ContextEngineHookCoverage)[] = [
  "ingest",
  "after_turn",
  "assemble",
  "compact",
  "bootstrap",
  "prepare_subagent",
  "merge_subagent",
];

const MAX_TRACES = 50;

function modeLabelKey(mode: ContextEngineMode): string {
  if (mode === "stacked") return "context_engine.chain.mode_stacked";
  if (mode === "single") return "context_engine.chain.mode_single";
  return "context_engine.chain.mode_default";
}

function previewText(value: unknown): string {
  if (value === undefined) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value) ?? "";
  } catch {
    return String(value);
  }
}

function formatStarted(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleTimeString();
}

function SectionHeader({ icon, title, badge }: { icon: ReactNode; title: string; badge?: ReactNode }) {
  return (
    <div className="mb-4 flex items-center gap-2">
      <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-brand/10">{icon}</div>
      <h2 className="text-sm font-black uppercase tracking-tight">{title}</h2>
      {badge}
    </div>
  );
}

function ValueRow({ label, children, mono }: { label: string; children: ReactNode; mono?: boolean }) {
  return (
    <div className="flex items-center justify-between gap-3 border-b border-border-subtle/50 py-1.5 last:border-0">
      <span className="text-xs text-text-dim">{label}</span>
      <span className={`text-right text-xs text-text-main ${mono ? "font-mono" : ""}`}>{children}</span>
    </div>
  );
}

function Flag({ label, enabled }: { label: string; enabled: boolean | undefined }) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center justify-between gap-3 py-1.5">
      <span className="text-xs text-text-dim">{label}</span>
      <Badge variant={enabled ? "success" : "default"}>
        {enabled ? t("context_engine.sandbox.allowed") : t("context_engine.sandbox.denied")}
      </Badge>
    </div>
  );
}

function HealthCard() {
  const { t } = useTranslation();
  const healthQuery = useContextEngineHealth();
  const health = healthQuery.data ?? null;

  return (
    <Card padding="lg" data-testid="context-engine-health">
      <SectionHeader
        icon={<Gauge className="h-4 w-4 text-brand" />}
        title={t("context_engine.health.title")}
        badge={
          health ? (
            <Badge variant={health.healthy ? "success" : "error"} dot className="ml-auto">
              {health.healthy ? t("context_engine.health.healthy") : t("context_engine.health.unhealthy")}
            </Badge>
          ) : undefined
        }
      />

      {health === null ? (
        <div>
          <p className="text-sm font-bold">{t("context_engine.no_engine")}</p>
          <p className="mt-1 text-xs text-text-dim">{t("context_engine.no_engine_desc")}</p>
        </div>
      ) : (
        <div className="space-y-4">
          <div>
            <p className="mb-1.5 text-[10px] font-bold uppercase tracking-wider text-text-dim/60">
              {t("context_engine.health.active_plugins")}
            </p>
            <div className="flex flex-wrap gap-1.5">
              {health.active_plugins.map((plugin) => (
                <Badge key={plugin} variant="brand">{plugin}</Badge>
              ))}
            </div>
          </div>
          <div>
            <p className="mb-1.5 text-[10px] font-bold uppercase tracking-wider text-text-dim/60">
              {t("context_engine.health.issues")}
            </p>
            {health.issues.length === 0 ? (
              <p className="text-xs text-text-dim">{t("context_engine.health.no_issues")}</p>
            ) : (
              <ul className="space-y-2">
                {health.issues.map((issue) => (
                  <li key={issue.plugin} className="rounded-lg bg-main/40 px-3 py-2">
                    <p className="text-xs font-bold">{issue.plugin}</p>
                    <ul className="mt-1 list-disc pl-4 text-[11px] text-error">
                      {issue.errors.map((error) => (
                        <li key={error} className="break-words">{error}</li>
                      ))}
                    </ul>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      )}
    </Card>
  );
}

function ChainCard() {
  const { t } = useTranslation();
  const chainQuery = useContextEngineChain();
  const chain = chainQuery.data;

  return (
    <Card padding="lg" data-testid="context-engine-chain">
      <SectionHeader
        icon={<GitBranch className="h-4 w-4 text-brand" />}
        title={t("context_engine.chain.title")}
        badge={chain ? <Badge variant="brand" className="ml-auto">{t(modeLabelKey(chain.mode))}</Badge> : undefined}
      />
      {chain ? (
        <>
          <div className="mb-3 space-y-1">
            <ValueRow label={t("context_engine.chain.mode")}>{t(modeLabelKey(chain.mode))}</ValueRow>
            <ValueRow label={t("context_engine.chain.length")}>{chain.chain_length}</ValueRow>
            <ValueRow label={t("context_engine.chain.fallback")}>{chain.fallback}</ValueRow>
          </div>

          {chain.chain.length === 0 ? (
            <p className="text-xs text-text-dim">{t("context_engine.no_engine")}</p>
          ) : (
            <ol className="space-y-2">
              {chain.chain.map((entry, index) => {
                const hooks = entry.hooks;
                return (
                  <li key={`${entry.plugin}-${index}`} className="flex gap-2.5 rounded-lg bg-main/40 px-3 py-2">
                    <span className="mt-0.5 font-mono text-[10px] text-text-dim">{index + 1}</span>
                    <div className="min-w-0 flex-1">
                      <p className="text-xs font-bold">{entry.plugin}</p>
                      {"error" in hooks ? (
                        <p className="mt-1 text-[11px] text-error">
                          {t("context_engine.chain.plugin_error")}: {hooks.error}
                        </p>
                      ) : (
                        <div className="mt-1 flex flex-wrap items-center gap-1">
                          {CHAIN_HOOKS.filter((hook) => hooks[hook] === true).map((hook) => (
                            <Badge key={hook} variant="info">{hook}</Badge>
                          ))}
                          {hooks.runtime ? <Badge variant="brand">{hooks.runtime}</Badge> : null}
                          {!hooks.enabled ? (
                            <Badge variant="warning">{t("context_engine.chain.disabled")}</Badge>
                          ) : null}
                          {!hooks.hooks_valid ? (
                            <Badge variant="error">{t("context_engine.chain.invalid")}</Badge>
                          ) : null}
                          {hooks.cache_ttl_secs ? (
                            <span className="text-[10px] text-text-dim">
                              {t("context_engine.chain.cache_ttl")} {hooks.cache_ttl_secs}s
                            </span>
                          ) : null}
                        </div>
                      )}
                    </div>
                  </li>
                );
              })}
            </ol>
          )}
        </>
      ) : null}
    </Card>
  );
}

function MetricsCard() {
  const { t } = useTranslation();
  const metricsQuery = useContextEngineMetrics();
  const metrics = metricsQuery.data ?? null;

  return (
    <Card padding="lg" data-testid="context-engine-metrics">
      <SectionHeader
        icon={<ListChecks className="h-4 w-4 text-brand" />}
        title={t("context_engine.metrics.title")}
      />
      {metrics === null ? (
        <p className="text-xs text-text-dim">{t("context_engine.no_engine")}</p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-left text-xs">
            <thead>
              <tr className="border-b border-border-subtle text-[10px] uppercase tracking-wider text-text-dim/60">
                <th scope="col" className="py-2 pr-3 font-bold">{t("context_engine.metrics.hook")}</th>
                <th scope="col" className="px-3 py-2 text-right font-bold">{t("context_engine.metrics.calls")}</th>
                <th scope="col" className="px-3 py-2 text-right font-bold">{t("context_engine.metrics.successes")}</th>
                <th scope="col" className="px-3 py-2 text-right font-bold">{t("context_engine.metrics.failures")}</th>
                <th scope="col" className="py-2 pl-3 text-right font-bold">{t("context_engine.metrics.avg_latency")}</th>
              </tr>
            </thead>
            <tbody>
              {HOOK_NAMES.map((hook) => {
                const stats = metrics[hook];
                const avgMs = stats.calls > 0 ? Math.round(stats.total_ms / stats.calls) : 0;
                return (
                  <tr key={hook} className="border-b border-border-subtle/50 last:border-0">
                    <td className="py-2 pr-3 font-mono font-bold">{hook}</td>
                    <td className="px-3 py-2 text-right tabular-nums">{stats.calls}</td>
                    <td className="px-3 py-2 text-right tabular-nums text-success">{stats.successes}</td>
                    <td className={`px-3 py-2 text-right tabular-nums ${stats.failures > 0 ? "text-error" : ""}`}>
                      {stats.failures}
                    </td>
                    <td className="py-2 pl-3 text-right tabular-nums">{avgMs} ms</td>
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

function TraceRow({ trace }: { trace: ContextEngineTrace }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const panelId = useId();
  const preview = previewText(trace.input_preview);

  return (
    <div
      data-testid="context-engine-trace"
      className="rounded-lg border border-border-subtle/60 bg-main/30"
    >
      <button
        type="button"
        aria-expanded={expanded}
        aria-controls={panelId}
        onClick={() => setExpanded((open) => !open)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-main/50"
      >
        <ChevronDown
          aria-hidden="true"
          className={`h-3.5 w-3.5 shrink-0 text-text-dim transition-transform ${expanded ? "rotate-180" : ""}`}
        />
        <Badge variant={trace.success ? "success" : "error"}>
          {trace.success ? t("context_engine.traces.success") : t("context_engine.traces.failure")}
        </Badge>
        <span className="font-mono text-[11px] font-bold">{trace.hook}</span>
        <span className="shrink-0 text-[10px] tabular-nums text-text-dim">{trace.elapsed_ms} ms</span>
        {preview ? (
          <span className="hidden min-w-0 flex-1 truncate font-mono text-[10px] text-text-dim/70 sm:block">
            {preview.slice(0, 120)}
          </span>
        ) : null}
        <span className="ml-auto shrink-0 text-[10px] text-text-dim/70">{formatStarted(trace.started_at)}</span>
      </button>
      {expanded ? (
        <div id={panelId} className="space-y-2 border-t border-border-subtle/50 px-3 py-2">
          <p className="text-[10px] text-text-dim">
            {t("context_engine.traces.trace_id")}: <span className="font-mono">{trace.trace_id}</span>
          </p>
          {trace.error ? (
            <p className="break-words text-[11px] text-error">
              {t("context_engine.traces.error")}: {trace.error}
            </p>
          ) : null}
          <div className="grid gap-2 md:grid-cols-2">
            <div>
              <p className="mb-1 text-[10px] font-bold uppercase tracking-wider text-text-dim/60">
                {t("context_engine.traces.input")}
              </p>
              <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-main/60 p-2 font-mono text-[10px]">
                {preview}
              </pre>
            </div>
            {trace.output_preview != null ? (
              <div>
                <p className="mb-1 text-[10px] font-bold uppercase tracking-wider text-text-dim/60">
                  {t("context_engine.traces.output")}
                </p>
                <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-main/60 p-2 font-mono text-[10px]">
                  {previewText(trace.output_preview)}
                </pre>
              </div>
            ) : null}
            {trace.annotations != null ? (
              <div className="md:col-span-2">
                <p className="mb-1 text-[10px] font-bold uppercase tracking-wider text-text-dim/60">
                  {t("context_engine.traces.annotations")}
                </p>
                <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-main/60 p-2 font-mono text-[10px]">
                  {previewText(trace.annotations)}
                </pre>
              </div>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function TracesCard() {
  const { t } = useTranslation();
  const tracesQuery = useContextEngineTraces();
  const tracesData = tracesQuery.data ?? null;

  const traces = useMemo(
    () =>
      [...(tracesData?.traces ?? [])]
        .sort((a, b) => Date.parse(b.started_at) - Date.parse(a.started_at))
        .slice(0, MAX_TRACES),
    [tracesData],
  );

  return (
    <Card padding="lg" data-testid="context-engine-traces">
      <SectionHeader
        icon={<ScrollText className="h-4 w-4 text-brand" />}
        title={t("context_engine.traces.title")}
        badge={
          tracesData ? (
            <Badge variant="default" className="ml-auto">
              {t("context_engine.traces.count", { total: tracesData.count })}
            </Badge>
          ) : undefined
        }
      />
      {tracesData === null ? (
        <p className="text-xs text-text-dim">{t("context_engine.no_engine")}</p>
      ) : traces.length === 0 ? (
        <EmptyState
          icon={<ScrollText className="h-6 w-6" />}
          title={t("context_engine.traces.empty")}
          description={t("context_engine.traces.empty_desc")}
        />
      ) : (
        <div className="space-y-2">
          {traces.map((trace) => (
            <TraceRow key={trace.trace_id} trace={trace} />
          ))}
        </div>
      )}
    </Card>
  );
}

function ConfigCard({ config }: { config: ContextEngineConfigSnapshot }) {
  const { t } = useTranslation();
  // The runtime runs the stack when one is configured; `plugin` would name a
  // plugin that is not in the active chain.
  const stack = config.plugin_stack ?? [];
  const plugins = stack.length > 0
    ? stack.join(", ")
    : (config.plugin ?? t("common.none"));

  return (
    <Card padding="lg" data-testid="context-engine-config">
      <SectionHeader
        icon={<Brain className="h-4 w-4 text-brand" />}
        title={t("context_engine.config.title")}
      />
      <div className="grid gap-x-8 md:grid-cols-2">
        <ValueRow label={t("context_engine.config.engine")} mono>{config.engine}</ValueRow>
        <ValueRow label={t("context_engine.config.plugin")} mono>{plugins}</ValueRow>
        <ValueRow label={t("context_engine.config.timeout")}>
          {config.hook_timeout_secs === null ? (
            <span className="flex items-center justify-end gap-1.5">
              30s <Badge variant="default">{t("context_engine.chain.mode_default")}</Badge>
            </span>
          ) : (
            `${config.hook_timeout_secs}s`
          )}
        </ValueRow>
        <ValueRow label={t("context_engine.config.on_failure")}>{config.on_hook_failure}</ValueRow>
        <ValueRow label={t("context_engine.config.max_retries")}>{config.max_retries}</ValueRow>
        <ValueRow label={t("context_engine.config.retry_delay")}>{config.retry_delay_ms} ms</ValueRow>
        <ValueRow label={t("context_engine.config.hook_cache_ttl")}>
          {config.hook_cache_ttl_secs ? `${config.hook_cache_ttl_secs}s` : t("common.none")}
        </ValueRow>
        <ValueRow label={t("context_engine.config.assemble_cache_ttl")}>
          {config.assemble_cache_ttl_secs ? `${config.assemble_cache_ttl_secs}s` : t("common.none")}
        </ValueRow>
        <ValueRow label={t("context_engine.config.compact_cache_ttl")}>
          {config.compact_cache_ttl_secs ? `${config.compact_cache_ttl_secs}s` : t("common.none")}
        </ValueRow>
        <ValueRow label={t("context_engine.config.allow_network")}>
          <Badge variant={config.allow_network ? "success" : "default"}>
            {config.allow_network ? t("runtime.enabled") : t("runtime.disabled")}
          </Badge>
        </ValueRow>
        <ValueRow label={t("context_engine.config.allow_filesystem")}>
          <Badge variant={config.allow_filesystem ? "success" : "default"}>
            {config.allow_filesystem ? t("runtime.enabled") : t("runtime.disabled")}
          </Badge>
        </ValueRow>
        <ValueRow label={t("context_engine.config.max_memory")}>
          {config.max_memory_mb ? `${config.max_memory_mb} MB` : t("context_engine.sandbox.unlimited")}
        </ValueRow>
        <ValueRow label={t("context_engine.config.circuit_breaker")}>
          {config.circuit_breaker
            ? `${config.circuit_breaker.max_failures} / ${config.circuit_breaker.reset_secs}s`
            : t("common.none")}
        </ValueRow>
        <ValueRow label={t("context_engine.config.registries")}>
          {config.registries.length > 0
            ? config.registries.map((registry) => registry.name).join(", ")
            : t("common.none")}
        </ValueRow>
      </div>
    </Card>
  );
}

function SandboxPluginCard({ plugin }: { plugin: ContextEngineSandboxPlugin }) {
  const { t } = useTranslation();

  return (
    <div className="rounded-xl border border-border-subtle/60 bg-main/30 p-3">
      <p className="mb-1 text-xs font-bold">{plugin.name}</p>
      {plugin.error ? (
        <p className="break-words text-[11px] text-error">
          {t("context_engine.sandbox.plugin_error")}: {plugin.error}
        </p>
      ) : (
        <div>
          <Flag label={t("context_engine.sandbox.filesystem")} enabled={plugin.allow_filesystem} />
          <Flag label={t("context_engine.sandbox.network")} enabled={plugin.allow_network} />
          <ValueRow label={t("context_engine.sandbox.memory")}>
            {plugin.max_memory_mb ? `${plugin.max_memory_mb} MB` : t("context_engine.sandbox.unlimited")}
          </ValueRow>
          <ValueRow label={t("context_engine.sandbox.persistent")}>
            {plugin.persistent_subprocess ? t("runtime.enabled") : t("runtime.disabled")}
          </ValueRow>
          <ValueRow label={t("context_engine.sandbox.prewarm")}>
            {plugin.prewarm_subprocesses ? t("runtime.enabled") : t("runtime.disabled")}
          </ValueRow>
          <ValueRow label={t("context_engine.sandbox.queue_depth")}>
            {plugin.after_turn_queue_depth ?? 0}
          </ValueRow>
          {plugin.circuit_breaker ? (
            <ValueRow label={t("context_engine.sandbox.circuit_breaker")}>
              {plugin.circuit_breaker.max_failures} / {plugin.circuit_breaker.reset_secs}s
            </ValueRow>
          ) : null}
          {plugin.otel_endpoint ? (
            <ValueRow label={t("context_engine.sandbox.otel_endpoint")} mono>{plugin.otel_endpoint}</ValueRow>
          ) : null}
        </div>
      )}
    </div>
  );
}

function SandboxCard() {
  const { t } = useTranslation();
  const sandboxQuery = useContextEngineSandboxPolicy();
  const sandbox = sandboxQuery.data;

  return (
    <Card padding="lg" data-testid="context-engine-sandbox">
      <SectionHeader
        icon={<ShieldCheck className="h-4 w-4 text-brand" />}
        title={t("context_engine.sandbox.title")}
        badge={
          sandbox ? <Badge variant="default" className="ml-auto">{t(modeLabelKey(sandbox.mode))}</Badge> : undefined
        }
      />
      {sandbox == null ? (
        <div>
          <p className="text-sm font-bold">{t("context_engine.no_engine")}</p>
          <p className="mt-1 text-xs text-text-dim">{t("context_engine.no_engine_desc")}</p>
        </div>
      ) : sandbox.mode === "default" ? (
        <div>
          <p className="mb-2 text-xs font-bold">{t("context_engine.sandbox.default_mode")}</p>
          {sandbox.global_defaults ? (
            <div>
              <Flag
                label={t("context_engine.sandbox.filesystem")}
                enabled={sandbox.global_defaults.allow_filesystem}
              />
              <Flag
                label={t("context_engine.sandbox.network")}
                enabled={sandbox.global_defaults.allow_network}
              />
              <ValueRow label={t("context_engine.sandbox.memory")}>
                {sandbox.global_defaults.max_memory_mb
                  ? `${sandbox.global_defaults.max_memory_mb} MB`
                  : t("context_engine.sandbox.unlimited")}
              </ValueRow>
            </div>
          ) : null}
        </div>
      ) : (
        <div className="grid gap-3 md:grid-cols-2">
          {sandbox.plugins.map((plugin) => (
            <SandboxPluginCard key={plugin.name} plugin={plugin} />
          ))}
        </div>
      )}
    </Card>
  );
}

export function ContextEnginePage() {
  const { t } = useTranslation();
  const healthQuery = useContextEngineHealth();
  const chainQuery = useContextEngineChain();
  const metricsQuery = useContextEngineMetrics();
  const tracesQuery = useContextEngineTraces();
  const configQuery = useContextEngineConfig();
  const sandboxQuery = useContextEngineSandboxPolicy();

  const queries = [healthQuery, chainQuery, metricsQuery, tracesQuery, configQuery, sandboxQuery];
  const isLoading = queries.some((query) => query.isLoading);
  const isError = queries.some((query) => query.isError);
  const isFetching = queries.some((query) => query.isFetching);

  function refreshAll() {
    void healthQuery.refetch();
    void chainQuery.refetch();
    void metricsQuery.refetch();
    void tracesQuery.refetch();
    void configQuery.refetch();
    void sandboxQuery.refetch();
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("context_engine.badge")}
        title={t("context_engine.title")}
        subtitle={t("context_engine.subtitle")}
        isFetching={isFetching}
        onRefresh={refreshAll}
        icon={<Brain className="h-4 w-4" />}
      />

      {isLoading ? (
        <StaggerList className="grid gap-4">
          <CardSkeleton />
          <CardSkeleton />
          <CardSkeleton />
        </StaggerList>
      ) : isError ? (
        <ErrorState message={t("context_engine.load_error")} onRetry={refreshAll} />
      ) : (
        <>
          <StaggerList className="grid gap-4 md:grid-cols-2">
            <HealthCard />
            <ChainCard />
          </StaggerList>
          <MetricsCard />
          {configQuery.data ? <ConfigCard config={configQuery.data} /> : null}
          <SandboxCard />
          <TracesCard />
        </>
      )}
    </div>
  );
}
