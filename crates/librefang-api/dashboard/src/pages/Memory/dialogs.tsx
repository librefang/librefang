import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Plus } from "lucide-react";
import { Button } from "../../components/ui/Button";
import { DrawerPanel } from "../../components/ui/DrawerPanel";
import { useUIStore } from "../../lib/store";
import {
  useAddMemory,
  useUpdateMemory,
  useUpdateMemoryConfig,
} from "../../lib/mutations/memory";
import { useMemoryConfig } from "../../lib/queries/memory";
import { useModels } from "../../lib/queries/models";
import {
  KNOWN_EMBEDDING_MODELS,
  EMBEDDING_PROVIDER_LABELS,
  CUSTOM_OPTION,
} from "./constants";

const inputCls =
  "w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand focus:ring-1 focus:ring-brand/20";
const labelCls = "text-[11px] font-bold uppercase tracking-widest text-text-dim mb-1 block";

/* ----------------------------------------------------------------------- *
 * Add Memory
 * ----------------------------------------------------------------------- */

export function AddMemoryDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const [content, setContent] = useState("");
  const [agentId, setAgentId] = useState("");
  const [level, setLevel] = useState("session");

  const addMutation = useAddMemory();

  const handleAdd = () => {
    addMutation.mutate(
      { content, level, agentId: agentId || undefined },
      {
        onSuccess: () => onClose(),
        onError: (err) => addToast(err instanceof Error ? err.message : t("common.error"), "error"),
      },
    );
  };

  return (
    <DrawerPanel isOpen onClose={onClose} title={t("memory.add_memory")} size="md">
      <div className="p-4 sm:p-6">
        <div className="space-y-4">
          <div>
            <label className={labelCls}>{t("memory.content")}</label>
            <textarea
              value={content}
              onChange={(e) => setContent(e.target.value)}
              placeholder={t("memory.content_placeholder")}
              rows={4}
              className={`${inputCls} resize-none`}
            />
          </div>

          <div>
            <label className={labelCls}>{t("memory.level", { defaultValue: "Level" })}</label>
            <select value={level} onChange={(e) => setLevel(e.target.value)} className={inputCls}>
              <option value="user">{t("memory.user", { defaultValue: "user" })}</option>
              <option value="session">{t("memory.session", { defaultValue: "session" })}</option>
              <option value="agent">{t("memory.agent", { defaultValue: "agent" })}</option>
            </select>
          </div>

          <div>
            <label className={labelCls}>{t("memory.agent_id")}</label>
            <input
              type="text"
              value={agentId}
              onChange={(e) => setAgentId(e.target.value)}
              placeholder={t("memory.agent_optional")}
              className={inputCls}
            />
          </div>
        </div>

        <div className="flex gap-3 mt-6">
          <Button variant="secondary" className="flex-1" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            variant="primary"
            className="flex-1"
            onClick={handleAdd}
            disabled={!content.trim() || addMutation.isPending}
          >
            {addMutation.isPending ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              <Plus className="w-4 h-4" />
            )}
            {t("common.save")}
          </Button>
        </div>
      </div>
    </DrawerPanel>
  );
}

/* ----------------------------------------------------------------------- *
 * Edit Memory
 * ----------------------------------------------------------------------- */

export function EditMemoryDialog({
  memory,
  onClose,
}: {
  memory: { id: string; content?: string };
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const [content, setContent] = useState(memory.content || "");

  const editMutation = useUpdateMemory();

  const handleSave = () => {
    editMutation.mutate(
      { id: memory.id, content },
      {
        onSuccess: () => onClose(),
        onError: (err) => addToast(err instanceof Error ? err.message : t("common.error"), "error"),
      },
    );
  };

  return (
    <DrawerPanel isOpen onClose={onClose} title={t("memory.edit_memory")} size="md">
      <div className="p-4 sm:p-6">
        <div>
          <label className={labelCls}>{t("memory.content")}</label>
          <textarea
            value={content}
            onChange={(e) => setContent(e.target.value)}
            rows={6}
            className={`${inputCls} resize-none`}
          />
        </div>

        <div className="flex gap-3 mt-6">
          <Button variant="secondary" className="flex-1" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            variant="primary"
            className="flex-1"
            onClick={handleSave}
            disabled={!content.trim() || editMutation.isPending}
          >
            {editMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : t("common.save")}
          </Button>
        </div>
      </div>
    </DrawerPanel>
  );
}

/* ----------------------------------------------------------------------- *
 * Memory Configuration
 * ----------------------------------------------------------------------- */

interface MemoryConfigForm {
  embedding_provider: string;
  embedding_model: string;
  embedding_api_key_env: string;
  decay_rate: string;
  pm_enabled: boolean;
  pm_auto_memorize: boolean;
  pm_auto_retrieve: boolean;
  pm_extraction_model: string;
  pm_max_retrieve: string;
}

export function MemoryConfigDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);

  const configQuery = useMemoryConfig();
  const updateConfig = useUpdateMemoryConfig();
  // Chat-model catalog for the Extraction Model suggestions. Unfiltered so
  // configured-but-not-yet-probed providers still appear.
  const modelsQuery = useModels();
  const chatModels = modelsQuery.data?.models ?? [];

  const [form, setForm] = useState<MemoryConfigForm | null>(null);

  // Suggestion list for the Embedding Model dropdown. When the provider is
  // pinned and known, surface only that provider's catalog. When the provider
  // is "Auto-detect" (empty) or a not-yet-known string, fall back to the union
  // of every provider's catalog — otherwise a stored value like
  // `text-embedding-3-small` would be flagged Custom whenever the user hasn't
  // explicitly pinned `openai`, which is wrong and surprising.
  const embeddingProvider = form?.embedding_provider ?? "";
  const embeddingProviderKnown = embeddingProvider in KNOWN_EMBEDDING_MODELS;
  const embeddingProviderSuggestions = embeddingProviderKnown
    ? KNOWN_EMBEDDING_MODELS[embeddingProvider]
    : Array.from(new Set(Object.values(KNOWN_EMBEDDING_MODELS).flat()));
  const embeddingKnownSet = new Set(embeddingProviderSuggestions);
  const embeddingIsCustom =
    !!form?.embedding_model && !embeddingKnownSet.has(form.embedding_model);
  const chatModelIdSet = new Set(chatModels.map((m) => m.id));
  // Guard on isSuccess so the stored value doesn't flicker through Custom
  // during the initial useModels() fetch (chatModels is [] while loading).
  const extractionIsCustom =
    modelsQuery.isSuccess &&
    !!form?.pm_extraction_model &&
    !chatModelIdSet.has(form.pm_extraction_model);

  useEffect(() => {
    if (!configQuery.data || form) return;
    setForm({
      embedding_provider: configQuery.data.embedding_provider || "",
      embedding_model: configQuery.data.embedding_model || "",
      embedding_api_key_env: configQuery.data.embedding_api_key_env || "",
      decay_rate: String(configQuery.data.decay_rate ?? 0.05),
      pm_enabled: configQuery.data.proactive_memory?.enabled ?? true,
      pm_auto_memorize: configQuery.data.proactive_memory?.auto_memorize ?? true,
      pm_auto_retrieve: configQuery.data.proactive_memory?.auto_retrieve ?? true,
      pm_extraction_model: configQuery.data.proactive_memory?.extraction_model || "",
      pm_max_retrieve: String(configQuery.data.proactive_memory?.max_retrieve ?? 10),
    });
  }, [configQuery.data, form]);

  const handleSave = async () => {
    if (!form) return;
    try {
      const decayRate = Number(form.decay_rate);
      const maxRetrieve = Number.parseInt(form.pm_max_retrieve, 10);

      await updateConfig.mutateAsync({
        embedding_provider: form.embedding_provider || undefined,
        embedding_model: form.embedding_model || undefined,
        embedding_api_key_env: form.embedding_api_key_env || undefined,
        decay_rate: Number.isFinite(decayRate) ? decayRate : 0.05,
        proactive_memory: {
          enabled: form.pm_enabled,
          auto_memorize: form.pm_auto_memorize,
          auto_retrieve: form.pm_auto_retrieve,
          extraction_model: form.pm_extraction_model || undefined,
          max_retrieve: Number.isFinite(maxRetrieve) ? maxRetrieve : 10,
        },
      });
      addToast(t("common.success"), "success");
      onClose();
    } catch (error) {
      addToast(error instanceof Error ? error.message : "Failed to save", "error");
    }
  };

  return (
    <DrawerPanel
      isOpen
      onClose={onClose}
      title={t("memory.config_title", { defaultValue: "Memory Configuration" })}
      size="lg"
    >
      <div className="p-4 sm:p-6">
        <p className="text-xs text-text-dim mb-4">
          {t("memory.config_desc", {
            defaultValue: "Changes are written to config.toml. Restart required for full effect.",
          })}
        </p>

        {configQuery.isLoading || !form ? (
          <div className="p-6 text-center">
            <Loader2 className="w-5 h-5 animate-spin mx-auto" />
          </div>
        ) : (
          <div className="space-y-6">
            {/* Embedding */}
            <section>
              <h4 className="text-xs font-bold mb-3">
                {t("memory.embedding_section", { defaultValue: "Embedding" })}
              </h4>
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                <div>
                  <span className={labelCls}>
                    {t("memory.provider", { defaultValue: "Provider" })}
                  </span>
                  <select
                    value={form.embedding_provider ?? ""}
                    onChange={(e) => setForm({ ...form, embedding_provider: e.target.value })}
                    className={inputCls}
                  >
                    <option value="">
                      {t("memory.auto_detect", { defaultValue: "Auto-detect" })}
                    </option>
                    <option value="openai">
                      {t("memory.provider_openai", { defaultValue: "OpenAI" })}
                    </option>
                    <option value="ollama">
                      {t("memory.provider_ollama", { defaultValue: "Ollama" })}
                    </option>
                    <option value="vllm">
                      {t("memory.provider_vllm", { defaultValue: "vLLM" })}
                    </option>
                    <option value="lmstudio">
                      {t("memory.provider_lmstudio", { defaultValue: "LM Studio" })}
                    </option>
                    <option value="gemini">
                      {t("memory.provider_gemini", { defaultValue: "Gemini" })}
                    </option>
                    <option value="minimax">
                      {t("memory.provider_minimax", { defaultValue: "MiniMax" })}
                    </option>
                  </select>
                </div>
                <div>
                  <span className={labelCls}>
                    {t("memory.model", { defaultValue: "Model" })}
                  </span>
                  <select
                    value={embeddingIsCustom ? CUSTOM_OPTION : (form.embedding_model ?? "")}
                    onChange={(e) => {
                      const v = e.target.value;
                      if (v === CUSTOM_OPTION) {
                        if (!embeddingIsCustom) setForm({ ...form, embedding_model: "" });
                      } else {
                        setForm({ ...form, embedding_model: v });
                      }
                    }}
                    className={inputCls}
                  >
                    <option value="">
                      {t("memory.embedding_model_default", {
                        defaultValue: "Auto / provider default",
                      })}
                    </option>
                    {embeddingProviderKnown ? (
                      embeddingProviderSuggestions.map((name) => (
                        <option key={name} value={name}>
                          {name}
                        </option>
                      ))
                    ) : (
                      Object.entries(KNOWN_EMBEDDING_MODELS).map(([prov, names]) => (
                        <optgroup
                          key={prov}
                          label={EMBEDDING_PROVIDER_LABELS[prov] ?? prov}
                        >
                          {names.map((name) => (
                            <option key={`${prov}:${name}`} value={name}>
                              {name}
                            </option>
                          ))}
                        </optgroup>
                      ))
                    )}
                    <option value={CUSTOM_OPTION}>
                      {t("memory.custom_model_option", { defaultValue: "Custom…" })}
                    </option>
                  </select>
                  {embeddingIsCustom && (
                    <input
                      value={form.embedding_model ?? ""}
                      onChange={(e) => setForm({ ...form, embedding_model: e.target.value })}
                      placeholder="text-embedding-3-small"
                      className={`${inputCls} mt-2`}
                      autoFocus
                    />
                  )}
                </div>
              </div>
              <div className="mt-3">
                <span className={labelCls}>
                  {t("memory.api_key_env", { defaultValue: "API Key Env" })}
                </span>
                <input
                  value={form.embedding_api_key_env ?? ""}
                  onChange={(e) => setForm({ ...form, embedding_api_key_env: e.target.value })}
                  placeholder="OPENAI_API_KEY"
                  className={inputCls}
                />
                <p className="text-xs text-text-dim mt-1">
                  {t("memory.api_key_env_hint", {
                    defaultValue:
                      "Local providers (Ollama / vLLM / LM Studio) typically don't need a key — leave blank.",
                  })}
                </p>
              </div>
            </section>

            {/* Proactive Memory */}
            <section>
              <h4 className="text-xs font-bold mb-3">
                {t("memory.proactive_memory", { defaultValue: "Proactive Memory" })}
              </h4>
              <div className="space-y-2">
                {[
                  { key: "pm_enabled", label: t("memory.proactive_enabled", { defaultValue: "Enabled" }) },
                  { key: "pm_auto_memorize", label: t("memory.auto_memorize", { defaultValue: "Auto Memorize" }) },
                  { key: "pm_auto_retrieve", label: t("memory.auto_retrieve", { defaultValue: "Auto Retrieve" }) },
                ].map((opt) => (
                  <label
                    key={opt.key}
                    className="flex items-center justify-between rounded-lg bg-main/50 px-3 py-2"
                  >
                    <span className="text-xs font-medium">{opt.label}</span>
                    <button
                      role="switch"
                      aria-checked={!!form[opt.key as keyof MemoryConfigForm]}
                      aria-label={opt.label}
                      onClick={() =>
                        setForm({
                          ...form,
                          [opt.key]: !form[opt.key as keyof MemoryConfigForm],
                        })
                      }
                      className={`w-10 h-5 rounded-full transition-colors ${
                        form[opt.key as keyof MemoryConfigForm] ? "bg-brand" : "bg-border-subtle"
                      }`}
                    >
                      <div
                        className={`w-4 h-4 rounded-full bg-white shadow transition-transform ${
                          form[opt.key as keyof MemoryConfigForm]
                            ? "translate-x-5"
                            : "translate-x-0.5"
                        }`}
                      />
                    </button>
                  </label>
                ))}
              </div>
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 mt-3">
                <div>
                  <span className={labelCls}>
                    {t("memory.extraction_model_label", { defaultValue: "Extraction Model" })}
                  </span>
                  <select
                    value={extractionIsCustom ? CUSTOM_OPTION : (form.pm_extraction_model ?? "")}
                    onChange={(e) => {
                      const v = e.target.value;
                      if (v === CUSTOM_OPTION) {
                        if (!extractionIsCustom) setForm({ ...form, pm_extraction_model: "" });
                      } else {
                        setForm({ ...form, pm_extraction_model: v });
                      }
                    }}
                    className={inputCls}
                  >
                    <option value="">
                      {t("memory.extraction_model_default", { defaultValue: "Use kernel default" })}
                    </option>
                    {chatModels.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.display_name && m.display_name !== m.id
                          ? `${m.display_name} (${m.provider})`
                          : `${m.id} (${m.provider})`}
                      </option>
                    ))}
                    {/* While useModels() is still loading the catalog is empty;
                        render the stored value as a transient option so the
                        select's `value=` has something to match. */}
                    {!modelsQuery.isSuccess &&
                      form.pm_extraction_model &&
                      !chatModelIdSet.has(form.pm_extraction_model) && (
                        <option value={form.pm_extraction_model}>
                          {form.pm_extraction_model}
                        </option>
                      )}
                    <option value={CUSTOM_OPTION}>
                      {t("memory.custom_model_option", { defaultValue: "Custom…" })}
                    </option>
                  </select>
                  {extractionIsCustom && (
                    <input
                      value={form.pm_extraction_model ?? ""}
                      onChange={(e) =>
                        setForm({ ...form, pm_extraction_model: e.target.value })
                      }
                      placeholder={t("memory.extraction_model_placeholder", {
                        defaultValue: "e.g. openai/gpt-4o-mini",
                      })}
                      className={`${inputCls} mt-2`}
                      autoFocus
                    />
                  )}
                </div>
                <div>
                  <span className={labelCls}>
                    {t("memory.max_retrieve", { defaultValue: "Max Retrieve" })}
                  </span>
                  <input
                    type="number"
                    min={1}
                    max={50}
                    value={form.pm_max_retrieve ?? 10}
                    onChange={(e) => setForm({ ...form, pm_max_retrieve: e.target.value })}
                    className={inputCls}
                  />
                </div>
              </div>
            </section>

            {/* Decay */}
            <section>
              <span className={labelCls}>
                {t("memory.decay_rate", { defaultValue: "Decay Rate" })}
              </span>
              <input
                type="number"
                step={0.01}
                min={0}
                max={1}
                value={form.decay_rate ?? 0.05}
                onChange={(e) => setForm({ ...form, decay_rate: e.target.value })}
                className={inputCls}
              />
              <p className="text-xs text-text-dim mt-1">
                {t("memory.decay_rate_hint", {
                  defaultValue:
                    "Per-day decay applied to memory confidence. Lower = memories age more slowly.",
                })}
              </p>
            </section>
          </div>
        )}

        <div className="flex gap-2 mt-6">
          <Button
            variant="primary"
            className="flex-1"
            onClick={handleSave}
            disabled={updateConfig.isPending}
          >
            {updateConfig.isPending ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              t("common.save")
            )}
          </Button>
          <Button variant="secondary" className="flex-1" onClick={onClose}>
            {t("common.cancel")}
          </Button>
        </div>
      </div>
    </DrawerPanel>
  );
}
