import { Link, Outlet, useLocation } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Globe, Sun, Moon, Search, ChevronLeft, ChevronRight, ChevronDown, Menu, Home, Layers, MessageCircle, Clock, CheckCircle, Calendar, Shield, Users, Server, Network, Bell, Hand, BarChart3, Database, Activity, FileText, Settings, Puzzle, Cpu, Lock, Share2, LogOut } from "lucide-react";
import { useUIStore } from "./lib/store";
import { CommandPalette, useCommandPalette } from "./components/ui/CommandPalette";
import { checkAuthRequired, setApiKey, hasApiKey, clearApiKey, checkDashboardAuthMode, dashboardLogin, setOnUnauthorized, type AuthMode } from "./api";
import { SkillOutputPanel } from "./components/ui/SkillOutputPanel";

function LoginScreen({ mode, onAuthenticated }: { mode: AuthMode; onAuthenticated: () => void }) {
  const { t } = useTranslation();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [key, setKey] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  async function handleCredentialsLogin(e: React.FormEvent) {
    e.preventDefault();
    if (!username.trim() || !password.trim()) return;
    setLoading(true);
    setError("");
    const result = await dashboardLogin(username.trim(), password.trim());
    setLoading(false);
    if (result.ok) {
      onAuthenticated();
    } else {
      setError(result.error || t("auth.invalid"));
    }
  }

  async function handleApiKeyLogin(e: React.FormEvent) {
    e.preventDefault();
    if (!key.trim()) return;
    setLoading(true);
    setError("");
    setApiKey(key.trim());
    const stillNeeded = await checkAuthRequired();
    setLoading(false);
    if (stillNeeded) {
      setError(t("auth.invalid"));
    } else {
      onAuthenticated();
    }
  }

  return (
    <div className="fixed inset-0 z-[200] flex bg-main overflow-auto">
      {/* Left panel — branding (desktop only) */}
      <div className="hidden lg:flex lg:w-[45%] relative overflow-hidden bg-gradient-to-br from-slate-900 via-slate-800 to-slate-900 items-center justify-center">
        <div className="absolute inset-0">
          <div className="absolute top-1/4 left-1/4 w-72 h-72 bg-brand/20 rounded-full blur-[100px]" />
          <div className="absolute bottom-1/4 right-1/4 w-96 h-96 bg-accent/15 rounded-full blur-[120px]" />
          <div className="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 w-48 h-48 bg-brand/10 rounded-full blur-[80px] animate-pulse" />
        </div>
        <div className="relative z-10 text-center px-12">
          <div className="w-24 h-24 rounded-3xl bg-white/10 backdrop-blur-sm flex items-center justify-center mx-auto mb-8 ring-1 ring-white/20 shadow-2xl">
            <div className="w-10 h-10 rounded-full bg-gradient-to-br from-brand to-cyan-400 shadow-lg shadow-brand/40" />
          </div>
          <h1 className="text-4xl font-black text-white tracking-tight">LibreFang</h1>
          <p className="text-lg text-white/50 mt-3 font-medium">Agent Operating System</p>
          <div className="flex items-center justify-center gap-3 mt-8">
            <div className="flex items-center gap-2 px-4 py-2 rounded-full bg-white/5 border border-white/10">
              <span className="w-2 h-2 rounded-full bg-success animate-pulse" />
              <span className="text-xs text-white/60 font-medium">{t("common.daemon_online", { defaultValue: "Daemon Online" })}</span>
            </div>
          </div>
        </div>
      </div>

      {/* Right panel — login form */}
      <div className="flex-1 flex flex-col items-center justify-center p-6 sm:p-8">
        <div className="w-full max-w-[400px] animate-fade-in-scale flex-1 flex flex-col justify-center">
          {/* Mobile logo */}
          <div className="flex flex-col items-center mb-8 lg:hidden">
            <div className="w-16 h-16 rounded-2xl bg-gradient-to-br from-brand/20 to-accent/10 flex items-center justify-center mb-4 ring-1 ring-brand/20 shadow-xl">
              <div className="w-7 h-7 rounded-full bg-gradient-to-br from-brand to-cyan-400" />
            </div>
            <h1 className="text-2xl font-black tracking-tight">LibreFang</h1>
          </div>

          {/* Welcome text */}
          <div className="mb-8">
            <h2 className="text-2xl sm:text-3xl font-black tracking-tight">
              {mode === "credentials" ? t("auth.welcome_back", { defaultValue: "Welcome back" }) : t("auth.title")}
            </h2>
            <p className="text-sm text-text-dim mt-2">
              {mode === "credentials" ? t("auth.login_desc", { defaultValue: "Sign in to your dashboard" }) : t("auth.description")}
            </p>
          </div>

          {/* Form */}
          {mode === "credentials" ? (
            <form onSubmit={handleCredentialsLogin} className="space-y-5">
              <div>
                <label className="text-xs font-bold text-text-dim mb-2 block">
                  {t("auth.username", { defaultValue: "Username" })}
                </label>
                <input
                  type="text"
                  value={username}
                  onChange={(e) => { setUsername(e.target.value); setError(""); }}
                  placeholder={t("auth.username_placeholder", { defaultValue: "Enter username" })}
                  autoFocus
                  autoComplete="username"
                  className={`w-full rounded-xl border px-4 py-3.5 text-sm focus:ring-2 outline-none transition-all ${
                    error ? "border-error/50 focus:border-error focus:ring-error/10" : "border-border-subtle bg-main focus:border-brand focus:ring-brand/10"
                  }`}
                />
              </div>
              <div>
                <label className="text-xs font-bold text-text-dim mb-2 block">
                  {t("auth.password", { defaultValue: "Password" })}
                </label>
                <input
                  type="password"
                  value={password}
                  onChange={(e) => { setPassword(e.target.value); setError(""); }}
                  placeholder={t("auth.password_placeholder", { defaultValue: "Enter password" })}
                  autoComplete="current-password"
                  className={`w-full rounded-xl border px-4 py-3.5 text-sm focus:ring-2 outline-none transition-all ${
                    error ? "border-error/50 focus:border-error focus:ring-error/10" : "border-border-subtle bg-main focus:border-brand focus:ring-brand/10"
                  }`}
                />
              </div>

              {error && (
                <div className="flex items-center gap-2 px-4 py-3 rounded-xl bg-error/10 border border-error/20">
                  <Lock className="h-4 w-4 text-error shrink-0" />
                  <p className="text-sm text-error font-medium">{error}</p>
                </div>
              )}

              <button
                type="submit"
                disabled={loading || !username.trim() || !password.trim()}
                className="w-full rounded-xl bg-gradient-to-r from-brand to-brand/80 py-4 text-sm font-bold text-white hover:shadow-xl hover:shadow-brand/25 hover:-translate-y-0.5 active:translate-y-0 transition-all duration-200 disabled:opacity-40 disabled:hover:translate-y-0 disabled:hover:shadow-none"
              >
                {loading ? (
                  <span className="flex items-center justify-center gap-2">
                    <span className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                  </span>
                ) : t("auth.login", { defaultValue: "Sign In" })}
              </button>
            </form>
          ) : (
            <form onSubmit={handleApiKeyLogin} className="space-y-5">
              <div>
                <label className="text-xs font-bold text-text-dim mb-2 block">
                  API Key
                </label>
                <input
                  type="password"
                  value={key}
                  onChange={(e) => { setKey(e.target.value); setError(""); }}
                  placeholder={t("auth.placeholder")}
                  autoFocus
                  className={`w-full rounded-xl border px-4 py-3.5 text-sm focus:ring-2 outline-none transition-all ${
                    error ? "border-error/50 focus:border-error focus:ring-error/10" : "border-border-subtle bg-main focus:border-brand focus:ring-brand/10"
                  }`}
                />
              </div>

              {error && (
                <div className="flex items-center gap-2 px-4 py-3 rounded-xl bg-error/10 border border-error/20">
                  <Lock className="h-4 w-4 text-error shrink-0" />
                  <p className="text-sm text-error font-medium">{error}</p>
                </div>
              )}

              <button
                type="submit"
                disabled={loading || !key.trim()}
                className="w-full rounded-xl bg-gradient-to-r from-brand to-brand/80 py-4 text-sm font-bold text-white hover:shadow-xl hover:shadow-brand/25 hover:-translate-y-0.5 active:translate-y-0 transition-all duration-200 disabled:opacity-40 disabled:hover:translate-y-0 disabled:hover:shadow-none"
              >
                {loading ? (
                  <span className="flex items-center justify-center gap-2">
                    <span className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                  </span>
                ) : t("auth.submit")}
              </button>
            </form>
          )}

        </div>
        {/* Footer — pinned to bottom */}
        <div className="flex items-center justify-center gap-2 py-4 text-text-dim/30 shrink-0">
          <Lock className="h-3 w-3" />
          <span className="text-[10px] font-medium tracking-wider uppercase">{t("auth.secure_connection", { defaultValue: "Encrypted Connection" })}</span>
        </div>
      </div>
    </div>
  );
}

export function App() {
  const { t } = useTranslation();
  const { theme, toggleTheme, language, setLanguage, isMobileMenuOpen, setMobileMenuOpen, isSidebarCollapsed, toggleSidebar, navLayout, collapsedNavGroups, toggleNavGroup } = useUIStore();
  const { isOpen: isPaletteOpen, setIsOpen: setPaletteOpen } = useCommandPalette();
  const location = useLocation();

  const [authNeeded, setAuthNeeded] = useState(false);
  const [authMode, setAuthMode] = useState<AuthMode>("none");
  const [authChecked, setAuthChecked] = useState(false);

  // Check auth on mount — determine if credentials or api_key login is needed
  useEffect(() => {
    // Register global 401 handler — any API call returning 401 will trigger login
    setOnUnauthorized(() => setAuthNeeded(true));

    checkDashboardAuthMode().then((mode) => {
      setAuthMode(mode);
      if (mode === "none") {
        setAuthNeeded(false);
      } else {
        // credentials or api_key mode: need login unless we already have a stored token
        setAuthNeeded(!hasApiKey());
      }
      setAuthChecked(true);
    });

    return () => setOnUnauthorized(null);
  }, []);

  useEffect(() => {
    const root = window.document.documentElement;
    if (theme === "dark") {
      root.classList.add("dark");
    } else {
      root.classList.remove("dark");
    }
  }, [theme]);

  const navBase = `flex items-center rounded-xl border border-transparent py-2.5 text-sm text-text-dim transition-all duration-300 hover:bg-surface-hover hover:text-brand group ${
    isSidebarCollapsed ? "lg:justify-center lg:px-2 lg:gap-0" : "px-3 gap-3"
  }`;
  const navActive = "border-brand/20 bg-brand/10 text-brand font-semibold shadow-sm shadow-brand/5";

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
        { to: "/network", label: t("nav.network"), icon: Share2 },
        { to: "/a2a", label: t("nav.a2a"), icon: Globe },
        { to: "/runtime", label: t("nav.runtime"), icon: Activity },
        { to: "/logs", label: t("nav.logs"), icon: FileText },
      ],
    },
  ];

  // Block rendering until auth check completes to prevent dashboard flash
  if (!authChecked) {
    return <div className="h-screen bg-main" />;
  }

  if (authNeeded) {
    return <LoginScreen mode={authMode} onAuthenticated={() => {
      setAuthNeeded(false);
      // Reset 401 flag so future 401s can trigger logout again
      setOnUnauthorized(() => setAuthNeeded(true));
    }} />;
  }

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
        fixed inset-y-0 left-0 z-50 flex flex-col border-r border-border-subtle bg-surface/80 backdrop-blur-xl lg:static lg:translate-x-0
        transition-[width,transform] duration-500 ease-[cubic-bezier(0.22,1,0.36,1)]
        ${isMobileMenuOpen ? "translate-x-0 shadow-2xl" : "-translate-x-full"}
        w-[280px] ${isSidebarCollapsed ? "lg:w-[72px]" : "lg:w-[280px]"}
      `}>
        {/* Sidebar Header */}
        <div className={`flex h-16 items-center border-b border-border-subtle transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] ${
          isSidebarCollapsed ? "lg:justify-center lg:px-0" : "justify-between px-4"
        }`}>
          <div className={`flex items-center gap-3 ${isSidebarCollapsed ? "lg:hidden" : ""}`}>
            <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-brand/20 shadow-[0_0_15px_rgba(14,165,233,0.3)] ring-1 ring-brand/40 shrink-0">
              <div className="h-3 w-3 rounded-full bg-brand animate-pulse" />
            </div>
            <div className="flex flex-col">
              <strong className="text-sm font-bold tracking-tight whitespace-nowrap">LibreFang</strong>
              <span className="text-[10px] font-semibold uppercase tracking-wider text-text-dim whitespace-nowrap">{t("common.infrastructure")}</span>
            </div>
          </div>
          {/* Mobile close button */}
          <button
            onClick={() => setMobileMenuOpen(false)}
            className="flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-brand hover:bg-surface-hover transition-all lg:hidden"
          >
            <ChevronLeft className="h-5 w-5" />
          </button>
          {/* Desktop collapse button */}
          <button
            onClick={toggleSidebar}
            className="hidden lg:flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-brand hover:bg-surface-hover transition-all"
            title={isSidebarCollapsed ? "Expand" : "Collapse"}
          >
            {isSidebarCollapsed ? <ChevronRight className="h-4 w-4" /> : <ChevronLeft className="h-4 w-4" />}
          </button>
        </div>

        {/* Navigation */}
        <nav className="overflow-y-auto overflow-x-hidden p-4 scrollbar-thin" style={{ maxHeight: "calc(100vh - 160px)" }}>
          {/* Search Button */}
          <button
            onClick={() => setPaletteOpen(true)}
            className={`mb-4 flex w-full items-center gap-2 rounded-xl border border-border-subtle bg-surface-hover px-3 py-2.5 text-text-dim hover:border-brand/30 hover:text-brand transition-all ${isSidebarCollapsed ? "lg:max-h-0 lg:opacity-0 lg:overflow-hidden lg:!p-0 lg:!m-0 lg:!mb-0" : "lg:max-h-20 lg:opacity-100"} transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] overflow-hidden`}
            title="Search (Cmd+K)"
          >
            <Search className="h-4 w-4" />
            <span className="flex-1 text-left text-xs font-medium">Search</span>
            <kbd className="text-[10px] font-mono bg-main px-1.5 py-0.5 rounded">⌘K</kbd>
          </button>

          <div className={`flex flex-col transition-all duration-500 ${isSidebarCollapsed ? "lg:gap-1" : "gap-6"}`}>
            {navGroups.map((group) => (
              <div key={group.key} className="flex flex-col gap-1">
                {navLayout === "collapsible" ? (
                  // 二级菜单布局 - 可折叠
                  <>
                    <button
                      onClick={() => toggleNavGroup(group.key)}
                      className={`flex items-center justify-between px-3 text-[11px] font-bold uppercase tracking-[0.1em] text-text-dim/80 hover:text-brand transition-colors ${isSidebarCollapsed ? "lg:max-h-0 lg:opacity-0 lg:overflow-hidden lg:!p-0 lg:!m-0 lg:!mb-0" : "lg:max-h-20 lg:opacity-100"} transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] overflow-hidden`}
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
                          <span className={`flex-1 ${isSidebarCollapsed ? "lg:max-h-0 lg:opacity-0 lg:overflow-hidden lg:!p-0 lg:!m-0 lg:!mb-0" : "lg:max-h-20 lg:opacity-100"} transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] overflow-hidden`}>{item.label}</span>
                        </Link>
                      ))}
                    </div>
                  </>
                ) : (
                  // 分组布局 - 全部显示
                  <>
                    <h3 className={`px-3 text-[11px] font-bold uppercase tracking-[0.1em] text-text-dim/80 ${isSidebarCollapsed ? "lg:max-h-0 lg:opacity-0 lg:overflow-hidden lg:!p-0 lg:!m-0 lg:!mb-0" : "lg:max-h-20 lg:opacity-100"} transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] overflow-hidden`}>
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
                          <span className={`flex-1 ${isSidebarCollapsed ? "lg:max-h-0 lg:opacity-0 lg:overflow-hidden lg:!p-0 lg:!m-0 lg:!mb-0" : "lg:max-h-20 lg:opacity-100"} transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] overflow-hidden`}>{item.label}</span>
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
        <div className={`border-t border-border-subtle p-4 ${isSidebarCollapsed ? "lg:max-h-0 lg:opacity-0 lg:overflow-hidden lg:!p-0 lg:!m-0 lg:!mb-0" : "lg:max-h-20 lg:opacity-100"} transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] overflow-hidden`}>
          <div className="rounded-xl bg-gradient-to-r from-success/5 to-transparent p-3 border border-success/10">
            <p className="text-[10px] font-bold text-text-dim uppercase tracking-wider">{t("common.status")}</p>
            <div className="mt-2 flex items-center gap-2">
              <span className="relative flex h-2 w-2">
                <span className="absolute inline-flex h-full w-full rounded-full bg-success opacity-75 animate-ping" />
                <span className="relative inline-flex rounded-full h-2 w-2 bg-success" />
              </span>
              <span className="text-xs font-semibold text-success">{t("common.daemon_online")}</span>
            </div>
          </div>
        </div>
      </aside>

      {/* Main Content Area */}
      <div className="flex flex-1 flex-col overflow-hidden">
        {/* Top Header */}
        <header className="flex h-12 sm:h-14 lg:h-16 shrink-0 items-center justify-between border-b border-border-subtle bg-surface/80 px-3 sm:px-6 backdrop-blur-xl">
          <div className="flex items-center gap-2 min-w-0">
            <button
              onClick={() => setMobileMenuOpen(true)}
              className="flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-brand hover:bg-surface-hover transition-all duration-200 lg:hidden shrink-0"
            >
              <Menu className="h-5 w-5" />
            </button>
            {/* Mobile page title */}
            <span className="text-sm font-bold truncate lg:hidden text-text-dim">
              {navGroups.flatMap(g => g.items).find(i => location.pathname === i.to)?.label || "LibreFang"}
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <Link
              to="/settings"
              className="flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-brand hover:bg-surface-hover transition-all duration-200"
              title={t("nav.settings")}
            >
              <Settings className="h-4 w-4" />
            </Link>
            <button
              onClick={() => setLanguage(language === "en" ? "zh" : "en")}
              className="flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-brand hover:bg-surface-hover transition-all duration-200"
              title={t("common.change_language")}
            >
              <Globe className="h-4 w-4" />
            </button>
            <button
              onClick={toggleTheme}
              className="flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-brand hover:bg-surface-hover transition-all duration-200"
              title={t("common.toggle_theme")}
            >
              {theme === "dark" ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
            </button>
            {authMode !== "none" && hasApiKey() && (
              <button
                onClick={() => { clearApiKey(); setAuthNeeded(true); }}
                className="flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-error hover:bg-error/10 transition-all duration-200"
                title={t("auth.logout", { defaultValue: "Logout" })}
              >
                <LogOut className="h-4 w-4" />
              </button>
            )}
          </div>
        </header>

        {/* Main Content */}
        <main className="flex-1 overflow-auto bg-main">
          <div key={location.pathname} className="mx-auto max-w-7xl p-3 sm:p-4 lg:p-8 pb-[env(safe-area-inset-bottom,12px)] animate-fade-in-up">
            <Outlet />
          </div>
        </main>
      </div>

      <CommandPalette isOpen={isPaletteOpen} onClose={() => setPaletteOpen(false)} />
      <SkillOutputPanel />
    </div>
  );
}
