import { useMutation, useQuery } from "@tanstack/react-query";
import { formatTime, formatDateTime } from "../lib/datetime";
import { useMemo, useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { listProviders, listModels, testProvider, setProviderKey, deleteProviderKey, setProviderUrl, createRegistryContent, setDefaultProvider, getStatus } from "../api";
import type { ProviderItem } from "../api";
import { isProviderAvailable } from "../lib/status";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge, type BadgeVariant } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { Select } from "../components/ui/Select";
import { Modal } from "../components/ui/Modal";
import { useUIStore } from "../lib/store";
import { useCreateShortcut } from "../lib/useCreateShortcut";
import {
  Server, Zap, Clock, Key, Globe, CheckCircle2, XCircle, Loader2, AlertCircle, Search,
  SortAsc, SortDesc, CheckSquare, Square, ChevronRight, X, Grid3X3, List, Filter,
  Activity, Cpu, Cloud, Bot, Globe2, Sparkles, Plus, Star, Pencil, Trash2,
  Check, ChevronLeft
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

function getAuthBadge(status?: string): { variant: BadgeVariant; label: string } {
  switch (status) {
    case "configured":
    case "validated_key":
      return { variant: "success", label: "KEY" };
    case "configured_cli":
      return { variant: "default", label: "CLI" };
    case "auto_detected":
      return { variant: "warning", label: "AUTO" };
    case "not_required":
      return { variant: "success", label: "LOCAL" };
    case "invalid_key":
      return { variant: "error", label: "INVALID" };
    case "cli_not_installed":
      return { variant: "error", label: "CLI N/A" };
    case "missing":
    default:
      return { variant: "warning", label: "SETUP" };
  }
}

type SortField = "name" | "models" | "latency";
type SortOrder = "asc" | "desc";
type ViewMode = "grid" | "list";
type FilterStatus = "all" | "reachable" | "unreachable";

// ── SetDefaultModelSection — model picker + "set as default" in config modal ──

function SetDefaultModelSection({ providerId, currentDefault, onSetDefault, t }: {
  providerId: string;
  currentDefault?: string;
  onSetDefault: (id: string, model?: string) => Promise<void>;
  t: (key: string, opts?: any) => string;
}) {
  const [selectedModel, setSelectedModel] = useState("");
  const [setting, setSetting] = useState(false);
  const isDefault = currentDefault === providerId;

  const modelsQuery = useQuery({
    queryKey: ["models", "provider", providerId, { available: true }],
    queryFn: () => listModels({ provider: providerId, available: true }),
    staleTime: 60_000,
  });

  const models = modelsQuery.data?.models || [];

  const handleSetDefault = async () => {
    setSetting(true);
    try {
      await onSetDefault(providerId, selectedModel || undefined);
    } finally {
      setSetting(false);
    }
  };

  return (
    <div className="border-t border-border-subtle pt-3 mt-1 space-y-2">
      <label className="text-[10px] font-bold text-text-dim uppercase">{t("providers.set_as_default")}</label>
      {modelsQuery.isLoading ? (
        <div className="w-full h-10 rounded-xl bg-bg-subtle animate-pulse" />
      ) : models.length > 0 ? (
        <select
          value={selectedModel}
          onChange={e => setSelectedModel(e.target.value)}
          className="w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand focus:ring-1 focus:ring-brand/20"
        >
          <option value="">{t("providers.auto_select_model")}</option>
          {models.map(m => (
            <option key={m.id} value={m.id}>{m.display_name || m.id}</option>
          ))}
        </select>
      ) : (
        <input
          type="text"
          value={selectedModel}
          onChange={e => setSelectedModel(e.target.value)}
          placeholder={t("providers.model_name_placeholder")}
          className="w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm font-mono outline-none focus:border-brand focus:ring-1 focus:ring-brand/20"
        />
      )}
      <Button
        variant={isDefault ? "ghost" : "secondary"}
        className="w-full"
        onClick={handleSetDefault}
        disabled={setting || isDefault}
      >
        {setting ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Star className="w-4 h-4 mr-1" />}
        {isDefault ? t("providers.is_default") : t("providers.set_as_default")}
      </Button>
    </div>
  );
}

// ── useProviderConfig hook ────────────────────────────────────────

interface ProviderConfigState {
  provider: ProviderItem | null;
  keyInput: string;
  urlInput: string;
  proxyInput: string;
  hasStoredKey: boolean;
  saving: boolean;
  error: string | null;
  testing: boolean;
  testResult: { ok: boolean; message: string } | null;
}

function useProviderConfig(
  refetchProviders: () => void,
  testMutation: ReturnType<typeof useMutation<any, any, string>>,
  addToast: (msg: string, type?: "success" | "error" | "info") => void,
  t: (key: string, opts?: any) => string,
  activeTab: string,
  setActiveTab: (tab: "configured" | "unconfigured") => void,
) {
  const [state, setState] = useState<ProviderConfigState>({
    provider: null, keyInput: "", urlInput: "", proxyInput: "", hasStoredKey: false,
    saving: false, error: null, testing: false, testResult: null,
  });

  const open = useCallback((p: ProviderItem) => {
    setState({
      provider: p, keyInput: "", urlInput: p.base_url || "", proxyInput: p.proxy_url || "",
      hasStoredKey: p.auth_status === "configured" || p.auth_status === "validated_key" || p.auth_status === "invalid_key" || p.auth_status === "auto_detected",
      saving: false, error: null, testing: false, testResult: null,
    });
  }, []);

  const close = useCallback(() => setState(s => ({ ...s, provider: null })), []);

  const setKeyInput = useCallback((v: string) => setState(s => ({ ...s, keyInput: v })), []);
  const setUrlInput = useCallback((v: string) => setState(s => ({ ...s, urlInput: v })), []);
  const setProxyInput = useCallback((v: string) => setState(s => ({ ...s, proxyInput: v })), []);

  const saveKey = useCallback(async () => {
    if (!state.provider) return;
    setState(s => ({ ...s, saving: true, error: null }));
    try {
      const urlChanged = state.urlInput.trim() && state.urlInput !== state.provider.base_url;
      const proxyChanged = state.proxyInput !== (state.provider.proxy_url || "");
      if (urlChanged || proxyChanged) {
        await setProviderUrl(state.provider.id, state.urlInput.trim() || state.provider.base_url || "", proxyChanged ? state.proxyInput.trim() : undefined);
      }
      if (state.keyInput.trim()) {
        await setProviderKey(state.provider.id, state.keyInput.trim());
      }
      refetchProviders();
      setState(s => ({ ...s, provider: null }));
      if (activeTab === "unconfigured") setActiveTab("configured");
      addToast(t("providers.key_saved"), "success");
    } catch (e: any) {
      setState(s => ({ ...s, error: e?.message || String(e) }));
    } finally {
      setState(s => ({ ...s, saving: false }));
    }
  }, [state.provider, state.keyInput, state.urlInput, state.proxyInput, refetchProviders, addToast, t, activeTab, setActiveTab]);

  const removeKey = useCallback(async () => {
    if (!state.provider) return;
    setState(s => ({ ...s, saving: true }));
    try {
      await deleteProviderKey(state.provider.id);
      refetchProviders();
      setState(s => ({ ...s, provider: null, hasStoredKey: false }));
      addToast(t("providers.key_removed"), "success");
    } catch (e: any) {
      setState(s => ({ ...s, error: e?.message || String(e) }));
    } finally {
      setState(s => ({ ...s, saving: false }));
    }
  }, [state.provider, refetchProviders, addToast, t]);

  const testKey = useCallback(async () => {
    if (!state.provider) return;
    setState(s => ({ ...s, testing: true, testResult: null }));
    try {
      if (state.keyInput.trim()) {
        await setProviderKey(state.provider.id, state.keyInput.trim());
        setState(s => ({ ...s, hasStoredKey: true, keyInput: "" }));
      }
      const urlChanged = state.urlInput.trim() && state.urlInput !== state.provider.base_url;
      const proxyChanged = state.proxyInput !== (state.provider.proxy_url || "");
      if (urlChanged || proxyChanged) {
        await setProviderUrl(state.provider.id, state.urlInput.trim() || state.provider.base_url || "", proxyChanged ? state.proxyInput.trim() : undefined);
      }
      const result = await testMutation.mutateAsync(state.provider.id);
      if (result.status === "error") {
        setState(s => ({ ...s, testResult: { ok: false, message: String(result.error_message || result.error || t("providers.unreachable")) } }));
      } else {
        setState(s => ({ ...s, testResult: { ok: true, message: t("providers.reachable") } }));
      }
      refetchProviders();
    } catch (e: any) {
      setState(s => ({ ...s, testResult: { ok: false, message: e?.message || t("common.error") } }));
    } finally {
      setState(s => ({ ...s, testing: false }));
    }
  }, [state.provider, state.keyInput, state.urlInput, state.proxyInput, testMutation, refetchProviders, t]);

  return { ...state, open, close, setKeyInput, setUrlInput, setProxyInput, saveKey, removeKey, testKey };
}

// ── ProviderCard ─────────────────────────────────────────────────

interface ProviderCardProps {
  provider: ProviderItem;
  isSelected: boolean;
  isDefault: boolean;
  pendingId: string | null;
  viewMode: ViewMode;
  onSelect: (id: string, checked: boolean) => void;
  onTest: (id: string) => void;
  onSetDefault: (id: string) => void;
  onViewDetails: (provider: ProviderItem) => void;
  onConfigure: (provider: ProviderItem) => void;
  onDelete: (provider: ProviderItem) => void;
  t: (key: string) => string;
}

function ProviderCard({ provider: p, isSelected, isDefault, pendingId, viewMode, onSelect, onTest, onSetDefault, onViewDetails, onConfigure, onDelete, t }: ProviderCardProps) {
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
          {isConfigured ? (
            <>
              {!isDefault && (
                <Button variant="ghost" size="sm" onClick={() => onSetDefault(p.id)} leftIcon={<Star className="w-3 h-3" />}>
                  <span className="hidden sm:inline">{t("providers.set_as_default")}</span>
                </Button>
              )}
              <Button variant="ghost" size="sm" onClick={() => onConfigure(p)} leftIcon={<Pencil className="w-3 h-3" />}>
                <span className="hidden sm:inline">{t("common.edit")}</span>
              </Button>
              <Button variant="ghost" size="sm" onClick={() => onDelete(p)} leftIcon={<Trash2 className="w-3 h-3 text-error" />}>
                <span className="hidden sm:inline text-error">{p.is_custom ? t("common.delete") : t("providers.remove_key")}</span>
              </Button>
              <Button
                variant="secondary" size="sm"
                onClick={() => onTest(p.id)}
                disabled={pendingId === p.id}
                leftIcon={pendingId === p.id ? <Loader2 className="w-3 h-3 animate-spin" /> : <Zap className="w-3 h-3" />}
                className="whitespace-nowrap"
              >
                <span className="hidden sm:inline">{pendingId === p.id ? t("providers.analyzing") : t("providers.test")}</span>
              </Button>
            </>
          ) : (
            <Button variant="ghost" size="sm" onClick={() => onConfigure(p)} leftIcon={<Key className="w-3 h-3" />}>
              <span className="hidden sm:inline">{t("providers.config")}</span>
            </Button>
          )}
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
          {isConfigured ? (
            <>
              <Button variant="ghost" size="sm" onClick={() => onConfigure(p)} leftIcon={<Pencil className="w-3 h-3" />}>
                {t("common.edit")}
              </Button>
              <Button variant="ghost" size="sm" onClick={() => onDelete(p)} leftIcon={<Trash2 className="w-3 h-3 text-error" />}>
                {p.is_custom ? t("common.delete") : t("providers.remove_key")}
              </Button>
              <Button
                variant="secondary" size="sm"
                onClick={() => onTest(p.id)}
                disabled={pendingId === p.id}
                leftIcon={pendingId === p.id ? <Loader2 className="w-3 h-3 animate-spin" /> : <Zap className="w-3 h-3" />}
                className="flex-1 whitespace-nowrap"
              >
                {pendingId === p.id ? t("providers.analyzing") : t("providers.test")}
              </Button>
            </>
          ) : (
            <Button variant="ghost" size="sm" onClick={() => onConfigure(p)} leftIcon={<Key className="w-3 h-3" />} className="flex-1 whitespace-nowrap">
              {t("providers.config")}
            </Button>
          )}
        </div>
      </div>
    </Card>
  );
}

// ── Details Modal ────────────────────────────────────────────────

function DetailsModal({ provider, onClose, onTest, pendingId, t }: {
  provider: ProviderItem;
  onClose: () => void;
  onTest: (id: string) => void;
  pendingId: string | null;
  t: (key: string, opts?: any) => string;
}) {
  const isConfigured = isProviderAvailable(provider.auth_status);
  const authBadge = getAuthBadge(provider.auth_status);

  const modelsQuery = useQuery({
    queryKey: ["models", "provider", provider.id],
    queryFn: () => listModels({ provider: provider.id }),
  });
  const models = modelsQuery.data?.models ?? [];

  return (
    <Modal isOpen onClose={onClose} title={provider.display_name || provider.id} size="lg">
      <div className="p-6 space-y-4">
        {/* Header info */}
        <div className="flex items-center gap-3">
          <div className={`w-12 h-12 rounded-xl flex items-center justify-center text-2xl ${isConfigured ? "bg-success/10 border border-success/20" : "bg-brand/10 border border-brand/20"}`}>
            {getProviderIcon(provider.id)}
          </div>
          <div className="flex-1">
            <p className="text-xs font-black uppercase tracking-widest text-text-dim/60">{provider.id}</p>
          </div>
          <Badge variant={authBadge.variant}>{authBadge.label}</Badge>
        </div>

        {/* Stats */}
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

        {/* Properties */}
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
              <Badge variant={authBadge.variant}>{authBadge.label}</Badge>
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

        {/* Model list */}
        <div className="space-y-3">
          <h3 className="text-xs font-black uppercase tracking-wider text-text-dim">{t("providers.provider_models")}</h3>
          {modelsQuery.isLoading ? (
            <p className="text-xs text-text-dim">{t("common.loading")}</p>
          ) : models.length === 0 ? (
            <p className="text-xs text-text-dim">{t("providers.no_models_for_provider")}</p>
          ) : (
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-1.5 max-h-48 overflow-y-auto">
              {models.map(m => (
                <div key={m.id} className="flex items-center gap-2 p-2 rounded-lg bg-main/20 text-xs">
                  <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${m.available ? "bg-success" : "bg-text-dim/30"}`} />
                  <span className="truncate font-mono">{m.display_name || m.id}</span>
                </div>
              ))}
            </div>
          )}
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
            variant="primary" className="flex-1"
            onClick={() => onTest(provider.id)}
            disabled={pendingId === provider.id}
            leftIcon={pendingId === provider.id ? <Loader2 className="w-4 h-4 animate-spin" /> : <Zap className="w-4 h-4" />}
          >
            {pendingId === provider.id ? t("providers.analyzing") : t("providers.test_connection")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

// ── Filter Chips ─────────────────────────────────────────────────

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

// ── Create Provider Wizard ──────────────────────────────────────

interface ModelEntry {
  id: string;
  display_name: string;
  tier: string;
  context_window: number | "";
  max_output_tokens: number | "";
  input_cost_per_m: number | "";
  output_cost_per_m: number | "";
}

function toTitleCase(id: string): string {
  return id.split("-").map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join(" ");
}

function toEnvVar(id: string): string {
  return id.toUpperCase().replace(/-/g, "_") + "_API_KEY";
}

const EMPTY_MODEL: ModelEntry = {
  id: "", display_name: "", tier: "balanced",
  context_window: "", max_output_tokens: "",
  input_cost_per_m: "", output_cost_per_m: "",
};

const TIER_OPTIONS = [
  { value: "fast", label: "Fast" },
  { value: "balanced", label: "Balanced" },
  { value: "smart", label: "Smart" },
  { value: "reasoning", label: "Reasoning" },
];

function CreateProviderWizard({
  onSubmit,
  onCancel,
}: {
  onSubmit: (values: Record<string, unknown>) => Promise<void>;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const [step, setStep] = useState(0);
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  const [id, setId] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");

  const [displayName, setDisplayName] = useState("");
  const [apiKeyEnv, setApiKeyEnv] = useState("");
  const [keyRequired, setKeyRequired] = useState(true);
  const [derivedOverridden, setDerivedOverridden] = useState(false);

  const [models, setModels] = useState<ModelEntry[]>([]);
  const [errors, setErrors] = useState<string[]>([]);

  const derivedDisplayName = toTitleCase(id);
  const derivedApiKeyEnv = toEnvVar(id);
  const effectiveDisplayName = derivedOverridden ? displayName : derivedDisplayName;
  const effectiveApiKeyEnv = derivedOverridden ? apiKeyEnv : derivedApiKeyEnv;
  const effectiveKeyRequired = derivedOverridden ? keyRequired : apiKey.trim().length > 0;

  const steps = [
    t("providers.wizard_step_basics"),
    t("providers.wizard_step_advanced"),
    t("providers.wizard_step_models"),
  ];

  const validateStep0 = () => {
    const errs: string[] = [];
    if (!id.trim()) errs.push("id");
    if (!baseUrl.trim()) errs.push("base_url");
    setErrors(errs);
    return errs.length === 0;
  };

  const handleNext = () => {
    if (step === 0 && !validateStep0()) return;
    if (step === 0 && !derivedOverridden) {
      setDisplayName(derivedDisplayName);
      setApiKeyEnv(derivedApiKeyEnv);
      setKeyRequired(apiKey.trim().length > 0);
    }
    setStep((s) => Math.min(s + 1, 2));
    setErrors([]);
  };

  const handleBack = () => {
    setStep((s) => Math.max(s - 1, 0));
    setErrors([]);
  };

  const buildValues = () => {
    const values: Record<string, unknown> = {
      id: id.trim(),
      display_name: effectiveDisplayName,
      api_key_env: effectiveApiKeyEnv,
      base_url: baseUrl.trim(),
      key_required: effectiveKeyRequired,
    };
    if (apiKey.trim()) values.api_key = apiKey.trim();
    if (models.length > 0) {
      values.models = models
        .filter((m) => m.id.trim())
        .map((m) => ({
          id: m.id.trim(),
          display_name: m.display_name.trim() || m.id.trim(),
          tier: m.tier,
          context_window: typeof m.context_window === "number" ? m.context_window : 128000,
          max_output_tokens: typeof m.max_output_tokens === "number" ? m.max_output_tokens : 4096,
          input_cost_per_m: typeof m.input_cost_per_m === "number" ? m.input_cost_per_m : 0,
          output_cost_per_m: typeof m.output_cost_per_m === "number" ? m.output_cost_per_m : 0,
        }));
    }
    return values;
  };

  const handleCreate = async () => {
    setSubmitting(true);
    setSubmitError(null);
    try {
      await onSubmit(buildValues());
    } catch (err) {
      setSubmitError(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  };

  const updateModel = (idx: number, field: keyof ModelEntry, value: unknown) => {
    setModels((prev) => prev.map((m, i) => (i === idx ? { ...m, [field]: value } : m)));
  };

  const removeModel = (idx: number) => {
    setModels((prev) => prev.filter((_, i) => i !== idx));
  };

  return (
    <div>
      {/* Step indicator */}
      <div className="px-5 pt-4 pb-2">
        <div className="flex items-center gap-1">
          {steps.map((label, i) => (
            <button key={i} className="flex items-center gap-1 flex-1 group"
              onClick={() => { if (i < step) setStep(i); }} disabled={i > step}>
              <div className={`w-6 h-6 rounded-full flex items-center justify-center text-[10px] font-bold shrink-0 transition-colors ${
                i < step ? "bg-success text-white cursor-pointer" : i === step ? "bg-brand text-white" : "bg-main text-text-dim"
              }`}>
                {i < step ? <Check className="w-3 h-3" /> : i + 1}
              </div>
              <span className={`text-[10px] font-bold uppercase tracking-wider truncate ${i === step ? "text-text-main" : "text-text-dim"}`}>
                {label}
              </span>
              {i < steps.length - 1 && <div className={`flex-1 h-px ${i < step ? "bg-success" : "bg-border-subtle"}`} />}
            </button>
          ))}
        </div>
      </div>

      <div className="p-5 space-y-4">
        {/* Step 1: Basics */}
        {step === 0 && (
          <>
            <Input label={t("providers.wizard_id_label") + " *"} value={id}
              onChange={(e) => { setId(e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, "")); setErrors(prev => prev.filter(e2 => e2 !== "id")); }}
              placeholder={t("providers.wizard_id_placeholder")} className={errors.includes("id") ? "border-error" : ""} />
            {errors.includes("id") && <p className="text-[10px] text-error -mt-2">{t("providers.wizard_id_required")}</p>}
            <p className="text-[10px] text-text-dim/60 -mt-2">{t("providers.wizard_id_hint")}</p>

            <Input label={t("providers.wizard_base_url_label") + " *"} value={baseUrl}
              onChange={(e) => { setBaseUrl(e.target.value); setErrors(prev => prev.filter(e2 => e2 !== "base_url")); }}
              placeholder={t("providers.wizard_base_url_placeholder")} className={errors.includes("base_url") ? "border-error" : ""} />
            {errors.includes("base_url") && <p className="text-[10px] text-error -mt-2">{t("providers.wizard_base_url_required")}</p>}

            <Input label={t("providers.wizard_api_key_label")} type="password" value={apiKey}
              onChange={(e) => setApiKey(e.target.value)} placeholder={t("providers.wizard_api_key_placeholder")} />
            <p className="text-[10px] text-text-dim/60 -mt-2">{t("providers.wizard_api_key_hint")}</p>

            {id.trim() && (
              <div className="p-3 rounded-xl bg-main/40 border border-border-subtle/50 space-y-1.5">
                <p className="text-[9px] font-bold uppercase tracking-wider text-text-dim/60">{t("providers.wizard_auto_derived")}</p>
                <div className="flex items-center gap-2 text-xs">
                  <span className="text-text-dim w-24 shrink-0">{t("providers.wizard_display_name_label")}</span>
                  <span className="font-mono text-text-main">{derivedDisplayName}</span>
                </div>
                <div className="flex items-center gap-2 text-xs">
                  <span className="text-text-dim w-24 shrink-0">{t("providers.wizard_env_var")}</span>
                  <span className="font-mono text-text-main">{derivedApiKeyEnv}</span>
                </div>
              </div>
            )}
          </>
        )}

        {/* Step 2: Advanced */}
        {step === 1 && (
          <>
            <p className="text-[10px] text-text-dim/60">{t("providers.wizard_advanced_hint")}</p>
            <Input label={t("providers.wizard_display_name_label")} value={displayName}
              onChange={(e) => { setDisplayName(e.target.value); setDerivedOverridden(true); }} placeholder={derivedDisplayName} />
            <Input label={t("providers.wizard_api_key_env_label")} value={apiKeyEnv}
              onChange={(e) => { setApiKeyEnv(e.target.value); setDerivedOverridden(true); }} placeholder={derivedApiKeyEnv} />
            <div className="space-y-1">
              <label className="flex items-center gap-3 cursor-pointer">
                <button type="button" role="checkbox" aria-checked={keyRequired}
                  onClick={() => { setKeyRequired(!keyRequired); setDerivedOverridden(true); }}
                  className={`relative w-10 h-5 rounded-full transition-colors duration-200 shrink-0 ${keyRequired ? "bg-brand" : "bg-main border border-border-subtle"}`}>
                  <span className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform duration-200 ${keyRequired ? "translate-x-5" : "translate-x-0"}`} />
                </button>
                <span className="text-xs font-bold text-text-main">{t("providers.wizard_key_required_label")}</span>
              </label>
            </div>
          </>
        )}

        {/* Step 3: Models */}
        {step === 2 && (
          <>
            <p className="text-[10px] text-text-dim/60">{t("providers.wizard_models_hint")}</p>
            {models.map((m, idx) => (
              <div key={idx} className="p-3 rounded-xl border border-border-subtle/50 bg-main/20 space-y-3">
                <div className="flex items-center justify-between">
                  <span className="text-[10px] font-bold text-text-dim uppercase tracking-widest">#{idx + 1}</span>
                  <button onClick={() => removeModel(idx)} className="p-1 rounded hover:bg-error/10 text-text-dim hover:text-error transition-colors">
                    <Trash2 className="w-3.5 h-3.5" />
                  </button>
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <Input label={t("providers.wizard_model_id") + " *"} value={m.id} onChange={(e) => updateModel(idx, "id", e.target.value)} placeholder="gpt-4o" />
                  <Input label={t("providers.wizard_model_name")} value={m.display_name} onChange={(e) => updateModel(idx, "display_name", e.target.value)} placeholder="GPT-4o" />
                </div>
                <div className="grid grid-cols-3 gap-3">
                  <Select label={t("providers.wizard_model_tier")} options={TIER_OPTIONS} value={m.tier} onChange={(e) => updateModel(idx, "tier", e.target.value)} />
                  <Input label={t("providers.wizard_model_context")} type="number" value={m.context_window === "" ? "" : String(m.context_window)}
                    onChange={(e) => updateModel(idx, "context_window", e.target.value === "" ? "" : Number(e.target.value))} placeholder="128000" />
                  <Input label={t("providers.wizard_model_max_output")} type="number" value={m.max_output_tokens === "" ? "" : String(m.max_output_tokens)}
                    onChange={(e) => updateModel(idx, "max_output_tokens", e.target.value === "" ? "" : Number(e.target.value))} placeholder="4096" />
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <Input label={t("providers.wizard_model_input_cost")} type="number" value={m.input_cost_per_m === "" ? "" : String(m.input_cost_per_m)}
                    onChange={(e) => updateModel(idx, "input_cost_per_m", e.target.value === "" ? "" : Number(e.target.value))} placeholder="2.5" />
                  <Input label={t("providers.wizard_model_output_cost")} type="number" value={m.output_cost_per_m === "" ? "" : String(m.output_cost_per_m)}
                    onChange={(e) => updateModel(idx, "output_cost_per_m", e.target.value === "" ? "" : Number(e.target.value))} placeholder="10.0" />
                </div>
              </div>
            ))}
            <button type="button" onClick={() => setModels(prev => [...prev, { ...EMPTY_MODEL }])}
              className="w-full py-2 rounded-xl border border-dashed border-border-subtle text-xs font-bold text-text-dim hover:text-brand hover:border-brand transition-colors flex items-center justify-center gap-1.5">
              <Plus className="w-3.5 h-3.5" />
              {t("schema_form.add_item")}
            </button>
          </>
        )}

        {submitError && (
          <div className="flex items-center gap-2 text-error text-xs">
            <AlertCircle className="w-4 h-4 shrink-0" />
            {submitError}
          </div>
        )}

        {/* Navigation */}
        <div className="flex gap-2 pt-2">
          {step > 0 && (
            <Button variant="ghost" onClick={handleBack} disabled={submitting}>
              <ChevronLeft className="w-4 h-4 mr-1" />
              {t("providers.wizard_back")}
            </Button>
          )}
          <div className="flex-1" />
          {step < 2 && (
            <>
              {step === 1 && (
                <Button variant="ghost" onClick={handleCreate} disabled={submitting}>
                  {submitting ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : null}
                  {t("providers.wizard_skip_create")}
                </Button>
              )}
              <Button variant="primary" onClick={handleNext}>
                {t("providers.wizard_next")}
                <ChevronRight className="w-4 h-4 ml-1" />
              </Button>
            </>
          )}
          {step === 2 && (
            <>
              {models.length === 0 && (
                <Button variant="ghost" onClick={handleCreate} disabled={submitting}>
                  {submitting ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : null}
                  {t("providers.wizard_skip_create")}
                </Button>
              )}
              <Button variant="primary" onClick={handleCreate} disabled={submitting}>
                {submitting ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Plus className="w-4 h-4 mr-1" />}
                {submitting ? t("providers.wizard_creating") : t("providers.wizard_create")}
              </Button>
            </>
          )}
          <Button variant="secondary" onClick={onCancel} disabled={submitting}>
            {t("common.cancel")}
          </Button>
        </div>
      </div>
    </div>
  );
}

// ── Main Page ────────────────────────────────────────────────────

type TabType = "configured" | "unconfigured";

export function ProvidersPage() {
  const { t } = useTranslation();
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [testingIds, setTestingIds] = useState<Set<string>>(new Set());
  const [activeTab, setActiveTab] = useState<TabType>("configured");
  const [search, setSearch] = useState("");
  const [sortField, setSortField] = useState<SortField>("name");
  const [sortOrder, setSortOrder] = useState<SortOrder>("asc");
  const [viewMode, setViewMode] = useState<ViewMode>("grid");
  const [filterStatus, setFilterStatus] = useState<FilterStatus>("all");
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [detailsProvider, setDetailsProvider] = useState<ProviderItem | null>(null);
  const [showCreateForm, setShowCreateForm] = useState(false);
  useCreateShortcut(() => setShowCreateForm(true));
  const [deleteConfirmProvider, setDeleteConfirmProvider] = useState<ProviderItem | null>(null);
  const addToast = useUIStore((s) => s.addToast);

  const providersQuery = useQuery({ queryKey: ["providers", "list"], queryFn: listProviders, refetchInterval: REFRESH_MS });
  const statusQuery = useQuery({ queryKey: ["status"], queryFn: getStatus, refetchInterval: REFRESH_MS });
  const testMutation = useMutation({ mutationFn: testProvider });
  const defaultProviderMutation = useMutation({ mutationFn: ({ id, model }: { id: string; model?: string }) => setDefaultProvider(id, model) });

  const config = useProviderConfig(
    () => void providersQuery.refetch(),
    testMutation,
    addToast,
    t,
    activeTab,
    setActiveTab,
  );

  const providers = providersQuery.data ?? [];
  const currentDefaultProvider = statusQuery.data?.default_provider ?? "";
  const configuredCount = useMemo(() => providers.filter(p => isProviderAvailable(p.auth_status)).length, [providers]);
  const unconfiguredCount = useMemo(() => providers.filter(p => !isProviderAvailable(p.auth_status)).length, [providers]);

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

  const handleTabChange = (tab: TabType) => { setActiveTab(tab); setSelectedIds(new Set()); setFilterStatus("all"); };
  const handleSearch = (value: string) => { setSearch(value); setSelectedIds(new Set()); };
  const handleFilterChange = (filter: FilterStatus) => { setFilterStatus(filter); setSelectedIds(new Set()); };

  const handleSort = (field: SortField) => {
    if (sortField === field) setSortOrder(sortOrder === "asc" ? "desc" : "asc");
    else { setSortField(field); setSortOrder("desc"); }
  };

  const handleSelect = (id: string, checked: boolean) => {
    setSelectedIds(prev => { const next = new Set(prev); if (checked) next.add(id); else next.delete(id); return next; });
  };

  const handleSelectAll = () => {
    if (selectedIds.size === filteredProviders.length) setSelectedIds(new Set());
    else setSelectedIds(new Set(filteredProviders.map(p => p.id)));
  };

  const handleBatchTest = async () => {
    const ids = Array.from(selectedIds);
    setTestingIds(new Set(ids));
    await Promise.all(ids.map(async (id) => {
      try {
        await testMutation.mutateAsync(id);
      } catch {
        // continue
      } finally {
        setTestingIds(prev => { const next = new Set(prev); next.delete(id); return next; });
      }
    }));
    addToast(t("common.success"), "success");
    void providersQuery.refetch();
  };

  const handleTest = async (id: string) => {
    setPendingId(id);
    try {
      const result = await testMutation.mutateAsync(id);
      if (result.status === "error") addToast(String(result.error_message || result.error || t("common.error")), "error");
      else addToast(t("common.success"), "success");
      await providersQuery.refetch();
    } catch (e: any) {
      addToast(e.message || t("common.error"), "error");
      await providersQuery.refetch();
    } finally {
      setPendingId(null);
    }
  };

  const handleSetDefault = async (id: string, model?: string) => {
    try {
      await defaultProviderMutation.mutateAsync({ id, model });
      await statusQuery.refetch();
      addToast(t("providers.default_set"), "success");
    } catch (e: any) {
      addToast(e?.message || t("common.error"), "error");
    }
  };

  const handleDeleteConfirm = async () => {
    if (!deleteConfirmProvider) return;
    try {
      await deleteProviderKey(deleteConfirmProvider.id);
      await providersQuery.refetch();
      setDeleteConfirmProvider(null);
      addToast(t("providers.key_removed"), "success");
    } catch (e: any) {
      addToast(e?.message || t("common.error"), "error");
    }
  };

  const allSelected = filteredProviders.length > 0 && selectedIds.size === filteredProviders.length;

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
            <Button variant="primary" size="sm" onClick={() => setShowCreateForm(true)} leftIcon={<Plus className="w-3.5 h-3.5" />} title={t("providers.add") + " (n)"}>
              <span>{t("providers.add")}</span>
              <kbd className="hidden sm:inline-flex h-4 min-w-[16px] items-center justify-center rounded border border-white/30 bg-white/10 px-1 text-[8px] font-mono font-semibold ml-1.5">n</kbd>
            </Button>
            <div className="hidden rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase text-text-dim sm:block">
              {configuredCount} / {providers.length} {t("providers.configured")}
            </div>
          </div>
        }
      />

      {/* Search & Controls */}
      <div className="flex flex-col sm:flex-row gap-3">
        <div className="flex-1">
          <Input value={search} onChange={(e) => handleSearch(e.target.value)} placeholder={t("common.search")}
            leftIcon={<Search className="w-4 h-4" />}
            rightIcon={search && (
              <button onClick={() => setSearch("")} className="hover:text-text-main" aria-label={t("common.clear_search")}>
                <X className="w-3 h-3" />
              </button>
            )} />
        </div>

        <div className="flex gap-2 items-center flex-wrap">
          <div className="flex gap-1 p-1 bg-main/30 rounded-lg">
            {(["name", "models", "latency"] as SortField[]).map(field => (
              <button key={field} onClick={() => handleSort(field)}
                className={`flex items-center gap-1 px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${sortField === field ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}>
                {sortField === field && (sortOrder === "asc" ? <SortAsc className="w-3 h-3" /> : <SortDesc className="w-3 h-3" />)}
                {t(`providers.${field === "name" ? "name" : field}`)}
              </button>
            ))}
          </div>

          <div className="flex gap-1 p-1 bg-main/30 rounded-lg">
            <button onClick={() => setViewMode("grid")} className={`p-1.5 rounded-md transition-colors ${viewMode === "grid" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}>
              <Grid3X3 className="w-4 h-4" />
            </button>
            <button onClick={() => setViewMode("list")} className={`p-1.5 rounded-md transition-colors ${viewMode === "list" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}>
              <List className="w-4 h-4" />
            </button>
          </div>
        </div>
      </div>

      {/* Tabs & Filter */}
      <div className="flex items-center justify-between gap-3 flex-wrap overflow-x-auto">
        <div className="flex gap-1 p-1 bg-main/30 rounded-xl w-fit">
          <button onClick={() => handleTabChange("configured")}
            className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-colors ${activeTab === "configured" ? "bg-surface text-success shadow-sm" : "text-text-dim hover:text-text-main"}`}>
            <CheckCircle2 className="w-4 h-4" />
            {t("providers.configured")}
            <span className={`ml-1 px-1.5 py-0.5 rounded-full text-[10px] ${activeTab === "configured" ? "bg-success/20 text-success" : "bg-border-subtle text-text-dim"}`}>{configuredCount}</span>
          </button>
          <button onClick={() => handleTabChange("unconfigured")}
            className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-colors ${activeTab === "unconfigured" ? "bg-surface text-brand shadow-sm" : "text-text-dim hover:text-text-main"}`}>
            <XCircle className="w-4 h-4" />
            {t("providers.unconfigured")}
            <span className={`ml-1 px-1.5 py-0.5 rounded-full text-[10px] ${activeTab === "unconfigured" ? "bg-brand/20 text-brand" : "bg-border-subtle text-text-dim"}`}>{unconfiguredCount}</span>
          </button>
        </div>

        {activeTab === "configured" && <FilterChips activeFilter={filterStatus} onChange={handleFilterChange} t={t} />}

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
        <div className={viewMode === "grid" ? "grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4 3xl:grid-cols-5 4xl:grid-cols-6" : "flex flex-col gap-2"}>
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
          <div className="flex items-center gap-2">
            <button onClick={handleSelectAll} className="flex items-center gap-2 text-xs font-bold text-text-dim hover:text-text-main transition-colors">
              {allSelected ? <CheckSquare className="w-4 h-4 text-brand" /> : <Square className="w-4 h-4" />}
              {t("providers.select_all")}
            </button>
            {(search || filterStatus !== "all") && (
              <span className="text-xs text-text-dim">({filteredProviders.length} {t("providers.results")})</span>
            )}
          </div>

          <div className={viewMode === "grid" ? "grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4 3xl:grid-cols-5 4xl:grid-cols-6" : "flex flex-col gap-2"}>
            {filteredProviders.map((p) => (
              <ProviderCard
                key={p.id} provider={p}
                isSelected={selectedIds.has(p.id)}
                isDefault={p.id === currentDefaultProvider}
                pendingId={testingIds.has(p.id) ? p.id : pendingId}
                viewMode={viewMode}
                onSelect={handleSelect}
                onTest={handleTest}
                onSetDefault={handleSetDefault}
                onViewDetails={setDetailsProvider}
                onConfigure={config.open}
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

      {/* Config Modal */}
      <Modal isOpen={!!config.provider} onClose={config.close} title={t("providers.configure_provider")} size="md">
        {config.provider && (
          <div className="p-5 space-y-4">
            <div className="flex items-center gap-3 p-3 rounded-xl bg-main">
              <div className="w-10 h-10 rounded-xl bg-brand/10 flex items-center justify-center">
                {providerIcons[config.provider.id] || <Server className="w-5 h-5 text-brand" />}
              </div>
              <div>
                <p className="text-sm font-bold">{config.provider.display_name || config.provider.id}</p>
                <p className="text-[10px] text-text-dim font-mono">{config.provider.id}</p>
              </div>
              <Badge variant={getAuthBadge(config.provider.auth_status).variant} className="ml-auto">
                {getAuthBadge(config.provider.auth_status).label}
              </Badge>
            </div>

            {config.provider.key_required !== false && (
              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">API Key</label>
                <input type="password" value={config.keyInput} onChange={e => config.setKeyInput(e.target.value)}
                  placeholder={config.hasStoredKey ? t("providers.key_placeholder_existing") : t("providers.key_placeholder")}
                  className="mt-1 w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm font-mono outline-none focus:border-brand focus:ring-1 focus:ring-brand/20" />
              </div>
            )}

            <div>
              <label className="text-[10px] font-bold text-text-dim uppercase">Base URL <span className="normal-case font-normal text-text-dim/50">({t("providers.optional")})</span></label>
              <input type="text" value={config.urlInput} onChange={e => config.setUrlInput(e.target.value)}
                placeholder="https://api.example.com/v1"
                className="mt-1 w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm font-mono outline-none focus:border-brand focus:ring-1 focus:ring-brand/20" />
            </div>

            <div>
              <label className="text-[10px] font-bold text-text-dim uppercase">{t("providers.proxy_url")} <span className="normal-case font-normal text-text-dim/50">({t("providers.optional")})</span></label>
              <input type="text" value={config.proxyInput} onChange={e => config.setProxyInput(e.target.value)}
                placeholder={t("providers.proxy_url_placeholder")}
                className="mt-1 w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm font-mono outline-none focus:border-brand focus:ring-1 focus:ring-brand/20" />
            </div>

            {config.error && (
              <div className="flex items-center gap-2 text-error text-xs">
                <AlertCircle className="w-4 h-4 shrink-0" />
                {config.error}
              </div>
            )}

            {config.testResult && (
              <div className={`flex items-center gap-2 text-xs p-3 rounded-xl ${config.testResult.ok ? "bg-success/10 border border-success/20 text-success" : "bg-error/10 border border-error/20 text-error"}`}>
                {config.testResult.ok ? <CheckCircle2 className="w-4 h-4 shrink-0" /> : <XCircle className="w-4 h-4 shrink-0" />}
                {config.testResult.message}
              </div>
            )}

            <div className="flex gap-2 pt-2">
              <Button variant="primary" className="flex-1" onClick={config.saveKey}
                disabled={config.saving || config.testing || (!config.keyInput.trim() && config.urlInput === (config.provider.base_url || "") && config.proxyInput === (config.provider.proxy_url || ""))}>
                {config.saving ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Key className="w-4 h-4 mr-1" />}
                {t("common.save")}
              </Button>
              <Button variant="secondary" onClick={config.testKey} disabled={config.saving || config.testing || (!config.hasStoredKey && !config.keyInput.trim())}>
                {config.testing ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Zap className="w-4 h-4 mr-1" />}
                {t("providers.test")}
              </Button>
              {config.hasStoredKey && (
                <Button variant="secondary" onClick={config.removeKey} disabled={config.saving || config.testing}>
                  <XCircle className="w-4 h-4 mr-1 text-error" />
                  {t("providers.remove_key")}
                </Button>
              )}
            </div>

            {config.hasStoredKey && (
              <SetDefaultModelSection
                providerId={config.provider.id}
                currentDefault={statusQuery.data?.default_provider}
                onSetDefault={handleSetDefault}
                t={t}
              />
            )}
          </div>
        )}
      </Modal>

      {/* Delete Confirmation Modal */}
      <Modal isOpen={!!deleteConfirmProvider} onClose={() => setDeleteConfirmProvider(null)}
        title={deleteConfirmProvider?.is_custom ? t("providers.delete_confirm_title") : t("providers.remove_key_confirm_title")} size="sm">
        {deleteConfirmProvider && (
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
            <p className="text-sm text-text-dim">
              {deleteConfirmProvider.is_custom ? t("providers.delete_confirm_message") : t("providers.remove_key_confirm_message")}
            </p>
            <div className="flex gap-2 pt-2">
              <Button variant="ghost" className="flex-1" onClick={() => setDeleteConfirmProvider(null)}>
                {t("common.cancel")}
              </Button>
              <Button variant="primary" className="flex-1 !bg-error hover:!bg-error/80" onClick={handleDeleteConfirm}>
                <Trash2 className="w-4 h-4 mr-1" />
                {deleteConfirmProvider.is_custom ? t("common.delete") : t("providers.remove_key")}
              </Button>
            </div>
          </div>
        )}
      </Modal>

      {/* Create Provider Wizard */}
      <Modal isOpen={showCreateForm} onClose={() => setShowCreateForm(false)} title={t("providers.add")} size="xl" hideCloseButton>
        <CreateProviderWizard
          onSubmit={async (values) => {
            await createRegistryContent("provider", values);
            setShowCreateForm(false);
            setActiveTab("configured");
            void providersQuery.refetch();
          }}
          onCancel={() => setShowCreateForm(false)}
        />
      </Modal>
    </div>
  );
}
