import { useState } from "react";
import { useTranslation } from "react-i18next";
import { AnimatePresence, motion } from "motion/react";
import { tabContent } from "../lib/motion";
import {
  X,
  Plus,
  Check,
  FlaskConical,
  GitBranch,
  Trash2,
  Play,
  Pause,
  BarChart3,
} from "lucide-react";
import {
  type PromptVersion,
  type PromptExperiment,
  type ExperimentVariantMetrics,
} from "../api";
import {
  usePromptVersions,
  useExperiments,
  useExperimentMetrics,
} from "../lib/queries/agents";
import {
  useCreatePromptVersion,
  useCreateExperiment,
  useActivatePromptVersion,
  useStartExperiment,
  usePauseExperiment,
  useCompleteExperiment,
  useDeletePromptVersion,
} from "../lib/mutations/agents";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { EmptyState } from "./ui/EmptyState";
import { CardSkeleton } from "./ui/Skeleton";
import { buildEvenTrafficSplit } from "./trafficSplit";

export function PromptsExperimentsModal({
  agentId,
  agentName,
  onClose,
}: {
  agentId: string;
  agentName: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<"versions" | "experiments">(
    "versions",
  );
  const [showCreateVersion, setShowCreateVersion] = useState(false);
  const [showCreateExperiment, setShowCreateExperiment] = useState(false);
  const [newPromptSystemPrompt, setNewPromptSystemPrompt] = useState("");
  const [newPromptDescription, setNewPromptDescription] = useState("");
  const [newExperimentName, setNewExperimentName] = useState("");
  const [selectedMetrics, setSelectedMetrics] = useState<string | null>(null);
  const [selectedVariantIds, setSelectedVariantIds] = useState<string[]>([]);

  const versionsQuery = usePromptVersions(agentId);
  const experimentsQuery = useExperiments(
    activeTab === "experiments" ? agentId : "",
  );
  const metricsQuery = useExperimentMetrics(selectedMetrics ?? "");

  const createVersionMutation = useCreatePromptVersion();
  const createExperimentMutation = useCreateExperiment();
  const activateMutation = useActivatePromptVersion();
  const startExpMutation = useStartExperiment();
  const pauseExpMutation = usePauseExperiment();
  const completeExpMutation = useCompleteExperiment();
  const deleteVersionMutation = useDeletePromptVersion();

  const versions = versionsQuery.data ?? [];
  const experiments = experimentsQuery.data ?? [];
  const metrics = metricsQuery.data ?? [];

  return (
    <div
      className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/40 backdrop-blur-xl"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="prompts-experiments-dialog-title"
        className="bg-surface rounded-t-2xl sm:rounded-2xl shadow-2xl border border-border-subtle w-full sm:w-[640px] sm:max-w-[90vw] max-h-[85vh] overflow-hidden flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="px-6 py-4 border-b border-border-subtle flex items-center justify-between shrink-0">
          <div>
            <h3
              id="prompts-experiments-dialog-title"
              className="text-lg font-black"
            >
              {agentName}
            </h3>
            <p className="text-xs text-text-dim">{t("agents.prompts_experiments.title")}</p>
          </div>
          <button
            onClick={onClose}
            className="p-2 rounded-xl hover:bg-main"
            aria-label={t("common.close", { defaultValue: "Close dialog" })}
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div
          role="tablist"
          aria-label={t("agents.prompts_experiments.title")}
          className="px-6 py-3 border-b border-border-subtle flex gap-2 shrink-0"
        >
          <button
            id="agents-tab-versions"
            role="tab"
            aria-selected={activeTab === "versions"}
            aria-controls="agents-panel-versions"
            tabIndex={activeTab === "versions" ? 0 : -1}
            onClick={() => setActiveTab("versions")}
            className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${activeTab === "versions" ? "bg-brand text-white" : "bg-main text-text-dim"}`}
          >
            <FlaskConical className="w-3 h-3 inline mr-1" /> {t("agents.prompts_experiments.versions_tab")}
          </button>
          <button
            id="agents-tab-experiments"
            role="tab"
            aria-selected={activeTab === "experiments"}
            aria-controls="agents-panel-experiments"
            tabIndex={activeTab === "experiments" ? 0 : -1}
            onClick={() => setActiveTab("experiments")}
            className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${activeTab === "experiments" ? "bg-brand text-white" : "bg-main text-text-dim"}`}
          >
            <GitBranch className="w-3 h-3 inline mr-1" /> {t("agents.prompts_experiments.experiments_tab")}
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-6">
          <AnimatePresence mode="wait">
            <motion.div
              key={activeTab}
              variants={tabContent}
              initial="initial"
              animate="animate"
              exit="exit"
            >
              {activeTab === "versions" && (
                <div
                  id="agents-panel-versions"
                  role="tabpanel"
                  aria-labelledby="agents-tab-versions"
                  className="space-y-4"
                >
                  <div className="flex justify-end">
                    <Button
                      variant="primary"
                      size="sm"
                      onClick={() => setShowCreateVersion(true)}
                    >
                      <Plus className="w-3 h-3 mr-1" /> {t("agents.prompts_experiments.new_version")}
                    </Button>
                  </div>

                  {versionsQuery.isLoading ? (
                    <CardSkeleton />
                  ) : versions.length === 0 ? (
                    <EmptyState
                      title={t("agents.prompts_experiments.no_versions")}
                      icon={<FlaskConical className="h-6 w-6" />}
                    />
                  ) : (
                    <div className="space-y-2">
                      {versions.map((v: PromptVersion) => (
                        <div
                          key={v.id}
                          className={`p-4 rounded-xl border ${v.is_active ? "border-success bg-success/5" : "border-border-subtle bg-main/30"}`}
                        >
                          <div className="flex items-center justify-between mb-2">
                            <div className="flex items-center gap-2">
                              <span className="font-bold text-sm">
                                v{v.version}
                              </span>
                              {v.is_active && (
                                      <Badge variant="success">{t("agents.prompts_experiments.active")}</Badge>
                              )}
                              {v.description && (
                                <span className="text-xs text-text-dim">
                                  - {v.description}
                                </span>
                              )}
                            </div>
                            <div className="flex gap-2">
                              {!v.is_active && (
                                <Button
                                  variant="secondary"
                                  size="sm"
                                  onClick={() =>
                                    activateMutation.mutate({
                                      versionId: v.id,
                                      agentId,
                                    })
                                  }
                                >
                                  <Check className="w-3 h-3 mr-1" /> {t("agents.prompts_experiments.activate")}
                                </Button>
                              )}
                              {!v.is_active && (
                                <Button
                                  variant="secondary"
                                  size="sm"
                                  onClick={() =>
                                    deleteVersionMutation.mutate({
                                      versionId: v.id,
                                      agentId,
                                    })
                                  }
                                >
                                  <Trash2 className="w-3 h-3" />
                                </Button>
                              )}
                            </div>
                          </div>
                          <pre className="text-xs text-text-dim whitespace-pre-wrap max-h-24 overflow-y-auto">
                            {v.system_prompt.slice(0, 200)}...
                          </pre>
                          <p className="text-[10px] text-text-dim mt-2">
                            {t("agents.prompts_experiments.created_label")} {new Date(v.created_at).toLocaleDateString()}
                          </p>
                        </div>
                      ))}
                    </div>
                  )}

                  {showCreateVersion && (
                    <div
                      className="fixed inset-0 z-60 flex items-end sm:items-center justify-center bg-black/50 p-0 sm:p-4"
                      onClick={() => setShowCreateVersion(false)}
                    >
                      <div
                        role="dialog"
                        aria-modal="true"
                        aria-labelledby="create-version-dialog-title"
                        className="bg-surface rounded-t-2xl sm:rounded-xl shadow-2xl border border-border-subtle p-6 w-full max-w-lg"
                        onClick={(e) => e.stopPropagation()}
                      >
                        <h4
                          id="create-version-dialog-title"
                          className="font-bold mb-4"
                        >
                          {t("agents.prompts_experiments.create_version_title")}
                        </h4>
                        <div className="space-y-4">
                          <div>
                            <label className="text-xs text-text-dim">
                              {t("agents.prompts_experiments.system_prompt")}
                            </label>
                            <textarea
                              value={newPromptSystemPrompt}
                              onChange={(e) =>
                                setNewPromptSystemPrompt(e.target.value)
                              }
                              rows={6}
                              className="w-full mt-1 rounded-xl border border-border-subtle bg-main px-3 py-2 text-xs font-mono"
                              placeholder={t("agents.prompts_experiments.system_prompt_placeholder")}
                            />
                          </div>
                          <div>
                            <label className="text-xs text-text-dim">
                              {t("agents.prompts_experiments.description_optional")}
                            </label>
                            <input
                              value={newPromptDescription}
                              onChange={(e) =>
                                setNewPromptDescription(e.target.value)
                              }
                              className="w-full mt-1 rounded-xl border border-border-subtle bg-main px-3 py-2 text-xs"
                              placeholder={t("agents.prompts_experiments.description_placeholder")}
                            />
                          </div>
                        </div>
                        <div className="flex gap-2 mt-4">
                          <Button
                            variant="primary"
                            className="flex-1"
                            isLoading={createVersionMutation.isPending}
                            onClick={() =>
                              createVersionMutation.mutate(
                                {
                                  agentId,
                                  version: {
                                    system_prompt: newPromptSystemPrompt,
                                    description: newPromptDescription,
                                    version:
                                      (versionsQuery.data?.length || 0) + 1,
                                    content_hash: "",
                                    tools: [],
                                    variables: [],
                                    created_by: "dashboard",
                                  },
                                },
                                {
                                  onSuccess: () => {
                                    setShowCreateVersion(false);
                                    setNewPromptSystemPrompt("");
                                    setNewPromptDescription("");
                                  },
                                },
                              )
                            }
                            disabled={
                              !newPromptSystemPrompt.trim() ||
                              createVersionMutation.isPending
                            }
                          >
                            {createVersionMutation.isPending
                              ? t("common.creating")
                              : t("common.create")}
                          </Button>
                          <Button
                            variant="secondary"
                            onClick={() => setShowCreateVersion(false)}
                          >
                            {t("common.cancel")}
                          </Button>
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              )}

              {activeTab === "experiments" && (
                <div
                  id="agents-panel-experiments"
                  role="tabpanel"
                  aria-labelledby="agents-tab-experiments"
                  className="space-y-4"
                >
                  <div className="flex justify-end">
                    <Button
                      variant="primary"
                      size="sm"
                      onClick={() => setShowCreateExperiment(true)}
                    >
                      <Plus className="w-3 h-3 mr-1" /> {t("agents.prompts_experiments.new_experiment")}
                    </Button>
                  </div>

                  {experimentsQuery.isLoading ? (
                    <CardSkeleton />
                  ) : experiments.length === 0 ? (
                    <EmptyState
                      title={t("agents.prompts_experiments.no_experiments")}
                      icon={<GitBranch className="h-6 w-6" />}
                    />
                  ) : (
                    <div className="space-y-2">
                      {experiments.map((exp: PromptExperiment) => (
                        <div
                          key={exp.id}
                          className="p-4 rounded-xl border border-border-subtle bg-main/30"
                        >
                          <div className="flex items-center justify-between mb-2">
                            <div className="flex items-center gap-2">
                              <span className="font-bold text-sm">
                                {exp.name}
                              </span>
                              <Badge
                                variant={
                                  exp.status === "running"
                                    ? "success"
                                    : exp.status === "completed"
                                      ? "default"
                                      : "warning"
                                }
                              >
                                {exp.status}
                              </Badge>
                            </div>
                            <div className="flex gap-2">
                              {exp.status === "draft" && (
                                <Button
                                  variant="secondary"
                                  size="sm"
                                  onClick={() =>
                                    startExpMutation.mutate({
                                      experimentId: exp.id,
                                      agentId,
                                    })
                                  }
                                >
                                  <Play className="w-3 h-3 mr-1" />
                                  {t("agents.prompts_experiments.start")}
                                </Button>
                              )}
                              {exp.status === "running" && (
                                <Button
                                  variant="secondary"
                                  size="sm"
                                  onClick={() =>
                                    pauseExpMutation.mutate({
                                      experimentId: exp.id,
                                      agentId,
                                    })
                                  }
                                >
                                  <Pause className="w-3 h-3 mr-1" />
                                  {t("agents.prompts_experiments.pause")}
                                </Button>
                              )}
                              {(exp.status === "running" ||
                                exp.status === "paused") && (
                                <Button
                                  variant="secondary"
                                  size="sm"
                                  onClick={() =>
                                    completeExpMutation.mutate({
                                      experimentId: exp.id,
                                      agentId,
                                    })
                                  }
                                >
                                  <Check className="w-3 h-3 mr-1" />
                                  {t("agents.prompts_experiments.complete")}
                                </Button>
                              )}
                              {(exp.status === "running" ||
                                exp.status === "paused") && (
                                <Button
                                  variant="secondary"
                                  size="sm"
                                  onClick={() => setSelectedMetrics(exp.id)}
                                >
                                  <BarChart3 className="w-3 h-3 mr-1" />
                                  {t("agents.prompts_experiments.metrics")}
                                </Button>
                              )}
                            </div>
                          </div>
                          <p className="text-xs text-text-dim">
                            {t("agents.prompts_experiments.variants_count", { count: exp.variants?.length || 0 })}
                          </p>
                        </div>
                      ))}
                    </div>
                  )}

                  {selectedMetrics && metricsQuery.data && (
                    <div className="mt-4 p-4 rounded-xl bg-main/50 border border-border-subtle">
                      <h5 className="text-xs font-bold mb-3">
                        {t("agents.prompts_experiments.experiment_metrics")}
                      </h5>
                      <div className="space-y-2">
                        {metrics.map((m: ExperimentVariantMetrics) => (
                          <div
                            key={m.variant_id}
                            className="p-3 rounded-lg bg-surface border border-border-subtle"
                          >
                            <div className="flex items-center justify-between mb-2">
                              <span className="font-bold text-xs">
                                {m.variant_name}
                              </span>
                              <Badge
                                variant={
                                  m.success_rate >= 80
                                    ? "success"
                                    : m.success_rate >= 50
                                      ? "warning"
                                      : "default"
                                }
                              >
                                {m.success_rate?.toFixed(1)}%
                              </Badge>
                            </div>
                            <div className="grid grid-cols-3 gap-2 text-[10px] text-text-dim">
                              <div>
                                <span className="block text-text-dim/60">
                                  {t("agents.prompts_experiments.requests")}
                                </span>
                                <span className="font-mono">
                                  {m.total_requests} ({m.successful_requests}{" "}
                                  {t("agents.prompts_experiments.ok_label")} / {m.failed_requests} {t("agents.prompts_experiments.err_label")})
                                </span>
                              </div>
                              <div>
                                <span className="block text-text-dim/60">
                                  {t("agents.prompts_experiments.avg_latency")}
                                </span>
                                <span className="font-mono">
                                  {m.avg_latency_ms?.toFixed(0)}ms
                                </span>
                              </div>
                              <div>
                                <span className="block text-text-dim/60">
                                  {t("agents.prompts_experiments.avg_cost")}
                                </span>
                                <span className="font-mono">
                                  ${m.avg_cost_usd?.toFixed(4)}
                                </span>
                              </div>
                            </div>
                          </div>
                        ))}
                      </div>
                      <Button
                        variant="secondary"
                        size="sm"
                        className="mt-3 w-full"
                        onClick={() => setSelectedMetrics(null)}
                      >
                        {t("agents.prompts_experiments.close_metrics")}
                      </Button>
                    </div>
                  )}

                  {showCreateExperiment && (
                    <div
                      className="fixed inset-0 z-60 flex items-end sm:items-center justify-center bg-black/50 p-0 sm:p-4"
                      onClick={() => setShowCreateExperiment(false)}
                    >
                      <div
                        role="dialog"
                        aria-modal="true"
                        aria-labelledby="create-experiment-dialog-title"
                        className="bg-surface rounded-t-2xl sm:rounded-xl shadow-2xl border border-border-subtle p-6 w-full max-w-lg"
                        onClick={(e) => e.stopPropagation()}
                      >
                        <h4
                          id="create-experiment-dialog-title"
                          className="font-bold mb-4"
                        >
                          {t("agents.prompts_experiments.create_experiment_title")}
                        </h4>
                        <div className="space-y-4">
                          <div>
                            <label className="text-xs text-text-dim">
                              {t("agents.prompts_experiments.experiment_name")}
                            </label>
                            <input
                              value={newExperimentName}
                              onChange={(e) =>
                                setNewExperimentName(e.target.value)
                              }
                              className="w-full mt-1 rounded-xl border border-border-subtle bg-main px-3 py-2 text-xs"
                              placeholder={t("agents.prompts_experiments.experiment_name_placeholder")}
                            />
                          </div>
                          <div>
                            <label className="text-xs text-text-dim mb-2 block">
                              {t("agents.prompts_experiments.select_versions")}
                            </label>
                            {versions.length < 2 ? (
                              <p className="text-xs text-warning">
                                {t("agents.prompts_experiments.create_versions_first")}
                              </p>
                            ) : (
                              <div className="space-y-1 max-h-40 overflow-y-auto">
                                {versions.map((v: PromptVersion) => (
                                  <label
                                    key={v.id}
                                    className={`flex items-center gap-2 p-2 rounded-lg cursor-pointer text-xs ${selectedVariantIds.includes(v.id) ? "bg-brand/10 border border-brand" : "bg-main/30 border border-border-subtle"}`}
                                  >
                                    <input
                                      type="checkbox"
                                      checked={selectedVariantIds.includes(
                                        v.id,
                                      )}
                                      onChange={(e) => {
                                        if (e.target.checked)
                                          setSelectedVariantIds([
                                            ...selectedVariantIds,
                                            v.id,
                                          ]);
                                        else
                                          setSelectedVariantIds(
                                            selectedVariantIds.filter(
                                              (id) => id !== v.id,
                                            ),
                                          );
                                      }}
                                      className="rounded"
                                    />
                                    <span className="font-bold">
                                      v{v.version}
                                    </span>
                                    {v.is_active && (
                                <Badge variant="success">{t("agents.prompts_experiments.active")}</Badge>
                                    )}
                                    <span className="text-text-dim truncate">
                                      {v.description ||
                                        v.system_prompt.slice(0, 40) + "..."}
                                    </span>
                                  </label>
                                ))}
                              </div>
                            )}
                          </div>
                        </div>
                        <div className="flex gap-2 mt-4">
                          <Button
                            variant="primary"
                            className="flex-1"
                            isLoading={createExperimentMutation.isPending}
                            onClick={() =>
                              createExperimentMutation.mutate(
                                {
                                  agentId,
                                  experiment: {
                                    name: newExperimentName,
                                    status: "draft",
                                    traffic_split: buildEvenTrafficSplit(
                                      selectedVariantIds.length,
                                    ),
                                    success_criteria: {
                                      require_user_helpful: true,
                                      require_no_tool_errors: true,
                                      require_non_empty: true,
                                    },
                                    variants: selectedVariantIds.map(
                                      (vId, i) => {
                                        const ver = versions.find(
                                          (v) => v.id === vId,
                                        );
                                        return {
                                          name:
                                            i === 0
                                              ? t("agents.prompts_experiments.control")
                                              : `Variant ${String.fromCharCode(65 + i)}`,
                                          prompt_version_id: vId,
                                          description: ver
                                            ? `v${ver.version}`
                                            : undefined,
                                        };
                                      },
                                    ),
                                  },
                                },
                                {
                                  onSuccess: () => {
                                    setShowCreateExperiment(false);
                                    setNewExperimentName("");
                                    setSelectedVariantIds([]);
                                  },
                                },
                              )
                            }
                            disabled={
                              !newExperimentName.trim() ||
                              selectedVariantIds.length < 2 ||
                              createExperimentMutation.isPending
                            }
                          >
                            {createExperimentMutation.isPending
                              ? t("common.creating")
                              : t("agents.prompts_experiments.create_with_variants", { count: selectedVariantIds.length })}
                          </Button>
                          <Button
                            variant="secondary"
                            onClick={() => {
                              setShowCreateExperiment(false);
                              setSelectedVariantIds([]);
                            }}
                          >
                            {t("common.cancel")}
                          </Button>
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              )}
            </motion.div>
          </AnimatePresence>
        </div>
      </div>
    </div>
  );
}
