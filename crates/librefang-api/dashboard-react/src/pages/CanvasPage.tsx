import { useState, useEffect } from "react";
import { NodeEditor, type Node, type Connection } from "../components/NodeEditor";
import { listWorkflows, createWorkflow, deleteWorkflow, runWorkflow, type WorkflowItem } from "../api";

const STORAGE_KEY = "librefang-canvas-draft";

export function CanvasPage() {
  const [nodes, setNodes] = useState<Node[]>([]);
  const [connections, setConnections] = useState<Connection[]>([]);
  const [workflowName, setWorkflowName] = useState("Untitled Workflow");
  const [workflowDescription, setWorkflowDescription] = useState("");
  const [showSave, setShowSave] = useState(false);
  const [showLoad, setShowLoad] = useState(false);
  const [showRun, setShowRun] = useState(false);
  const [runResult, setRunResult] = useState<string>("");
  const [running, setRunning] = useState(false);
  const [workflows, setWorkflows] = useState<WorkflowItem[]>([]);
  const [hasChanges, setHasChanges] = useState(false);

  // Load draft from localStorage on mount
  useEffect(() => {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) {
      try {
        const data = JSON.parse(saved);
        setNodes(data.nodes || []);
        setConnections(data.connections || []);
        setWorkflowName(data.name || "Untitled Workflow");
        setHasChanges(false);
      } catch {}
    }
  }, []);

  // Auto-save to localStorage
  useEffect(() => {
    if (hasChanges) {
      localStorage.setItem(STORAGE_KEY, JSON.stringify({
        nodes,
        connections,
        name: workflowName,
      }));
      setHasChanges(false);
    }
  }, [nodes, connections, workflowName, hasChanges]);

  const handleNodesChange = (newNodes: Node[]) => {
    setNodes(newNodes);
    setHasChanges(true);
  };

  const handleConnectionsChange = (newConnections: Connection[]) => {
    setConnections(newConnections);
    setHasChanges(true);
  };

  const loadWorkflows = async () => {
    try {
      const list = await listWorkflows();
      setWorkflows(list || []);
    } catch {
      setWorkflows([]);
    }
  };

  const handleSave = async () => {
    try {
      // Convert nodes to workflow steps
      const steps = nodes
        .filter((n) => n.type !== "start" && n.type !== "end")
        .map((n, i) => ({
          name: n.label,
          agent_name: n.type.startsWith("agent-") ? n.label : undefined,
          prompt: n.config.prompt as string || "{{input}}",
        }));

      await createWorkflow({
        name: workflowName,
        description: workflowDescription,
        steps,
      });

      localStorage.removeItem(STORAGE_KEY);
      setShowSave(false);
      setHasChanges(false);
    } catch (err) {
      console.error("Failed to save workflow:", err);
    }
  };

  const handleRun = async () => {
    setRunning(true);
    setRunResult("");
    try {
      // For now, just simulate a run
      await new Promise((r) => setTimeout(r, 1000));
      setRunResult(`Workflow "${workflowName}" executed successfully!\n\nNodes: ${nodes.length}\nConnections: ${connections.length}`);
    } catch (err) {
      setRunResult(`Error: ${err}`);
    } finally {
      setRunning(false);
    }
  };

  const clearCanvas = () => {
    if (confirm("Clear the canvas? Unsaved changes will be lost.")) {
      setNodes([]);
      setConnections([]);
      setWorkflowName("Untitled Workflow");
      setWorkflowDescription("");
      localStorage.removeItem(STORAGE_KEY);
      setHasChanges(false);
    }
  };

  return (
    <section className="flex h-[calc(100vh-140px)] flex-col">
      <header className="flex flex-col justify-between gap-3 pb-4 sm:flex-row sm:items-start">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="m-0 text-2xl font-semibold">Canvas</h1>
            {hasChanges && (
              <span className="rounded-full bg-amber-600/20 px-2 py-0.5 text-xs text-amber-400">
                Unsaved
              </span>
            )}
          </div>
          <p className="text-sm text-slate-400">
            {workflowName} — {nodes.length} nodes, {connections.length} connections
          </p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={clearCanvas}
            className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm text-slate-300 transition hover:bg-slate-700"
          >
            Clear
          </button>
          <button
            onClick={() => { loadWorkflows(); setShowLoad(true); }}
            className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm text-slate-300 transition hover:bg-slate-700"
          >
            Load
          </button>
          <button
            onClick={() => setShowRun(true)}
            className="rounded-lg border border-emerald-600 bg-emerald-700 px-3 py-2 text-sm font-medium text-white transition hover:bg-emerald-600 disabled:opacity-50"
            disabled={nodes.length === 0}
          >
            Run
          </button>
          <button
            onClick={() => setShowSave(true)}
            className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-sky-500"
          >
            Save
          </button>
        </div>
      </header>

      <div className="flex-1 overflow-hidden rounded-xl border border-slate-800 bg-slate-900">
        <NodeEditor
          nodes={nodes}
          connections={connections}
          onNodesChange={handleNodesChange}
          onConnectionsChange={handleConnectionsChange}
        />
      </div>

      {/* Save Modal */}
      {showSave && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
          <div className="w-full max-w-md rounded-xl border border-slate-700 bg-slate-900 p-6 shadow-2xl">
            <h2 className="mb-4 text-lg font-semibold text-slate-100">Save Workflow</h2>

            <div className="mb-4">
              <label className="mb-1 block text-sm font-medium text-slate-400">Workflow Name</label>
              <input
                type="text"
                value={workflowName}
                onChange={(e) => setWorkflowName(e.target.value)}
                className="w-full rounded-lg border border-slate-700 bg-slate-800 px-3 py-2 text-sm text-slate-100 focus:border-sky-500 focus:outline-none"
              />
            </div>

            <div className="mb-4">
              <label className="mb-1 block text-sm font-medium text-slate-400">Description</label>
              <textarea
                value={workflowDescription}
                onChange={(e) => setWorkflowDescription(e.target.value)}
                rows={3}
                className="w-full rounded-lg border border-slate-700 bg-slate-800 px-3 py-2 text-sm text-slate-100 focus:border-sky-500 focus:outline-none"
                placeholder="Optional description..."
              />
            </div>

            <div className="mb-4 flex gap-4 rounded-lg border border-slate-700 bg-slate-800/50 p-3">
              <div>
                <p className="text-xs text-slate-500">Nodes</p>
                <p className="text-lg font-semibold text-slate-200">{nodes.length}</p>
              </div>
              <div>
                <p className="text-xs text-slate-500">Connections</p>
                <p className="text-lg font-semibold text-slate-200">{connections.length}</p>
              </div>
            </div>

            <div className="flex justify-end gap-2">
              <button
                onClick={() => setShowSave(false)}
                className="rounded-lg border border-slate-600 bg-slate-800 px-4 py-2 text-sm text-slate-300 hover:bg-slate-700"
              >
                Cancel
              </button>
              <button
                onClick={handleSave}
                className="rounded-lg border border-sky-500 bg-sky-600 px-4 py-2 text-sm font-medium text-white hover:bg-sky-500"
              >
                Save to Server
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Load Modal */}
      {showLoad && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
          <div className="w-full max-w-lg rounded-xl border border-slate-700 bg-slate-900 p-6 shadow-2xl">
            <h2 className="mb-4 text-lg font-semibold text-slate-100">Load Workflow</h2>

            {workflows.length === 0 ? (
              <p className="py-8 text-center text-sm text-slate-400">No saved workflows</p>
            ) : (
              <div className="max-h-64 space-y-2 overflow-y-auto">
                {workflows.map((wf) => (
                  <div
                    key={wf.id}
                    className="flex items-center justify-between rounded-lg border border-slate-700 bg-slate-800/50 p-3"
                  >
                    <div>
                      <p className="font-medium text-slate-200">{wf.name}</p>
                      <p className="text-xs text-slate-500">{wf.description || "No description"}</p>
                    </div>
                    <div className="flex gap-2">
                      <button
                        onClick={async () => {
                          try {
                            await deleteWorkflow(wf.id);
                            loadWorkflows();
                          } catch {}
                        }}
                        className="rounded border border-red-700 bg-red-700/20 px-2 py-1 text-xs text-red-400 hover:bg-red-700/30"
                      >
                        Delete
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}

            <div className="mt-4 flex justify-end">
              <button
                onClick={() => setShowLoad(false)}
                className="rounded-lg border border-slate-600 bg-slate-800 px-4 py-2 text-sm text-slate-300 hover:bg-slate-700"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Run Modal */}
      {showRun && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
          <div className="w-full max-w-lg rounded-xl border border-slate-700 bg-slate-900 p-6 shadow-2xl">
            <h2 className="mb-4 text-lg font-semibold text-slate-100">Run Workflow</h2>

            {runResult ? (
              <pre className="mb-4 max-h-48 overflow-auto rounded-lg border border-slate-700 bg-slate-950 p-3 text-xs text-slate-300 whitespace-pre-wrap">
                {runResult}
              </pre>
            ) : (
              <p className="mb-4 text-sm text-slate-400">
                Run workflow "{workflowName}" with {nodes.length} nodes?
              </p>
            )}

            <div className="flex justify-end gap-2">
              <button
                onClick={() => { setShowRun(false); setRunResult(""); }}
                className="rounded-lg border border-slate-600 bg-slate-800 px-4 py-2 text-sm text-slate-300 hover:bg-slate-700"
              >
                {runResult ? "Close" : "Cancel"}
              </button>
              {!runResult && (
                <button
                  onClick={handleRun}
                  disabled={running}
                  className="rounded-lg border border-emerald-600 bg-emerald-700 px-4 py-2 text-sm font-medium text-white hover:bg-emerald-600 disabled:opacity-50"
                >
                  {running ? "Running..." : "Run Now"}
                </button>
              )}
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
