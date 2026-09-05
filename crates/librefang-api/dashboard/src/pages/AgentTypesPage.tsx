import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import { Edit2, ExternalLink, History, LayoutTemplate, Lock, Play, Plus, RotateCcw, Share2, ShieldCheck, Trash2 } from "lucide-react";
import type { AgentTemplate, AgentTypeSpec, SpawnEphemeralResult } from "../api";
import { useAgentType, useAgentTypes, useAgentTypeHistory } from "../lib/queries/agentTypes";
import { useAgents, useTools } from "../lib/queries/agents";
import { useSkills } from "../lib/queries/skills";
import { useProviders } from "../lib/queries/providers";
import { useModels } from "../lib/queries/models";
import { useMcpServers } from "../lib/queries/mcp";
import {
  useCreateAgentType,
  useDeleteAgentType,
  usePromoteAgentType,
  useRestoreTemplateVersion,
  useSpawnEphemeral,
  useUpdateAgentTypeToml,
} from "../lib/mutations/agentTypes";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { ErrorState } from "../components/ui/ErrorState";
import { EmptyState } from "../components/ui/EmptyState";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Modal } from "../components/ui/Modal";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { AgentManifestForm } from "../components/AgentManifestForm";
import type { ManifestCatalogEntry } from "../components/AgentManifestForm";
import {
  emptyManifestExtras,
  emptyManifestForm,
  parseManifestToml,
  serializeManifestForm,
  validateManifestForm,
  type ManifestExtras,
  type ManifestFormState,
} from "../lib/agentManifest";
import { useUIStore } from "../lib/store";
import { toastErr } from "../lib/errors";
import { copyToClipboard } from "../lib/clipboard";

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

function AgentTypeEditor({
  name,
  onClose,
}: {
  name: string | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const isCreate = name === null;

  const detail = useAgentType(name ?? "", { enabled: !isCreate });
  const createMutation = useCreateAgentType();
  const updateTomlMutation = useUpdateAgentTypeToml();

  const providersQuery = useProviders();
  const modelsQuery = useModels();
  const toolsQuery = useTools();
  const skillsQuery = useSkills();
  const mcpServersQuery = useMcpServers();

  const [newName, setNewName] = useState("");
  const [formState, setFormState] = useState<ManifestFormState>(emptyManifestForm);
  const [formExtras, setFormExtras] = useState<ManifestExtras>(emptyManifestExtras);
  const [invalidFields, setInvalidFields] = useState<Set<string>>(new Set());
  const [parseError, setParseError] = useState<string | null>(null);
  const [seeded, setSeeded] = useState(false);

  useEffect(() => {
    if (isCreate || seeded) return;
    const toml = detail.data?.manifest_toml;
    if (!toml) return;
    const parsed = parseManifestToml(toml);
    if (parsed.ok) {
      setFormState(parsed.form);
      setFormExtras(parsed.extras);
      setParseError(null);
    } else {
      setParseError(
        parsed.message === "json_schema_unsafe_integer"
          ? t("agents.form.json_schema_unsafe_integer")
          : parsed.message,
      );
    }
    setSeeded(true);
  }, [isCreate, seeded, detail.data, t]);

  const providers = useMemo(
    () => (providersQuery.data ?? []).map((p) => ({ name: p.id })),
    [providersQuery.data],
  );

  const models = useMemo(
    () =>
      (modelsQuery.data?.models ?? []).map((m) => ({
        provider: m.provider ?? "",
        id: m.id,
      })),
    [modelsQuery.data],
  );

  const skillCatalog = useMemo<ManifestCatalogEntry[]>(
    () => (skillsQuery.data ?? []).map((s) => ({ name: s.name, description: s.description })),
    [skillsQuery.data],
  );

  const toolCatalog = useMemo<ManifestCatalogEntry[]>(
    () => (toolsQuery.data ?? []).map((t) => ({ name: t.name, description: t.description })),
    [toolsQuery.data],
  );

  const mcpCatalog = useMemo<ManifestCatalogEntry[]>(
    () =>
      mcpServersQuery.data
        ? mcpServersQuery.data.configured.map((s: { name: string }) => ({ name: s.name }))
        : [],
    [mcpServersQuery.data],
  );

  const saving = createMutation.isPending || updateTomlMutation.isPending;

  async function handleSave() {
    const errors = validateManifestForm(formState);
    setInvalidFields(new Set(errors));
    if (errors.length > 0) return;

    try {
      if (isCreate) {
        const trimmed = newName.trim();
        await createMutation.mutateAsync({
          name: trimmed,
          description: formState.description,
        } as AgentTypeSpec);
        const toml = serializeManifestForm(formState, formExtras);
        await updateTomlMutation.mutateAsync({ name: trimmed, toml });
        addToast(t("agentTypes.created"), "success");
      } else {
        const toml = serializeManifestForm(formState, formExtras);
        await updateTomlMutation.mutateAsync({ name: name as string, toml });
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
      size="xl"
      overflowVisible
      title={isCreate ? t("agentTypes.create_title") : t("agentTypes.edit_title", { name })}
    >
      {!isCreate && detail.isLoading ? (
        <ListSkeleton rows={4} />
      ) : !isCreate && detail.isError ? (
        <ErrorState message={detail.error?.message} onRetry={() => void detail.refetch()} />
      ) : parseError ? (
        <div className="space-y-3">
          <ErrorState message={parseError} />
          <div className="flex justify-end">
            <Button variant="ghost" onClick={onClose}>
              {t("common.close")}
            </Button>
          </div>
        </div>
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

          <AgentManifestForm
            value={formState}
            onChange={setFormState}
            providers={providers}
            models={models}
            invalidFields={invalidFields}
            extras={formExtras}
            skillCatalog={skillCatalog}
            toolCatalog={toolCatalog}
            mcpCatalog={mcpCatalog}
          />

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
  onPreviewPromote,
  onHistory,
}: {
  type: AgentTemplate;
  onQuickRun: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onPromote: () => void;
  onPreviewPromote: () => void;
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
          onClick={onPreviewPromote}
          className="rounded-lg p-1.5 text-text-dim hover:bg-main/50 hover:text-brand"
          aria-label={t("agentTypes.promote")}
          title={t("agentTypes.promote")}
        >
          <ShieldCheck className="h-3.5 w-3.5" />
        </button>

        {type.editable ? (
          <>
            <button
              type="button"
              onClick={onPromote}
              className="rounded-lg p-1.5 text-text-dim hover:bg-main/50 hover:text-brand"
              aria-label={t("agentTypes.promote")}
              title={t("agentTypes.promote")}
            >
              <Share2 className="h-3.5 w-3.5" />
            </button>
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
  const promoteMutation = usePromoteAgentType();

  const [editing, setEditing] = useState<{ name: string | null } | null>(null);
  const [quickRun, setQuickRun] = useState<AgentTemplate | null>(null);
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const [promoting, setPromoting] = useState<string | null>(null);
  const [pendingPromote, setPendingPromote] = useState<string | null>(null);
  const [promotedPrUrl, setPromotedPrUrl] = useState<string | null>(null);
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

  async function confirmPromote() {
    if (!pendingPromote) return;
    try {
      const result = await promoteMutation.mutateAsync(pendingPromote);
      setPromotedPrUrl(result.pr_url);
      addToast(t("agentTypes.promoted"), "success");
    } catch (err) {
      addToast(toastErr(err, t("agentTypes.promote_failed")), "error");
    } finally {
      setPendingPromote(null);
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
              onPromote={() => setPendingPromote(type.name)}
              onPreviewPromote={() => setPromoting(type.name)}
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

      <ConfirmDialog
        isOpen={pendingPromote !== null}
        onClose={() => setPendingPromote(null)}
        onConfirm={() => void confirmPromote()}
        title={t("agentTypes.promote")}
        message={t("agentTypes.confirm_promote", { name: pendingPromote ?? "" })}
      />

      {/* Success dialog after promotion — shows the PR link */}
      <Modal
        isOpen={promotedPrUrl !== null}
        onClose={() => setPromotedPrUrl(null)}
        title={t("agentTypes.promoted")}
        size="sm"
      >
        <div className="space-y-3">
          <p className="text-[13px] text-text-main">{t("agentTypes.promote_success")}</p>
          <a
            href={promotedPrUrl ?? "#"}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1.5 rounded-lg border border-brand/30 bg-brand/10 px-3 py-1.5 text-[13px] font-medium text-brand hover:bg-brand/20"
          >
            <ExternalLink className="h-3.5 w-3.5" />
            {t("agentTypes.view_pr")}
          </a>
          <div className="flex justify-end pt-1">
            <Button variant="ghost" onClick={() => setPromotedPrUrl(null)}>
              {t("common.close")}
            </Button>
          </div>
        </div>
      </Modal>
    </div>
  );
}
