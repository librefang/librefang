import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Boxes, Plus, Pencil, Trash2, Zap, Loader2 } from "lucide-react";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { Modal } from "../components/ui/Modal";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { MarkdownContent } from "../components/ui/MarkdownContent";
import { toastErr } from "../lib/errors";
import type {
  AgentType,
  AgentTypeInput,
  AgentTypeSummary,
  EphemeralResult,
} from "../api";
import { useAgentTypes, useAgentType } from "../lib/queries/agentTypes";
import {
  useCreateAgentType,
  useUpdateAgentType,
  useDeleteAgentType,
  useSpawnEphemeral,
} from "../lib/mutations/agentTypes";

const TEXTAREA_CLASS =
  "w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm font-mono leading-relaxed resize-y disabled:opacity-50 focus:outline-none focus:border-brand";

interface FormState {
  name: string;
  description: string;
  system_prompt: string;
  provider: string;
  model: string;
  tools: string;
}

const EMPTY_FORM: FormState = {
  name: "",
  description: "",
  system_prompt: "",
  provider: "",
  model: "",
  tools: "",
};

function toForm(type: AgentType): FormState {
  return {
    name: type.name ?? "",
    description: type.description ?? "",
    system_prompt: type.system_prompt ?? "",
    provider: type.provider ?? "",
    model: type.model ?? "",
    tools: (type.tools ?? []).join(", "),
  };
}

function splitList(raw: string): string[] {
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

function toInput(form: FormState): AgentTypeInput {
  return {
    name: form.name.trim(),
    description: form.description.trim() || undefined,
    system_prompt: form.system_prompt.trim() || undefined,
    provider: form.provider.trim() || undefined,
    model: form.model.trim() || undefined,
    tools: splitList(form.tools),
  };
}

export function AgentTypesPage() {
  const { t } = useTranslation();
  const { data: types, isLoading, isFetching, refetch } = useAgentTypes();

  const createType = useCreateAgentType();
  const updateType = useUpdateAgentType();
  const deleteType = useDeleteAgentType();
  const spawn = useSpawnEphemeral();

  // Create/edit dialog. `editing` is null while creating, or the type name
  // while editing (the name field is locked on edit so the PUT path stays
  // stable). The edit form is populated from the detail fetch below.
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);

  // Detail fetch that backs the edit form. Disabled until a name is selected.
  const detail = useAgentType(editing ?? "");
  // Track which type we've loaded so a re-render can't clobber in-progress edits.
  const loadedFor = useRef<string | null>(null);
  useEffect(() => {
    if (editing && detail.data && loadedFor.current !== editing) {
      setForm(toForm(detail.data));
      loadedFor.current = editing;
    }
  }, [editing, detail.data]);

  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

  // Quick-run dialog.
  const [runTarget, setRunTarget] = useState<string | null>(null);
  const [runMessage, setRunMessage] = useState("");
  const [runResult, setRunResult] = useState<EphemeralResult | null>(null);

  function openCreate() {
    setEditing(null);
    loadedFor.current = null;
    setForm(EMPTY_FORM);
    setFormOpen(true);
  }

  function openEdit(type: AgentTypeSummary) {
    setEditing(type.name);
    loadedFor.current = null;
    setForm(EMPTY_FORM);
    setFormOpen(true);
  }

  function submitForm() {
    const input = toInput(form);
    if (!input.name) return;
    if (editing) {
      updateType.mutate(
        { name: editing, body: input },
        {
          onSuccess: () => setFormOpen(false),
          onError: (e) => toastErr(e, t("agentTypes.edit")),
        },
      );
    } else {
      createType.mutate(input, {
        onSuccess: () => setFormOpen(false),
        onError: (e) => toastErr(e, t("agentTypes.create")),
      });
    }
  }

  function openRun(type: AgentTypeSummary) {
    setRunTarget(type.name);
    setRunMessage("");
    setRunResult(null);
  }

  function submitRun() {
    if (!runTarget || !runMessage.trim()) return;
    spawn.mutate(
      { agent_type: runTarget, message: runMessage.trim() },
      {
        onSuccess: (res) => setRunResult(res),
        onError: (e) => toastErr(e, t("agentTypes.quickRun")),
      },
    );
  }

  const formPending = createType.isPending || updateType.isPending;
  const editLoading = !!editing && detail.isLoading;

  return (
    <div className="space-y-6">
      <PageHeader
        icon={<Boxes className="h-5 w-5" />}
        title={t("agentTypes.title")}
        isFetching={isFetching}
        onRefresh={() => refetch()}
        actions={
          <Button variant="primary" size="sm" onClick={openCreate}>
            <Plus className="h-4 w-4" />
            {t("agentTypes.create")}
          </Button>
        }
      />

      {isLoading ? (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          <CardSkeleton />
          <CardSkeleton />
          <CardSkeleton />
        </div>
      ) : !types || types.length === 0 ? (
        <EmptyState
          icon={<Boxes className="h-6 w-6" />}
          title={t("agentTypes.noTypes")}
          action={
            <Button variant="primary" size="sm" onClick={openCreate}>
              <Plus className="h-4 w-4" />
              {t("agentTypes.create")}
            </Button>
          }
        />
      ) : (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {types.map((type) => (
            <Card key={type.name} padding="md" className="flex flex-col gap-3">
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <h3 className="truncate text-sm font-bold text-text-main">
                    {type.name}
                  </h3>
                  {type.description && (
                    <p className="mt-1 line-clamp-2 text-xs text-text-dim">
                      {type.description}
                    </p>
                  )}
                </div>
                {type.source && <Badge variant="brand">{type.source}</Badge>}
              </div>

              <div className="mt-auto flex items-center gap-2 pt-1">
                <Button variant="primary" size="sm" onClick={() => openRun(type)}>
                  <Zap className="h-3.5 w-3.5" />
                  {t("agentTypes.quickRun")}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  aria-label={t("agentTypes.edit")}
                  onClick={() => openEdit(type)}
                >
                  <Pencil className="h-3.5 w-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  aria-label={t("agentTypes.delete")}
                  onClick={() => setDeleteTarget(type.name)}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </div>
            </Card>
          ))}
        </div>
      )}

      {/* Create / edit dialog — structured form */}
      <Modal
        isOpen={formOpen}
        onClose={() => setFormOpen(false)}
        title={editing ? t("agentTypes.edit") : t("agentTypes.create")}
        size="lg"
      >
        {editLoading ? (
          <div className="flex h-48 items-center justify-center">
            <Loader2 className="h-5 w-5 animate-spin text-text-dim" />
          </div>
        ) : (
          <div className="space-y-4">
            <Input
              label={t("agentTypes.name")}
              value={form.name}
              disabled={!!editing}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
            />
            <Input
              label={t("agentTypes.description")}
              value={form.description}
              onChange={(e) =>
                setForm({ ...form, description: e.target.value })
              }
            />
            <div className="flex flex-col gap-1.5">
              <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                {t("agentTypes.systemPrompt")}
              </label>
              <textarea
                value={form.system_prompt}
                rows={6}
                disabled={formPending}
                onChange={(e) =>
                  setForm({ ...form, system_prompt: e.target.value })
                }
                className={TEXTAREA_CLASS}
              />
            </div>
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
              <Input
                label={t("agentTypes.provider")}
                value={form.provider}
                onChange={(e) =>
                  setForm({ ...form, provider: e.target.value })
                }
              />
              <Input
                label={t("agentTypes.model")}
                value={form.model}
                onChange={(e) => setForm({ ...form, model: e.target.value })}
              />
            </div>
            <Input
              label={t("agentTypes.tools")}
              value={form.tools}
              placeholder="web_fetch, file_read"
              onChange={(e) => setForm({ ...form, tools: e.target.value })}
            />
            <div className="flex justify-end gap-2 pt-2">
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setFormOpen(false)}
              >
                {t("common.cancel", { defaultValue: "Cancel" })}
              </Button>
              <Button
                variant="primary"
                size="sm"
                disabled={!form.name.trim() || formPending}
                onClick={submitForm}
              >
                {formPending && <Loader2 className="h-4 w-4 animate-spin" />}
                {t("common.save", { defaultValue: "Save" })}
              </Button>
            </div>
          </div>
        )}
      </Modal>

      {/* Quick-run dialog */}
      <Modal
        isOpen={runTarget !== null}
        onClose={() => setRunTarget(null)}
        title={`${t("agentTypes.quickRun")} — ${runTarget ?? ""}`}
        size="lg"
      >
        <div className="space-y-4">
          <div className="flex flex-col gap-1.5">
            <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
              {t("agentTypes.message")}
            </label>
            <textarea
              value={runMessage}
              rows={4}
              disabled={spawn.isPending}
              onChange={(e) => setRunMessage(e.target.value)}
              className={TEXTAREA_CLASS}
            />
          </div>

          {runResult && (
            <div className="space-y-1.5">
              <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                {t("agentTypes.result")}
              </label>
              <div className="max-h-[40vh] overflow-auto rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm">
                <MarkdownContent>{runResult.response}</MarkdownContent>
              </div>
              <div className="flex flex-wrap gap-3 text-[11px] text-text-dim">
                <span>
                  {t("agentTypes.iterations")}: {runResult.iterations}
                </span>
                <span>
                  {t("agentTypes.latency")}: {runResult.latency_ms} ms
                </span>
                {runResult.cost_usd !== null && (
                  <span>
                    {t("agentTypes.cost")}: ${runResult.cost_usd.toFixed(4)}
                  </span>
                )}
              </div>
            </div>
          )}

          <div className="flex justify-end gap-2 pt-2">
            <Button variant="ghost" size="sm" onClick={() => setRunTarget(null)}>
              {t("common.close", { defaultValue: "Close" })}
            </Button>
            <Button
              variant="primary"
              size="sm"
              disabled={!runMessage.trim() || spawn.isPending}
              onClick={submitRun}
            >
              {spawn.isPending ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Zap className="h-4 w-4" />
              )}
              {t("agentTypes.quickRun")}
            </Button>
          </div>
        </div>
      </Modal>

      <ConfirmDialog
        isOpen={deleteTarget !== null}
        title={t("agentTypes.delete")}
        message={t("agentTypes.confirmDelete")}
        confirmLabel={t("agentTypes.delete")}
        tone="destructive"
        onClose={() => setDeleteTarget(null)}
        onConfirm={() => {
          if (!deleteTarget) return;
          deleteType.mutate(deleteTarget, {
            onSuccess: () => setDeleteTarget(null),
            onError: (e) => toastErr(e, t("agentTypes.delete")),
          });
        }}
      />
    </div>
  );
}
