import { useCallback, useState, useEffect } from "react";
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  addEdge,
  useNodesState,
  useEdgesState,
  type Node,
  type Edge,
  type Connection,
  MarkerType,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { listAgents, type AgentItem } from "../api";

const STORAGE_KEY = "librefang-canvas-draft";

interface WorkflowNodeData {
  label: string;
  type: string;
  description: string;
  config: Record<string, unknown>;
}

const nodeTypes = [
  { type: "start", label: "Start", color: "#22c55e", icon: "S", description: "Workflow start", inputs: 0, outputs: 1 },
  { type: "end", label: "End", color: "#ef4444", icon: "E", description: "Workflow end", inputs: 1, outputs: 0 },
  { type: "schedule", label: "Schedule", color: "#f59e0b", icon: "C", description: "Run on schedule", inputs: 0, outputs: 1 },
  { type: "webhook", label: "Webhook", color: "#06b6d4", icon: "W", description: "HTTP webhook", inputs: 0, outputs: 1 },
  { type: "channel", label: "Channel", color: "#14b8a6", icon: "M", description: "Message trigger", inputs: 0, outputs: 1 },
  { type: "condition", label: "Condition", color: "#10b981", icon: "?", description: "Branch logic", inputs: 1, outputs: 2 },
  { type: "loop", label: "Loop", color: "#ec4899", icon: "L", description: "Loop items", inputs: 1, outputs: 1 },
  { type: "parallel", label: "Parallel", color: "#f97316", icon: "P", description: "Parallel branches", inputs: 1, outputs: 3 },
  { type: "wait", label: "Wait", color: "#64748b", icon: "T", description: "Wait duration", inputs: 1, outputs: 1 },
  { type: "respond", label: "Respond", color: "#22c55e", icon: "R", description: "Send response", inputs: 1, outputs: 0 },
  { type: "agent", label: "Agent", color: "#6366f1", icon: "A", description: "Run agent", inputs: 1, outputs: 1 },
];

function CustomNode({ data, type }: { data: WorkflowNodeData; type?: string }) {
  const nodeType = nodeTypes.find(n => n.type === type) || nodeTypes[10];

  return (
    <div className="rounded-lg border-2 border-slate-700 bg-slate-900 shadow-lg min-w-[150px] overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2" style={{ backgroundColor: nodeType.color }}>
        <span className="text-sm font-bold text-white">{nodeType.icon}</span>
        <span className="text-sm font-medium text-white truncate">{data.label}</span>
      </div>
      <div className="px-3 py-2">
        <p className="text-xs text-slate-400">{data.description}</p>
      </div>
    </div>
  );
}

const initialNodes: Node[] = [
  { id: "1", type: "start", position: { x: 50, y: 200 }, data: { label: "Start", type: "start", description: "Workflow start" } },
  { id: "2", type: "agent", position: { x: 300, y: 200 }, data: { label: "Agent", type: "agent", description: "Run agent" } },
  { id: "3", type: "respond", position: { x: 550, y: 200 }, data: { label: "Respond", type: "respond", description: "Send response" } },
];

const initialEdges: Edge[] = [
  { id: "e1-2", source: "1", target: "2", markerEnd: { type: MarkerType.ArrowClosed } },
  { id: "e2-3", source: "2", target: "3", markerEnd: { type: MarkerType.ArrowClosed } },
];

export function CanvasPage() {
  const [nodes, setNodes, onNodesChange] = useNodesState(initialNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(initialEdges);
  const [agents, setAgents] = useState<AgentItem[]>([]);
  const [showSave, setShowSave] = useState(false);
  const [showLoad, setShowLoad] = useState(false);
  const [workflowName, setWorkflowName] = useState("My Workflow");

  // Load agents
  useEffect(() => {
    listAgents().then(setAgents).catch(() => setAgents([]));
  }, []);

  // Load from localStorage
  useEffect(() => {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) {
      try {
        const data = JSON.parse(saved);
        if (data.nodes) setNodes(data.nodes);
        if (data.edges) setEdges(data.edges);
        if (data.name) setWorkflowName(data.name);
      } catch {}
    }
  }, []);

  // Auto-save
  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      nodes, edges, name: workflowName
    }));
  }, [nodes, edges, workflowName]);

  const onConnect = useCallback(
    (params: Connection) => setEdges((eds) => addEdge({ ...params, markerEnd: { type: MarkerType.ArrowClosed } }, eds)),
    [setEdges]
  );

  const addNode = useCallback((type: string) => {
    const nodeType = nodeTypes.find(n => n.type === type) || nodeTypes[10];
    const newNode: Node = {
      id: `${type}-${Date.now()}`,
      type,
      position: { x: Math.random() * 400, y: Math.random() * 300 },
      data: { label: nodeType.label, type, description: nodeType.description, config: {} }
    };
    setNodes((nds) => [...nds, newNode]);
  }, [setNodes]);

  const onDragOver = useCallback((event: React.DragEvent) => {
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
  }, []);

  const onDrop = useCallback(
    (event: React.DragEvent) => {
      event.preventDefault();
      const type = event.dataTransfer.getData("application/reactflow");
      if (!type) return;

      const nodeType = nodeTypes.find(n => n.type === type) || nodeTypes[10];
      const newNode: Node = {
        id: `${type}-${Date.now()}`,
        type,
        position: { x: event.clientX - 300, y: event.clientY - 100 },
        data: { label: nodeType.label, type, description: nodeType.description, config: {} }
      };
      setNodes((nds) => [...nds, newNode]);
    },
    [setNodes]
  );

  const handleSave = () => {
    // TODO: Save to backend
    setShowSave(false);
  };

  return (
    <section className="flex h-[calc(100vh-140px)] flex-col">
      <header className="flex items-center justify-between gap-4 pb-4">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Workflow Canvas</h1>
          <p className="text-sm text-slate-400">Drag nodes to build your automation</p>
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => { setNodes([]); setEdges([]); }}
            className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm text-slate-300 hover:bg-slate-700"
          >
            Clear
          </button>
          <button
            onClick={() => setShowLoad(true)}
            className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm text-slate-300 hover:bg-slate-700"
          >
            Load
          </button>
          <button
            onClick={() => setShowSave(true)}
            className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white hover:bg-sky-500"
          >
            Save
          </button>
        </div>
      </header>

      <div className="flex flex-1 gap-0 overflow-hidden rounded-xl border border-slate-800">
        {/* Sidebar */}
        <aside className="w-56 flex-shrink-0 overflow-y-auto border-r border-slate-700 bg-slate-900/90 p-3">
          <h3 className="mb-3 text-xs font-semibold uppercase text-slate-400">Nodes</h3>
          <div className="space-y-1">
            <p className="mb-2 text-xs font-medium text-slate-500">TRIGGERS</p>
            {nodeTypes.filter(n => ["start", "schedule", "webhook", "channel"].includes(n.type)).map(n => (
              <button
                key={n.type}
                draggable
                onDragStart={(e) => e.dataTransfer.setData("application/reactflow", n.type)}
                onClick={() => addNode(n.type)}
                className="flex w-full items-center gap-2 rounded-lg border border-slate-700/50 bg-slate-800/50 p-2 text-left text-xs transition hover:bg-slate-800"
              >
                <div className="flex h-6 w-6 items-center justify-center rounded text-xs font-bold text-white" style={{ backgroundColor: n.color }}>
                  {n.icon}
                </div>
                <span className="text-slate-300">{n.label}</span>
              </button>
            ))}
            <p className="mb-2 mt-4 text-xs font-medium text-slate-500">ACTIONS</p>
            {nodeTypes.filter(n => ["agent", "condition", "loop", "parallel", "wait", "respond"].includes(n.type)).map(n => (
              <button
                key={n.type}
                draggable
                onDragStart={(e) => e.dataTransfer.setData("application/reactflow", n.type)}
                onClick={() => addNode(n.type)}
                className="flex w-full items-center gap-2 rounded-lg border border-slate-700/50 bg-slate-800/50 p-2 text-left text-xs transition hover:bg-slate-800"
              >
                <div className="flex h-6 w-6 items-center justify-center rounded text-xs font-bold text-white" style={{ backgroundColor: n.color }}>
                  {n.icon}
                </div>
                <span className="text-slate-300">{n.label}</span>
              </button>
            ))}
            {agents.length > 0 && (
              <>
                <p className="mb-2 mt-4 text-xs font-medium text-slate-500">MY AGENTS</p>
                {agents.slice(0, 5).map(a => (
                  <button
                    key={a.id}
                    draggable
                    onDragStart={(e) => e.dataTransfer.setData("application/reactflow", a.id)}
                    onClick={() => {
                      const newNode: Node = {
                        id: `agent-${Date.now()}`,
                        type: "agent",
                        position: { x: Math.random() * 400, y: Math.random() * 300 },
                        data: { label: a.name, type: a.id, description: a.description || "Agent", config: {} }
                      };
                      setNodes((nds) => [...nds, newNode]);
                    }}
                    className="flex w-full items-center gap-2 rounded-lg border border-slate-700/50 bg-slate-800/50 p-2 text-left text-xs transition hover:bg-slate-800"
                  >
                    <div className="flex h-6 w-6 items-center justify-center rounded bg-indigo-600 text-xs font-bold text-white">A</div>
                    <span className="truncate text-slate-300">{a.name}</span>
                  </button>
                ))}
              </>
            )}
          </div>
        </aside>

        {/* React Flow Canvas */}
        <div className="flex-1" onDragOver={onDragOver} onDrop={onDrop}>
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            nodeTypes={{ custom: CustomNode }}
            fitView
            className="bg-slate-950"
          >
            <Background color="#334155" gap={20} />
            <Controls className="bg-slate-800 border-slate-700" />
            <MiniMap
              nodeColor={(n) => nodeTypes.find(t => t.type === n.type)?.color || "#6366f1"}
              className="bg-slate-900 border-slate-700"
            />
          </ReactFlow>
        </div>
      </div>

      {/* Save Modal */}
      {showSave && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
          <div className="w-full max-w-md rounded-xl border border-slate-700 bg-slate-900 p-6">
            <h2 className="mb-4 text-lg font-semibold">Save Workflow</h2>
            <input
              type="text"
              value={workflowName}
              onChange={(e) => setWorkflowName(e.target.value)}
              placeholder="Workflow name"
              className="mb-4 w-full rounded-lg border border-slate-700 bg-slate-800 px-3 py-2 text-slate-100"
            />
            <p className="mb-4 text-sm text-slate-400">
              {nodes.length} nodes, {edges.length} connections
            </p>
            <div className="flex justify-end gap-2">
              <button onClick={() => setShowSave(false)} className="rounded-lg border border-slate-600 bg-slate-800 px-4 py-2 text-slate-300">Cancel</button>
              <button onClick={handleSave} className="rounded-lg border border-sky-500 bg-sky-600 px-4 py-2 text-white">Save</button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
