import { Link, Outlet } from "@tanstack/react-router";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useUIStore } from "./lib/store";

export function App() {
  const { t } = useTranslation();
  const { theme, toggleTheme, language, setLanguage, isMobileMenuOpen, setMobileMenuOpen } = useUIStore();

  useEffect(() => {
    const root = window.document.documentElement;
    if (theme === "dark") {
      root.classList.add("dark");
    } else {
      root.classList.remove("dark");
    }
  }, [theme]);

  const navBase =
    "flex items-center gap-3 rounded-lg border border-transparent px-3 py-2 text-sm text-text-dim transition-all duration-200 hover:border-border-subtle hover:bg-surface-hover hover:text-brand group";
  const navActive = "border-brand/20 bg-brand-muted text-brand font-medium shadow-sm";

  const navGroups = [
    {
      label: t("nav.core"),
      items: [
        { to: "/overview", label: t("nav.overview"), icon: <path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" /> },
        { to: "/workflows", label: t("nav.workflows"), icon: <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" /> },
        { to: "/chat", label: t("nav.chat"), icon: <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" /> },
        { to: "/sessions", label: t("nav.sessions"), icon: <><circle cx="12" cy="12" r="10" /><polyline points="12 6 12 12 16 14" /></> },
        { to: "/approvals", label: t("nav.approvals"), icon: <><path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" /><polyline points="22 4 12 14.01 9 11.01" /></> },
      ],
    },
    {
      label: t("nav.automation"),
      items: [
        { to: "/scheduler", label: t("nav.scheduler"), icon: <><rect x="3" y="4" width="18" height="18" rx="2" ry="2" /><line x1="16" y1="2" x2="16" y2="6" /><line x1="8" y1="2" x2="8" y2="6" /><line x1="3" y1="10" x2="21" y2="10" /></> },
        { to: "/goals", label: t("nav.goals"), icon: <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" /> },
      ],
    },
    {
      label: t("nav.resources"),
      items: [
        { to: "/agents", label: t("nav.agents"), icon: <><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" /><circle cx="9" cy="7" r="4" /><path d="M23 21v-2a4 4 0 0 0-3-3.87" /><path d="M16 3.13a4 4 0 0 1 0 7.75" /></> },
        { to: "/providers", label: t("nav.providers"), icon: <><rect x="2" y="2" width="20" height="8" rx="2" ry="2" /><rect x="2" y="14" width="20" height="8" rx="2" ry="2" /><line x1="6" y1="6" x2="6.01" y2="6" /><line x1="6" y1="18" x2="6.01" y2="18" /></> },
        { to: "/channels", label: t("nav.channels"), icon: <><circle cx="18" cy="5" r="3" /><circle cx="6" cy="12" r="3" /><circle cx="18" cy="19" r="3" /><line x1="8.59" y1="13.51" x2="15.42" y2="17.49" /><line x1="15.41" y1="6.51" x2="8.59" y2="10.49" /></> },
        { to: "/skills", label: t("nav.skills"), icon: <><path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" /><path d="M13.73 21a2 2 0 0 1-3.46 0" /></> },
        { to: "/hands", label: t("nav.hands"), icon: <><path d="M18 11V6a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v0" /><path d="M14 10V4a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v0" /><path d="M10 10.5V6a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v0" /><path d="M18 8a2 2 0 1 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.86-5.99-2.34l-3.6-3.6a2 2 0 0 1 2.83-2.82L7 15" /></> },
      ],
    },
    {
      label: t("nav.system"),
      items: [
        { to: "/analytics", label: t("nav.analytics"), icon: <><path d="M18 20V10" /><path d="M12 20V4" /><path d="M6 20V14" /></> },
        { to: "/memory", label: t("nav.memory"), icon: <><path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" /><polyline points="3.27 6.96 12 12.01 20.73 6.96" /><line x1="12" y1="22.08" x2="12" y2="12" /></> },
        { to: "/comms", label: t("nav.comms"), icon: <polyline points="22 12 18 12 15 21 9 3 6 12 2 12" /> },
        { to: "/runtime", label: t("nav.runtime"), icon: <path d="M22 12h-4l-3 9L9 3l-3 9H2" /> },
        { to: "/logs", label: t("nav.logs"), icon: <><line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" /><line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" /></> },
        { to: "/settings", label: t("nav.settings"), icon: <><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" /></> },
      ],
    },
  ];

  return (
    <div className="flex min-h-screen flex-col bg-main text-slate-900 dark:text-slate-100 lg:flex-row transition-colors duration-300">
      {/* Sidebar Overlay (Mobile) */}
      {isMobileMenuOpen && (
        <div 
          className="fixed inset-0 z-40 bg-black/60 backdrop-blur-sm lg:hidden"
          onClick={() => setMobileMenuOpen(false)}
        />
      )}

      {/* Sidebar */}
      <aside className={`
        fixed inset-y-0 left-0 z-50 flex w-[280px] flex-col border-r border-border-subtle bg-surface/80 backdrop-blur-xl transition-all duration-300 ease-in-out lg:static lg:translate-x-0
        ${isMobileMenuOpen ? "translate-x-0 shadow-2xl" : "-translate-x-full"}
      `}>
        {/* Sidebar Header */}
        <div className="flex h-16 items-center justify-between border-b border-border-subtle px-6">
          <div className="flex items-center gap-3">
            <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-brand/20 shadow-[0_0_15px_rgba(14,165,233,0.3)] ring-1 ring-brand/40">
              <div className="h-3 w-3 rounded-full bg-brand animate-pulse" />
            </div>
            <div className="flex flex-col">
              <strong className="text-sm font-bold tracking-tight">LibreFang</strong>
              <span className="text-[10px] font-semibold uppercase tracking-wider text-text-dim">{t("common.infrastructure")}</span>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <button 
              onClick={() => setLanguage(language === "en" ? "zh" : "en")}
              className="flex h-8 w-8 items-center justify-center rounded-lg border border-border-subtle bg-surface text-[10px] font-black text-text-dim hover:text-brand transition-all shadow-sm"
              title={t("common.change_language")}
            >
              {language === "en" ? "ZH" : "EN"}
            </button>
            <button 
              onClick={toggleTheme}
              className="flex h-8 w-8 items-center justify-center rounded-lg border border-border-subtle bg-surface text-text-dim hover:text-brand transition-all shadow-sm"
              title={t("common.toggle_theme")}
            >
              {theme === "dark" ? (
                <svg viewBox="0 0 24 24" className="h-4 w-4" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 3v1m0 16v1m9-9h-1M4 12H3m15.364-6.364l-.707.707M6.343 17.657l-.707.707m12.728 0l-.707-.707M6.343 6.343l-.707-.707m12.728 12.728L12 12m0 0a4 4 0 100-8 4 4 0 000 8z" /></svg>
              ) : (
                <svg viewBox="0 0 24 24" className="h-4 w-4" fill="none" stroke="currentColor" strokeWidth="2"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" /></svg>
              )}
            </button>
          </div>
        </div>

        {/* Navigation */}
        <nav className="flex-1 overflow-y-auto overflow-x-hidden p-4 scrollbar-thin scrollbar-thumb-border-subtle scrollbar-track-transparent">
          <div className="flex flex-col gap-6">
            {navGroups.map((group) => (
              <div key={group.label} className="flex flex-col gap-1">
                <h3 className="px-3 text-[11px] font-bold uppercase tracking-[0.1em] text-text-dim/80">
                  {group.label}
                </h3>
                <div className="mt-1 flex flex-col gap-0.5">
                  {group.items.map((item) => (
                    <Link
                      key={item.to}
                      to={item.to as any}
                      className={navBase}
                      activeProps={{ className: `${navBase} ${navActive}` }}
                      onClick={() => setMobileMenuOpen(false)}
                    >
                      <svg 
                        className="h-4 w-4 stroke-[1.5] transition-transform group-hover:scale-110 group-hover:text-brand" 
                        viewBox="0 0 24 24" 
                        fill="none" 
                        stroke="currentColor" 
                        strokeLinecap="round" 
                        strokeLinejoin="round"
                      >
                        {item.icon}
                      </svg>
                      <span className="flex-1">{item.label}</span>
                    </Link>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </nav>

        {/* Sidebar Footer */}
        <div className="border-t border-border-subtle p-4">
          <div className="rounded-xl bg-surface-hover p-3 ring-1 ring-border-subtle">
            <p className="text-[10px] font-medium text-text-dim uppercase tracking-wider">{t("common.status")}</p>
            <div className="mt-2 flex items-center gap-2">
              <div className="h-2 w-2 rounded-full bg-success shadow-[0_0_8px_var(--success-color)]" />
              <span className="text-xs font-semibold">{t("common.daemon_online")}</span>
            </div>
          </div>
        </div>
      </aside>

      {/* Main Content Area */}
      <div className="flex flex-1 flex-col overflow-hidden">
        {/* Top Header (Mobile Only) */}
        <header className="flex h-16 items-center justify-between border-b border-border-subtle bg-surface/80 px-6 backdrop-blur-xl lg:hidden">
          <div className="flex items-center gap-3">
            <div className="h-6 w-6 rounded-md bg-brand/20 ring-1 ring-brand/40" />
            <strong className="text-sm font-bold">LibreFang</strong>
          </div>
          <div className="flex items-center gap-2">
            <button 
              onClick={() => setLanguage(language === "en" ? "zh" : "en")}
              className="flex h-8 w-8 items-center justify-center rounded-lg border border-border-subtle bg-surface text-[10px] font-black text-text-dim hover:text-brand transition-all shadow-sm"
            >
              {language === "en" ? "ZH" : "EN"}
            </button>
            <button 
              onClick={toggleTheme}
              className="rounded-md border border-border-subtle bg-surface p-2 text-text-dim hover:text-brand shadow-sm"
            >
              {theme === "dark" ? (
                <svg viewBox="0 0 24 24" className="h-4 w-4" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 3v1m0 16v1m9-9h-1M4 12H3m15.364-6.364l-.707.707M6.343 17.657l-.707.707m12.728 0l-.707-.707M6.343 6.343l-.707-.707m12.728 12.728L12 12m0 0a4 4 0 100-8 4 4 0 000 8z" /></svg>
              ) : (
                <svg viewBox="0 0 24 24" className="h-4 w-4" fill="none" stroke="currentColor" strokeWidth="2"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" /></svg>
              )}
            </button>
            <button 
              onClick={() => setMobileMenuOpen(true)}
              className="rounded-md border border-border-subtle bg-surface p-2 text-text-dim hover:text-brand shadow-sm"
            >
              <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
              </svg>
            </button>
          </div>
        </header>

        {/* Main Content */}
        <main className="flex-1 overflow-auto bg-main">
          <div className="mx-auto max-w-7xl p-4 lg:p-8">
            <Outlet />
          </div>
        </main>
      </div>
    </div>
  );
}
