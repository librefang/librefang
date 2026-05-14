import { describe, expect, it } from "vitest";
import type { Edge, Node } from "@xyflow/react";
import { removeNodeAndCascadeEdges } from "./canvas";

type N = Node<{ label: string }>;
type E = Edge;

const mkNode = (id: string): N => ({
  id,
  position: { x: 0, y: 0 },
  data: { label: id },
});

const mkEdge = (id: string, source: string, target: string): E => ({
  id,
  source,
  target,
});

describe("removeNodeAndCascadeEdges", () => {
  it("removes the node by id", () => {
    const nodes = [mkNode("a"), mkNode("b"), mkNode("c")];
    const edges: E[] = [];
    const next = removeNodeAndCascadeEdges(nodes, edges, "b");
    expect(next.nodes.map((n) => n.id)).toEqual(["a", "c"]);
    expect(next.edges).toEqual([]);
  });

  it("cascades edges where source === deletedId", () => {
    const nodes = [mkNode("a"), mkNode("b")];
    const edges = [mkEdge("e1", "a", "b")];
    const next = removeNodeAndCascadeEdges(nodes, edges, "a");
    expect(next.nodes.map((n) => n.id)).toEqual(["b"]);
    expect(next.edges).toEqual([]);
  });

  it("cascades edges where target === deletedId", () => {
    const nodes = [mkNode("a"), mkNode("b")];
    const edges = [mkEdge("e1", "a", "b")];
    const next = removeNodeAndCascadeEdges(nodes, edges, "b");
    expect(next.nodes.map((n) => n.id)).toEqual(["a"]);
    expect(next.edges).toEqual([]);
  });

  it("keeps edges that do not touch the deleted node", () => {
    const nodes = [mkNode("a"), mkNode("b"), mkNode("c")];
    const edges = [mkEdge("e1", "a", "b"), mkEdge("e2", "b", "c")];
    // Delete 'a': e1 should drop (source=a), e2 should remain (b->c).
    const next = removeNodeAndCascadeEdges(nodes, edges, "a");
    expect(next.nodes.map((n) => n.id)).toEqual(["b", "c"]);
    expect(next.edges.map((e) => e.id)).toEqual(["e2"]);
  });

  it("cascades multiple edges sharing the deleted endpoint", () => {
    const nodes = [mkNode("a"), mkNode("b"), mkNode("c"), mkNode("d")];
    const edges = [
      mkEdge("e1", "a", "b"),
      mkEdge("e2", "c", "b"),
      mkEdge("e3", "b", "d"),
      mkEdge("e4", "a", "d"),
    ];
    // Delete 'b': e1, e2, e3 all reference it; e4 (a->d) survives.
    const next = removeNodeAndCascadeEdges(nodes, edges, "b");
    expect(next.nodes.map((n) => n.id)).toEqual(["a", "c", "d"]);
    expect(next.edges.map((e) => e.id)).toEqual(["e4"]);
  });

  it("is a no-op when the node id is unknown", () => {
    const nodes = [mkNode("a"), mkNode("b")];
    const edges = [mkEdge("e1", "a", "b")];
    const next = removeNodeAndCascadeEdges(nodes, edges, "missing");
    expect(next.nodes.map((n) => n.id)).toEqual(["a", "b"]);
    expect(next.edges.map((e) => e.id)).toEqual(["e1"]);
  });

  it("returns new arrays without mutating the inputs (so a prior snapshot survives for undo)", () => {
    const nodes = [mkNode("a"), mkNode("b")];
    const edges = [mkEdge("e1", "a", "b")];
    // Snapshot what pushHistory() would have captured before mutation.
    const snapshotNodes = [...nodes];
    const snapshotEdges = [...edges];

    const next = removeNodeAndCascadeEdges(nodes, edges, "a");

    // Helper produced the cascade.
    expect(next.nodes.map((n) => n.id)).toEqual(["b"]);
    expect(next.edges).toEqual([]);

    // Inputs are untouched — undo via the previous snapshot fully restores
    // both the node AND the connecting edge (the regression in #5001).
    expect(nodes).toEqual(snapshotNodes);
    expect(edges).toEqual(snapshotEdges);
    expect(snapshotNodes.map((n) => n.id)).toEqual(["a", "b"]);
    expect(snapshotEdges.map((e) => e.id)).toEqual(["e1"]);
  });
});
