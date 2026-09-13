// Knowledge bases page (#8327).
//
// Surfaces:
//   - The bases, each with its document count, size and the agents that hold it
//   - Create and delete a base
//   - Upload and remove documents
//   - Choose which agents hold a base, and whether each may write
//
// A base is a named workspace under `{workspaces_dir}/knowledge/`, so what this
// page edits is each agent's manifest and a directory — there is no store of
// this feature's own. That is also why sharing is shown from both sides: the
// base card lists its holders here, and the agent's own workspace list shows
// the same grant from the other direction. #8321 was the same feature with only
// one of those two views, and it read as broken.
//
// All API access lives in `lib/queries/knowledge.ts` and
// `lib/mutations/knowledge.ts`. This file only renders.

import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { FileText, Library, Plus, Search, Trash2, Upload, Users } from "lucide-react";

import type { KnowledgeBase, KnowledgeDocument } from "../api";
import { useKnowledgeBases, useKnowledgeDocuments } from "../lib/queries/knowledge";
import { useAgents } from "../lib/queries/agents";
import {
  useCreateKnowledgeBase,
  useDeleteKnowledgeBase,
  useDeleteKnowledgeDocument,
  useSetKnowledgeHolders,
  useUploadKnowledgeDocument,
} from "../lib/mutations/knowledge";
import { isValidBaseName, isValidDocumentName } from "../lib/knowledgeNames";
import { useUIStore } from "../lib/store";

import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { Modal } from "../components/ui/Modal";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { EmptyState } from "../components/ui/EmptyState";
import { ErrorState } from "../components/ui/ErrorState";
import { CardSkeleton } from "../components/ui/Skeleton";

function errorMessage(err: unknown, fallback: string): string {
  return err instanceof Error && err.message ? err.message : fallback;
}

/**
 * Largest document the server will store, mirroring `MAX_DOCUMENT_BYTES` in
 * `routes/knowledge.rs`.
 *
 * Checked here so an oversized file is refused by name and size in one legible
 * sentence. Left to the server it arrives as a 413 whose body is not JSON, so
 * the toast degrades to the bare status text with no filename and no limit.
 */
export const MAX_DOCUMENT_BYTES = 4 * 1024 * 1024;

/** Bytes as something an operator reads at a glance rather than counts. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function KnowledgePage() {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);

  const [search, setSearch] = useState("");
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [confirmDelete, setConfirmDelete] = useState<KnowledgeBase | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [sharing, setSharing] = useState<KnowledgeBase | null>(null);

  const basesQuery = useKnowledgeBases();
  const createBase = useCreateKnowledgeBase();
  const deleteBase = useDeleteKnowledgeBase();

  const bases = useMemo(() => {
    const all = basesQuery.data ?? [];
    const needle = search.trim().toLowerCase();
    if (!needle) return all;
    return all.filter(
      (base) =>
        base.name.toLowerCase().includes(needle) ||
        base.agents.some((holder) => holder.agent_name.toLowerCase().includes(needle)),
    );
  }, [basesQuery.data, search]);

  // Only the safety rule. The server also refuses a name that reads as an
  // instruction, and that answer is a 400 rendered verbatim rather than a
  // second scanner here — see `lib/knowledgeNames.ts`.
  const nameIsValid = isValidBaseName(newName.trim());
  // An empty list means two different things, and "create your first base" is
  // actively wrong advice for the one where bases exist but none match.
  const searching = search.trim().length > 0;

  async function handleCreate() {
    const name = newName.trim();
    if (!nameIsValid) return;
    try {
      await createBase.mutateAsync(name);
      setCreating(false);
      setNewName("");
    } catch (err) {
      addToast(
        errorMessage(
          err,
          t("knowledge.create_failed", { defaultValue: "Could not create the knowledge base." }),
        ),
        "error",
      );
    }
  }

  async function handleDelete() {
    if (!confirmDelete) return;
    try {
      await deleteBase.mutateAsync(confirmDelete.name);
      setConfirmDelete(null);
    } catch (err) {
      addToast(
        errorMessage(
          err,
          t("knowledge.delete_failed", { defaultValue: "Could not delete the knowledge base." }),
        ),
        "error",
      );
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title={t("knowledge.title", { defaultValue: "Knowledge" })}
        subtitle={t("knowledge.description", {
          defaultValue:
            "Documents uploaded once and shared with the agents you choose. Each base becomes a named workspace the agent reads with @name.",
        })}
        icon={<Library className="h-5 w-5" />}
        actions={
          <Button variant="primary" onClick={() => setCreating(true)}>
            <Plus className="h-4 w-4" />
            {t("knowledge.new_base", { defaultValue: "New base" })}
          </Button>
        }
      />

      <div className="relative">
        <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-text-dim" />
        <Input
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t("knowledge.search", { defaultValue: "Search bases or agents…" })}
          className="pl-9"
        />
      </div>

      {basesQuery.isError ? (
        // Before the error branch existed, a failed request fell through to the
        // empty state and told the operator their bases did not exist.
        <ErrorState
          message={basesQuery.error?.message}
          onRetry={() => void basesQuery.refetch()}
        />
      ) : basesQuery.isLoading ? (
        <CardSkeleton />
      ) : bases.length === 0 ? (
        <EmptyState
          icon={<Library className="h-5 w-5" />}
          title={
            searching
              ? t("knowledge.no_matches_title", { defaultValue: "No matching bases" })
              : t("knowledge.empty_title", { defaultValue: "No knowledge bases yet" })
          }
          description={
            searching
              ? t("knowledge.no_matches_description", {
                  defaultValue: "No base or agent matches {{query}}.",
                  query: search.trim(),
                })
              : t("knowledge.empty_description", {
                  defaultValue:
                    "Create one, upload the documents your agents should be able to read, then pick which agents get it.",
                })
          }
        />
      ) : (
        <div className="space-y-3">
          {bases.map((base) => (
            <Card key={base.name} className="p-4">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <Library className="h-4 w-4 text-brand" />
                    <span className="font-bold">{base.name}</span>
                    <code className="text-xs text-text-dim">@{base.name}</code>
                  </div>
                  <p className="mt-1 text-xs text-text-dim">
                    {t("knowledge.base_summary", {
                      defaultValue: "{{count}} document(s) · {{size}}",
                      count: base.document_count,
                      size: formatBytes(base.total_bytes),
                    })}
                  </p>
                  <div className="mt-2 flex flex-wrap items-center gap-1">
                    <Users className="h-3 w-3 text-text-dim" />
                    {base.agents.length === 0 ? (
                      <span className="text-xs text-text-dim">
                        {t("knowledge.no_holders", { defaultValue: "Shared with nobody" })}
                      </span>
                    ) : (
                      base.agents.map((holder) => (
                        <Badge
                          key={`${holder.agent_id}:${holder.alias}`}
                          variant={holder.mode === "rw" ? "warning" : "default"}
                        >
                          {holder.agent_name}
                          {holder.mode === "rw"
                            ? ` · ${t("knowledge.mode_rw", { defaultValue: "read-write" })}`
                            : ""}
                        </Badge>
                      ))
                    )}
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  <Button variant="secondary" onClick={() => setSharing(base)}>
                    <Users className="h-4 w-4" />
                    {t("knowledge.share", { defaultValue: "Share" })}
                  </Button>
                  <Button
                    variant="secondary"
                    onClick={() => setExpanded(expanded === base.name ? null : base.name)}
                  >
                    <FileText className="h-4 w-4" />
                    {t("knowledge.documents", { defaultValue: "Documents" })}
                  </Button>
                  <Button
                    variant="danger"
                    aria-label={t("knowledge.delete_base", {
                      defaultValue: "Delete this knowledge base",
                    })}
                    onClick={() => setConfirmDelete(base)}
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
              </div>

              {expanded === base.name && <DocumentList baseName={base.name} />}
            </Card>
          ))}
        </div>
      )}

      <Modal
        isOpen={creating}
        onClose={() => setCreating(false)}
        title={t("knowledge.new_base", { defaultValue: "New base" })}
      >
        <div className="space-y-3">
          <Input
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            placeholder={t("knowledge.name_placeholder", { defaultValue: "handbook" })}
            autoFocus
          />
          <p className="text-xs text-text-dim">
            {t("knowledge.name_hint", {
              defaultValue:
                "Any language. Not a slash, a leading or trailing dot, or surrounding spaces. Agents will read it as @name.",
            })}
          </p>
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={() => setCreating(false)}>
              {t("common.cancel", { defaultValue: "Cancel" })}
            </Button>
            <Button
              variant="primary"
              disabled={!nameIsValid}
              isLoading={createBase.isPending}
              onClick={() => void handleCreate()}
            >
              {t("common.create", { defaultValue: "Create" })}
            </Button>
          </div>
        </div>
      </Modal>

      {sharing && <ShareModal base={sharing} onClose={() => setSharing(null)} />}

      <ConfirmDialog
        isOpen={confirmDelete !== null}
        onClose={() => setConfirmDelete(null)}
        onConfirm={() => void handleDelete()}
        title={t("knowledge.delete_title", { defaultValue: "Delete this knowledge base?" })}
        message={t("knowledge.delete_message", {
          defaultValue:
            "Its documents are removed and it is revoked from every agent holding it. This cannot be undone.",
        })}
        confirmLabel={t("common.delete", { defaultValue: "Delete" })}
        tone="destructive"
      />
    </div>
  );
}

/** The documents in one base, with upload and remove. */
function DocumentList({ baseName }: { baseName: string }) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const documentsQuery = useKnowledgeDocuments(baseName);
  const upload = useUploadKnowledgeDocument();
  const remove = useDeleteKnowledgeDocument();
  const [confirmRemove, setConfirmRemove] = useState<KnowledgeDocument | null>(null);

  async function handleFiles(files: FileList | null) {
    if (!files) return;
    for (const file of Array.from(files)) {
      if (!isValidDocumentName(file.name)) {
        addToast(t("knowledge.bad_filename", {
            defaultValue:
              "{{name}} cannot be used as a document name. Any language is fine, but not a slash, a leading or trailing dot, or surrounding spaces.",
            name: file.name,
          }), "error");
        continue;
      }
      if (file.size > MAX_DOCUMENT_BYTES) {
        addToast(t("knowledge.too_large", {
            defaultValue: "{{name}} is {{size}} — a document may be at most {{limit}}.",
            name: file.name,
            size: formatBytes(file.size),
            limit: formatBytes(MAX_DOCUMENT_BYTES),
          }), "error");
        continue;
      }
      try {
        await upload.mutateAsync({ name: baseName, filename: file.name, file });
      } catch (err) {
        addToast(errorMessage(
            err,
            t("knowledge.upload_failed", { defaultValue: "Could not upload the document." }),
          ), "error");
      }
    }
  }

  return (
    <div className="mt-4 border-t border-border-subtle/50 pt-3">
      <div className="mb-2 flex items-center justify-between">
        <span className="text-xs font-bold uppercase tracking-wide text-text-dim">
          {t("knowledge.documents", { defaultValue: "Documents" })}
        </span>
        <div className="flex items-center gap-2">
          <span className="text-[11px] text-text-dim">
            {t("knowledge.size_hint", {
              defaultValue: "Up to {{limit}} per document",
              limit: formatBytes(MAX_DOCUMENT_BYTES),
            })}
          </span>
          <label className="inline-flex cursor-pointer items-center gap-1 rounded-lg px-2 py-1 text-xs text-brand hover:bg-surface-hover">
            <Upload className="h-3.5 w-3.5" />
            {t("knowledge.upload", { defaultValue: "Upload" })}
            <input
              type="file"
              multiple
              className="hidden"
              onChange={(e) => {
                void handleFiles(e.target.files);
                // Clear it, or picking the same file twice in a row is a no-op
                // because `change` never fires for an unchanged value.
                e.target.value = "";
              }}
            />
          </label>
        </div>
      </div>

      {documentsQuery.isError ? (
        // Without this the panel claimed "No documents yet." directly under a
        // card header still reporting the base's document count and size.
        <ErrorState
          message={documentsQuery.error?.message}
          onRetry={() => void documentsQuery.refetch()}
        />
      ) : documentsQuery.isLoading ? (
        <p className="text-xs text-text-dim">{t("common.loading", { defaultValue: "Loading..." })}</p>
      ) : (documentsQuery.data ?? []).length === 0 ? (
        <p className="text-xs text-text-dim">
          {t("knowledge.no_documents", { defaultValue: "No documents yet." })}
        </p>
      ) : (
        <ul className="space-y-1">
          {(documentsQuery.data ?? []).map((doc) => (
            <li
              key={doc.filename}
              className="flex items-center justify-between rounded px-2 py-1 text-xs hover:bg-surface-hover"
            >
              <span className="flex min-w-0 items-center gap-2">
                <FileText className="h-3.5 w-3.5 shrink-0 text-text-dim" />
                <span className="truncate">{doc.filename}</span>
                <span className="shrink-0 text-text-dim">{formatBytes(doc.bytes)}</span>
              </span>
              <button
                type="button"
                aria-label={t("knowledge.remove_document", { defaultValue: "Remove this document" })}
                className="rounded p-1 text-text-dim hover:text-error"
                onClick={() => setConfirmRemove(doc)}
              >
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            </li>
          ))}
        </ul>
      )}

      <ConfirmDialog
        isOpen={confirmRemove !== null}
        onClose={() => setConfirmRemove(null)}
        // ConfirmDialog closes itself once this resolves, and keeps the dialog
        // open when it rejects so a failed removal can be retried.
        onConfirm={async () => {
          if (!confirmRemove) return;
          await remove.mutateAsync({ name: baseName, filename: confirmRemove.filename });
        }}
        title={t("knowledge.delete_document_title", { defaultValue: "Remove this document?" })}
        message={t("knowledge.delete_document_message", {
          defaultValue:
            "{{name}} is deleted from the base, so every agent holding it loses the document. This cannot be undone.",
          name: confirmRemove?.filename ?? "",
        })}
        confirmLabel={t("common.delete", { defaultValue: "Delete" })}
        tone="destructive"
      />
    </div>
  );
}

/** Pick which agents hold a base, and whether each may write to it. */
function ShareModal({ base, onClose }: { base: KnowledgeBase; onClose: () => void }) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  // Hands are included deliberately: `holders_of` walks the whole agent
  // registry, so a hand member holding this base shows as a badge on the card.
  // Excluding hands here would render that holder unrevocable — visible on the
  // card, absent from the only list that can untick it.
  const agentsQuery = useAgents({ includeHands: true });
  const setHolders = useSetKnowledgeHolders();

  // Seeded from the base's current holders, then edited locally so the whole
  // sharing decision lands as one request. Sending each toggle separately would
  // leave a half-applied grant behind if one of them failed.
  const [selection, setSelection] = useState<Record<string, "r" | "rw">>(() =>
    Object.fromEntries(base.agents.map((holder) => [holder.agent_id, holder.mode])),
  );

  const agents = agentsQuery.data ?? [];

  async function handleSave() {
    try {
      await setHolders.mutateAsync({
        name: base.name,
        agents: Object.entries(selection).map(([agent_id, mode]) => ({ agent_id, mode })),
      });
      onClose();
    } catch (err) {
      addToast(errorMessage(
          err,
          t("knowledge.share_failed", { defaultValue: "Could not update sharing." }),
        ), "error");
    }
  }

  return (
    <Modal
      isOpen
      onClose={onClose}
      title={t("knowledge.share_title", {
        defaultValue: "Who can read {{name}}?",
        name: base.name,
      })}
    >
      <div className="space-y-3">
        <p className="text-xs text-text-dim">
          {t("knowledge.share_hint", {
            defaultValue:
              "A selected agent reads the documents with @{{name}}. Read-write also lets it change them for everyone else.",
            name: base.name,
          })}
        </p>
        {agentsQuery.isError ? (
          // The query is cold until this modal opens, so without the three
          // states below the first paint was an empty list that read as
          // "there are no agents to share with".
          <ErrorState
            message={agentsQuery.error?.message}
            onRetry={() => void agentsQuery.refetch()}
          />
        ) : agentsQuery.isLoading ? (
          <p className="text-xs text-text-dim">
            {t("common.loading", { defaultValue: "Loading..." })}
          </p>
        ) : agents.length === 0 ? (
          <p className="text-xs text-text-dim">
            {t("knowledge.no_agents", { defaultValue: "No agents to share with yet." })}
          </p>
        ) : (
          <div className="max-h-80 space-y-1 overflow-y-auto scrollbar-thin">
            {agents.map((agent) => {
              const mode = selection[agent.id];
              return (
                <div
                  key={agent.id}
                  className="flex items-center justify-between rounded px-2 py-1.5 text-sm hover:bg-surface-hover"
                >
                  <label className="flex min-w-0 cursor-pointer items-center gap-2">
                    <input
                      type="checkbox"
                      className="rounded"
                      checked={mode !== undefined}
                      onChange={(e) => {
                        setSelection((current) => {
                          const next = { ...current };
                          if (e.target.checked) {
                            next[agent.id] = "r";
                          } else {
                            delete next[agent.id];
                          }
                          return next;
                        });
                      }}
                    />
                    <span className="truncate">{agent.name}</span>
                  </label>
                  <label
                    className={`flex shrink-0 items-center gap-1 text-xs ${
                      mode === undefined ? "opacity-40" : "cursor-pointer"
                    }`}
                  >
                    <input
                      type="checkbox"
                      className="rounded"
                      disabled={mode === undefined}
                      checked={mode === "rw"}
                      onChange={(e) => {
                        setSelection((current) => ({
                          ...current,
                          [agent.id]: e.target.checked ? "rw" : "r",
                        }));
                      }}
                    />
                    {t("knowledge.mode_rw", { defaultValue: "read-write" })}
                  </label>
                </div>
              );
            })}
          </div>
        )}
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose}>
            {t("common.cancel", { defaultValue: "Cancel" })}
          </Button>
          <Button variant="primary" isLoading={setHolders.isPending} onClick={() => void handleSave()}>
            {t("common.save", { defaultValue: "Save" })}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
