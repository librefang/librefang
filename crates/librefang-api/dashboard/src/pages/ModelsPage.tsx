import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { formatCompact, formatCost as formatCostUtil } from "../lib/format";
import type { ModelItem } from "../api";
import { FormEvent, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { listModels, addCustomModel, removeCustomModel } from "../api";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Modal } from "../components/ui/Modal";
import { useCreateShortcut } from "../lib/useCreateShortcut";
import { useUIStore } from "../lib/store";
import {
  Cpu, Search, Check, X, Eye, EyeOff, Wrench, Zap, AlertCircle, Lock, Plus, Trash2, Loader2, Sparkles,
  ChevronDown, ChevronRight, Brain, ArrowUpDown, ChevronsUpDown, Tag,
} from "lucide-react";
import { modelKey } from "../lib/hiddenModels";

type SortField = "model" | "provider" | "tier" | "context" | "input_cost" | "output_cost";
type SortDir = "asc" | "desc";

const GRID_COLS = "grid-cols-[minmax(140px,1fr)_90px_70px_70px_70px_70px_40px_40px_40px_40px_70px]";
const GRID_MIN_W = "min-w-[860px]";

const REFRESH_MS = 60000;
export function ModelsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);
  const [search, setSearch] = useState("");
  const [tierFilter, setTierFilter] = useState<string>("all");
  const [providerFilter, setProviderFilter] = useState<string>("all");
  const availableOnly = useUIStore((s) => s.modelsAvailableOnly);
  const setAvailableOnly = useUIStore((s) => s.setModelsAvailableOnly);
  const [showAdd, setShowAdd] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  useCreateShortcut(() => setShowAdd(true));
  const [showHidden, setShowHidden] = useState(false);
  const [expandedProviders, setExpandedProviders] = useState<Set<string>>(new Set());
  const [expandedModelId, setExpandedModelId] = useState<string | null>(null);
  const [sortField, setSortField] = useState<SortField>("model");
  const [sortDir, setSortDir] = useState<SortDir>("asc");
  const [isMobile, setIsMobile] = useState(false);
  const hiddenModelKeys = useUIStore((s) => s.hiddenModelKeys);
  const hideModelAction = useUIStore((s) => s.hideModel);
  const unhideModelAction = useUIStore((s) => s.unhideModel);
  const pruneHiddenKeys = useUIStore((s) => s.pruneHiddenKeys);

  useEffect(() => {
    const mq = window.matchMedia("(max-width: 767px)");
    setIsMobile(mq.matches);
    const handler = (e: MediaQueryListEvent) => setIsMobile(e.matches);
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, []);

  // Form state
  const [formId, setFormId] = useState("");
  const [formProvider, setFormProvider] = useState("");
  const [formDisplayName, setFormDisplayName] = useState("");
  const [formContextWindow, setFormContextWindow] = useState(128000);
  const [formMaxOutput, setFormMaxOutput] = useState(8192);
  const [formInputCost, setFormInputCost] = useState(0);
  const [formOutputCost, setFormOutputCost] = useState(0);
  const [formTools, setFormTools] = useState(true);
  const [formVision, setFormVision] = useState(false);
  const [formStreaming, setFormStreaming] = useState(true);

  const modelsQuery = useQuery({
    queryKey: ["models"],
    queryFn: () => listModels(),
    refetchInterval: REFRESH_MS,
  });

  const addMut = useMutation({
    mutationFn: addCustomModel,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["models"] });
      addToast(t("models.model_added"), "success");
      resetForm();
    },
  });

  const deleteMut = useMutation({
    mutationFn: removeCustomModel,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["models"] });
      addToast(t("models.model_deleted"), "success");
    },
  });

  const resetForm = () => {
    setShowAdd(false);
    setFormId("");
    setFormProvider("");
    setFormDisplayName("");
    setFormContextWindow(128000);
    setFormMaxOutput(8192);
    setFormInputCost(0);
    setFormOutputCost(0);
    setFormTools(true);
    setFormVision(false);
    setFormStreaming(true);
  };

  const handleAdd = async (e: FormEvent) => {
    e.preventDefault();
    if (!formId.trim() || !formProvider.trim()) return;
    await addMut.mutateAsync({
      id: formId.trim(),
      provider: formProvider.trim(),
      display_name: formDisplayName.trim() || undefined,
      context_window: formContextWindow,
      max_output_tokens: formMaxOutput,
      input_cost_per_m: formInputCost,
      output_cost_per_m: formOutputCost,
      supports_tools: formTools,
      supports_vision: formVision,
      supports_streaming: formStreaming,
    });
  };

  const handleDelete = async (id: string) => {
    if (confirmDeleteId !== id) { setConfirmDeleteId(id); return; }
    setConfirmDeleteId(null);
    try {
      await deleteMut.mutateAsync(id);
      const orphan = hiddenModelKeys.find(k => k.endsWith(`:${id}`));
      if (orphan) unhideModelAction(orphan);
    } catch (err: any) { addToast(err.message || t("common.error"), "error"); }
  };

  // Available models first, unavailable last
  const allModels = useMemo(
    () => [...(modelsQuery.data?.models ?? [])].sort((a, b) => {
      if (a.available && !b.available) return -1;
      if (!a.available && b.available) return 1;
      return 0;
    }),
    [modelsQuery.data],
  );
  const totalAvailable = modelsQuery.data?.available ?? 0;

  const providers = useMemo(
    () => ["all", ...Array.from(new Set(allModels.map(m => m.provider))).sort()],
    [allModels],
  );
  const tiers = useMemo(
    () => ["all", ...Array.from(new Set(allModels.map(m => m.tier).filter(Boolean))).sort()],
    [allModels],
  );

  const hiddenSet = useMemo(() => new Set(hiddenModelKeys), [hiddenModelKeys]);

  useEffect(() => {
    if (allModels.length === 0) return;
    pruneHiddenKeys(new Set(allModels.map(modelKey)));
  }, [allModels, pruneHiddenKeys]);

  const filtered = useMemo(
    () => allModels.filter(m => {
      const q = search.toLowerCase();
      if (search
        && !m.id.toLowerCase().includes(q)
        && !(m.display_name || "").toLowerCase().includes(q)
        && !m.provider.toLowerCase().includes(q)
        && !(m.aliases ?? []).some(a => a.toLowerCase().includes(q))
      ) return false;
      if (tierFilter !== "all" && m.tier !== tierFilter) return false;
      if (providerFilter !== "all" && m.provider !== providerFilter) return false;
      if (availableOnly && !m.available) return false;
      return showHidden === hiddenSet.has(modelKey(m));
    }),
    [allModels, search, tierFilter, providerFilter, availableOnly, showHidden, hiddenSet],
  );

  const hiddenCount = useMemo(() => allModels.filter(m => hiddenSet.has(modelKey(m))).length, [allModels, hiddenSet]);

  // Sort
  const sortedFiltered = useMemo(() => {
    const sorted = [...filtered];
    const dir = sortDir === "asc" ? 1 : -1;
    sorted.sort((a, b) => {
      let cmp = 0;
      switch (sortField) {
        case "model": cmp = (a.display_name || a.id).localeCompare(b.display_name || b.id); break;
        case "provider": cmp = a.provider.localeCompare(b.provider); break;
        case "tier": cmp = (a.tier || "").localeCompare(b.tier || ""); break;
        case "context": cmp = (a.context_window ?? 0) - (b.context_window ?? 0); break;
        case "input_cost": cmp = (a.input_cost_per_m ?? 0) - (b.input_cost_per_m ?? 0); break;
        case "output_cost": cmp = (a.output_cost_per_m ?? 0) - (b.output_cost_per_m ?? 0); break;
      }
      return cmp * dir;
    });
    return sorted;
  }, [filtered, sortField, sortDir]);

  // Group by provider when showing all providers
  const grouped = useMemo(() => {
    if (providerFilter !== "all") return null;
    const map = new Map<string, ModelItem[]>();
    for (const m of sortedFiltered) {
      const list = map.get(m.provider);
      if (list) list.push(m);
      else map.set(m.provider, [m]);
    }
    return new Map([...map.entries()].sort(([a], [b]) => a.localeCompare(b)));
  }, [sortedFiltered, providerFilter]);

  const allGroupedProviders = useMemo(() => grouped ? Array.from(grouped.keys()) : [], [grouped]);
  const allExpanded = allGroupedProviders.length > 0 && allGroupedProviders.every(p => expandedProviders.has(p));

  const toggleSort = (field: SortField) => {
    if (sortField === field) setSortDir(d => d === "asc" ? "desc" : "asc");
    else { setSortField(field); setSortDir("asc"); }
  };

  const tierColor = (tier?: string) => {
    switch (tier) {
      case "basic": return "bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-400";
      case "fast": return "bg-cyan-50 text-cyan-600 dark:bg-cyan-900/30 dark:text-cyan-400";
      case "smart": return "bg-blue-50 text-blue-600 dark:bg-blue-900/30 dark:text-blue-400";
      case "balanced": return "bg-teal-50 text-teal-600 dark:bg-teal-900/30 dark:text-teal-400";
      case "standard": return "bg-green-50 text-green-600 dark:bg-green-900/30 dark:text-green-400";
      case "advanced": return "bg-purple-50 text-purple-600 dark:bg-purple-900/30 dark:text-purple-400";
      case "frontier": return "bg-rose-50 text-rose-600 dark:bg-rose-900/30 dark:text-rose-400";
      case "enterprise": return "bg-amber-50 text-amber-600 dark:bg-amber-900/30 dark:text-amber-400";
      case "local": return "bg-orange-50 text-orange-600 dark:bg-orange-900/30 dark:text-orange-400";
      case "custom": return "bg-violet-50 text-violet-600 dark:bg-violet-900/30 dark:text-violet-400";
      default: return "bg-main text-text-dim";
    }
  };

  const formatCost = (cost?: number) => {
    if (cost === undefined || cost === null) return "-";
    if (cost === 0) return t("models.free");
    return formatCostUtil(cost);
  };

  const formatCtx = (tokens?: number) => {
    if (!tokens) return "-";
    return formatCompact(tokens);
  };

  const SortHeader = ({ field, children, className = "" }: { field: SortField; children: React.ReactNode; className?: string }) => (
    <button type="button" onClick={() => toggleSort(field)}
      className={`group flex items-center gap-0.5 cursor-pointer hover:text-text transition-colors select-none ${className}`}>
      {children}
      {sortField === field
        ? <ArrowUpDown className="w-3 h-3 text-brand" />
        : <ArrowUpDown className="w-3 h-3 opacity-0 group-hover:opacity-30" />}
    </button>
  );

  const inputClass = "w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";

  // Collapsed provider summary: tier badges + cheapest cost
  const providerSummary = (models: ModelItem[]) => {
    const tierSet = new Set(models.map(m => m.tier).filter(Boolean));
    const tiers = Array.from(tierSet).sort();
    const costs = models.map(m => m.input_cost_per_m ?? 0).filter(c => c > 0);
    const minCost = costs.length > 0 ? Math.min(...costs) : null;
    return (
      <div className="flex items-center gap-1.5 ml-auto mr-2">
        {tiers.slice(0, 4).map(tier => (
          <span key={tier} className={`text-[9px] font-bold px-1.5 py-0.5 rounded ${tierColor(tier)}`}>{tier}</span>
        ))}
        {tiers.length > 4 && <span className="text-[9px] text-text-dim">+{tiers.length - 4}</span>}
        {minCost !== null && (
          <span className="text-[10px] text-text-dim font-mono ml-1">{t("models.cost_range")} {formatCostUtil(minCost)}+</span>
        )}
      </div>
    );
  };

  // Mobile card for a single model
  const renderMobileCard = (m: ModelItem) => {
    const isCustom = m.tier === "custom";
    const mKey = `${m.provider}:${m.id}`;
    const isExpanded = expandedModelId === mKey;
    return (
      <div key={mKey} className={`rounded-xl border border-border-subtle p-3 space-y-2 ${!m.available ? "opacity-40" : ""}`}>
        <div className="flex items-start justify-between gap-2">
          <button type="button" onClick={() => setExpandedModelId(isExpanded ? null : mKey)} className="text-left min-w-0 flex-1">
            <div className="flex items-center gap-1.5">
              <p className="text-sm font-bold truncate">{m.display_name || m.id}</p>
              {m.available ? <span className="w-2 h-2 rounded-full bg-success shrink-0" /> : <Lock className="w-3 h-3 text-text-dim/60 shrink-0" />}
              {isCustom && <Sparkles className="w-3 h-3 text-violet-500 shrink-0" />}
            </div>
            {m.display_name && m.display_name !== m.id && (
              <p className="text-[10px] text-text-dim/40 font-mono truncate">{m.id}</p>
            )}
          </button>
          <div className="flex items-center gap-1 shrink-0">
            {showHidden ? (
              <button onClick={() => { unhideModelAction(modelKey(m)); addToast(t("models.model_unhidden"), "success"); }}
                className="p-1 rounded text-text-dim/40 hover:text-success" title={t("models.unhide_model")}><Eye className="w-3.5 h-3.5" /></button>
            ) : (
              <button onClick={() => { hideModelAction(modelKey(m)); addToast(t("models.model_hidden"), "success"); }}
                className="p-1 rounded text-text-dim/40 hover:text-warning" title={t("models.hide_model")}><EyeOff className="w-3.5 h-3.5" /></button>
            )}
            {isCustom && !showHidden && (
              confirmDeleteId === m.id
                ? <button onClick={() => handleDelete(m.id)} className="px-1.5 py-0.5 rounded bg-error text-white text-[9px] font-bold">{t("common.confirm")}</button>
                : <button onClick={() => handleDelete(m.id)} className="p-1 rounded text-text-dim/20 hover:text-error" title={t("models.delete_model")}><Trash2 className="w-3.5 h-3.5" /></button>
            )}
          </div>
        </div>
        <div className="flex flex-wrap gap-1.5 items-center">
          <span className="text-[10px] font-semibold text-text-dim bg-surface px-1.5 py-0.5 rounded">{m.provider}</span>
          <span className={`text-[9px] font-bold px-1.5 py-0.5 rounded ${tierColor(m.tier)}`}>{m.tier === "custom" ? t("models.custom") : m.tier || "-"}</span>
          <span className="text-[10px] font-mono text-text-dim">{formatCtx(m.context_window)}</span>
          <span className="text-[10px] font-mono text-text">{formatCost(m.input_cost_per_m)}/{formatCost(m.output_cost_per_m)}</span>
        </div>
        <div className="flex gap-2 items-center">
          {m.supports_tools && <Wrench className="w-3 h-3 text-success" />}
          {m.supports_vision && <Eye className="w-3 h-3 text-success" />}
          {m.supports_streaming && <Zap className="w-3 h-3 text-success" />}
          {m.supports_thinking && <Brain className="w-3 h-3 text-success" />}
        </div>
        {isExpanded && (
          <div className="pt-2 border-t border-border-subtle/50 space-y-1 text-[11px] text-text-dim">
            <p><span className="font-bold">{t("models.model_id")}:</span> <span className="font-mono">{m.id}</span></p>
            <p><span className="font-bold">{t("models.max_output_tokens")}:</span> {formatCtx(m.max_output_tokens)}</p>
            {(m.aliases ?? []).length > 0 && (
              <div className="flex items-center gap-1 flex-wrap">
                <Tag className="w-3 h-3" />
                <span className="font-bold">{t("models.aliases")}:</span>
                {m.aliases!.map(a => <span key={a} className="font-mono bg-surface px-1 rounded">{a}</span>)}
              </div>
            )}
          </div>
        )}
      </div>
    );
  };

  // Desktop table row
  const renderRow = (m: ModelItem, i: number) => {
    const isCustom = m.tier === "custom";
    const mKey = `${m.provider}:${m.id}`;
    const isExpanded = expandedModelId === mKey;
    return (
      <div key={mKey}>
        <div
          className={`grid ${GRID_COLS} ${GRID_MIN_W} gap-3 px-5 py-3 items-center border-t border-border-subtle/50 hover:bg-surface transition-colors cursor-pointer ${
            !m.available ? "opacity-40" : ""
          } ${i % 2 === 0 ? "" : "bg-main/30"}`}
          onClick={() => setExpandedModelId(isExpanded ? null : mKey)}
        >
          <div className="min-w-0">
            <div className="flex items-center gap-1.5">
              <p className="text-sm font-bold truncate">{m.display_name || m.id}</p>
              {m.available ? (
                <span className="w-2 h-2 rounded-full bg-success shrink-0" />
              ) : (
                <span className="flex items-center gap-0.5 text-[9px] text-text-dim/60 shrink-0">
                  <Lock className="w-3 h-3" /> {t("models.no_key")}
                </span>
              )}
              {isCustom && <Sparkles className="w-3 h-3 text-violet-500 shrink-0" />}
            </div>
            {m.display_name && m.display_name !== m.id && (
              <p className="text-[10px] text-text-dim/40 font-mono truncate">{m.id}</p>
            )}
          </div>
          <span className="text-xs font-semibold text-text truncate">{m.provider}</span>
          <span className={`text-[10px] font-bold px-2 py-0.5 rounded-md w-fit ${tierColor(m.tier)}`}>
            {m.tier === "custom" ? t("models.custom") : m.tier || "-"}
          </span>
          <span className="text-xs font-mono text-text">{formatCtx(m.context_window)}</span>
          <span className="text-xs font-mono text-text">{formatCost(m.input_cost_per_m)}</span>
          <span className="text-xs font-mono text-text">{formatCost(m.output_cost_per_m)}</span>
          <span className="text-center">{m.supports_tools ? <Check className="w-4 h-4 text-success inline" /> : <X className="w-4 h-4 text-text-dim/15 inline" />}</span>
          <span className="text-center">{m.supports_vision ? <Check className="w-4 h-4 text-success inline" /> : <X className="w-4 h-4 text-text-dim/15 inline" />}</span>
          <span className="text-center">{m.supports_streaming ? <Check className="w-4 h-4 text-success inline" /> : <X className="w-4 h-4 text-text-dim/15 inline" />}</span>
          <span className="text-center">{m.supports_thinking ? <Check className="w-4 h-4 text-success inline" /> : <X className="w-4 h-4 text-text-dim/15 inline" />}</span>
          <span className="flex items-center justify-center gap-1" onClick={e => e.stopPropagation()}>
            {showHidden ? (
              <button onClick={() => { unhideModelAction(modelKey(m)); addToast(t("models.model_unhidden"), "success"); }}
                className="p-1 rounded text-text-dim/40 hover:text-success hover:bg-success/10 transition-colors" title={t("models.unhide_model")} aria-label={t("models.unhide_model")}>
                <Eye className="w-3.5 h-3.5" />
              </button>
            ) : (
              <button onClick={() => { hideModelAction(modelKey(m)); addToast(t("models.model_hidden"), "success"); }}
                className="p-1 rounded text-text-dim/40 hover:text-warning hover:bg-warning/10 transition-colors" title={t("models.hide_model")} aria-label={t("models.hide_model")}>
                <EyeOff className="w-3.5 h-3.5" />
              </button>
            )}
            {isCustom && !showHidden && (
              confirmDeleteId === m.id ? (
                <button onClick={() => handleDelete(m.id)} className="px-1.5 py-0.5 rounded bg-error text-white text-[9px] font-bold">{t("common.confirm")}</button>
              ) : (
                <button onClick={() => handleDelete(m.id)} className="p-1 rounded text-text-dim/20 hover:text-error hover:bg-error/10 transition-colors" title={t("models.delete_model")}>
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              )
            )}
          </span>
        </div>
        {/* Inline detail panel */}
        {isExpanded && (
          <div className={`px-5 py-3 bg-surface/50 border-t border-border-subtle/30 ${GRID_MIN_W}`}>
            <div className="flex flex-wrap gap-x-6 gap-y-1 text-[11px] text-text-dim">
              <span><span className="font-bold">{t("models.model_id")}:</span> <span className="font-mono">{m.id}</span></span>
              <span><span className="font-bold">{t("models.max_output_tokens")}:</span> {formatCtx(m.max_output_tokens)}</span>
              <span><span className="font-bold">{t("models.col_output")}:</span> {formatCost(m.output_cost_per_m)}</span>
              {(m.aliases ?? []).length > 0 && (
                <span className="flex items-center gap-1">
                  <Tag className="w-3 h-3" />
                  <span className="font-bold">{t("models.aliases")}:</span>
                  {m.aliases!.map(a => <span key={a} className="font-mono bg-main px-1 rounded">{a}</span>)}
                </span>
              )}
            </div>
          </div>
        )}
      </div>
    );
  };

  const toggleProvider = (p: string) => {
    setExpandedProviders(prev => {
      const next = new Set(prev);
      if (next.has(p)) next.delete(p); else next.add(p);
      return next;
    });
  };

  const toggleAllProviders = () => {
    if (allExpanded) {
      setExpandedProviders(new Set());
    } else {
      setExpandedProviders(new Set(allGroupedProviders));
    }
  };

  const colHeader = (
    <div className={`grid ${GRID_COLS} ${GRID_MIN_W} gap-3 px-5 py-3 bg-main text-[11px] font-bold text-text-dim/60 uppercase`}>
      <SortHeader field="model">{t("models.col_model")}</SortHeader>
      <SortHeader field="provider">{t("models.col_provider")}</SortHeader>
      <SortHeader field="tier">{t("models.col_tier")}</SortHeader>
      <SortHeader field="context">{t("models.col_context")}</SortHeader>
      <SortHeader field="input_cost">{t("models.col_input")}</SortHeader>
      <SortHeader field="output_cost">{t("models.col_output")}</SortHeader>
      <span className="text-center" title={t("models.col_tools")}><Wrench className="w-3.5 h-3.5 inline" /></span>
      <span className="text-center" title={t("models.col_vision")}><Eye className="w-3.5 h-3.5 inline" /></span>
      <span className="text-center" title={t("models.col_streaming")}><Zap className="w-3.5 h-3.5 inline" /></span>
      <span className="text-center" title={t("models.col_thinking")}><Brain className="w-3.5 h-3.5 inline" /></span>
      <span></span>
    </div>
  );

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("models.section")}
        title={t("models.title")}
        subtitle={t("models.subtitle")}
        icon={<Cpu className="h-4 w-4" />}
        isFetching={modelsQuery.isFetching}
        onRefresh={() => modelsQuery.refetch()}
        helpText={t("models.help")}
        actions={
          <div className="flex items-center gap-2">
            {allModels.length > 0 && <Badge variant="brand">{totalAvailable} / {allModels.length} {t("models.available")}</Badge>}
            <Button variant="primary" onClick={() => setShowAdd(true)} title={t("models.add_model") + " (n)"}>
              <Plus className="w-4 h-4" />
              <span>{t("models.add_model")}</span>
              <kbd className="hidden sm:inline-flex h-5 min-w-[20px] items-center justify-center rounded border border-white/30 bg-white/10 px-1 text-[9px] font-mono font-semibold">n</kbd>
            </Button>
          </div>
        }
      />

      {modelsQuery.isError && (
        <div className="flex items-center gap-3 p-4 rounded-2xl bg-error/5 border border-error/20 text-error">
          <AlertCircle className="w-5 h-5 shrink-0" />
          <p className="text-sm">{t("models.load_error")}</p>
        </div>
      )}

      {/* Filters */}
      <div className="flex flex-wrap gap-2 sm:gap-3 items-center">
        <div className="flex-1 min-w-[160px] sm:min-w-[200px] max-w-sm">
          <Input value={search} onChange={e => setSearch(e.target.value)}
            placeholder={t("models.search_placeholder")}
            leftIcon={<Search className="h-4 w-4" />}
            data-shortcut-search />
        </div>

        <select value={providerFilter} onChange={e => setProviderFilter(e.target.value)}
          className="rounded-xl border border-border-subtle bg-surface px-3 py-2.5 text-xs outline-none focus:border-brand">
          {providers.map(p => <option key={p} value={p}>{p === "all" ? t("models.all_providers") : p}</option>)}
        </select>

        <div className="hidden sm:flex gap-0.5 rounded-xl border border-border-subtle bg-surface p-0.5 flex-wrap overflow-x-auto">
          {tiers.map(tier => (
            <button key={tier} onClick={() => setTierFilter(tier || "all")}
              className={`px-2.5 py-1.5 rounded-lg text-[10px] font-bold transition-colors ${
                tierFilter === tier ? "bg-brand text-white shadow-sm" : "text-text-dim hover:text-text hover:bg-main"
              }`}>
              {t(`models.tier_${tier}`, { defaultValue: tier })}
            </button>
          ))}
        </div>

        <button onClick={() => setAvailableOnly(!availableOnly)}
          className={`flex items-center gap-1.5 px-3 py-2.5 rounded-xl border text-xs font-bold transition-colors ${
            availableOnly ? "border-success bg-success/10 text-success" : "border-border-subtle text-text-dim hover:border-brand/30"
          }`}>
          <Check className="w-3 h-3" />
          {t("models.available_only")}
        </button>

        <button onClick={() => setShowHidden(!showHidden)}
          className={`flex items-center gap-1.5 px-3 py-2.5 rounded-xl border text-xs font-bold transition-colors ${
            showHidden ? "border-warning bg-warning/10 text-warning" : "border-border-subtle text-text-dim hover:border-brand/30"
          }`}>
          <EyeOff className="w-3 h-3" />
          {t("models.show_hidden")}
          {hiddenCount > 0 && (
            <span className="ml-1 px-1.5 py-0.5 rounded-full bg-warning/20 text-warning text-[9px] font-bold">{hiddenCount}</span>
          )}
        </button>

        {grouped && allGroupedProviders.length > 1 && (
          <button onClick={toggleAllProviders}
            className="flex items-center gap-1.5 px-3 py-2.5 rounded-xl border border-border-subtle text-xs font-bold text-text-dim hover:border-brand/30 transition-colors">
            <ChevronsUpDown className="w-3 h-3" />
            {allExpanded ? t("models.collapse_all") : t("models.expand_all")}
          </button>
        )}
      </div>

      <p className="text-xs text-text-dim">{filtered.length} {t("models.results")}</p>

      {/* Model List */}
      {modelsQuery.isLoading ? (
        <ListSkeleton rows={5} />
      ) : filtered.length === 0 ? (
        <EmptyState
          icon={<Cpu className="w-7 h-7" />}
          title={allModels.length === 0 ? t("models.no_models") : t("models.no_results")}
        />
      ) : isMobile ? (
        /* Mobile card layout */
        <div className="flex flex-col gap-2">
          {sortedFiltered.map(m => renderMobileCard(m))}
        </div>
      ) : grouped ? (
        <div className="flex flex-col gap-3">
          {Array.from(grouped.entries()).map(([provider, models]) => {
            const collapsed = !expandedProviders.has(provider);
            const availCount = models.filter(m => m.available).length;
            return (
              <div key={provider} className="rounded-2xl border border-border-subtle overflow-hidden overflow-x-auto">
                <button
                  type="button"
                  onClick={() => toggleProvider(provider)}
                  className="flex items-center gap-3 w-full px-5 py-3.5 bg-surface hover:bg-main/60 transition-colors cursor-pointer select-none min-w-[780px]"
                >
                  {collapsed
                    ? <ChevronRight className="w-4 h-4 text-text-dim shrink-0" />
                    : <ChevronDown className="w-4 h-4 text-text-dim shrink-0" />}
                  <span className="text-sm font-bold text-text">{provider}</span>
                  <span className="px-2 py-0.5 rounded-full bg-brand/10 text-brand text-[11px] font-bold">{models.length}</span>
                  {availCount > 0 && availCount < models.length && (
                    <span className="text-[11px] text-text-dim">{availCount} {t("models.available")}</span>
                  )}
                  {collapsed && providerSummary(models)}
                </button>
                {!collapsed && (
                  <>
                    {colHeader}
                    {models.map((m, i) => renderRow(m, i))}
                  </>
                )}
              </div>
            );
          })}
        </div>
      ) : (
        <div className="rounded-2xl border border-border-subtle overflow-hidden overflow-x-auto">
          {colHeader}
          {sortedFiltered.map((m, i) => renderRow(m, i))}
        </div>
      )}

      {/* Add Model Modal */}
      <Modal isOpen={showAdd} onClose={resetForm} title={t("models.add_custom_model")} size="lg">
        <form onSubmit={handleAdd} className="p-5 space-y-4 max-h-[70vh] overflow-y-auto">
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                <div className="sm:col-span-2">
                  <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.model_id")} *</label>
                  <input value={formId} onChange={e => setFormId(e.target.value)} placeholder={t("models.model_id_placeholder")} className={inputClass} required />
                </div>
                <div>
                  <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.provider")} *</label>
                  <input value={formProvider} onChange={e => setFormProvider(e.target.value)} placeholder={t("models.provider_placeholder")} className={inputClass} required />
                </div>
                <div>
                  <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.display_name")}</label>
                  <input value={formDisplayName} onChange={e => setFormDisplayName(e.target.value)} placeholder={t("models.display_name_placeholder")} className={inputClass} />
                </div>
                <div>
                  <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.context_window")}</label>
                  <input type="number" value={formContextWindow} onChange={e => setFormContextWindow(+e.target.value)} className={inputClass} />
                </div>
                <div>
                  <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.max_output")}</label>
                  <input type="number" value={formMaxOutput} onChange={e => setFormMaxOutput(+e.target.value)} className={inputClass} />
                </div>
                <div>
                  <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.input_cost")}</label>
                  <input type="number" step="0.01" value={formInputCost} onChange={e => setFormInputCost(+e.target.value)} className={inputClass} />
                </div>
                <div>
                  <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.output_cost")}</label>
                  <input type="number" step="0.01" value={formOutputCost} onChange={e => setFormOutputCost(+e.target.value)} className={inputClass} />
                </div>
              </div>
              <div className="flex flex-wrap gap-3">
                {([
                  ["tools", formTools, setFormTools, t("models.supports_tools")] as const,
                  ["vision", formVision, setFormVision, t("models.supports_vision")] as const,
                  ["streaming", formStreaming, setFormStreaming, t("models.supports_streaming")] as const,
                ]).map(([key, val, setter, label]) => (
                  <button key={key} type="button" onClick={() => setter(!val)}
                    className={`flex items-center gap-1.5 px-3 py-2 rounded-xl border text-xs font-bold transition-colors ${
                      val ? "border-success bg-success/10 text-success" : "border-border-subtle text-text-dim"
                    }`}>
                    <Check className="w-3 h-3" />
                    {label}
                  </button>
                ))}
              </div>
              {addMut.error && (
                <div className="flex items-center gap-2 text-error text-xs"><AlertCircle className="w-4 h-4" /> {(addMut.error as any)?.message}</div>
              )}
              <div className="flex gap-2 pt-2">
                <Button type="submit" variant="primary" className="flex-1" disabled={addMut.isPending || !formId.trim() || !formProvider.trim()}>
                  {addMut.isPending ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Plus className="w-4 h-4 mr-1" />}
                  {t("models.add_model")}
                </Button>
                <Button type="button" variant="secondary" onClick={() => resetForm()}>{t("common.cancel")}</Button>
              </div>
        </form>
      </Modal>
    </div>
  );
}
