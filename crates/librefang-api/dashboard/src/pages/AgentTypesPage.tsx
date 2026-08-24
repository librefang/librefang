import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import { Edit2, LayoutTemplate, Lock, Plus, Trash2 } from "lucide-react";
import type { AgentTemplate, AgentTypeSpec } from "../api";
import { useAgentType, useAgentTypes } from "../lib/queries/agentTypes";
import { useTools } from "../lib/queries/agents";
import { useSkills } from "../lib/queries/skills";
import {
  useCreateAgentType,
  useDeleteAgentType,
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

function AgentTypeRow({
  type,
  onEdit,
  onDelete,
}: {
  type: AgentTemplate;
  onEdit: () => void;
  onDelete: () => void;
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

      {/* A workspace-sourced row is a live agent's own manifest. The write verbs
          refuse it by design, so rendering Edit/Delete here would offer a control
          that cannot succeed — point at the surface that can instead (#7731). */}
      {type.editable ? (
        <div className="flex shrink-0 items-center gap-1">
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
        </div>
      ) : (
        <Link
          to="/agents"
          className="flex shrink-0 items-center gap-1 rounded-lg border border-border-subtle px-2 py-1 text-[11px] text-text-dim hover:text-text-main"
          title={t("agentTypes.managed_elsewhere_hint")}
        >
          <Lock className="h-3 w-3" />
          {t("agentTypes.managed_elsewhere")}
        </Link>
      )}
    </div>
  );
}

export function AgentTypesPage() {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const types = useAgentTypes();
  const deleteMutation = useDeleteAgentType();

  const [editing, setEditing] = useState<{ name: string | null } | null>(null);
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);

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
              onEdit={() => setEditing({ name: type.name })}
              onDelete={() => setPendingDelete(type.name)}
            />
          ))}
        </div>
      )}

      {editing && (
        <AgentTypeEditor name={editing.name} onClose={() => setEditing(null)} />
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
