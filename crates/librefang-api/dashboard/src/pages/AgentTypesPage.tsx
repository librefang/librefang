import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import { Edit2, History, LayoutTemplate, Lock, Play, Plus, RotateCcw, ShieldCheck, Trash2 } from "lucide-react";
import type { AgentTemplate, AgentTypeSpec, SpawnEphemeralResult } from "../api";
import { useAgentType, useAgentTypes, useAgentTypeHistory } from "../lib/queries/agentTypes";
import { useAgents, useTools } from "../lib/queries/agents";
import { useSkills } from "../lib/queries/skills";
import {
  useCreateAgentType,
  useDeleteAgentType,
  useRestoreTemplateVersion,
  useSpawnEphemeral,
  useUpdateAgentType,
} from "../lib/mutations/agentTypes";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { ErrorState } from "../components/ui/ErrorState";
import { EmptyState } from "../components/ui/EmptyState";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Modal } from "../components/ui/Modal";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { MultiSelectCmdk } from "../components/ui/MultiSelectCmdk";
import { useUIStore } from "../lib/store";
import { toastErr } from "../lib/errors";
import { copyToClipboard } from "../lib/clipboard";

/**
 * The subset of an agent type this editor writes.
 *
 * `name` is absent on purpose: it identifies the document and the `PUT` route
 * takes it from the URL, so the form cannot rename a type out from under itself.
 */
interface FormState {
  description: string;
  system_prompt: string;
  provider: string;
  model: string;
  tools: string[];
  skills: string[];
}

const EMPTY_FORM: FormState = {
  description: "",
  system_prompt: "",
  provider: "",
  model: "",
  tools: [],
  skills: [],
};

function formFromSpec(spec: AgentTypeSpec): FormState {
  return {
    description: spec.description ?? "",
    system_prompt: spec.system_prompt ?? "",
    provider: spec.provider ?? "",
    model: spec.model ?? "",
    tools: spec.tools ?? [],
    skills: spec.skills ?? [],
  };
}

const inputClass =
  "w-full rounded-lg border border-border-subtle bg-main/40 px-2.5 py-1.5 text-[13px] " +
  "text-text-main placeholder:text-text-dim/50 focus:border-brand/50 focus:outline-none";

/**
 * Union the catalog with what the type already references, so an identifier the
 * registry does not know about (a skill installed on another host, a tool from a
 * plugin that has not loaded yet) still renders as a chip instead of vanishing
 * from the form and, with it, from the saved document.
 */
function mergeCatalog(
  catalog: { name: string; description?: string }[] | undefined,
  selected: string[],
): { options: string[]; meta: Record<string, { description?: string }> } {
  const meta: Record<string, { description?: string }> = {};
  const options = new Set<string>();
  for (const entry of catalog ?? []) {
    options.add(entry.name);
    if (entry.description) meta[entry.name] = { description: entry.description };
  }
  for (const name of selected) options.add(name);
  return { options: [...options].sort(), meta };
}

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

function AgentTypeEditor({
  name,
  onClose,
}: {
  /** `null` opens the create form; a string opens the editor for that type. */
  name: string | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const isCreate = name === null;

  const detail = useAgentType(name ?? "", { enabled: !isCreate });
  const toolsQuery = useTools();
  const skillsQuery = useSkills();
  const createMutation = useCreateAgentType();
  const updateMutation = useUpdateAgentType();

  const [newName, setNewName] = useState("");
  const [form, setForm] = useState<FormState>(EMPTY_FORM);

  const spec = detail.data?.spec;
  useEffect(() => {
    setForm(spec ? formFromSpec(spec) : EMPTY_FORM);
  }, [spec]);

  const toolFinder = useMemo(
    () => mergeCatalog(toolsQuery.data, form.tools),
    [toolsQuery.data, form.tools],
  );
  const skillFinder = useMemo(
    () => mergeCatalog(skillsQuery.data, form.skills),
    [skillsQuery.data, form.skills],
  );

  const saving = createMutation.isPending || updateMutation.isPending;
  const update = (patch: Partial<FormState>) => setForm((prev) => ({ ...prev, ...patch }));

  async function handleSave() {
    // Send exactly the keys this form owns. The server merges them over the
    // stored manifest, so everything it does not mention — triggers, compaction,
    // MCP allowlists, session mode — survives the save untouched (#7740).
    const payload: AgentTypeSpec = { ...form };
    try {
      if (isCreate) {
        await createMutation.mutateAsync({ ...payload, name: newName.trim() });
        addToast(t("agentTypes.created"), "success");
      } else {
        await updateMutation.mutateAsync({ name: name as string, spec: payload });
        addToast(t("agentTypes.saved"), "success");
      }
      onClose();
    } catch (err) {
      addToast(toastErr(err, t("agentTypes.save_failed")), "error");
    }
  }

  return (
    <Modal
      isOpen
      onClose={onClose}
      variant="panel-right"
      size="lg"
      overflowVisible
      title={isCreate ? t("agentTypes.create_title") : t("agentTypes.edit_title", { name })}
    >
      {!isCreate && detail.isLoading ? (
        <ListSkeleton rows={4} />
      ) : !isCreate && detail.isError ? (
        <ErrorState message={detail.error?.message} onRetry={() => void detail.refetch()} />
      ) : (
        <div className="space-y-4">
          {isCreate && (
            <Field label={t("agentTypes.name")} hint={t("agentTypes.name_hint")}>
              <input
                type="text"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                placeholder={t("agentTypes.name_placeholder")}
                className={inputClass}
                autoFocus
              />
            </Field>
          )}

          <Field label={t("agentTypes.description")}>
            <input
              type="text"
              value={form.description}
              onChange={(e) => update({ description: e.target.value })}
              className={inputClass}
            />
          </Field>

          <Field label={t("agentTypes.system_prompt")} hint={t("agentTypes.system_prompt_hint")}>
            <textarea
              value={form.system_prompt}
              onChange={(e) => update({ system_prompt: e.target.value })}
              rows={6}
              className={`${inputClass} font-mono resize-y`}
            />
          </Field>

          <div className="grid grid-cols-2 gap-3">
            <Field label={t("agentTypes.provider")} hint={t("agentTypes.provider_hint")}>
              <input
                type="text"
                value={form.provider}
                onChange={(e) => update({ provider: e.target.value })}
                placeholder={t("agentTypes.inherit_placeholder")}
                className={inputClass}
              />
            </Field>
            <Field label={t("agentTypes.model")} hint={t("agentTypes.model_hint")}>
              <input
                type="text"
                value={form.model}
                onChange={(e) => update({ model: e.target.value })}
                placeholder={t("agentTypes.inherit_placeholder")}
                className={inputClass}
              />
            </Field>
          </div>

          <Field label={t("agentTypes.tools")} hint={t("agentTypes.tools_hint")}>
            <MultiSelectCmdk
              options={toolFinder.options}
              optionMeta={toolFinder.meta}
              value={form.tools}
              onChange={(next) =>
                update({ tools: typeof next === "function" ? next(form.tools) : next })
              }
              placeholder={t("agentTypes.tools_search")}
              allowFreeText
            />
          </Field>

          <Field label={t("agentTypes.skills")} hint={t("agentTypes.skills_hint")}>
            <MultiSelectCmdk
              options={skillFinder.options}
              optionMeta={skillFinder.meta}
              value={form.skills}
              onChange={(next) =>
                update({ skills: typeof next === "function" ? next(form.skills) : next })
              }
              placeholder={t("agentTypes.skills_search")}
              allowFreeText
            />
          </Field>

          <p className="rounded-lg border border-border-subtle bg-main/30 px-3 py-2 text-[11px] text-text-dim">
            {t("agentTypes.preserved_note")}
          </p>

          <div className="flex justify-end gap-2 pt-1">
            <Button variant="ghost" onClick={onClose} disabled={saving}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="primary"
              onClick={() => void handleSave()}
              isLoading={saving}
              disabled={isCreate && newName.trim() === ""}
            >
              {t("common.save")}
            </Button>
          </div>
        </div>
      )}
    </Modal>
  );
}

/**
 * Run an agent type once, on the spot, and show what came back (#6699).
 *
 * The run is an *ephemeral worker*: no agent is registered, no session is
 * persisted, and the mission workspace is deleted when the turn ends. The only
 * thing that outlives it is the text below and the spend on the parent's ledger
 * — which is why picking the parent is a deliberate choice here and not a
 * hidden default. The parent is billed for the run, its `[resources]` quota is
 * the one enforced, and its own tool set is the ceiling on the worker's.
 */
function QuickRunModal({
  type,
  onClose,
}: {
  type: AgentTemplate;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const agents = useAgents();
  const spawn = useSpawnEphemeral();

  const [parent, setParent] = useState("");
  const [task, setTask] = useState("");
  const [result, setResult] = useState<SpawnEphemeralResult | null>(null);

  const candidates = useMemo(
    () => (agents.data ?? []).filter((a) => !a.is_hand),
    [agents.data],
  );

  // Preselect the first agent so the common case is two fields, not three.
  // Guarded on `parent` staying empty so a refetch never moves a choice the
  // operator already made.
  useEffect(() => {
    if (parent === "" && candidates.length > 0) setParent(candidates[0].id);
  }, [candidates, parent]);

  async function run() {
    try {
      const res = await spawn.mutateAsync({
        parent,
        message: task,
        agent_type: type.name,
        label: type.name,
      });
      setResult(res);
    } catch (err) {
      addToast(toastErr(err, t("agentTypes.quick_run_failed")), "error");
    }
  }

  return (
    <Modal
      isOpen
      onClose={onClose}
      variant="panel-right"
      size="lg"
      title={t("agentTypes.quick_run_title", { name: type.name })}
    >
      <div className="space-y-4">
        <Field label={t("agentTypes.quick_run_parent")} hint={t("agentTypes.quick_run_parent_hint")}>
          {agents.isLoading ? (
            <ListSkeleton rows={1} />
          ) : candidates.length === 0 ? (
            <p className="text-[12px] text-text-dim">{t("agentTypes.quick_run_no_agents")}</p>
          ) : (
            <select
              value={parent}
              onChange={(e) => setParent(e.target.value)}
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

        <Field label={t("agentTypes.quick_run_task")}>
          <textarea
            value={task}
            onChange={(e) => setTask(e.target.value)}
            rows={5}
            placeholder={t("agentTypes.quick_run_task_placeholder")}
            className={`${inputClass} resize-y`}
            autoFocus
          />
        </Field>

        {result && (
          <div className="space-y-2 rounded-xl border border-border-subtle bg-main/30 px-3 py-2.5">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-[11px] font-semibold uppercase tracking-wide text-text-dim">
                {t("agentTypes.quick_run_result")}
              </span>
              <Badge variant="default">{result.name}</Badge>
              <span className="text-[11px] text-text-dim">
                {t("agentTypes.quick_run_meta", {
                  iterations: result.iterations,
                  tools: result.tools.length,
                })}
              </span>
              {typeof result.cost_usd === "number" && (
                <span className="text-[11px] text-text-dim">
                  {t("agentTypes.quick_run_cost", { cost: result.cost_usd.toFixed(4) })}
                </span>
              )}
            </div>
            <p className="whitespace-pre-wrap break-words text-[13px] text-text-main">
              {result.response}
            </p>
            <p className="text-[11px] text-text-dim/70">
              {t("agentTypes.quick_run_ephemeral_note")}
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
            {t("agentTypes.quick_run_submit")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

/**
 * Read-only privacy pass over an agent type, ahead of contributing it to a
 * shared registry (#7771). The backend already sanitizes and scans the
 * manifest; this just shows the operator what would ship.
 */
function PromotionPreviewModal({ name, onClose }: { name: string; onClose: () => void }) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const detail = useAgentType(name);
  const preview = detail.data?.promotion_preview;

  async function copyToml() {
    if (!preview?.manifest_toml) return;
    if (await copyToClipboard(preview.manifest_toml)) {
      addToast(t("agentTypes.promote_copied"), "success");
    } else {
      addToast(t("agentTypes.promote_copy_failed"), "error");
    }
  }

  return (
    <Modal
      isOpen
      onClose={onClose}
      variant="panel-right"
      size="lg"
      title={t("agentTypes.promote_title", { name })}
    >
      {detail.isLoading ? (
        <ListSkeleton rows={4} />
      ) : detail.isError ? (
        <ErrorState message={detail.error?.message} onRetry={() => void detail.refetch()} />
      ) : !preview ? (
        <p className="text-[12px] text-text-dim">{t("agentTypes.promote_unavailable")}</p>
      ) : (
        <div className="space-y-4">
          {preview.requires_review && (
            <p className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-[12px] text-warning">
              {t("agentTypes.promote_review_warning")}
            </p>
          )}

          {preview.findings.length > 0 && (
            <div className="space-y-1.5">
              <p className="text-[11px] font-semibold uppercase tracking-wide text-text-dim">
                {t("agentTypes.promote_findings")}
              </p>
              {preview.findings.map((finding, i) => (
                <div
                  key={`${finding.field}:${i}`}
                  className="flex items-start justify-between gap-2 rounded-lg border border-border-subtle bg-main/30 px-2.5 py-1.5"
                >
                  <div className="min-w-0">
                    <span className="font-mono text-[12px] text-text-main">{finding.field}</span>
                    <p className="truncate text-[11px] text-text-dim">{finding.preview}</p>
                  </div>
                  <Badge variant={finding.removed_by_sanitizer ? "default" : "warning"}>
                    {finding.removed_by_sanitizer
                      ? t("agentTypes.promote_stripped")
                      : t("agentTypes.promote_review")}
                  </Badge>
                </div>
              ))}
            </div>
          )}

          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <p className="text-[11px] font-semibold uppercase tracking-wide text-text-dim">
                {t("agentTypes.promote_toml")}
              </p>
              <Button variant="ghost" size="sm" onClick={() => void copyToml()} disabled={!preview.manifest_toml}>
                {t("agentTypes.promote_copy")}
              </Button>
            </div>
            <pre className="max-h-96 overflow-auto rounded-lg border border-border-subtle bg-main/40 p-3 text-[11px] text-text-main whitespace-pre-wrap">
              {preview.manifest_toml ?? t("agentTypes.promote_toml_unavailable")}
            </pre>
          </div>

          <div className="flex justify-end pt-1">
            <Button variant="ghost" onClick={onClose}>
              {t("common.close")}
            </Button>
          </div>
        </div>
      )}
    </Modal>
  );
}

function TemplateHistoryModal({
  name,
  onClose,
}: {
  name: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const history = useAgentTypeHistory(name, { enabled: true });
  const restore = useRestoreTemplateVersion();
  const [expanded, setExpanded] = useState<number | null>(null);

  return (
    <Modal
      isOpen
      onClose={onClose}
      variant="panel-right"
      size="lg"
      title={t("agentTypes.history_title", { name, defaultValue: `History: ${name}` })}
    >
      {history.isLoading ? (
        <ListSkeleton rows={4} />
      ) : history.isError ? (
        <ErrorState message={history.error?.message} onRetry={() => void history.refetch()} />
      ) : (history.data?.versions ?? []).length === 0 ? (
        <EmptyState
          icon={<History className="h-5 w-5" />}
          title={t("agentTypes.history_empty", { defaultValue: "No version history yet" })}
          description={t("agentTypes.history_empty_desc", {
            defaultValue: "Version snapshots are recorded when a template is created or edited.",
          })}
        />
      ) : (
        <div className="space-y-2">
          {(history.data?.versions ?? []).map((v) => (
            <div
              key={v.id}
              className="rounded-lg border border-border-subtle bg-surface-dim px-3 py-2"
            >
              <div className="flex items-center justify-between gap-2">
                <div className="min-w-0">
                  <span className="text-[12px] font-medium text-text-main">
                    {new Date(v.timestamp + "Z").toLocaleString()}
                  </span>
                  <Badge variant="default" className="ml-2">{v.change_source}</Badge>
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  <button
                    type="button"
                    onClick={() => setExpanded(expanded === v.id ? null : v.id)}
                    className="rounded-lg px-2 py-1 text-[11px] text-text-dim hover:bg-main/50 hover:text-text-main"
                  >
                    {expanded === v.id ? t("common.collapse", { defaultValue: "Collapse" }) : t("common.expand", { defaultValue: "Expand" })}
                  </button>
                  <button
                    type="button"
                    onClick={async () => {
                      try {
                        await restore.mutateAsync({ name, versionId: v.id });
                        addToast(t("agentTypes.restored", { defaultValue: "Version restored" }), "success");
                        onClose();
                      } catch (err) {
                        addToast(toastErr(err, t("agentTypes.restore_failed", { defaultValue: "Restore failed" })), "error");
                      }
                    }}
                    disabled={restore.isPending}
                    className="flex items-center gap-1 rounded-lg px-2 py-1 text-[11px] text-text-dim hover:bg-brand/10 hover:text-brand"
                    title={t("agentTypes.restore", { defaultValue: "Restore this version" })}
                  >
                    <RotateCcw className="h-3 w-3" />
                    {t("agentTypes.restore_btn", { defaultValue: "Restore" })}
                  </button>
                </div>
              </div>
              {expanded === v.id && (
                <pre className="mt-2 max-h-64 overflow-auto rounded border border-border-subtle bg-surface p-2 text-[11px] text-text-dim">
                  {v.manifest_toml}
                </pre>
              )}
            </div>
          ))}
        </div>
      )}
    </Modal>
  );
}

function AgentTypeRow({
  type,
  onQuickRun,
  onEdit,
  onDelete,
  onPromote,
  onHistory,
}: {
  type: AgentTemplate;
  onQuickRun: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onPromote: () => void;
  onHistory: () => void;
}) {
  const { t } = useTranslation();

  return (
    <div className="flex items-start justify-between gap-3 rounded-xl border border-border-subtle bg-surface px-3 py-2.5">
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="truncate text-[13px] font-semibold text-text-main">{type.name}</span>
          {type.provider && type.model && (
            <Badge variant="default">{`${type.provider} / ${type.model}`}</Badge>
          )}
        </div>
        {type.description && (
          <p className="mt-0.5 truncate text-[12px] text-text-dim">{type.description}</p>
        )}
      </div>

      <div className="flex shrink-0 items-center gap-1">
        {/* Quick Run is offered on every row, editable or not. Spawnability and
            writability are different questions: a workspace-sourced row is a live
            agent's manifest this API refuses to edit, but the spawn engine resolves
            it by name just as happily as an operator-authored type (#6699). */}
        <button
          type="button"
          onClick={onQuickRun}
          className="rounded-lg p-1.5 text-text-dim hover:bg-main/50 hover:text-brand"
          aria-label={t("agentTypes.quick_run")}
          title={t("agentTypes.quick_run")}
        >
          <Play className="h-3.5 w-3.5" />
        </button>

        <button
          type="button"
          onClick={onPromote}
          className="rounded-lg p-1.5 text-text-dim hover:bg-main/50 hover:text-brand"
          aria-label={t("agentTypes.promote")}
          title={t("agentTypes.promote")}
        >
          <ShieldCheck className="h-3.5 w-3.5" />
        </button>

        {/* A workspace-sourced row is a live agent's own manifest. The write verbs
            refuse it by design, so rendering Edit/Delete here would offer a control
            that cannot succeed — point at the surface that can instead (#7731). */}
        {type.editable ? (
          <>
            <button
              type="button"
              onClick={onHistory}
              className="rounded-lg p-1.5 text-text-dim hover:bg-main/50 hover:text-text-main"
              aria-label={t("agentTypes.history", { defaultValue: "History" })}
              title={t("agentTypes.history", { defaultValue: "History" })}
            >
              <History className="h-3.5 w-3.5" />
            </button>
            <button
              type="button"
              onClick={onEdit}
              className="rounded-lg p-1.5 text-text-dim hover:bg-main/50 hover:text-text-main"
              aria-label={t("agentTypes.edit")}
              title={t("agentTypes.edit")}
            >
              <Edit2 className="h-3.5 w-3.5" />
            </button>
            <button
              type="button"
              onClick={onDelete}
              className="rounded-lg p-1.5 text-text-dim hover:bg-error/10 hover:text-error"
              aria-label={t("agentTypes.delete")}
              title={t("agentTypes.delete")}
            >
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          </>
        ) : (
          <Link
            to="/agents"
            className="flex items-center gap-1 rounded-lg border border-border-subtle px-2 py-1 text-[11px] text-text-dim hover:text-text-main"
            title={t("agentTypes.managed_elsewhere_hint")}
          >
            <Lock className="h-3 w-3" />
            {t("agentTypes.managed_elsewhere")}
          </Link>
        )}
      </div>
    </div>
  );
}

export function AgentTypesPage() {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const types = useAgentTypes();
  const deleteMutation = useDeleteAgentType();

  const [editing, setEditing] = useState<{ name: string | null } | null>(null);
  const [quickRun, setQuickRun] = useState<AgentTemplate | null>(null);
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const [promoting, setPromoting] = useState<string | null>(null);
  const [historyName, setHistoryName] = useState<string | null>(null);

  async function confirmDelete() {
    if (!pendingDelete) return;
    try {
      await deleteMutation.mutateAsync(pendingDelete);
      addToast(t("agentTypes.deleted"), "success");
    } catch (err) {
      addToast(toastErr(err, t("agentTypes.delete_failed")), "error");
    } finally {
      setPendingDelete(null);
    }
  }

  return (
    <div className="space-y-4">
      <PageHeader
        icon={<LayoutTemplate className="h-4 w-4" />}
        title={t("agentTypes.title")}
        subtitle={t("agentTypes.subtitle")}
        isFetching={types.isFetching}
        onRefresh={() => void types.refetch()}
        actions={
          <Button
            variant="primary"
            leftIcon={<Plus className="h-3.5 w-3.5" />}
            onClick={() => setEditing({ name: null })}
          >
            {t("agentTypes.new")}
          </Button>
        }
      />

      {types.isLoading ? (
        <ListSkeleton rows={4} />
      ) : types.isError ? (
        <ErrorState message={types.error?.message} onRetry={() => void types.refetch()} />
      ) : (types.data ?? []).length === 0 ? (
        <EmptyState
          icon={<LayoutTemplate className="h-5 w-5" />}
          title={t("agentTypes.empty_title")}
          description={t("agentTypes.empty_description")}
        />
      ) : (
        <div className="space-y-2">
          {(types.data ?? []).map((type) => (
            <AgentTypeRow
              key={`${type.source}:${type.name}`}
              type={type}
              onQuickRun={() => setQuickRun(type)}
              onEdit={() => setEditing({ name: type.name })}
              onDelete={() => setPendingDelete(type.name)}
              onPromote={() => setPromoting(type.name)}
              onHistory={() => setHistoryName(type.name)}
            />
          ))}
        </div>
      )}

      {editing && (
        <AgentTypeEditor name={editing.name} onClose={() => setEditing(null)} />
      )}

      {quickRun && <QuickRunModal type={quickRun} onClose={() => setQuickRun(null)} />}

      {promoting && (
        <PromotionPreviewModal name={promoting} onClose={() => setPromoting(null)} />
      )}

      {historyName && (
        <TemplateHistoryModal name={historyName} onClose={() => setHistoryName(null)} />
      )}

      <ConfirmDialog
        isOpen={pendingDelete !== null}
        onClose={() => setPendingDelete(null)}
        onConfirm={() => void confirmDelete()}
        title={t("agentTypes.delete")}
        message={t("agentTypes.confirm_delete", { name: pendingDelete ?? "" })}
        tone="destructive"
      />
    </div>
  );
}
