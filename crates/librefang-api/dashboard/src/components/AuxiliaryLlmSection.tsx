import { useState, useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash2, Save, X } from "lucide-react";
import { useConfigSchema, useAuxiliaryChains } from "../lib/queries/config";
import { useModels } from "../lib/queries/models";
import { useSetConfigValue } from "../lib/mutations/config";
import { useProviders } from "../lib/queries/providers";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { useUIStore } from "../lib/store";
import { toastErr } from "../lib/errors";

/** Stable fallback so the loading state does not churn `useCallback` deps. */
const EMPTY_CHAINS: Record<string, string[]> = {};

export function AuxiliaryLlmSection() {
  const { t } = useTranslation();

  // Task list comes from the kernel: `x-aux-tasks` on the config schema is
  // built from the compile-guarded `AuxTask::ALL` (librefang-types), so a new
  // `AuxTask` variant shows up here without a dashboard change (#8059 review).
  const schema = useConfigSchema();
  const auxTasks: string[] = schema.data?.["x-aux-tasks"] ?? [];

  const auxiliary = useAuxiliaryChains().data ?? EMPTY_CHAINS;

  const setConfig = useSetConfigValue();
  const addToast = useUIStore((s) => s.addToast);

  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState<string[]>([]);

  const modelOptions = useModels().data?.models ?? [];
  const registeredProviders = useProviders().data ?? [];

  // Save-time validation (#8059 review): a typo'd `provider:model` entry used to save cleanly and then silently fall back to the primary model at resolution time.
  // Validated only when at least one provider is known, so a failed catalog query degrades to the historical accept-any behaviour instead of bricking the editor.
  const knownProviders = useMemo(() => {
    const ids = new Set<string>();
    for (const p of registeredProviders) {
      if (p.id) ids.add(p.id);
    }
    for (const m of modelOptions) {
      if (m.provider) ids.add(m.provider);
    }
    return ids;
  }, [registeredProviders, modelOptions]);

  const startEdit = useCallback(
    (task: string) => {
      setEditing(task);
      setDraft([...(auxiliary[task] ?? []), ""]);
    },
    [auxiliary],
  );

  const cancelEdit = useCallback(() => {
    setEditing(null);
    setDraft([]);
  }, []);

  const saveEdit = useCallback(async () => {
    if (!editing) return;
    const chain = draft.map((s) => s.trim()).filter(Boolean);
    if (knownProviders.size > 0) {
      const bad = chain.filter((entry) => !knownProviders.has(entry.split(":", 1)[0] ?? ""));
      if (bad.length > 0) {
        addToast(
          t(
            "config.auxiliary_chain_unknown_provider",
            'Unknown provider in "{{bad}}". Registered: {{known}}.',
            { bad: bad.join(", "), known: [...knownProviders].sort().join(", ") },
          ),
          "error",
        );
        return;
      }
    }
    try {
      await setConfig.mutateAsync({
        path: `llm.auxiliary.${editing}`,
        value: chain,
      });
      addToast(t("common.saved", "Saved"), "success");
      setEditing(null);
    } catch (err) {
      addToast(toastErr(err, t("common.save_failed", "Save failed")), "error");
    }
  }, [editing, draft, knownProviders, setConfig, addToast, t]);

  return (
    <div className="rounded-2xl border border-border-subtle bg-surface overflow-hidden">
      <div className="flex items-center gap-2 px-5 py-2.5 border-b border-border-subtle/50">
        <span className="text-xs font-semibold text-text-dim">
          {t("config.auxiliary_llm_title", "Auxiliary LLM Chains")}
        </span>
        {/* The classification behind this badge lives in kernel
            `build_reload_plan` (docs/operations/config-reload.md, row `llm`);
            if the two ever diverge this badge lies — the doc is the tie. */}
        <Badge variant="info">{t("config.hot_reload", "Hot Reload")}</Badge>
      </div>
      <p className="px-5 py-2 text-[11px] text-text-dim">
        {t(
          "config.auxiliary_llm_description",
          "Route internal side-tasks to cheaper models. Empty = uses primary model.",
        )}
      </p>
      <div className="divide-y divide-border-subtle/30">
        {auxTasks.map((task) => {
          const chain = auxiliary[task] ?? [];
          const isEditing = editing === task;

          return (
            <div key={task} className="px-5 py-3">
              <div className="flex items-center justify-between gap-2">
                <span className="text-xs font-medium text-text-main min-w-0 truncate">
                  {t(`config.auxiliary_task_${task}`, task.replace(/_/g, " "))}
                </span>
                {!isEditing && (
                  <div className="flex items-center gap-2 shrink-0">
                    <span className="text-[11px] text-text-dim font-mono truncate max-w-[300px]">
                      {chain.length > 0
                        ? chain.join(" → ")
                        : t("config.auxiliary_chain_default", "Primary (default)")}
                    </span>
                    <Button variant="ghost" size="sm" onClick={() => startEdit(task)}>
                      {t("common.edit", "Edit")}
                    </Button>
                  </div>
                )}
              </div>

              {isEditing && (
                <div className="mt-2 space-y-1.5">
                  {draft.map((entry, i) => (
                    <div key={i} className="flex items-center gap-1.5">
                      <input
                        type="text"
                        value={entry}
                        onChange={(e) => {
                          const next = [...draft];
                          next[i] = e.target.value;
                          setDraft(next);
                        }}
                        placeholder={t(
                          "config.auxiliary_chain_placeholder",
                          "provider:model",
                        )}
                        list={`aux-chain-suggestions-${i}`}
                        className="flex-1 rounded-lg border border-border-subtle bg-main px-2.5 py-1.5 text-xs font-mono outline-none focus:border-brand"
                      />
                      <datalist id={`aux-chain-suggestions-${i}`}>
                        {modelOptions.map((m) => (
                          <option key={m.id} value={`${m.provider}:${m.id}`}>
                            {m.display_name ?? m.id}
                          </option>
                        ))}
                      </datalist>
                      <button
                        type="button"
                        onClick={() => setDraft(draft.filter((_, j) => j !== i))}
                        className="p-1 text-text-dim hover:text-error"
                      >
                        <Trash2 className="w-3 h-3" />
                      </button>
                    </div>
                  ))}
                  <div className="flex items-center gap-2 pt-1">
                    <button
                      type="button"
                      onClick={() => setDraft([...draft, ""])}
                      className="flex items-center gap-1 text-[11px] text-brand hover:underline"
                    >
                      <Plus className="w-3 h-3" />
                      {t("config.auxiliary_chain_add", "Add model")}
                    </button>
                    <div className="ml-auto flex items-center gap-1.5">
                      <Button variant="ghost" size="sm" onClick={cancelEdit}>
                        <X className="w-3 h-3 mr-0.5" />
                        {t("common.cancel", "Cancel")}
                      </Button>
                      <Button
                        variant="primary"
                        size="sm"
                        onClick={saveEdit}
                        isLoading={setConfig.isPending}
                      >
                        <Save className="w-3 h-3 mr-0.5" />
                        {t("common.save", "Save")}
                      </Button>
                    </div>
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}