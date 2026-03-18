import { useEffect, useState } from "react";
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
  const [snapshot, setSnapshot] = useState<DashboardSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>("");
  const [refreshAt, setRefreshAt] = useState<Date | null>(null);

  async function refresh() {
    try {
      const next = await loadDashboardSnapshot();
      setSnapshot(next);
      setError("");
      setRefreshAt(new Date());
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load dashboard.");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      if (cancelled) return;
      await refresh();
    };
    void run();
    const timer = window.setInterval(() => {
      void run();
    }, REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  const providersReady =
    snapshot?.providers.filter((provider) => provider.auth_status === "configured").length ?? 0;
  const channelsReady =
    snapshot?.channels.filter((channel) => channel.configured && channel.has_token).length ?? 0;

  return (
    <section className="page">
      <header className="page-header">
        <div>
          <h1>Dashboard (React)</h1>
          <p className="muted">React dashboard is now the only UI entry point.</p>
        </div>
        <div className="header-actions">
          <span className={`badge ${snapshot?.health.status === "ok" ? "ok" : "warn"}`}>
            {snapshot?.health.status === "ok" ? "Healthy" : "Unreachable"}
          </span>
          <button className="btn" onClick={() => void refresh()}>
            Refresh
          </button>
        </div>
      </header>

      {loading ? <div className="card">Loading dashboard snapshot...</div> : null}
      {error ? <div className="card error">{error}</div> : null}

      {snapshot ? (
        <>
          <div className="stats-grid">
            <article className="card stat">
              <span className="muted">Agents</span>
              <strong>{snapshot.status.agent_count ?? 0}</strong>
            </article>
            <article className="card stat">
              <span className="muted">Version</span>
              <strong>{snapshot.status.version ?? "-"}</strong>
            </article>
            <article className="card stat">
              <span className="muted">Uptime</span>
              <strong>{formatUptime(snapshot.status.uptime_seconds)}</strong>
            </article>
            <article className="card stat">
              <span className="muted">Skills</span>
              <strong>{snapshot.skillCount}</strong>
            </article>
          </div>

          <div className="panel-grid">
            <article className="card">
              <h2>Providers</h2>
              <p className="muted">{providersReady}/{snapshot.providers.length} configured</p>
              <ul className="list">
                {snapshot.providers.slice(0, 8).map((provider) => (
                  <li key={provider.id}>
                    <span>{provider.display_name ?? provider.id}</span>
                    <span className="muted">{provider.model_count ?? 0} models</span>
                  </li>
                ))}
              </ul>
            </article>

            <article className="card">
              <h2>Channels</h2>
              <p className="muted">{channelsReady}/{snapshot.channels.length} ready</p>
              <ul className="list">
                {snapshot.channels.slice(0, 8).map((channel) => (
                  <li key={channel.name}>
                    <span>{channel.display_name ?? channel.name}</span>
                    <span className={`chip ${channel.configured && channel.has_token ? "ok" : "muted"}`}>
                      {channel.configured && channel.has_token ? "Ready" : "Not Ready"}
                    </span>
                  </li>
                ))}
              </ul>
            </article>
          </div>

          <p className="muted footer-note">
            Last refresh: {refreshAt ? refreshAt.toLocaleTimeString() : "-"}
          </p>
        </>
      ) : null}
    </section>
  );
}
