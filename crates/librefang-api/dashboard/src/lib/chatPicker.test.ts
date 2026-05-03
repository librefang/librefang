import { describe, expect, it } from "vitest";
import type { AgentItem, HandInstanceItem } from "../api";
import { groupedPicker } from "./chatPicker";

// Minimal AgentItem factory — only fills fields the picker logic reads.
// The `as AgentItem` cast is intentional: AgentItem has many fields the
// grouping logic does not touch, and the goal is to keep tests focused.
function agent(
  id: string,
  name: string,
  overrides: Partial<AgentItem> = {},
): AgentItem {
  return {
    id,
    name,
    state: "Running",
    mode: "interactive",
    created_at: "2026-04-14T00:00:00Z",
    last_active: "2026-04-14T00:00:00Z",
    model_provider: "test",
    model_name: "test-model",
    model_tier: "standard",
    auth_status: "configured",
    ready: true,
    is_hand: false,
    ...overrides,
  } as AgentItem;
}

function hand(
  instance_id: string,
  hand_id: string,
  hand_name: string,
  agent_ids: Record<string, string>,
  coordinator_role: string | undefined,
  overrides: Partial<HandInstanceItem> = {},
): HandInstanceItem {
  return {
    instance_id,
    hand_id,
    hand_name,
    hand_icon: "🔧",
    status: "Active",
    agent_ids,
    coordinator_role,
    activated_at: "2026-04-14T00:00:00Z",
    ...overrides,
  };
}

describe("groupedPicker", () => {
  it("short-circuits to flat standalone when showHandAgents is false", () => {
    const agents = [
      agent("a1", "general"),
      agent("a2", "hand-spawned", { is_hand: true }),
    ];
    const hands: HandInstanceItem[] = [
      hand("inst-1", "code-review", "Code Review", { main: "a2" }, "main"),
    ];

    const result = groupedPicker(agents, hands, false);

    expect(result.standalone).toEqual([agents[0]]);
    expect(result.handGroups).toEqual([]);
  });

  it("returns all standalone agents when there are no hand instances", () => {
    const agents = [agent("a1", "general"), agent("a2", "researcher")];
    const result = groupedPicker(agents, [], true);

    expect(result.standalone.map((a) => a.id)).toEqual(["a1", "a2"]);
    expect(result.handGroups).toEqual([]);
  });

  // Issue #4296 Bug A: while the hands query is still loading
  // (`handInstances === undefined`), is_hand agents must remain visible so
  // a URL-pinned hand-agent selection isn't silently dropped during the
  // bootstrap window. Once the query resolves with `[]` (paused / no active
  // hands), the existing semantics still drop them.
  it("keeps is_hand agents visible while handInstances is loading (undefined)", () => {
    const agents = [
      agent("a1", "general"),
      agent("hand-main", "main", { is_hand: true }),
    ];

    const result = groupedPicker(agents, undefined, true);

    expect(result.standalone.map((a) => a.id)).toEqual(["a1", "hand-main"]);
    expect(result.handGroups).toEqual([]);
  });

  it("drops is_hand agents once handInstances resolves to [] (paused hands)", () => {
    const agents = [
      agent("a1", "general"),
      agent("hand-main", "main", { is_hand: true }),
    ];

    const result = groupedPicker(agents, [], true);

    expect(result.standalone.map((a) => a.id)).toEqual(["a1"]);
    expect(result.handGroups).toEqual([]);
  });

  it("groups a single-agent hand under one header", () => {
    const agents = [
      agent("a1", "general"),
      agent("hand-main", "main", { is_hand: true }),
    ];
    const hands = [
      hand(
        "inst-1",
        "code-review",
        "Code Review",
        { main: "hand-main" },
        "main",
      ),
    ];

    const result = groupedPicker(agents, hands, true);

    expect(result.standalone.map((a) => a.id)).toEqual(["a1"]);
    expect(result.handGroups).toHaveLength(1);
    expect(result.handGroups[0].hand_name).toBe("Code Review");
    expect(result.handGroups[0].agents).toHaveLength(1);
    expect(result.handGroups[0].agents[0].id).toBe("hand-main");
    expect(result.handGroups[0].agents[0].role).toBe("main");
    expect(result.handGroups[0].agents[0].isCoordinator).toBe(true);
  });

  it("orders multi-role hand agents with coordinator first, others alphabetical", () => {
    const agents = [
      agent("hand-main", "main", { is_hand: true }),
      agent("hand-linter", "linter", { is_hand: true }),
      agent("hand-zsec", "zsec", { is_hand: true }),
    ];
    const hands = [
      hand(
        "inst-1",
        "code-review",
        "Code Review",
        { main: "hand-main", linter: "hand-linter", zsec: "hand-zsec" },
        "main",
      ),
    ];

    const result = groupedPicker(agents, hands, true);
    const roles = result.handGroups[0].agents.map((a) => a.role);

    // Coordinator first, then "linter" then "zsec" alphabetical.
    expect(roles).toEqual(["main", "linter", "zsec"]);
  });

  it("sorts hand groups alphabetically by hand_name", () => {
    const agents = [
      agent("a1", "research-main", { is_hand: true }),
      agent("a2", "code-main", { is_hand: true }),
    ];
    const hands = [
      hand("inst-1", "research", "Research", { main: "a1" }, "main"),
      hand("inst-2", "code-review", "Code Review", { main: "a2" }, "main"),
    ];

    const result = groupedPicker(agents, hands, true);

    expect(result.handGroups.map((g) => g.hand_name)).toEqual([
      "Code Review",
      "Research",
    ]);
  });

  it("hides hand instances with empty agent_ids", () => {
    const agents = [agent("a1", "general")];
    const hands = [
      hand("inst-1", "code-review", "Code Review", {}, undefined),
    ];

    const result = groupedPicker(agents, hands, true);
    expect(result.handGroups).toEqual([]);
    expect(result.standalone.map((a) => a.id)).toEqual(["a1"]);
  });

  it("hides hand instances whose status is not Active", () => {
    const agents = [
      agent("a1", "general"),
      agent("hand-main", "main", { is_hand: true }),
    ];
    const hands = [
      hand(
        "inst-1",
        "code-review",
        "Code Review",
        { main: "hand-main" },
        "main",
        { status: "Paused" },
      ),
    ];

    const result = groupedPicker(agents, hands, true);
    expect(result.handGroups).toEqual([]);
    // The hand-spawned agent does NOT fall back to standalone — we hide it
    // entirely because the user opted in to hand grouping and the hand is
    // unavailable.
    expect(result.standalone.map((a) => a.id)).toEqual(["a1"]);
  });

  it("falls back to alphabetical role order when coordinator_role is missing", () => {
    const agents = [
      agent("a1", "linter", { is_hand: true }),
      agent("a2", "main", { is_hand: true }),
    ];
    const hands = [
      hand(
        "inst-1",
        "code-review",
        "Code Review",
        { main: "a2", linter: "a1" },
        undefined,
      ),
    ];

    const result = groupedPicker(agents, hands, true);
    const roles = result.handGroups[0].agents.map((a) => a.role);
    const flags = result.handGroups[0].agents.map((a) => a.isCoordinator);

    expect(roles).toEqual(["linter", "main"]); // alphabetical
    expect(flags).toEqual([false, false]); // no coordinator marked
  });
});
