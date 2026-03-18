import { useQuery } from "@tanstack/react-query";
import {
  getQueueStatus,
  getVersionInfo,
  listAgents,
  loadDashboardSnapshot,
  type ProviderItem,
  type QueueLaneStatus
} from "../api";

const REFRESH_MS = 30000;

interface RuntimeSnapshot {
  version: Awaited<ReturnType<typeof getVersionInfo>>;
  status: Awaited<ReturnType<typeof loadDashboardSnapshot>>["status"];
  providers: ProviderItem[];
  queueLanes: QueueLaneStatus[];
  agentCount: number;
}

function formatUptime(seconds?: number): string {
  if (!seconds || seconds < 0) return "-";
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
  return `${Math.floor(seconds / 86400)}d ${Math.floor((seconds % 86400) / 3600)}h`;
}

function providerState(provider: ProviderItem): string {
  if (provider.reachable) return "Online";
  if (provider.auth_status?.toLowerCase() === "configured") return "Ready";
  return "Not configured";
}

function providerStateClass(provider: ProviderItem): string {
  if (provider.reachable) return "border-emerald-700 bg-emerald-700/20 text-emerald-200";
  if (provider.auth_status?.toLowerCase() === "configured") {
    return "border-sky-700 bg-sky-700/20 text-sky-200";
  }
  return "border-slate-700 bg-slate-800/60 text-slate-300";
}

async function loadRuntimeSnapshot(): Promise<RuntimeSnapshot> {
  const [dashboard, version, agents, queue] = await Promise.all([
    loadDashboardSnapshot(),
    getVersionInfo(),
    listAgents(),
    getQueueStatus()
  ]);

  return {
    version,
    status: dashboard.status,
    providers: dashboard.providers,
    queueLanes: queue.lanes ?? [],
    agentCount: dashboard.status.agent_count ?? agents.length
  };
}

export function RuntimePage() {
  const runtimeQuery = useQuery({
    queryKey: ["runtime", "snapshot"],
    queryFn: loadRuntimeSnapshot,
    refetchInterval: REFRESH_MS
  });

  const runtime = runtimeQuery.data ?? null;
  const loading = runtimeQuery.isLoading;
  const error = runtimeQuery.error instanceof Error ? runtimeQuery.error.message : "";

  const providers = runtime?.providers ?? [];
  const queueLanes = runtime?.queueLanes ?? [];

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Runtime</h1>
          <p className="text-sm text-slate-400">Daemon runtime health, build info, providers, and queue lanes.</p>
        </div>
        <button
          className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
          onClick={() => void runtimeQuery.refetch()}
          disabled={runtimeQuery.isFetching}
        >
          Refresh
        </button>
      </header>

      {loading ? (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">Loading runtime snapshot...</div>
      ) : null}
      {error ? (
        <div className="rounded-xl border border-rose-700 bg-rose-700/15 p-4 text-rose-200">{error}</div>
      ) : null}

      {runtime ? (
        <>
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <span className="text-sm text-slate-400">Version</span>
              <strong className="mt-1 block text-2xl">{runtime.version.version ?? runtime.status.version ?? "-"}</strong>
            </article>
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <span className="text-sm text-slate-400">Platform</span>
              <strong className="mt-1 block text-2xl">
                {runtime.version.platform ?? "-"} / {runtime.version.arch ?? "-"}
              </strong>
            </article>
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <span className="text-sm text-slate-400">Agents</span>
              <strong className="mt-1 block text-2xl">{runtime.agentCount}</strong>
            </article>
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <span className="text-sm text-slate-400">Uptime</span>
              <strong className="mt-1 block text-2xl">{formatUptime(runtime.status.uptime_seconds)}</strong>
            </article>
          </div>

          <div className="grid gap-3 xl:grid-cols-2">
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <h2 className="m-0 text-base font-semibold">Daemon</h2>
              <dl className="mt-3 grid grid-cols-[120px_1fr] gap-y-2 text-sm">
                <dt className="text-slate-400">Default model</dt>
                <dd>{runtime.status.default_model ?? "-"}</dd>
                <dt className="text-slate-400">API listen</dt>
                <dd>{runtime.status.api_listen ?? "-"}</dd>
                <dt className="text-slate-400">Home dir</dt>
                <dd className="break-all">{runtime.status.home_dir ?? "-"}</dd>
                <dt className="text-slate-400">Log level</dt>
                <dd>{runtime.status.log_level ?? "-"}</dd>
                <dt className="text-slate-400">Network</dt>
                <dd>
                  <span
                    className={`rounded-full border px-2 py-1 text-xs ${
                      runtime.status.network_enabled
                        ? "border-emerald-700 bg-emerald-700/20 text-emerald-200"
                        : "border-slate-700 bg-slate-800/60 text-slate-300"
                    }`}
                  >
                    {runtime.status.network_enabled ? "Enabled" : "Disabled"}
                  </span>
                </dd>
              </dl>
            </article>

            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <h2 className="m-0 text-base font-semibold">Queue Lanes</h2>
              {queueLanes.length === 0 ? (
                <p className="mt-2 text-sm text-slate-400">No queue lanes reported.</p>
              ) : (
                <ul className="mt-3 flex list-none flex-col gap-2 p-0">
                  {queueLanes.map((lane) => (
                    <li
                      key={lane.lane ?? "unknown"}
                      className="grid grid-cols-[1fr_auto_auto] items-center gap-3 rounded-lg border border-slate-800 bg-slate-950/70 px-3 py-2 text-sm"
                    >
                      <span className="font-medium">{lane.lane ?? "-"}</span>
                      <span className="text-slate-400">active {lane.active ?? 0}</span>
                      <span className="text-slate-400">capacity {lane.capacity ?? 0}</span>
                    </li>
                  ))}
                </ul>
              )}
            </article>
          </div>

          <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
            <h2 className="m-0 text-base font-semibold">Providers</h2>
            {providers.length === 0 ? (
              <p className="mt-2 text-sm text-slate-400">No providers detected.</p>
            ) : (
              <ul className="mt-3 flex list-none flex-col gap-2 p-0">
                {providers.map((provider) => (
                  <li
                    key={provider.id}
                    className="grid grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-3 rounded-lg border border-slate-800 bg-slate-950/70 px-3 py-2"
                  >
                    <div className="min-w-0">
                      <strong className="block truncate text-sm">{provider.display_name ?? provider.id}</strong>
                      <span className="text-xs text-slate-400">{provider.id}</span>
                    </div>
                    <span className="text-sm text-slate-400">{provider.latency_ms ? `${provider.latency_ms}ms` : "-"}</span>
                    <span className={`rounded-full border px-2 py-1 text-xs ${providerStateClass(provider)}`}>
                      {providerState(provider)}
                    </span>
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
