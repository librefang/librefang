import { useTranslation } from "react-i18next";
import { useState } from "react";
import { PageHeader } from "../components/ui/PageHeader";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import {
  Globe, Sun, Moon, Settings, PanelLeftClose, PanelLeft, Languages, LayoutDashboard,
  Shield, CheckCircle, XCircle, Download, Play, Square,
  Database, ArrowRight, Loader2, Link, Unlink,
} from "lucide-react";
import { useUIStore } from "../lib/store";
import { useAutoDreamStatus } from "../lib/queries/autoDream";
import { useTotpStatus } from "../lib/queries/approvals";
import { useStorageConfig, useStorageStatus } from "../lib/queries/storage";
import {
  useTriggerAutoDream,
  useAbortAutoDream,
  useSetAutoDreamEnabled,
} from "../lib/mutations/autoDream";
import {
  useTotpSetup,
  useTotpConfirm,
  useTotpRevoke,
} from "../lib/mutations/approvals";
import {
  useMigrateStorage,
  useLinkUarStorage,
  useUnlinkUarStorage,
} from "../lib/mutations/storage";
import type { AutoDreamAgentStatus } from "../api";

interface SegmentOption<T extends string> {
  value: T;
  icon: React.ElementType;
  label: string;
}

function SegmentControl<T extends string>({
  options,
  value,
  onChange,
}: {
  options: SegmentOption<T>[];
  value: T;
  onChange: (v: T) => void;
}) {
  return (
    <div className="flex bg-main rounded-lg p-0.5 border border-border-subtle gap-0.5 shrink-0">
      {options.map((opt) => {
        const active = opt.value === value;
        return (
          <button
            key={opt.value}
            onClick={() => onChange(opt.value)}
            className={`flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-semibold transition-all duration-150 ${
              active
                ? "bg-surface shadow-sm text-brand border border-brand/15"
                : "text-text-dim hover:text-text"
            }`}
          >
            <opt.icon className="w-3 h-3 shrink-0" />
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}

function SettingRow({
  icon: Icon,
  iconColor,
  label,
  description,
  children,
}: {
  icon: React.ElementType;
  iconColor: string;
  label: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-4 py-4 border-b border-border-subtle/50 last:border-0">
      <Icon className={`w-4 h-4 shrink-0 ${iconColor}`} />
      <div className="flex-1 min-w-0">
        <p className="text-sm font-semibold">{label}</p>
        <p className="text-xs text-text-dim mt-0.5">{description}</p>
      </div>
      {children}
    </div>
  );
}

export function SettingsPage() {
  const { t } = useTranslation();
  const theme = useUIStore((s) => s.theme);
  const toggleTheme = useUIStore((s) => s.toggleTheme);
  const language = useUIStore((s) => s.language);
  const setLanguage = useUIStore((s) => s.setLanguage);
  const navLayout = useUIStore((s) => s.navLayout);
  const setNavLayout = useUIStore((s) => s.setNavLayout);
  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("settings.system_config")}
        title={t("settings.title")}
        subtitle={t("settings.subtitle")}
        icon={<Settings className="h-4 w-4" />}

      />

      <div className="rounded-2xl border border-border-subtle bg-surface">
        <div className="px-5 py-3 border-b border-border-subtle/50">
          <p className="text-[10px] font-black uppercase tracking-widest text-text-dim">
            {t("settings.appearance")}
          </p>
        </div>
        <div className="px-5">
          <SettingRow
            icon={theme === "dark" ? Moon : Sun}
            iconColor="text-amber-500"
            label={t("settings.theme")}
            description={t("settings.theme_desc")}
          >
            <SegmentControl
              value={theme}
              onChange={(v) => v !== theme && toggleTheme()}
              options={[
                { value: "light", icon: Sun, label: t("settings.theme_light") },
                { value: "dark", icon: Moon, label: t("settings.theme_dark") },
              ]}
            />
          </SettingRow>

          <SettingRow
            icon={Languages}
            iconColor="text-sky-500"
            label={t("settings.language")}
            description={t("settings.language_desc")}
          >
            <SegmentControl
              value={language}
              onChange={setLanguage}
              options={[
                { value: "en", icon: Globe, label: "English" },
                { value: "zh", icon: Globe, label: "中文" },
              ]}
            />
          </SettingRow>

          <SettingRow
            icon={LayoutDashboard}
            iconColor="text-violet-500"
            label={t("settings.nav_layout")}
            description={t("settings.nav_layout_desc")}
          >
            <SegmentControl
              value={navLayout}
              onChange={setNavLayout}
              options={[
                { value: "grouped", icon: PanelLeft, label: t("settings.nav_grouped") },
                { value: "collapsible", icon: PanelLeftClose, label: t("settings.nav_collapsible") },
              ]}
            />
          </SettingRow>
        </div>
      </div>

      {/* TOTP Second Factor */}
      <TotpSection />

      {/* Config Backup */}
      <ConfigBackupSection />

      {/* Auto-Dream (background memory consolidation) */}
      <AutoDreamSection />

      {/* Storage Administration */}
      <StorageSection />
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  TOTP Management Section                                            */
/* ------------------------------------------------------------------ */

function TotpSection() {
  const { t } = useTranslation();
  const [setupData, setSetupData] = useState<{ otpauth_uri: string; secret: string; qr_code: string | null; recovery_codes: string[] } | null>(null);
  const [confirmCode, setConfirmCode] = useState("");
  const [resetCode, setResetCode] = useState("");
  const [revokeCode, setRevokeCode] = useState("");
  const [showResetPrompt, setShowResetPrompt] = useState(false);
  const [showRevokePrompt, setShowRevokePrompt] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  const statusQuery = useTotpStatus();
  const setupTotp = useTotpSetup();
  const confirmTotp = useTotpConfirm();
  const revokeTotp = useTotpRevoke();

  const status = statusQuery.data;
  const loading =
    setupTotp.isPending || confirmTotp.isPending || revokeTotp.isPending;

  async function handleSetup(currentCode?: string) {
    if (loading) return;
    setError(null);
    setSuccess(null);
    try {
      const data = await setupTotp.mutateAsync(currentCode);
      setSetupData({ otpauth_uri: data.otpauth_uri, secret: data.secret, qr_code: data.qr_code, recovery_codes: data.recovery_codes });
      setShowResetPrompt(false);
      setResetCode("");
    } catch (e) {
      setError(e instanceof Error ? e.message : t("settings.totp_setup_failed", "Setup failed"));
    }
  }

  function initiateSetup() {
    if (status?.confirmed) {
      setShowResetPrompt(true);
      setShowRevokePrompt(false);
      setError(null);
    } else {
      handleSetup();
    }
  }

  async function handleRevoke() {
    if (loading) return;
    if (!revokeCode) return;
    setError(null);
    setSuccess(null);
    try {
      await revokeTotp.mutateAsync(revokeCode);
      setSuccess(t("settings.totp_revoked_success", "TOTP revoked. Set second_factor = \"none\" in config."));
      setShowRevokePrompt(false);
      setRevokeCode("");
      setSetupData(null);
      setConfirmCode("");
    } catch (e) {
      setError(e instanceof Error ? e.message : t("settings.totp_revoke_failed", "Revoke failed"));
    }
  }

  async function handleConfirm() {
    if (loading) return;
    if (confirmCode.length !== 6) return;
    setError(null);
    setSuccess(null);
    try {
      await confirmTotp.mutateAsync(confirmCode);
      setSuccess(t("settings.totp_confirmed_success", "TOTP confirmed. Set second_factor = \"totp\" in config to enforce."));
      setSetupData(null);
      setConfirmCode("");
    } catch (e) {
      setError(e instanceof Error ? e.message : t("settings.totp_invalid_code", "Invalid code"));
    }
  }

  return (
    <div className="rounded-2xl border border-border-subtle bg-surface">
      <div className="px-5 py-3 border-b border-border-subtle/50">
        <p className="text-[10px] font-black uppercase tracking-widest text-text-dim">
          {t("settings.security", "Security")}
        </p>
      </div>
      <div className="px-5">
        <SettingRow
          icon={Shield}
          iconColor="text-emerald-500"
          label={t("settings.totp_title", "TOTP Second Factor")}
          description={t("settings.totp_desc", "Require authenticator app code when approving critical tool executions")}
        >
          <div className="flex items-center gap-2">
            {status?.confirmed ? (
              <Badge variant="success">
                <CheckCircle className="w-3 h-3 mr-1" />
                {t("settings.totp_enrolled", "Enrolled")}
              </Badge>
            ) : (
              <Badge variant="default">
                <XCircle className="w-3 h-3 mr-1" />
                {t("settings.totp_not_enrolled", "Not enrolled")}
              </Badge>
            )}
            {status?.enforced && (
              <Badge variant="info">{t("settings.totp_enforced", "Enforced")}</Badge>
            )}
          </div>
        </SettingRow>


        {status?.confirmed && status.remaining_recovery_codes <= 2 && (
          <div className="px-1 py-2 text-sm text-warning flex items-center gap-2">
            <Shield className="w-4 h-4 shrink-0" />
            {status.remaining_recovery_codes === 0
              ? t("settings.totp_no_recovery", "No recovery codes remaining. Reset TOTP to generate new ones.")
              : t("settings.totp_low_recovery", {
                  defaultValue: "Only {{count}} recovery code(s) remaining.",
                  count: status.remaining_recovery_codes,
                })}
          </div>
        )}

        <div className="py-4">
          {showResetPrompt && !setupData ? (
            <div className="flex flex-col sm:flex-row sm:items-center gap-2">
              <input
                type="text"
                value={resetCode}
                onChange={(e) => setResetCode(e.target.value)}
                placeholder={t("settings.totp_reset_placeholder", "Current TOTP or recovery code")}
                className="w-full sm:w-48 rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm font-mono focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none transition-colors"
                onKeyDown={(e) => e.key === "Enter" && resetCode && !loading && handleSetup(resetCode)}
              />
              <Button variant="primary" size="sm" onClick={() => handleSetup(resetCode)} disabled={!resetCode || loading} isLoading={loading}>
                {t("settings.totp_verify_reset", "Verify & Reset")}
              </Button>
              <Button variant="ghost" size="sm" onClick={() => { setShowResetPrompt(false); setResetCode(""); }}>
                {t("common.cancel", "Cancel")}
              </Button>
            </div>
          ) : showRevokePrompt && !setupData ? (
            <div className="flex flex-col sm:flex-row sm:items-center gap-2">
              <input
                type="text"
                value={revokeCode}
                onChange={(e) => setRevokeCode(e.target.value)}
                placeholder={t("settings.totp_revoke_placeholder", "TOTP or recovery code")}
                className="w-full sm:w-48 rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm font-mono focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none transition-colors"
                onKeyDown={(e) => e.key === "Enter" && revokeCode && !loading && handleRevoke()}
              />
              <Button variant="danger" size="sm" onClick={handleRevoke} disabled={!revokeCode || loading} isLoading={loading}>
                {t("settings.totp_confirm_revoke", "Confirm Revoke")}
              </Button>
              <Button variant="ghost" size="sm" onClick={() => { setShowRevokePrompt(false); setRevokeCode(""); }}>
                {t("common.cancel", "Cancel")}
              </Button>
            </div>
          ) : !setupData ? (
            <div className="flex gap-2">
              <Button variant="secondary" size="sm" onClick={initiateSetup} isLoading={loading}>
                {status?.confirmed
                  ? t("settings.totp_reset", "Reset TOTP")
                  : t("settings.totp_setup", "Set up TOTP")}
              </Button>
              {status?.confirmed && (
                <Button
                  variant="danger"
                  size="sm"
                  onClick={() => { setShowRevokePrompt(true); setShowResetPrompt(false); setError(null); }}
                >
                  {t("settings.totp_revoke", "Revoke TOTP")}
                </Button>
              )}
            </div>
          ) : (
            <div className="flex flex-col gap-3">
              <p className="text-sm text-text-dim">
                {t("settings.totp_scan", "Scan the QR code or enter the secret in your authenticator app:")}
              </p>
              {setupData.qr_code && (
                <div className="flex justify-center p-4 bg-white rounded-xl border border-border-subtle">
                  <img src={setupData.qr_code} alt="TOTP QR Code" className="w-40 h-40 sm:w-48 sm:h-48" />
                </div>
              )}
              <code className="block text-sm font-mono bg-main border border-border-subtle rounded-lg px-3 py-2 break-all select-all">
                {setupData.secret}
              </code>
              {setupData.recovery_codes.length > 0 && (
                <div className="mt-2">
                  <p className="text-xs font-bold text-text-dim mb-1">
                    {t("settings.totp_recovery_title", "Recovery Codes (save these somewhere safe):")}
                  </p>
                  <div className="grid grid-cols-2 gap-1 bg-main border border-border-subtle rounded-lg p-3">
                    {setupData.recovery_codes.map((code) => (
                      <code key={code} className="text-sm font-mono text-center select-all">{code}</code>
                    ))}
                  </div>
                </div>
              )}
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  inputMode="numeric"
                  maxLength={6}
                  pattern="[0-9]*"
                  value={confirmCode}
                  onChange={(e) => setConfirmCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
                  placeholder="000000"
                  className="w-28 rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm font-mono tracking-widest text-center focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none transition-colors"
                  onKeyDown={(e) => e.key === "Enter" && !loading && handleConfirm()}
                />
                <Button variant="primary" size="sm" onClick={handleConfirm} disabled={confirmCode.length !== 6 || loading} isLoading={loading}>
                  {t("settings.totp_confirm", "Confirm")}
                </Button>
                <Button variant="ghost" size="sm" onClick={() => { setSetupData(null); setConfirmCode(""); setError(null); }}>
                  {t("common.cancel", "Cancel")}
                </Button>
              </div>
            </div>
          )}

          {error && <p className="mt-2 text-sm text-danger">{error}</p>}
          {success && <p className="mt-2 text-sm text-success">{success}</p>}
        </div>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  Config Backup Section                                              */
/* ------------------------------------------------------------------ */

function ConfigBackupSection() {
  const { t } = useTranslation();

  return (
    <div className="rounded-2xl border border-border-subtle bg-surface">
      <div className="px-5 py-3 border-b border-border-subtle/50">
        <p className="text-[10px] font-black uppercase tracking-widest text-text-dim">
          {t("settings.backup", "Backup")}
        </p>
      </div>
      <div className="px-5">
        <SettingRow
          icon={Download}
          iconColor="text-blue-500"
          label={t("settings.export_config_title", "Export Config")}
          description={t(
            "settings.export_config_desc",
            "Download a backup of your current config.toml settings file"
          )}
        >
          <a
            href="/api/config/export"
            download="librefang-config.toml"
            className="inline-flex items-center justify-center gap-2 rounded-xl font-bold transition-all duration-[400ms] ease-[cubic-bezier(0.22,1,0.36,1)] active:scale-[0.96] active:duration-100 focus:outline-none focus:ring-2 focus:ring-brand/30 focus:ring-offset-1 border border-border-subtle bg-surface text-text-main hover:bg-main/50 hover:border-brand/20 shadow-sm px-3 py-1.5 text-xs"
          >
            <Download className="w-3.5 h-3.5 mr-1.5" />
            {t("settings.export_config_btn", "Download")}
          </a>
        </SettingRow>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  Auto-Dream Section                                                 */
/* ------------------------------------------------------------------ */

// Format an epoch-ms into a short human-readable "N hours ago" / "in N
// hours" label. Returns "never" when ts is 0 or undefined — the status
// endpoint omits `next_eligible_at_ms` for never-dreamed agents, and
// `last_consolidated_at_ms` is 0 in the same case.
function formatRelativeMs(ts: number | undefined, now: number): string {
  if (ts === undefined || ts === 0) return "never";
  const diff = ts - now;
  const absHours = Math.abs(diff) / 3_600_000;
  if (absHours < 1) {
    const mins = Math.round(Math.abs(diff) / 60_000);
    return diff >= 0 ? `in ${mins}m` : `${mins}m ago`;
  }
  const h = absHours >= 24 ? `${(absHours / 24).toFixed(1)}d` : `${absHours.toFixed(1)}h`;
  return diff >= 0 ? `in ${h}` : `${h} ago`;
}

// Human-readable duration for effective_min_hours. Switches between hours,
// days, and weeks so "every 168h" renders as "every 1w" etc.
function formatHours(hours: number): string {
  if (hours < 1) return `${(hours * 60).toFixed(0)}m`;
  if (hours < 24) return `${hours % 1 === 0 ? hours.toFixed(0) : hours.toFixed(1)}h`;
  const days = hours / 24;
  if (days < 7) return `${days % 1 === 0 ? days.toFixed(0) : days.toFixed(1)}d`;
  const weeks = days / 7;
  return `${weeks % 1 === 0 ? weeks.toFixed(0) : weeks.toFixed(1)}w`;
}

function AutoDreamAgentRow({
  agent,
  disabled,
  onTrigger,
  onAbort,
  onToggle,
  triggerPending,
  abortPending,
  togglePending,
}: {
  agent: AutoDreamAgentStatus;
  disabled: boolean;
  onTrigger: (id: string) => void;
  onAbort: (id: string) => void;
  onToggle: (id: string, enabled: boolean) => void;
  triggerPending: boolean;
  abortPending: boolean;
  togglePending: boolean;
}) {
  const { t } = useTranslation();
  const now = Date.now();
  const progress = agent.progress;
  const running = progress?.status === "running";
  const lastTurn = progress?.turns[progress.turns.length - 1];
  const optedIn = agent.auto_dream_enabled;

  return (
    <div className="rounded-lg border border-border-subtle/50 bg-main">
      <div className="flex items-center justify-between px-3 py-2">
        <div className="flex items-start gap-2 min-w-0 flex-1">
          <Moon
            className={`w-4 h-4 shrink-0 mt-0.5 ${
              optedIn
                ? running
                  ? "text-purple-400 animate-pulse"
                  : "text-purple-400"
                : "text-text-dim"
            }`}
          />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <p className="text-sm font-medium truncate">{agent.agent_name}</p>
              {progress && (
                <Badge
                  variant={
                    progress.status === "running"
                      ? "info"
                      : progress.status === "completed"
                      ? "success"
                      : progress.status === "aborted"
                      ? "warning"
                      : "error"
                  }
                >
                  {t(`settings.auto_dream_status_${progress.status}`, progress.status)}
                </Badge>
              )}
            </div>
            {optedIn ? (
              <p className="text-[11px] text-text-dim">
                {t("settings.auto_dream_last", "Last")}:{" "}
                {formatRelativeMs(agent.last_consolidated_at_ms, now)}
                {" · "}
                {t("settings.auto_dream_next", "Next")}:{" "}
                {formatRelativeMs(agent.next_eligible_at_ms, now)}
                {" · "}
                {agent.effective_min_sessions > 0 ? (
                  <span
                    title={t(
                      "settings.auto_dream_sessions_progress_title",
                      "Sessions touched since last dream / required threshold",
                    )}
                  >
                    {agent.sessions_since_last}/{agent.effective_min_sessions}{" "}
                    {t("settings.auto_dream_sessions_since", "sessions since")}
                  </span>
                ) : (
                  <>
                    {agent.sessions_since_last}{" "}
                    {t("settings.auto_dream_sessions_since", "sessions since")}
                  </>
                )}
                {" · "}
                <span
                  title={t(
                    "settings.auto_dream_effective_title",
                    "Resolved threshold — manifest override or global default",
                  )}
                >
                  {t("settings.auto_dream_every", "every")}{" "}
                  {formatHours(agent.effective_min_hours)}
                </span>
              </p>
            ) : running ? (
              // Agent was toggled off while a manual dream was already in
              // flight. Keep the operator informed — the run continues to
              // completion or abort, and the abort button above stays live.
              <p className="text-[11px] text-text-dim italic">
                {t(
                  "settings.auto_dream_opt_out_running",
                  "Disabled mid-dream — the current run will finish or can be aborted.",
                )}
              </p>
            ) : (
              <p className="text-[11px] text-text-dim italic">
                {t(
                  "settings.auto_dream_opt_in_hint",
                  "Not enrolled — toggle on to include in the scheduler.",
                )}
              </p>
            )}
          </div>
        </div>
        <div className="flex gap-2 shrink-0 items-center">
          <label
            className="flex items-center gap-1.5 cursor-pointer select-none"
            title={t("settings.auto_dream_toggle_title", "Opt this agent in or out")}
          >
            <input
              type="checkbox"
              checked={optedIn}
              disabled={togglePending}
              onChange={(e) => onToggle(agent.agent_id, e.target.checked)}
              className="w-3.5 h-3.5 accent-purple-500"
            />
            <span className="text-[11px] text-text-dim">
              {optedIn
                ? t("settings.auto_dream_enrolled", "Enrolled")
                : t("settings.auto_dream_not_enrolled", "Off")}
            </span>
          </label>
          {running && agent.can_abort && (
            // Surface the abort affordance even when the agent has been
            // toggled off mid-dream — otherwise the in-flight operation
            // keeps spending tokens with no UI to stop it.
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onAbort(agent.agent_id)}
              disabled={abortPending}
            >
              <Square className="w-3.5 h-3.5 mr-1.5" />
              {t("settings.auto_dream_abort", "Abort")}
            </Button>
          )}
          {optedIn && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onTrigger(agent.agent_id)}
              disabled={triggerPending || disabled || running}
              title={disabled ? t("settings.auto_dream_off", "Disabled") : undefined}
            >
              <Play className="w-3.5 h-3.5 mr-1.5" />
              {t("settings.auto_dream_trigger", "Dream now")}
            </Button>
          )}
        </div>
      </div>

      {progress && (progress.status !== "completed" || progress.memories_touched.length > 0) && (
        <div className="px-3 pb-2 pt-1 border-t border-border-subtle/30 space-y-1">
          <p className="text-[10px] text-text-dim">
            <span className="uppercase tracking-wider">
              {t("settings.auto_dream_phase", "Phase")}:
            </span>{" "}
            <span className="font-mono">{progress.phase}</span>
            {" · "}
            {progress.tool_use_count}{" "}
            {t("settings.auto_dream_tool_calls", "tool calls")}
            {progress.memories_touched.length > 0 && (
              <>
                {" · "}
                {progress.memories_touched.length}{" "}
                {t("settings.auto_dream_memories_touched", "memories touched")}
              </>
            )}
          </p>
          {lastTurn && lastTurn.text && (
            <p className="text-[11px] text-text-muted line-clamp-2 italic">
              &ldquo;{lastTurn.text}&rdquo;
            </p>
          )}
          {progress.error && (
            <p className="text-[11px] text-red-500">
              <XCircle className="w-3 h-3 inline mr-1" />
              {progress.error}
            </p>
          )}
          {/* Cache-hit visibility. Since the forkedAgent migration, dreams
              fork off the parent turn and hit Anthropic's prompt cache on
              the (system + tools + messages) prefix. Surfacing the hit
              rate here lets operators see the actual cost win — the
              whole reason the forkedAgent PR exists. Only shown for
              completed dreams (usage is populated then) and only when
              there actually was input (avoids 0/0 noise). */}
          {progress.usage && progress.usage.input_tokens > 0 && (
            <p className="text-[10px] text-text-dim">
              <span className="uppercase tracking-wider">
                {t("settings.auto_dream_cache", "Cache")}:
              </span>{" "}
              {(() => {
                const u = progress.usage!;
                const totalIn =
                  u.input_tokens +
                  u.cache_read_input_tokens +
                  u.cache_creation_input_tokens;
                const hitPct =
                  totalIn > 0
                    ? Math.round((u.cache_read_input_tokens / totalIn) * 100)
                    : 0;
                return (
                  <span
                    title={t(
                      "settings.auto_dream_cache_title",
                      "Prompt cache hit rate for this dream — higher means more of the prefix came from Anthropic's cache instead of being re-billed.",
                    )}
                  >
                    <span className="font-mono">{hitPct}%</span>
                    {" "}
                    ({u.cache_read_input_tokens.toLocaleString()}/
                    {totalIn.toLocaleString()} tok)
                  </span>
                );
              })()}
              {typeof progress.usage.cost_usd === "number" && (
                <>
                  {" · "}
                  <span
                    title={t(
                      "settings.auto_dream_cost_title",
                      "Measured provider cost for this dream turn (input + output, cached tokens billed at the reduced rate).",
                    )}
                  >
                    ${progress.usage.cost_usd.toFixed(5)}
                  </span>
                </>
              )}
            </p>
          )}
        </div>
      )}
    </div>
  );
}

function AutoDreamSection() {
  const { t } = useTranslation();
  const statusQuery = useAutoDreamStatus();
  const trigger = useTriggerAutoDream();
  const abort = useAbortAutoDream();
  const setEnabled = useSetAutoDreamEnabled();
  const [error, setError] = useState<string | null>(null);
  const [lastMsg, setLastMsg] = useState<string | null>(null);

  const status = statusQuery.data;

  const onTrigger = async (agentId: string) => {
    setError(null);
    setLastMsg(null);
    try {
      const outcome = await trigger.mutateAsync(agentId);
      setLastMsg(outcome.fired ? t("settings.auto_dream_fired", "Consolidation fired") : outcome.reason);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const onAbort = async (agentId: string) => {
    setError(null);
    setLastMsg(null);
    try {
      const outcome = await abort.mutateAsync(agentId);
      setLastMsg(outcome.aborted ? t("settings.auto_dream_aborted", "Abort signalled") : outcome.reason);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const onToggle = async (agentId: string, enabled: boolean) => {
    setError(null);
    setLastMsg(null);
    try {
      await setEnabled.mutateAsync({ agentId, enabled });
      setLastMsg(
        enabled
          ? t("settings.auto_dream_enrolled_ok", "Agent enrolled")
          : t("settings.auto_dream_unenrolled_ok", "Agent unenrolled"),
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="rounded-2xl border border-border-subtle bg-surface">
      <div className="px-5 py-3 border-b border-border-subtle/50 flex items-center justify-between">
        <p className="text-[10px] font-black uppercase tracking-widest text-text-dim">
          {t("settings.auto_dream", "Auto-Dream")}
        </p>
        {status && (
          <Badge variant={status.enabled ? "success" : "default"}>
            {status.enabled
              ? t("settings.auto_dream_on", "Enabled")
              : t("settings.auto_dream_off", "Disabled")}
          </Badge>
        )}
      </div>
      <div className="px-5 py-3">
        <p className="text-xs text-text-dim mb-3">
          {t(
            "settings.auto_dream_desc",
            "Periodically asks opt-in agents to consolidate their memory. Configure in config.toml via [auto_dream] enabled + per-agent auto_dream_enabled.",
          )}
        </p>

        {statusQuery.isLoading && (
          <p className="text-xs text-text-dim">{t("common.loading", "Loading…")}</p>
        )}
        {statusQuery.isError && (
          <p className="text-xs text-red-500">
            {t("settings.auto_dream_load_err", "Failed to load auto-dream status")}
          </p>
        )}

        {status && status.agents.length === 0 && (
          <p className="text-xs text-text-dim italic">
            {t(
              "settings.auto_dream_no_agents_registered",
              "No agents registered yet. Create an agent first, then toggle it on here.",
            )}
          </p>
        )}

        {status && status.agents.length > 0 && (
          <div className="space-y-2">
            {status.agents.map((a) => (
              <AutoDreamAgentRow
                key={a.agent_id}
                agent={a}
                disabled={!status.enabled}
                onTrigger={onTrigger}
                onAbort={onAbort}
                onToggle={onToggle}
                triggerPending={trigger.isPending}
                abortPending={abort.isPending}
                togglePending={setEnabled.isPending}
              />
            ))}
          </div>
        )}

        {lastMsg && (
          <p className="text-xs text-green-500 mt-2">
            <CheckCircle className="w-3 h-3 inline mr-1" />
            {lastMsg}
          </p>
        )}
        {error && (
          <p className="text-xs text-red-500 mt-2">
            <XCircle className="w-3 h-3 inline mr-1" />
            {error}
          </p>
        )}
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  Storage Administration Section                                     */
/* ------------------------------------------------------------------ */

function StorageSection() {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);

  const configQuery = useStorageConfig();
  const statusQuery = useStorageStatus();
  const migrate = useMigrateStorage();
  const linkUarMutation = useLinkUarStorage();
  const unlinkUar = useUnlinkUarStorage();

  const [dryRunResult, setDryRunResult] = useState<Record<string, number> | null>(null);

  // Link UAR form state (remote only)
  const [showLinkForm, setShowLinkForm] = useState(false);
  const [lkRemoteUrl, setLkRemoteUrl] = useState("");
  const [lkRootUser, setLkRootUser] = useState("root");
  const [lkRootPassEnv, setLkRootPassEnv] = useState("SURREAL_ROOT_PASS");
  const [lkNamespace, setLkNamespace] = useState("uar");
  const [lkAppUser, setLkAppUser] = useState("uar_app");
  const [lkAppPassEnv, setLkAppPassEnv] = useState("SURREAL_UAR_APP_PASS");
  const [lkAlsoMemory, setLkAlsoMemory] = useState(true);

  const cfg = configQuery.data;
  const status = statusQuery.data;

  const handleDryRun = () => {
    setDryRunResult(null);
    migrate.mutate(
      { from: "sqlite", dry_run: true },
      {
        onSuccess: (res) => setDryRunResult(res.copied),
        onError: (err) => addToast(err instanceof Error ? err.message : t("common.error"), "error"),
      },
    );
  };

  const handleMigrate = () => {
    migrate.mutate(
      { from: "sqlite", dry_run: false },
      {
        onSuccess: () =>
          addToast(t("settings.storage_migrate_success", { defaultValue: "Migration complete" }), "success"),
        onError: (err) => addToast(err instanceof Error ? err.message : t("common.error"), "error"),
      },
    );
  };

  const handleUnlinkUar = () => {
    unlinkUar.mutate(undefined, {
      onSuccess: () =>
        addToast(t("settings.storage_unlink_success", { defaultValue: "UAR unlinked" }), "success"),
      onError: (err) => addToast(err instanceof Error ? err.message : t("common.error"), "error"),
    });
  };

  const handleLinkUar = () => {
    const remoteUrl = lkRemoteUrl.trim() || cfg?.remote_url || "";
    if (!remoteUrl) {
      addToast(t("settings.storage_link_uar_no_url", { defaultValue: "Provide the remote SurrealDB URL" }), "error");
      return;
    }
    linkUarMutation.mutate(
      {
        remote_url: remoteUrl,
        root_user: lkRootUser.trim(),
        root_pass_ref: lkRootPassEnv.trim(),
        namespace: lkNamespace.trim() || "uar",
        app_user: lkAppUser.trim() || "uar_app",
        app_pass_ref: lkAppPassEnv.trim(),
        also_link_memory: lkAlsoMemory,
      },
      {
        onSuccess: () => {
          setShowLinkForm(false);
          addToast(t("settings.storage_link_success", { defaultValue: "UAR linked to SurrealDB" }), "success");
        },
        onError: (err) =>
          addToast(err instanceof Error ? err.message : t("common.error"), "error"),
      },
    );
  };

  return (
    <div className="rounded-2xl border border-border-subtle bg-surface">
      {/* Header */}
      <div className="px-5 py-3 border-b border-border-subtle/50 flex items-center gap-2">
        <Database className="h-3.5 w-3.5 text-text-dim" />
        <p className="text-[10px] font-black uppercase tracking-widest text-text-dim">
          {t("settings.storage", "Storage")}
        </p>
      </div>

      <div className="px-5 py-4 space-y-5">
        {/* Status row */}
        <div className="flex flex-wrap items-center gap-x-6 gap-y-2 text-xs">
          <div className="flex items-center gap-1.5">
            <span className="text-text-dim">{t("settings.storage_backend", { defaultValue: "Backend" })}:</span>
            <Badge variant="brand">{cfg?.backend_kind ?? "…"}</Badge>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="text-text-dim">{t("settings.storage_location", { defaultValue: "Location" })}:</span>
            <span className="font-mono text-text-main truncate max-w-48">{status?.backend_location ?? cfg?.embedded_path ?? cfg?.remote_url ?? "…"}</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="text-text-dim">{t("settings.storage_connection", { defaultValue: "Connected" })}:</span>
            {statusQuery.isLoading ? (
              <Loader2 className="h-3 w-3 animate-spin text-brand" />
            ) : (
              <Badge variant={status?.connected ? "success" : "error"}>
                {status?.connected ? t("common.yes", "Yes") : t("common.no", "No")}
              </Badge>
            )}
          </div>
          {status?.uar_linked && (
            <div className="flex items-center gap-1.5">
              <span className="text-text-dim">{t("settings.storage_uar", { defaultValue: "UAR" })}:</span>
              <Badge variant="success">{t("settings.storage_uar_linked", { defaultValue: "linked" })}</Badge>
            </div>
          )}
        </div>

        {/* Table counts */}
        {status?.table_counts && (
          <div className="grid grid-cols-2 sm:grid-cols-5 gap-2">
            {Object.entries(status.table_counts).map(([table, count]) => (
              <div key={table} className="rounded-lg bg-main/40 border border-border-subtle/60 px-3 py-2 text-center">
                <p className="text-lg font-black text-text-main">{count}</p>
                <p className="text-[10px] text-text-dim truncate">{table.replace(/_/g, " ")}</p>
              </div>
            ))}
          </div>
        )}

        {/* Namespace / database */}
        {cfg && (
          <div className="flex flex-wrap gap-3">
            <div className="flex-1 min-w-32">
              <label className="block text-[10px] font-bold text-text-dim uppercase tracking-wider mb-1">
                {t("settings.storage_namespace", { defaultValue: "Namespace" })}
              </label>
              <Input
                value={cfg.namespace}
                readOnly
                className="text-xs font-mono"
                onChange={() => {}}
              />
            </div>
            <div className="flex-1 min-w-32">
              <label className="block text-[10px] font-bold text-text-dim uppercase tracking-wider mb-1">
                {t("settings.storage_database", { defaultValue: "Database" })}
              </label>
              <Input
                value={cfg.database}
                readOnly
                className="text-xs font-mono"
                onChange={() => {}}
              />
            </div>
          </div>
        )}

        {/* Migrate actions (only shown when migration is available) */}
        {status?.migration_available && (
          <div className="rounded-xl border border-warning/30 bg-warning/5 p-4 space-y-3">
            <div className="flex items-start gap-2">
              <ArrowRight className="h-4 w-4 text-warning shrink-0 mt-0.5" />
              <div>
                <p className="text-sm font-bold text-text-main">
                  {t("settings.storage_migrate_title", { defaultValue: "SQLite → SurrealDB Migration Available" })}
                </p>
                <p className="text-xs text-text-dim mt-0.5">
                  {t("settings.storage_migrate_desc", { defaultValue: "Run a dry-run first to preview row counts, then perform the live migration." })}
                </p>
              </div>
            </div>

            {dryRunResult && (
              <div className="rounded-lg bg-surface border border-border-subtle text-xs p-3 font-mono space-y-1">
                {Object.entries(dryRunResult).map(([table, count]) => (
                  <div key={table} className="flex justify-between">
                    <span className="text-text-dim">{table}</span>
                    <span className="font-bold text-text-main">{count} rows</span>
                  </div>
                ))}
              </div>
            )}

            <div className="flex gap-2">
              <Button
                variant="secondary"
                size="sm"
                isLoading={migrate.isPending && !dryRunResult}
                onClick={handleDryRun}
              >
                {t("settings.storage_dry_run", { defaultValue: "Dry Run" })}
              </Button>
              <Button
                variant="primary"
                size="sm"
                isLoading={migrate.isPending && !!dryRunResult}
                onClick={handleMigrate}
              >
                {t("settings.storage_migrate_btn", { defaultValue: "Migrate Now" })}
              </Button>
            </div>
          </div>
        )}

        {/* UAR link / unlink actions */}
        {status?.uar_linked ? (
          <div className="flex items-center gap-3 rounded-xl border border-success/30 bg-success/5 px-4 py-3">
            <CheckCircle className="h-4 w-4 text-success shrink-0" />
            <div className="flex-1 min-w-0">
              <p className="text-sm font-bold text-text-main">
                {t("settings.storage_uar_linked_title", { defaultValue: "UAR is sharing this SurrealDB instance" })}
              </p>
              {status.uar_namespace && (
                <p className="text-xs text-text-dim font-mono mt-0.5">{status.uar_namespace}/main</p>
              )}
            </div>
            <Button
              variant="secondary"
              size="sm"
              leftIcon={<Unlink className="h-3.5 w-3.5" />}
              isLoading={unlinkUar.isPending}
              onClick={handleUnlinkUar}
            >
              {t("settings.storage_unlink_uar", { defaultValue: "Unlink" })}
            </Button>
          </div>
        ) : (
          <div className="rounded-xl border border-border-subtle bg-surface">
            <button
              type="button"
              className="w-full flex items-center justify-between px-4 py-3 text-left"
              onClick={() => setShowLinkForm((v) => !v)}
            >
              <div className="flex items-center gap-2">
                <Link className="h-4 w-4 text-text-dim" />
                <span className="text-sm font-bold text-text-main">
                  {t("settings.storage_link_uar", { defaultValue: "Link UAR to this SurrealDB" })}
                </span>
              </div>
              <ArrowRight className={`h-4 w-4 text-text-dim transition-transform ${showLinkForm ? "rotate-90" : ""}`} />
            </button>

            {showLinkForm && (
              <div className="px-4 pb-4 space-y-3 border-t border-border-subtle/50 pt-4">
                <p className="text-xs text-text-dim">
                  {t("settings.storage_link_uar_form_desc", {
                    defaultValue:
                      "Provisions a `uar` namespace + least-privilege app user on the remote instance. Root credentials are never persisted.",
                  })}
                </p>

                {cfg?.backend_kind === "embedded" && (
                  <div className="rounded-lg bg-warning/5 border border-warning/30 px-3 py-2 text-xs text-warning">
                    {t("settings.storage_embedded_uar_warning", {
                      defaultValue:
                        "The embedded engine is single-writer. UAR co-location requires a remote SurrealDB instance.",
                    })}
                  </div>
                )}

                <div className="grid grid-cols-1 gap-2">
                  <Input
                    label={t("settings.storage_remote_url", { defaultValue: "Remote SurrealDB URL" })}
                    placeholder={cfg?.remote_url || "ws://surreal:8000"}
                    value={lkRemoteUrl}
                    onChange={(e) => setLkRemoteUrl(e.target.value)}
                    disabled={cfg?.backend_kind === "embedded"}
                  />
                </div>
                <div className="grid grid-cols-2 gap-2">
                  <Input
                    label={t("settings.storage_uar_ns", { defaultValue: "UAR Namespace" })}
                    placeholder="uar"
                    value={lkNamespace}
                    onChange={(e) => setLkNamespace(e.target.value)}
                  />
                  <Input
                    label={t("settings.storage_uar_app_user", { defaultValue: "App User" })}
                    placeholder="uar_app"
                    value={lkAppUser}
                    onChange={(e) => setLkAppUser(e.target.value)}
                  />
                </div>
                <div className="grid grid-cols-2 gap-2">
                  <Input
                    label={t("settings.storage_root_user", { defaultValue: "Root Username" })}
                    placeholder="root"
                    value={lkRootUser}
                    onChange={(e) => setLkRootUser(e.target.value)}
                  />
                  <Input
                    label={t("settings.storage_root_pass_env", { defaultValue: "Root Pass Env Var" })}
                    placeholder="SURREAL_ROOT_PASS"
                    value={lkRootPassEnv}
                    onChange={(e) => setLkRootPassEnv(e.target.value)}
                  />
                </div>
                <div className="grid grid-cols-2 gap-2">
                  <Input
                    label={t("settings.storage_app_pass_env", { defaultValue: "App Pass Env Var" })}
                    placeholder="SURREAL_UAR_APP_PASS"
                    value={lkAppPassEnv}
                    onChange={(e) => setLkAppPassEnv(e.target.value)}
                  />
                </div>
                <label className="flex items-start gap-2 cursor-pointer">
                  <input
                    type="checkbox"
                    className="mt-0.5 h-4 w-4 rounded border-border-subtle accent-brand"
                    checked={lkAlsoMemory}
                    onChange={(e) => setLkAlsoMemory(e.target.checked)}
                  />
                  <div>
                    <p className="text-xs font-bold text-text-main">
                      {t("settings.storage_also_link_memory", { defaultValue: "Also share storage for UAR memory (share_librefang_storage)" })}
                    </p>
                    <p className="text-[10px] text-text-dim">
                      {t("settings.storage_also_link_memory_desc", {
                        defaultValue: "Sets share_librefang_storage = true in config.toml so UAR reuses the same remote config.",
                      })}
                    </p>
                  </div>
                </label>
                <Button
                  variant="primary"
                  size="sm"
                  leftIcon={<Link className="h-3.5 w-3.5" />}
                  isLoading={linkUarMutation.isPending}
                  disabled={cfg?.backend_kind === "embedded"}
                  onClick={handleLinkUar}
                >
                  {t("settings.storage_provision_link", { defaultValue: "Provision & Link" })}
                </Button>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
