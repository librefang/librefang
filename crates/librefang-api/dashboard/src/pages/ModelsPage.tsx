import { formatCost as formatCostUtil } from "../lib/format";
import type { ModelItem, ModelOverrides } from "../api";
import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useModels, useModelOverrides } from "../lib/queries/models";
import { useAddCustomModel, useRemoveCustomModel, useUpdateModelOverrides, useDeleteModelOverrides } from "../lib/mutations/models";
import { SliderInput } from "../components/ui/SliderInput";
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
  Cpu, Search, Check, Eye, EyeOff, Wrench, Zap, AlertCircle, Lock, Plus, Trash2, Loader2,
  Brain, Tag, Settings,
} from "lucide-react";
import { modelKey } from "../lib/hiddenModels";

// ── Helpers ───────────────────────────────────────────────────────

const tierClass = (tier?: string) => {
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

const formatCtx = (tokens?: number) => {
  if (!tokens) return "—";
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(tokens % 1_000_000 === 0 ? 0 : 1)}M`;
  if (tokens >= 1_000) return `${Math.round(tokens / 1_000)}K`;
  return String(tokens);
};

// ── ModelCard ─────────────────────────────────────────────────────

type CardProps = {
  m: ModelItem;
  hidden: boolean;
  onOpen: () => void;
  onSettings: () => void;
  onToggleHidden: () => void;
  onDelete: () => void;
  pendingDelete: boolean;
};

function ModelCard({ m, hidden, onOpen, onSettings, onToggleHidden, onDelete, pendingDelete }: CardProps) {
  const { t } = useTranslation();
  const isCustom = m.tier === "custom";
  const free = (m.input_cost_per_m ?? 0) === 0 && (m.output_cost_per_m ?? 0) === 0;

  const formatCost = (cost?: number) => {
    if (cost === undefined || cost === null) return "—";
    if (cost === 0) return "0";
    return formatCostUtil(cost);
  };

  return (
    <div
      onClick={onOpen}
      className={`group relative flex flex-col gap-2.5 p-4 rounded-2xl border bg-surface hover:bg-main/40 hover:border-brand/40 transition-colors cursor-pointer min-h-[124px] ${
        hidden ? "border-warning/30 bg-warning/5" : "border-border-subtle"
      } ${!m.available ? "opacity-60" : ""}`}
    >
      {/* Top row: name + tier */}
      <div className="flex items-start gap-2 min-w-0">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5">
            {m.available
              ? <span className="w-1.5 h-1.5 rounded-full bg-success shrink-0" title={t("models.available")} />
              : <Lock className="w-3 h-3 text-text-dim/60 shrink-0" />}
            <span className="text-sm font-bold text-text truncate">{m.display_name || m.id}</span>
          </div>
          <div className="text-[10px] font-mono text-text-dim truncate mt-0.5">{m.provider}/{m.id}</div>
        </div>
        {m.tier && (
          <span className={`px-1.5 py-0.5 rounded text-[9px] font-bold uppercase tracking-wide shrink-0 ${tierClass(m.tier)}`}>
            {t(`models.tier_${m.tier}`, { defaultValue: m.tier })}
          </span>
        )}
      </div>

      {/* Middle row: context + cost */}
      <div className="flex items-center gap-3 text-[11px] text-text-dim">
        <span className="font-mono" title={t("models.context_window")}>{formatCtx(m.context_window)}</span>
        <span className="text-border-subtle">·</span>
        {free
          ? <span className="font-mono text-success font-bold">{t("models.free")}</span>
          : (
            <span className="font-mono">
              <span className="text-text" title={t("models.col_input")}>${formatCost(m.input_cost_per_m)}</span>
              <span className="text-text-dim/50"> / </span>
              <span className="text-text" title={t("models.col_output")}>${formatCost(m.output_cost_per_m)}</span>
              <span className="text-text-dim/40"> / M</span>
            </span>
          )}
      </div>

      {/* Bottom row: capabilities */}
      <div className="flex items-center gap-1.5 mt-auto">
        {[
          { on: m.supports_tools, Icon: Wrench, label: t("models.col_tools") },
          { on: m.supports_vision, Icon: Eye, label: t("models.col_vision") },
          { on: m.supports_streaming, Icon: Zap, label: t("models.col_streaming") },
          { on: m.supports_thinking, Icon: Brain, label: t("models.col_thinking") },
        ].map(({ on, Icon, label }, i) => (
          <span key={i} title={label}
            className={`flex items-center justify-center w-6 h-6 rounded-md ${
              on ? "bg-brand/10 text-brand" : "bg-main/40 text-text-dim/30"
            }`}>
            <Icon className="w-3 h-3" />
          </span>
        ))}
        {(m.aliases?.length ?? 0) > 0 && (
          <span className="ml-1 inline-flex items-center gap-1 text-[9px] text-text-dim font-mono" title={(m.aliases ?? []).join(", ")}>
            <Tag className="w-2.5 h-2.5" />
            {m.aliases!.length}
          </span>
        )}

        {/* Hover-revealed actions */}
        <div className="ml-auto flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
          <button type="button" title={t("models.settings_title")}
            onClick={(e) => { e.stopPropagation(); onSettings(); }}
            className="flex items-center justify-center w-6 h-6 rounded-md text-text-dim hover:bg-main hover:text-text transition-colors">
            <Settings className="w-3 h-3" />
          </button>
          <button type="button" title={hidden ? t("models.unhide_model") : t("models.hide_model")}
            onClick={(e) => { e.stopPropagation(); onToggleHidden(); }}
            className="flex items-center justify-center w-6 h-6 rounded-md text-text-dim hover:bg-main hover:text-text transition-colors">
            {hidden ? <Eye className="w-3 h-3" /> : <EyeOff className="w-3 h-3" />}
          </button>
          {isCustom && (
            <button type="button" title={t("models.delete_model")}
              onClick={(e) => { e.stopPropagation(); onDelete(); }}
              className={`flex items-center justify-center w-6 h-6 rounded-md transition-colors ${
                pendingDelete ? "bg-error/15 text-error" : "text-text-dim hover:bg-error/10 hover:text-error"
              }`}>
              {pendingDelete ? <Check className="w-3 h-3" /> : <Trash2 className="w-3 h-3" />}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

// ── ModelsPage ────────────────────────────────────────────────────

export function ModelsPage() {
  const { t } = useTranslation();
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
  const hiddenModelKeys = useUIStore((s) => s.hiddenModelKeys);
  const hideModelAction = useUIStore((s) => s.hideModel);
  const unhideModelAction = useUIStore((s) => s.unhideModel);
  const pruneHiddenKeys = useUIStore((s) => s.pruneHiddenKeys);
  const [settingsModel, setSettingsModel] = useState<ModelItem | null>(null);
  const [detailModel, setDetailModel] = useState<ModelItem | null>(null);

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

  const modelsQuery = useModels();
  const addMut = useAddCustomModel();
  const deleteMut = useRemoveCustomModel();

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
    try {
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
      addToast(t("models.model_added"), "success");
      resetForm();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      addToast(msg || t("common.error"), "error");
    }
  };

  const handleDelete = async (id: string) => {
    if (confirmDeleteId !== id) { setConfirmDeleteId(id); return; }
    setConfirmDeleteId(null);
    try {
      const model = allModels.find(m => m.id === id);
      const key = model ? modelKey(model) : null;
      await deleteMut.mutateAsync(id);
      addToast(t("models.model_deleted"), "success");
      if (key && hiddenModelKeys.includes(key)) unhideModelAction(key);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      addToast(msg || t("common.error"), "error");
    }
  };

  const allModels = useMemo(
    () => [...(modelsQuery.data?.models ?? [])].sort((a, b) => {
      if (a.available && !b.available) return -1;
      if (!a.available && b.available) return 1;
      return (a.display_name || a.id).localeCompare(b.display_name || b.id);
    }),
    [modelsQuery.data],
  );
  const totalAvailable = modelsQuery.data?.available ?? 0;

  const { providers, tiers } = useMemo(() => {
    const providerSet = new Set<string>();
    const tierSet = new Set<string>();
    for (const m of allModels) {
      providerSet.add(m.provider);
      if (m.tier) tierSet.add(m.tier);
    }
    return {
      providers: ["all", ...Array.from(providerSet).sort()],
      tiers: ["all", ...Array.from(tierSet).sort()],
    };
  }, [allModels]);

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

  // Always group by provider — no toggle, sticky headers do the work.
  const grouped = useMemo(() => {
    const map = new Map<string, ModelItem[]>();
    for (const m of filtered) {
      const list = map.get(m.provider);
      if (list) list.push(m);
      else map.set(m.provider, [m]);
    }
    return new Map([...map.entries()].sort(([a], [b]) => a.localeCompare(b)));
  }, [filtered]);

  const toggleHidden = (m: ModelItem) => {
    const key = modelKey(m);
    if (hiddenSet.has(key)) {
      unhideModelAction(key);
      addToast(t("models.model_unhidden"), "success");
    } else {
      hideModelAction(key);
      addToast(t("models.model_hidden"), "success");
    }
  };

  const inputClass = "w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";

  return (
    <div className="flex flex-col gap-5 transition-colors duration-300">
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

      {/* Filter bar — search + provider + tier + hidden toggle */}
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex-1 min-w-[200px] max-w-md">
          <Input value={search} onChange={e => setSearch(e.target.value)}
            placeholder={t("models.search_placeholder")}
            leftIcon={<Search className="h-4 w-4" />}
            data-shortcut-search />
        </div>

        <select value={providerFilter} onChange={e => setProviderFilter(e.target.value)}
          className="rounded-xl border border-border-subtle bg-surface px-3 py-2 text-xs outline-none focus:border-brand cursor-pointer">
          {providers.map(p => <option key={p} value={p}>{p === "all" ? t("models.all_providers") : p}</option>)}
        </select>

        <select value={tierFilter} onChange={e => setTierFilter(e.target.value)}
          className="rounded-xl border border-border-subtle bg-surface px-3 py-2 text-xs outline-none focus:border-brand cursor-pointer">
          {tiers.map(tier => (
            <option key={tier} value={tier}>
              {tier === "all" ? t("models.all_tiers") : t(`models.tier_${tier}`, { defaultValue: tier })}
            </option>
          ))}
        </select>

        <button onClick={() => setAvailableOnly(!availableOnly)}
          title={t("models.available_only")}
          className={`flex items-center gap-1 px-2.5 py-2 rounded-xl border text-xs font-bold transition-colors ${
            availableOnly ? "border-success bg-success/10 text-success" : "border-border-subtle text-text-dim hover:border-brand/30"
          }`}>
          <Check className="w-3 h-3" />
          <span className="hidden sm:inline">{t("models.available_only")}</span>
        </button>

        {hiddenCount > 0 && (
          <button onClick={() => setShowHidden(!showHidden)}
            title={t("models.show_hidden")}
            className={`flex items-center gap-1 px-2.5 py-2 rounded-xl border text-xs font-bold transition-colors ${
              showHidden ? "border-warning bg-warning/10 text-warning" : "border-border-subtle text-text-dim hover:border-brand/30"
            }`}>
            <EyeOff className="w-3 h-3" />
            <span>{hiddenCount}</span>
          </button>
        )}

        <span className="text-[11px] text-text-dim ml-auto">{filtered.length} {t("models.results")}</span>
      </div>

      {/* Model grid — always grouped by provider with sticky headers */}
      {modelsQuery.isLoading ? (
        <ListSkeleton rows={5} />
      ) : filtered.length === 0 ? (
        <EmptyState
          icon={<Cpu className="w-7 h-7" />}
          title={allModels.length === 0 ? t("models.no_models") : t("models.no_results")}
        />
      ) : (
        <div className="flex flex-col gap-6">
          {Array.from(grouped.entries()).map(([provider, models]) => {
            const availCount = models.filter(m => m.available).length;
            return (
              <section key={provider}>
                <header className="sticky top-0 z-10 flex items-center gap-3 -mx-2 px-2 py-2 mb-2 backdrop-blur-md bg-bg/85 border-b border-border-subtle/40">
                  <span className="text-sm font-bold text-text">{provider}</span>
                  <span className="px-1.5 py-0.5 rounded-md bg-brand/10 text-brand text-[10px] font-bold tabular-nums">{models.length}</span>
                  {availCount < models.length && (
                    <span className="text-[10px] text-text-dim">
                      {availCount} {t("models.available")}
                    </span>
                  )}
                </header>
                <div className="grid gap-3 grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
                  {models.map(m => (
                    <ModelCard
                      key={`${m.provider}:${m.id}`}
                      m={m}
                      hidden={hiddenSet.has(modelKey(m))}
                      onOpen={() => setDetailModel(m)}
                      onSettings={() => setSettingsModel(m)}
                      onToggleHidden={() => toggleHidden(m)}
                      onDelete={() => handleDelete(m.id)}
                      pendingDelete={confirmDeleteId === m.id}
                    />
                  ))}
                </div>
              </section>
            );
          })}
        </div>
      )}

      {/* Detail drawer (preview before going into Settings) */}
      {detailModel && (
        <Modal isOpen onClose={() => setDetailModel(null)} title={detailModel.display_name || detailModel.id} size="md" variant="panel-right">
          <div className="p-5 space-y-4 text-sm">
            <div>
              <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.col_provider")}</div>
              <div className="font-mono text-xs">{detailModel.provider}/{detailModel.id}</div>
            </div>
            {detailModel.aliases && detailModel.aliases.length > 0 && (
              <div>
                <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.aliases")}</div>
                <div className="flex flex-wrap gap-1">
                  {detailModel.aliases.map(a => (
                    <span key={a} className="px-2 py-0.5 rounded-md bg-main text-[10px] font-mono">{a}</span>
                  ))}
                </div>
              </div>
            )}
            <div className="grid grid-cols-2 gap-3">
              <div>
                <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.col_tier")}</div>
                {detailModel.tier ? (
                  <span className={`inline-block px-2 py-0.5 rounded text-[10px] font-bold uppercase ${tierClass(detailModel.tier)}`}>
                    {t(`models.tier_${detailModel.tier}`, { defaultValue: detailModel.tier })}
                  </span>
                ) : "—"}
              </div>
              <div>
                <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.col_context")}</div>
                <span className="font-mono">{formatCtx(detailModel.context_window)}</span>
              </div>
              <div>
                <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.col_input")}</div>
                <span className="font-mono">${detailModel.input_cost_per_m ?? 0} / M</span>
              </div>
              <div>
                <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.col_output")}</div>
                <span className="font-mono">${detailModel.output_cost_per_m ?? 0} / M</span>
              </div>
              <div>
                <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.max_output")}</div>
                <span className="font-mono">{formatCtx(detailModel.max_output_tokens)}</span>
              </div>
              <div>
                <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.section")}</div>
                <span>{detailModel.available ? <span className="text-success font-bold">●</span> : <span className="text-text-dim">○</span>} {detailModel.available ? t("models.available") : t("models.no_key")}</span>
              </div>
            </div>
            <div>
              <div className="text-[10px] font-bold text-text-dim uppercase mb-1.5">{t("models.capabilities")}</div>
              <div className="flex flex-wrap gap-2">
                {([
                  ["tools", detailModel.supports_tools, Wrench] as const,
                  ["vision", detailModel.supports_vision, Eye] as const,
                  ["streaming", detailModel.supports_streaming, Zap] as const,
                  ["thinking", detailModel.supports_thinking, Brain] as const,
                ]).map(([key, on, Icon]) => (
                  <span key={key} className={`flex items-center gap-1.5 px-2.5 py-1 rounded-lg border text-[11px] font-bold ${
                    on ? "border-brand/30 bg-brand/10 text-brand" : "border-border-subtle text-text-dim/40"
                  }`}>
                    <Icon className="w-3 h-3" />
                    {t(`models.col_${key}`)}
                  </span>
                ))}
              </div>
            </div>
            <div className="flex gap-2 pt-2 border-t border-border-subtle/50">
              <Button variant="primary" className="flex-1" onClick={() => { setSettingsModel(detailModel); setDetailModel(null); }}>
                <Settings className="w-4 h-4 mr-1.5" />
                {t("models.settings_title")}
              </Button>
              <Button variant="secondary" onClick={() => { toggleHidden(detailModel); setDetailModel(null); }}>
                {hiddenSet.has(modelKey(detailModel)) ? <Eye className="w-4 h-4" /> : <EyeOff className="w-4 h-4" />}
              </Button>
            </div>
          </div>
        </Modal>
      )}

      {/* Add Model Modal */}
      <Modal isOpen={showAdd} onClose={resetForm} title={t("models.add_custom_model")} size="lg" variant="panel-right">
        <form onSubmit={handleAdd} className="p-5 space-y-4">
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
            <div className="flex items-center gap-2 text-error text-xs"><AlertCircle className="w-4 h-4" /> {(addMut.error as Error)?.message}</div>
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

      {/* Model Settings Modal */}
      {settingsModel && (
        <ModelSettingsModal
          model={settingsModel}
          onClose={() => setSettingsModel(null)}
          onSaved={() => {
            modelsQuery.refetch();
            addToast(t("models.overrides_saved"), "success");
          }}
          onReset={() => {
            modelsQuery.refetch();
            addToast(t("models.overrides_reset"), "success");
          }}
          onError={(msg) => addToast(msg || t("models.overrides_error"), "error")}
        />
      )}
    </div>
  );
}

// ── Toggle helper (defined outside render to avoid remount) ──────

function SettingsToggle({ value, onChange, label }: { value: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <label className="flex items-center justify-between gap-2 py-1.5 cursor-pointer">
      <span className="text-xs text-text">{label}</span>
      <button type="button" onClick={() => onChange(!value)}
        className={`relative w-9 h-5 rounded-full transition-colors cursor-pointer ${value ? "bg-brand" : "bg-border-subtle"}`}>
        <span className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${value ? "translate-x-4.5" : "translate-x-0.5"}`} />
      </button>
    </label>
  );
}

// ── Model Settings Modal ──────────────────────────────────────────

function ModelSettingsModal({ model, onClose, onSaved, onReset, onError }: {
  model: ModelItem;
  onClose: () => void;
  onSaved: () => void;
  onReset: () => void;
  onError: (msg?: string) => void;
}) {
  const { t } = useTranslation();
  const overrideKey = `${model.provider}:${model.id}`;

  const overridesQuery = useModelOverrides(overrideKey);
  const updateMut = useUpdateModelOverrides();
  const deleteMut = useDeleteModelOverrides();

  const [saving, setSaving] = useState(false);

  const [modelType, setModelType] = useState<"chat" | "speech" | "embedding">("chat");
  const [temperature, setTemperature] = useState(0.7);
  const [tempEnabled, setTempEnabled] = useState(false);
  const [topP, setTopP] = useState(1.0);
  const [topPEnabled, setTopPEnabled] = useState(false);
  const [maxTokens, setMaxTokens] = useState(4096);
  const [maxTokensEnabled, setMaxTokensEnabled] = useState(false);
  const [freqPenalty, setFreqPenalty] = useState(0.0);
  const [freqEnabled, setFreqEnabled] = useState(false);
  const [presPenalty, setPresPenalty] = useState(0.0);
  const [presEnabled, setPresEnabled] = useState(false);
  const [reasoningEffort, setReasoningEffort] = useState<string>("");
  const [useMaxCompletionTokens, setUseMaxCompletionTokens] = useState(false);
  const [noSystemRole, setNoSystemRole] = useState(false);
  const [forceMaxTokens, setForceMaxTokens] = useState(false);
  const [overridesLoaded, setOverridesLoaded] = useState(false);

  // Load existing overrides from query
  useEffect(() => {
    const o = overridesQuery.data;
    if (!o || overridesLoaded) return;
    if (o.model_type) setModelType(o.model_type);
    if (o.temperature != null) { setTemperature(o.temperature); setTempEnabled(true); }
    if (o.top_p != null) { setTopP(o.top_p); setTopPEnabled(true); }
    if (o.max_tokens != null) { setMaxTokens(o.max_tokens); setMaxTokensEnabled(true); }
    if (o.frequency_penalty != null) { setFreqPenalty(o.frequency_penalty); setFreqEnabled(true); }
    if (o.presence_penalty != null) { setPresPenalty(o.presence_penalty); setPresEnabled(true); }
    if (o.reasoning_effort) setReasoningEffort(o.reasoning_effort);
    if (o.use_max_completion_tokens != null) setUseMaxCompletionTokens(o.use_max_completion_tokens);
    if (o.no_system_role != null) setNoSystemRole(o.no_system_role);
    if (o.force_max_tokens != null) setForceMaxTokens(o.force_max_tokens);
    setOverridesLoaded(true);
  }, [overridesQuery.data, overridesLoaded]);

  const handleSave = useCallback(async () => {
    setSaving(true);
    const overrides: ModelOverrides = {};
    if (modelType !== "chat") overrides.model_type = modelType;
    if (tempEnabled) overrides.temperature = temperature;
    if (topPEnabled) overrides.top_p = topP;
    if (maxTokensEnabled) overrides.max_tokens = maxTokens;
    if (freqEnabled) overrides.frequency_penalty = freqPenalty;
    if (presEnabled) overrides.presence_penalty = presPenalty;
    if (reasoningEffort) overrides.reasoning_effort = reasoningEffort;
    if (useMaxCompletionTokens) overrides.use_max_completion_tokens = true;
    if (noSystemRole) overrides.no_system_role = true;
    if (forceMaxTokens) overrides.force_max_tokens = true;
    try {
      await updateMut.mutateAsync({ modelKey: overrideKey, overrides });
      onSaved();
      onClose();
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      onError(msg);
    } finally {
      setSaving(false);
    }
  }, [overrideKey, modelType, temperature, tempEnabled, topP, topPEnabled, maxTokens, maxTokensEnabled, freqPenalty, freqEnabled, presPenalty, presEnabled, reasoningEffort, useMaxCompletionTokens, noSystemRole, forceMaxTokens, onSaved, onClose, onError, updateMut]);

  const handleReset = useCallback(async () => {
    try {
      await deleteMut.mutateAsync(overrideKey);
      onReset();
      onClose();
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      onError(msg);
    }
  }, [overrideKey, onReset, onClose, onError, deleteMut]);

  if (overridesQuery.isLoading) {
    return (
      <Modal isOpen onClose={onClose} title={t("models.settings_title")} size="lg" variant="panel-right">
        <div className="flex items-center justify-center p-12">
          <Loader2 className="w-6 h-6 animate-spin text-brand" />
        </div>
      </Modal>
    );
  }

  return (
    <Modal isOpen onClose={onClose} title={t("models.settings_title")} size="lg" variant="panel-right">
      <div className="p-5 space-y-5">
        {/* Model header */}
        <div className="flex items-center gap-3">
          <Cpu className="w-5 h-5 text-brand" />
          <div>
            <p className="text-sm font-bold">{model.display_name || model.id}</p>
            <p className="text-[10px] text-text-dim font-mono">{model.provider}:{model.id}</p>
          </div>
        </div>

        {/* Model Type */}
        <div className="space-y-1.5">
          <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.model_type")}</label>
          <div className="flex gap-0.5 rounded-xl border border-border-subtle bg-surface p-0.5">
            {(["chat", "speech", "embedding"] as const).map((mt) => (
              <button key={mt} type="button" onClick={() => setModelType(mt)}
                className={`flex-1 px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${
                  modelType === mt ? "bg-brand text-white shadow-sm" : "text-text-dim hover:text-text hover:bg-main"
                }`}>
                {t(`models.type_${mt}`)}
              </button>
            ))}
          </div>
        </div>

        {/* Capabilities */}
        <div className="space-y-1.5">
          <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.capabilities")}</label>
          <div className="flex flex-wrap gap-2">
            {([
              ["vision", model.supports_vision, Eye] as const,
              ["tools", model.supports_tools, Wrench] as const,
              ["thinking", model.supports_thinking, Brain] as const,
            ]).map(([key, supported, Icon]) => (
              <span key={key} className={`flex items-center gap-1.5 px-3 py-1.5 rounded-xl border text-xs font-bold ${
                supported ? "border-success/30 bg-success/10 text-success" : "border-border-subtle text-text-dim/40"
              }`}>
                <Icon className="w-3.5 h-3.5" />
                {t(`models.supports_${key}`)}
              </span>
            ))}
          </div>
        </div>

        {/* Parameters */}
        <div className="space-y-3">
          <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.parameters")}</label>

          <SliderInput
            label={t("models.context_window")}
            value={model.context_window ?? 128000}
            onChange={() => {}}
            min={1024} max={1048576} step={1024}
            enabled={false}
            ticks={[32768, 131072, 524288, 1048576]}
            formatTick={(v) => v >= 1048576 ? "1M" : `${Math.round(v/1024)}K`}
          />

          <SliderInput
            label={t("models.temperature")}
            value={temperature} onChange={setTemperature}
            min={0} max={2} step={0.01}
            enabled={tempEnabled} onToggle={setTempEnabled}
          />

          <SliderInput
            label={t("models.top_p")}
            value={topP} onChange={setTopP}
            min={0} max={1} step={0.01}
            enabled={topPEnabled} onToggle={setTopPEnabled}
          />

          <SliderInput
            label={t("models.max_tokens_param")}
            value={maxTokens} onChange={(v) => setMaxTokens(Math.round(v))}
            min={256} max={1048576} step={256}
            enabled={maxTokensEnabled} onToggle={setMaxTokensEnabled}
            ticks={[256, 32768, 131072, 1048576]}
            formatTick={(v) => v >= 1048576 ? "1M" : v >= 1024 ? `${Math.round(v/1024)}K` : String(v)}
          />

          <SliderInput
            label={t("models.frequency_penalty")}
            value={freqPenalty} onChange={setFreqPenalty}
            min={-2} max={2} step={0.01}
            enabled={freqEnabled} onToggle={setFreqEnabled}
            ticks={[-2, 0, 2]}
          />

          <SliderInput
            label={t("models.presence_penalty")}
            value={presPenalty} onChange={setPresPenalty}
            min={-2} max={2} step={0.01}
            enabled={presEnabled} onToggle={setPresEnabled}
            ticks={[-2, 0, 2]}
          />

          {/* Reasoning Effort */}
          <div className="space-y-1.5">
            <label className="text-xs font-bold text-text-dim">{t("models.reasoning_effort")}</label>
            <select value={reasoningEffort} onChange={(e) => setReasoningEffort(e.target.value)}
              className="w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-xs outline-none focus:border-brand">
              <option value="">—</option>
              <option value="low">{t("models.effort_low")}</option>
              <option value="medium">{t("models.effort_medium")}</option>
              <option value="high">{t("models.effort_high")}</option>
            </select>
          </div>
        </div>

        {/* Flags */}
        <div className="space-y-1">
          <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.flags")}</label>
          <SettingsToggle value={useMaxCompletionTokens} onChange={setUseMaxCompletionTokens} label={t("models.use_max_completion_tokens")} />
          <SettingsToggle value={noSystemRole} onChange={setNoSystemRole} label={t("models.no_system_role")} />
          <SettingsToggle value={forceMaxTokens} onChange={setForceMaxTokens} label={t("models.force_max_tokens")} />
        </div>

        {/* Actions */}
        <div className="flex gap-2 pt-2">
          <Button variant="primary" className="flex-1" onClick={handleSave} disabled={saving}>
            {saving && <Loader2 className="w-4 h-4 animate-spin mr-1" />}
            {t("common.save")}
          </Button>
          <Button variant="secondary" onClick={handleReset}>
            {t("models.reset_defaults")}
          </Button>
          <Button variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
