import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  createWorkflow,
  deleteWorkflow,
  listWorkflowRuns,
  listWorkflows,
  runWorkflow,
} from "../api";
import { WorkflowEditor } from "../components/WorkflowEditor";
import { workflowTemplates, type WorkflowTemplate } from "../data/workflowTemplates";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Layers, RefreshCw, Trash2, FilePlus, Sparkles } from "lucide-react";

const REFRESH_MS = 30000;

export function WorkflowsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [isEditing, setIsEditing] = useState(false);
  const [showTemplates, setShowTemplates] = useState(false);
  const [selectedWorkflowId, setSelectedWorkflowId] = useState<string>("");
  const [runInput, setRunInput] = useState("");
  const [initialData, setInitialData] = useState<{ nodes: any[]; edges: any[] } | undefined>();

  const workflowsQuery = useQuery({ queryKey: ["workflows", "list"], queryFn: listWorkflows, refetchInterval: REFRESH_MS });
  const runsQuery = useQuery({ queryKey: ["workflows", "runs", selectedWorkflowId], queryFn: () => listWorkflowRuns(selectedWorkflowId), enabled: Boolean(selectedWorkflowId) });

  const runMutation = useMutation({ mutationFn: ({ workflowId, input }: any) => runWorkflow(workflowId, input) });
  const deleteMutation = useMutation({ mutationFn: deleteWorkflow });
  const createMutation = useMutation({ mutationFn: createWorkflow });

  const workflows = useMemo(() => [...(workflowsQuery.data ?? [])].sort((a, b) => (b.created_at ?? "").localeCompare(a.created_at ?? "")), [workflowsQuery.data]);

  const handleRun = async () => {
    if (!selectedWorkflowId) return;
    try {
      await runMutation.mutateAsync({ workflowId: selectedWorkflowId, input: runInput });
      await runsQuery.refetch();
    } catch { /* ignore */ }
  };

  const handleDelete = async (id: string) => {
    if (!window.confirm(t("common.confirm"))) return;
    try { await deleteMutation.mutateAsync(id); await queryClient.invalidateQueries({ queryKey: ["workflows"] }); } catch { /* ignore */ }
  };

  const handleUseTemplate = async (template: WorkflowTemplate) => {
    try {
      await createMutation.mutateAsync({
        name: t(template.name),
        description: t(template.description),
        steps: template.steps?.map((s: { name: string; prompt: string }) => ({ name: s.name, prompt: s.prompt })) ?? []
      });
      await queryClient.invalidateQueries({ queryKey: ["workflows"] });
      setShowTemplates(false);
    } catch { /* ignore */ }
  };

  const handleNewWorkflow = (template?: WorkflowTemplate) => {
    setInitialData(template ? { nodes: template.nodes, edges: template.edges } : undefined);
    setShowTemplates(false);
    setIsEditing(true);
  };

  if (isEditing) {
    return <WorkflowEditor initialNodes={initialData?.nodes} initialEdges={initialData?.edges} onClose={() => { setIsEditing(false); setInitialData(undefined); }} onSave={() => { setIsEditing(false); setInitialData(undefined); }} />;
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <Layers className="h-4 w-4" />
            {t("workflows.automation_hub")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">{t("workflows.title")}</h1>
          <p className="mt-1 text-text-dim font-medium">{t("workflows.subtitle")}</p>
        </div>
        <div className="flex gap-3">
          <div className="relative">
            <Button variant="primary" onClick={() => setShowTemplates(!showTemplates)}>
              <FilePlus className="h-4 w-4" />
              {t("common.symbols.expand")} {t("overview.new_workflow")}
            </Button>
            {showTemplates && (
              <div className="absolute top-full mt-2 left-0 w-64 rounded-xl border border-border-subtle bg-surface shadow-xl z-50 overflow-hidden">
                <div className="p-3 border-b border-border-subtle">
                  <button onClick={() => handleNewWorkflow()} className="w-full flex items-center gap-2 px-3 py-2 rounded-lg hover:bg-main transition-colors text-left">
                    <div className="h-6 w-6 rounded-md bg-brand/20 flex items-center justify-center text-brand">
                      <FilePlus className="h-3.5 w-3.5" />
                    </div>
                    <div>
                      <p className="text-xs font-bold">{t("workflows.create_blank")}</p>
                      <p className="text-[10px] text-text-dim">{t("workflows.use_template")}</p>
                    </div>
                  </button>
                </div>
                <div className="p-2 max-h-64 overflow-y-auto">
                  {workflowTemplates.map(template => (
                    <button key={template.id} onClick={() => handleNewWorkflow(template)} className="w-full flex items-center gap-3 px-3 py-2 rounded-lg hover:bg-main transition-colors text-left mb-1">
                      <div className="h-8 w-8 rounded-lg bg-gradient-to-br from-brand/20 to-brand/5 flex items-center justify-center text-lg">
                        {template.icon}
                      </div>
                      <div className="flex-1 min-w-0">
                        <p className="text-xs font-bold truncate">{t(template.name)}</p>
                        <p className="text-[10px] text-text-dim truncate">{t(template.description)}</p>
                      </div>
                    </button>
                  ))}
                </div>
                <div className="p-2 border-t border-border-subtle">
                  <button onClick={() => handleUseTemplate(workflowTemplates[0])} className="w-full flex items-center gap-2 px-3 py-2 rounded-lg hover:bg-brand/10 text-brand transition-colors text-left">
                    <Sparkles className="h-3.5 w-3.5" />
                    <span className="text-xs font-bold">{t("workflows.use_template")}</span>
                  </button>
                </div>
              </div>
            )}
          </div>
          <Button variant="secondary" onClick={() => void workflowsQuery.refetch()}>
            <RefreshCw className={`h-3.5 w-3.5 ${workflowsQuery.isFetching ? "animate-spin" : ""}`} />
            {t("common.refresh")}
          </Button>
        </div>
      </header>

      <div className="grid gap-6 lg:grid-cols-[1fr_320px]">
        <Card padding="lg">
          <h2 className="text-lg font-black tracking-tight mb-6">{t("workflows.all_workflows")}</h2>
          <div className="grid gap-3 sm:grid-cols-2">
            {workflows.map(wf => (
              <Card key={wf.id} hover padding="sm" className={`cursor-pointer ${selectedWorkflowId === wf.id ? 'border-brand' : ''}`} onClick={() => setSelectedWorkflowId(wf.id)}>
                <div className="flex justify-between items-start">
                  <div className="min-w-0 flex-1">
                    <h3 className="text-sm font-black truncate group-hover:text-brand transition-colors">{wf.name}</h3>
                    <p className="text-[10px] text-text-dim mt-1 line-clamp-1 italic">{wf.description || t("common.no_data")}</p>
                    <div className="mt-3 flex items-center gap-3 text-[9px] font-bold text-text-dim/60 uppercase">
                      <span>{t("workflows.steps_count", { count: wf.steps || 0 })}</span>
                      <div className="h-1 w-1 rounded-full bg-border-subtle" />
                      <span>{t("common.created")}: {new Date(wf.created_at || "").toLocaleDateString()}</span>
                    </div>
                  </div>
                  <button onClick={(e) => { e.stopPropagation(); handleDelete(wf.id); }} className="opacity-0 group-hover:opacity-100 p-1.5 rounded-lg text-text-dim hover:text-error transition-all">
                    <Trash2 className="h-4 w-4" />
                  </button>
                </div>
              </Card>
            ))}
          </div>
        </Card>

        <div className="flex flex-col gap-6">
          <Card padding="lg">
            <h3 className="text-xs font-black uppercase tracking-widest text-text-dim mb-4">{t("workflows.run_workflow")}</h3>
            <select value={selectedWorkflowId} onChange={(e) => setSelectedWorkflowId(e.target.value)} className="w-full rounded-xl border border-border-subtle bg-main px-4 py-2 text-sm mb-3 outline-none focus:border-brand">
              <option value="">{t("workflows.select_workflow")}</option>
              {workflows.map(wf => <option key={wf.id} value={wf.id}>{wf.name}</option>)}
            </select>
            <textarea value={runInput} onChange={e => setRunInput(e.target.value)} placeholder={t("chat.transmit_command")} rows={3} className="w-full rounded-xl border border-border-subtle bg-main px-4 py-2 text-sm mb-4 outline-none focus:border-brand resize-none" />
            <Button variant="primary" className="w-full bg-success border-success hover:bg-success/90" disabled={!selectedWorkflowId || runMutation.isPending} onClick={handleRun}>{t("scheduler.run_now")}</Button>
          </Card>

          <Card padding="lg">
            <h3 className="text-xs font-black uppercase tracking-widest text-text-dim mb-4">{t("workflows.recent_runs")}</h3>
            <p className="text-[10px] text-text-dim italic text-center py-8">{selectedWorkflowId ? t("workflows.no_runs") : t("workflows.select_workflow_hint")}</p>
          </Card>
        </div>
      </div>
    </div>
  );
}
