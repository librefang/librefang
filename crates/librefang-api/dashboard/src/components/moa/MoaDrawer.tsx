import { useCallback, useEffect, useMemo, useState } from "react";
import { Plus, Trash2, Pencil, ArrowLeft, Loader2 } from "lucide-react";
import type { MoaPreset, MoaSlot } from "../../api";
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

// ── Types ────────────────────────────────────────────────────────

interface MoaDrawerProps {
  isOpen: boolean;
  onClose: () => void;
}

type FanoutMode = "user_turn" | "always" | "every_n";

interface SlotForm {
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

// ── Helpers ──────────────────────────────────────────────────────

const emptySlot = (): SlotForm => ({ enabled: true, provider: "", model: "", api_key_env: "", base_url: "" });

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
  const fanoutMode: FanoutMode =
    preset.fanout === "user_turn" ? "user_turn" : preset.fanout === "always" ? "always" : "every_n";
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
            Enabled
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
          <label className="text-[10px] font-bold text-text-dim uppercase">Provider</label>
          <select
            value={slot.provider}
            onChange={(e) => onChange({ ...slot, provider: e.target.value, model: "" })}
            className={inputClass}
          >
            <option value="">Select…</option>
            {providers.map((p) => (
              <option key={p} value={p}>{p}</option>
            ))}
          </select>
        </div>
        <div>
          <label className="text-[10px] font-bold text-text-dim uppercase">Model</label>
          <select
            value={slot.model}
            onChange={(e) => onChange({ ...slot, model: e.target.value })}
            disabled={!slot.provider}
            className={inputClass}
          >
            <option value="">{slot.provider ? (modelsQuery.isLoading ? "Loading…" : "Select…") : "Pick provider first"}</option>
            {visibleModels.map((m) => (
              <option key={modelKey(m)} value={m.id}>{m.display_name || m.id}</option>
            ))}
          </select>
        </div>
        <div>
          <label className="text-[10px] font-bold text-text-dim uppercase">API Key Env</label>
          <input
            value={slot.api_key_env}
            onChange={(e) => onChange({ ...slot, api_key_env: e.target.value })}
            placeholder="OPENAI_API_KEY"
            className={inputClass}
          />
        </div>
        <div>
          <label className="text-[10px] font-bold text-text-dim uppercase">Base URL</label>
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
  const presetsQuery = useMoaPresets();
  const providersQuery = useProviders();
  const putPreset = usePutMoaPreset();
  const deletePreset = useDeleteMoaPreset();
  const putConfig = usePutMoaConfig();

  // View state: "list" | "editor"
  const [view, setView] = useState<"list" | "editor">("list");
  const [form, setForm] = useState<PresetForm>(emptyPresetForm());
  const [editingName, setEditingName] = useState<string | null>(null);

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

  const handleDelete = useCallback((name: string) => {
    deletePreset.mutate(name, {
      onSuccess: () => addToast(`Preset "${name}" deleted`, "success"),
      onError: (err) => addToast(`Delete failed: ${err.message}`, "error"),
    });
  }, [deletePreset, addToast]);

  const handleConfigChange = useCallback((patch: { default_preset?: string; privacy_filter?: string; save_traces?: boolean }) => {
    putConfig.mutate(patch, {
      onError: (err) => addToast(`Config update failed: ${err.message}`, "error"),
    });
  }, [putConfig, addToast]);

  const inputClass = "w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";

  return (
    <DrawerPanel isOpen={isOpen} onClose={onClose} title="MoA Configuration" size="xl">
      <div className="p-5 space-y-5">
        {presetsQuery.isLoading && (
          <div className="flex items-center justify-center py-12 text-text-dim">
            <Loader2 className="w-5 h-5 animate-spin" />
          </div>
        )}

        {presetsQuery.isError && (
          <div className="rounded-xl bg-error/5 border border-error/20 p-4 text-sm text-error">
            Failed to load MoA presets.
          </div>
        )}

        {!presetsQuery.isLoading && !presetsQuery.isError && view === "list" && (
          <>
            {/* Global settings */}
            <div className="rounded-xl border border-border-subtle p-4 space-y-3">
              <h3 className="text-xs font-bold uppercase tracking-widest text-text-dim">Global Settings</h3>
              <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
                <Select
                  label="Default Preset"
                  value={defaultPreset}
                  onChange={(e) => handleConfigChange({ default_preset: e.target.value })}
                  options={presets.map((p) => ({ value: p.name, label: p.name }))}
                />
                <Select
                  label="Privacy Filter"
                  value={presetsQuery.data ? "off" : "off"}
                  onChange={(e) => handleConfigChange({ privacy_filter: e.target.value })}
                  options={[
                    { value: "off", label: "Off" },
                    { value: "display", label: "Display" },
                    { value: "full", label: "Full" },
                  ]}
                />
                <div className="flex flex-col gap-1.5">
                  <span className="text-[10px] font-black uppercase tracking-widest text-text-dim">Save Traces</span>
                  <label className="flex items-center gap-2 text-sm cursor-pointer">
                    <input
                      type="checkbox"
                      onChange={(e) => handleConfigChange({ save_traces: e.target.checked })}
                      className="rounded border-border-subtle"
                    />
                    <span className="text-text-secondary">Persist advisor traces to disk</span>
                  </label>
                </div>
              </div>
            </div>

            {/* Preset list */}
            <div className="flex items-center justify-between">
              <h3 className="text-xs font-bold uppercase tracking-widest text-text-dim">Presets</h3>
              <Button variant="primary" onClick={() => openEditor()}>
                <Plus className="w-4 h-4" />
                <span>Add preset</span>
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
                      {entry.preset.reference_models.length} advisors · {entry.preset.aggregator.model || "no aggregator"}
                    </span>
                  </div>
                  <div className="flex items-center gap-1">
                    <button
                      type="button"
                      onClick={() => openEditor(entry.name)}
                      className="p-1.5 rounded-lg text-text-dim hover:text-brand hover:bg-brand/5 transition-colors"
                      title="Edit"
                    >
                      <Pencil className="w-4 h-4" />
                    </button>
                    <button
                      type="button"
                      onClick={() => handleDelete(entry.name)}
                      disabled={deletePreset.isPending}
                      className="p-1.5 rounded-lg text-text-dim hover:text-error hover:bg-error/5 transition-colors disabled:opacity-50"
                      title="Delete"
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>
                </div>
              ))}
              {presets.length === 0 && (
                <p className="text-sm text-text-dim py-4 text-center">No presets configured.</p>
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
              <h3 className="text-sm font-bold">{editingName ? `Edit: ${editingName}` : "New Preset"}</h3>
            </div>

            <div className="space-y-4">
              {/* Name + enabled */}
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="text-[10px] font-bold text-text-dim uppercase">Preset Name</label>
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
                    <span className="text-text-secondary">Enabled</span>
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
                  <span className="text-[10px] font-bold uppercase tracking-widest text-text-dim">Reference Models (Advisors)</span>
                  <button
                    type="button"
                    onClick={() => setForm({ ...form, reference_models: [...form.reference_models, emptySlot()] })}
                    className="text-xs text-brand hover:underline"
                  >
                    + Add slot
                  </button>
                </div>
                {form.reference_models.map((slot, i) => (
                  <SlotEditor
                    key={i}
                    label={`Advisor ${i + 1}`}
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
                <span className="text-[10px] font-bold uppercase tracking-widest text-text-dim">Parameters</span>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="text-[10px] font-bold text-text-dim uppercase">Reference Temp</label>
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
                    <label className="text-[10px] font-bold text-text-dim uppercase">Aggregator Temp</label>
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
                    <label className="text-[10px] font-bold text-text-dim uppercase">Timeout (secs)</label>
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
                    <label className="text-[10px] font-bold text-text-dim uppercase">Max Tokens</label>
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
                    <label className="text-[10px] font-bold text-text-dim uppercase">Degraded Policy</label>
                    <select
                      value={form.degraded_reference_policy}
                      onChange={(e) => setForm({ ...form, degraded_reference_policy: e.target.value as "loud" | "silent" })}
                      className={inputClass}
                    >
                      <option value="loud">Loud (surface errors)</option>
                      <option value="silent">Silent (skip failed)</option>
                    </select>
                  </div>
                  <div>
                    <label className="text-[10px] font-bold text-text-dim uppercase">Fanout</label>
                    <div className="flex gap-2">
                      <select
                        value={form.fanout_mode}
                        onChange={(e) => setForm({ ...form, fanout_mode: e.target.value as FanoutMode })}
                        className={inputClass}
                      >
                        <option value="user_turn">User turn</option>
                        <option value="always">Always</option>
                        <option value="every_n">Every N</option>
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
                <Button variant="ghost" onClick={() => setView("list")}>Cancel</Button>
                <Button variant="primary" onClick={handleSave} disabled={putPreset.isPending}>
                  {putPreset.isPending && <Loader2 className="w-4 h-4 animate-spin" />}
                  <span>{editingName ? "Save changes" : "Create preset"}</span>
                </Button>
              </div>
            </div>
          </>
        )}
      </div>
    </DrawerPanel>
  );
}
