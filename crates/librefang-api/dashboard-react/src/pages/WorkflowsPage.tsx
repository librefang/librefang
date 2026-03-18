import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FormEvent, useMemo, useState } from "react";
import {
  createWorkflow,
  deleteWorkflow,
  listAgents,
  listWorkflowRuns,
  listWorkflows,
  runWorkflow,
  type ApiActionResponse,
  type WorkflowItem,
  type WorkflowRunItem
} from "../api";

const REFRESH_MS = 30000;

interface ActionFeedback {
  type: "ok" | "error";
  text: string;
}

function actionText(action: ApiActionResponse): string {
  if (typeof action.message === "string" && action.message.trim().length > 0) return action.message;
  if (typeof action.status === "string" && action.status.trim().length > 0) return action.status;
  if (typeof action.output === "string" && action.output.trim().length > 0) return action.output;
  return JSON.stringify(action);
}

function toDateText(value?: string | null): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function workflowSort(a: WorkflowItem, b: WorkflowItem): number {
  return (b.created_at ?? "").localeCompare(a.created_at ?? "");
}

export function WorkflowsPage() {
  const queryClient = useQueryClient();
  const [feedback, setFeedback] = useState<ActionFeedback | null>(null);
  const [workflowName, setWorkflowName] = useState("");
  const [workflowDescription, setWorkflowDescription] = useState("");
  const [workflowPrompt, setWorkflowPrompt] = useState("{{input}}");
  const [workflowAgent, setWorkflowAgent] = useState("");
  const [selectedWorkflowId, setSelectedWorkflowId] = useState<string>("");
  const [runInput, setRunInput] = useState("");
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);

  const workflowsQuery = useQuery({
    queryKey: ["workflows", "list"],
    queryFn: listWorkflows,
    refetchInterval: REFRESH_MS
  });

  const agentsQuery = useQuery({
    queryKey: ["agents", "list", "workflows-helper"],
    queryFn: listAgents,
    refetchInterval: REFRESH_MS
  });

  const runsQuery = useQuery({
    queryKey: ["workflows", "runs", selectedWorkflowId],
    queryFn: () => listWorkflowRuns(selectedWorkflowId),
    enabled: selectedWorkflowId.length > 0
  });

  const createMutation = useMutation({
    mutationFn: createWorkflow
  });
  const runMutation = useMutation({
    mutationFn: ({ workflowId, input }: { workflowId: string; input: string }) =>
      runWorkflow(workflowId, input)
  });
  const deleteMutation = useMutation({
    mutationFn: deleteWorkflow
  });

  const workflows = useMemo(
    () => [...(workflowsQuery.data ?? [])].sort(workflowSort),
    [workflowsQuery.data]
  );
  const agents = agentsQuery.data ?? [];
  const runs = runsQuery.data ?? [];

  const workflowsError = workflowsQuery.error instanceof Error ? workflowsQuery.error.message : "";
  const runsError = runsQuery.error instanceof Error ? runsQuery.error.message : "";

  async function refreshWorkflows() {
    await queryClient.invalidateQueries({ queryKey: ["workflows"] });
    await workflowsQuery.refetch();
    if (selectedWorkflowId) {
      await runsQuery.refetch();
    }
  }

  async function handleCreateWorkflow(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const name = workflowName.trim();
    const prompt = workflowPrompt.trim();
    const agent = workflowAgent.trim();
    if (!name || !prompt || !agent || createMutation.isPending) return;

    try {
      const result = await createMutation.mutateAsync({
        name,
        description: workflowDescription.trim(),
        steps: [
          {
            name: "step-1",
            agent_name: agent,
            prompt
          }
        ]
      });
      setFeedback({ type: "ok", text: actionText(result) });
      setWorkflowName("");
      setWorkflowDescription("");
      setWorkflowPrompt("{{input}}");
      await refreshWorkflows();
    } catch (error) {
      setFeedback({
        type: "error",
        text: error instanceof Error ? error.message : "Workflow creation failed."
      });
    }
  }

  async function handleRunWorkflow(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selectedWorkflowId || runMutation.isPending) return;
    try {
      const result = await runMutation.mutateAsync({
        workflowId: selectedWorkflowId,
        input: runInput
      });
      setFeedback({ type: "ok", text: actionText(result) });
      setRunInput("");
      await runsQuery.refetch();
    } catch (error) {
      setFeedback({
        type: "error",
        text: error instanceof Error ? error.message : "Workflow run failed."
      });
    }
  }

  async function handleDeleteWorkflow(workflowId: string) {
    if (deleteMutation.isPending) return;
    const item = workflows.find((w) => w.id === workflowId);
    if (!window.confirm(`Delete workflow "${item?.name ?? workflowId}"?`)) return;

    setPendingDeleteId(workflowId);
    try {
      const result = await deleteMutation.mutateAsync(workflowId);
      setFeedback({ type: "ok", text: actionText(result) });
      if (selectedWorkflowId === workflowId) {
        setSelectedWorkflowId("");
      }
      await refreshWorkflows();
    } catch (error) {
      setFeedback({
        type: "error",
        text: error instanceof Error ? error.message : "Workflow deletion failed."
      });
    } finally {
      setPendingDeleteId(null);
    }
  }

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Workflows</h1>
          <p className="text-sm text-slate-400">Multi-step agent pipelines and manual workflow runs.</p>
        </div>
        <div className="flex items-center gap-2">
          <span className="rounded-full border border-slate-700 bg-slate-800/60 px-2 py-1 text-xs text-slate-300">
            {workflows.length} workflows
          </span>
          <button
            className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
            type="button"
            onClick={() => void refreshWorkflows()}
            disabled={workflowsQuery.isFetching}
          >
            Refresh
          </button>
        </div>
      </header>

      {feedback ? (
        <div
          className={`rounded-xl border p-3 text-sm ${
            feedback.type === "ok"
              ? "border-emerald-700 bg-emerald-700/10 text-emerald-200"
              : "border-rose-700 bg-rose-700/10 text-rose-200"
          }`}
        >
          {feedback.text}
        </div>
      ) : null}

      {workflowsError ? (
        <div className="rounded-xl border border-rose-700 bg-rose-700/15 p-4 text-rose-200">{workflowsError}</div>
      ) : null}

      <div className="grid gap-3 xl:grid-cols-[340px_1fr]">
        <aside className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="mb-3 mt-0 text-base font-semibold">Create Workflow</h2>
          <form className="flex flex-col gap-2" onSubmit={handleCreateWorkflow}>
            <input
              type="text"
              value={workflowName}
              onChange={(event) => setWorkflowName(event.target.value)}
              placeholder="Workflow name"
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={createMutation.isPending}
            />
            <input
              type="text"
              value={workflowDescription}
              onChange={(event) => setWorkflowDescription(event.target.value)}
              placeholder="Description (optional)"
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={createMutation.isPending}
            />
            <select
              value={workflowAgent}
              onChange={(event) => setWorkflowAgent(event.target.value)}
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={createMutation.isPending}
            >
              <option value="">Select agent</option>
              {agents.map((agent) => (
                <option key={agent.id} value={agent.name}>
                  {agent.name}
                </option>
              ))}
            </select>
            <textarea
              value={workflowPrompt}
              onChange={(event) => setWorkflowPrompt(event.target.value)}
              placeholder="Prompt template"
              rows={4}
              className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={createMutation.isPending}
            />
            <button
              className="mt-1 rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
              type="submit"
              disabled={
                createMutation.isPending ||
                workflowName.trim().length === 0 ||
                workflowPrompt.trim().length === 0 ||
                workflowAgent.trim().length === 0
              }
            >
              {createMutation.isPending ? "Creating..." : "Create"}
            </button>
          </form>
        </aside>

        <section className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <div className="grid gap-3 lg:grid-cols-[1fr_340px]">
            <div className="flex flex-col gap-2">
              <h2 className="m-0 text-base font-semibold">Workflow List</h2>
              {workflowsQuery.isLoading && workflows.length === 0 ? (
                <p className="text-sm text-slate-400">Loading workflows...</p>
              ) : null}
              {!workflowsQuery.isLoading && workflows.length === 0 ? (
                <p className="text-sm text-slate-400">No workflows found.</p>
              ) : null}

              <div className="flex max-h-[65vh] flex-col gap-2 overflow-y-auto pr-1">
                {workflows.map((workflow) => (
                  <article
                    key={workflow.id}
                    className={`rounded-lg border p-3 ${
                      workflow.id === selectedWorkflowId
                        ? "border-sky-500 bg-sky-500/15"
                        : "border-slate-700 bg-slate-950/70"
                    }`}
                  >
                    <button
                      type="button"
                      onClick={() => setSelectedWorkflowId(workflow.id)}
                      className="w-full text-left"
                    >
                      <h3 className="m-0 text-sm font-semibold">{workflow.name ?? workflow.id}</h3>
                      <p className="mt-1 text-xs text-slate-400">{workflow.description || "No description."}</p>
                      <p className="mt-1 text-xs text-slate-500">
                        steps: {workflow.steps ?? 0} · created: {toDateText(workflow.created_at)}
                      </p>
                    </button>
                    <div className="mt-2 flex justify-end">
                      <button
                        className="rounded-lg border border-rose-700 bg-rose-700/20 px-2 py-1 text-[11px] font-medium text-rose-200 transition hover:bg-rose-700/30 disabled:cursor-not-allowed disabled:opacity-60"
                        type="button"
                        onClick={() => void handleDeleteWorkflow(workflow.id)}
                        disabled={pendingDeleteId === workflow.id}
                      >
                        {pendingDeleteId === workflow.id ? "Deleting..." : "Delete"}
                      </button>
                    </div>
                  </article>
                ))}
              </div>
            </div>

            <div className="flex flex-col gap-3 rounded-lg border border-slate-700 bg-slate-950/60 p-3">
              <h2 className="m-0 text-base font-semibold">Run Workflow</h2>
              <form className="flex flex-col gap-2" onSubmit={handleRunWorkflow}>
                <select
                  value={selectedWorkflowId}
                  onChange={(event) => setSelectedWorkflowId(event.target.value)}
                  className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
                  disabled={runMutation.isPending}
                >
                  <option value="">Select workflow</option>
                  {workflows.map((workflow) => (
                    <option key={workflow.id} value={workflow.id}>
                      {workflow.name ?? workflow.id}
                    </option>
                  ))}
                </select>
                <textarea
                  value={runInput}
                  onChange={(event) => setRunInput(event.target.value)}
                  placeholder="Run input (optional)"
                  rows={4}
                  className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
                  disabled={runMutation.isPending}
                />
                <button
                  className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
                  type="submit"
                  disabled={runMutation.isPending || selectedWorkflowId.length === 0}
                >
                  {runMutation.isPending ? "Running..." : "Run"}
                </button>
              </form>

              <div className="mt-1">
                <h3 className="mb-2 mt-0 text-sm font-semibold">Recent Runs</h3>
                {runsError ? <p className="text-xs text-rose-300">{runsError}</p> : null}
                {runsQuery.isLoading && selectedWorkflowId ? (
                  <p className="text-xs text-slate-400">Loading runs...</p>
                ) : null}
                {!selectedWorkflowId ? (
                  <p className="text-xs text-slate-400">Select a workflow to view runs.</p>
                ) : null}
                {selectedWorkflowId && runs.length === 0 && !runsQuery.isLoading ? (
                  <p className="text-xs text-slate-400">No runs yet.</p>
                ) : null}
                <div className="max-h-56 space-y-2 overflow-y-auto pr-1">
                  {runs.slice(0, 20).map((run: WorkflowRunItem, index) => (
                    <article key={run.id ?? `run-${index}`} className="rounded border border-slate-700 bg-slate-900/60 p-2">
                      <p className="text-xs text-slate-200">{run.workflow_name ?? "-"}</p>
                      <p className="text-[11px] text-slate-400">
                        steps: {run.steps_completed ?? 0} · started: {toDateText(run.started_at)}
                      </p>
                      <p className="text-[11px] text-slate-500">completed: {toDateText(run.completed_at ?? undefined)}</p>
                    </article>
                  ))}
                </div>
              </div>
            </div>
          </div>
        </section>
      </div>
    </section>
  );
}
