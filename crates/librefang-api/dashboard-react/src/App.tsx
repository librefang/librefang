import { Link, Outlet } from "@tanstack/react-router";

export function App() {
  const navBase =
    "rounded-lg border border-transparent px-3 py-2 text-sm text-slate-300 transition hover:border-slate-600 hover:bg-slate-800/60 hover:text-white";
  const navActive = "border-sky-500 bg-sky-500/20 text-white";

  return (
    <div className="grid min-h-screen grid-cols-1 bg-slate-950 text-slate-100 lg:grid-cols-[260px_1fr]">
      <aside className="border-b border-slate-800 bg-slate-900/70 p-4 backdrop-blur lg:border-b-0 lg:border-r">
        <div className="mb-6 flex items-center gap-3">
          <div className="h-3 w-3 rounded-full bg-gradient-to-br from-cyan-400 to-emerald-400 shadow-[0_0_18px_rgba(34,211,238,0.6)]" />
          <div>
            <strong className="block text-sm">LibreFang</strong>
            <p className="text-xs text-slate-400">React Dashboard</p>
          </div>
        </div>
        <nav className="flex flex-col gap-2">
          <Link to="/overview" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Overview
          </Link>
          <Link to="/agents" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Agents
          </Link>
          <Link to="/sessions" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Sessions
          </Link>
          <Link to="/approvals" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Approvals
          </Link>
          <Link to="/comms" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Comms
          </Link>
          <Link to="/providers" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Providers
          </Link>
          <Link to="/channels" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Channels
          </Link>
          <Link to="/skills" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Skills
          </Link>
          <Link to="/hands" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Hands
          </Link>
          <Link to="/workflows" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Workflows
          </Link>
          <Link to="/scheduler" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Scheduler
          </Link>
          <Link to="/goals" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Goals
          </Link>
          <Link to="/analytics" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Analytics
          </Link>
          <Link to="/memory" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Memory
          </Link>
          <Link to="/runtime" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Runtime
          </Link>
          <Link to="/logs" className={navBase} activeProps={{ className: `${navBase} ${navActive}` }}>
            Logs
          </Link>
        </nav>
      </aside>
      <main className="p-4 lg:p-6">
        <Outlet />
      </main>
    </div>
  );
}
