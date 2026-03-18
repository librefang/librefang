import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
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
  const navigate = useNavigate();
  const snapshotQuery = useQuery<DashboardSnapshot>({
    queryKey: ["dashboard", "snapshot"],
    queryFn: loadDashboardSnapshot,
    refetchInterval: REFRESH_MS
  });

  const snapshot = snapshotQuery.data ?? null;
  const loading = snapshotQuery.isLoading;
  const error = snapshotQuery.error instanceof Error ? snapshotQuery.error.message : "";

  const providersReady = snapshot?.providers.filter((p) => p.auth_status === "configured").length ?? 0;
  const channelsReady = snapshot?.channels.filter((c) => c.configured && c.has_token).length ?? 0;
  const agentsActive = snapshot?.status.active_agent_count ?? 0;

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Dashboard</h1>
          <p className="text-sm text-slate-400">LibreFang Agent Operating System</p>
        </div>
        <div className="flex items-center gap-2">
          <span
            className={`rounded-full border px-3 py-1 text-xs font-medium ${
              snapshot?.health.status === "ok"
                ? "border-emerald-700 bg-emerald-700/20 text-emerald-300"
                : "border-amber-700 bg-amber-700/20 text-amber-300"
            }`}
          >
            {snapshot?.health.status === "ok" ? "● Running" : "○ Issues"}
          </span>
          <button
            className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-sky-500"
            onClick={() => void snapshotQuery.refetch()}
            disabled={snapshotQuery.isFetching}
          >
            Refresh
          </button>
        </div>
      </header>

      {loading ? (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">Loading...</div>
      ) : null}
      {error ? (
        <div className="rounded-xl border border-rose-700 bg-rose-700/15 p-4 text-rose-200">{error}</div>
      ) : null}

      {snapshot ? (
        <>
          {/* Quick Stats */}
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            <button
              onClick={() => navigate({ to: "/agents" })}
              className="group rounded-xl border border-slate-800 bg-slate-900/70 p-4 text-left transition hover:border-sky-500"
            >
              <span className="text-sm text-slate-400">Agents</span>
              <strong className="mt-1 block text-3xl">{snapshot.status.agent_count ?? 0}</strong>
              <span className="mt-2 block text-xs text-emerald-400">
                {agentsActive > 0 ? `${agentsActive} active` : "Idle"}
              </span>
            </button>

            <button
              onClick={() => navigate({ to: "/canvas" })}
              className="group rounded-xl border border-slate-800 bg-slate-900/70 p-4 text-left transition hover:border-sky-500"
            >
              <span className="text-sm text-slate-400">Canvas</span>
              <strong className="mt-1 block text-3xl">n8n</strong>
              <span className="mt-2 block text-xs text-slate-500 group-hover:text-sky-400">
                Click to open workflow editor →
              </span>
            </button>

            <button
              onClick={() => navigate({ to: "/providers" })}
              className="group rounded-xl border border-slate-800 bg-slate-900/70 p-4 text-left transition hover:border-sky-500"
            >
              <span className="text-sm text-slate-400">Providers</span>
              <strong className="mt-1 block text-3xl">{providersReady}</strong>
              <span className="mt-2 block text-xs text-slate-500">
                {snapshot.providers.length} configured
              </span>
            </button>

            <button
              onClick={() => navigate({ to: "/channels" })}
              className="group rounded-xl border border-slate-800 bg-slate-900/70 p-4 text-left transition hover:border-sky-500"
            >
              <span className="text-sm text-slate-400">Channels</span>
              <strong className="mt-1 block text-3xl">{channelsReady}</strong>
              <span className="mt-2 block text-xs text-slate-500">
                {snapshot.channels.length} total
              </span>
            </button>
          </div>

          {/* Quick Actions */}
          <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
            <h2 className="mb-3 mt-0 text-base font-semibold">Quick Actions</h2>
            <div className="flex flex-wrap gap-2">
              <button
                onClick={() => navigate({ to: "/canvas" })}
                className="rounded-lg border border-sky-500/50 bg-sky-600/20 px-4 py-2 text-sm text-sky-400 transition hover:bg-sky-600/30"
              >
                + New Workflow
              </button>
              <button
                onClick={() => navigate({ to: "/agents" })}
                className="rounded-lg border border-slate-700 bg-slate-800 px-4 py-2 text-sm text-slate-300 transition hover:bg-slate-700"
              >
                Manage Agents
              </button>
              <button
                onClick={() => navigate({ to: "/chat" })}
                className="rounded-lg border border-slate-700 bg-slate-800 px-4 py-2 text-sm text-slate-300 transition hover:bg-slate-700"
              >
                Open Chat
              </button>
              <button
                onClick={() => navigate({ to: "/scheduler" })}
                className="rounded-lg border border-slate-700 bg-slate-800 px-4 py-2 text-sm text-slate-300 transition hover:bg-slate-700"
              >
                Schedule Tasks
              </button>
            </div>
          </div>

          {/* System Info */}
          <div className="grid gap-4 lg:grid-cols-2">
            <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <h2 className="mb-3 mt-0 text-sm font-semibold uppercase text-slate-400">System</h2>
              <dl className="grid grid-cols-2 gap-2 text-sm">
                <dt className="text-slate-500">Version</dt>
                <dd className="text-slate-200">{snapshot.status.version ?? "-"}</dd>
                <dt className="text-slate-500">Uptime</dt>
                <dd className="text-slate-200">{formatUptime(snapshot.status.uptime_seconds)}</dd>
                <dt className="text-slate-500">Memory</dt>
                <dd className="text-slate-200">{snapshot.status.memory_used_mb ? `${snapshot.status.memory_used_mb} MB` : "-"}</dd>
              </dl>
            </div>

            <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <h2 className="mb-3 mt-0 text-sm font-semibold uppercase text-slate-400">Health</h2>
              <div className="space-y-2">
                {snapshot.health.checks?.map((check, i) => (
                  <div key={i} className="flex items-center justify-between text-sm">
                    <span className="text-slate-400">{check.name}</span>
                    <span className={check.status === "ok" ? "text-emerald-400" : "text-amber-400"}>
                      {check.status === "ok" ? "✓" : "!"}
                    </span>
                  </div>
                )) ?? <p className="text-sm text-slate-500">No health checks</p>}
              </div>
            </div>
          </div>

          {/* Recent Agents */}
          <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
            <div className="mb-3 flex items-center justify-between">
              <h2 className="m-0 text-base font-semibold">Recent Agents</h2>
              <button
                onClick={() => navigate({ to: "/agents" })}
                className="text-sm text-sky-400 transition hover:text-sky-300"
              >
                View all →
              </button>
            </div>
            <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
              {snapshot.agents.slice(0, 6).map((agent) => (
                <button
                  key={agent.id}
                  onClick={() => navigate({ to: "/agents" })}
                  className="flex items-center gap-3 rounded-lg border border-slate-700 bg-slate-800/50 p-3 text-left transition hover:border-slate-600"
                >
                  <div
                    className={`h-2 w-2 rounded-full ${
                      agent.status === "running" ? "bg-emerald-500" : "bg-slate-500"
                    }`}
                  />
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium text-slate-200">{agent.name}</p>
                    <p className="truncate text-xs text-slate-500">{agent.id.slice(0, 8)}...</p>
                  </div>
                </button>
              ))}
              {snapshot.agents.length === 0 && (
                <p className="col-span-full py-4 text-center text-sm text-slate-500">
                  No agents yet. Create one from the Agents page.
                </p>
              )}
            </div>
          </div>
        </>
      ) : null}
    </section>
  );
}
