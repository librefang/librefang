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
 * Returns the new node/edge arrays; callers are responsible for any
 * surrounding `pushHistory()` so undo works.
 */
export function removeNodeAndCascadeEdges<N extends Node, E extends Edge>(
  nodes: readonly N[],
  edges: readonly E[],
  nodeId: string,
): { nodes: N[]; edges: E[] } {
  return {
    nodes: nodes.filter((n) => n.id !== nodeId),
    edges: edges.filter((e) => e.source !== nodeId && e.target !== nodeId),
  };
}
