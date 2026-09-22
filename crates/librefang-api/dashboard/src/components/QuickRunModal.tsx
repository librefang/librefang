import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Play } from "lucide-react";
import type { SpawnEphemeralResult } from "../api";
import { useAgents, useAgentTemplates } from "../lib/queries/agents";
import { useSpawnEphemeral } from "../lib/mutations/agents";
import { useUIStore } from "../lib/store";
import { toastErr } from "../lib/errors";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { Modal } from "./ui/Modal";
import { ListSkeleton } from "./ui/Skeleton";

const inputClass =
  "w-full rounded-lg border border-border-subtle bg-main/40 px-2.5 py-1.5 text-[13px] " +
  "text-text-main placeholder:text-text-dim/50 focus:border-brand/50 focus:outline-none";

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-1">
      <label className="block text-[11px] font-semibold uppercase tracking-wide text-text-dim">
        {label}
      </label>
      {children}
      {hint && <p className="text-[11px] text-text-dim/70">{hint}</p>}
    </div>
  );
}

/**
 * Run one task on the spot and show what came back (#6699).
 *
 * The run is an *ephemeral worker*: no agent is registered, no session is
 * persisted, and the mission workspace is deleted when the turn ends. The only
 * thing that outlives it is the text below and the spend on the parent's ledger
 * — which is why picking the parent is a deliberate choice here and not a
 * hidden default. The parent is billed for the run, its `[resources]` quota is
 * the one enforced, and its own tool set is the ceiling on the worker's.
 *
 * `initialParent` is the agent this was opened from, and is preselected so the
 * common case is two fields rather than three. It is a preference, not a lock:
 * the picker stays live, and an id that is not a viable parent (a hand, or one
 * deleted since the page loaded) falls back to the first candidate rather than
 * leaving the select blank.
 *
 * The type picker is what makes the run *a type* rather than *a copy of the
 * parent*: naming one sends `agent_type`, and the kernel loads that template's
 * manifest for the worker's system prompt, model and declared tools. Leaving it
 * unset is the documented server behaviour — the worker manifest becomes
 * `parent.manifest.clone()` — so "run as the parent" stays available and is
 * labelled as such rather than being an implicit default nobody can see.
 */
export function QuickRunModal({
  initialParent,
  onClose,
}: {
  initialParent?: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const agents = useAgents();
  const templates = useAgentTemplates();
  const spawn = useSpawnEphemeral();

  const [parent, setParent] = useState("");
  const [typeName, setTypeName] = useState("");
  const [task, setTask] = useState("");
  const [result, setResult] = useState<SpawnEphemeralResult | null>(null);

  // Sorted so the list is stable across renders; `useAgentTemplates` is backed
  // by the same `/api/templates` listing the Agent Types page renders.
  const typeOptions = useMemo(
    () => (templates.data ?? []).map((template) => template.name).sort(),
    [templates.data],
  );

  // A hand cannot be a parent: the parent supplies the budget, the resource
  // quota and the tool ceiling, and a hand owns none of its own.
  const candidates = useMemo(
    () => (agents.data ?? []).filter((a) => !a.is_hand),
    [agents.data],
  );

  // Preselect the agent the modal was opened from so the common case is two
  // fields, not three. Guarded on `parent` staying empty so a refetch never
  // moves a choice the operator already made.
  useEffect(() => {
    if (parent !== "" || candidates.length === 0) return;
    const preferred = candidates.find((a) => a.id === initialParent);
    setParent(preferred ? preferred.id : candidates[0].id);
  }, [candidates, parent, initialParent]);

  async function run() {
    try {
      const res = await spawn.mutateAsync({
        parent,
        message: task,
        // Both or neither: `agent_type` picks the template manifest, and the
        // server defaults `label` to it so the worker and its mission
        // workspace carry the type's name.
        ...(typeName === "" ? {} : { agent_type: typeName, label: typeName }),
      });
      setResult(res);
    } catch (err) {
      addToast(toastErr(err, t("agents.quick_run_failed")), "error");
    }
  }

  // Name what is about to run: the type when one is picked, otherwise the
  // agent standing in for it.
  const subject = typeName || candidates.find((a) => a.id === parent)?.name || "";

  return (
    <Modal
      isOpen
      onClose={onClose}
      variant="panel-right"
      size="lg"
      title={
        subject === ""
          ? t("agents.quick_run")
          : t("agents.quick_run_title", { name: subject })
      }
    >
      <div className="space-y-4">
        <Field label={t("agents.quick_run_parent")} hint={t("agents.quick_run_parent_hint")}>
          {agents.isLoading ? (
            <ListSkeleton rows={1} />
          ) : candidates.length === 0 ? (
            <p className="text-[12px] text-text-dim">{t("agents.quick_run_no_agents")}</p>
          ) : (
            <select
              value={parent}
              onChange={(e) => setParent(e.target.value)}
              aria-label={t("agents.quick_run_parent")}
              className={inputClass}
            >
              {candidates.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.name}
                </option>
              ))}
            </select>
          )}
        </Field>

        <Field label={t("agents.quick_run_type")} hint={t("agents.quick_run_type_hint")}>
          {templates.isLoading ? (
            <ListSkeleton rows={1} />
          ) : (
            <select
              value={typeName}
              onChange={(e) => setTypeName(e.target.value)}
              aria-label={t("agents.quick_run_type")}
              className={inputClass}
            >
              <option value="">{t("agents.quick_run_type_none")}</option>
              {typeOptions.map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </select>
          )}
        </Field>

        <Field label={t("agents.quick_run_task")}>
          <textarea
            value={task}
            onChange={(e) => setTask(e.target.value)}
            rows={5}
            placeholder={t("agents.quick_run_task_placeholder")}
            aria-label={t("agents.quick_run_task")}
            className={`${inputClass} resize-y`}
            autoFocus
          />
        </Field>

        {result && (
          <div className="space-y-2 rounded-xl border border-border-subtle bg-main/30 px-3 py-2.5">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-[11px] font-semibold uppercase tracking-wide text-text-dim">
                {t("agents.quick_run_result")}
              </span>
              <Badge variant="default">{result.name}</Badge>
              <span className="text-[11px] text-text-dim">
                {t("agents.quick_run_meta", {
                  iterations: result.iterations,
                  tools: result.tools.length,
                })}
              </span>
              {typeof result.cost_usd === "number" && (
                <span className="text-[11px] text-text-dim">
                  {t("agents.quick_run_cost", { cost: result.cost_usd.toFixed(4) })}
                </span>
              )}
            </div>
            <p className="whitespace-pre-wrap break-words text-[13px] text-text-main">
              {result.response}
            </p>
            <p className="text-[11px] text-text-dim/70">
              {t("agents.quick_run_ephemeral_note")}
            </p>
          </div>
        )}

        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" onClick={onClose} disabled={spawn.isPending}>
            {t("common.close")}
          </Button>
          <Button
            variant="primary"
            leftIcon={<Play className="h-3.5 w-3.5" />}
            onClick={() => void run()}
            isLoading={spawn.isPending}
            disabled={parent === "" || task.trim() === ""}
          >
            {t("agents.quick_run_submit")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
