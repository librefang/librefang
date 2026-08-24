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
  /** Agent *type* binding — the step resolves find-or-spawn from a template of
   *  this name rather than a pre-registered instance (#7712). Mutually
   *  exclusive with `agentId` / `agentName`; `stepAgentPayload` sends exactly one. */
  agentType?: string;
  /** Which binding the operator chose for this step's agent: a specific
   *  running instance (`agentId`), a durable agent name (`agentName`), or an
   *  agent type resolved find-or-spawn from a template (`agentType`).
   *  Stored explicitly rather than inferred from which field happens to be
   *  set, so switching between any two of the three is reversible in both
   *  directions. */
  agentSource?: "instance" | "name" | "type";
  /** Per-step `session_mode` override sent to the API: `"persistent"`,
   *  `"new"`, or absent to defer to the target agent's manifest. */
  sessionMode?: "persistent" | "new";
  /** Skill names this step's agent must be able to use (#7721).
   *  Absent — not `[]` — when the step requires nothing, which is what every step authored before this field existed looks like. */
  requiredSkills?: string[];
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
    "nodeType", "label", "name", "description", "agentId", "agentName", "agentType",
    "agentSource", "sessionMode", "prompt",
    "errorMode", "outputVar", "stepMode", "condition", "until", "_runState", "_groupId",
    "_origSource", "_origTarget",
  ];
  if (stringFields.some((field) => value[field] !== undefined && typeof value[field] !== "string")) return false;

  const numberFields = ["timeoutSecs", "maxRetries", "maxIterations", "_childCount"];
  if (numberFields.some((field) => value[field] !== undefined
    && (typeof value[field] !== "number" || !Number.isFinite(value[field])))) return false;

  const booleanFields = ["_expanded"];
  if (booleanFields.some((field) => value[field] !== undefined && typeof value[field] !== "boolean")) return false;

  for (const field of ["dependsOn", "_childIds", "requiredSkills"]) {
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

  for (const edge of value.edges) {
    if (isRecord(edge.data)) {
      for (const field of ["_origSource", "_origTarget"]) {
        const endpoint = edge.data[field];
        if (typeof endpoint === "string" && !nodeIds.has(endpoint)) {
          throw new Error("Canvas import contains an unknown restored edge endpoint");
        }
      }
    }
  }

  const stepNodes = value.nodes.filter((node) => stepAgentPayload(node.data) !== null);
  const dependencyOptions = stepNodes.map((node, index) => ({
    id: node.id,
    label: node.data.label || `Step ${index + 1}`,
  }));
  const stepIds = new Set(stepNodes.map((node) => node.id));
  const normalizedNodes = value.nodes.map((node) => {
    if (node.parentId && (node.parentId === node.id || !nodeIds.has(node.parentId))) {
      throw new Error("Canvas import contains an invalid parent reference");
    }
    if (node.data._groupId
      && (node.data._groupId === node.id || !nodeIds.has(node.data._groupId))) {
      throw new Error("Canvas import contains an invalid group reference");
    }
    if (node.data._childIds) {
      const childIds = new Set(node.data._childIds);
      if (childIds.size !== node.data._childIds.length
        || node.data._childIds.some((id) => id === node.id || !nodeIds.has(id))) {
        throw new Error("Canvas import contains an invalid group child reference");
      }
    }

    const dependencies = node.data.dependsOn;
    let resolvedDependencies: string[] | undefined;
    if (dependencies && dependencies.length > 0) {
      const uniqueDependencies = new Set(dependencies);
      resolvedDependencies = resolveDependencyIds(dependencies, dependencyOptions);
      if (!stepIds.has(node.id)
        || uniqueDependencies.size !== dependencies.length
        || resolvedDependencies.length !== dependencies.length
        || resolvedDependencies.includes(node.id)) {
        throw new Error("Canvas import contains an invalid dependency reference");
      }
    }

    return {
      ...node,
      data: {
        ...node.data,
        dependsOn: resolvedDependencies,
        _runState: undefined,
      },
    };
  });

  return {
    nodes: normalizedNodes,
    edges: value.edges,
    name: value.name,
    description: value.description,
  };
}

/** The routing key a workflow step sends to bind its agent. */
export type StepAgentPayload =
  | { agent_id: string }
  | { agent_type: string }
  | { agent_name: string };

/** A binding field's usable value: the trimmed string, or `""` when the field
 *  is absent, blank, or a non-string smuggled in through a hand-edited layout. */
function bindingField(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

/**
 * Pick the single agent routing key a canvas node contributes to a workflow
 * step payload (#7712).
 *
 * A node carries `agentId`, `agentName` and `agentType` together so it can
 * render a name next to the binding, but the workflow API accepts **exactly
 * one** of `agent_id` / `agent_name` / `agent_type` per step: a payload with
 * several is rejected rather than resolved by key precedence, because the step
 * would otherwise bind to whichever key the server read first while the canvas
 * kept showing the other one.
 *
 * `agentSource` records which of the three the operator actually chose (#7724),
 * so it decides whenever it is present and its own field is filled. A node
 * saved before the source was recorded falls back to precedence by specificity
 * — a concrete instance id, then a type, then a name — which is also the
 * fallback when an explicit source points at an empty field, so a binding the
 * card is still displaying can never be silently dropped from the saved
 * workflow.
 *
 * A node with no binding at all yields `null` so the caller can drop it rather
 * than send a step the API will refuse.
 */
export function stepAgentPayload(data: CanvasNodeData): StepAgentPayload | null {
  const agentId = bindingField(data.agentId);
  const agentType = bindingField(data.agentType);
  const agentName = bindingField(data.agentName);
  if (data.agentSource === "instance" && agentId) return { agent_id: agentId };
  if (data.agentSource === "type" && agentType) return { agent_type: agentType };
  if (data.agentSource === "name" && agentName) return { agent_name: agentName };
  if (agentId) return { agent_id: agentId };
  if (agentType) return { agent_type: agentType };
  if (agentName) return { agent_name: agentName };
  return null;
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
  const labelCounts = new Map<string, number>();
  for (const option of options) {
    labelCounts.set(option.label, (labelCounts.get(option.label) ?? 0) + 1);
  }
  const resolved = dependencies.flatMap((dependency) => {
    const currentLabel = idToLabel.get(dependency);
    if (currentLabel) return [currentLabel];
    return labelCounts.get(dependency) === 1 ? [dependency] : [];
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

/** Tailwind classes for the node config panel's controls.
 *
 * Exported so the panel and the step controls extracted out of it
 * (`components/StepAgentBinding.tsx`) stay visually identical from one
 * definition instead of two copies that drift.
 */
export const CANVAS_INPUT_CLASS =
  "mt-1 w-full rounded-lg border border-border-subtle bg-main px-2 py-1.5 text-xs outline-none focus:border-brand";
export const CANVAS_LABEL_CLASS = "text-[10px] font-bold text-text-dim uppercase";
