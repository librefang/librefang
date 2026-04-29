import { Link, Outlet, useRouterState } from "@tanstack/react-router";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AnimatePresence, motion } from "motion/react";
import { fadeInScale, pageTransition } from "./lib/motion";
import {
  Globe,
  Sun,
  Moon,
  Search,
  ChevronLeft,
  ChevronRight,
  ChevronDown,
  Menu,
  Home,
  Layers,
  MessageCircle,
  CheckCircle,
  Calendar,
  Shield,
  Users,
  User,
  Server,
  Network,
  Bell,
  Hand,
  BarChart3,
  Database,
  Activity,
  FileText,
  Settings,
  Puzzle,
  Cpu,
  Lock,
  Share2,
  Gauge,
  LogOut,
  UserCircle,
  X,
  Sparkles,
  Terminal,
  Plug,
} from "lucide-react";
import { useUIStore } from "./lib/store";
import { CommandPalette, useCommandPalette } from "./components/ui/CommandPalette";
import { PushDrawer } from "./components/ui/PushDrawer";
import { ShortcutsHelp } from "./components/ui/ShortcutsHelp";
import { useKeyboardShortcuts } from "./lib/useKeyboardShortcuts";
import { changePassword, checkDashboardAuthMode, clearApiKey, dashboardLogin, dashboardLogout, getDashboardUsername, getStatus, getVersionInfo, setApiKey, setOnUnauthorized, verifyStoredAuth, type AuthMode } from "./api";
import { NotificationCenter } from "./components/NotificationCenter";
import { OfflineBanner } from "./components/OfflineBanner";

function AuthDialog({ mode, onAuthenticated }: { mode: AuthMode; onAuthenticated: () => void }) {
  const { t } = useTranslation();
  const [key, setKey] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [authMethod, setAuthMethod] = useState<"credentials" | "api_key">(
    mode === "api_key" ? "api_key" : "credentials",
  );
  const [errorKey, setErrorKey] = useState<"invalid_api_key" | "invalid_credentials" | "invalid_totp" | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [totpRequired, setTotpRequired] = useState(false);
  const [totpCode, setTotpCode] = useState("");

  useEffect(() => {
    setAuthMethod(mode === "api_key" ? "api_key" : "credentials");
    setErrorKey(null);
    setTotpRequired(false);
    setTotpCode("");
  }, [mode]);

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
      if (totpRequired) {
        if (!totpCode || totpCode.length !== 6) {
          setErrorKey("invalid_totp");
          return;
        }
        const result = await dashboardLogin(username.trim(), password, totpCode);
        if (!result.ok) {
          setErrorKey("invalid_totp");
          return;
        }
        onAuthenticated();
        return;
      }

      if (!username.trim() || !password) {
        setErrorKey("invalid_credentials");
        return;
      }

      const result = await dashboardLogin(username.trim(), password);
      if (result.requires_totp) {
        setTotpRequired(true);
        setTotpCode("");
        return;
      }
      if (!result.ok) {
        setErrorKey("invalid_credentials");
        return;
      }

      onAuthenticated();
    } finally {
      setSubmitting(false);
    }
  }

  const isHybrid = mode === "hybrid";
  const isCredentials = authMethod === "credentials";

  return (
    <div className="fixed inset-0 z-200 flex items-center justify-center bg-black/70 backdrop-blur-md">
      <motion.div className="w-full max-w-md mx-4" variants={fadeInScale} initial="initial" animate="animate">
        <div role="dialog" aria-modal="true" aria-labelledby="auth-dialog-title" className="rounded-2xl border border-border-subtle bg-surface shadow-2xl p-8">
          <div className="flex flex-col items-center mb-6">
            <div className="w-14 h-14 rounded-2xl bg-brand/10 flex items-center justify-center mb-4 ring-2 ring-brand/20">
              {isCredentials ? <User className="h-7 w-7 text-brand" /> : <Lock className="h-7 w-7 text-brand" />}
            </div>
            <h2 id="auth-dialog-title" className="text-xl font-black tracking-tight">{t(isCredentials ? "auth.credentials_title" : "auth.title")}</h2>
            <p className="text-sm text-text-dim mt-1">{t(isCredentials ? "auth.credentials_description" : "auth.description")}</p>
          </div>
          {isHybrid && (
            <div className="mb-4 grid grid-cols-2 gap-2 rounded-xl bg-main p-1">
              <button
                type="button"
                onClick={() => { setAuthMethod("credentials"); setErrorKey(null); setKey(""); setTotpRequired(false); setTotpCode(""); }}
                className={`rounded-lg px-3 py-2 text-sm font-semibold transition-colors ${
                  isCredentials ? "bg-brand text-white shadow-sm" : "text-text-dim hover:text-brand"
                }`}
              >
                {t("auth.credentials_tab")}
              </button>
              <button
                type="button"
                onClick={() => { setAuthMethod("api_key"); setErrorKey(null); setUsername(""); setPassword(""); setTotpRequired(false); setTotpCode(""); }}
                className={`rounded-lg px-3 py-2 text-sm font-semibold transition-colors ${
                  !isCredentials ? "bg-brand text-white shadow-sm" : "text-text-dim hover:text-brand"
                }`}
              >
                {t("auth.api_key_tab")}
              </button>
            </div>
          )}
          <form onSubmit={isCredentials ? handleCredentialsSubmit : handleApiKeySubmit} className="space-y-4">
            {isCredentials && totpRequired ? (
              <>
                <p className="text-sm text-text-dim text-center">{t("auth.totp_prompt")}</p>
                <input
                  type="text"
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  maxLength={6}
                  value={totpCode}
                  onChange={(e) => { setTotpCode(e.target.value.replace(/\D/g, "").slice(0, 6)); setErrorKey(null); }}
                  placeholder="000000"
                  autoFocus
                  className={`w-full rounded-xl border px-4 py-3 text-center text-2xl font-mono tracking-[0.5em] focus:ring-2 outline-none transition-colors ${
                    errorKey === "invalid_totp"
                      ? "border-error focus:border-error focus:ring-error/10"
                      : "border-border-subtle bg-main focus:border-brand focus:ring-brand/10"
                  }`}
                />
              </>
            ) : isCredentials ? (
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
              disabled={submitting || (isCredentials ? (totpRequired ? totpCode.length !== 6 : !username.trim() || !password) : !key.trim())}
              className="w-full rounded-xl bg-brand py-3 text-sm font-bold text-white hover:bg-brand/90 transition-colors shadow-lg shadow-brand/20"
            >
              {totpRequired ? t("auth.verify_totp") : t("auth.submit")}
            </button>
          </form>
        </div>
      </motion.div>
    </div>
  );
}

const INPUT_CLASS = "w-full rounded-xl border border-border-subtle bg-main px-4 py-3 text-sm focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none transition-colors placeholder:text-text-dim/40";

function ChangePasswordModal({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const [currentUsername, setCurrentUsername] = useState("");
  const [newUsername, setNewUsername] = useState("");
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [message, setMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);

  useEffect(() => {
    let cancelled = false;
    getDashboardUsername().then((u) => {
      if (cancelled) return;
      setCurrentUsername(u);
      setNewUsername(u);
    });
    return () => { cancelled = true; };
  }, []);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setMessage(null);

    const changedUsername = newUsername.trim() !== currentUsername.trim() ? newUsername.trim() : null;
    const changedPassword = newPassword || null;

    if (!changedUsername && !changedPassword) {
      setMessage({ type: "error", text: t("settings.pw_no_changes") });
      return;
    }
    if (changedPassword) {
      if (newPassword !== confirmPassword) {
        setMessage({ type: "error", text: t("settings.pw_mismatch") });
        return;
      }
      if (newPassword.length < 8) {
        setMessage({ type: "error", text: t("settings.pw_too_short") });
        return;
      }
    }
    if (changedUsername && changedUsername.length < 2) {
      setMessage({ type: "error", text: t("settings.username_too_short") });
      return;
    }

    setSubmitting(true);
    try {
      const res = await changePassword(currentPassword, changedPassword, changedUsername);
      if (res.ok) {
        setMessage({ type: "success", text: t("settings.pw_success") });
        setTimeout(() => { clearApiKey(); window.location.reload(); }, 1500);
      } else {
        setMessage({ type: "error", text: res.error || t("settings.pw_failed") });
      }
    } catch (err: any) {
      setMessage({ type: "error", text: err.message || t("settings.pw_failed") });
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="fixed inset-0 z-200 flex items-center justify-center bg-black/60 backdrop-blur-sm">
      <motion.div className="w-full max-w-md mx-4" variants={fadeInScale} initial="initial" animate="animate">
        <div role="dialog" aria-modal="true" aria-labelledby="change-credentials-dialog-title" className="rounded-2xl border border-border-subtle bg-surface shadow-2xl">
          <div className="flex items-center justify-between px-6 pt-6 pb-4">
            <h2 id="change-credentials-dialog-title" className="text-base font-black tracking-tight">{t("settings.change_credentials")}</h2>
            <button
              onClick={onClose}
              aria-label={t("common.close", { defaultValue: "Close dialog" })}
              className="h-7 w-7 flex items-center justify-center rounded-lg text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>

          <form onSubmit={handleSubmit}>
            <div className="px-6 space-y-5">
              <div>
                <label className="block text-xs font-semibold text-text-dim mb-1.5">{t("settings.new_username")}</label>
                <input
                  type="text"
                  value={newUsername}
                  onChange={(e) => { setNewUsername(e.target.value); setMessage(null); }}
                  autoComplete="username"
                  autoFocus
                  className={INPUT_CLASS}
                />
              </div>

              <div>
                <div className="flex items-baseline justify-between mb-1.5">
                  <label className="text-xs font-semibold text-text-dim">{t("settings.pw_new")}</label>
                  <span className="text-[10px] text-text-dim/50">{t("settings.pw_leave_blank")}</span>
                </div>
                <input
                  type="password"
                  value={newPassword}
                  onChange={(e) => { setNewPassword(e.target.value); setMessage(null); }}
                  placeholder="••••••••"
                  autoComplete="new-password"
                  className={INPUT_CLASS}
                />
              </div>

              <div className={newPassword ? "" : "opacity-40 pointer-events-none"}>
                <label className="block text-xs font-semibold text-text-dim mb-1.5">{t("settings.pw_confirm")}</label>
                <input
                  type="password"
                  value={confirmPassword}
                  onChange={(e) => { setConfirmPassword(e.target.value); setMessage(null); }}
                  placeholder="••••••••"
                  autoComplete="new-password"
                  tabIndex={newPassword ? 0 : -1}
                  className={`${INPUT_CLASS} ${newPassword && confirmPassword && newPassword !== confirmPassword ? "border-error focus:border-error focus:ring-error/10" : ""}`}
                />
              </div>
            </div>

            <div className="mx-6 mt-5 rounded-xl bg-surface-hover/60 border border-border-subtle px-4 py-3.5">
              <label className="block text-[10px] font-bold uppercase tracking-widest text-text-dim mb-2">{t("settings.pw_verify_identity")}</label>
              <input
                type="password"
                value={currentPassword}
                onChange={(e) => { setCurrentPassword(e.target.value); setMessage(null); }}
                placeholder={t("settings.pw_current_placeholder")}
                autoComplete="current-password"
                className={INPUT_CLASS}
              />
            </div>

            {message && (
              <p className={`mx-6 mt-3 text-xs font-semibold ${message.type === "success" ? "text-success" : "text-error"}`}>
                {message.text}
              </p>
            )}

            <div className="flex gap-3 px-6 py-5">
              <button
                type="button"
                onClick={onClose}
                className="flex-1 rounded-xl border border-border-subtle py-2.5 text-sm font-bold text-text-dim hover:bg-surface-hover transition-colors"
              >
                {t("common.cancel")}
              </button>
              <button
                type="submit"
                disabled={submitting || !currentPassword}
                className="flex-1 rounded-xl bg-brand py-2.5 text-sm font-bold text-white hover:bg-brand/90 transition-colors disabled:opacity-50"
              >
                {submitting ? t("common.saving") : t("common.save")}
              </button>
            </div>
          </form>
        </div>
      </motion.div>
    </div>
  );
}

// Sidebar user-row + dropdown menu. Mirrors the design canvas
// `shell.jsx::Sidebar` footer (avatar + name + chevron) and reuses the
// existing AppShell auth/theme/language wiring. The dropdown is anchored
// above the row so it stays inside the viewport on short screens.
type SidebarUserBlockProps = {
  collapsed: boolean;
  authMode: AuthMode;
  hostname: string;
  username: string;
  onOpenChangePassword: () => void;
  onLogout: () => void | Promise<void>;
  onToggleTheme: () => void;
  onSwitchLanguage: () => void;
  theme: "dark" | "light";
  language: string;
  t: ReturnType<typeof useTranslation>["t"];
};

function SidebarUserBlock({
  collapsed,
  authMode,
  hostname,
  username,
  onOpenChangePassword,
  onLogout,
  onToggleTheme,
  onSwitchLanguage,
  theme,
  language,
  t,
}: SidebarUserBlockProps) {
  const [open, setOpen] = useState(false);
  const initials = (username || "U").slice(0, 2).toUpperCase();

  return (
    <div className="relative border-t border-border-subtle">
      <button
        onClick={() => setOpen((x) => !x)}
        aria-expanded={open}
        aria-haspopup="menu"
        className={`flex w-full items-center gap-2.5 ${collapsed ? "lg:justify-center px-2" : "px-3"} py-2.5 text-left transition-colors ${open ? "bg-brand/5" : "hover:bg-surface-hover"}`}
      >
        <div
          className="h-[26px] w-[26px] rounded-full grid place-items-center text-white text-[11px] font-semibold shrink-0"
          style={{ background: "linear-gradient(135deg,#a78bfa,#7c3aed)" }}
        >
          {initials}
        </div>
        {!collapsed && (
          <>
            <div className="flex-1 min-w-0">
              <div className="text-xs font-medium text-text-main truncate">{username || t("common.user", { defaultValue: "User" })}</div>
              <div className="font-mono text-[10px] text-text-dim truncate">
                {hostname || (language === "en" ? "en-US" : "zh-CN")}
              </div>
            </div>
            <ChevronRight className={`h-3 w-3 text-text-dim transition-transform ${open ? "rotate-90" : ""}`} />
          </>
        )}
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-[90]" onClick={() => setOpen(false)} />
          <div
            className={`absolute z-[100] ${collapsed ? "left-full bottom-1 ml-2" : "left-2 right-2 bottom-full mb-1.5"} rounded-lg border border-border-subtle bg-surface shadow-2xl py-1.5`}
          >
            <button
              onClick={() => { setOpen(false); onToggleTheme(); }}
              className="flex w-full items-center gap-2 px-3 py-2 text-xs font-medium text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
            >
              {theme === "dark" ? <Sun className="h-3.5 w-3.5" /> : <Moon className="h-3.5 w-3.5" />}
              <span className="flex-1 text-left">{t("common.toggle_theme")}</span>
              <span className="font-mono text-[10px] text-text-dim/70">{theme === "dark" ? "dark" : "light"}</span>
            </button>
            <button
              onClick={() => { setOpen(false); onSwitchLanguage(); }}
              className="flex w-full items-center gap-2 px-3 py-2 text-xs font-medium text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
            >
              <Globe className="h-3.5 w-3.5" />
              <span className="flex-1 text-left">{t("common.change_language")}</span>
              <span className="font-mono text-[10px] text-text-dim/70">{language === "en" ? "EN" : "中文"}</span>
            </button>
            <div className="my-1 h-px bg-border-subtle" />
            <Link
              to="/settings"
              onClick={() => setOpen(false)}
              className="flex items-center gap-2 px-3 py-2 text-xs font-medium text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
            >
              <Settings className="h-3.5 w-3.5" />
              <span>{t("nav.settings")}</span>
            </Link>
            <button
              onClick={() => { setOpen(false); onOpenChangePassword(); }}
              className="flex w-full items-center gap-2 px-3 py-2 text-xs font-medium text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
            >
              <Lock className="h-3.5 w-3.5" />
              <span>{t("settings.change_password")}</span>
            </button>
            {authMode !== "none" && (
              <>
                <div className="my-1 h-px bg-border-subtle" />
                <button
                  onClick={async () => { setOpen(false); await onLogout(); }}
                  className="flex w-full items-center gap-2 px-3 py-2 text-xs font-medium text-text-dim hover:text-red-500 hover:bg-surface-hover transition-colors"
                >
                  <LogOut className="h-3.5 w-3.5" />
                  <span>{t("nav.logout")}</span>
                </button>
              </>
            )}
          </div>
        </>
      )}
    </div>
  );
}

// Routes that must fill the remaining viewport height without scrolling.
const FULL_HEIGHT_ROUTES = new Set(["/terminal"]);

// Routes that must render even when no daemon credentials are configured.
// `/connect` is the mobile pairing wizard — by definition the user has
// no API key yet, so the AuthDialog gate would deadlock the first launch.
const NO_AUTH_ROUTES = new Set(["/connect"]);

export function App() {
  const { t } = useTranslation();
  const theme = useUIStore((s) => s.theme);
  const toggleTheme = useUIStore((s) => s.toggleTheme);
  const { location } = useRouterState();
  const isFullHeightPage = FULL_HEIGHT_ROUTES.has(location.pathname);
  const isNoAuthRoute = NO_AUTH_ROUTES.has(location.pathname);
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
  const [username, setUsername] = useState("");
  const [userMenuOpen, setUserMenuOpen] = useState(false);
  const [showChangePassword, setShowChangePassword] = useState(false);
  const [showShortcuts, setShowShortcuts] = useState(false);
  const terminalEnabled = useUIStore((s) => s.terminalEnabled);
  const setTerminalEnabled = useUIStore((s) => s.setTerminalEnabled);

  useKeyboardShortcuts({ onShowHelp: () => setShowShortcuts(true) });

  // Wire up global 401 handler so any failed request re-shows login
  useEffect(() => {
    let cancelled = false;

    // First-run pairing wizard must reach the screen without credentials —
    // skip the auth probe entirely so the AuthDialog never gates `/connect`.
    if (NO_AUTH_ROUTES.has(window.location.pathname)) {
      setAuthNeeded(false);
      setAuthChecked(true);
      return () => {
        cancelled = true;
      };
    }

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
    }).catch(() => { /* Version info is non-essential; silently ignore failure. */ });

    getStatus().then((s) => {
      setTerminalEnabled(s.terminal_enabled !== false);
    }).catch(() => {
      // If status fetch fails, assume terminal is available (fail-open).
      // The WebSocket connection itself will enforce actual policy.
      setTerminalEnabled(true);
    });

    getDashboardUsername().then((u) => {
      if (cancelled) return;
      setUsername(u);
    }).catch(() => { /* unauth or no-auth mode — fine, avatar shows the icon. */ });

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

  // Per design canvas (dashboard/project/app/shell.jsx::SidebarItem): 30px row,
  // 13px font, brand-tinted bg, brand text, with a left-edge sky-blue glow bar
  // marking the active state. Spacing matches the canvas to keep the nav dense
  // enough for 5 sections to fit without scrolling on a 13" laptop.
  const navBase = `relative flex items-center rounded-md border border-transparent text-[13px] text-text-dim transition-colors duration-200 hover:bg-surface-hover hover:text-brand group ${
    isSidebarCollapsed ? "lg:justify-center lg:px-2 lg:gap-0 h-[30px]" : "px-2.5 gap-2.5 h-[30px]"
  }`;
  const navActive = "bg-brand/10 text-brand font-medium before:absolute before:left-[-4px] before:top-1.5 before:bottom-1.5 before:w-[2px] before:rounded-full before:bg-brand before:shadow-[0_0_8px_var(--color-brand)]";

  const navGroups = useMemo(() => {
    const advancedItems = [
      { to: "/comms", label: t("nav.comms"), icon: Activity },
      ...(terminalEnabled ? [{ to: "/terminal" as const, label: t("nav.terminal"), icon: Terminal }] : []),
      { to: "/network", label: t("nav.network"), icon: Share2 },
      { to: "/a2a", label: t("nav.a2a"), icon: Globe },
      { to: "/telemetry", label: t("nav.telemetry"), icon: Gauge },
    ];
    return [
    {
      key: "core",
      label: t("nav.core"),
      items: [
        { to: "/overview", label: t("nav.overview"), icon: Home },
        { to: "/chat", label: t("nav.chat"), icon: MessageCircle },
        { to: "/agents", label: t("nav.agents"), icon: Users },
        { to: "/users", label: t("nav.users", "Users"), icon: Users },
        { to: "/approvals", label: t("nav.approvals"), icon: CheckCircle },
        { to: "/hands", label: t("nav.hands"), icon: Hand },
      ],
    },
    {
      key: "configure",
      label: t("nav.configure"),
      items: [
        { to: "/providers", label: t("nav.providers"), icon: Server },
        { to: "/models", label: t("nav.models"), icon: Cpu },
        { to: "/media", label: t("nav.media"), icon: Sparkles },
        { to: "/channels", label: t("nav.channels"), icon: Network },
        { to: "/skills", label: t("nav.skills"), icon: Bell },
        { to: "/plugins", label: t("nav.plugins"), icon: Puzzle },
        { to: "/mcp-servers", label: t("nav.mcp_servers"), icon: Plug },
      ],
    },
    {
      key: "config",
      label: t("nav.config"),
      items: [
        { to: "/config/general", label: t("config.cat_general"), icon: Settings },
        { to: "/config/memory", label: t("config.cat_memory"), icon: Database },
        { to: "/config/tools", label: t("config.cat_tools"), icon: Sparkles },
        { to: "/config/channels", label: t("config.cat_channels"), icon: Network },
        { to: "/config/security", label: t("config.cat_security"), icon: Shield },
        { to: "/config/network", label: t("config.cat_network"), icon: Share2 },
        { to: "/config/infra", label: t("config.cat_infra"), icon: Server },
      ],
    },
    {
      key: "automate",
      label: t("nav.automate"),
      items: [
        { to: "/workflows", label: t("nav.workflows"), icon: Layers },
        { to: "/scheduler", label: t("nav.scheduler"), icon: Calendar },
        { to: "/goals", label: t("nav.goals"), icon: Shield },
      ],
    },
    {
      key: "observe",
      label: t("nav.observe"),
      items: [
        { to: "/analytics", label: t("nav.analytics"), icon: BarChart3 },
        { to: "/memory", label: t("nav.memory"), icon: Database },
        { to: "/logs", label: t("nav.logs"), icon: FileText },
        { to: "/audit", label: t("nav.audit", "Audit"), icon: FileText },
        { to: "/runtime", label: t("nav.runtime"), icon: Activity },
      ],
    },
    {
      key: "advanced",
      label: t("nav.advanced"),
      items: advancedItems,
    },
  ]; }, [t, terminalEnabled]);

  // Until auth is confirmed, do NOT mount the shell — `<Outlet />` and
  // `<NotificationCenter />` both fire `useDashboardSnapshot` /
  // `useApprovalCount` (5s refetchInterval) the moment they render.
  // Those endpoints sit behind the auth gate, so polling them before the
  // user logs in (or after a token expiry) produces an endless 401 storm
  // in server logs.  Render only the AuthDialog here, then fall through
  // to the full layout once authentication is established.
  if (!isNoAuthRoute && authChecked && authNeeded) {
    return (
      <div className="flex h-screen items-center justify-center bg-main text-slate-900 dark:text-slate-100">
        <AuthDialog
          mode={authMode}
          onAuthenticated={() => { setAuthNeeded(false); window.location.hash = "#/overview"; }}
        />
      </div>
    );
  }

  return (
    <div className="flex h-screen flex-col bg-main text-slate-900 dark:text-slate-100 lg:flex-row transition-colors duration-300 overflow-hidden">
      <a
        href="#main-content"
        className="sr-only focus:not-sr-only focus:fixed focus:top-4 focus:left-4 focus:z-[200] focus:rounded-lg focus:bg-brand focus:px-4 focus:py-2 focus:text-sm focus:font-bold focus:text-white focus:shadow-lg focus:outline-none"
      >
        {t("nav.skip_to_content", { defaultValue: "Skip to content" })}
      </a>

      {isMobileMenuOpen && (
        <div 
          className="fixed inset-0 z-40 bg-black/60 backdrop-blur-sm lg:hidden"
          onClick={() => setMobileMenuOpen(false)}
        />
      )}

      <aside className={`
        fixed inset-y-0 left-0 z-50 flex w-[232px] flex-col border-r border-border-subtle bg-surface lg:static lg:translate-x-0
        transition-[width,transform] duration-500 ease-[cubic-bezier(0.22,1,0.36,1)]
        ${isMobileMenuOpen ? "translate-x-0 shadow-2xl" : "-translate-x-full"}
        ${isSidebarCollapsed ? "lg:w-[64px]" : "lg:w-[232px]"}
      `}>
        {/* Brand block — 26px sky-gradient square with the LibreFang fang glyph,
            "librefang" + "v{version} · prod" subtitle. Mirrors the design's
            shell.jsx::Sidebar header. */}
        <div className={`flex h-14 items-center transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] ${
          isSidebarCollapsed ? "lg:justify-center lg:px-0" : "justify-between px-3.5"
        }`}>
          <div className={`flex items-center gap-2.5 ${isSidebarCollapsed ? "lg:hidden" : ""}`}>
            <div
              className="flex h-[26px] w-[26px] items-center justify-center rounded-[7px] shrink-0 shadow-[0_0_16px_rgba(56,189,248,0.45),inset_0_1px_0_rgba(255,255,255,0.3)]"
              style={{ background: "linear-gradient(135deg,#38bdf8,#0ea5e9)" }}
            >
              <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
                <path d="M2 2 L7 12 L12 2 L9.5 4 L7 8 L4.5 4 Z" fill="#0c1424" stroke="#0c1424" strokeWidth="0.5" strokeLinejoin="round" />
              </svg>
            </div>
            <div className="flex flex-col min-w-0">
              <strong className="text-[13.5px] font-semibold tracking-tight whitespace-nowrap leading-tight">librefang</strong>
              <span className="text-[10px] font-mono text-text-dim/80 whitespace-nowrap leading-tight">
                {appVersion ? `v${appVersion}` : "v0.0.0"} · prod
              </span>
            </div>
          </div>
          <button
            onClick={toggleSidebar}
            className="hidden lg:flex h-7 w-7 items-center justify-center rounded-md text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
            title={isSidebarCollapsed ? t("nav.expand_sidebar", { defaultValue: "Expand sidebar" }) : t("nav.collapse_sidebar", { defaultValue: "Collapse sidebar" })}
            aria-label={isSidebarCollapsed ? t("nav.expand_sidebar", { defaultValue: "Expand sidebar" }) : t("nav.collapse_sidebar", { defaultValue: "Collapse sidebar" })}
            aria-expanded={!isSidebarCollapsed}
          >
            {isSidebarCollapsed ? <ChevronRight className="h-3.5 w-3.5" /> : <ChevronLeft className="h-3.5 w-3.5" />}
          </button>
        </div>

        <nav className="overflow-y-auto overflow-x-hidden px-2 pb-3 scrollbar-thin max-h-[calc(100vh-140px)]">
          <button
            onClick={() => setPaletteOpen(true)}
            className={`mx-1 mb-3 flex items-center gap-2 rounded-lg border border-border-subtle bg-surface-hover/60 px-2.5 h-8 text-text-dim hover:border-brand/30 hover:text-brand ${isSidebarCollapsed ? "lg:max-h-0 lg:opacity-0 lg:overflow-hidden lg:p-0! lg:m-0! lg:mb-0!" : "lg:max-h-20 lg:opacity-100"} transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] overflow-hidden`}
            title={`${t("common.search")} (⌘K)`}
            aria-label={`${t("common.search")} (⌘K)`}
            style={{ width: "calc(100% - 8px)" }}
          >
            <Search className="h-3.5 w-3.5" />
            <span className="flex-1 text-left text-xs">{t("common.search")}…</span>
            <kbd className="text-[10px] font-mono bg-main border border-border-subtle px-1 py-px rounded">⌘K</kbd>
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
                          title={isSidebarCollapsed ? item.label : undefined}
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
                          title={isSidebarCollapsed ? item.label : undefined}
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

        {/* User-avatar footer — opens the unified user menu (theme / language /
            settings / change credentials / logout). Replaces the old "daemon
            online" status pane. Hostname & version moved into the brand block /
            user menu so this row stays compact. */}
        <SidebarUserBlock
          collapsed={isSidebarCollapsed}
          authMode={authMode}
          hostname={hostname}
          username={username}
          onOpenChangePassword={() => setShowChangePassword(true)}
          onLogout={async () => { await dashboardLogout(); window.location.reload(); }}
          onToggleTheme={toggleTheme}
          onSwitchLanguage={() => setLanguage(language === "en" ? "zh" : "en")}
          theme={theme}
          language={language}
          t={t}
        />
      </aside>

      <div className="flex flex-1 flex-col overflow-hidden">
        {/* Compact topbar (h-12, ~48px). Theme/language/avatar moved into the
            sidebar's user-row dropdown to match the design. Notifications
            stays inline as a single iconed button. Mobile keeps a hamburger
            and the brand block since the sidebar is hidden. */}
        <header className="flex h-12 shrink-0 items-center justify-between border-b border-border-subtle bg-surface/80 backdrop-blur-md px-3 sm:px-4">
          <div className="flex items-center gap-2 min-w-0">
            <button
              onClick={() => setMobileMenuOpen(true)}
              className="flex h-8 w-8 items-center justify-center rounded-md text-text-dim hover:text-brand hover:bg-surface-hover transition-colors duration-200 lg:hidden"
              aria-label={t("nav.open_menu", { defaultValue: "Open navigation menu" })}
              aria-expanded={isMobileMenuOpen}
            >
              <Menu className="h-4 w-4" />
            </button>
            <div className="flex items-center gap-2 lg:hidden">
              <div
                className="flex h-6 w-6 items-center justify-center rounded-md shrink-0 shadow-[0_0_12px_rgba(56,189,248,0.4)]"
                style={{ background: "linear-gradient(135deg,#38bdf8,#0ea5e9)" }}
              >
                <svg width="11" height="11" viewBox="0 0 14 14" fill="none" aria-hidden="true">
                  <path d="M2 2 L7 12 L12 2 L9.5 4 L7 8 L4.5 4 Z" fill="#0c1424" />
                </svg>
              </div>
              <strong className="text-[13px] font-semibold tracking-tight">librefang</strong>
            </div>
            {/* Desktop: hostname / page hint */}
            <div className="hidden lg:flex items-center gap-2 text-text-dim min-w-0">
              {hostname && (
                <span className="font-mono text-[11px] truncate">{hostname}</span>
              )}
            </div>
          </div>
          <div className="flex items-center gap-1">
            <NotificationCenter />
            {/* Avatar button — top-right pattern from the design canvas
                (`shell.jsx::TopBar`, "user-menu" variant). Visible on every
                breakpoint so the menu is always one click away from the
                topbar; the sidebar's user-row dropdown is the secondary
                "user-menu-sidebar" variant. */}
            <div className="relative">
              <button
                onClick={() => setUserMenuOpen(!userMenuOpen)}
                className={`flex h-7 w-7 items-center justify-center rounded-full transition-colors duration-200 active:scale-95 ${
                  userMenuOpen
                    ? "ring-2 ring-brand/40 ring-offset-1 ring-offset-surface"
                    : "ring-1 ring-border-subtle hover:ring-brand/30"
                }`}
                style={{ background: "linear-gradient(135deg,#a78bfa,#7c3aed)" }}
                title={t("nav.user_center")}
                aria-label={t("nav.user_center")}
                aria-expanded={userMenuOpen}
                aria-haspopup="menu"
              >
                {username ? (
                  <span className="text-white text-[10px] font-semibold">
                    {username.slice(0, 2).toUpperCase()}
                  </span>
                ) : (
                  <UserCircle className="h-4 w-4 text-white" />
                )}
              </button>
              {userMenuOpen && (
                <>
                  <div className="fixed inset-0 z-[90]" onClick={() => setUserMenuOpen(false)} />
                  {/* Use position:fixed so the menu is not clipped by the
                      ancestor `overflow-hidden` flex column. Anchor to the
                      topbar bottom (h-12 = 48px) + a 6px gap, flush to the
                      right padding (px-3 / px-4 mobile / desktop). */}
                  <div className="fixed top-[54px] right-3 sm:right-4 z-[100] w-52 rounded-lg border border-border-subtle bg-surface shadow-2xl py-1.5">
                    <button
                      onClick={() => { setUserMenuOpen(false); toggleTheme(); }}
                      className="flex w-full items-center gap-2 px-3 py-2 text-xs font-medium text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
                    >
                      {theme === "dark" ? <Sun className="h-3.5 w-3.5" /> : <Moon className="h-3.5 w-3.5" />}
                      <span className="flex-1 text-left">{t("common.toggle_theme")}</span>
                    </button>
                    <button
                      onClick={() => { setUserMenuOpen(false); setLanguage(language === "en" ? "zh" : "en"); }}
                      className="flex w-full items-center gap-2 px-3 py-2 text-xs font-medium text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
                    >
                      <Globe className="h-3.5 w-3.5" />
                      <span className="flex-1 text-left">{t("common.change_language")}</span>
                    </button>
                    <div className="my-1 h-px bg-border-subtle" />
                    <Link
                      to="/settings"
                      onClick={() => setUserMenuOpen(false)}
                      className="flex items-center gap-2 px-3 py-2 text-xs font-medium text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
                    >
                      <Settings className="h-3.5 w-3.5" />
                      {t("nav.settings")}
                    </Link>
                    <button
                      onClick={() => { setUserMenuOpen(false); setShowChangePassword(true); }}
                      className="flex w-full items-center gap-2 px-3 py-2 text-xs font-medium text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
                    >
                      <Lock className="h-3.5 w-3.5" />
                      {t("settings.change_password")}
                    </button>
                    {authMode !== "none" && (
                      <button
                        onClick={async () => { await dashboardLogout(); window.location.reload(); }}
                        className="flex w-full items-center gap-2 px-3 py-2 text-xs font-medium text-text-dim hover:text-red-500 hover:bg-surface-hover transition-colors"
                      >
                        <LogOut className="h-3.5 w-3.5" />
                        {t("nav.logout")}
                      </button>
                    )}
                  </div>
                </>
              )}
            </div>
          </div>
        </header>

        <main
          id="main-content"
          className={`bg-main ${isFullHeightPage ? "flex flex-col flex-1 overflow-hidden" : "flex-1 overflow-y-auto overflow-x-hidden"}`}
          tabIndex={-1}
        >
          <AnimatePresence mode="wait" initial={false}>
            {isFullHeightPage ? (
              <motion.div
                key={`full:${location.pathname}`}
                className="flex flex-col flex-1 min-h-0"
                variants={pageTransition}
                initial="initial"
                animate="animate"
                exit="exit"
              >
                <Outlet />
              </motion.div>
            ) : (
              <motion.div
                key={`std:${location.pathname}`}
                className="w-full p-3 sm:p-4 lg:p-8"
                variants={pageTransition}
                initial="initial"
                animate="animate"
                exit="exit"
              >
                <Outlet />
              </motion.div>
            )}
          </AnimatePresence>
        </main>
      </div>

      {!isNoAuthRoute && <OfflineBanner />}
      <PushDrawer />

      <CommandPalette isOpen={isPaletteOpen} onClose={() => setPaletteOpen(false)} />
      <ShortcutsHelp isOpen={showShortcuts} onClose={() => setShowShortcuts(false)} />
      {showChangePassword && <ChangePasswordModal onClose={() => setShowChangePassword(false)} />}
    </div>
  );
}
