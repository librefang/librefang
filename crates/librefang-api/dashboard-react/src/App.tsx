import { Link, Outlet } from "@tanstack/react-router";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Globe, Sun, Moon, Search, ChevronLeft, ChevronRight, ChevronDown, Menu, Home, Layers, MessageCircle, Clock, CheckCircle, Calendar, Shield, Users, Server, Network, Bell, Hand, BarChart3, Database, Activity, FileText, Settings, Puzzle, Cpu } from "lucide-react";
import { useUIStore } from "./lib/store";
import { CommandPalette, useCommandPalette } from "./components/ui/CommandPalette";

export function App() {
  const { t } = useTranslation();
  const { theme, toggleTheme, language, setLanguage, isMobileMenuOpen, setMobileMenuOpen, isSidebarCollapsed, toggleSidebar, navLayout, collapsedNavGroups, toggleNavGroup } = useUIStore();
  const { isOpen: isPaletteOpen, setIsOpen: setPaletteOpen } = useCommandPalette();

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
      key: "core",
      label: t("nav.core"),
      items: [
        { to: "/overview", label: t("nav.overview"), icon: Home },
        { to: "/workflows", label: t("nav.workflows"), icon: Layers },
        { to: "/chat", label: t("nav.chat"), icon: MessageCircle },
        { to: "/sessions", label: t("nav.sessions"), icon: Clock },
        { to: "/approvals", label: t("nav.approvals"), icon: CheckCircle },
      ],
    },
    {
      key: "automation",
      label: t("nav.automation"),
      items: [
        { to: "/scheduler", label: t("nav.scheduler"), icon: Calendar },
        { to: "/goals", label: t("nav.goals"), icon: Shield },
      ],
    },
    {
      key: "resources",
      label: t("nav.resources"),
      items: [
        { to: "/agents", label: t("nav.agents"), icon: Users },
        { to: "/providers", label: t("nav.providers"), icon: Server },
        { to: "/models", label: t("nav.models"), icon: Cpu },
        { to: "/channels", label: t("nav.channels"), icon: Network },
        { to: "/skills", label: t("nav.skills"), icon: Bell },
        { to: "/hands", label: t("nav.hands"), icon: Hand },
        { to: "/plugins", label: t("nav.plugins"), icon: Puzzle },
      ],
    },
    {
      key: "system",
      label: t("nav.system"),
      items: [
        { to: "/analytics", label: t("nav.analytics"), icon: BarChart3 },
        { to: "/memory", label: t("nav.memory"), icon: Database },
        { to: "/comms", label: t("nav.comms"), icon: Activity },
        { to: "/runtime", label: t("nav.runtime"), icon: Activity },
        { to: "/logs", label: t("nav.logs"), icon: FileText },
      ],
    },
  ];

  return (
    <div className="flex h-screen flex-col bg-main text-slate-900 dark:text-slate-100 lg:flex-row transition-colors duration-300 overflow-hidden">
      {/* Sidebar Overlay (Mobile) */}
      {isMobileMenuOpen && (
        <div 
          className="fixed inset-0 z-40 bg-black/60 backdrop-blur-sm lg:hidden"
          onClick={() => setMobileMenuOpen(false)}
        />
      )}

      {/* Sidebar */}
      <aside className={`
        fixed inset-y-0 left-0 z-50 flex flex-col border-r border-border-subtle bg-surface/80 backdrop-blur-xl transition-all duration-300 ease-in-out lg:static lg:translate-x-0
        ${isMobileMenuOpen ? "translate-x-0 shadow-2xl" : "-translate-x-full"}
        ${isSidebarCollapsed ? "lg:w-[72px]" : "lg:w-[280px]"}
      `}>
        {/* Sidebar Header */}
        <div className="flex h-16 items-center justify-between border-b border-border-subtle px-4">
          <div className="flex items-center gap-3">
            <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-brand/20 shadow-[0_0_15px_rgba(14,165,233,0.3)] ring-1 ring-brand/40 shrink-0">
              <div className="h-3 w-3 rounded-full bg-brand animate-pulse" />
            </div>
            {!isSidebarCollapsed && (
              <div className="flex flex-col">
                <strong className="text-sm font-bold tracking-tight">LibreFang</strong>
                <span className="text-[10px] font-semibold uppercase tracking-wider text-text-dim">{t("common.infrastructure")}</span>
              </div>
            )}
          </div>
          <div className="flex items-center gap-1">
            <button
              onClick={toggleSidebar}
              className="hidden lg:flex h-8 w-8 items-center justify-center rounded-lg border border-border-subtle bg-surface text-text-dim hover:text-brand transition-all shadow-sm"
              title={isSidebarCollapsed ? "Expand" : "Collapse"}
            >
              {isSidebarCollapsed ? <ChevronRight className="h-4 w-4" /> : <ChevronLeft className="h-4 w-4" />}
            </button>
          </div>
        </div>

        {/* Navigation */}
        <nav className="overflow-y-auto overflow-x-hidden p-4 scrollbar-thin scrollbar-thumb-border-subtle scrollbar-track-transparent" style={{ maxHeight: "calc(100vh - 160px)" }}>
          {/* Search Button */}
          <button
            onClick={() => setPaletteOpen(true)}
            className={`mb-4 flex w-full items-center gap-2 rounded-xl border border-border-subtle bg-surface-hover px-3 py-2.5 text-text-dim hover:border-brand/30 hover:text-brand transition-all ${isSidebarCollapsed ? "lg:hidden" : ""}`}
            title="Search (Cmd+K)"
          >
            <Search className="h-4 w-4" />
            <span className="flex-1 text-left text-xs font-medium">Search</span>
            <kbd className="text-[10px] font-mono bg-main px-1.5 py-0.5 rounded">⌘K</kbd>
          </button>

          <div className="flex flex-col gap-6">
            {navGroups.map((group) => (
              <div key={group.key} className="flex flex-col gap-1">
                {navLayout === "collapsible" ? (
                  // 二级菜单布局 - 可折叠
                  <>
                    <button
                      onClick={() => toggleNavGroup(group.key)}
                      className={`flex items-center justify-between px-3 text-[11px] font-bold uppercase tracking-[0.1em] text-text-dim/80 hover:text-brand transition-colors ${isSidebarCollapsed ? "lg:hidden" : ""}`}
                    >
                      {group.label}
                      <ChevronDown className={`h-3 w-3 transition-transform ${collapsedNavGroups[group.key] ? "-rotate-90" : ""}`} />
                    </button>
                    <div className={`mt-1 flex flex-col gap-0.5 ${collapsedNavGroups[group.key] ? "lg:hidden" : ""}`}>
                      {group.items.map((item) => (
                        <Link
                          key={item.to}
                          to={item.to as any}
                          className={navBase}
                          activeProps={{ className: `${navBase} ${navActive}` }}
                          onClick={() => setMobileMenuOpen(false)}
                        >
                          {item.icon && <item.icon className="h-4 w-4 transition-transform group-hover:scale-110 group-hover:text-brand shrink-0" />}
                          <span className={`flex-1 ${isSidebarCollapsed ? "lg:hidden" : ""}`}>{item.label}</span>
                        </Link>
                      ))}
                    </div>
                  </>
                ) : (
                  // 分组布局 - 全部显示
                  <>
                    <h3 className={`px-3 text-[11px] font-bold uppercase tracking-[0.1em] text-text-dim/80 ${isSidebarCollapsed ? "lg:hidden" : ""}`}>
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
                          {item.icon && <item.icon className="h-4 w-4 transition-transform group-hover:scale-110 group-hover:text-brand shrink-0" />}
                          <span className={`flex-1 ${isSidebarCollapsed ? "lg:hidden" : ""}`}>{item.label}</span>
                        </Link>
                      ))}
                    </div>
                  </>
                )}
              </div>
            ))}
          </div>
        </nav>

        {/* Sidebar Footer */}
        <div className={`border-t border-border-subtle p-4 ${isSidebarCollapsed ? "lg:hidden" : ""}`}>
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
        {/* Top Header */}
        <header className="flex h-16 shrink-0 items-center justify-between border-b border-border-subtle bg-surface/80 px-6 backdrop-blur-xl">
          <div className="flex items-center gap-3">
          </div>
          <div className="flex items-center gap-2">
            <Link
              to="/settings"
              className="flex h-8 w-8 items-center justify-center rounded-lg border border-border-subtle bg-surface text-text-dim hover:text-brand transition-all shadow-sm"
            >
              <Settings className="h-4 w-4" />
            </Link>
            <button
              onClick={() => setLanguage(language === "en" ? "zh" : "en")}
              className="flex h-8 w-8 items-center justify-center rounded-lg border border-border-subtle bg-surface text-text-dim hover:text-brand transition-all shadow-sm"
            >
              <Globe className="h-4 w-4" />
            </button>
            <button
              onClick={toggleTheme}
              className="rounded-md border border-border-subtle bg-surface p-2 text-text-dim hover:text-brand shadow-sm"
            >
              {theme === "dark" ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
            </button>
            <button
              onClick={() => setMobileMenuOpen(true)}
              className="rounded-md border border-border-subtle bg-surface p-2 text-text-dim hover:text-brand shadow-sm lg:hidden"
            >
              <Menu className="h-5 w-5" />
            </button>
          </div>
        </header>

        {/* Main Content */}
        <main className="flex-1 overflow-auto bg-main">
          <div className="mx-auto max-w-7xl p-4 lg:p-8 animate-fade-in-up">
            <Outlet />
          </div>
        </main>
      </div>

      <CommandPalette isOpen={isPaletteOpen} onClose={() => setPaletteOpen(false)} />
    </div>
  );
}
