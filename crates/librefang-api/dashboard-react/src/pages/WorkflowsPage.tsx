import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  deleteWorkflow,
  listWorkflowRuns,
  listWorkflows,
  runWorkflow,
  type WorkflowItem,
  type WorkflowRunItem
} from "../api";

const REFRESH_MS = 30000;

function toDateText(value?: string | null): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

export function WorkflowsPage() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [selectedWorkflowId, setSelectedWorkflowId] = useState<string>("");
  const [runInput, setRunInput] = useState("");
  const [running, setRunning] = useState(false);
  const [runResult, setRunResult] = useState<string>("");

  const workflowsQuery = useQuery({
    queryKey: ["workflows", "list"],
    queryFn: listWorkflows,
    refetchInterval: REFRESH_MS
  });

  const runsQuery = useQuery({
    queryKey: ["workflows", "runs", selectedWorkflowId],
    queryFn: () => listWorkflowRuns(selectedWorkflowId),
    enabled: Boolean(selectedWorkflowId)
  });

  const runMutation = useMutation({
    mutationFn: ({ workflowId, input }: { workflowId: string; input: string }) =>
      runWorkflow(workflowId, input)
  });

  const deleteMutation = useMutation({
    mutationFn: deleteWorkflow
  });

  const workflows = useMemo(
    () => [...(workflowsQuery.data ?? [])].sort(
      (a, b) => (b.created_at ?? "").localeCompare(a.created_at ?? "")
    ),
    [workflowsQuery.data]
  );

  const runs = runsQuery.data ?? [];

  async function handleRun() {
    if (!selectedWorkflowId || !runInput.trim()) return;
    setRunning(true);
    try {
      const result = await runMutation.mutateAsync({
        workflowId: selectedWorkflowId,
        input: runInput
      });
      setRunResult(typeof result.message === "string" ? result.message : JSON.stringify(result));
      await runsQuery.refetch();
    } catch (err) {
      setRunResult(`Error: ${err}`);
    } finally {
      setRunning(false);
    }
  }

  async function handleDelete(workflowId: string) {
    const wf = workflows.find(w => w.id === workflowId);
    if (!confirm(`Delete workflow "${wf?.name || workflowId}"?`)) return;

    try {
      await deleteMutation.mutateAsync(workflowId);
      if (selectedWorkflowId === workflowId) {
        setSelectedWorkflowId("");
      }
      await queryClient.invalidateQueries({ queryKey: ["workflows"] });
    } catch {}
  }

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Workflows</h1>
          <p className="text-sm text-slate-400">Manage your automation workflows.</p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => navigate({ to: "/canvas" })}
            className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-sky-500"
          >
            + New Canvas
          </button>
          <button
            onClick={() => void workflowsQuery.refetch()}
            className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm text-slate-300 transition hover:bg-slate-700"
            disabled={workflowsQuery.isFetching}
          >
            Refresh
          </button>
        </div>
      </header>

      <div className="grid gap-4 lg:grid-cols-[1fr_320px]">
        {/* Workflow List */}
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="mb-3 mt-0 text-base font-semibold">All Workflows</h2>

          {workflowsQuery.isLoading && workflows.length === 0 ? (
            <p className="text-sm text-slate-400">Loading workflows...</p>
          ) : workflows.length === 0 ? (
            <div className="py-8 text-center">
              <p className="mb-4 text-sm text-slate-400">No workflows yet</p>
              <button
                onClick={() => navigate({ to: "/canvas" })}
                className="rounded-lg border border-sky-500 bg-sky-600 px-4 py-2 text-sm font-medium text-white"
              >
                Create Your First Workflow
              </button>
            </div>
          ) : (
            <div className="grid gap-2 sm:grid-cols-2">
              {workflows.map((wf) => (
                <article
                  key={wf.id}
                  className={`cursor-pointer rounded-lg border p-3 transition ${
                    selectedWorkflowId === wf.id
                      ? "border-sky-500 bg-sky-500/10"
                      : "border-slate-700 bg-slate-800/50 hover:border-slate-600"
                  }`}
                  onClick={() => setSelectedWorkflowId(wf.id)}
                >
                  <div className="flex items-start justify-between">
                    <div>
                      <h3 className="m-0 text-sm font-semibold text-slate-200">{wf.name}</h3>
                      <p className="mt-1 text-xs text-slate-400">{wf.description || "No description"}</p>
                      <p className="mt-2 text-xs text-slate-500">
                        {wf.steps || 0} steps · {toDateText(wf.created_at)}
                      </p>
                    </div>
                    <button
                      onClick={(e) => { e.stopPropagation(); handleDelete(wf.id); }}
                      className="rounded border border-red-700 bg-red-700/20 px-2 py-1 text-xs text-red-400 hover:bg-red-700/30"
                    >
                      Delete
                    </button>
                  </div>
                </article>
              ))}
            </div>
          )}
        </div>

        {/* Selected Workflow Details */}
        <div className="flex flex-col gap-4">
          {/* Run Panel */}
          <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
            <h3 className="mb-3 mt-0 text-sm font-semibold">Run Workflow</h3>

            <select
              value={selectedWorkflowId}
              onChange={(e) => setSelectedWorkflowId(e.target.value)}
              className="mb-3 w-full rounded-lg border border-slate-700 bg-slate-800 px-3 py-2 text-sm text-slate-200"
            >
              <option value="">Select workflow</option>
              {workflows.map((wf) => (
                <option key={wf.id} value={wf.id}>{wf.name}</option>
              ))}
            </select>

            <textarea
              value={runInput}
              onChange={(e) => setRunInput(e.target.value)}
              placeholder="Input (optional)"
              rows={3}
              className="mb-3 w-full rounded-lg border border-slate-700 bg-slate-800 px-3 py-2 text-sm text-slate-200"
            />

            <button
              onClick={handleRun}
              disabled={!selectedWorkflowId || !runInput.trim() || running}
              className="w-full rounded-lg border border-emerald-600 bg-emerald-700 px-3 py-2 text-sm font-medium text-white hover:bg-emerald-600 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {running ? "Running..." : "Run Now"}
            </button>

            {runResult && (
              <pre className="mt-3 max-h-32 overflow-auto rounded border border-slate-700 bg-slate-950 p-2 text-xs text-slate-300 whitespace-pre-wrap">
                {runResult}
              </pre>
            )}
          </div>

          {/* Recent Runs */}
          <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
            <h3 className="mb-3 mt-0 text-sm font-semibold">Recent Runs</h3>

            {!selectedWorkflowId ? (
              <p className="text-xs text-slate-400">Select a workflow to view runs</p>
            ) : runsQuery.isLoading ? (
              <p className="text-xs text-slate-400">Loading...</p>
            ) : runs.length === 0 ? (
              <p className="text-xs text-slate-400">No runs yet</p>
            ) : (
              <div className="space-y-2">
                {runs.slice(0, 10).map((run, i) => (
                  <div key={run.id ?? i} className="rounded border border-slate-700 bg-slate-800/50 p-2">
                    <p className="text-xs text-slate-300">{run.workflow_name}</p>
                    <p className="text-[10px] text-slate-500">
                      {run.steps_completed ?? 0} steps · {toDateText(run.started_at)}
                    </p>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}
