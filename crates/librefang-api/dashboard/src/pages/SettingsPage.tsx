import { useTranslation } from "react-i18next";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { PageHeader } from "../components/ui/PageHeader";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import {
  Globe, Sun, Moon, Settings, PanelLeftClose, PanelLeft, Languages, LayoutDashboard,
  Shield, CheckCircle, XCircle, Download,
} from "lucide-react";
import { useUIStore } from "../lib/store";
import { totpSetup, totpConfirm, totpStatus, totpRevoke } from "../api";

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

      <TotpSection />

      <ConfigBackupSection />
    </div>
  );
}

function TotpSection() {
  const { t } = useTranslation();
  const [setupData, setSetupData] = useState<{ secret: string; qr_code: string | null; recovery_codes: string[] } | null>(null);
  const [confirmCode, setConfirmCode] = useState("");
  const [revokeCode, setRevokeCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const statusQuery = useQuery({
    queryKey: ["totp", "status"],
    queryFn: totpStatus,
    staleTime: 30_000,
  });

  async function handleSetup() {
    setLoading(true);
    setError(null);
    try {
      const data = await totpSetup();
      setSetupData({ secret: data.secret, qr_code: data.qr_code, recovery_codes: data.recovery_codes });
    } catch (e: any) {
      setError(e.message || "Setup failed");
    } finally {
      setLoading(false);
    }
  }

  async function handleConfirm() {
    setLoading(true);
    setError(null);
    try {
      await totpConfirm(confirmCode);
      setSuccess("TOTP confirmed.");
      setSetupData(null);
      setConfirmCode("");
      statusQuery.refetch();
    } catch (e: any) {
      setError(e.message || "Confirm failed");
    } finally {
      setLoading(false);
    }
  }

  async function handleRevoke() {
    setLoading(true);
    setError(null);
    try {
      await totpRevoke(revokeCode);
      setSuccess("TOTP revoked.");
      setRevokeCode("");
      statusQuery.refetch();
    } catch (e: any) {
      setError(e.message || "Revoke failed");
    } finally {
      setLoading(false);
    }
  }

  const status = statusQuery.data;

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
              <Badge variant="success"><CheckCircle className="w-3 h-3 mr-1" />{t("settings.totp_enrolled", "Enrolled")}</Badge>
            ) : (
              <Badge variant="default"><XCircle className="w-3 h-3 mr-1" />{t("settings.totp_not_enrolled", "Not enrolled")}</Badge>
            )}
            {status?.enforced && <Badge variant="info">{t("settings.totp_enforced", "Enforced")}</Badge>}
          </div>
        </SettingRow>
        <div className="py-4 flex flex-col gap-3">
          {!setupData ? (
            <div className="flex gap-2 flex-wrap">
              <Button variant="secondary" size="sm" onClick={handleSetup} isLoading={loading}>
                {t("settings.totp_setup", "Set up TOTP")}
              </Button>
              <input
                type="text"
                value={revokeCode}
                onChange={(e) => setRevokeCode(e.target.value)}
                placeholder={t("settings.totp_revoke_placeholder", "TOTP or recovery code")}
                className="w-48 rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm font-mono"
              />
              <Button variant="danger" size="sm" onClick={handleRevoke} disabled={!revokeCode || loading}>
                {t("settings.totp_revoke", "Revoke TOTP")}
              </Button>
            </div>
          ) : (
            <div className="flex flex-col gap-3">
              {setupData.qr_code && <img src={setupData.qr_code} alt="TOTP QR Code" className="w-48 h-48 rounded-xl border border-border-subtle bg-white p-3" />}
              <code className="block text-sm font-mono bg-main border border-border-subtle rounded-lg px-3 py-2 break-all select-all">{setupData.secret}</code>
              <div className="grid grid-cols-2 gap-1 bg-main border border-border-subtle rounded-lg p-3">
                {setupData.recovery_codes.map((code, i) => <code key={i} className="text-sm font-mono text-center select-all">{code}</code>)}
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  inputMode="numeric"
                  maxLength={6}
                  value={confirmCode}
                  onChange={(e) => setConfirmCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
                  placeholder="000000"
                  className="w-28 rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm font-mono tracking-widest text-center"
                />
                <Button variant="primary" size="sm" onClick={handleConfirm} disabled={confirmCode.length !== 6 || loading}>
                  {t("settings.totp_confirm", "Confirm")}
                </Button>
              </div>
            </div>
          )}
          {error && <p className="text-sm text-danger">{error}</p>}
          {success && <p className="text-sm text-success">{success}</p>}
        </div>
      </div>
    </div>
  );
}

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
          description={t("settings.export_config_desc", "Download a backup of your current config.toml settings file")}
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
