import { useTranslation } from "react-i18next";
import { useState, useMemo, useCallback } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { PageHeader } from "../components/ui/PageHeader";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import {
  Globe, Sun, Moon, Settings, PanelLeftClose, PanelLeft, Languages, LayoutDashboard,
  Shield, CheckCircle, XCircle, Download, ChevronDown, RefreshCw, Save, Zap,
} from "lucide-react";
import { useUIStore } from "../lib/store";
import {
  totpSetup, totpConfirm, totpStatus, totpRevoke,
  getConfigSchema, getFullConfig, setConfigValue, reloadConfig,
  type ConfigSectionSchema, type ConfigFieldSchema,
} from "../api";

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

      {/* System Configuration */}
      <SystemConfigSection />

      {/* TOTP Second Factor */}
      <TotpSection />

      {/* Config Backup */}
      <ConfigBackupSection />
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  System Configuration Section                                       */
/* ------------------------------------------------------------------ */

const SECTION_ORDER = [
  "general", "default_model", "budget", "memory", "proactive_memory",
  "web", "browser", "media", "links", "tts",
  "exec_policy", "approval", "auto_reply", "thinking",
  "network", "a2a", "extensions", "vault", "docker",
  "channels", "broadcast", "canvas",
  "session", "queue", "reload", "webhook_triggers",
  "pairing", "vertex_ai", "oauth", "external_auth",
];

function sectionLabel(key: string): string {
  return key
    .split("_")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

function resolveFieldType(
  schema: string | ConfigFieldSchema
): { type: string; options?: (string | { id: string; name: string; provider: string })[] } {
  if (typeof schema === "string") return { type: schema };
  return { type: schema.type || "string", options: schema.options };
}

function getNestedValue(obj: Record<string, unknown>, section: string, field: string, rootLevel?: boolean): unknown {
  if (rootLevel) return obj[field];
  const sec = obj[section] as Record<string, unknown> | undefined;
  return sec?.[field];
}

function ConfigFieldInput({
  fieldType,
  options,
  value,
  onChange,
}: {
  fieldType: string;
  options?: (string | { id: string; name: string; provider: string })[];
  value: unknown;
  onChange: (v: unknown) => void;
}) {
  const inputClass =
    "w-full px-3 py-1.5 rounded-xl border border-border-subtle bg-main text-xs font-mono outline-none focus:border-brand transition-colors";

  if (fieldType === "boolean") {
    return (
      <button
        onClick={() => onChange(!value)}
        className={`relative w-10 h-5 rounded-full transition-colors ${
          value ? "bg-brand" : "bg-border-subtle"
        }`}
      >
        <span
          className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${
            value ? "left-5" : "left-0.5"
          }`}
        />
      </button>
    );
  }

  if (fieldType === "select" && options) {
    const strOptions = options.map((o) =>
      typeof o === "string" ? o : o.id
    );
    return (
      <select
        value={String(value ?? "")}
        onChange={(e) => onChange(e.target.value)}
        className={inputClass}
      >
        <option value="">—</option>
        {strOptions.map((o) => (
          <option key={o} value={o}>
            {o}
          </option>
        ))}
      </select>
    );
  }

  if (fieldType === "number") {
    return (
      <input
        type="number"
        value={value != null ? String(value) : ""}
        onChange={(e) => {
          const v = e.target.value;
          onChange(v === "" ? null : Number(v));
        }}
        className={inputClass}
      />
    );
  }

  if (fieldType === "string[]" || fieldType === "array") {
    const arr = Array.isArray(value) ? value : [];
    return (
      <input
        type="text"
        value={arr.join(", ")}
        onChange={(e) =>
          onChange(
            e.target.value
              .split(",")
              .map((s) => s.trim())
              .filter(Boolean)
          )
        }
        placeholder="comma-separated values"
        className={inputClass}
      />
    );
  }

  if (fieldType === "object") {
    return (
      <pre className="text-[10px] text-text-dim font-mono bg-main rounded-lg px-3 py-2 max-h-24 overflow-auto border border-border-subtle">
        {value != null ? JSON.stringify(value, null, 2) : "—"}
      </pre>
    );
  }

  // Default: string input
  return (
    <input
      type="text"
      value={String(value ?? "")}
      onChange={(e) => onChange(e.target.value || null)}
      className={inputClass}
    />
  );
}

function SystemConfigSection() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [expandedSections, setExpandedSections] = useState<Set<string>>(new Set());
  const [pendingChanges, setPendingChanges] = useState<Record<string, unknown>>({});
  const [saveStatus, setSaveStatus] = useState<{ path: string; ok: boolean; msg: string } | null>(null);

  const schemaQuery = useQuery({
    queryKey: ["config", "schema"],
    queryFn: getConfigSchema,
    staleTime: 300_000,
  });

  const configQuery = useQuery({
    queryKey: ["config", "full"],
    queryFn: getFullConfig,
    staleTime: 30_000,
  });

  const saveMutation = useMutation({
    mutationFn: ({ path, value }: { path: string; value: unknown }) =>
      setConfigValue(path, value),
    onSuccess: (_data, variables) => {
      setSaveStatus({ path: variables.path, ok: true, msg: "Saved" });
      setPendingChanges((p) => {
        const next = { ...p };
        delete next[variables.path];
        return next;
      });
      queryClient.invalidateQueries({ queryKey: ["config", "full"] });
      setTimeout(() => setSaveStatus(null), 2000);
    },
    onError: (err: Error, variables) => {
      setSaveStatus({ path: variables.path, ok: false, msg: err.message });
      setTimeout(() => setSaveStatus(null), 3000);
    },
  });

  const reloadMutation = useMutation({
    mutationFn: reloadConfig,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["config", "full"] });
    },
  });

  const sections = schemaQuery.data?.sections ?? {};
  const config = configQuery.data ?? {};

  const sortedSections = useMemo(() => {
    const keys = Object.keys(sections);
    return keys.sort((a, b) => {
      const ai = SECTION_ORDER.indexOf(a);
      const bi = SECTION_ORDER.indexOf(b);
      return (ai === -1 ? 999 : ai) - (bi === -1 ? 999 : bi);
    });
  }, [sections]);

  const toggleSection = useCallback((key: string) => {
    setExpandedSections((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const handleFieldChange = useCallback(
    (sectionKey: string, fieldKey: string, value: unknown, rootLevel?: boolean) => {
      const path = rootLevel ? fieldKey : `${sectionKey}.${fieldKey}`;
      setPendingChanges((p) => ({ ...p, [path]: value }));
    },
    []
  );

  const handleSave = useCallback(
    (path: string) => {
      if (path in pendingChanges) {
        saveMutation.mutate({ path, value: pendingChanges[path] });
      }
    },
    [pendingChanges, saveMutation]
  );

  if (schemaQuery.isLoading || configQuery.isLoading) {
    return (
      <div className="rounded-2xl border border-border-subtle bg-surface p-8 text-center text-text-dim text-sm">
        {t("common.loading", "Loading configuration...")}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <div>
          <p className="text-[10px] font-black uppercase tracking-widest text-text-dim">
            {t("settings.config_system", "System Configuration")}
          </p>
          <p className="text-xs text-text-dim mt-0.5">
            {t("settings.config_system_desc", "View and edit config.toml settings. Changes are saved immediately.")}
          </p>
        </div>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => reloadMutation.mutate()}
          isLoading={reloadMutation.isPending}
        >
          <RefreshCw className="w-3 h-3 mr-1.5" />
          {t("settings.reload_config", "Reload")}
        </Button>
      </div>

      {sortedSections.map((sectionKey) => {
        const sec = sections[sectionKey];
        const isExpanded = expandedSections.has(sectionKey);
        const fields = Object.entries(sec.fields);

        return (
          <div
            key={sectionKey}
            className="rounded-2xl border border-border-subtle bg-surface overflow-hidden"
          >
            <button
              onClick={() => toggleSection(sectionKey)}
              className="w-full flex items-center justify-between px-5 py-3 hover:bg-surface-hover transition-colors"
            >
              <div className="flex items-center gap-2">
                <p className="text-sm font-semibold">{sectionLabel(sectionKey)}</p>
                {sec.hot_reloadable && (
                  <Badge variant="success">
                    <Zap className="w-2.5 h-2.5 mr-0.5" />
                    {t("settings.hot_reload", "Hot")}
                  </Badge>
                )}
                {sec.root_level && (
                  <Badge variant="info">
                    {t("settings.root_level", "Root")}
                  </Badge>
                )}
              </div>
              <div className="flex items-center gap-2">
                <span className="text-[10px] text-text-dim">
                  {fields.length} {fields.length === 1 ? "field" : "fields"}
                </span>
                <ChevronDown
                  className={`w-4 h-4 text-text-dim transition-transform ${
                    isExpanded ? "" : "-rotate-90"
                  }`}
                />
              </div>
            </button>

            {isExpanded && (
              <div className="px-5 pb-4 border-t border-border-subtle/50">
                {fields.map(([fieldKey, fieldSchema]) => {
                  const { type: fieldType, options } = resolveFieldType(fieldSchema);
                  const path = sec.root_level ? fieldKey : `${sectionKey}.${fieldKey}`;
                  const currentValue =
                    path in pendingChanges
                      ? pendingChanges[path]
                      : getNestedValue(config, sectionKey, fieldKey, sec.root_level);
                  const hasPending = path in pendingChanges;
                  const isSaving = saveMutation.isPending && saveMutation.variables?.path === path;
                  const statusForField = saveStatus?.path === path ? saveStatus : null;

                  return (
                    <div
                      key={fieldKey}
                      className="flex items-start gap-4 py-3 border-b border-border-subtle/30 last:border-0"
                    >
                      <div className="flex-1 min-w-0">
                        <p className="text-xs font-semibold font-mono">{fieldKey}</p>
                        <p className="text-[10px] text-text-dim mt-0.5">{fieldType}</p>
                      </div>
                      <div className="w-64 shrink-0">
                        <ConfigFieldInput
                          fieldType={fieldType}
                          options={options}
                          value={currentValue}
                          onChange={(v) =>
                            handleFieldChange(sectionKey, fieldKey, v, sec.root_level)
                          }
                        />
                      </div>
                      <div className="w-16 shrink-0 flex items-center justify-end">
                        {fieldType !== "object" && hasPending && (
                          <Button
                            variant="primary"
                            size="sm"
                            onClick={() => handleSave(path)}
                            isLoading={isSaving}
                            disabled={isSaving}
                          >
                            <Save className="w-3 h-3" />
                          </Button>
                        )}
                        {statusForField && (
                          <span
                            className={`text-[10px] font-semibold ${
                              statusForField.ok ? "text-success" : "text-danger"
                            }`}
                          >
                            {statusForField.msg}
                          </span>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        );
      })}
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
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  const statusQuery = useQuery({
    queryKey: ["totp", "status"],
    queryFn: totpStatus,
    staleTime: 30_000,
  });

  const status = statusQuery.data;

  async function handleSetup(currentCode?: string) {
    setLoading(true);
    setError(null);
    try {
      const data = await totpSetup(currentCode);
      setSetupData({ otpauth_uri: data.otpauth_uri, secret: data.secret, qr_code: data.qr_code, recovery_codes: data.recovery_codes });
      setShowResetPrompt(false);
      setResetCode("");
    } catch (e: any) {
      setError(e.message || "Setup failed");
    } finally {
      setLoading(false);
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
    if (!revokeCode) return;
    setLoading(true);
    setError(null);
    try {
      await totpRevoke(revokeCode);
      setSuccess("TOTP revoked. Set second_factor = \"none\" in config.");
      setShowRevokePrompt(false);
      setRevokeCode("");
      statusQuery.refetch();
    } catch (e: any) {
      setError(e.message || "Revoke failed");
    } finally {
      setLoading(false);
    }
  }

  async function handleConfirm() {
    if (confirmCode.length !== 6) return;
    setLoading(true);
    setError(null);
    try {
      await totpConfirm(confirmCode);
      setSuccess("TOTP confirmed. Set second_factor = \"totp\" in config to enforce.");
      setSetupData(null);
      setConfirmCode("");
      statusQuery.refetch();
    } catch (e: any) {
      setError(e.message || "Invalid code");
    } finally {
      setLoading(false);
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

        {/* Recovery codes warning */}
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

        {/* Setup flow */}
        <div className="py-4">
          {showResetPrompt && !setupData ? (
            <div className="flex flex-col sm:flex-row sm:items-center gap-2">
              <input
                type="text"
                value={resetCode}
                onChange={(e) => setResetCode(e.target.value)}
                placeholder={t("settings.totp_reset_placeholder", "Current TOTP or recovery code")}
                className="w-full sm:w-48 rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm font-mono focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none transition-colors"
                onKeyDown={(e) => e.key === "Enter" && resetCode && handleSetup(resetCode)}
              />
              <Button
                variant="primary"
                size="sm"
                onClick={() => handleSetup(resetCode)}
                disabled={!resetCode || loading}
                isLoading={loading}
              >
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
                onKeyDown={(e) => e.key === "Enter" && revokeCode && handleRevoke()}
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
              <Button
                variant="secondary"
                size="sm"
                onClick={initiateSetup}
                isLoading={loading}
              >
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
                    {setupData.recovery_codes.map((code, i) => (
                      <code key={i} className="text-sm font-mono text-center select-all">{code}</code>
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
                  onKeyDown={(e) => e.key === "Enter" && handleConfirm()}
                />
                <Button
                  variant="primary"
                  size="sm"
                  onClick={handleConfirm}
                  disabled={confirmCode.length !== 6 || loading}
                  isLoading={loading}
                >
                  {t("settings.totp_confirm", "Confirm")}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => { setSetupData(null); setConfirmCode(""); setError(null); }}
                >
                  {t("common.cancel", "Cancel")}
                </Button>
              </div>
            </div>
          )}

          {error && (
            <p className="mt-2 text-sm text-danger">{error}</p>
          )}
          {success && (
            <p className="mt-2 text-sm text-success">{success}</p>
          )}
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
          <a href="/api/config/export" download="librefang-config.toml">
            <Button variant="secondary" size="sm">
              <Download className="w-3.5 h-3.5 mr-1.5" />
              {t("settings.export_config_btn", "Download")}
            </Button>
          </a>
        </SettingRow>
      </div>
    </div>
  );
}
