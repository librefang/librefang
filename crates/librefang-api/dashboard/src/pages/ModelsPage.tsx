import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { formatCompact, formatCost as formatCostUtil } from "../lib/format";
import { FormEvent, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { listModels, addCustomModel, removeCustomModel } from "../api";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { useUIStore } from "../lib/store";
import {
  Cpu, Search, Check, X, Eye, EyeOff, Wrench, Zap, AlertCircle, Lock, Plus, Trash2, Loader2, Sparkles
} from "lucide-react";
import { modelKey } from "../lib/hiddenModels";

const REFRESH_MS = 60000;
export function ModelsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);
  const [search, setSearch] = useState("");
  const [tierFilter, setTierFilter] = useState<string>("all");
  const [providerFilter, setProviderFilter] = useState<string>("all");
  const [availableOnly, setAvailableOnly] = useState(false);
  const [showAdd, setShowAdd] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [showHidden, setShowHidden] = useState(false);
  const hiddenModelKeys = useUIStore((s) => s.hiddenModelKeys);
  const hideModelAction = useUIStore((s) => s.hideModel);
  const unhideModelAction = useUIStore((s) => s.unhideModel);
  const pruneHiddenKeys = useUIStore((s) => s.pruneHiddenKeys);

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
      if (search && !m.id.toLowerCase().includes(q) && !(m.display_name || "").toLowerCase().includes(q) && !m.provider.toLowerCase().includes(q)) return false;
      if (tierFilter !== "all" && m.tier !== tierFilter) return false;
      if (providerFilter !== "all" && m.provider !== providerFilter) return false;
      if (availableOnly && !m.available) return false;
      return showHidden === hiddenSet.has(modelKey(m));
    }),
    [allModels, search, tierFilter, providerFilter, availableOnly, showHidden, hiddenSet],
  );

  const hiddenCount = useMemo(() => allModels.filter(m => hiddenSet.has(modelKey(m))).length, [allModels, hiddenSet]);

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

  const inputClass = "w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";

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
            <Button variant="primary" onClick={() => setShowAdd(true)}>
              <Plus className="w-4 h-4" /> {t("models.add_model")}
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
            leftIcon={<Search className="h-4 w-4" />} />
        </div>

        <select value={providerFilter} onChange={e => setProviderFilter(e.target.value)}
          className="rounded-xl border border-border-subtle bg-surface px-3 py-2.5 text-xs outline-none focus:border-brand">
          {providers.map(p => <option key={p} value={p}>{p === "all" ? t("models.all_providers") : p}</option>)}
        </select>

        <div className="flex gap-0.5 rounded-xl border border-border-subtle bg-surface p-0.5 flex-wrap overflow-x-auto">
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
      </div>

      <p className="text-xs text-text-dim">{filtered.length} {t("models.results")}</p>

      {/* Model List */}
      {modelsQuery.isLoading ? (
        <ListSkeleton rows={5} />
      ) : filtered.length === 0 ? (
        <div className="text-center py-16">
          <Cpu className="w-10 h-10 text-text-dim/20 mx-auto mb-3" />
          <p className="text-sm text-text-dim">{allModels.length === 0 ? t("models.no_models") : t("models.no_results")}</p>
        </div>
      ) : (
        <div className="rounded-2xl border border-border-subtle overflow-hidden overflow-x-auto">
          <div className="grid grid-cols-[minmax(160px,1fr)_100px_80px_80px_80px_50px_50px_50px_80px] min-w-[780px] gap-3 px-5 py-3 bg-main text-[11px] font-bold text-text-dim/60 uppercase">
            <span>{t("models.col_model")}</span>
            <span>{t("models.col_provider")}</span>
            <span>{t("models.col_tier")}</span>
            <span>{t("models.col_context")}</span>
            <span>{t("models.col_input")}</span>
            <span className="text-center" title={t("models.col_tools")}><Wrench className="w-3.5 h-3.5 inline" /></span>
            <span className="text-center" title={t("models.col_vision")}><Eye className="w-3.5 h-3.5 inline" /></span>
            <span className="text-center" title={t("models.col_streaming")}><Zap className="w-3.5 h-3.5 inline" /></span>
            <span></span>
          </div>

          {filtered.map((m, i) => {
            const isCustom = m.tier === "custom";
            return (
              <div key={`${m.provider}:${m.id}`}
                className={`grid grid-cols-[minmax(160px,1fr)_100px_80px_80px_80px_50px_50px_50px_80px] min-w-[780px] gap-3 px-5 py-3 items-center border-t border-border-subtle/50 hover:bg-surface transition-colors ${
                  !m.available ? "opacity-40" : ""
                } ${i % 2 === 0 ? "" : "bg-main/30"}`}>
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
                    {isCustom && (
                      <Sparkles className="w-3 h-3 text-violet-500 shrink-0" />
                    )}
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
                <span className="text-center">{m.supports_tools ? <Check className="w-4 h-4 text-success inline" /> : <X className="w-4 h-4 text-text-dim/15 inline" />}</span>
                <span className="text-center">{m.supports_vision ? <Check className="w-4 h-4 text-success inline" /> : <X className="w-4 h-4 text-text-dim/15 inline" />}</span>
                <span className="text-center">{m.supports_streaming ? <Check className="w-4 h-4 text-success inline" /> : <X className="w-4 h-4 text-text-dim/15 inline" />}</span>
                <span className="flex items-center justify-center gap-1">
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
            );
          })}
        </div>
      )}

      {/* Add Model Modal */}
      {showAdd && (
        <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/30 backdrop-blur-sm" onClick={() => resetForm()}>
          <div className="bg-surface rounded-t-2xl sm:rounded-2xl shadow-2xl border border-border-subtle w-full sm:w-[480px] sm:max-w-[90vw] animate-fade-in-scale" onClick={e => e.stopPropagation()}>
            <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle">
              <h3 className="text-sm font-bold">{t("models.add_custom_model")}</h3>
              <button onClick={() => resetForm()} className="p-1 rounded hover:bg-main"><X className="w-4 h-4" /></button>
            </div>
            <form onSubmit={handleAdd} className="p-5 space-y-4 max-h-[70vh] overflow-y-auto">
              <div className="grid grid-cols-2 gap-3">
                <div className="col-span-2">
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
          </div>
        </div>
      )}
    </div>
  );
}
