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
import { useUIStore } from "../lib/store";

const STORAGE_KEY = "librefang-canvas-draft";

interface WorkflowNodeData {
  label: string;
  type: string;
  description: string;
  config: Record<string, unknown>;
}

// Map logical types to semantic colors defined in index.css
const nodeTypes = [
  { type: "start", label: "Start", color: "var(--success-color)", icon: "S", description: "Workflow start", inputs: 0, outputs: 1 },
  { type: "end", label: "End", color: "var(--error-color)", icon: "E", description: "Workflow end", inputs: 1, outputs: 0 },
  { type: "schedule", label: "Schedule", color: "var(--warning-color)", icon: "C", description: "Run on schedule", inputs: 0, outputs: 1 },
  { type: "webhook", label: "Webhook", color: "var(--brand-color)", icon: "W", description: "HTTP webhook", inputs: 0, outputs: 1 },
  { type: "channel", label: "Channel", color: "var(--accent-color)", icon: "M", description: "Message trigger", inputs: 0, outputs: 1 },
  { type: "condition", label: "Condition", color: "var(--success-color)", icon: "?", description: "Branch logic", inputs: 1, outputs: 2 },
  { type: "loop", label: "Loop", color: "var(--accent-color)", icon: "L", description: "Loop items", inputs: 1, outputs: 1 },
  { type: "parallel", label: "Parallel", color: "var(--warning-color)", icon: "P", description: "Parallel branches", inputs: 1, outputs: 3 },
  { type: "wait", label: "Wait", color: "var(--text-muted)", icon: "T", description: "Wait duration", inputs: 1, outputs: 1 },
  { type: "respond", label: "Respond", color: "var(--success-color)", icon: "R", description: "Send response", inputs: 1, outputs: 0 },
  { type: "agent", label: "Agent", color: "var(--brand-color)", icon: "A", description: "Run agent", inputs: 1, outputs: 1 },
];

function CustomNode({ data, type }: { data: WorkflowNodeData; type?: string }) {
  const nodeType = nodeTypes.find(n => n.type === type) || nodeTypes[10];

  return (
    <div className="rounded-lg border-2 border-border-subtle bg-surface shadow-lg min-w-[150px] overflow-hidden transition-colors duration-300">
      <div className="flex items-center gap-2 px-3 py-2" style={{ backgroundColor: nodeType.color }}>
        <span className="text-sm font-bold text-white drop-shadow-sm">{nodeType.icon}</span>
        <span className="text-sm font-bold text-white truncate drop-shadow-sm">{data.label}</span>
      </div>
      <div className="px-3 py-2 bg-surface">
        <p className="text-[10px] font-medium text-text-dim leading-tight">{data.description}</p>
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
  const { theme } = useUIStore();
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
    setShowSave(false);
  };

  return (
    <div className="flex h-[calc(100vh-140px)] flex-col transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 pb-6 md:flex-row md:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" />
            </svg>
            Visual Orchestrator
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight">Workflow Canvas</h1>
          <p className="mt-1 text-text-dim font-medium">Design autonomous agent behaviors using a visual logic flow.</p>
        </div>
        
        <div className="flex gap-2">
          <button
            onClick={() => { setNodes([]); setEdges([]); }}
            className="rounded-xl border border-border-subtle bg-surface px-4 py-2 text-sm font-bold text-text-dim hover:text-brand hover:border-brand/30 transition-all shadow-sm"
          >
            Clear
          </button>
          <button
            onClick={() => setShowLoad(true)}
            className="rounded-xl border border-border-subtle bg-surface px-4 py-2 text-sm font-bold text-text-dim hover:text-brand hover:border-brand/30 transition-all shadow-sm"
          >
            Load
          </button>
          <button
            onClick={() => setShowSave(true)}
            className="rounded-xl bg-brand px-6 py-2 text-sm font-bold text-white hover:opacity-90 transition-all shadow-md shadow-brand/20"
          >
            Save Workflow
          </button>
        </div>
      </header>

      <div className="flex flex-1 overflow-hidden rounded-2xl border border-border-subtle bg-surface shadow-xl relative ring-1 ring-black/5 dark:ring-white/5">
        {/* Sidebar */}
        <aside className="w-64 flex-shrink-0 overflow-y-auto border-r border-border-subtle bg-main/50 backdrop-blur-md p-4 scrollbar-thin">
          <h3 className="mb-4 text-[10px] font-black uppercase tracking-[0.2em] text-text-dim/60">Node Library</h3>
          
          <div className="space-y-6">
            <section>
              <p className="mb-2 text-[10px] font-bold text-brand uppercase tracking-wider">Triggers</p>
              <div className="grid gap-2">
                {nodeTypes.filter(n => ["start", "schedule", "webhook", "channel"].includes(n.type)).map(n => (
                  <button
                    key={n.type}
                    draggable
                    onDragStart={(e) => e.dataTransfer.setData("application/reactflow", n.type)}
                    onClick={() => addNode(n.type)}
                    className="flex w-full items-center gap-3 rounded-xl border border-border-subtle bg-surface p-2.5 text-left transition-all hover:border-brand/30 hover:shadow-sm hover:translate-x-1 group"
                  >
                    <div className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg text-xs font-black text-white shadow-sm" style={{ backgroundColor: n.color }}>
                      {n.icon}
                    </div>
                    <div className="min-w-0">
                      <p className="text-xs font-bold text-slate-700 dark:text-slate-200 group-hover:text-brand">{n.label}</p>
                      <p className="text-[9px] text-text-dim truncate">{n.description}</p>
                    </div>
                  </button>
                ))}
              </div>
            </section>

            <section>
              <p className="mb-2 text-[10px] font-bold text-brand uppercase tracking-wider">Logic & Actions</p>
              <div className="grid gap-2">
                {nodeTypes.filter(n => ["agent", "condition", "loop", "parallel", "wait", "respond"].includes(n.type)).map(n => (
                  <button
                    key={n.type}
                    draggable
                    onDragStart={(e) => e.dataTransfer.setData("application/reactflow", n.type)}
                    onClick={() => addNode(n.type)}
                    className="flex w-full items-center gap-3 rounded-xl border border-border-subtle bg-surface p-2.5 text-left transition-all hover:border-brand/30 hover:shadow-sm hover:translate-x-1 group"
                  >
                    <div className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg text-xs font-black text-white shadow-sm" style={{ backgroundColor: n.color }}>
                      {n.icon}
                    </div>
                    <div className="min-w-0">
                      <p className="text-xs font-bold text-slate-700 dark:text-slate-200 group-hover:text-brand">{n.label}</p>
                      <p className="text-[9px] text-text-dim truncate">{n.description}</p>
                    </div>
                  </button>
                ))}
              </div>
            </section>

            {agents.length > 0 && (
              <section>
                <p className="mb-2 text-[10px] font-bold text-brand uppercase tracking-wider">My Agents</p>
                <div className="grid gap-2">
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
                      className="flex w-full items-center gap-3 rounded-xl border border-border-subtle bg-surface p-2.5 text-left transition-all hover:border-brand/30 hover:shadow-sm hover:translate-x-1 group"
                    >
                      <div className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg bg-brand text-xs font-black text-white shadow-sm">A</div>
                      <div className="min-w-0">
                        <p className="text-xs font-bold text-slate-700 dark:text-slate-200 group-hover:text-brand truncate">{a.name}</p>
                        <p className="text-[9px] text-text-dim truncate">{a.model_name || "Active Agent"}</p>
                      </div>
                    </button>
                  ))}
                </div>
              </section>
            )}
          </div>
        </aside>

        {/* React Flow Canvas */}
        <main className="flex-1 relative" onDragOver={onDragOver} onDrop={onDrop}>
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            nodeTypes={{ custom: CustomNode }}
            fitView
            colorMode={theme}
            className="bg-main"
          >
            <Background 
              color={theme === "dark" ? "rgba(255,255,255,0.05)" : "rgba(0,0,0,0.05)"} 
              gap={24} 
              size={1} 
            />
            <Controls className="!bg-surface !border-border-subtle !shadow-lg !rounded-lg overflow-hidden fill-text-dim" />
            <MiniMap
              nodeStrokeColor="var(--border-color)"
              nodeColor={(n) => nodeTypes.find(t => t.type === n.type)?.color || "var(--brand-color)"}
              maskColor={theme === "dark" ? "rgba(0,0,0,0.6)" : "rgba(255,255,255,0.6)"}
              className="!bg-surface !border-border-subtle !rounded-xl !shadow-2xl"
            />
          </ReactFlow>
        </main>
      </div>

      {/* Save Modal */}
      {showSave && (
        <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/60 backdrop-blur-sm p-4">
          <div className="w-full max-w-md rounded-2xl border border-border-subtle bg-surface p-8 shadow-2xl animate-in zoom-in-95 duration-200">
            <h2 className="text-xl font-black tracking-tight">Save Workflow</h2>
            <p className="mt-1 text-sm text-text-dim font-medium">Give your automation a name to save it to your library.</p>
            
            <input
              type="text"
              value={workflowName}
              autoFocus
              onChange={(e) => setWorkflowName(e.target.value)}
              placeholder="e.g. Daily Market Summary"
              className="mt-6 w-full rounded-xl border border-border-subtle bg-main px-4 py-3 text-sm font-bold focus:border-brand focus:ring-2 focus:ring-brand/20 transition-all outline-none"
            />
            
            <div className="mt-6 flex items-center gap-4 p-4 rounded-xl bg-brand-muted border border-brand/10">
              <div className="h-10 w-10 flex items-center justify-center rounded-full bg-brand/20 text-brand font-black">!</div>
              <div className="text-xs font-bold text-slate-600 dark:text-slate-300">
                {nodes.length} logic nodes and {edges.length} connections will be serialized.
              </div>
            </div>

            <div className="mt-8 flex justify-end gap-3">
              <button 
                onClick={() => setShowSave(false)} 
                className="rounded-xl border border-border-subtle px-6 py-2.5 text-sm font-bold text-text-dim hover:bg-surface-hover transition-all"
              >
                Cancel
              </button>
              <button 
                onClick={handleSave} 
                className="rounded-xl bg-brand px-8 py-2.5 text-sm font-bold text-white hover:opacity-90 shadow-lg shadow-brand/20 transition-all"
              >
                Confirm Save
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
