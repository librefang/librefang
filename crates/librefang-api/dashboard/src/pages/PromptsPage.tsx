import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { motion } from "motion/react";
import {
  FileText,
  Search,
  Plus,
  Check,
  Trash2,
  History,
  Link2,
  AlertTriangle,
} from "lucide-react";
import type { PromptOverviewItem, PromptVersion } from "../api";
import { usePromptsOverview } from "../lib/queries/prompts";
import { usePromptVersions } from "../lib/queries/agents";
import {
  useCreatePromptVersionForRepo,
  useDeletePromptVersionForRepo,
  useBindPromptVersionToAgent,
} from "../lib/mutations/prompts";
import { useUIStore } from "../lib/store";
import { toastErr } from "../lib/errors";
import { staggerContainer, staggerItem } from "../lib/motion";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { EmptyState } from "../components/ui/EmptyState";
import { CardSkeleton } from "../components/ui/Skeleton";
import { Modal } from "../components/ui/Modal";

const PREVIEW_LEN = 160;

export function PromptsPage() {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);

  const overviewQuery = usePromptsOverview();
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<PromptOverviewItem | null>(null);

  const items = overviewQuery.data ?? [];
  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return items;
    return items.filter((it) => it.agent_name.toLowerCase().includes(q));
  }, [items, search]);

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("common.infrastructure")}
        title={t("prompts.title")}
        subtitle={t("prompts.subtitle")}
        isFetching={overviewQuery.isFetching}
        onRefresh={() => void overviewQuery.refetch()}
        icon={<FileText className="h-4 w-4" />}
        helpText={t("prompts.help")}
      />

      <div className="max-w-sm">
        <Input
          leftIcon={<Search className="h-4 w-4" />}
          placeholder={t("prompts.search_placeholder")}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label={t("prompts.search_placeholder")}
        />
      </div>

      {overviewQuery.isLoading ? (
        <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
          {[1, 2, 3].map((i) => (
            <CardSkeleton key={i} />
          ))}
        </div>
      ) : overviewQuery.isError ? (
        <Card className="flex flex-col items-center gap-3 py-12 text-center">
          <AlertTriangle className="h-8 w-8 text-error" />
          <p className="text-sm font-bold">{t("prompts.error_title")}</p>
          <p className="text-xs text-text-dim">{t("prompts.error_body")}</p>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void overviewQuery.refetch()}
          >
            {t("common.retry", { defaultValue: "Retry" })}
          </Button>
        </Card>
      ) : items.length === 0 ? (
        <EmptyState
          title={t("prompts.empty_title")}
          description={t("prompts.empty_body")}
          icon={<FileText className="h-6 w-6" />}
        />
      ) : filtered.length === 0 ? (
        <EmptyState
          title={t("prompts.no_match_title")}
          icon={<Search className="h-6 w-6" />}
        />
      ) : (
        <motion.div
          variants={staggerContainer}
          initial="initial"
          animate="animate"
          className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4"
        >
          {filtered.map((item) => (
            <motion.div key={item.agent_id} variants={staggerItem}>
              <Card
                hover
                onClick={() => setSelected(item)}
                className="h-full flex flex-col gap-3"
              >
                <div className="flex items-start justify-between gap-2">
                  <h3 className="font-black text-sm truncate" title={item.agent_name}>
                    {item.agent_name}
                  </h3>
                  {item.active_version != null ? (
                    <Badge variant="success">
                      {t("prompts.active_badge", { version: item.active_version })}
                    </Badge>
                  ) : (
                    <Badge variant="default">{t("prompts.no_active_badge")}</Badge>
                  )}
                </div>
                <p className="text-xs text-text-dim line-clamp-3 min-h-[2.5rem] whitespace-pre-wrap">
                  {item.live_system_prompt.trim().length > 0
                    ? item.live_system_prompt.slice(0, PREVIEW_LEN)
                    : t("prompts.empty_live_prompt")}
                </p>
                <div className="mt-auto flex items-center gap-2 text-[11px] text-text-dim">
                  <History className="h-3 w-3" />
                  <span>
                    {t("prompts.version_count", { count: item.version_count })}
                  </span>
                </div>
              </Card>
            </motion.div>
          ))}
        </motion.div>
      )}

      {selected && (
        <AgentPromptRepoModal
          item={selected}
          onClose={() => setSelected(null)}
          addToast={addToast}
        />
      )}
    </div>
  );
}

function AgentPromptRepoModal({
  item,
  onClose,
  addToast,
}: {
  item: PromptOverviewItem;
  onClose: () => void;
  addToast: (message: string, type?: "success" | "error" | "info") => void;
}) {
  const { t } = useTranslation();
  const versionsQuery = usePromptVersions(item.agent_id);
  const createMutation = useCreatePromptVersionForRepo();
  const deleteMutation = useDeletePromptVersionForRepo();
  const bindMutation = useBindPromptVersionToAgent();

  const [showCreate, setShowCreate] = useState(false);
  const [draftPrompt, setDraftPrompt] = useState("");
  const [draftDescription, setDraftDescription] = useState("");
  const [bindAfterCreate, setBindAfterCreate] = useState(true);

  const versions = versionsQuery.data ?? [];

  const handleBind = (version: PromptVersion) => {
    bindMutation.mutate(
      { agentId: item.agent_id, version },
      {
        onSuccess: () =>
          addToast(
            t("prompts.bind_success", { version: version.version }),
            "success",
          ),
        onError: (err) => addToast(toastErr(err, t("prompts.bind_error")), "error"),
      },
    );
  };

  const handleDelete = (version: PromptVersion) => {
    deleteMutation.mutate(
      { versionId: version.id, agentId: item.agent_id },
      {
        onSuccess: () => addToast(t("prompts.delete_success"), "success"),
        onError: (err) =>
          addToast(toastErr(err, t("prompts.delete_error")), "error"),
      },
    );
  };

  const handleCreate = () => {
    createMutation.mutate(
      {
        agentId: item.agent_id,
        version: {
          system_prompt: draftPrompt,
          description: draftDescription || undefined,
          version: 0, // server overwrites with the monotonic next number.
          content_hash: "",
          tools: [],
          variables: [],
          created_by: "dashboard",
        },
      },
      {
        onSuccess: (created) => {
          setShowCreate(false);
          setDraftPrompt("");
          setDraftDescription("");
          if (bindAfterCreate) {
            handleBind(created);
          } else {
            addToast(t("prompts.create_success"), "success");
          }
        },
        onError: (err) =>
          addToast(toastErr(err, t("prompts.create_error")), "error"),
      },
    );
  };

  return (
    <Modal
      isOpen
      onClose={onClose}
      title={item.agent_name}
      size="2xl"
      variant="panel-right"
    >
      <div className="flex flex-col gap-4">
        <div className="flex items-center justify-between gap-2">
          <p className="text-xs text-text-dim">{t("prompts.versions_subtitle")}</p>
          <Button
            variant="primary"
            size="sm"
            onClick={() => setShowCreate((v) => !v)}
          >
            <Plus className="h-3 w-3 mr-1" /> {t("prompts.new_version")}
          </Button>
        </div>

        {showCreate && (
          <Card padding="md" className="flex flex-col gap-3">
            <h4 className="text-sm font-bold">{t("prompts.create_version_title")}</h4>
            <div>
              <label
                htmlFor="prompt-create-body"
                className="text-xs text-text-dim"
              >
                {t("prompts.system_prompt_label")}
              </label>
              <textarea
                id="prompt-create-body"
                value={draftPrompt}
                onChange={(e) => setDraftPrompt(e.target.value)}
                rows={8}
                className="w-full mt-1 rounded-xl border border-border-subtle bg-main px-3 py-2 text-xs font-mono"
                placeholder={t("prompts.system_prompt_placeholder")}
              />
            </div>
            <Input
              label={t("prompts.description_label")}
              value={draftDescription}
              onChange={(e) => setDraftDescription(e.target.value)}
              placeholder={t("prompts.description_placeholder")}
            />
            <label className="flex items-center gap-2 text-xs text-text-dim">
              <input
                type="checkbox"
                checked={bindAfterCreate}
                onChange={(e) => setBindAfterCreate(e.target.checked)}
                className="rounded"
              />
              {t("prompts.bind_after_create")}
            </label>
            <div className="flex gap-2">
              <Button
                variant="primary"
                size="sm"
                className="flex-1"
                isLoading={createMutation.isPending || bindMutation.isPending}
                disabled={
                  !draftPrompt.trim() ||
                  createMutation.isPending ||
                  bindMutation.isPending
                }
                onClick={handleCreate}
              >
                {t("common.create")}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => setShowCreate(false)}
              >
                {t("common.cancel")}
              </Button>
            </div>
          </Card>
        )}

        {versionsQuery.isLoading ? (
          <CardSkeleton />
        ) : versions.length === 0 ? (
          <EmptyState
            title={t("prompts.no_versions")}
            description={t("prompts.no_versions_body")}
            icon={<History className="h-6 w-6" />}
          />
        ) : (
          <div className="flex flex-col gap-2">
            {versions.map((v) => (
              <div
                key={v.id}
                className={`p-4 rounded-xl border ${
                  v.is_active
                    ? "border-success bg-success/5"
                    : "border-border-subtle bg-main/30"
                }`}
              >
                <div className="flex items-center justify-between gap-2 mb-2">
                  <div className="flex items-center gap-2 min-w-0">
                    <span className="font-bold text-sm shrink-0">
                      {t("prompts.version_label", { version: v.version })}
                    </span>
                    {v.is_active && (
                      <Badge variant="success">{t("prompts.active")}</Badge>
                    )}
                    {v.description && (
                      <span className="text-xs text-text-dim truncate">
                        {v.description}
                      </span>
                    )}
                  </div>
                  <div className="flex gap-2 shrink-0">
                    {!v.is_active && (
                      <Button
                        variant="secondary"
                        size="sm"
                        isLoading={bindMutation.isPending}
                        disabled={bindMutation.isPending}
                        onClick={() => handleBind(v)}
                      >
                        <Link2 className="h-3 w-3 mr-1" /> {t("prompts.bind")}
                      </Button>
                    )}
                    {v.is_active && (
                      <span className="flex items-center text-xs text-success">
                        <Check className="h-3 w-3 mr-1" /> {t("prompts.bound")}
                      </span>
                    )}
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={v.is_active || deleteMutation.isPending}
                      onClick={() => handleDelete(v)}
                      aria-label={t("prompts.delete")}
                      title={
                        v.is_active
                          ? t("prompts.delete_blocked_active", {
                              defaultValue:
                                "Active version — activate another version before deleting",
                            })
                          : t("prompts.delete")
                      }
                    >
                      <Trash2 className="h-3 w-3" />
                    </Button>
                  </div>
                </div>
                <pre className="text-xs text-text-dim whitespace-pre-wrap max-h-32 overflow-y-auto font-mono">
                  {v.system_prompt}
                </pre>
                <p className="text-[10px] text-text-dim mt-2">
                  {t("prompts.created_label")}{" "}
                  {new Date(v.created_at).toLocaleString()}
                </p>
              </div>
            ))}
          </div>
        )}
      </div>
    </Modal>
  );
}
