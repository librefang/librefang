import { useCallback, useState, useEffect } from "react";
import { listAgents, type AgentItem } from "../api";

export interface Node {
  id: string;
  type: string;
  label: string;
  x: number;
  y: number;
  config: Record<string, unknown>;
}

export interface Connection {
  id: string;
  from: string;
  to: string;
}

export interface NodeType {
  type: string;
  label: string;
  color: string;
  icon: string;
  description: string;
  inputs: number;
  outputs: number;
  category: "core" | "agent" | "trigger" | "logic" | "output" | "myagents";
}

const BUILTIN_NODE_TYPES: NodeType[] = [
  // Core
  { type: "start", label: "Start", color: "#22c55e", icon: "S", description: "Workflow start point", inputs: 0, outputs: 1, category: "core" },
  { type: "end", label: "End", color: "#ef4444", icon: "E", description: "Workflow end point", inputs: 1, outputs: 0, category: "core" },

  // Trigger
  { type: "schedule", label: "Schedule", color: "#f59e0b", icon: "C", description: "Run on schedule", inputs: 0, outputs: 1, category: "trigger" },
  { type: "webhook", label: "Webhook", color: "#06b6d4", icon: "W", description: "HTTP webhook trigger", inputs: 0, outputs: 1, category: "trigger" },
  { type: "channel", label: "Channel", color: "#14b8a6", icon: "M", description: "Message channel trigger", inputs: 0, outputs: 1, category: "trigger" },

  // Logic
  { type: "condition", label: "Condition", color: "#10b981", icon: "?", description: "Branch by condition", inputs: 1, outputs: 2, category: "logic" },
  { type: "loop", label: "Loop", color: "#ec4899", icon: "L", description: "Loop over items", inputs: 1, outputs: 1, category: "logic" },
  { type: "parallel", label: "Parallel", color: "#f97316", icon: "P", description: "Run in parallel", inputs: 1, outputs: 3, category: "logic" },
  { type: "wait", label: "Wait", color: "#64748b", icon: "T", description: "Wait for duration", inputs: 1, outputs: 1, category: "logic" },

  // Output
  { type: "respond", label: "Respond", color: "#22c55e", icon: "R", description: "Send response", inputs: 1, outputs: 0, category: "output" },
  { type: "notify", label: "Notify", color: "#eab308", icon: "N", description: "Send notification", inputs: 1, outputs: 1, category: "output" },
  { type: "save", label: "Save", color: "#0ea5e9", icon: "S", description: "Save to memory", inputs: 1, outputs: 1, category: "output" },
];

interface NodeEditorProps {
  nodes: Node[];
  connections: Connection[];
  onNodesChange: (nodes: Node[]) => void;
  onConnectionsChange: (connections: Connection[]) => void;
}

export function NodeEditor({ nodes, connections, onNodesChange, onConnectionsChange }: NodeEditorProps) {
  const [selectedNode, setSelectedNode] = useState<string | null>(null);
  const [connecting, setConnecting] = useState<{ nodeId: string; output: number } | null>(null);
  const [dragging, setDragging] = useState<{ nodeId: string; startX: number; startY: number } | null>(null);
  const [canvasOffset, setCanvasOffset] = useState({ x: 0, y: 0 });
  const [zoom, setZoom] = useState(1);
  const [isPanning, setIsPanning] = useState(false);
  const [panStart, setPanStart] = useState({ x: 0, y: 0 });
  const [agents, setAgents] = useState<AgentItem[]>([]);

  // Load agents
  useEffect(() => {
    listAgents().then(setAgents).catch(() => setAgents([]));
  }, []);

  const getNodeType = (type: string) => {
    // Check built-in types first
    const builtin = BUILTIN_NODE_TYPES.find((n) => n.type === type);
    if (builtin) return builtin;
    // Check if it's an agent
    const agent = agents.find((a) => a.id === type || a.name === type);
    if (agent) {
      return {
        type: agent.id,
        label: agent.name,
        color: "#6366f1",
        icon: "A",
        description: agent.description || "Custom agent",
        inputs: 1,
        outputs: 1,
        category: "myagents" as const,
      };
    }
    return null;
  };

  const allNodeTypes = [
    ...BUILTIN_NODE_TYPES,
    ...agents.map((a) => ({
      type: a.id,
      label: a.name,
      color: "#6366f1",
      icon: "A",
      description: a.description || "Your agent",
      inputs: 1,
      outputs: 1,
      category: "myagents" as const,
    })),
  ];

  const categories = [
    { id: "myagents", label: "My Agents", icon: "👤" },
    { id: "core", label: "Core", icon: "⚡" },
    { id: "trigger", label: "Triggers", icon: "🔔" },
    { id: "logic", label: "Logic", icon: "🔀" },
    { id: "output", label: "Output", icon: "📤" },
  ] as const;

  const handleDragStart = useCallback((e: React.MouseEvent, nodeId: string) => {
    const node = nodes.find((n) => n.id === nodeId);
    if (!node) return;
    setDragging({ nodeId, startX: e.clientX - node.x, startY: e.clientY - node.y });
    setSelectedNode(nodeId);
  }, [nodes]);

  const handleMouseMove = useCallback((e: React.MouseEvent) => {
    if (dragging) {
      const newNodes = nodes.map((n) =>
        n.id === dragging.nodeId
          ? { ...n, x: (e.clientX - dragging.startX) / zoom, y: (e.clientY - dragging.startY) / zoom }
          : n
      );
      onNodesChange(newNodes);
    } else if (isPanning) {
      setCanvasOffset((prev) => ({
        x: prev.x + e.clientX - panStart.x,
        y: prev.y + e.clientY - panStart.y,
      }));
      setPanStart({ x: e.clientX, y: e.clientY });
    }
  }, [dragging, isPanning, panStart, zoom, nodes, onNodesChange]);

  const handleMouseUp = useCallback(() => {
    setDragging(null);
    setIsPanning(false);
  }, []);

  const handleCanvasMouseDown = useCallback((e: React.MouseEvent) => {
    if (e.button === 1 || (e.button === 0 && e.altKey)) {
      setIsPanning(true);
      setPanStart({ x: e.clientX, y: e.clientY });
    } else if (e.target === e.currentTarget) {
      setSelectedNode(null);
    }
  }, []);

  const addNode = useCallback((type: string, x: number, y: number) => {
    const nodeType = allNodeTypes.find((n) => n.type === type);
    if (!nodeType) return;

    const newNode: Node = {
      id: `${type}-${Date.now()}`,
      type,
      label: nodeType.label,
      x: (x - canvasOffset.x) / zoom,
      y: (y - canvasOffset.y) / zoom,
      config: {},
    };
    onNodesChange([...nodes, newNode]);
  }, [allNodeTypes, canvasOffset, zoom, nodes, onNodesChange]);

  const deleteNode = useCallback((nodeId: string) => {
    const newNodes = nodes.filter((n) => n.id !== nodeId);
    const newConnections = connections.filter(
      (c) => c.from !== nodeId && c.to !== nodeId
    );
    onNodesChange(newNodes);
    onConnectionsChange(newConnections);
    setSelectedNode(null);
  }, [nodes, connections, onNodesChange, onConnectionsChange]);

  const startConnection = useCallback((nodeId: string, output: number) => {
    setConnecting({ nodeId, output });
  }, []);

  const endConnection = useCallback((nodeId: string, input: number) => {
    if (connecting && connecting.nodeId !== nodeId) {
      const exists = connections.some(
        (c) => c.from === connecting.nodeId && c.to === nodeId
      );
      if (!exists) {
        const newConnection: Connection = {
          id: `${connecting.nodeId}-${nodeId}-${Date.now()}`,
          from: connecting.nodeId,
          to: nodeId,
        };
        onConnectionsChange([...connections, newConnection]);
      }
    }
    setConnecting(null);
  }, [connecting, connections, onConnectionsChange]);

  const renderConnections = () => {
    return connections.map((conn) => {
      const fromNode = nodes.find((n) => n.id === conn.from);
      const toNode = nodes.find((n) => n.id === conn.to);
      if (!fromNode || !toNode) return null;

      const fromX = fromNode.x + 180;
      const fromY = fromNode.y + 40;
      const toX = toNode.x;
      const toY = toNode.y + 40;

      const midX = (fromX + toX) / 2;
      const path = `M ${fromX} ${fromY} C ${midX + 50} ${fromY}, ${midX - 50} ${toY}, ${toX} ${toY}`;

      return (
        <g key={conn.id}>
          <path
            d={path}
            fill="none"
            stroke="#6366f1"
            strokeWidth="2"
            className="transition-opacity hover:opacity-70"
          />
          <circle
            cx={toX}
            cy={toY}
            r="4"
            fill="#6366f1"
            className="cursor-pointer hover:fill-red-500"
            onClick={() => {
              const newConns = connections.filter((c) => c.id !== conn.id);
              onConnectionsChange(newConns);
            }}
          />
        </g>
      );
    });
  };

  // Keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Delete" && selectedNode) {
        deleteNode(selectedNode);
      } else if (e.key === "Escape") {
        setSelectedNode(null);
        setConnecting(null);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [selectedNode, deleteNode]);

  return (
    <div className="flex h-full gap-0 select-none">
      {/* Node Palette */}
      <aside className="w-64 flex-shrink-0 overflow-y-auto border-r border-slate-700 bg-slate-900/90 p-3">
        <h3 className="mb-3 text-xs font-semibold uppercase text-slate-400">Nodes</h3>

        {categories.map((cat) => (
          <div key={cat.id} className="mb-4">
            <h4 className="mb-2 flex items-center gap-1 text-xs font-medium text-slate-500 uppercase">
              <span>{cat.icon}</span> {cat.label}
            </h4>
            <div className="flex flex-col gap-1">
              {allNodeTypes.filter((n) => n.category === cat.id).map((nodeType) => (
                <button
                  key={nodeType.type}
                  draggable
                  onDragStart={(e) => {
                    e.dataTransfer.setData("nodeType", nodeType.type);
                  }}
                  onClick={() => addNode(nodeType.type, 400, 300)}
                  className="flex items-center gap-2 rounded-lg border border-slate-700/50 bg-slate-800/50 p-2 text-left text-xs transition hover:border-slate-600 hover:bg-slate-800"
                >
                  <div
                    className="flex h-6 w-6 flex-shrink-0 items-center justify-center rounded text-xs font-bold text-white"
                    style={{ backgroundColor: nodeType.color }}
                  >
                    {nodeType.icon}
                  </div>
                  <div className="min-w-0">
                    <p className="truncate font-medium text-slate-200">{nodeType.label}</p>
                    <p className="truncate text-[10px] text-slate-500">{nodeType.description}</p>
                  </div>
                </button>
              ))}
            </div>
          </div>
        ))}
      </aside>

      {/* Canvas */}
      <div
        className="flex-1 relative overflow-hidden bg-slate-950 cursor-grab active:cursor-grabbing"
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseUp}
        onMouseDown={handleCanvasMouseDown}
      >
        {/* Grid Background */}
        <div
          className="absolute inset-0 pointer-events-none"
          style={{
            backgroundImage: `radial-gradient(circle, #334155 1px, transparent 1px)`,
            backgroundSize: `${20 * zoom}px ${20 * zoom}px`,
            backgroundPosition: `${canvasOffset.x}px ${canvasOffset.y}px`,
          }}
        />

        {/* SVG for connections */}
        <svg className="pointer-events-none absolute inset-0 h-full w-full">
          <g transform={`translate(${canvasOffset.x}, ${canvasOffset.y}) scale(${zoom})`}>
            {renderConnections()}
          </g>
        </svg>

        {/* Nodes */}
        <div
          className="absolute inset-0 pointer-events-none"
          style={{
            transform: `translate(${canvasOffset.x}px, ${canvasOffset.y}px) scale(${zoom})`,
            transformOrigin: "0 0",
          }}
        >
          {nodes.map((node) => {
            const nodeType = getNodeType(node.type);
            if (!nodeType) return null;

            return (
              <div
                key={node.id}
                className="pointer-events-auto absolute cursor-move rounded-lg border-2 bg-slate-900 shadow-lg transition-shadow hover:shadow-xl"
                style={{
                  left: node.x,
                  top: node.y,
                  width: 180,
                  borderColor: selectedNode === node.id ? "#3b82f6" : "#334155",
                  boxShadow: selectedNode === node.id ? "0 0 20px rgba(59, 130, 246, 0.3)" : undefined,
                }}
                onMouseDown={(e) => {
                  e.stopPropagation();
                  handleDragStart(e, node.id);
                }}
              >
                {/* Header */}
                <div
                  className="flex items-center gap-2 rounded-t-lg px-3 py-2"
                  style={{ backgroundColor: nodeType.color }}
                >
                  <span className="text-sm font-bold text-white">{nodeType.icon}</span>
                  <span className="truncate text-sm font-medium text-white">{node.label}</span>
                </div>

                {/* Body */}
                <div className="p-2">
                  <p className="truncate text-xs text-slate-400">{nodeType.description}</p>
                </div>

                {/* Input Ports */}
                {nodeType.inputs > 0 && (
                  <div className="absolute -left-1.5 top-1/2 flex -translate-y-1/2 flex-col gap-1">
                    {Array.from({ length: nodeType.inputs }).map((_, i) => (
                      <div
                        key={`input-${i}`}
                        className={`h-3.5 w-3.5 cursor-crosshair rounded-full border-2 border-slate-600 bg-slate-800 transition hover:border-sky-400 hover:bg-sky-500/30 ${
                          connecting ? "animate-pulse" : ""
                        }`}
                        onClick={(e) => {
                          e.stopPropagation();
                          endConnection(node.id, i);
                        }}
                      />
                    ))}
                  </div>
                )}

                {/* Output Ports */}
                {nodeType.outputs > 0 && (
                  <div className="absolute -right-1.5 top-1/2 flex -translate-y-1/2 flex-col gap-1">
                    {Array.from({ length: nodeType.outputs }).map((_, i) => (
                      <div
                        key={`output-${i}`}
                        className="h-3.5 w-3.5 cursor-crosshair rounded-full border-2 border-slate-600 bg-slate-800 transition hover:border-sky-400 hover:bg-sky-500/30"
                        onClick={(e) => {
                          e.stopPropagation();
                          startConnection(node.id, i);
                        }}
                      />
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>

        {/* Mini Map */}
        <div className="absolute bottom-4 right-4 h-24 w-32 rounded-lg border border-slate-700 bg-slate-900/80 overflow-hidden">
          <div className="relative h-full w-full">
            {nodes.map((node) => (
              <div
                key={node.id}
                className="absolute rounded-sm"
                style={{
                  left: (node.x / 20) + 50,
                  top: (node.y / 20) + 30,
                  width: 8,
                  height: 4,
                  backgroundColor: getNodeType(node.type)?.color || "#6366f1",
                }}
              />
            ))}
          </div>
        </div>

        {/* Toolbar */}
        <div className="absolute bottom-4 left-4 flex items-center gap-2">
          <button
            className="rounded-lg border border-slate-700 bg-slate-800/80 px-2 py-1 text-xs text-slate-300 hover:bg-slate-700 backdrop-blur"
            onClick={() => setZoom(Math.max(0.25, zoom - 0.25))}
          >
            −
          </button>
          <span className="rounded-lg border border-slate-700 bg-slate-800/80 px-2 py-1 text-xs text-slate-300 backdrop-blur">
            {Math.round(zoom * 100)}%
          </span>
          <button
            className="rounded-lg border border-slate-700 bg-slate-800/80 px-2 py-1 text-xs text-slate-300 hover:bg-slate-700 backdrop-blur"
            onClick={() => setZoom(Math.min(2, zoom + 0.25))}
          >
            +
          </button>
          <button
            className="rounded-lg border border-slate-700 bg-slate-800/80 px-2 py-1 text-xs text-slate-300 hover:bg-slate-700 backdrop-blur"
            onClick={() => { setZoom(1); setCanvasOffset({ x: 0, y: 0 }); }}
          >
            Reset
          </button>
        </div>
      </div>

      {/* Properties Panel */}
      {selectedNode && (
        <aside className="w-72 flex-shrink-0 border-l border-slate-700 bg-slate-900/90 p-3">
          <h3 className="mb-3 text-sm font-semibold text-slate-200">Properties</h3>
          {(() => {
            const node = nodes.find((n) => n.id === selectedNode);
            const nodeType = node ? getNodeType(node.type) : null;
            if (!node || !nodeType) return null;

            return (
              <div className="space-y-3">
                <div>
                  <label className="block text-xs font-medium text-slate-400">Label</label>
                  <input
                    type="text"
                    value={node.label}
                    onChange={(e) => {
                      const newNodes = nodes.map((n) =>
                        n.id === selectedNode ? { ...n, label: e.target.value } : n
                      );
                      onNodesChange(newNodes);
                    }}
                    className="mt-1 w-full rounded border border-slate-700 bg-slate-800 px-2 py-1.5 text-sm text-slate-200 focus:border-sky-500 focus:outline-none"
                  />
                </div>

                <div>
                  <label className="block text-xs font-medium text-slate-400">Type</label>
                  <p className="mt-1 text-sm text-slate-200">{nodeType.label}</p>
                </div>

                <div className="rounded border border-slate-700 bg-slate-800/50 p-2">
                  <p className="text-xs text-slate-500">
                    Position: {Math.round(node.x)}, {Math.round(node.y)}
                  </p>
                </div>

                <div className="pt-2">
                  <button
                    onClick={() => deleteNode(selectedNode)}
                    className="w-full rounded border border-red-700 bg-red-700/20 px-3 py-1.5 text-sm text-red-400 hover:bg-red-700/30"
                  >
                    Delete Node
                  </button>
                </div>
              </div>
            );
          })()}
        </aside>
      )}
    </div>
  );
}
