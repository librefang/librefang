import { useMutation, useQuery } from "@tanstack/react-query";
import { formatTime, formatDateTime } from "../lib/datetime";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { listProviders, testProvider, setProviderKey, deleteProviderKey, setProviderUrl, createRegistryContent, setDefaultProvider } from "../api";
import { isProviderAvailable } from "../lib/status";
import { SchemaForm } from "../components/SchemaForm";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { useUIStore } from "../lib/store";
import { useCreateShortcut } from "../lib/useCreateShortcut";
import {
  Server, Zap, Clock, Key, Globe, CheckCircle2, XCircle, Loader2, AlertCircle, Search,
  SortAsc, SortDesc, CheckSquare, Square, ChevronRight, X, Grid3X3, List, Filter,
  ExternalLink, Activity, Cpu, Cloud, Bot, Globe2, Sparkles, Plus, Star, Pencil, Trash2
} from "lucide-react";

const REFRESH_MS = 30000;

const providerIcons: Record<string, React.ReactNode> = {
  openai: <Sparkles className="w-5 h-5" />,
  anthropic: <Cpu className="w-5 h-5" />,
  google: <Globe2 className="w-5 h-5" />,
  azure: <Cloud className="w-5 h-5" />,
  aws: <Cloud className="w-5 h-5" />,
  ollama: <Cpu className="w-5 h-5" />,
  groq: <Sparkles className="w-5 h-5" />,
  deepseek: <Bot className="w-5 h-5" />,
  mistral: <Cpu className="w-5 h-5" />,
  cohere: <Cpu className="w-5 h-5" />,
  fireworks: <Sparkles className="w-5 h-5" />,
  voyage: <Bot className="w-5 h-5" />,
  together: <Globe className="w-5 h-5" />,
};

function getProviderIcon(id: string): React.ReactNode {
  const key = id.toLowerCase().split("-")[0];
  return providerIcons[key] || <Cpu className="w-5 h-5" />;
}

function getLatencyColor(ms?: number) {
  if (ms == null) return "text-text-dim";
  if (ms < 200) return "text-success";
  if (ms < 500) return "text-warning";
  return "text-error";
}


type SortField = "name" | "models" | "latency";
type SortOrder = "asc" | "desc";
type ViewMode = "grid" | "list";
type FilterStatus = "all" | "reachable" | "unreachable";

interface Provider {
  id: string;
  display_name?: string;
  auth_status?: string;
  reachable?: boolean;
  model_count?: number;
  latency_ms?: number;
  api_key_env?: string;
  base_url?: string;
  key_required?: boolean;
  health?: string;
  last_tested?: string;
  error_message?: string;
  media_capabilities?: string[];
}

interface ProviderCardProps {
  provider: Provider;
  isSelected: boolean;
  isDefault: boolean;
  pendingId: string | null;
  viewMode: ViewMode;
  onSelect: (id: string, checked: boolean) => void;
  onTest: (id: string) => void;
  onSetDefault: (id: string) => void;
  onViewDetails: (provider: Provider) => void;
  onQuickConfig: (provider: Provider) => void;
  onEdit: (provider: Provider) => void;
  onDelete: (provider: Provider) => void;
  t: (key: string) => string;
}

function ProviderCard({ provider: p, isSelected, isDefault, pendingId, viewMode, onSelect, onTest, onSetDefault, onViewDetails, onQuickConfig, onEdit, onDelete, t }: ProviderCardProps) {
  const isConfigured = isProviderAvailable(p.auth_status);
  const isCli = p.auth_status === "configured_cli" || p.auth_status === "cli_not_installed" || (!p.base_url && !p.key_required);

  if (viewMode === "list") {
    return (
      <Card hover padding="sm" className={`flex flex-col sm:flex-row items-start sm:items-center gap-3 sm:gap-4 group transition-all ${isSelected ? "ring-2 ring-brand" : ""}`}>
        <div className="flex items-center gap-3 w-full sm:w-auto">
          <button
            onClick={(e) => { e.stopPropagation(); onSelect(p.id, !isSelected); }}
            className="shrink-0 text-text-dim hover:text-brand transition-colors"
          >
            {isSelected ? <CheckSquare className="w-5 h-5 text-brand" /> : <Square className="w-5 h-5" />}
          </button>

          <div className={`w-8 h-8 rounded-lg flex items-center justify-center text-lg shrink-0 ${isConfigured ? "bg-success/10 border border-success/20" : "bg-brand/10 border border-brand/20"}`}>
            {getProviderIcon(p.id)}
          </div>

          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <h3 className="font-black truncate">{p.display_name || p.id}</h3>
              {isCli && <Badge variant="default" className="shrink-0">CLI</Badge>}
              {isConfigured ? (
                <Badge variant={p.reachable === true ? "success" : p.reachable === false ? "error" : "default"} className="shrink-0">
                  {p.reachable === true ? t("providers.online") : p.reachable === false ? t("providers.offline") : t("providers.not_checked")}
                </Badge>
              ) : (
                <Badge variant="warning" className="shrink-0">{t("common.setup")}</Badge>
              )}
            </div>
            <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 truncate">{p.id}</p>
          </div>
        </div>

        <div className="hidden md:flex items-center gap-6 shrink-0">
          <div className="text-center">
            <p className="text-xs font-black">{p.model_count ?? 0}</p>
            <p className="text-[8px] uppercase text-text-dim">{t("providers.models")}</p>
          </div>
          <div className="text-center">
            <p className={`text-xs font-black ${getLatencyColor(p.latency_ms)}`}>{p.latency_ms != null ? `${p.latency_ms}ms` : "-"}</p>
            <p className="text-[8px] uppercase text-text-dim">{t("providers.latency")}</p>
          </div>
          {p.last_tested && (
            <div className="text-center w-20">
              <p className="text-[10px] font-mono text-text-dim">{formatTime(p.last_tested)}</p>
              <p className="text-[8px] uppercase text-text-dim">{t("providers.last_test")}</p>
            </div>
          )}
          {p.media_capabilities && p.media_capabilities.length > 0 && (
            <div className="flex flex-wrap gap-1">
              {p.media_capabilities.map((cap: string) => (
                <Badge key={cap} variant="default" className="text-[8px] px-1 py-0">
                  {cap.replace(/_/g, " ")}
                </Badge>
              ))}
            </div>
          )}
        </div>

        <div className="flex items-center gap-1 shrink-0 self-end sm:self-auto">
          {isDefault && (
            <Badge variant="brand" className="shrink-0">
              <Star className="w-3 h-3 mr-1 inline" />{t("providers.is_default")}
            </Badge>
          )}
          {!isConfigured && (
            <Button variant="ghost" size="sm" onClick={() => onQuickConfig(p)} leftIcon={<Key className="w-3 h-3" />}>
              <span className="hidden sm:inline">{t("providers.config")}</span>
            </Button>
          )}
          {isConfigured && !isDefault && (
            <Button variant="ghost" size="sm" onClick={() => onSetDefault(p.id)} leftIcon={<Star className="w-3 h-3" />}>
              <span className="hidden sm:inline">{t("providers.set_as_default")}</span>
            </Button>
          )}
          {isConfigured && (
            <Button variant="ghost" size="sm" onClick={() => onEdit(p)} leftIcon={<Pencil className="w-3 h-3" />}>
              <span className="hidden sm:inline">{t("common.edit")}</span>
            </Button>
          )}
          {isConfigured && (
            <Button variant="ghost" size="sm" onClick={() => onDelete(p)} leftIcon={<Trash2 className="w-3 h-3 text-error" />}>
              <span className="hidden sm:inline text-error">{t("common.delete")}</span>
            </Button>
          )}
          <Button
            variant="secondary"
            size="sm"
            onClick={() => onTest(p.id)}
            disabled={pendingId === p.id}
            leftIcon={pendingId === p.id ? <Loader2 className="w-3 h-3 animate-spin" /> : <Zap className="w-3 h-3" />}
            className="whitespace-nowrap"
          >
            <span className="hidden sm:inline">{pendingId === p.id ? t("providers.analyzing") : t("providers.test")}</span>
          </Button>
          <Button variant="ghost" size="sm" onClick={() => onViewDetails(p)}>
            <ChevronRight className="w-4 h-4" />
          </Button>
        </div>
      </Card>
    );
  }

  // Grid view
  return (
    <Card hover padding="none" className={`relative flex flex-col overflow-hidden group transition-all ${isSelected ? "ring-2 ring-brand" : ""}`}>
      {isCli && (
        <div className="absolute top-1.5 left-0 z-10 overflow-hidden w-20 h-20 pointer-events-none">
          <div className="absolute top-[12px] left-[-18px] w-[90px] text-center text-[9px] font-black uppercase tracking-wider text-text-dim bg-surface/80 border-y border-border-subtle rotate-[-45deg] py-px">
            CLI
          </div>
        </div>
      )}
      <div className={`relative z-20 h-1.5 bg-gradient-to-r ${isConfigured ? "from-success via-success/60 to-success/30" : "from-brand via-brand/60 to-brand/30"}`} />
      <div className="p-5 flex-1 flex flex-col">
        {/* Header */}
        <div className="flex items-start justify-between gap-3 mb-4">
          <div className="flex items-center gap-3 min-w-0">
            <button
              onClick={(e) => { e.stopPropagation(); onSelect(p.id, !isSelected); }}
              className="shrink-0 text-text-dim hover:text-brand transition-colors"
            >
              {isSelected ? <CheckSquare className="w-5 h-5 text-brand" /> : <Square className="w-5 h-5" />}
            </button>
            <div className={`w-10 h-10 rounded-lg flex items-center justify-center text-xl shadow-sm ${isConfigured ? "bg-gradient-to-br from-success/10 to-success/5 border border-success/20" : "bg-gradient-to-br from-brand/10 to-brand/5 border border-brand/20"}`}>
              {getProviderIcon(p.id)}
            </div>
            <div className="min-w-0">
              <h2 className={`text-base font-black truncate transition-colors ${isConfigured ? "group-hover:text-success" : "group-hover:text-brand"}`}>{p.display_name || p.id}</h2>
              <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 truncate">{p.id}</p>
            </div>
          </div>
          {isConfigured ? (
            <Badge variant={p.reachable === true ? "success" : p.reachable === false ? "error" : "default"}>
              {p.reachable === true ? t("providers.online") : p.reachable === false ? t("providers.offline") : t("providers.not_checked")}
            </Badge>
          ) : (
            <Badge variant="warning">{t("common.setup")}</Badge>
          )}
        </div>

        {/* Stats */}
        <div className="grid grid-cols-2 gap-3 mb-4">
          <div className="p-3 rounded-xl bg-gradient-to-br from-main/60 to-main/30 border border-border-subtle/50">
            <div className="flex items-center gap-1.5 mb-1">
              <Zap className={`w-3 h-3 ${isConfigured ? "text-success" : "text-brand"}`} />
              <p className="text-[9px] font-black uppercase tracking-wider text-text-dim/70">{t("providers.models")}</p>
            </div>
            <p className="text-xl font-black text-text-main">{p.model_count ?? 0}</p>
          </div>
          <div className="p-3 rounded-xl bg-gradient-to-br from-main/60 to-main/30 border border-border-subtle/50">
            <div className="flex items-center gap-1.5 mb-1">
              <Clock className="w-3 h-3 text-warning" />
              <p className="text-[9px] font-black uppercase tracking-wider text-text-dim/70">{t("providers.latency")}</p>
            </div>
            <p className={`text-xl font-black ${getLatencyColor(p.latency_ms)}`}>
              {p.latency_ms != null ? `${p.latency_ms}ms` : "-"}
            </p>
          </div>
        </div>

        {/* Media capabilities */}
        {p.media_capabilities && p.media_capabilities.length > 0 && (
          <div className="flex flex-wrap gap-1 mb-3">
            {p.media_capabilities.map((cap: string) => (
              <Badge key={cap} variant="default" className="text-[8px] px-1.5 py-0.5">
                {cap.replace(/_/g, " ")}
              </Badge>
            ))}
          </div>
        )}

        {/* Info */}
        <div className="space-y-1.5 mb-4 flex-1">
          {p.base_url && (
            <div className="flex items-center gap-2 text-xs">
              <Globe className="w-3 h-3 text-text-dim/50 shrink-0" />
              <span className="text-text-dim truncate font-mono text-[10px]">{p.base_url}</span>
            </div>
          )}
          {p.api_key_env && (
            <div className="flex items-center gap-2 text-xs">
              <Key className="w-3 h-3 text-text-dim/50 shrink-0" />
              <span className="text-text-dim font-mono text-[10px]">{p.api_key_env}</span>
            </div>
          )}
          <div className="flex items-center gap-2 text-xs">
            {isConfigured ? (
              p.reachable === true ? (
                <>
                  <CheckCircle2 className="w-3 h-3 text-success shrink-0" />
                  <span className="text-success font-bold text-[10px]">{t("providers.reachable")}</span>
                </>
              ) : p.reachable === false ? (
                <>
                  <XCircle className="w-3 h-3 text-error shrink-0" />
                  <span className="text-error font-bold text-[10px]">{t("providers.unreachable")}</span>
                </>
              ) : (
                <span className="text-text-dim font-bold text-[10px]">{t("providers.not_checked")}</span>
              )
            ) : (
              <>
                <AlertCircle className="w-3 h-3 text-text-dim/50 shrink-0" />
                <span className="text-text-dim font-bold text-[10px]">{t("providers.require_config")}</span>
              </>
            )}
          </div>
          {p.last_tested && (
            <div className="flex items-center gap-2 text-xs">
              <Activity className="w-3 h-3 text-text-dim/50 shrink-0" />
              <span className="text-text-dim font-mono text-[10px]">
                {t("providers.last_test")}: {formatTime(p.last_tested)}
              </span>
            </div>
          )}
          {p.error_message && (
            <div className="flex items-center gap-2 text-xs text-error">
              <AlertCircle className="w-3 h-3 shrink-0" />
              <span className="text-[10px] truncate">{p.error_message}</span>
            </div>
          )}
        </div>

        {/* Default status */}
        <div className="mb-2">
          {isDefault ? (
            <Badge variant="brand">
              <Star className="w-3 h-3 mr-1 inline" />{t("providers.is_default")}
            </Badge>
          ) : isConfigured ? (
            <button onClick={() => onSetDefault(p.id)} className="inline-flex items-center gap-1 text-[10px] font-bold text-brand/70 hover:text-brand cursor-pointer transition-colors">
              <Star className="w-3 h-3" />{t("providers.set_as_default")}
            </button>
          ) : null}
        </div>

        {/* Actions */}
        <div className="flex gap-2 mt-auto">
          {!isConfigured && (
            <Button variant="ghost" size="sm" onClick={() => onQuickConfig(p)} leftIcon={<Key className="w-3 h-3" />} className="flex-1 whitespace-nowrap">
              {t("providers.config")}
            </Button>
          )}
          {isConfigured && (
            <Button variant="ghost" size="sm" onClick={() => onEdit(p)} leftIcon={<Pencil className="w-3 h-3" />}>
              {t("common.edit")}
            </Button>
          )}
          {isConfigured && (
            <Button variant="ghost" size="sm" onClick={() => onDelete(p)} leftIcon={<Trash2 className="w-3 h-3 text-error" />}>
              {t("common.delete")}
            </Button>
          )}
          <Button
            variant="secondary"
            size="sm"
            onClick={() => onTest(p.id)}
            disabled={pendingId === p.id}
            leftIcon={pendingId === p.id ? <Loader2 className="w-3 h-3 animate-spin" /> : <Zap className="w-3 h-3" />}
            className="flex-1 whitespace-nowrap"
          >
            {pendingId === p.id ? t("providers.analyzing") : t("providers.test")}
          </Button>
        </div>
      </div>
    </Card>
  );
}

// Details Modal
function DetailsModal({ provider, onClose, onTest, pendingId, t }: {
  provider: Provider;
  onClose: () => void;
  onTest: (id: string) => void;
  pendingId: string | null;
  t: (key: string) => string
}) {
  const isConfigured = isProviderAvailable(provider.auth_status);

  return (
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center p-0 sm:p-4 bg-black/50 backdrop-blur-sm" onClick={onClose}>
      <div className="bg-surface rounded-2xl border border-border-subtle w-full sm:max-w-lg shadow-2xl rounded-t-2xl sm:rounded-2xl max-h-[90vh] overflow-y-auto animate-fade-in-scale" onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className={`h-2 bg-gradient-to-r ${isConfigured ? "from-success via-success/60 to-success/30" : "from-brand via-brand/60 to-brand/30"} rounded-t-2xl`} />
        <div className="p-6 border-b border-border-subtle">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className={`w-12 h-12 rounded-xl flex items-center justify-center text-2xl ${isConfigured ? "bg-success/10 border border-success/20" : "bg-brand/10 border border-brand/20"}`}>
                {getProviderIcon(provider.id)}
              </div>
              <div>
                <h2 className="text-xl font-black">{provider.display_name || provider.id}</h2>
                <p className="text-xs font-black uppercase tracking-widest text-text-dim/60">{provider.id}</p>
              </div>
            </div>
            <button onClick={onClose} className="p-2 hover:bg-main/30 rounded-lg transition-colors">
              <X className="w-5 h-5 text-text-dim" />
            </button>
          </div>
        </div>

        {/* Content */}
        <div className="p-6 space-y-4">
          <div className="grid grid-cols-2 gap-4">
            <div className="p-4 rounded-xl bg-main/30">
              <p className="text-[10px] font-black uppercase tracking-wider text-text-dim/70 mb-1">{t("providers.models")}</p>
              <p className="text-2xl font-black">{provider.model_count ?? 0}</p>
            </div>
            <div className="p-4 rounded-xl bg-main/30">
              <p className="text-[10px] font-black uppercase tracking-wider text-text-dim/70 mb-1">{t("providers.latency")}</p>
              <p className={`text-2xl font-black ${getLatencyColor(provider.latency_ms)}`}>
                {provider.latency_ms ? `${provider.latency_ms}ms` : "-"}
              </p>
            </div>
          </div>

          <div className="space-y-3">
            <h3 className="text-xs font-black uppercase tracking-wider text-text-dim">{t("common.properties")}</h3>
            <div className="space-y-2">
              {provider.base_url && (
                <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                  <span className="text-xs font-bold text-text-dim">{t("providers.base_url")}</span>
                  <span className="text-xs font-mono text-text-main truncate max-w-[200px]">{provider.base_url}</span>
                </div>
              )}
              {provider.api_key_env && (
                <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                  <span className="text-xs font-bold text-text-dim">{t("providers.api_key")}</span>
                  <span className="text-xs font-mono text-text-main">{provider.api_key_env}</span>
                </div>
              )}
              <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                <span className="text-xs font-bold text-text-dim">{t("common.status")}</span>
                <Badge variant={isConfigured ? "success" : "warning"}>
                  {isConfigured ? t("common.active") : t("common.setup")}
                </Badge>
              </div>
              <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                <span className="text-xs font-bold text-text-dim">{t("providers.health")}</span>
                {provider.reachable !== undefined ? (
                  <Badge variant={provider.reachable === true ? "success" : "error"}>
                    {provider.reachable === true ? t("providers.reachable") : t("providers.unreachable")}
                  </Badge>
                ) : <Badge variant="default">{t("providers.not_checked")}</Badge>}
              </div>
              {provider.key_required !== undefined && (
                <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                  <span className="text-xs font-bold text-text-dim">{t("providers.key_required")}</span>
                  <span className="text-xs font-bold">{provider.key_required ? t("common.yes") : t("common.no")}</span>
                </div>
              )}
              {provider.last_tested && (
                <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                  <span className="text-xs font-bold text-text-dim">{t("providers.last_test")}</span>
                  <span className="text-xs font-mono text-text-main">{formatDateTime(provider.last_tested)}</span>
                </div>
              )}
            </div>
          </div>

          {provider.error_message && (
            <div className="p-4 rounded-xl bg-error/10 border border-error/20">
              <h3 className="text-xs font-black uppercase tracking-wider text-error mb-2">{t("providers.error")}</h3>
              <p className="text-xs font-mono text-error">{provider.error_message}</p>
            </div>
          )}

          {/* Quick Actions */}
          <div className="flex gap-2 pt-2">
            <Button
              variant="primary"
              className="flex-1"
              onClick={() => onTest(provider.id)}
              disabled={pendingId === provider.id}
              leftIcon={pendingId === provider.id ? <Loader2 className="w-4 h-4 animate-spin" /> : <Zap className="w-4 h-4" />}
            >
              {pendingId === provider.id ? t("providers.analyzing") : t("providers.test_connection")}
            </Button>
            <Button variant="secondary" leftIcon={<ExternalLink className="w-4 h-4" />}>
              {t("providers.open_settings")}
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

// Filter Chips
function FilterChips({ activeFilter, onChange, t }: {
  activeFilter: FilterStatus;
  onChange: (filter: FilterStatus) => void;
  t: (key: string) => string;
}) {
  const filters: { value: FilterStatus; label: string; icon: React.ReactNode }[] = [
    { value: "all", label: t("providers.filter_all"), icon: <Filter className="w-3 h-3" /> },
    { value: "reachable", label: t("providers.filter_reachable"), icon: <CheckCircle2 className="w-3 h-3 text-success" /> },
    { value: "unreachable", label: t("providers.filter_unreachable"), icon: <XCircle className="w-3 h-3 text-error" /> },
  ];

  return (
    <div className="flex gap-1 p-1 bg-main/30 rounded-lg">
      {filters.map(f => (
        <button
          key={f.value}
          onClick={() => onChange(f.value)}
          className={`flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${
            activeFilter === f.value
              ? "bg-surface shadow-sm text-text-main"
              : "text-text-dim hover:text-text-main"
          }`}
        >
          {f.icon}
          {f.label}
        </button>
      ))}
    </div>
  );
}

type TabType = "configured" | "unconfigured";

export function ProvidersPage() {
  const { t } = useTranslation();
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<TabType>("configured");
  const [search, setSearch] = useState("");
  const [sortField, setSortField] = useState<SortField>("name");
  const [sortOrder, setSortOrder] = useState<SortOrder>("asc");
  const [viewMode, setViewMode] = useState<ViewMode>("grid");
  const [filterStatus, setFilterStatus] = useState<FilterStatus>("all");
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [detailsProvider, setDetailsProvider] = useState<Provider | null>(null);
  const [configProvider, setConfigProvider] = useState<Provider | null>(null);
  const [showCreateForm, setShowCreateForm] = useState(false);
  useCreateShortcut(() => setShowCreateForm(true));
  const [keyInput, setKeyInput] = useState("");
  const [urlInput, setUrlInput] = useState("");
  const [keySaving, setKeySaving] = useState(false);
  const [keyError, setKeyError] = useState<string | null>(null);
  const [keyTesting, setKeyTesting] = useState(false);
  const [keyTestResult, setKeyTestResult] = useState<{ ok: boolean; message: string } | null>(null);
  const [deleteConfirmProvider, setDeleteConfirmProvider] = useState<Provider | null>(null);
  const addToast = useUIStore((s) => s.addToast);

  const providersQuery = useQuery({ queryKey: ["providers", "list"], queryFn: listProviders, refetchInterval: REFRESH_MS });
  const statusQuery = useQuery({ queryKey: ["status"], queryFn: () => fetch("/api/status").then(r => r.json()) as Promise<{ default_provider?: string }>, refetchInterval: REFRESH_MS });
  const testMutation = useMutation({ mutationFn: testProvider });
  const defaultProviderMutation = useMutation({ mutationFn: setDefaultProvider });

  const providers = providersQuery.data ?? [];
  const currentDefaultProvider = statusQuery.data?.default_provider ?? "";
  const configuredCount = useMemo(() => providers.filter(p => isProviderAvailable(p.auth_status)).length, [providers]);
  const unconfiguredCount = useMemo(() => providers.filter(p => !isProviderAvailable(p.auth_status)).length, [providers]);

  // Filter, search, and sort
  const filteredProviders = useMemo(
    () => [...providers]
      .filter(p => {
        const tabMatch = activeTab === "configured" ? isProviderAvailable(p.auth_status) : !isProviderAvailable(p.auth_status);
        const searchMatch = !search || (p.display_name || p.id).toLowerCase().includes(search.toLowerCase()) || p.id.toLowerCase().includes(search.toLowerCase());

        let statusMatch = true;
        if (filterStatus === "reachable") statusMatch = p.reachable === true;
        else if (filterStatus === "unreachable") statusMatch = p.reachable === false;

        return tabMatch && searchMatch && statusMatch;
      })
      .sort((a, b) => {
        // CLI providers always sort after non-CLI
        const aCli = a.auth_status === "configured_cli" || a.auth_status === "cli_not_installed" || (!a.base_url && !a.key_required) ? 1 : 0;
        const bCli = b.auth_status === "configured_cli" || b.auth_status === "cli_not_installed" || (!b.base_url && !b.key_required) ? 1 : 0;
        if (aCli !== bCli) return aCli - bCli;
        let cmp = 0;
        if (sortField === "name") cmp = a.id.localeCompare(b.id);
        else if (sortField === "models") cmp = (a.model_count ?? 0) - (b.model_count ?? 0);
        else if (sortField === "latency") cmp = (a.latency_ms ?? 0) - (b.latency_ms ?? 0);
        return sortOrder === "asc" ? cmp : -cmp;
      }),
    [providers, activeTab, search, filterStatus, sortField, sortOrder],
  );

  const paginatedProviders = filteredProviders;

  // Reset page when filters change
  const handleTabChange = (tab: TabType) => {
    setActiveTab(tab);
    setSelectedIds(new Set());
    setFilterStatus("all");
  };

  const handleSearch = (value: string) => {
    setSearch(value);
    setSelectedIds(new Set());
  };

  const handleFilterChange = (filter: FilterStatus) => {
    setFilterStatus(filter);
    setSelectedIds(new Set());
  };

  const handleSort = (field: SortField) => {
    if (sortField === field) {
      setSortOrder(sortOrder === "asc" ? "desc" : "asc");
    } else {
      setSortField(field);
      setSortOrder("desc");
    }
  };

  const handleSelect = (id: string, checked: boolean) => {
    setSelectedIds(prev => {
      const next = new Set(prev);
      if (checked) next.add(id);
      else next.delete(id);
      return next;
    });
  };

  const handleSelectAll = () => {
    if (selectedIds.size === paginatedProviders.length) {
      setSelectedIds(new Set());
    } else {
      setSelectedIds(new Set(paginatedProviders.map(p => p.id)));
    }
  };

  const handleBatchTest = async () => {
    const ids = Array.from(selectedIds);
    for (const id of ids) {
      setPendingId(id);
      try {
        await testMutation.mutateAsync(id);
      } catch (e: any) {
        // Continue testing others
      }
    }
    setPendingId(null);
    addToast(t("common.success"), "success");
    void providersQuery.refetch();
  };

  const handleTest = async (id: string) => {
    setPendingId(id);
    try {
      const result = await testMutation.mutateAsync(id);
      if (result.status === "error") {
        addToast(String(result.error_message || result.error || t("common.error")), "error");
      } else {
        addToast(t("common.success"), "success");
      }
      await providersQuery.refetch();
    } catch (e: any) {
      addToast(e.message || t("common.error"), "error");
      await providersQuery.refetch();
    } finally {
      setPendingId(null);
    }
  };

  const handleQuickConfig = (provider: Provider) => {
    setConfigProvider(provider);
    setKeyInput("");
    setUrlInput(provider.base_url || "");
    setKeyError(null);
    setKeyTestResult(null);
  };

  const handleSaveKey = async () => {
    if (!configProvider) return;
    setKeySaving(true);
    setKeyError(null);
    try {
      if (urlInput.trim() && urlInput !== configProvider.base_url) {
        await setProviderUrl(configProvider.id, urlInput.trim());
      }
      if (keyInput.trim()) {
        await setProviderKey(configProvider.id, keyInput.trim());
      }
      await providersQuery.refetch();
      setConfigProvider(null);
      addToast(t("providers.key_saved"), "success");
    } catch (e: any) {
      setKeyError(e?.message || String(e));
    } finally {
      setKeySaving(false);
    }
  };

  const handleDeleteKey = async () => {
    if (!configProvider) return;
    setKeySaving(true);
    try {
      await deleteProviderKey(configProvider.id);
      await providersQuery.refetch();
      setConfigProvider(null);
      addToast(t("providers.key_removed"), "success");
    } catch (e: any) {
      setKeyError(e?.message || String(e));
    } finally {
      setKeySaving(false);
    }
  };

  const handleEdit = (provider: Provider) => handleQuickConfig(provider);

  const handleDeleteConfirm = async () => {
    if (!deleteConfirmProvider) return;
    setKeySaving(true);
    try {
      await deleteProviderKey(deleteConfirmProvider.id);
      await providersQuery.refetch();
      setDeleteConfirmProvider(null);
      addToast(t("providers.key_removed"), "success");
    } catch (e: any) {
      addToast(e?.message || t("common.error"), "error");
    } finally {
      setKeySaving(false);
    }
  };

  const handleTestKey = async () => {
    if (!configProvider) return;
    setKeyTesting(true);
    setKeyTestResult(null);
    try {
      // Save any pending key/url input before testing so the backend uses the new value
      if (keyInput.trim()) {
        await setProviderKey(configProvider.id, keyInput.trim());
        setKeyInput("");
      }
      if (urlInput.trim() && urlInput !== configProvider.base_url) {
        await setProviderUrl(configProvider.id, urlInput.trim());
      }
      const result = await testMutation.mutateAsync(configProvider.id);
      if (result.status === "error") {
        setKeyTestResult({ ok: false, message: String(result.error_message || result.error || t("providers.unreachable")) });
      } else {
        setKeyTestResult({ ok: true, message: t("providers.reachable") });
      }
      await providersQuery.refetch();
    } catch (e: any) {
      setKeyTestResult({ ok: false, message: e?.message || t("common.error") });
    } finally {
      setKeyTesting(false);
    }
  };

  const handleSetDefault = async (id: string) => {
    try {
      await defaultProviderMutation.mutateAsync(id);
      await statusQuery.refetch();
      addToast(t("providers.default_set"), "success");
    } catch (e: any) {
      addToast(e?.message || t("common.error"), "error");
    }
  };

  const allSelected = paginatedProviders.length > 0 && selectedIds.size === paginatedProviders.length;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("common.infrastructure")}
        title={t("providers.title")}
        subtitle={t("providers.subtitle")}
        isFetching={providersQuery.isFetching}
        onRefresh={() => void providersQuery.refetch()}
        icon={<Server className="h-4 w-4" />}
        helpText={t("providers.help")}
        actions={
          <div className="flex items-center gap-2">
            <Button variant="primary" size="sm" onClick={() => setShowCreateForm(true)} leftIcon={<Plus className="w-3.5 h-3.5" />}>
              {t("providers.add")}
            </Button>
            <div className="hidden rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase text-text-dim sm:block">
              {t("providers.configured_count", { configured: configuredCount, total: providers.length })}
            </div>
          </div>
        }
      />

      {/* Search & Controls */}
      <div className="flex flex-col sm:flex-row gap-3">
        <div className="flex-1">
          <Input
            value={search}
            onChange={(e) => handleSearch(e.target.value)}
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
              {sortField === "name" && (sortOrder === "asc" ? <SortAsc className="w-3 h-3" /> : <SortDesc className="w-3 h-3" />)}
              {t("providers.name")}
            </button>
            <button
              onClick={() => handleSort("models")}
              className={`flex items-center gap-1 px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${sortField === "models" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}
            >
              {sortField === "models" && (sortOrder === "asc" ? <SortAsc className="w-3 h-3" /> : <SortDesc className="w-3 h-3" />)}
              {t("providers.models")}
            </button>
            <button
              onClick={() => handleSort("latency")}
              className={`flex items-center gap-1 px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${sortField === "latency" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}
            >
              {sortField === "latency" && (sortOrder === "asc" ? <SortAsc className="w-3 h-3" /> : <SortDesc className="w-3 h-3" />)}
              {t("providers.latency")}
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

      {/* Tabs & Filter */}
      <div className="flex items-center justify-between gap-3 flex-wrap overflow-x-auto">
        <div className="flex gap-1 p-1 bg-main/30 rounded-xl w-fit">
          <button
            onClick={() => handleTabChange("configured")}
            className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-colors ${
              activeTab === "configured" ? "bg-surface text-success shadow-sm" : "text-text-dim hover:text-text-main"
            }`}
          >
            <CheckCircle2 className="w-4 h-4" />
            {t("providers.configured")}
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
            {t("providers.unconfigured")}
            <span className={`ml-1 px-1.5 py-0.5 rounded-full text-[10px] ${activeTab === "unconfigured" ? "bg-brand/20 text-brand" : "bg-border-subtle text-text-dim"}`}>
              {unconfiguredCount}
            </span>
          </button>
        </div>

        {/* Filter chips - only show for configured tab */}
        {activeTab === "configured" && (
          <FilterChips activeFilter={filterStatus} onChange={handleFilterChange} t={t} />
        )}

        {/* Batch actions */}
        {selectedIds.size > 0 && (
          <div className="flex items-center gap-2">
            <span className="text-xs font-bold text-text-dim">{selectedIds.size} selected</span>
            <Button variant="secondary" size="sm" onClick={handleBatchTest} leftIcon={<Zap className="w-3 h-3" />}>
              {t("providers.batch_test")}
            </Button>
          </div>
        )}
      </div>

      {providersQuery.isLoading ? (
        <div className={viewMode === "grid" ? "grid gap-4 md:grid-cols-2 xl:grid-cols-3" : "flex flex-col gap-2"}>
          {[1, 2, 3, 4, 5, 6].map((i) => <CardSkeleton key={i} />)}
        </div>
      ) : providers.length === 0 ? (
        <EmptyState title={t("common.no_data")} icon={<Server className="h-6 w-6" />} />
      ) : filteredProviders.length === 0 ? (
        <EmptyState
          title={search || filterStatus !== "all" ? t("providers.no_results") : (activeTab === "configured" ? t("providers.no_configured") : t("providers.no_unconfigured"))}
          icon={<Search className="h-6 w-6" />}
        />
      ) : (
        <>
          {/* Select all */}
          <div className="flex items-center gap-2">
            <button
              onClick={handleSelectAll}
              className="flex items-center gap-2 text-xs font-bold text-text-dim hover:text-text-main transition-colors"
            >
              {allSelected ? <CheckSquare className="w-4 h-4 text-brand" /> : <Square className="w-4 h-4" />}
              {t("providers.select_all")}
            </button>
            {(search || filterStatus !== "all") && (
              <span className="text-xs text-text-dim">
                ({filteredProviders.length} {t("providers.results")})
              </span>
            )}
          </div>

          <div className={viewMode === "grid" ? "grid gap-4 md:grid-cols-2 xl:grid-cols-3" : "flex flex-col gap-2"}>
            {paginatedProviders.map((p) => (
              <ProviderCard
                key={p.id}
                provider={p}
                isSelected={selectedIds.has(p.id)}
                isDefault={p.id === currentDefaultProvider}
                pendingId={pendingId}
                viewMode={viewMode}
                onSelect={handleSelect}
                onTest={handleTest}
                onSetDefault={handleSetDefault}
                onViewDetails={setDetailsProvider}
                onQuickConfig={handleQuickConfig}
                onEdit={handleEdit}
                onDelete={setDeleteConfirmProvider}
                t={t}
              />
            ))}
          </div>
        </>
      )}

      {/* Details Modal */}
      {detailsProvider && (
        <DetailsModal
          provider={detailsProvider}
          onClose={() => setDetailsProvider(null)}
          onTest={handleTest}
          pendingId={pendingId}
          t={t}
        />
      )}

      {/* API Key Config Modal */}
      {configProvider && (
        <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/30 backdrop-blur-sm" onClick={() => setConfigProvider(null)}>
          <div className="bg-surface rounded-2xl shadow-2xl border border-border-subtle w-[440px] max-w-[90vw] animate-fade-in-scale" onClick={e => e.stopPropagation()}>
            <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle">
              <div className="flex items-center gap-2">
                <Key className="w-4 h-4 text-brand" />
                <h3 className="text-sm font-bold">{t("providers.configure_provider")}</h3>
              </div>
              <button onClick={() => setConfigProvider(null)} className="p-1 rounded hover:bg-main"><X className="w-4 h-4" /></button>
            </div>
            <div className="p-5 space-y-4">
              <div className="flex items-center gap-3 p-3 rounded-xl bg-main">
                <div className="w-10 h-10 rounded-xl bg-brand/10 flex items-center justify-center">
                  {providerIcons[configProvider.id] || <Server className="w-5 h-5 text-brand" />}
                </div>
                <div>
                  <p className="text-sm font-bold">{configProvider.display_name || configProvider.id}</p>
                  <p className="text-[10px] text-text-dim font-mono">{configProvider.id}</p>
                </div>
                <Badge variant={isProviderAvailable(configProvider.auth_status) ? "success" : "error"} className="ml-auto">
                  {configProvider.auth_status}
                </Badge>
              </div>

              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">API Key</label>
                <input type="password" value={keyInput} onChange={e => setKeyInput(e.target.value)}
                  placeholder={isProviderAvailable(configProvider.auth_status) ? t("providers.key_placeholder_existing") : t("providers.key_placeholder")}
                  className="mt-1 w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm font-mono outline-none focus:border-brand focus:ring-1 focus:ring-brand/20" />
              </div>

              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">Base URL <span className="normal-case font-normal text-text-dim/50">({t("providers.optional")})</span></label>
                <input type="text" value={urlInput} onChange={e => setUrlInput(e.target.value)}
                  placeholder="https://api.example.com/v1"
                  className="mt-1 w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm font-mono outline-none focus:border-brand focus:ring-1 focus:ring-brand/20" />
              </div>

              {keyError && (
                <div className="flex items-center gap-2 text-error text-xs">
                  <AlertCircle className="w-4 h-4 shrink-0" />
                  {keyError}
                </div>
              )}

              {keyTestResult && (
                <div className={`flex items-center gap-2 text-xs p-3 rounded-xl ${keyTestResult.ok ? "bg-success/10 border border-success/20 text-success" : "bg-error/10 border border-error/20 text-error"}`}>
                  {keyTestResult.ok ? <CheckCircle2 className="w-4 h-4 shrink-0" /> : <XCircle className="w-4 h-4 shrink-0" />}
                  {keyTestResult.message}
                </div>
              )}

              <div className="flex gap-2 pt-2">
                <Button variant="primary" className="flex-1" onClick={handleSaveKey} disabled={keySaving || keyTesting || (!keyInput.trim() && urlInput === (configProvider.base_url || ""))}>
                  {keySaving ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Key className="w-4 h-4 mr-1" />}
                  {t("common.save")}
                </Button>
                <Button variant="secondary" onClick={handleTestKey} disabled={keySaving || keyTesting || (!isProviderAvailable(configProvider.auth_status) && !keyInput.trim())}>
                  {keyTesting ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Zap className="w-4 h-4 mr-1" />}
                  {t("providers.test")}
                </Button>
                {(configProvider.auth_status === "configured" || configProvider.auth_status === "validated_key") && (
                  <Button variant="secondary" onClick={handleDeleteKey} disabled={keySaving || keyTesting}>
                    <XCircle className="w-4 h-4 mr-1 text-error" />
                    {t("providers.remove_key")}
                  </Button>
                )}
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Delete Confirmation Modal */}
      {deleteConfirmProvider && (
        <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/30 backdrop-blur-sm" onClick={() => setDeleteConfirmProvider(null)}>
          <div className="bg-surface rounded-2xl shadow-2xl border border-border-subtle w-[400px] max-w-[90vw] animate-fade-in-scale" onClick={e => e.stopPropagation()}>
            <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle">
              <div className="flex items-center gap-2">
                <Trash2 className="w-4 h-4 text-error" />
                <h3 className="text-sm font-bold">{t("providers.delete_confirm_title")}</h3>
              </div>
              <button onClick={() => setDeleteConfirmProvider(null)} className="p-1 rounded hover:bg-main"><X className="w-4 h-4" /></button>
            </div>
            <div className="p-5 space-y-4">
              <div className="flex items-center gap-3 p-3 rounded-xl bg-main">
                <div className="w-10 h-10 rounded-xl bg-error/10 flex items-center justify-center">
                  {providerIcons[deleteConfirmProvider.id] || <Server className="w-5 h-5 text-error" />}
                </div>
                <div>
                  <p className="text-sm font-bold">{deleteConfirmProvider.display_name || deleteConfirmProvider.id}</p>
                  <p className="text-[10px] text-text-dim font-mono">{deleteConfirmProvider.id}</p>
                </div>
              </div>
              <p className="text-sm text-text-dim">{t("providers.delete_confirm_message")}</p>
              <div className="flex gap-2 pt-2">
                <Button variant="ghost" className="flex-1" onClick={() => setDeleteConfirmProvider(null)}>
                  {t("common.cancel")}
                </Button>
                <Button variant="primary" className="flex-1 !bg-error hover:!bg-error/80" onClick={handleDeleteConfirm} disabled={keySaving}>
                  {keySaving ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Trash2 className="w-4 h-4 mr-1" />}
                  {t("common.delete")}
                </Button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Create Provider Modal */}
      {showCreateForm && (
        <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/30 backdrop-blur-sm" onClick={() => setShowCreateForm(false)}>
          <div className="bg-surface rounded-2xl shadow-2xl border border-border-subtle w-[540px] max-w-[90vw] max-h-[85vh] overflow-y-auto animate-fade-in-scale" onClick={e => e.stopPropagation()}>
            <SchemaForm
              contentType="provider"
              title={t("providers.add")}
              submitLabel={t("common.create")}
              onSubmit={async (values) => {
                await createRegistryContent("provider", values);
                setShowCreateForm(false);
                void providersQuery.refetch();
              }}
              onCancel={() => setShowCreateForm(false)}
            />
          </div>
        </div>
      )}
    </div>
  );
}
