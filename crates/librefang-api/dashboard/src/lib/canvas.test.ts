import { describe, expect, it } from "vitest";
import type { Edge, Node } from "@xyflow/react";
import {
  parseCanvasImport,
  removeEdgeById,
  removeNodeAndCascadeEdges,
  resolveDependencyIds,
  resolveDependencyNames,
  workflowStepsToCanvasState,
} from "./canvas";
import { extractWorkflowJson } from "./chat";

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

// #6943 review: "Save as Workflow" must transform the raw workflow_create steps into canvas nodes/edges before the draft is handed to /canvas.
describe("workflowStepsToCanvasState", () => {
  it("lays out steps left-to-right and chains them with plain edges when no depends_on is present", () => {
    const { nodes, edges } = workflowStepsToCanvasState([
      { name: "fetch", prompt_template: "Fetch the data", agent: "researcher" },
      { name: "summarize", prompt_template: "Summarize {{input}}", agent: { id: "a-1", name: "writer" } },
    ]);

    expect(nodes).toHaveLength(2);
    expect(nodes[0]).toMatchObject({
      id: "node-0",
      type: "custom",
      position: { x: 80, y: 100 },
      data: { label: "fetch", prompt: "Fetch the data", nodeType: "agent", agentName: "researcher" },
    });
    expect(nodes[1]).toMatchObject({
      id: "node-1",
      position: { x: 340, y: 100 },
      data: { label: "summarize", agentId: "a-1", agentName: "writer" },
    });

    expect(edges).toEqual([{ id: "e-0", source: "node-0", target: "node-1" }]);
  });

  it("builds dashed depends-on edges instead of a linear chain when any step declares depends_on", () => {
    const { edges } = workflowStepsToCanvasState([
      { name: "fetch", prompt_template: "Fetch", agent: "researcher" },
      { name: "analyze", prompt_template: "Analyze", agent: "researcher", depends_on: ["fetch"] },
    ]);

    expect(edges).toEqual([
      {
        id: "dep-1-0",
        source: "node-0",
        target: "node-1",
        style: { strokeDasharray: "6 3" },
        label: "depends",
        labelStyle: { fontSize: 9, fill: "#6b7280" },
      },
    ]);
  });

  it("drops a depends_on entry naming an unknown or self step instead of emitting a dangling edge", () => {
    const { edges } = workflowStepsToCanvasState([
      { name: "fetch", prompt_template: "Fetch", agent: "researcher" },
      { name: "analyze", prompt_template: "Analyze", agent: "researcher", depends_on: ["nope", "analyze", "fetch"] },
    ]);
    expect(edges).toEqual([
      {
        id: "dep-1-2",
        source: "node-0",
        target: "node-1",
        style: { strokeDasharray: "6 3" },
        label: "depends",
        labelStyle: { fontSize: 9, fill: "#6b7280" },
      },
    ]);
  });

  it("falls back to positional labels and empty prompts for a bare step", () => {
    const { nodes } = workflowStepsToCanvasState([{}]);
    expect(nodes[0].data).toMatchObject({ label: "Step 1", prompt: "", nodeType: "agent" });
  });

  it("produces no edges for a single step", () => {
    const { nodes, edges } = workflowStepsToCanvasState([
      { name: "only-step", prompt_template: "Do it", agent: "assistant" },
    ]);
    expect(nodes).toHaveLength(1);
    expect(edges).toEqual([]);
  });

  it("produces no nodes or edges for an empty step list", () => {
    expect(workflowStepsToCanvasState([])).toEqual({ nodes: [], edges: [] });
  });

  // #6943 review — the full "Save as Workflow" chat action, composed end to end.
  // A chat message containing a workflow_create-shaped JSON blob (with trailing prose that used to break the greedy extraction regex) is parsed, transformed into canvas nodes/edges, and the result is fed through `parseCanvasImport` — the same structural validator the canvas page's own import path runs — to prove the hand-off produces a well-formed canvas rather than a shape that merely type-checks.
  it("round-trips a chat message into a canvas import CanvasPage can load", () => {
    const workflow = {
      name: "bug-triage",
      description: "Triage and summarize a bug report",
      steps: [
        { name: "fetch", prompt_template: "Fetch the report", agent: "researcher" },
        { name: "summarize", prompt_template: "Summarize {{input}}", agent: "writer", depends_on: ["fetch"] },
      ],
    };
    const content = `Sure, here's the workflow:\n\n${JSON.stringify(workflow)}\n\nLet me know if you'd like changes {here}`;

    const jsonText = extractWorkflowJson(content);
    expect(jsonText).not.toBeNull();
    const parsed = JSON.parse(jsonText!) as typeof workflow;
    expect(parsed).toEqual(workflow);

    const { nodes, edges } = workflowStepsToCanvasState(parsed.steps);
    const imported = parseCanvasImport({
      nodes,
      edges,
      name: parsed.name,
      description: parsed.description,
    });

    expect(imported.nodes.map((n) => n.id)).toEqual(["node-0", "node-1"]);
    expect(imported.nodes.map((n) => n.data.label)).toEqual(["fetch", "summarize"]);
    expect(imported.edges).toEqual([
      {
        id: "dep-1-0",
        source: "node-0",
        target: "node-1",
        style: { strokeDasharray: "6 3" },
        label: "depends",
        labelStyle: { fontSize: 9, fill: "#6b7280" },
      },
    ]);
  });
});
