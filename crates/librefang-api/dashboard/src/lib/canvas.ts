/**
 * Pure helpers for the workflow canvas (`pages/CanvasPage.tsx`).
 *
 * Kept as a tiny pure module so the cascade-delete logic can be unit-tested
 * without spinning up the ~2600-line CanvasPage component / xyflow runtime.
 */
import type { Edge, Node } from "@xyflow/react";

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
