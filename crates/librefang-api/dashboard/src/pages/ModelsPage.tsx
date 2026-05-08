import { formatCost as formatCostUtil } from "../lib/format";
import type { ModelItem, ModelOverrides } from "../api";
import { FormEvent, memo, useCallback, useEffect, useReducer, useRef, useMemo, useState } from "react";
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
import { DrawerPanel } from "../components/ui/DrawerPanel";
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

// ── Add-form reducer (MD4) ──────────────────────────────────────

type AddFormState = {
  id: string;
  provider: string;
  displayName: string;
  contextWindow: number;
  maxOutput: number;
  inputCost: number;
  outputCost: number;
  tools: boolean;
  vision: boolean;
  streaming: boolean;
};

type AddFormAction =
  | { type: "SET_FIELD"; field: keyof AddFormState; value: AddFormState[keyof AddFormState] }
  | { type: "RESET" };

const addFormInitial: AddFormState = {
  id: "",
  provider: "",
  displayName: "",
  contextWindow: 128000,
  maxOutput: 8192,
  inputCost: 0,
  outputCost: 0,
  tools: true,
  vision: false,
  streaming: true,
};

function addFormReducer(state: AddFormState, action: AddFormAction): AddFormState {
  switch (action.type) {
    case "SET_FIELD":
      return { ...state, [action.field]: action.value };
    case "RESET":
      return addFormInitial;
    default:
      return state;
  }
}

// ── Settings-form reducer (MD5/MD6/MD7) ────────────────────────

// Tri-state capability override (refs #4745). `default` means "use the
// catalog/provider value"; `on` / `off` force the capability regardless of
// what the catalog declares. Maps to ModelOverrides on the wire as:
//   default → field absent (undefined)
//   on      → true
//   off     → false
type CapOverride = "default" | "on" | "off";

type SettingsState = {
  modelType: "chat" | "speech" | "embedding";
  temperature: number;
  tempEnabled: boolean;
  topP: number;
  topPEnabled: boolean;
  maxTokens: number;
  maxTokensEnabled: boolean;
  freqPenalty: number;
  freqEnabled: boolean;
  presPenalty: number;
  presEnabled: boolean;
  reasoningEffort: string;
  useMaxCompletionTokens: boolean;
  noSystemRole: boolean;
  forceMaxTokens: boolean;
  toolsOverride: CapOverride;
  visionOverride: CapOverride;
  streamingOverride: CapOverride;
  thinkingOverride: CapOverride;
};

type SettingsAction =
  | { type: "SET_FIELD"; field: keyof SettingsState; value: SettingsState[keyof SettingsState] }
  | { type: "HYDRATE"; payload: Partial<SettingsState> };

const settingsInitial: SettingsState = {
  modelType: "chat",
  temperature: 0.7,
  tempEnabled: false,
  topP: 1.0,
  topPEnabled: false,
  maxTokens: 4096,
  maxTokensEnabled: false,
  freqPenalty: 0.0,
  freqEnabled: false,
  presPenalty: 0.0,
  presEnabled: false,
  reasoningEffort: "",
  useMaxCompletionTokens: false,
  noSystemRole: false,
  forceMaxTokens: false,
  toolsOverride: "default",
  visionOverride: "default",
  streamingOverride: "default",
  thinkingOverride: "default",
};

function boolToOverride(v: boolean | undefined | null): CapOverride {
  if (v === true) return "on";
  if (v === false) return "off";
  return "default";
}

function overrideToBool(v: CapOverride): boolean | undefined {
  if (v === "on") return true;
  if (v === "off") return false;
  return undefined;
}

function settingsReducer(state: SettingsState, action: SettingsAction): SettingsState {
  switch (action.type) {
    case "SET_FIELD":
      return { ...state, [action.field]: action.value };
    case "HYDRATE":
      return { ...state, ...action.payload };
    default:
      return state;
  }
}

// ── ModelCard (MD2: memo, MD3: stable callback signatures) ──────

type CardProps = {
  m: ModelItem;
  hidden: boolean;
  onOpen: (m: ModelItem) => void;
  onSettings: (m: ModelItem) => void;
  onToggleHidden: (m: ModelItem) => void;
  onDelete: (id: string) => void;
  pendingDelete: boolean;
};

const ModelCard = memo(function ModelCard({ m, hidden, onOpen, onSettings, onToggleHidden, onDelete, pendingDelete }: CardProps) {
  const { t } = useTranslation();
  const isCustom = m.tier === "custom";
  const free = m.input_cost_per_m === 0 && m.output_cost_per_m === 0;

  const formatCost = (cost?: number) => {
    if (cost === undefined || cost === null) return "—";
    if (cost === 0) return "0";
    return formatCostUtil(cost);
  };

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={m.display_name || m.id}
      onClick={() => onOpen(m)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen(m);
        }
      }}
      className={`group relative flex flex-col gap-2.5 p-4 rounded-2xl border bg-surface hover:bg-main/40 hover:border-brand/40 focus-visible:outline-none focus-visible:border-brand focus-visible:ring-2 focus-visible:ring-brand/30 transition-colors cursor-pointer min-h-[124px] ${
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
            onClick={(e) => { e.stopPropagation(); onSettings(m); }}
            className="flex items-center justify-center w-6 h-6 rounded-md text-text-dim hover:bg-main hover:text-text transition-colors">
            <Settings className="w-3 h-3" />
          </button>
          <button type="button" title={hidden ? t("models.unhide_model") : t("models.hide_model")}
            onClick={(e) => { e.stopPropagation(); onToggleHidden(m); }}
            className="flex items-center justify-center w-6 h-6 rounded-md text-text-dim hover:bg-main hover:text-text transition-colors">
            {hidden ? <Eye className="w-3 h-3" /> : <EyeOff className="w-3 h-3" />}
          </button>
          {isCustom && (
            <button type="button" title={t("models.delete_model")}
              onClick={(e) => { e.stopPropagation(); onDelete(m.id); }}
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
});

// ── ModelDetailBody ───────────────────────────────────────────────
// Body rendered inside the global PushDrawer when a card is opened.
// Reads hiddenSet from UIStore directly so the toggle button reflects
// changes without re-pushing the whole drawer body to drawerStore.
function ModelDetailBody({
  m,
  hidden,
  onOpenSettings,
  onToggleHidden,
}: {
  m: ModelItem;
  hidden: boolean;
  onOpenSettings: () => void;
  onToggleHidden: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="p-5 space-y-4 text-sm">
      <div>
        <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.col_provider")}</div>
        <div className="font-mono text-xs">{m.provider}/{m.id}</div>
      </div>
      {m.aliases && m.aliases.length > 0 && (
        <div>
          <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.aliases")}</div>
          <div className="flex flex-wrap gap-1">
            {m.aliases.map((a) => (
              <span key={a} className="px-2 py-0.5 rounded-md bg-main text-[10px] font-mono">{a}</span>
            ))}
          </div>
        </div>
      )}
      <div className="grid grid-cols-2 gap-3">
        <div>
          <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.col_tier")}</div>
          {m.tier ? (
            <span className={`inline-block px-2 py-0.5 rounded text-[10px] font-bold uppercase ${tierClass(m.tier)}`}>
              {t(`models.tier_${m.tier}`, { defaultValue: m.tier })}
            </span>
          ) : "—"}
        </div>
        <div>
          <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.col_context")}</div>
          <span className="font-mono">{formatCtx(m.context_window)}</span>
        </div>
        <div>
          <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.col_input")}</div>
          <span className="font-mono">{formatCostUtil(m.input_cost_per_m ?? 0)} / M</span>
        </div>
        <div>
          <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.col_output")}</div>
          <span className="font-mono">{formatCostUtil(m.output_cost_per_m ?? 0)} / M</span>
        </div>
        <div>
          <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.max_output")}</div>
          <span className="font-mono">{formatCtx(m.max_output_tokens)}</span>
        </div>
        <div>
          <div className="text-[10px] font-bold text-text-dim uppercase mb-1">{t("models.availability")}</div>
          <span>{m.available ? <span className="text-success font-bold">●</span> : <span className="text-text-dim">○</span>} {m.available ? t("models.available") : t("models.no_key")}</span>
        </div>
      </div>
      <div>
        <div className="text-[10px] font-bold text-text-dim uppercase mb-1.5">{t("models.capabilities")}</div>
        <div className="flex flex-wrap gap-2">
          {([
            ["tools", m.supports_tools, Wrench] as const,
            ["vision", m.supports_vision, Eye] as const,
            ["streaming", m.supports_streaming, Zap] as const,
            ["thinking", m.supports_thinking, Brain] as const,
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
        <Button variant="primary" className="flex-1" onClick={onOpenSettings}>
          <Settings className="w-4 h-4 mr-1.5" />
          {t("models.settings_title")}
        </Button>
        <Button variant="secondary" onClick={onToggleHidden}>
          {hidden ? <Eye className="w-4 h-4" /> : <EyeOff className="w-4 h-4" />}
        </Button>
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

  const [form, dispatchForm] = useReducer(addFormReducer, addFormInitial);

  const modelsQuery = useModels();
  const addMut = useAddCustomModel();
  const deleteMut = useRemoveCustomModel();

  const resetForm = useCallback(() => {
    setShowAdd(false);
    dispatchForm({ type: "RESET" });
  }, []);

  const handleAdd = async (e: FormEvent) => {
    e.preventDefault();
    if (!form.id.trim() || !form.provider.trim()) return;
    try {
      await addMut.mutateAsync({
        id: form.id.trim(),
        provider: form.provider.trim(),
        display_name: form.displayName.trim() || undefined,
        context_window: form.contextWindow,
        max_output_tokens: form.maxOutput,
        input_cost_per_m: form.inputCost,
        output_cost_per_m: form.outputCost,
        supports_tools: form.tools,
        supports_vision: form.vision,
        supports_streaming: form.streaming,
      });
      addToast(t("models.model_added"), "success");
      resetForm();
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

  const grouped = useMemo(() => {
    const map = new Map<string, ModelItem[]>();
    for (const m of filtered) {
      const list = map.get(m.provider);
      if (list) list.push(m);
      else map.set(m.provider, [m]);
    }
    return new Map([...map.entries()].sort(([a], [b]) => a.localeCompare(b)));
  }, [filtered]);

  const toggleHidden = useCallback((m: ModelItem) => {
    const key = modelKey(m);
    if (hiddenSet.has(key)) {
      unhideModelAction(key);
      addToast(t("models.model_unhidden"), "success");
    } else {
      hideModelAction(key);
      addToast(t("models.model_hidden"), "success");
    }
  }, [hiddenSet, unhideModelAction, hideModelAction, addToast, t]);

  const handleCardOpen = useCallback((m: ModelItem) => setDetailModel(m), []);
  const handleCardSettings = useCallback((m: ModelItem) => setSettingsModel(m), []);

  const allModelsRef = useRef(allModels);
  allModelsRef.current = allModels;
  const hiddenModelKeysRef = useRef(hiddenModelKeys);
  hiddenModelKeysRef.current = hiddenModelKeys;
  const confirmDeleteIdRef = useRef(confirmDeleteId);
  confirmDeleteIdRef.current = confirmDeleteId;

  const handleDelete = useCallback(async (id: string) => {
    if (confirmDeleteIdRef.current !== id) { setConfirmDeleteId(id); return; }
    setConfirmDeleteId(null);
    try {
      const model = allModelsRef.current.find(m => m.id === id);
      const key = model ? modelKey(model) : null;
      await deleteMut.mutateAsync(id);
      addToast(t("models.model_deleted"), "success");
      if (key && hiddenModelKeysRef.current.includes(key)) unhideModelAction(key);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      addToast(msg || t("common.error"), "error");
    }
  }, [deleteMut, addToast, t, unhideModelAction]);

  const detailHidden = detailModel ? hiddenSet.has(modelKey(detailModel)) : false;

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
                      onOpen={handleCardOpen}
                      onSettings={handleCardSettings}
                      onToggleHidden={toggleHidden}
                      onDelete={handleDelete}
                      pendingDelete={confirmDeleteId === m.id}
                    />
                  ))}
                </div>
              </section>
            );
          })}
        </div>
      )}

      {/* Detail drawer — pushes content via the global slot, mirroring
          the sidebar collapse instead of overlaying the page. */}
      <DrawerPanel
        isOpen={!!detailModel}
        onClose={() => setDetailModel(null)}
        title={detailModel ? (detailModel.display_name || detailModel.id) : undefined}
        size="md"
      >
        {detailModel && (
          <ModelDetailBody
            m={detailModel}
            hidden={detailHidden}
            onOpenSettings={() => {
              setSettingsModel(detailModel);
              setDetailModel(null);
            }}
            onToggleHidden={() => {
              toggleHidden(detailModel);
              setDetailModel(null);
            }}
          />
        )}
      </DrawerPanel>

      {/* Add Model Modal */}
      <DrawerPanel isOpen={showAdd} onClose={resetForm} title={t("models.add_custom_model")} size="lg">
        <form onSubmit={handleAdd} className="p-5 space-y-4">
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <div className="sm:col-span-2">
              <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.model_id")} *</label>
              <input value={form.id} onChange={e => dispatchForm({ type: "SET_FIELD", field: "id", value: e.target.value })} placeholder={t("models.model_id_placeholder")} className={inputClass} required />
            </div>
            <div>
              <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.provider")} *</label>
              <input value={form.provider} onChange={e => dispatchForm({ type: "SET_FIELD", field: "provider", value: e.target.value })} placeholder={t("models.provider_placeholder")} className={inputClass} required />
            </div>
            <div>
              <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.display_name")}</label>
              <input value={form.displayName} onChange={e => dispatchForm({ type: "SET_FIELD", field: "displayName", value: e.target.value })} placeholder={t("models.display_name_placeholder")} className={inputClass} />
            </div>
            <div>
              <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.context_window")}</label>
              <input type="number" value={form.contextWindow} onChange={e => dispatchForm({ type: "SET_FIELD", field: "contextWindow", value: +e.target.value })} className={inputClass} />
            </div>
            <div>
              <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.max_output")}</label>
              <input type="number" value={form.maxOutput} onChange={e => dispatchForm({ type: "SET_FIELD", field: "maxOutput", value: +e.target.value })} className={inputClass} />
            </div>
            <div>
              <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.input_cost")}</label>
              <input type="number" step="0.01" value={form.inputCost} onChange={e => dispatchForm({ type: "SET_FIELD", field: "inputCost", value: +e.target.value })} className={inputClass} />
            </div>
            <div>
              <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.output_cost")}</label>
              <input type="number" step="0.01" value={form.outputCost} onChange={e => dispatchForm({ type: "SET_FIELD", field: "outputCost", value: +e.target.value })} className={inputClass} />
            </div>
          </div>
          <div className="flex flex-wrap gap-3">
            {([
              ["tools", form.tools, "tools", t("models.supports_tools")] as const,
              ["vision", form.vision, "vision", t("models.supports_vision")] as const,
              ["streaming", form.streaming, "streaming", t("models.supports_streaming")] as const,
            ]).map(([key, val, field, label]) => (
              <button key={key} type="button" onClick={() => dispatchForm({ type: "SET_FIELD", field, value: !val })}
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
            <Button type="submit" variant="primary" className="flex-1" disabled={addMut.isPending || !form.id.trim() || !form.provider.trim()}>
              {addMut.isPending ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Plus className="w-4 h-4 mr-1" />}
              {t("models.add_model")}
            </Button>
            <Button type="button" variant="secondary" onClick={() => resetForm()}>{t("common.cancel")}</Button>
          </div>
        </form>
      </DrawerPanel>

      {/* Model Settings Modal */}
      {settingsModel && (
        <ModelSettingsModal
          key={`${settingsModel.provider}:${settingsModel.id}`}
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

  const [state, dispatch] = useReducer(settingsReducer, settingsInitial);
  const stateRef = useRef(state);
  stateRef.current = state;
  const hydratedRef = useRef(false);

  useEffect(() => {
    if (hydratedRef.current) return;
    const o = overridesQuery.data;
    if (!o) return;
    const payload: Partial<SettingsState> = {};
    if (o.model_type) payload.modelType = o.model_type;
    if (o.temperature != null) { payload.temperature = o.temperature; payload.tempEnabled = true; }
    if (o.top_p != null) { payload.topP = o.top_p; payload.topPEnabled = true; }
    if (o.max_tokens != null) { payload.maxTokens = o.max_tokens; payload.maxTokensEnabled = true; }
    if (o.frequency_penalty != null) { payload.freqPenalty = o.frequency_penalty; payload.freqEnabled = true; }
    if (o.presence_penalty != null) { payload.presPenalty = o.presence_penalty; payload.presEnabled = true; }
    if (o.reasoning_effort) payload.reasoningEffort = o.reasoning_effort;
    if (o.use_max_completion_tokens != null) payload.useMaxCompletionTokens = o.use_max_completion_tokens;
    if (o.no_system_role != null) payload.noSystemRole = o.no_system_role;
    if (o.force_max_tokens != null) payload.forceMaxTokens = o.force_max_tokens;
    payload.toolsOverride = boolToOverride(o.supports_tools);
    payload.visionOverride = boolToOverride(o.supports_vision);
    payload.streamingOverride = boolToOverride(o.supports_streaming);
    payload.thinkingOverride = boolToOverride(o.supports_thinking);
    dispatch({ type: "HYDRATE", payload });
    hydratedRef.current = true;
  }, [overridesQuery.data]);

  const handleSave = useCallback(async () => {
    const s = stateRef.current;
    setSaving(true);
    const overrides: ModelOverrides = {};
    if (s.modelType !== "chat") overrides.model_type = s.modelType;
    if (s.tempEnabled) overrides.temperature = s.temperature;
    if (s.topPEnabled) overrides.top_p = s.topP;
    if (s.maxTokensEnabled) overrides.max_tokens = s.maxTokens;
    if (s.freqEnabled) overrides.frequency_penalty = s.freqPenalty;
    if (s.presEnabled) overrides.presence_penalty = s.presPenalty;
    if (s.reasoningEffort) overrides.reasoning_effort = s.reasoningEffort;
    if (s.useMaxCompletionTokens) overrides.use_max_completion_tokens = true;
    if (s.noSystemRole) overrides.no_system_role = true;
    if (s.forceMaxTokens) overrides.force_max_tokens = true;
    const tools = overrideToBool(s.toolsOverride);
    if (tools !== undefined) overrides.supports_tools = tools;
    const vision = overrideToBool(s.visionOverride);
    if (vision !== undefined) overrides.supports_vision = vision;
    const streaming = overrideToBool(s.streamingOverride);
    if (streaming !== undefined) overrides.supports_streaming = streaming;
    const thinking = overrideToBool(s.thinkingOverride);
    if (thinking !== undefined) overrides.supports_thinking = thinking;
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
  }, [overrideKey, updateMut, onSaved, onClose, onError]);

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
      <DrawerPanel isOpen onClose={onClose} title={t("models.settings_title")} size="lg">
        <div className="flex items-center justify-center p-12">
          <Loader2 className="w-6 h-6 animate-spin text-brand" />
        </div>
      </DrawerPanel>
    );
  }

  return (
    <DrawerPanel isOpen onClose={onClose} title={t("models.settings_title")} size="lg">
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
              <button key={mt} type="button" onClick={() => dispatch({ type: "SET_FIELD", field: "modelType", value: mt })}
                className={`flex-1 px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${
                  state.modelType === mt ? "bg-brand text-white shadow-sm" : "text-text-dim hover:text-text hover:bg-main"
                }`}>
                {t(`models.type_${mt}`)}
              </button>
            ))}
          </div>
        </div>

        {/* Capabilities — refs #4745: editable per-model overrides.
            Each capability has a tri-state segmented control:
              Auto   = use catalog/provider default (no override stored)
              On     = force capability on regardless of catalog
              Off    = force capability off regardless of catalog
            Useful when the provider's `capabilities` field is wrong, missing,
            or non-standard. The "Auto" segment shows the catalog default in
            parens so the user can see what they'd revert to. */}
        <div className="space-y-1.5">
          <label className="text-[10px] font-bold text-text-dim uppercase">{t("models.capabilities")}</label>
          <p className="text-[11px] text-text-dim leading-snug">{t("models.capabilities_override_hint")}</p>
          <div className="space-y-1.5">
            {([
              ["tools", state.toolsOverride, model.supports_tools, Wrench, "toolsOverride"] as const,
              ["vision", state.visionOverride, model.supports_vision, Eye, "visionOverride"] as const,
              ["streaming", state.streamingOverride, model.supports_streaming, Zap, "streamingOverride"] as const,
              ["thinking", state.thinkingOverride, model.supports_thinking, Brain, "thinkingOverride"] as const,
            ]).map(([key, current, catalogDefault, Icon, field]) => (
              <div key={key} className="flex items-center gap-3">
                <span className="flex items-center gap-1.5 text-xs font-bold text-text min-w-[6.5rem]">
                  <Icon className="w-3.5 h-3.5" />
                  {t(`models.supports_${key}`)}
                </span>
                <div className="flex flex-1 gap-0.5 rounded-xl border border-border-subtle bg-surface p-0.5">
                  {(["default", "on", "off"] as const).map((opt) => {
                    const label = opt === "default"
                      ? `${t("models.cap_auto")} (${catalogDefault ? t("models.cap_on") : t("models.cap_off")})`
                      : opt === "on" ? t("models.cap_force_on") : t("models.cap_force_off");
                    return (
                      <button
                        key={opt}
                        type="button"
                        onClick={() => dispatch({ type: "SET_FIELD", field, value: opt })}
                        className={`flex-1 px-2 py-1.5 rounded-lg text-[11px] font-bold transition-colors ${
                          current === opt ? "bg-brand text-white shadow-sm" : "text-text-dim hover:text-text hover:bg-main"
                        }`}>
                        {label}
                      </button>
                    );
                  })}
                </div>
              </div>
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
            value={state.temperature} onChange={(v) => dispatch({ type: "SET_FIELD", field: "temperature", value: v })}
            min={0} max={2} step={0.01}
            enabled={state.tempEnabled} onToggle={(v) => dispatch({ type: "SET_FIELD", field: "tempEnabled", value: v })}
          />

          <SliderInput
            label={t("models.top_p")}
            value={state.topP} onChange={(v) => dispatch({ type: "SET_FIELD", field: "topP", value: v })}
            min={0} max={1} step={0.01}
            enabled={state.topPEnabled} onToggle={(v) => dispatch({ type: "SET_FIELD", field: "topPEnabled", value: v })}
          />

          <SliderInput
            label={t("models.max_tokens_param")}
            value={state.maxTokens} onChange={(v) => dispatch({ type: "SET_FIELD", field: "maxTokens", value: Math.round(v) })}
            min={256} max={1048576} step={256}
            enabled={state.maxTokensEnabled} onToggle={(v) => dispatch({ type: "SET_FIELD", field: "maxTokensEnabled", value: v })}
            ticks={[256, 32768, 131072, 1048576]}
            formatTick={(v) => v >= 1048576 ? "1M" : v >= 1024 ? `${Math.round(v/1024)}K` : String(v)}
          />

          <SliderInput
            label={t("models.frequency_penalty")}
            value={state.freqPenalty} onChange={(v) => dispatch({ type: "SET_FIELD", field: "freqPenalty", value: v })}
            min={-2} max={2} step={0.01}
            enabled={state.freqEnabled} onToggle={(v) => dispatch({ type: "SET_FIELD", field: "freqEnabled", value: v })}
            ticks={[-2, 0, 2]}
          />

          <SliderInput
            label={t("models.presence_penalty")}
            value={state.presPenalty} onChange={(v) => dispatch({ type: "SET_FIELD", field: "presPenalty", value: v })}
            min={-2} max={2} step={0.01}
            enabled={state.presEnabled} onToggle={(v) => dispatch({ type: "SET_FIELD", field: "presEnabled", value: v })}
            ticks={[-2, 0, 2]}
          />

          {/* Reasoning Effort */}
          <div className="space-y-1.5">
            <label className="text-xs font-bold text-text-dim">{t("models.reasoning_effort")}</label>
            <select value={state.reasoningEffort} onChange={(e) => dispatch({ type: "SET_FIELD", field: "reasoningEffort", value: e.target.value })}
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
          <SettingsToggle value={state.useMaxCompletionTokens} onChange={(v) => dispatch({ type: "SET_FIELD", field: "useMaxCompletionTokens", value: v })} label={t("models.use_max_completion_tokens")} />
          <SettingsToggle value={state.noSystemRole} onChange={(v) => dispatch({ type: "SET_FIELD", field: "noSystemRole", value: v })} label={t("models.no_system_role")} />
          <SettingsToggle value={state.forceMaxTokens} onChange={(v) => dispatch({ type: "SET_FIELD", field: "forceMaxTokens", value: v })} label={t("models.force_max_tokens")} />
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
    </DrawerPanel>
  );
}
