import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { Plus, Trash2, Pencil, ArrowLeft, Loader2 } from "lucide-react";
import type { MoaPreset, MoaPrivacyFilter, MoaSlot } from "../../api";
import { getMoaConfig } from "../../api";
import { useMoaPresets } from "../../lib/queries/moa";
import { usePutMoaPreset, useDeleteMoaPreset, usePutMoaConfig } from "../../lib/mutations/moa";
import { useProviders } from "../../lib/queries/providers";
import { useModels } from "../../lib/queries/models";
import { isProviderAvailable } from "../../lib/status";
import { filterVisible, modelKey } from "../../lib/hiddenModels";
import { useUIStore } from "../../lib/store";
import { DrawerPanel } from "../ui/DrawerPanel";
import { Button } from "../ui/Button";
import { Badge } from "../ui/Badge";
import { Select } from "../ui/Select";

// Mirrors `useMoaPresets`'s read shape (see lib/queries/moa.ts): a react-query
// hook that pulls the full normalized `MoaConfig` from `GET /api/moa`. Enabled
// only while the drawer is open so an idle tab doesn't hold a subscription.
function useMoaConfig(isOpen: boolean) {
  return useQuery({
    queryKey: ["moa", "config"] as const,
    queryFn: getMoaConfig,
    enabled: isOpen,
    staleTime: 30_000,
  });
}

// ── Types ────────────────────────────────────────────────────────

interface MoaDrawerProps {
  isOpen: boolean;
  onClose: () => void;
}

type FanoutMode = "user_turn" | "always" | "every_n";

interface SlotForm {
  // Stable client-only id for React keys; survives reorders so mid-list
  // deletion doesn't re-fire `useModels` in `SlotEditor` (index keys did).
  id: string;
  enabled: boolean;
  provider: string;
  model: string;
  api_key_env: string;
  base_url: string;
}

interface PresetForm {
  name: string;
  enabled: boolean;
  reference_models: SlotForm[];
  aggregator: SlotForm;
  reference_temperature: string;
  aggregator_temperature: string;
  reference_timeout_secs: string;
  reference_max_tokens: string;
  degraded_reference_policy: "loud" | "silent";
  fanout_mode: FanoutMode;
  fanout_every_n: string;
}

const emptySlot = (): SlotForm => ({ id: crypto.randomUUID(), enabled: true, provider: "", model: "", api_key_env: "", base_url: "" });


const emptyPresetForm = (): PresetForm => ({
  name: "",
  enabled: true,
  reference_models: [emptySlot(), emptySlot()],
  aggregator: emptySlot(),
  reference_temperature: "",
  aggregator_temperature: "",
  reference_timeout_secs: "",
  reference_max_tokens: "",
  degraded_reference_policy: "loud",
  fanout_mode: "user_turn",
  fanout_every_n: "3",
});

function slotToForm(slot: MoaSlot): SlotForm {
  return {
    id: crypto.randomUUID(),
    enabled: slot.enabled,
    provider: slot.provider,
    model: slot.model,
    api_key_env: slot.api_key_env ?? "",
    base_url: slot.base_url ?? "",
  };
}

function formToSlot(f: SlotForm): MoaSlot {
  return {
    enabled: f.enabled,
    provider: f.provider,
    model: f.model,
    api_key_env: f.api_key_env || null,
    base_url: f.base_url || null,
  };
}

function presetToForm(name: string, preset: MoaPreset): PresetForm {
  let fanoutMode: FanoutMode = "every_n";
  if (preset.fanout === "user_turn") fanoutMode = "user_turn";
  else if (preset.fanout === "always") fanoutMode = "always";
  return {
    name,
    enabled: preset.enabled,
    reference_models: preset.reference_models.map(slotToForm),
    aggregator: slotToForm(preset.aggregator),
    reference_temperature: preset.reference_temperature != null ? String(preset.reference_temperature) : "",
    aggregator_temperature: preset.aggregator_temperature != null ? String(preset.aggregator_temperature) : "",
    reference_timeout_secs: preset.reference_timeout_secs != null ? String(preset.reference_timeout_secs) : "",
    reference_max_tokens: preset.reference_max_tokens != null ? String(preset.reference_max_tokens) : "",
    degraded_reference_policy: preset.degraded_reference_policy,
    fanout_mode: fanoutMode,
    fanout_every_n: typeof preset.fanout === "object" ? String(preset.fanout.every_n.n) : "3",
  };
}

function formToPreset(f: PresetForm): MoaPreset {
  const fanout: MoaPreset["fanout"] =
    f.fanout_mode === "every_n" ? { every_n: { n: Math.max(1, parseInt(f.fanout_every_n) || 3) } } : f.fanout_mode;
  return {
    enabled: f.enabled,
    reference_models: f.reference_models.filter((s) => s.provider && s.model).map(formToSlot),
    aggregator: formToSlot(f.aggregator),
    reference_temperature: f.reference_temperature ? parseFloat(f.reference_temperature) : null,
    aggregator_temperature: f.aggregator_temperature ? parseFloat(f.aggregator_temperature) : null,
    reference_timeout_secs: f.reference_timeout_secs ? parseInt(f.reference_timeout_secs) : null,
    reference_max_tokens: f.reference_max_tokens ? parseInt(f.reference_max_tokens) : null,
    degraded_reference_policy: f.degraded_reference_policy,
    fanout,
  };
}

// ── Slot editor sub-component ────────────────────────────────────

function SlotEditor({
  label,
  slot,
  providers,
  onChange,
  onRemove,
}: {
  label: string;
  slot: SlotForm;
  providers: string[];
  onChange: (s: SlotForm) => void;
  onRemove?: () => void;
}) {
  const { t } = useTranslation();
  const hiddenModelKeys = useUIStore((s) => s.hiddenModelKeys);
  const hiddenSet = useMemo(() => new Set(hiddenModelKeys), [hiddenModelKeys]);
  const modelsQuery = useModels(
    { provider: slot.provider },
    { enabled: !!slot.provider },
  );

  const visibleModels = useMemo(
    () => filterVisible(modelsQuery.data?.models ?? [], hiddenSet),
    [modelsQuery.data?.models, hiddenSet],
  );

  const inputClass = "w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";
  return (
    <div className="rounded-xl border border-border-subtle p-3 space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-bold uppercase tracking-widest text-text-dim">{label}</span>
        <div className="flex items-center gap-2">
          <label className="flex items-center gap-1.5 text-xs text-text-dim cursor-pointer">
            <input
              type="checkbox"
              checked={slot.enabled}
              onChange={(e) => onChange({ ...slot, enabled: e.target.checked })}
              className="rounded border-border-subtle"
            />
            {t("moa.enabled")}
          </label>
          {onRemove && (
            <button type="button" onClick={onRemove} className="text-text-dim hover:text-error transition-colors">
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          )}
        </div>
      </div>
      <div className="grid grid-cols-2 gap-2">
        <div>
          <label className="text-[10px] font-bold text-text-dim uppercase">{t("moa.provider")}</label>
          <select
            value={slot.provider}
            onChange={(e) => onChange({ ...slot, provider: e.target.value, model: "" })}
            className={inputClass}
          >
            <option value="">{t("moa.select_provider")}</option>
            {providers.map((p) => (
              <option key={p} value={p}>{p}</option>
            ))}
          </select>
        </div>
        <div>
          <label className="text-[10px] font-bold text-text-dim uppercase">{t("moa.model")}</label>
          <select
            value={slot.model}
            onChange={(e) => onChange({ ...slot, model: e.target.value })}
            disabled={!slot.provider}
            className={inputClass}
          >
            <option value="">{slot.provider ? (modelsQuery.isLoading ? t("common.loading") : t("moa.select_provider")) : t("moa.pick_provider_first")}</option>
            {visibleModels.map((m) => (
              <option key={modelKey(m)} value={m.id}>{m.display_name || m.id}</option>
            ))}
          </select>
        </div>
        <div>
          <label className="text-[10px] font-bold text-text-dim uppercase">{t("moa.api_key_env")}</label>
          <input
            value={slot.api_key_env}
            onChange={(e) => onChange({ ...slot, api_key_env: e.target.value })}
            placeholder="OPENAI_API_KEY"
            className={inputClass}
          />
        </div>
        <div>
          <label className="text-[10px] font-bold text-text-dim uppercase">{t("moa.base_url")}</label>
          <input
            value={slot.base_url}
            onChange={(e) => onChange({ ...slot, base_url: e.target.value })}
            placeholder="https://api.openai.com/v1"
            className={inputClass}
          />
        </div>
      </div>
    </div>
  );
}

// ── Main component ───────────────────────────────────────────────

export function MoaDrawer({ isOpen, onClose }: MoaDrawerProps) {
  const addToast = useUIStore((s) => s.addToast);
  const { t } = useTranslation();
  const presetsQuery = useMoaPresets();
  const providersQuery = useProviders();
  const putPreset = usePutMoaPreset();
  const deletePreset = useDeleteMoaPreset();
  const putConfig = usePutMoaConfig();

  // View state: "list" | "editor"
  const [view, setView] = useState<"list" | "editor">("list");
  const [form, setForm] = useState<PresetForm>(emptyPresetForm());
  const [editingName, setEditingName] = useState<string | null>(null);
  // Global config (privacy_filter + save_traces) — hydrated from the server
  // config endpoint. The presets list endpoint omits these fields, so the
  // selects/checkbox would otherwise be dead. Local state is seeded from the
  // query and edited optimistically; `handleConfigChange` PUTs via the safe
  // getMoaConfig-backed `usePutMoaConfig`.
  const configQuery = useMoaConfig(isOpen);
  const [privacyFilter, setPrivacyFilter] = useState<MoaPrivacyFilter>("off");
  const [saveTraces, setSaveTraces] = useState(false);

  useEffect(() => {
    if (configQuery.data) {
      setPrivacyFilter(configQuery.data.privacy_filter);
      setSaveTraces(configQuery.data.save_traces);
    }
  }, [configQuery.data]);

  // Reset to list view when drawer closes
  useEffect(() => {
    if (!isOpen) setView("list");
  }, [isOpen]);

  const providerIds = useMemo(
    () => (providersQuery.data ?? []).filter((p) => isProviderAvailable(p.auth_status)).map((p) => p.id),
    [providersQuery.data],
  );
  const presets = useMemo(() => presetsQuery.data?.presets ?? [], [presetsQuery.data]);
  const defaultPreset = presetsQuery.data?.default_preset ?? "default";

  const openEditor = useCallback((name?: string) => {
    if (name) {
      const entry = presets.find((p) => p.name === name);
      if (entry) {
        setForm(presetToForm(entry.name, entry.preset));
        setEditingName(name);
      }
    } else {
      setForm(emptyPresetForm());
      setEditingName(null);
    }
    setView("editor");
  }, [presets]);

  const handleSave = useCallback(() => {
    const name = form.name.trim();
    if (!name) {
      addToast("Preset name is required", "error");
      return;
    }
    if (!form.aggregator.provider || !form.aggregator.model) {
      addToast("Aggregator provider and model are required", "error");
      return;
    }
    const completeRefs = form.reference_models.filter((s) => s.provider && s.model);
    if (form.enabled && completeRefs.length === 0) {
      addToast("At least one reference model with provider and model is required", "error");
      return;
    }
    const preset = formToPreset(form);
    putPreset.mutate(
      { name, preset },
      {
        onSuccess: () => {
          addToast(`Preset "${name}" saved`, "success");
          setView("list");
        },
        onError: (err) => addToast(`Save failed: ${err.message}`, "error"),
      },
    );
  }, [form, putPreset, addToast]);

  // Two-step inline delete confirmation — mirrors AgentSchedulePanel's
  // `pendingDelete` pattern: first click reveals confirm/cancel affordances,
  // confirm fires the mutation.
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);

  const confirmDelete = useCallback((name: string) => {
    deletePreset.mutate(name, {
      onSuccess: () => {
        addToast(`Preset "${name}" deleted`, "success");
        setPendingDelete(null);
      },
      onError: (err) => addToast(`Delete failed: ${err.message}`, "error"),
    });
  }, [deletePreset, addToast]);

  const handleConfigChange = useCallback((patch: { default_preset?: string; privacy_filter?: MoaPrivacyFilter; save_traces?: boolean }) => {
    putConfig.mutate(patch, {
      onError: (err) => addToast(`Config update failed: ${err.message}`, "error"),
    });
  }, [putConfig, addToast]);

  const inputClass = "w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";

  return (
    <DrawerPanel isOpen={isOpen} onClose={onClose} title={t("moa.config_title")} size="xl">
      <div className="p-5 space-y-5">
        {presetsQuery.isLoading && (
          <div className="flex items-center justify-center py-12 text-text-dim">
            <Loader2 className="w-5 h-5 animate-spin" />
          </div>
        )}

        {presetsQuery.isError && (
          <div className="rounded-xl bg-error/5 border border-error/20 p-4 text-sm text-error">
            {t("moa.load_error")}
          </div>
        )}

        {!presetsQuery.isLoading && !presetsQuery.isError && view === "list" && (
          <>
            {/* Global settings */}
            <div className="rounded-xl border border-border-subtle p-4 space-y-3">
              <h3 className="text-xs font-bold uppercase tracking-widest text-text-dim">{t("moa.global_settings")}</h3>
              <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
                <Select
                  label={t("moa.default_preset")}
                  value={defaultPreset}
                  onChange={(e) => handleConfigChange({ default_preset: e.target.value })}
                  options={presets.map((p) => ({ value: p.name, label: p.name }))}
                />
                <Select
                  label={t("moa.privacy_filter")}
                  value={privacyFilter}
                  onChange={(e) => {
                    const next = e.target.value as MoaPrivacyFilter;
                    setPrivacyFilter(next);
                    handleConfigChange({ privacy_filter: next });
                  }}
                  options={[
                    { value: "off", label: t("moa.privacy_off") },
                    { value: "display", label: t("moa.privacy_display") },
                    { value: "full", label: t("moa.privacy_full") },
                  ]}
                />
                <div className="flex flex-col gap-1.5">
                  <span className="text-[10px] font-black uppercase tracking-widest text-text-dim">{t("moa.save_traces")}</span>
                  <label className="flex items-center gap-2 text-sm cursor-pointer">
                    <input
                      type="checkbox"
                      checked={saveTraces}
                      onChange={(e) => {
                        setSaveTraces(e.target.checked);
                        handleConfigChange({ save_traces: e.target.checked });
                      }}
                      className="rounded border-border-subtle"
                    />
                    <span className="text-text-secondary">{t("moa.save_traces_hint")}</span>
                  </label>
                </div>
              </div>
            </div>

            {/* Preset list */}
            <div className="flex items-center justify-between">
              <h3 className="text-xs font-bold uppercase tracking-widest text-text-dim">{t("moa.presets")}</h3>
              <Button variant="primary" onClick={() => openEditor()}>
                <Plus className="w-4 h-4" />
                <span>{t("moa.add_preset")}</span>
              </Button>
            </div>

            <div className="space-y-2">
              {presets.map((entry) => (
                <div
                  key={entry.name}
                  className="flex items-center justify-between rounded-xl border border-border-subtle px-4 py-3"
                >
                  <div className="flex items-center gap-3">
                    <span className="text-sm font-medium">{entry.name}</span>
                    {entry.is_default && <Badge variant="brand">default</Badge>}
                    {!entry.enabled && <Badge variant="default">disabled</Badge>}
                    <span className="text-xs text-text-dim">
                      {t("moa.advisors", { count: entry.preset.reference_models.length })} {entry.preset.aggregator.model || t("moa.no_aggregator")}
                    </span>
                  </div>
                  <div className="flex items-center gap-1">
                    <button
                      type="button"
                      onClick={() => openEditor(entry.name)}
                      className="p-1.5 rounded-lg text-text-dim hover:text-brand hover:bg-brand/5 transition-colors"
                      title={t("common.edit")}
                    >
                      <Pencil className="w-4 h-4" />
                    </button>
                    {pendingDelete === entry.name ? (
                      <div className="flex items-center gap-1">
                        <Button
                          variant="danger"
                          size="sm"
                          onClick={() => confirmDelete(entry.name)}
                          disabled={deletePreset.isPending}
                        >
                          {t("common.confirm", { defaultValue: "Confirm" })}
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setPendingDelete(null)}
                        >
                          {t("common.cancel", { defaultValue: "Cancel" })}
                        </Button>
                      </div>
                    ) : (
                      <button
                        type="button"
                        onClick={() => setPendingDelete(entry.name)}
                        className="p-1.5 rounded-lg text-text-dim hover:text-error hover:bg-error/5 transition-colors"
                        title={t("common.delete")}
                      >
                        <Trash2 className="w-4 h-4" />
                      </button>
                    )}
                  </div>
                </div>
              ))}
              {presets.length === 0 && (
                <p className="text-sm text-text-dim py-4 text-center">{t("moa.no_presets")}</p>
              )}
            </div>
          </>
        )}

        {!presetsQuery.isLoading && view === "editor" && (
          <>
            {/* Editor header */}
            <div className="flex items-center gap-3">
              <button
                type="button"
                onClick={() => setView("list")}
                className="p-1.5 rounded-lg text-text-dim hover:text-brand hover:bg-brand/5 transition-colors"
              >
                <ArrowLeft className="w-4 h-4" />
              </button>
              <h3 className="text-sm font-bold">{editingName ? t("moa.edit_preset", { name: editingName }) : t("moa.new_preset")}</h3>
            </div>

            <div className="space-y-4">
              {/* Name + enabled */}
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="text-[10px] font-bold text-text-dim uppercase">{t("moa.preset_name")}</label>
                  <input
                    value={form.name}
                    onChange={(e) => setForm({ ...form, name: e.target.value })}
                    placeholder="my-preset"
                    disabled={!!editingName}
                    className={inputClass}
                  />
                </div>
                <div className="flex items-end pb-2">
                  <label className="flex items-center gap-2 text-sm cursor-pointer">
                    <input
                      type="checkbox"
                      checked={form.enabled}
                      onChange={(e) => setForm({ ...form, enabled: e.target.checked })}
                      className="rounded border-border-subtle"
                    />
                    <span className="text-text-secondary">{t("moa.enabled")}</span>
                  </label>
                </div>
              </div>

              {/* Aggregator */}
              <SlotEditor
                label="Aggregator"
                slot={form.aggregator}
                providers={providerIds}
                onChange={(s) => setForm({ ...form, aggregator: s })}
              />

              {/* Reference models */}
              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <span className="text-[10px] font-bold uppercase tracking-widest text-text-dim">{t("moa.reference_models")}</span>
                  <button
                    type="button"
                    onClick={() => setForm({ ...form, reference_models: [...form.reference_models, emptySlot()] })}
                    className="text-xs text-brand hover:underline"
                  >
                    {t("moa.add_slot")}
                  </button>
                </div>
                {form.reference_models.map((slot, i) => (
                  <SlotEditor
                    key={slot.id}
                    label={t("moa.advisor_n", { n: i + 1 })}
                    slot={slot}
                    providers={providerIds}
                    onChange={(s) => {
                      const next = [...form.reference_models];
                      next[i] = s;
                      setForm({ ...form, reference_models: next });
                    }}
                    onRemove={
                      form.reference_models.length > 1
                        ? () => setForm({ ...form, reference_models: form.reference_models.filter((_, j) => j !== i) })
                        : undefined
                    }
                  />
                ))}
              </div>

              {/* Parameters */}
              <div className="rounded-xl border border-border-subtle p-3 space-y-3">
                <span className="text-[10px] font-bold uppercase tracking-widest text-text-dim">{t("moa.parameters")}</span>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="text-[10px] font-bold text-text-dim uppercase">{t("moa.reference_temp")}</label>
                    <input
                      type="number"
                      step="0.1"
                      min="0"
                      max="2"
                      value={form.reference_temperature}
                      onChange={(e) => setForm({ ...form, reference_temperature: e.target.value })}
                      placeholder="0.7"
                      className={inputClass}
                    />
                  </div>
                  <div>
                    <label className="text-[10px] font-bold text-text-dim uppercase">{t("moa.aggregator_temp")}</label>
                    <input
                      type="number"
                      step="0.1"
                      min="0"
                      max="2"
                      value={form.aggregator_temperature}
                      onChange={(e) => setForm({ ...form, aggregator_temperature: e.target.value })}
                      placeholder="0.3"
                      className={inputClass}
                    />
                  </div>
                  <div>
                    <label className="text-[10px] font-bold text-text-dim uppercase">{t("moa.timeout_secs")}</label>
                    <input
                      type="number"
                      min="1"
                      value={form.reference_timeout_secs}
                      onChange={(e) => setForm({ ...form, reference_timeout_secs: e.target.value })}
                      placeholder="120"
                      className={inputClass}
                    />
                  </div>
                  <div>
                    <label className="text-[10px] font-bold text-text-dim uppercase">{t("moa.max_tokens")}</label>
                    <input
                      type="number"
                      min="1"
                      value={form.reference_max_tokens}
                      onChange={(e) => setForm({ ...form, reference_max_tokens: e.target.value })}
                      placeholder="4096"
                      className={inputClass}
                    />
                  </div>
                  <div>
                    <label className="text-[10px] font-bold text-text-dim uppercase">{t("moa.degraded_policy")}</label>
                    <select
                      value={form.degraded_reference_policy}
                      onChange={(e) => setForm({ ...form, degraded_reference_policy: e.target.value as "loud" | "silent" })}
                      className={inputClass}
                    >
                      <option value="loud">{t("moa.degraded_fail")}</option>
                      <option value="silent">{t("moa.degraded_silent")}</option>
                    </select>
                  </div>
                  <div>
                    <label className="text-[10px] font-bold text-text-dim uppercase">{t("moa.fanout")}</label>
                    <div className="flex gap-2">
                      <select
                        value={form.fanout_mode}
                        onChange={(e) => setForm({ ...form, fanout_mode: e.target.value as FanoutMode })}
                        className={inputClass}
                      >
                        <option value="user_turn">{t("moa.fanout_user_turn")}</option>
                        <option value="always">{t("moa.fanout_always")}</option>
                        <option value="every_n">{t("moa.fanout_every_n")}</option>
                      </select>
                      {form.fanout_mode === "every_n" && (
                        <input
                          type="number"
                          min="1"
                          value={form.fanout_every_n}
                          onChange={(e) => setForm({ ...form, fanout_every_n: e.target.value })}
                          className={`${inputClass} w-20`}
                        />
                      )}
                    </div>
                  </div>
                </div>
              </div>

              {/* Actions */}
              <div className="flex items-center justify-end gap-2 pt-2">
                <Button variant="ghost" onClick={() => setView("list")}>{t("common.cancel")}</Button>
                <Button variant="primary" onClick={handleSave} disabled={putPreset.isPending}>
                  {putPreset.isPending && <Loader2 className="w-4 h-4 animate-spin" />}
                  <span>{editingName ? t("moa.save_changes") : t("moa.create_preset")}</span>
                </Button>
              </div>
            </div>
          </>
        )}
      </div>
    </DrawerPanel>
  );
}
