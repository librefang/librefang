import { useState } from "react";
import { NodeEditor, type Node, type Connection } from "../components/NodeEditor";

export function CanvasPage() {
  const [nodes, setNodes] = useState<Node[]>([
    { id: "start-1", type: "start", label: "Start", x: 50, y: 200, config: {} },
    { id: "agent-1", type: "agent", label: "Agent", x: 300, y: 200, config: {} },
    { id: "respond-1", type: "respond", label: "Respond", x: 550, y: 200, config: {} },
  ]);
  const [connections, setConnections] = useState<Connection[]>([
    { id: "c1", from: "start-1", to: "agent-1" },
    { id: "c2", from: "agent-1", to: "respond-1" },
  ]);
  const [workflowName, setWorkflowName] = useState("My Workflow");
  const [showSave, setShowSave] = useState(false);

  const handleSave = () => {
    // TODO: Save to backend
    console.log("Saving workflow:", { name: workflowName, nodes, connections });
    setShowSave(false);
  };

  return (
    <section className="flex h-[calc(100vh-140px)] flex-col">
      <header className="flex flex-col justify-between gap-3 pb-4 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Canvas</h1>
          <p className="text-sm text-slate-400">Visual workflow editor - drag nodes to build automation.</p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => setShowSave(true)}
            className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-sky-500"
          >
            Save Workflow
          </button>
        </div>
      </header>

      <div className="flex-1 overflow-hidden rounded-xl border border-slate-800 bg-slate-900">
        <NodeEditor
          nodes={nodes}
          connections={connections}
          onNodesChange={setNodes}
          onConnectionsChange={setConnections}
        />
      </div>

      {/* Save Modal */}
      {showSave && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="w-full max-w-md rounded-xl border border-slate-700 bg-slate-900 p-6 shadow-xl">
            <h2 className="mb-4 text-lg font-semibold text-slate-100">Save Workflow</h2>

            <div className="mb-4">
              <label className="block mb-1 text-sm font-medium text-slate-400">Workflow Name</label>
              <input
                type="text"
                value={workflowName}
                onChange={(e) => setWorkflowName(e.target.value)}
                className="w-full rounded-lg border border-slate-700 bg-slate-800 px-3 py-2 text-sm text-slate-100"
              />
            </div>

            <div className="mb-4">
              <label className="block mb-1 text-sm font-medium text-slate-400">Nodes</label>
              <p className="text-sm text-slate-300">{nodes.length} nodes</p>
            </div>

            <div className="mb-4">
              <label className="block mb-1 text-sm font-medium text-slate-400">Connections</label>
              <p className="text-sm text-slate-300">{connections.length} connections</p>
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
                Save
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
