import { describe, expect, it } from "vitest";
import type { Edge, Node } from "@xyflow/react";
import {
  parseCanvasImport,
  removeEdgeById,
  removeNodeAndCascadeEdges,
  resolveDependencyIds,
  resolveDependencyNames,
  stepAgentPayload,
} from "./canvas";

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
    expect(next.nodes).toBe(nodes);
    expect(next.edges).toBe(edges);
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

describe("removeEdgeById", () => {
  it("returns a new array when the edge exists", () => {
    const edges = [mkEdge("e1", "a", "b"), mkEdge("e2", "b", "c")];

    const next = removeEdgeById(edges, "e1");

    expect(next.map((edge) => edge.id)).toEqual(["e2"]);
    expect(next).not.toBe(edges);
  });

  it("preserves the input reference when the edge id is stale", () => {
    const edges = [mkEdge("e1", "a", "b")];

    expect(removeEdgeById(edges, "missing")).toBe(edges);
  });
});

describe("parseCanvasImport", () => {
  it("accepts a complete exported canvas", () => {
    const imported = parseCanvasImport({
      nodes: [mkNode("a"), mkNode("b")],
      edges: [mkEdge("e1", "a", "b")],
      name: "Pipeline",
      description: "Imported workflow",
    });

    expect(imported.nodes.map((node) => node.id)).toEqual(["a", "b"]);
    expect(imported.edges.map((edge) => edge.id)).toEqual(["e1"]);
    expect(imported.name).toBe("Pipeline");
  });

  it("rejects missing arrays and malformed nodes", () => {
    expect(() => parseCanvasImport({ nodes: [] })).toThrow();
    expect(() => parseCanvasImport({
      nodes: [{ id: "a", position: { x: "0", y: 0 }, data: {} }],
      edges: [],
    })).toThrow();
    expect(() => parseCanvasImport({
      nodes: [{ id: "a", position: { x: 0, y: 0 }, data: { label: { unsafe: true } } }],
      edges: [],
    })).toThrow();
    expect(() => parseCanvasImport({
      nodes: [{ id: "a", position: { x: 0, y: 0 }, data: { dependsOn: [42] } }],
      edges: [],
    })).toThrow();
  });

  it("rejects duplicate IDs and orphaned edges", () => {
    expect(() => parseCanvasImport({ nodes: [mkNode("a"), mkNode("a")], edges: [] })).toThrow();
    expect(() => parseCanvasImport({
      nodes: [mkNode("a")],
      edges: [mkEdge("e1", "a", "missing")],
    })).toThrow();
  });

  it("rejects malformed React Flow state and self-references", () => {
    expect(() => parseCanvasImport({
      nodes: [{ ...mkNode("a"), hidden: "yes" }],
      edges: [],
    })).toThrow();
    expect(() => parseCanvasImport({
      nodes: [mkNode("a"), mkNode("b")],
      edges: [{ ...mkEdge("e1", "a", "b"), data: { _origSource: 42 } }],
    })).toThrow();
    expect(() => parseCanvasImport({
      nodes: [mkNode("a")],
      edges: [mkEdge("e1", "a", "a")],
    })).toThrow();
    expect(() => parseCanvasImport({
      nodes: [mkNode("a"), mkNode("b")],
      edges: [{ ...mkEdge("e1", "a", "b"), data: { _origSource: "missing" } }],
    })).toThrow();
    expect(() => parseCanvasImport({
      nodes: [{ ...mkNode("a"), parentId: "missing" }],
      edges: [],
    })).toThrow();
  });

  it("rejects unknown, ambiguous, and self dependency references", () => {
    expect(() => parseCanvasImport({
      nodes: [{ ...mkNode("a"), data: { label: "A", dependsOn: ["missing"] } }],
      edges: [],
    })).toThrow();
    expect(() => parseCanvasImport({
      nodes: [
        { ...mkNode("a"), data: { label: "Duplicate" } },
        { ...mkNode("b"), data: { label: "Duplicate" } },
        { ...mkNode("c"), data: { label: "C", dependsOn: ["Duplicate"] } },
      ],
      edges: [],
    })).toThrow();
    expect(() => parseCanvasImport({
      nodes: [{ ...mkNode("a"), data: { label: "A", dependsOn: ["a"] } }],
      edges: [],
    })).toThrow();
  });

  it("migrates valid legacy step dependencies and clears runtime state", () => {
    const imported = parseCanvasImport({
      nodes: [
        { ...mkNode("a"), data: { label: "Collect", agentId: "agent-a", _runState: "done" } },
        { ...mkNode("b"), data: { label: "Summarize", agentId: "agent-b", dependsOn: ["Collect"] } },
      ],
      edges: [],
    });

    expect(imported.nodes[0].data._runState).toBeUndefined();
    expect(imported.nodes[1].data.dependsOn).toEqual(["a"]);
  });
});

describe("canvas dependency references", () => {
  const options = [
    { id: "node-a", label: "Collect" },
    { id: "node-b", label: "Summarize" },
  ];

  it("stores current and unambiguous legacy dependencies as node IDs", () => {
    expect(resolveDependencyIds(["node-a", "Summarize", "missing"], options)).toEqual(["node-a", "node-b"]);
  });

  it("does not guess when a legacy label is ambiguous", () => {
    expect(resolveDependencyIds(["Duplicate"], [
      { id: "node-a", label: "Duplicate" },
      { id: "node-b", label: "Duplicate" },
    ])).toEqual([]);
  });

  it("resolves a stored ID to the node's latest label for the workflow API", () => {
    expect(resolveDependencyNames(["node-a"], [
      { id: "node-a", label: "Collect renamed" },
      options[1],
    ])).toEqual(["Collect renamed"]);
  });

  it("does not preserve ambiguous legacy labels when building the workflow API", () => {
    expect(resolveDependencyNames(["Duplicate"], [
      { id: "node-a", label: "Duplicate" },
      { id: "node-b", label: "Duplicate" },
    ])).toEqual([]);
  });
});

describe("stepAgentPayload", () => {
  it("sends only agent_id when a node is bound to a concrete instance", () => {
    // A node carries the name alongside the id purely so the card can render
    // it; sending both is the ambiguous payload the API rejects.
    expect(stepAgentPayload({ agentId: "abc", agentName: "researcher" })).toEqual({
      agent_id: "abc",
    });
  });

  it("sends agent_type when the node is bound to a type", () => {
    expect(stepAgentPayload({ agentType: "researcher" })).toEqual({
      agent_type: "researcher",
    });
  });

  it("prefers a concrete instance over a stale type binding", () => {
    expect(stepAgentPayload({ agentId: "abc", agentType: "researcher" })).toEqual({
      agent_id: "abc",
    });
  });

  it("falls back to agent_name when there is no id or type", () => {
    expect(stepAgentPayload({ agentName: "researcher" })).toEqual({
      agent_name: "researcher",
    });
  });

  it("returns null for an unbound node so the caller can drop the step", () => {
    expect(stepAgentPayload({ label: "unbound" })).toBeNull();
  });

  it("lets an explicit source outrank the specificity fallback", () => {
    // The fallback binds the most specific field present; a recorded source
    // (#7724) is the operator's actual choice and wins over it, which is what
    // makes leaving an id or a type binding possible at all.
    expect(stepAgentPayload({ agentSource: "name", agentId: "abc", agentName: "researcher" })).toEqual({
      agent_name: "researcher",
    });
    expect(stepAgentPayload({ agentSource: "type", agentId: "abc", agentType: "researcher" })).toEqual({
      agent_type: "researcher",
    });
    expect(stepAgentPayload({ agentSource: "instance", agentId: "abc", agentType: "researcher" })).toEqual({
      agent_id: "abc",
    });
  });

  it("falls back to specificity when the recorded source has no value", () => {
    // A half-authored node must not silently drop the binding its card is
    // still showing — that is how a workflow round-trips as zero steps.
    expect(stepAgentPayload({ agentSource: "name", agentName: "  ", agentType: "researcher" })).toEqual({
      agent_type: "researcher",
    });
  });

  it("treats a blank binding field as no binding", () => {
    expect(stepAgentPayload({ agentId: " ", agentName: "\t", agentType: "" })).toBeNull();
  });

  it("never returns more than one routing key", () => {
    const payload = stepAgentPayload({
      agentId: "abc",
      agentName: "researcher",
      agentType: "researcher",
    });
    expect(Object.keys(payload ?? {})).toHaveLength(1);
  });
});
