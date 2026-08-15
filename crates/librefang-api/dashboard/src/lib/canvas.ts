/**
 * Pure helpers for the workflow canvas (`pages/CanvasPage.tsx`).
 *
 * Kept as a tiny pure module so the cascade-delete logic can be unit-tested
 * without spinning up the ~2600-line CanvasPage component / xyflow runtime.
 */
import type { Edge, Node } from "@xyflow/react";

export type CanvasNodeData = {
  nodeType?: string;
  label?: string;
  name?: string;
  description?: string;
  agentId?: string;
  agentName?: string;
  prompt?: string;
  timeoutSecs?: number;
  maxRetries?: number;
  errorMode?: string;
  outputVar?: string;
  stepMode?: string;
  condition?: string;
  maxIterations?: number;
  until?: string;
  dependsOn?: string[];
  _runState?: string;
  _expanded?: boolean;
  _childCount?: number;
  _childIds?: string[];
  _origWidth?: number | string;
  _origHeight?: number | string;
  _groupId?: string;
  _onToggle?: (id: string) => void;
  _onUngroup?: (id: string) => void;
  _onDeleteGroup?: (id: string) => void;
  nodes?: CanvasNode[];
  edges?: Edge[];
  _origSource?: string;
  _origTarget?: string;
  [key: string]: unknown;
};

export type CanvasNode = Node<CanvasNodeData>;

export type CanvasImport = {
  nodes: CanvasNode[];
  edges: Edge[];
  name?: string;
  description?: string;
};

type DependencyOption = { id: string; label: string };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isCanvasNodeData(value: unknown, depth: number): value is CanvasNodeData {
  if (!isRecord(value) || depth > 20) return false;
  const stringFields = [
    "nodeType", "label", "name", "description", "agentId", "agentName", "prompt",
    "errorMode", "outputVar", "stepMode", "condition", "until", "_runState", "_groupId",
    "_origSource", "_origTarget",
  ];
  if (stringFields.some((field) => value[field] !== undefined && typeof value[field] !== "string")) return false;

  const numberFields = ["timeoutSecs", "maxRetries", "maxIterations", "_childCount"];
  if (numberFields.some((field) => value[field] !== undefined
    && (typeof value[field] !== "number" || !Number.isFinite(value[field])))) return false;

  const booleanFields = ["_expanded"];
  if (booleanFields.some((field) => value[field] !== undefined && typeof value[field] !== "boolean")) return false;

  for (const field of ["dependsOn", "_childIds"]) {
    const fieldValue = value[field];
    if (fieldValue !== undefined
      && (!Array.isArray(fieldValue) || fieldValue.some((item) => typeof item !== "string"))) return false;
  }

  for (const field of ["_origWidth", "_origHeight"]) {
    const fieldValue = value[field];
    if (fieldValue !== undefined
      && typeof fieldValue !== "string"
      && (typeof fieldValue !== "number" || !Number.isFinite(fieldValue))) return false;
  }

  if (value.nodes !== undefined
    && (!Array.isArray(value.nodes) || !value.nodes.every((node) => isCanvasNode(node, depth + 1)))) return false;
  if (value.edges !== undefined
    && (!Array.isArray(value.edges) || !value.edges.every(isCanvasEdge))) return false;

  // Callback overlays are reconstructed by the page and must never come from JSON.
  return value._onToggle === undefined
    && value._onUngroup === undefined
    && value._onDeleteGroup === undefined;
}

function isCanvasNode(value: unknown, depth = 0): value is CanvasNode {
  if (!isRecord(value) || typeof value.id !== "string" || value.id.trim() === "") return false;
  if (!isRecord(value.position)
    || typeof value.position.x !== "number" || !Number.isFinite(value.position.x)
    || typeof value.position.y !== "number" || !Number.isFinite(value.position.y)) return false;
  if (!isCanvasNodeData(value.data, depth)) return false;
  if (value.type !== undefined && typeof value.type !== "string") return false;
  for (const field of ["hidden", "selected", "draggable", "selectable", "connectable", "deletable", "focusable", "expandParent"]) {
    if (value[field] !== undefined && typeof value[field] !== "boolean") return false;
  }
  for (const field of ["width", "height", "initialWidth", "initialHeight", "zIndex"]) {
    if (value[field] !== undefined
      && (typeof value[field] !== "number" || !Number.isFinite(value[field]))) return false;
  }
  for (const field of ["parentId", "dragHandle", "ariaLabel", "className"]) {
    if (value[field] !== undefined && typeof value[field] !== "string") return false;
  }
  if (value.style !== undefined && !isRecord(value.style)) return false;
  if (value.measured !== undefined && (!isRecord(value.measured)
    || [value.measured.width, value.measured.height].some((size) =>
      size !== undefined && (typeof size !== "number" || !Number.isFinite(size))))) return false;
  return true;
}

function isCanvasEdge(value: unknown): value is Edge {
  if (!isRecord(value)
    || typeof value.id !== "string" || value.id.trim() === ""
    || typeof value.source !== "string" || value.source.trim() === ""
    || typeof value.target !== "string" || value.target.trim() === "") return false;
  for (const field of ["animated", "hidden", "selected", "selectable", "deletable", "focusable", "reconnectable"]) {
    if (value[field] !== undefined && typeof value[field] !== "boolean") return false;
  }
  for (const field of ["zIndex", "interactionWidth"]) {
    if (value[field] !== undefined
      && (typeof value[field] !== "number" || !Number.isFinite(value[field]))) return false;
  }
  for (const field of ["type", "sourceHandle", "targetHandle", "className", "ariaLabel"]) {
    if (value[field] !== undefined && value[field] !== null && typeof value[field] !== "string") return false;
  }
  for (const field of ["style", "labelStyle", "data"]) {
    if (value[field] !== undefined && !isRecord(value[field])) return false;
  }
  if (isRecord(value.data)) {
    for (const field of ["_origSource", "_origTarget"]) {
      if (value.data[field] !== undefined && typeof value.data[field] !== "string") return false;
    }
  }
  return true;
}

/** Validate an exported workflow before allowing it into React Flow state. */
export function parseCanvasImport(value: unknown): CanvasImport {
  if (!isRecord(value) || !Array.isArray(value.nodes) || !Array.isArray(value.edges)) {
    throw new Error("Canvas import must contain node and edge arrays");
  }
  if (!value.nodes.every(isCanvasNode) || !value.edges.every(isCanvasEdge)) {
    throw new Error("Canvas import contains an invalid node or edge");
  }
  if (value.name !== undefined && typeof value.name !== "string") {
    throw new Error("Canvas import name must be a string");
  }
  if (value.description !== undefined && typeof value.description !== "string") {
    throw new Error("Canvas import description must be a string");
  }

  const nodeIds = new Set(value.nodes.map((node) => node.id));
  const edgeIds = new Set(value.edges.map((edge) => edge.id));
  if (nodeIds.size !== value.nodes.length || edgeIds.size !== value.edges.length) {
    throw new Error("Canvas import contains duplicate IDs");
  }
  if (value.edges.some((edge) => !nodeIds.has(edge.source) || !nodeIds.has(edge.target))) {
    throw new Error("Canvas import contains an edge with an unknown endpoint");
  }
  if (value.edges.some((edge) => edge.source === edge.target)) {
    throw new Error("Canvas import contains a self-referencing edge");
  }

  const labelToId = new Map<string, string | null>();
  for (const node of value.nodes) {
    const label = node.data.label;
    if (!label) continue;
    labelToId.set(label, labelToId.has(label) ? null : node.id);
  }
  for (const node of value.nodes) {
    if (node.data._groupId && !nodeIds.has(node.data._groupId)) {
      throw new Error("Canvas import contains an unknown group reference");
    }
    if (node.data._childIds?.some((id) => id === node.id || !nodeIds.has(id))) {
      throw new Error("Canvas import contains an invalid group child reference");
    }
    if (node.data.dependsOn?.some((dependency) => {
      const targetId = nodeIds.has(dependency) ? dependency : labelToId.get(dependency);
      return !targetId || targetId === node.id;
    })) {
      throw new Error("Canvas import contains an invalid dependency reference");
    }
  }

  return {
    nodes: value.nodes,
    edges: value.edges,
    name: value.name,
    description: value.description,
  };
}

/** Convert current IDs and unambiguous legacy labels into stable dependency IDs. */
export function resolveDependencyIds(dependencies: string[], options: DependencyOption[]): string[] {
  const ids = new Set(options.map((option) => option.id));
  const labelToId = new Map<string, string | null>();
  for (const option of options) {
    labelToId.set(option.label, labelToId.has(option.label) ? null : option.id);
  }

  const resolved = dependencies.flatMap((dependency) => {
    if (ids.has(dependency)) return [dependency];
    const id = labelToId.get(dependency);
    return id ? [id] : [];
  });
  return [...new Set(resolved)];
}

/** Resolve stable canvas IDs to the current names required by the workflow API. */
export function resolveDependencyNames(dependencies: string[], options: DependencyOption[]): string[] {
  const idToLabel = new Map(options.map((option) => [option.id, option.label]));
  const labels = new Set(options.map((option) => option.label));
  const resolved = dependencies.flatMap((dependency) => {
    const currentLabel = idToLabel.get(dependency);
    if (currentLabel) return [currentLabel];
    return labels.has(dependency) ? [dependency] : [];
  });
  return [...new Set(resolved)];
}

/**
 * Remove a node by id and cascade-remove any edge that referenced it.
 *
 * Mirrors xyflow's built-in Backspace path (`applyNodeChanges` removes the
 * node and `onNodesChange` then signals connected edges for removal). The
 * context-menu delete must do the same thing — otherwise orphaned edges
 * remain in graph state pointing at a node that no longer exists.
 *
 * Returns new node/edge arrays when anything is removed. Unknown node ids
 * preserve both input references so callers can recognize a no-op.
 */
export function removeNodeAndCascadeEdges<N extends Node, E extends Edge>(
  nodes: N[],
  edges: E[],
  nodeId: string,
): { nodes: N[]; edges: E[] } {
  const nextNodes = nodes.filter((n) => n.id !== nodeId);
  const nextEdges = edges.filter((e) => e.source !== nodeId && e.target !== nodeId);

  return {
    nodes: nextNodes.length === nodes.length ? nodes : nextNodes,
    edges: nextEdges.length === edges.length ? edges : nextEdges,
  };
}

/** Remove an edge while preserving the input reference when its id is stale. */
export function removeEdgeById<E extends Edge>(edges: E[], edgeId: string): E[] {
  const nextEdges = edges.filter((edge) => edge.id !== edgeId);
  return nextEdges.length === edges.length ? edges : nextEdges;
}
