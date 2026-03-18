import { useQuery } from "@tanstack/react-query";
import type { DashboardSnapshot } from "../api";
import { loadDashboardSnapshot } from "../api";

const REFRESH_MS = 30000;

function formatUptime(seconds?: number): string {
  if (!seconds || seconds < 0) return "-";
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

export function OverviewPage() {
  const snapshotQuery = useQuery<DashboardSnapshot>({
    queryKey: ["dashboard", "snapshot"],
    queryFn: loadDashboardSnapshot,
    refetchInterval: REFRESH_MS
  });

  const snapshot = snapshotQuery.data ?? null;
  const loading = snapshotQuery.isLoading;
  const error = snapshotQuery.error instanceof Error ? snapshotQuery.error.message : "";
  const lastUpdated = snapshotQuery.dataUpdatedAt
    ? new Date(snapshotQuery.dataUpdatedAt).toLocaleTimeString()
    : "-";

  const providersReady =
    snapshot?.providers.filter((provider) => provider.auth_status === "configured").length ?? 0;
  const channelsReady =
    snapshot?.channels.filter((channel) => channel.configured && channel.has_token).length ?? 0;

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Dashboard</h1>
          <p className="text-sm text-slate-400">React dashboard is now the only UI entry point.</p>
        </div>
        <div className="flex items-center gap-2">
          <span
            className={`rounded-full border px-2 py-1 text-xs ${
              snapshot?.health.status === "ok"
                ? "border-emerald-700 bg-emerald-700/20 text-emerald-300"
                : "border-amber-700 bg-amber-700/20 text-amber-300"
            }`}
          >
            {snapshot?.health.status === "ok" ? "Healthy" : "Unreachable"}
          </span>
          <button
            className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
            onClick={() => void snapshotQuery.refetch()}
            disabled={snapshotQuery.isFetching}
          >
            Refresh
          </button>
        </div>
      </header>

      {loading ? (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">Loading dashboard snapshot...</div>
      ) : null}
      {error ? (
        <div className="rounded-xl border border-rose-700 bg-rose-700/15 p-4 text-rose-200">{error}</div>
      ) : null}

      {snapshot ? (
        <>
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <span className="text-sm text-slate-400">Agents</span>
              <strong className="mt-1 block text-2xl">{snapshot.status.agent_count ?? 0}</strong>
            </article>
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <span className="text-sm text-slate-400">Version</span>
              <strong className="mt-1 block text-2xl">{snapshot.status.version ?? "-"}</strong>
            </article>
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <span className="text-sm text-slate-400">Uptime</span>
              <strong className="mt-1 block text-2xl">{formatUptime(snapshot.status.uptime_seconds)}</strong>
            </article>
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <span className="text-sm text-slate-400">Skills</span>
              <strong className="mt-1 block text-2xl">{snapshot.skillCount}</strong>
            </article>
          </div>

          <div className="grid gap-3 xl:grid-cols-2">
            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <h2 className="m-0 text-base font-semibold">Providers</h2>
              <p className="mt-2 text-sm text-slate-400">
                {providersReady}/{snapshot.providers.length} configured
              </p>
              <ul className="mt-2 flex list-none flex-col gap-2 p-0">
                {snapshot.providers.slice(0, 8).map((provider) => (
                  <li key={provider.id} className="flex items-center justify-between gap-3">
                    <span>{provider.display_name ?? provider.id}</span>
                    <span className="text-sm text-slate-400">{provider.model_count ?? 0} models</span>
                  </li>
                ))}
              </ul>
            </article>

            <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <h2 className="m-0 text-base font-semibold">Channels</h2>
              <p className="mt-2 text-sm text-slate-400">
                {channelsReady}/{snapshot.channels.length} ready
              </p>
              <ul className="mt-2 flex list-none flex-col gap-2 p-0">
                {snapshot.channels.slice(0, 8).map((channel) => (
                  <li key={channel.name} className="flex items-center justify-between gap-3">
                    <span>{channel.display_name ?? channel.name}</span>
                    <span
                      className={`rounded-full border px-2 py-1 text-xs ${
                        channel.configured && channel.has_token
                          ? "border-emerald-700 bg-emerald-700/20 text-emerald-300"
                          : "border-slate-700 bg-slate-800/60 text-slate-300"
                      }`}
                    >
                      {channel.configured && channel.has_token ? "Ready" : "Not Ready"}
                    </span>
                  </li>
                ))}
              </ul>
            </article>
          </div>

          <p className="text-xs text-slate-400">Last refresh: {lastUpdated}</p>
        </>
      ) : null}
    </section>
  );
}
