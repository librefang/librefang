import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listChannels, configureChannel, wechatQrStart, wechatQrStatus } from "../api";
import QRCode from "qrcode";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { Pagination } from "../components/ui/Pagination";
import {
  Network, Search, CheckCircle2, XCircle, ChevronRight, X, Grid3X3, List,
  Settings, Key, Clock, AlertCircle, CheckSquare, Square,
  MessageCircle, Mail, Phone, Link2, Radio, Send, Bell, Wifi, Globe
} from "lucide-react";

const REFRESH_MS = 30000;
const ITEMS_PER_PAGE = 6;

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

interface Channel {
  name: string;
  display_name?: string;
  configured?: boolean;
  has_token?: boolean;
  category?: string;
  description?: string;
  icon?: string;
  difficulty?: string;
  setup_time?: string;
  quick_setup?: string;
  setup_type?: string;
  setup_steps?: string[];
  fields?: {
    key: string;
    label?: string;
    type?: string;
    required?: boolean;
    advanced?: boolean;
    has_value?: boolean;
    env_var?: string | null;
  }[];
}

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
      <Card hover padding="sm" className={`flex items-center gap-4 group transition-all ${isSelected ? "ring-2 ring-brand" : ""}`}>
        <button
          onClick={(e) => { e.stopPropagation(); onSelect(c.name, !isSelected); }}
          className="shrink-0 text-text-dim hover:text-brand transition-colors"
        >
          {isSelected ? <CheckSquare className="w-5 h-5 text-brand" /> : <Square className="w-5 h-5" />}
        </button>

        <div className={`w-8 h-8 rounded-lg flex items-center justify-center text-lg shrink-0 ${c.configured ? "bg-success/10 border border-success/20" : "bg-brand/10 border border-brand/20"}`}>
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
    <Card hover padding="none" className={`flex flex-col overflow-hidden group transition-all ${isSelected ? "ring-2 ring-brand" : ""}`}>
      <div className={`h-1.5 bg-linear-to-r ${c.configured ? "from-success via-success/60 to-success/30" : "from-brand via-brand/60 to-brand/30"}`} />
      <div className="p-5 flex-1 flex flex-col">
        {/* Header */}
        <div className="flex items-start justify-between gap-3 mb-4">
          <div className="flex items-center gap-3 min-w-0">
            <button
              onClick={(e) => { e.stopPropagation(); onSelect(c.name, !isSelected); }}
              className="shrink-0 text-text-dim hover:text-brand transition-colors"
            >
              {isSelected ? <CheckSquare className="w-5 h-5 text-brand" /> : <Square className="w-5 h-5" />}
            </button>
            <div className={`w-10 h-10 rounded-lg flex items-center justify-center text-xl shadow-sm ${c.configured ? "bg-linear-to-br from-success/10 to-success/5 border border-success/20" : "bg-linear-to-br from-brand/10 to-brand/5 border border-brand/20"}`}>
              {getChannelIcon(c.name)}
            </div>
            <div className="min-w-0">
              <h2 className={`text-base font-black truncate transition-colors ${c.configured ? "group-hover:text-success" : "group-hover:text-brand"}`}>{c.display_name || c.name}</h2>
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
function DetailsModal({ channel, onClose, onConfigure, t }: {
  channel: Channel;
  onClose: () => void;
  onConfigure: () => void;
  t: (key: string) => string
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center p-0 sm:p-4 bg-black/50 backdrop-blur-sm" onClick={onClose}>
      <div className="bg-surface rounded-2xl border border-border-subtle w-full sm:max-w-lg shadow-2xl rounded-t-2xl sm:rounded-2xl max-h-[90vh] overflow-y-auto animate-fade-in-scale" onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className={`h-2 bg-linear-to-r ${channel.configured ? "from-success via-success/60 to-success/30" : "from-brand via-brand/60 to-brand/30"} rounded-t-2xl`} />
        <div className="p-6 border-b border-border-subtle">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className={`w-12 h-12 rounded-xl flex items-center justify-center text-2xl ${channel.configured ? "bg-success/10 border border-success/20" : "bg-brand/10 border border-brand/20"}`}>
                {getChannelIcon(channel.name)}
              </div>
              <div>
                <h2 className="text-xl font-black">{channel.display_name || channel.name}</h2>
                <p className="text-xs font-black uppercase tracking-widest text-text-dim/60">{channel.category || channel.name}</p>
              </div>
            </div>
            <button onClick={onClose} className="p-2 hover:bg-main/30 rounded-lg transition-colors">
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

          {/* Setup Steps */}
          {channel.setup_steps && channel.setup_steps.length > 0 && (
            <div className="space-y-3">
              <h3 className="text-xs font-black uppercase tracking-wider text-text-dim">{t("channels.setup_steps")}</h3>
              <div className="space-y-2">
                {channel.setup_steps.map((step, idx) => (
                  <div key={idx} className="flex items-start gap-3 p-3 rounded-lg bg-main/20">
                    <span className="w-5 h-5 rounded-full bg-brand/20 text-brand text-xs font-bold flex items-center justify-center shrink-0">{idx + 1}</span>
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
          </div>
        </div>

        {/* Footer */}
        <div className="p-4 border-t border-border-subtle flex justify-end">
          <Button variant="ghost" onClick={onClose}>{t("common.close")}</Button>
        </div>
      </div>
    </div>
  );
}

// Config Dialog
function ConfigDialog({ channel, onClose, t }: { channel: Channel; onClose: () => void; t: (key: string) => string }) {
  const queryClient = useQueryClient();
  const [configs, setConfigs] = useState<Record<string, string>>({});

  const configMutation = useMutation({
    mutationFn: () => configureChannel(channel.name, configs),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["channels", "list"] });
      onClose();
    }
  });

  const handleAddConfig = (key: string, value: string) => {
    if (key.trim()) {
      setConfigs(prev => ({ ...prev, [key.trim()]: value }));
    }
  };

  const handleRemoveConfig = (key: string) => {
    setConfigs(prev => {
      const next = { ...prev };
      delete next[key];
      return next;
    });
  };

  return (
    <div className="fixed inset-0 bg-black/40 flex items-end sm:items-center justify-center z-50 backdrop-blur-sm" onClick={onClose}>
      <div className="bg-surface border border-border-subtle rounded-2xl w-full sm:max-w-md shadow-2xl rounded-t-2xl sm:rounded-2xl animate-fade-in-scale" onClick={e => e.stopPropagation()}>
        <div className="px-6 py-5 border-b border-border-subtle">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-xl bg-brand/10 flex items-center justify-center">
                <Settings className="w-5 h-5 text-brand" />
              </div>
              <div>
                <h3 className="text-base font-black">{channel.display_name || channel.name}</h3>
                <p className="text-[10px] text-text-dim mt-0.5">{t("channels.configure")}</p>
              </div>
            </div>
            <button onClick={onClose} className="p-2 rounded-xl hover:bg-main transition-colors"><X className="w-4 h-4" /></button>
          </div>
        </div>
        <div className="p-6">
        <p className="text-xs text-text-dim mb-5">{channel.description}</p>

        {/* Added Configurations */}
        {Object.keys(configs).length > 0 && (
          <div className="space-y-2 mb-4">
            {Object.entries(configs).map(([key, value]) => (
              <div key={key} className="flex items-center justify-between bg-main rounded-lg px-3 py-2">
                <div className="min-w-0 flex-1">
                  <span className="text-xs font-bold text-brand">{key}</span>
                  <span className="text-text-dim mx-2">:</span>
                  <span className="text-xs font-mono truncate">{value}</span>
                </div>
                <button onClick={() => handleRemoveConfig(key)} className="text-error hover:text-error/80 ml-2">
                  <X className="w-4 h-4" />
                </button>
              </div>
            ))}
          </div>
        )}

        {/* Configuration Fields */}
        {channel.fields && channel.fields.length > 0 ? (
          <div className="space-y-3 mb-6 max-h-60 overflow-y-auto">
            {channel.fields.filter(f => !f.advanced).map((field, idx) => (
              <div key={idx}>
                <label className="text-xs font-bold text-text-dim mb-1 block">
                  {field.label || field.key} {field.required && <span className="text-error">*</span>}
                </label>
                <input
                  type={field.type === "password" ? "password" : "text"}
                  defaultValue={field.has_value ? "••••••••" : ""}
                  onBlur={(e) => handleAddConfig(field.key, e.target.value)}
                  placeholder={field.env_var || field.key}
                  className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-xs focus:border-brand focus:ring-1 focus:ring-brand/20 outline-none"
                />
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
          <Button variant="primary" className="flex-1" onClick={() => configMutation.mutate()} disabled={configMutation.isPending}>
            {configMutation.isPending ? t("common.saving") : t("common.save")}
          </Button>
        </div>
        </div>
      </div>
    </div>
  );
}

// QR Login Dialog for channels with setup_type === "qr" (e.g. WeChat)
function QrLoginDialog({ channel, onClose, t }: { channel: Channel; onClose: () => void; t: (key: string) => string }) {
  const queryClient = useQueryClient();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const cancelledRef = useRef(false);
  const [phase, setPhase] = useState<"idle" | "loading" | "scanning" | "success" | "error">("idle");
  const [qrCode, setQrCode] = useState("");
  const [message, setMessage] = useState("");

  const cleanup = useCallback(() => {
    cancelledRef.current = true;
  }, []);

  useEffect(() => () => { cancelledRef.current = true; }, []);

  const startQr = useCallback(async () => {
    cancelledRef.current = false;
    setPhase("loading");
    setMessage("");
    try {
      const res = await wechatQrStart();
      if (!res.available || !res.qr_code) {
        setPhase("error");
        setMessage(res.message || "Failed to get QR code");
        return;
      }
      setQrCode(res.qr_code);
      setPhase("scanning");
      setMessage(res.message || "Scan this QR code with your WeChat app");

      // Render QR code to canvas — use the full URL so WeChat recognises the scan
      const qrContent = res.qr_url || res.qr_code;
      if (canvasRef.current && qrContent) {
        QRCode.toCanvas(canvasRef.current, qrContent, { width: 256, margin: 2 });
      }

      // Serial long-poll: wait for each request to finish before sending the next.
      // The backend holds each request ~30s (iLink long-poll), so setInterval would
      // stack up parallel requests that all resolve at once on scan → flashing UI.
      const pollLoop = async () => {
        while (!cancelledRef.current) {
          try {
            const status = await wechatQrStatus(res.qr_code!);
            if (cancelledRef.current) break;
            if (status.connected && status.bot_token) {
              cancelledRef.current = true;
              setPhase("success");
              setMessage("Login successful! Saving configuration...");
              await configureChannel(channel.name, { bot_token_env: status.bot_token });
              queryClient.invalidateQueries({ queryKey: ["channels", "list"] });
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
            if (cancelledRef.current) break;
            await new Promise(r => setTimeout(r, 3000));
          }
        }
      };
      pollLoop();
    } catch (e) {
      setPhase("error");
      setMessage(e instanceof Error ? e.message : "Failed to start QR login");
    }
  }, [channel.name, cleanup, onClose, queryClient]);

  // Auto-start on mount
  useEffect(() => { startQr(); }, [startQr]);

  return (
    <div className="fixed inset-0 bg-black/40 flex items-end sm:items-center justify-center z-50 backdrop-blur-xl backdrop-saturate-150" onClick={onClose}>
      <div className="bg-surface border border-border-subtle rounded-2xl w-full sm:max-w-md shadow-2xl rounded-t-2xl sm:rounded-2xl animate-fade-in-scale" onClick={e => e.stopPropagation()}>
        <div className="px-6 py-5 border-b border-border-subtle">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-xl bg-brand/10 flex items-center justify-center text-brand text-sm font-bold">
                {channel.icon || "QR"}
              </div>
              <div>
                <h3 className="text-base font-black">{channel.display_name || channel.name}</h3>
                <p className="text-[10px] text-text-dim mt-0.5">{t("channels.qr_login") || "QR Code Login"}</p>
              </div>
            </div>
            <button onClick={onClose} className="p-2 rounded-xl hover:bg-main transition-colors"><X className="w-4 h-4" /></button>
          </div>
        </div>

        <div className="p-6 flex flex-col items-center gap-4">
          {phase === "loading" && (
            <div className="w-64 h-64 flex items-center justify-center bg-main/30 rounded-xl">
              <div className="animate-spin w-8 h-8 border-2 border-brand border-t-transparent rounded-full" />
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
      </div>
    </div>
  );
}

type TabType = "configured" | "unconfigured";

export function ChannelsPage() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<TabType>("configured");
  const [page, setPage] = useState(1);
  const [search, setSearch] = useState("");
  const [sortField, setSortField] = useState<SortField>("name");
  const [sortOrder, setSortOrder] = useState<SortOrder>("asc");
  const [viewMode, setViewMode] = useState<ViewMode>("grid");
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [detailsChannel, setDetailsChannel] = useState<Channel | null>(null);
  const [configuringChannel, setConfiguringChannel] = useState<Channel | null>(null);
  const [qrLoginChannel, setQrLoginChannel] = useState<Channel | null>(null);

  const channelsQuery = useQuery({ queryKey: ["channels", "list"], queryFn: listChannels, refetchInterval: REFRESH_MS });

  const channels = channelsQuery.data ?? [];
  const configuredCount = useMemo(() => channels.filter(c => c.configured).length, [channels]);
  const unconfiguredCount = useMemo(() => channels.filter(c => !c.configured).length, [channels]);

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

  const totalPages = Math.ceil(filteredChannels.length / ITEMS_PER_PAGE);
  const paginatedChannels = filteredChannels.slice(
    (page - 1) * ITEMS_PER_PAGE,
    page * ITEMS_PER_PAGE
  );

  const handleTabChange = (tab: TabType) => {
    setActiveTab(tab);
    setPage(1);
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
          <div className="hidden rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase text-text-dim sm:block">
            {t("channels.configured_count", { count: configuredCount })}
          </div>
        }
      />

      {/* Search & Controls */}
      <div className="flex flex-col sm:flex-row gap-3">
        <div className="flex-1">
          <Input
            value={search}
            onChange={(e) => { setSearch(e.target.value); setPage(1); setSelectedIds(new Set()); }}
            placeholder={t("common.search")}
            leftIcon={<Search className="w-4 h-4" />}
            rightIcon={search && (
              <button onClick={() => setSearch("")} className="hover:text-text-main">
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
              activeTab === "unconfigured" ? "bg-surface text-brand shadow-sm" : "text-text-dim hover:text-text-main"
            }`}
          >
            <XCircle className="w-4 h-4" />
            {t("channels.unconfigured")}
            <span className={`ml-1 px-1.5 py-0.5 rounded-full text-[10px] ${activeTab === "unconfigured" ? "bg-brand/20 text-brand" : "bg-border-subtle text-text-dim"}`}>
              {unconfiguredCount}
            </span>
          </button>
        </div>
      </div>

      {channelsQuery.isLoading ? (
        <div className={viewMode === "grid" ? "grid gap-4 md:grid-cols-2 xl:grid-cols-3" : "flex flex-col gap-2"}>
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
              {allSelected ? <CheckSquare className="w-4 h-4 text-brand" /> : <Square className="w-4 h-4" />}
              {t("channels.select_all")}
            </button>
            {search && (
              <span className="text-xs text-text-dim">({filteredChannels.length} {t("channels.results")})</span>
            )}
          </div>

          <div className={viewMode === "grid" ? "grid gap-4 md:grid-cols-2 xl:grid-cols-3" : "flex flex-col gap-2"}>
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
          {totalPages > 1 && (
            <Pagination currentPage={page} totalPages={totalPages} onPageChange={setPage} />
          )}
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
