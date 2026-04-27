import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { wechatQrStart, wechatQrStatus, whatsappQrStart, whatsappQrStatus, type ChannelItem } from "../api";
import { useChannels } from "../lib/queries/channels";
import { useConfigureChannel, useTestChannel, useReloadChannels } from "../lib/mutations/channels";
import { useUIStore } from "../lib/store";
import { copyToClipboard } from "../lib/clipboard";
import QRCode from "qrcode";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { Modal } from "../components/ui/Modal";
import {
  Network, Search, CheckCircle2, XCircle, ChevronRight, X, Grid3X3, List,
  Settings, Key, Clock, AlertCircle, CheckSquare, Square,
  MessageCircle, Mail, Phone, Link2, Radio, Send, Bell, Wifi, Globe
} from "lucide-react";

const channelIcons: Record<string, React.ReactNode> = {
  slack: <MessageCircle className="w-5 h-5" />,
  discord: <MessageCircle className="w-5 h-5" />,
  telegram: <Send className="w-5 h-5" />,
  whatsapp: <Phone className="w-5 h-5" />,
  email: <Mail className="w-5 h-5" />,
  sms: <MessageCircle className="w-5 h-5" />,
  webhook: <Link2 className="w-5 h-5" />,
  http: <Globe className="w-5 h-5" />,
  websocket: <Radio className="w-5 h-5" />,
  mqtt: <Wifi className="w-5 h-5" />,
  slack_events: <Bell className="w-5 h-5" />,
  teams: <MessageCircle className="w-5 h-5" />,
};

function getChannelIcon(name: string): React.ReactNode {
  const key = name.toLowerCase().split("-")[0];
  return channelIcons[key] || <Radio className="w-5 h-5" />;
}

type SortField = "name" | "category";
type SortOrder = "asc" | "desc";
type ViewMode = "grid" | "list";

type Channel = ChannelItem;

interface ChannelCardProps {
  channel: Channel;
  isSelected: boolean;
  viewMode: ViewMode;
  onSelect: (name: string, checked: boolean) => void;
  onConfigure: (channel: Channel) => void;
  onViewDetails: (channel: Channel) => void;
  t: (key: string) => string;
}

function ChannelCard({ channel: c, isSelected, viewMode, onSelect, onConfigure, onViewDetails, t }: ChannelCardProps) {
  if (viewMode === "list") {
    return (
      <Card hover padding="sm" className={`flex items-center gap-4 group transition-all ${isSelected ? "ring-2 ring-primary" : ""}`}>
        <button
          onClick={(e) => { e.stopPropagation(); onSelect(c.name, !isSelected); }}
          className="shrink-0 text-text-dim hover:text-primary transition-colors"
        >
          {isSelected ? <CheckSquare className="w-5 h-5 text-primary" /> : <Square className="w-5 h-5" />}
        </button>

        <div className={`w-8 h-8 rounded-lg flex items-center justify-center text-lg shrink-0 ${c.configured ? "bg-success/10 border border-success/20" : "bg-primary/10 border border-primary/20"}`}>
          {getChannelIcon(c.name)}
        </div>

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h3 className="font-black truncate">{c.display_name || c.name}</h3>
            <Badge variant={c.configured ? "success" : "warning"} className="shrink-0">
              {c.configured ? t("common.online") : t("common.setup")}
            </Badge>
          </div>
          <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 truncate">{c.category || "-"}</p>
        </div>

        <div className="flex items-center gap-2 text-xs text-text-dim shrink-0">
          {c.difficulty && (
            <span className="px-2 py-1 rounded bg-main/50">{c.difficulty}</span>
          )}
          {c.setup_time && (
            <span className="flex items-center gap-1">
              <Clock className="w-3 h-3" />
              {c.setup_time}
            </span>
          )}
        </div>

        <div className="flex items-center gap-1 shrink-0">
          <Button variant="secondary" size="sm" onClick={() => onConfigure(c)} leftIcon={<Settings className="w-3 h-3" />}>
            {t("channels.config")}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => onViewDetails(c)}>
            <ChevronRight className="w-4 h-4" />
          </Button>
        </div>
      </Card>
    );
  }

  // Grid view
  return (
    <Card hover padding="none" className={`flex flex-col overflow-hidden group transition-all ${isSelected ? "ring-2 ring-primary" : ""}`}>
      <div className={`h-1.5 bg-linear-to-r ${c.configured ? "from-success via-success/60 to-success/30" : "from-brand via-brand/60 to-brand/30"}`} />
      <div className="p-5 flex-1 flex flex-col">
        {/* Header */}
        <div className="flex items-start justify-between gap-3 mb-4">
          <div className="flex items-center gap-3 min-w-0">
            <button
              onClick={(e) => { e.stopPropagation(); onSelect(c.name, !isSelected); }}
              className="shrink-0 text-text-dim hover:text-primary transition-colors"
            >
              {isSelected ? <CheckSquare className="w-5 h-5 text-primary" /> : <Square className="w-5 h-5" />}
            </button>
            <div className={`w-10 h-10 rounded-lg flex items-center justify-center text-xl shadow-sm ${c.configured ? "bg-linear-to-br from-success/10 to-success/5 border border-success/20" : "bg-linear-to-br from-brand/10 to-brand/5 border border-primary/20"}`}>
              {getChannelIcon(c.name)}
            </div>
            <div className="min-w-0">
              <h2 className={`text-base font-black truncate transition-colors ${c.configured ? "group-hover:text-success" : "group-hover:text-primary"}`}>{c.display_name || c.name}</h2>
              <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 truncate">{c.category || c.name}</p>
            </div>
          </div>
          <Badge variant={c.configured ? "success" : "warning"}>
            {c.configured ? t("common.online") : t("common.setup")}
          </Badge>
        </div>

        {/* Description */}
        <p className="text-xs text-text-dim line-clamp-2 italic mb-4 flex-1">{c.description || "-"}</p>

        {/* Info tags */}
        <div className="flex flex-wrap gap-2 mb-4">
          {c.difficulty && (
            <span className="px-2 py-1 rounded-lg bg-main/50 text-[10px] font-bold text-text-dim">{c.difficulty}</span>
          )}
          {c.setup_time && (
            <span className="flex items-center gap-1 px-2 py-1 rounded-lg bg-main/50 text-[10px] font-bold text-text-dim">
              <Clock className="w-3 h-3" />
              {c.setup_time}
            </span>
          )}
          {c.has_token !== undefined && (
            <span className={`flex items-center gap-1 px-2 py-1 rounded-lg text-[10px] font-bold ${c.has_token ? "bg-success/10 text-success" : "bg-warning/10 text-warning"}`}>
              <Key className="w-3 h-3" />
              {c.has_token ? t("channels.has_token") : t("channels.no_token")}
            </span>
          )}
        </div>

        {/* Actions */}
        <div className="flex gap-2 mt-auto">
          <Button variant="secondary" size="sm" className="flex-1" onClick={() => onConfigure(c)} leftIcon={<Settings className="w-3 h-3" />}>
            {t("channels.config")}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => onViewDetails(c)}>
            <ChevronRight className="w-4 h-4" />
          </Button>
        </div>
      </div>
    </Card>
  );
}

// Details Modal
function DetailsModal({ channel, onClose, onConfigure, onTest, t }: {
  channel: Channel;
  onClose: () => void;
  onConfigure: () => void;
  onTest: () => void;
  t: (key: string) => string
}) {
  return (
    <Modal isOpen onClose={onClose} variant="drawer-right" size="lg" hideCloseButton>
        {/* Coloured strip + custom header are kept inline so the
            configured/unconfigured stripe still renders. */}
        <div className={`h-2 bg-linear-to-r ${channel.configured ? "from-success via-success/60 to-success/30" : "from-brand via-brand/60 to-brand/30"}`} />
        <div className="p-6 border-b border-border-subtle">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className={`w-12 h-12 rounded-xl flex items-center justify-center text-2xl ${channel.configured ? "bg-success/10 border border-success/20" : "bg-primary/10 border border-primary/20"}`}>
                {getChannelIcon(channel.name)}
              </div>
              <div>
                <h2 className="text-xl font-black">{channel.display_name || channel.name}</h2>
                <p className="text-xs font-black uppercase tracking-widest text-text-dim/60">{channel.category || channel.name}</p>
              </div>
            </div>
            <button onClick={onClose} className="p-2 hover:bg-main/30 rounded-lg transition-colors" aria-label={t("common.close")}>
              <X className="w-5 h-5 text-text-dim" />
            </button>
          </div>
        </div>

        {/* Content */}
        <div className="p-6 space-y-4">
          <div className="p-4 rounded-xl bg-main/30">
            <p className="text-xs text-text-dim italic">{channel.description || "-"}</p>
          </div>

          <div className="space-y-3">
            <h3 className="text-xs font-black uppercase tracking-wider text-text-dim">{t("common.properties")}</h3>
            <div className="space-y-2">
              <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                <span className="text-xs font-bold text-text-dim">{t("common.status")}</span>
                <Badge variant={channel.configured ? "success" : "warning"}>
                  {channel.configured ? t("common.online") : t("common.setup")}
                </Badge>
              </div>
              {channel.difficulty && (
                <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                  <span className="text-xs font-bold text-text-dim">{t("channels.difficulty")}</span>
                  <span className="text-xs font-bold">{channel.difficulty}</span>
                </div>
              )}
              {channel.setup_time && (
                <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                  <span className="text-xs font-bold text-text-dim">{t("channels.setup_time")}</span>
                  <span className="text-xs font-bold">{channel.setup_time}</span>
                </div>
              )}
              {channel.setup_type && (
                <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                  <span className="text-xs font-bold text-text-dim">{t("channels.setup_type")}</span>
                  <span className="text-xs font-bold">{channel.setup_type}</span>
                </div>
              )}
              <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                <span className="text-xs font-bold text-text-dim">{t("channels.has_token")}</span>
                <span className={`text-xs font-bold ${channel.has_token ? "text-success" : "text-warning"}`}>
                  {channel.has_token ? t("common.yes") : t("common.no")}
                </span>
              </div>
            </div>
          </div>

          {/* Webhook Endpoint */}
          {channel.webhook_endpoint && (
            <div className="space-y-2">
              <h3 className="text-xs font-black uppercase tracking-wider text-text-dim">Webhook Endpoint</h3>
              <div className="p-3 rounded-lg bg-primary/5 border border-primary/20">
                <code className="text-xs font-mono text-primary break-all select-all">{channel.webhook_endpoint}</code>
                <p className="text-[10px] text-text-dim mt-1">Configure this path on the external platform. Port is the API listen port (default 4545).</p>
              </div>
            </div>
          )}

          {/* Setup Steps */}
          {channel.setup_steps && channel.setup_steps.length > 0 && (
            <div className="space-y-3">
              <h3 className="text-xs font-black uppercase tracking-wider text-text-dim">{t("channels.setup_steps")}</h3>
              <div className="space-y-2">
                {channel.setup_steps.map((step, idx) => (
                  <div key={idx} className="flex items-start gap-3 p-3 rounded-lg bg-main/20">
                    <span className="w-5 h-5 rounded-full bg-primary/20 text-primary text-xs font-bold flex items-center justify-center shrink-0">{idx + 1}</span>
                    <p className="text-xs text-text-main">{step}</p>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Fields */}
          {channel.fields && channel.fields.length > 0 && (
            <div className="space-y-3">
              <h3 className="text-xs font-black uppercase tracking-wider text-text-dim">{t("channels.required_fields")}</h3>
              <div className="space-y-2">
                {channel.fields.map((field, idx) => (
                  <div key={idx} className="flex items-center justify-between p-3 rounded-lg bg-main/20">
                    <div className="flex items-center gap-2">
                      <span className="text-xs font-bold text-text-main">{field.label || field.key}</span>
                      {field.required && <span className="text-error text-[10px]">*</span>}
                    </div>
                    <div className="flex items-center gap-2">
                      {field.has_value ? (
                        <CheckCircle2 className="w-4 h-4 text-success" />
                      ) : (
                        <AlertCircle className="w-4 h-4 text-warning" />
                      )}
                      {field.env_var && (
                        <span className="text-[10px] font-mono text-text-dim">{field.env_var}</span>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Actions */}
          <div className="flex gap-2 pt-2">
            <Button variant="primary" className="flex-1" onClick={onConfigure} leftIcon={<Settings className="w-4 h-4" />}>
              {channel.configured ? t("channels.update_config") : t("channels.setup_adapter")}
            </Button>
            {channel.configured && (
              <Button variant="secondary" onClick={onTest} leftIcon={<CheckCircle2 className="w-4 h-4" />}>
                {t("channels.test") || "Test"}
              </Button>
            )}
          </div>
        </div>

        {/* Footer */}
        <div className="p-4 border-t border-border-subtle flex justify-end">
          <Button variant="ghost" onClick={onClose}>{t("common.close")}</Button>
        </div>
    </Modal>
  );
}

// Config Dialog — standard form with controlled inputs
function ConfigDialog({ channel, onClose, t }: { channel: Channel; onClose: () => void; t: (key: string) => string }) {
  const addToast = useUIStore((s) => s.addToast);
  const fields = useMemo(() => (channel.fields ?? []).filter(f => !f.advanced), [channel.fields]);

  // Build initial form values: non-secret fields use saved value, secrets start empty
  const initialValues = useMemo(() => {
    const vals: Record<string, string> = {};
    for (const f of fields) {
      if (f.readonly) continue;
      if (f.type === "select" && f.options?.length) {
        // Select: use saved value or fall back to first option
        vals[f.key] = f.value || f.options[0];
      } else {
        vals[f.key] = (f.type !== "secret" && f.value) ? f.value : "";
      }
    }
    return vals;
  }, [fields]);
  const [values, setValues] = useState<Record<string, string>>(initialValues);

  const setValue = (key: string, val: string) => setValues(prev => ({ ...prev, [key]: val }));

  // Find the "controlling" select field (e.g. mode) to drive show_when visibility
  const controlField = useMemo(() => fields.find(f => f.type === "select" && f.options), [fields]);
  const controlValue = controlField ? (values[controlField.key] || "") : "";

  // Filter visible fields: hide those whose show_when doesn't match the control value
  const visibleFields = useMemo(
    () => fields.filter(f => !f.show_when || f.show_when === controlValue),
    [fields, controlValue],
  );

  // Only submit non-readonly, non-empty values (skip untouched secrets)
  const configMutation = useConfigureChannel();
  const handleSubmit = () => {
    const payload: Record<string, string> = {};
    for (const f of visibleFields) {
      if (f.readonly) continue;
      const v = values[f.key];
      if (v) payload[f.key] = v;
    }
    configMutation.mutate(
      { channelName: channel.name, config: payload },
      {
        onSuccess: () => {
          addToast(t("channels.config_success") || `${channel.display_name || channel.name} configured`, "success");
          onClose();
        },
        onError: (err: any) => addToast(err.message || t("channels.config_failed") || "Failed to configure channel", "error"),
      },
    );
  };

  return (
    <Modal isOpen onClose={onClose} variant="panel-right" size="md" hideCloseButton>
        <div className="px-6 py-5 border-b border-border-subtle">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-xl bg-primary/10 flex items-center justify-center">
                <Settings className="w-5 h-5 text-primary" />
              </div>
              <div>
                <h3 className="text-base font-black">{channel.display_name || channel.name}</h3>
                <p className="text-[10px] text-text-dim mt-0.5">{t("channels.configure")}</p>
              </div>
            </div>
            <button onClick={onClose} className="p-2 rounded-xl hover:bg-main transition-colors" aria-label={t("common.close")}><X className="w-4 h-4" /></button>
          </div>
        </div>
        <div className="p-6">
        <p className="text-xs text-text-dim mb-5">{channel.description}</p>

        {/* Configuration Fields */}
        {visibleFields.length > 0 ? (
          <div className="space-y-3 mb-6 max-h-80 overflow-y-auto">
            {visibleFields.map((field) => (
              <div key={field.key}>
                <label className="text-xs font-bold text-text-dim mb-1 block">
                  {field.label || field.key} {field.required && <span className="text-error">*</span>}
                </label>
                {field.readonly ? (
                  <div className="flex gap-2">
                    <input
                      type="text"
                      value={field.value || field.placeholder || ""}
                      readOnly
                      className="flex-1 rounded-lg border border-border-subtle bg-main/50 px-3 py-2 text-xs text-text-dim font-mono"
                    />
                    <button
                      onClick={async () => {
                        const ok = await copyToClipboard(field.value || field.placeholder || "");
                        addToast(ok ? t("common.copied") : t("common.copy_failed"), ok ? "success" : "error");
                      }}
                      className="px-3 py-2 rounded-lg bg-primary/10 text-primary text-xs hover:bg-primary/20 transition-colors shrink-0"
                      title={t("common.copy")}
                    >
                      {t("common.copy")}
                    </button>
                  </div>
                ) : field.type === "select" && field.options ? (
                  <select
                    value={values[field.key] || ""}
                    onChange={(e) => setValue(field.key, e.target.value)}
                    className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-xs focus:border-primary focus:ring-1 focus:ring-primary/20 outline-none"
                  >
                    {field.options.map((opt) => (
                      <option key={opt} value={opt}>{opt}</option>
                    ))}
                  </select>
                ) : (
                  <input
                    type={field.type === "secret" ? "password" : "text"}
                    value={values[field.key] || ""}
                    onChange={(e) => setValue(field.key, e.target.value)}
                    placeholder={field.has_value ? "••••••••  (leave empty to keep)" : (field.placeholder || field.env_var || field.key)}
                    className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-xs focus:border-primary focus:ring-1 focus:ring-primary/20 outline-none"
                  />
                )}
              </div>
            ))}
          </div>
        ) : (
          <div className="mb-6 p-4 rounded-lg bg-main/30 text-center">
            <p className="text-xs text-text-dim">{t("channels.no_fields_required")}</p>
          </div>
        )}

        {/* Buttons */}
        <div className="flex gap-3">
          <Button variant="secondary" className="flex-1" onClick={onClose}>{t("common.cancel")}</Button>
          <Button variant="primary" className="flex-1" onClick={handleSubmit} disabled={configMutation.isPending}>
            {configMutation.isPending ? t("common.saving") : t("common.save")}
          </Button>
        </div>
        </div>
    </Modal>
  );
}

// QR Login Dialog for channels with setup_type === "qr" (e.g. WeChat, WhatsApp)
function QrLoginDialog({ channel, onClose, t }: { channel: Channel; onClose: () => void; t: (key: string) => string }) {
  const configureChannelMutation = useConfigureChannel();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const cancelledRef = useRef(false);
  const pollIdRef = useRef(0);
  const [phase, setPhase] = useState<"idle" | "loading" | "scanning" | "success" | "error">("idle");
  const [message, setMessage] = useState("");

  useEffect(() => () => { cancelledRef.current = true; pollIdRef.current += 1; }, []);

  const startQr = useCallback(async () => {
    const pollId = ++pollIdRef.current;
    cancelledRef.current = false;
    setPhase("loading");
    setMessage("");
    try {
      // QR start/status are imperative long-poll probes, so they stay raw instead of going through cached query hooks.
      const qrStart = channel.name === "whatsapp" ? whatsappQrStart : wechatQrStart;
      const qrStatus = channel.name === "whatsapp" ? whatsappQrStatus : wechatQrStatus;
      const displayName = channel.name === "whatsapp" ? "WhatsApp" : "WeChat";

      const res = await qrStart();
      if (!res.available || !res.qr_code) {
        setPhase("error");
        setMessage(res.message || t("channels.qr_failed"));
        return;
      }
      setPhase("scanning");
      setMessage(res.message || `Scan this QR code with your ${displayName} app`);

      // Render QR code to canvas — use the full URL so the app recognises the scan
      const qrContent = res.qr_url || res.qr_code;
      if (canvasRef.current && qrContent) {
        QRCode.toCanvas(canvasRef.current, qrContent, { width: 256, margin: 2 });
      }

      // Serial long-poll: wait for each request to finish before sending the next.
      // The backend holds each request ~30s (long-poll), so setInterval would
      // stack up parallel requests that all resolve at once on scan → flashing UI.
      const pollLoop = async () => {
        while (!cancelledRef.current && pollIdRef.current === pollId) {
          try {
            const status = await qrStatus(res.qr_code!);
            if (cancelledRef.current || pollIdRef.current !== pollId) break;
            if (status.connected && status.bot_token) {
              cancelledRef.current = true;
              try {
                await configureChannelMutation.mutateAsync({
                  channelName: channel.name,
                  config: { bot_token_env: status.bot_token },
                });
              } catch (error) {
                setPhase("error");
                setMessage(error instanceof Error ? error.message : t("channels.qr_failed"));
                return;
              }
              setPhase("success");
              setMessage(t("channels.login_success"));
              setTimeout(onClose, 1500);
              return;
            } else if (status.expired) {
              cancelledRef.current = true;
              setPhase("error");
              setMessage(status.message || "QR code expired");
              return;
            }
          } catch {
            // Transient error — wait briefly then retry
            if (cancelledRef.current || pollIdRef.current !== pollId) break;
            await new Promise(r => setTimeout(r, 3000));
          }
        }
      };
      pollLoop();
    } catch (e) {
      setPhase("error");
      setMessage(e instanceof Error ? e.message : "Failed to start QR login");
    }
  }, [channel.name, configureChannelMutation, onClose, t]);

  // Auto-start on mount
  useEffect(() => { startQr(); }, [startQr]);

  return (
    <Modal isOpen onClose={onClose} variant="panel-right" size="md" hideCloseButton>
        <div className="px-6 py-5 border-b border-border-subtle">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-xl bg-primary/10 flex items-center justify-center text-primary text-sm font-bold">
                {channel.icon || "QR"}
              </div>
              <div>
                <h3 className="text-base font-black">{channel.display_name || channel.name}</h3>
                <p className="text-[10px] text-text-dim mt-0.5">{t("channels.qr_login") || "QR Code Login"}</p>
              </div>
            </div>
            <button onClick={onClose} className="p-2 rounded-xl hover:bg-main transition-colors" aria-label={t("common.close")}><X className="w-4 h-4" /></button>
          </div>
        </div>

        <div className="p-6 flex flex-col items-center gap-4">
          {phase === "loading" && (
            <div className="w-64 h-64 flex items-center justify-center bg-main/30 rounded-xl">
              <div className="animate-spin w-8 h-8 border-2 border-primary border-t-transparent rounded-full" />
            </div>
          )}

          {phase === "scanning" && (
            <div className="bg-white rounded-xl p-2">
              <canvas ref={canvasRef} />
            </div>
          )}

          {phase === "success" && (
            <div className="w-64 h-64 flex items-center justify-center bg-success/10 rounded-xl">
              <CheckCircle2 className="w-16 h-16 text-success" />
            </div>
          )}

          {phase === "error" && (
            <div className="w-64 h-64 flex flex-col items-center justify-center bg-error/10 rounded-xl gap-3">
              <XCircle className="w-16 h-16 text-error" />
              <Button variant="secondary" onClick={startQr}>{t("common.retry") || "Retry"}</Button>
            </div>
          )}

          <p className="text-xs text-text-dim text-center max-w-xs">{message}</p>
        </div>

        <div className="p-4 border-t border-border-subtle flex justify-end">
          <Button variant="ghost" onClick={onClose}>{t("common.close")}</Button>
        </div>
    </Modal>
  );
}

type TabType = "configured" | "unconfigured";

export function ChannelsPage() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<TabType>("configured");
  const [search, setSearch] = useState("");
  const [sortField, setSortField] = useState<SortField>("name");
  const [sortOrder, setSortOrder] = useState<SortOrder>("asc");
  const [viewMode, setViewMode] = useState<ViewMode>("grid");
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [detailsChannel, setDetailsChannel] = useState<Channel | null>(null);
  const [configuringChannel, setConfiguringChannel] = useState<Channel | null>(null);
  const [qrLoginChannel, setQrLoginChannel] = useState<Channel | null>(null);

  const addToast = useUIStore((s) => s.addToast);

  const channelsQuery = useChannels();
  const testMut = useTestChannel();
  const reloadMut = useReloadChannels();

  const handleTest = (name: string) => {
    testMut.mutate(name, {
      onSuccess: () => addToast(t("channels.test_success", { defaultValue: `Channel "${name}" test passed` }), "success"),
      onError: (err: any) => addToast(err.message || t("channels.test_failed", { defaultValue: `Channel "${name}" test failed` }), "error"),
    });
  };
  const handleReload = () => {
    reloadMut.mutate(undefined, {
      onSuccess: () => addToast(t("channels.reload_success", { defaultValue: "Channels reloaded" }), "success"),
      onError: (err: any) => addToast(err.message || t("common.error"), "error"),
    });
  };

  const channels = channelsQuery.data ?? [];
  const configuredCount = useMemo(() => channels.filter(c => c.configured).length, [channels]);
  const unconfiguredCount = useMemo(() => channels.filter(c => !c.configured).length, [channels]);

  // Auto-switch to "unconfigured" tab when no channels are configured,
  // so new users see the setup buttons instead of an empty page.
  const hasInitTab = useRef(false);
  useEffect(() => {
    if (!hasInitTab.current && channels.length > 0) {
      hasInitTab.current = true;
      if (configuredCount === 0) setActiveTab("unconfigured");
    }
  }, [channels.length, configuredCount]);

  // Filter, search, and sort
  const filteredChannels = useMemo(
    () => [...channels]
      .filter(c => {
        const tabMatch = activeTab === "configured" ? c.configured : !c.configured;
        const searchMatch = !search || (c.display_name || c.name).toLowerCase().includes(search.toLowerCase()) || c.category?.toLowerCase().includes(search.toLowerCase());
        return tabMatch && searchMatch;
      })
      .sort((a, b) => {
        let cmp = 0;
        if (sortField === "name") cmp = a.name.localeCompare(b.name);
        else if (sortField === "category") cmp = (a.category || "").localeCompare(b.category || "");
        return sortOrder === "asc" ? cmp : -cmp;
      }),
    [channels, activeTab, search, sortField, sortOrder],
  );

  const paginatedChannels = filteredChannels;

  const handleTabChange = (tab: TabType) => {
    setActiveTab(tab);
    setSelectedIds(new Set());
  };

  const handleSort = (field: SortField) => {
    if (sortField === field) {
      setSortOrder(sortOrder === "asc" ? "desc" : "asc");
    } else {
      setSortField(field);
      setSortOrder("asc");
    }
  };

  const handleSelect = (name: string, checked: boolean) => {
    setSelectedIds(prev => {
      const next = new Set(prev);
      if (checked) next.add(name);
      else next.delete(name);
      return next;
    });
  };

  const handleSelectAll = () => {
    if (selectedIds.size === paginatedChannels.length) {
      setSelectedIds(new Set());
    } else {
      setSelectedIds(new Set(paginatedChannels.map(c => c.name)));
    }
  };

  const allSelected = paginatedChannels.length > 0 && selectedIds.size === paginatedChannels.length;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("common.infrastructure")}
        title={t("channels.title")}
        subtitle={t("channels.subtitle")}
        isFetching={channelsQuery.isFetching}
        onRefresh={() => void channelsQuery.refetch()}
        icon={<Network className="h-4 w-4" />}
        helpText={t("channels.help")}
        actions={
          <div className="flex items-center gap-2">
            <Button variant="secondary" size="sm" onClick={handleReload} disabled={reloadMut.isPending}>
              {t("channels.reload", { defaultValue: "Reload" })}
            </Button>
            <div className="hidden rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase text-text-dim sm:block">
              {t("channels.configured_count", { count: configuredCount })}
            </div>
          </div>
        }
      />

      {/* Search & Controls */}
      <div className="flex flex-col sm:flex-row gap-3">
        <div className="flex-1">
          <Input
            value={search}
            onChange={(e) => { setSearch(e.target.value); setSelectedIds(new Set()); }}
            placeholder={t("common.search")}
            leftIcon={<Search className="w-4 h-4" />}
            rightIcon={search && (
              <button onClick={() => setSearch("")} className="hover:text-text-main" aria-label={t("common.clear_search", { defaultValue: "Clear search" })}>
                <X className="w-3 h-3" />
              </button>
            )}
          />
        </div>

        <div className="flex gap-2 items-center flex-wrap">
          {/* Sort buttons */}
          <div className="flex gap-1 p-1 bg-main/30 rounded-lg">
            <button
              onClick={() => handleSort("name")}
              className={`flex items-center gap-1 px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${sortField === "name" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}
            >
              {t("channels.name")}
            </button>
            <button
              onClick={() => handleSort("category")}
              className={`flex items-center gap-1 px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${sortField === "category" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}
            >
              {t("channels.category")}
            </button>
          </div>

          {/* View toggle */}
          <div className="flex gap-1 p-1 bg-main/30 rounded-lg">
            <button
              onClick={() => setViewMode("grid")}
              className={`p-1.5 rounded-md transition-colors ${viewMode === "grid" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}
            >
              <Grid3X3 className="w-4 h-4" />
            </button>
            <button
              onClick={() => setViewMode("list")}
              className={`p-1.5 rounded-md transition-colors ${viewMode === "list" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}
            >
              <List className="w-4 h-4" />
            </button>
          </div>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex items-center gap-4 flex-wrap overflow-x-auto">
        <div className="flex gap-1 p-1 bg-main/30 rounded-xl w-fit">
          <button
            onClick={() => handleTabChange("configured")}
            className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-colors ${
              activeTab === "configured" ? "bg-surface text-success shadow-sm" : "text-text-dim hover:text-text-main"
            }`}
          >
            <CheckCircle2 className="w-4 h-4" />
            {t("channels.configured")}
            <span className={`ml-1 px-1.5 py-0.5 rounded-full text-[10px] ${activeTab === "configured" ? "bg-success/20 text-success" : "bg-border-subtle text-text-dim"}`}>
              {configuredCount}
            </span>
          </button>
          <button
            onClick={() => handleTabChange("unconfigured")}
            className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-colors ${
              activeTab === "unconfigured" ? "bg-surface text-primary shadow-sm" : "text-text-dim hover:text-text-main"
            }`}
          >
            <XCircle className="w-4 h-4" />
            {t("channels.unconfigured")}
            <span className={`ml-1 px-1.5 py-0.5 rounded-full text-[10px] ${activeTab === "unconfigured" ? "bg-primary/20 text-primary" : "bg-border-subtle text-text-dim"}`}>
              {unconfiguredCount}
            </span>
          </button>
        </div>
      </div>

      {channelsQuery.isLoading ? (
        <div className={viewMode === "grid" ? "grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4 3xl:grid-cols-5 4xl:grid-cols-6" : "flex flex-col gap-2"}>
          {[1, 2, 3].map((i) => <CardSkeleton key={i} />)}
        </div>
      ) : channels.length === 0 ? (
        <EmptyState title={t("channels.no_channels")} icon={<Network className="h-6 w-6" />} />
      ) : filteredChannels.length === 0 ? (
        <EmptyState title={search ? t("channels.no_results") : (activeTab === "configured" ? t("channels.no_configured") : t("channels.no_unconfigured"))} icon={<Search className="h-6 w-6" />} />
      ) : (
        <>
          {/* Select all */}
          <div className="flex items-center gap-2">
            <button
              onClick={handleSelectAll}
              className="flex items-center gap-2 text-xs font-bold text-text-dim hover:text-text-main transition-colors"
            >
              {allSelected ? <CheckSquare className="w-4 h-4 text-primary" /> : <Square className="w-4 h-4" />}
              {t("channels.select_all")}
            </button>
            {search && (
              <span className="text-xs text-text-dim">({filteredChannels.length} {t("channels.results")})</span>
            )}
          </div>

          <div className={viewMode === "grid" ? "grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4 3xl:grid-cols-5 4xl:grid-cols-6" : "flex flex-col gap-2"}>
            {paginatedChannels.map((c) => (
              <ChannelCard
                key={c.name}
                channel={c}
                isSelected={selectedIds.has(c.name)}
                viewMode={viewMode}
                onSelect={handleSelect}
                onConfigure={(ch) => ch.setup_type === "qr" ? setQrLoginChannel(ch) : setConfiguringChannel(ch)}
                onViewDetails={setDetailsChannel}
                t={t}
              />
            ))}
          </div>
        </>
      )}

      {/* Details Modal */}
      {detailsChannel && (
        <DetailsModal
          channel={detailsChannel}
          onClose={() => setDetailsChannel(null)}
          onConfigure={() => {
            const ch = detailsChannel;
            setDetailsChannel(null);
            if (ch.setup_type === "qr") {
              setQrLoginChannel(ch);
            } else {
              setConfiguringChannel(ch);
            }
          }}
          onTest={() => handleTest(detailsChannel.name)}
          t={t}
        />
      )}

      {/* Config Dialog */}
      {configuringChannel && (
        <ConfigDialog
          channel={configuringChannel}
          onClose={() => setConfiguringChannel(null)}
          t={t}
        />
      )}

      {/* QR Login Dialog */}
      {qrLoginChannel && (
        <QrLoginDialog
          channel={qrLoginChannel}
          onClose={() => setQrLoginChannel(null)}
          t={t}
        />
      )}
    </div>
  );
}
