import { Link, Outlet } from "@tanstack/react-router";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Globe, Sun, Moon, Search, ChevronLeft, ChevronRight, ChevronDown, Menu, Home, Layers, MessageCircle, Clock, CheckCircle, Calendar, Shield, Users, User, Server, Network, Bell, Hand, BarChart3, Database, Activity, FileText, Settings, Puzzle, Cpu, Lock, Share2, Gauge } from "lucide-react";
import { useUIStore } from "./lib/store";
import { CommandPalette, useCommandPalette } from "./components/ui/CommandPalette";
import { checkDashboardAuthMode, dashboardLogin, getVersionInfo, setApiKey, setOnUnauthorized, verifyStoredAuth, type AuthMode } from "./api";

function AuthDialog({ mode, onAuthenticated }: { mode: AuthMode; onAuthenticated: () => void }) {
  const { t } = useTranslation();
  const [key, setKey] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [errorKey, setErrorKey] = useState<"invalid_api_key" | "invalid_credentials" | null>(null);
  const [submitting, setSubmitting] = useState(false);

  async function handleApiKeySubmit(e: React.FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    setErrorKey(null);

    try {
      if (!key.trim()) {
        setErrorKey("invalid_api_key");
        return;
      }

      setApiKey(key.trim());
      const isAuthenticated = await verifyStoredAuth();
      if (!isAuthenticated) {
        setErrorKey("invalid_api_key");
        return;
      }

      onAuthenticated();
    } finally {
      setSubmitting(false);
    }
  }

  async function handleCredentialsSubmit(e: React.FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    setErrorKey(null);

    try {
      if (!username.trim() || !password) {
        setErrorKey("invalid_credentials");
        return;
      }

      const result = await dashboardLogin(username.trim(), password);
      if (!result.ok) {
        setErrorKey("invalid_credentials");
        return;
      }

      // The login response already proves the credential path succeeded.
      // Avoid immediately probing session-backed auth before the new session
      // is fully visible server-side.
      onAuthenticated();
    } finally {
      setSubmitting(false);
    }
  }

  const isCredentials = mode === "credentials";

  return (
    <div className="fixed inset-0 z-200 flex items-center justify-center bg-black/70 backdrop-blur-md">
      <div className="w-full max-w-md mx-4 animate-fade-in-scale">
        <div className="rounded-2xl border border-border-subtle bg-surface shadow-2xl p-8">
          <div className="flex flex-col items-center mb-6">
            <div className="w-14 h-14 rounded-2xl bg-brand/10 flex items-center justify-center mb-4 ring-2 ring-brand/20">
              {isCredentials ? <User className="h-7 w-7 text-brand" /> : <Lock className="h-7 w-7 text-brand" />}
            </div>
            <h2 className="text-xl font-black tracking-tight">{t(isCredentials ? "auth.credentials_title" : "auth.title")}</h2>
            <p className="text-sm text-text-dim mt-1">{t(isCredentials ? "auth.credentials_description" : "auth.description")}</p>
          </div>
          <form onSubmit={isCredentials ? handleCredentialsSubmit : handleApiKeySubmit} className="space-y-4">
            {isCredentials ? (
              <>
                <input
                  type="text"
                  value={username}
                  onChange={(e) => { setUsername(e.target.value); setErrorKey(null); }}
                  placeholder={t("auth.username_placeholder")}
                  autoFocus
                  className={`w-full rounded-xl border px-4 py-3 text-sm focus:ring-2 outline-none transition-colors ${
                    errorKey
                      ? "border-error focus:border-error focus:ring-error/10"
                      : "border-border-subtle bg-main focus:border-brand focus:ring-brand/10"
                  }`}
                />
                <input
                  type="password"
                  value={password}
                  onChange={(e) => { setPassword(e.target.value); setErrorKey(null); }}
                  placeholder={t("auth.password_placeholder")}
                  className={`w-full rounded-xl border px-4 py-3 text-sm focus:ring-2 outline-none transition-colors ${
                    errorKey
                      ? "border-error focus:border-error focus:ring-error/10"
                      : "border-border-subtle bg-main focus:border-brand focus:ring-brand/10"
                  }`}
                />
              </>
            ) : (
              <input
                type="password"
                value={key}
                onChange={(e) => { setKey(e.target.value); setErrorKey(null); }}
                placeholder={t("auth.placeholder")}
                autoFocus
                className={`w-full rounded-xl border px-4 py-3 text-sm focus:ring-2 outline-none transition-colors ${
                  errorKey
                    ? "border-error focus:border-error focus:ring-error/10"
                    : "border-border-subtle bg-main focus:border-brand focus:ring-brand/10"
                }`}
              />
            )}
            {errorKey && (
              <p className="text-xs text-error font-medium">{t(`auth.${errorKey}`)}</p>
            )}
            <button
              type="submit"
              disabled={submitting || (isCredentials ? !username.trim() || !password : !key.trim())}
              className="w-full rounded-xl bg-brand py-3 text-sm font-bold text-white hover:bg-brand/90 transition-colors shadow-lg shadow-brand/20"
            >
              {t("auth.submit")}
            </button>
          </form>
        </div>
      </div>
    </div>
  );
}

export function App() {
  const { t } = useTranslation();
  const theme = useUIStore((s) => s.theme);
  const toggleTheme = useUIStore((s) => s.toggleTheme);
  const language = useUIStore((s) => s.language);
  const setLanguage = useUIStore((s) => s.setLanguage);
  const isMobileMenuOpen = useUIStore((s) => s.isMobileMenuOpen);
  const setMobileMenuOpen = useUIStore((s) => s.setMobileMenuOpen);
  const isSidebarCollapsed = useUIStore((s) => s.isSidebarCollapsed);
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const navLayout = useUIStore((s) => s.navLayout);
  const collapsedNavGroups = useUIStore((s) => s.collapsedNavGroups);
  const toggleNavGroup = useUIStore((s) => s.toggleNavGroup);
  const { isOpen: isPaletteOpen, setIsOpen: setPaletteOpen } = useCommandPalette();
  const [authNeeded, setAuthNeeded] = useState(false);
  const [authChecked, setAuthChecked] = useState(false);
  const [authMode, setAuthMode] = useState<AuthMode>("none");
  const [appVersion, setAppVersion] = useState("");
  const [hostname, setHostname] = useState("");

  // Wire up global 401 handler so any failed request re-shows login
  useEffect(() => {
    let cancelled = false;

    setOnUnauthorized(() => {
      checkDashboardAuthMode().then((mode) => {
        if (cancelled) {
          return;
        }
        setAuthMode(mode === "none" ? "api_key" : mode);
        setAuthNeeded(true);
        setAuthChecked(true);
      });
    });

    const checkAuth = async () => {
      const mode = await checkDashboardAuthMode();
      if (cancelled) {
        return;
      }

      setAuthMode(mode);
      if (mode === "none") {
        setAuthNeeded(false);
        setAuthChecked(true);
        return;
      }

      const authenticated = await verifyStoredAuth();
      if (cancelled) {
        return;
      }

      setAuthNeeded(!authenticated);
      setAuthChecked(true);
    };

    void checkAuth();
    getVersionInfo().then((v) => {
      setAppVersion(v.version ?? "");
      setHostname(v.hostname ?? "");
    }).catch(() => {});

    return () => {
      cancelled = true;
      setOnUnauthorized(null);
    };
  }, []);

  useEffect(() => {
    const root = window.document.documentElement;
    if (theme === "dark") {
      root.classList.add("dark");
    } else {
      root.classList.remove("dark");
    }
  }, [theme]);

  const navBase = `flex items-center rounded-xl border border-transparent py-2.5 text-sm text-text-dim transition-colors duration-200 hover:bg-surface-hover hover:text-brand group ${
    isSidebarCollapsed ? "lg:justify-center lg:px-2 lg:gap-0" : "px-3 gap-3"
  }`;
  const navActive = "border-brand/20 bg-brand/10 text-brand font-semibold shadow-sm shadow-brand/5";

  const navGroups = useMemo(() => [
    {
      key: "core",
      label: t("nav.core"),
      items: [
        { to: "/overview", label: t("nav.overview"), icon: Home },
        { to: "/hands", label: t("nav.hands"), icon: Hand },
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
        { to: "/telemetry", label: t("nav.telemetry"), icon: Gauge },
        { to: "/logs", label: t("nav.logs"), icon: FileText },
      ],
    },
  ], [t]);

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
        fixed inset-y-0 left-0 z-50 flex w-[220px] flex-col border-r border-border-subtle bg-surface lg:static lg:translate-x-0
        transition-[width,transform] duration-500 ease-[cubic-bezier(0.22,1,0.36,1)]
        ${isMobileMenuOpen ? "translate-x-0 shadow-2xl" : "-translate-x-full"}
        ${isSidebarCollapsed ? "lg:w-24" : "lg:w-[280px]"}
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
          <button
            onClick={toggleSidebar}
            className="hidden lg:flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
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
            className={`mb-4 flex w-full items-center gap-2 rounded-xl border border-border-subtle bg-surface-hover px-3 py-2.5 text-text-dim hover:border-brand/30 hover:text-brand ${isSidebarCollapsed ? "lg:max-h-0 lg:opacity-0 lg:overflow-hidden lg:p-0! lg:m-0! lg:mb-0!" : "lg:max-h-20 lg:opacity-100"} transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] overflow-hidden`}
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
                      className={`flex items-center justify-between px-3 text-[11px] font-bold uppercase tracking-widest text-text-dim/80 hover:text-brand transition-colors ${isSidebarCollapsed ? "lg:max-h-0 lg:opacity-0 lg:overflow-hidden lg:p-0! lg:m-0! lg:mb-0!" : "lg:max-h-20 lg:opacity-100"} transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] overflow-hidden`}
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
                          <span className={`flex-1 ${isSidebarCollapsed ? "lg:max-h-0 lg:opacity-0 lg:overflow-hidden lg:p-0! lg:m-0! lg:mb-0!" : "lg:max-h-20 lg:opacity-100"} transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] overflow-hidden`}>{item.label}</span>
                        </Link>
                      ))}
                    </div>
                  </>
                ) : (
                  // 分组布局 - 全部显示
                  <>
                    <h3 className={`px-3 text-[11px] font-bold uppercase tracking-widest text-text-dim/80 ${isSidebarCollapsed ? "lg:max-h-0 lg:opacity-0 lg:overflow-hidden lg:p-0! lg:m-0! lg:mb-0!" : "lg:max-h-20 lg:opacity-100"} transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] overflow-hidden`}>
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
                          <span className={`flex-1 ${isSidebarCollapsed ? "lg:max-h-0 lg:opacity-0 lg:overflow-hidden lg:p-0! lg:m-0! lg:mb-0!" : "lg:max-h-20 lg:opacity-100"} transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] overflow-hidden`}>{item.label}</span>
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
        <div className={`border-t border-border-subtle p-4 ${isSidebarCollapsed ? "lg:max-h-0 lg:opacity-0 lg:overflow-hidden lg:p-0! lg:m-0! lg:mb-0!" : "lg:max-h-28 lg:opacity-100"} transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] overflow-hidden`}>
          <div className="rounded-xl bg-linear-to-r from-success/5 to-transparent p-3 border border-success/10">
            <p className="text-[10px] font-bold text-text-dim uppercase tracking-wider">{t("common.status")}</p>
            <div className="mt-2 flex items-center gap-2">
              <span className="relative flex h-2 w-2 shrink-0">
                <span className="absolute inline-flex h-full w-full rounded-full bg-success opacity-75 animate-pulse" />
                <span className="relative inline-flex rounded-full h-2 w-2 bg-success" />
              </span>
              <span className="text-xs font-semibold text-success">{t("common.daemon_online")}</span>
            </div>
            {(appVersion || hostname) && (
              <div className="mt-1.5 space-y-0.5 text-[10px] font-mono text-text-dim">
                {appVersion && <p className="truncate">v{appVersion}</p>}
                {hostname && <p className="truncate">{hostname}</p>}
              </div>
            )}
          </div>
        </div>
      </aside>

      {/* Main Content Area */}
      <div className="flex flex-1 flex-col overflow-hidden">
        {/* Top Header */}
        <header className="flex h-14 sm:h-16 shrink-0 items-center justify-between border-b border-border-subtle bg-surface px-3 sm:px-6">
          <div className="flex items-center gap-2">
            <button
              onClick={() => setMobileMenuOpen(true)}
              className="flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-brand hover:bg-surface-hover transition-colors duration-200 lg:hidden"
            >
              <Menu className="h-5 w-5" />
            </button>
            <div className="flex items-center gap-2 lg:hidden">
              <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-brand/20 ring-1 ring-brand/40 shrink-0">
                <div className="h-2.5 w-2.5 rounded-full bg-brand animate-pulse" />
              </div>
              <strong className="text-sm font-bold tracking-tight">LibreFang</strong>
            </div>
          </div>
          <div className="flex items-center gap-1">
            <Link
              to="/settings"
              className="flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-brand hover:bg-surface-hover transition-colors duration-200"
              title={t("nav.settings")}
            >
              <Settings className="h-4 w-4" />
            </Link>
            <button
              onClick={() => setLanguage(language === "en" ? "zh" : "en")}
              className="flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-brand hover:bg-surface-hover transition-colors duration-200"
              title={t("common.change_language")}
            >
              <Globe className="h-4 w-4" />
            </button>
            <button
              onClick={toggleTheme}
              className="flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-brand hover:bg-surface-hover transition-colors duration-200"
              title={t("common.toggle_theme")}
            >
              {theme === "dark" ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
            </button>
          </div>
        </header>

        {/* Main Content */}
        <main className="flex-1 overflow-y-auto overflow-x-hidden bg-main">
          <div className="mx-auto max-w-7xl p-3 sm:p-4 lg:p-8">
            <Outlet />
          </div>
        </main>
      </div>

      <CommandPalette isOpen={isPaletteOpen} onClose={() => setPaletteOpen(false)} />
      {authChecked && authNeeded && (
        <AuthDialog mode={authMode} onAuthenticated={() => { setAuthNeeded(false); window.location.hash = "#/overview"; }} />
      )}
    </div>
  );
}
