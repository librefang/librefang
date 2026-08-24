/**
 * Tests for the workflow step's agent binding control (refs #7724).
 *
 * The defect this control is shaped to avoid: a single agent `<select>`
 * whose controlled value is forced to `""` while some other binding field
 * is set. That select can never emit the value that would clear the other
 * field, so once a step leaves the concrete-agent binding it can never come
 * back — a one-way door in a UI control. The tests below therefore walk the
 * switch in BOTH directions and assert the round trip is lossless.
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import {
  StepAgentBinding,
  bindingFromNodeData,
  bindingToNodeData,
  isStepBound,
  normalizeSessionMode,
  stepAgentFields,
  switchAgentSource,
  type StepAgentBindingValue,
} from "./StepAgentBinding";
import type { AgentItem } from "../api";
import type { CanvasNodeData } from "../lib/canvas";

const AGENTS = [
  { id: "id-research", name: "researcher", state: "Running" },
  { id: "id-writer", name: "writer", state: "Idle" },
] as unknown as AgentItem[];

/** The panel owns the binding state, so the harness does too. */
function Harness({
  initial,
  onValue,
}: {
  initial: StepAgentBindingValue;
  onValue: (v: StepAgentBindingValue) => void;
}) {
  const [value, setValue] = useState(initial);
  return (
    <StepAgentBinding
      value={value}
      agents={AGENTS}
      onChange={next => {
        setValue(next);
        onValue(next);
      }}
      t={(key: string) => key}
    />
  );
}

/** `Array.prototype.at` is past this project's TS lib target. */
function last(values: StepAgentBindingValue[]): StepAgentBindingValue {
  return values[values.length - 1];
}

const EMPTY: StepAgentBindingValue = {
  source: "instance",
  agentId: "",
  agentName: "",
  sessionMode: "",
};

describe("bindingFromNodeData", () => {
  it("infers the instance source when the step carries an agent id", () => {
    expect(bindingFromNodeData({ agentId: "id-research", agentName: "researcher" })).toEqual({
      source: "instance",
      agentId: "id-research",
      agentName: "researcher",
      sessionMode: "",
    });
  });

  it("infers the name source when only a name is set", () => {
    // This is how a workflow hydrated from `agent_name` steps arrives.
    expect(bindingFromNodeData({ agentName: "researcher" }).source).toBe("name");
  });

  it("honours an explicitly stored source over the inference", () => {
    expect(
      bindingFromNodeData({ agentSource: "name", agentId: "id-research", agentName: "researcher" })
        .source,
    ).toBe("name");
  });

  it("drops a session mode the API would not parse", () => {
    // A layout persisted by an older build, or hand-edited, can hold anything here.
    expect(bindingFromNodeData({ sessionMode: "bogus" } as unknown as CanvasNodeData).sessionMode).toBe("");
    expect(normalizeSessionMode(42)).toBe("");
    expect(normalizeSessionMode("new")).toBe("new");
  });
});

describe("bindingToNodeData", () => {
  it("emits only the id for an instance binding", () => {
    expect(
      bindingToNodeData(
        { source: "instance", agentId: "id-writer", agentName: "stale", sessionMode: "" },
        AGENTS,
      ),
    ).toEqual({
      agentSource: "instance",
      agentId: "id-writer",
      // Re-derived from the registry, so a stale echo cannot be saved.
      agentName: "writer",
      sessionMode: undefined,
    });
  });

  it("emits only the name for a name binding, dropping the stale id", () => {
    expect(
      bindingToNodeData(
        { source: "name", agentId: "id-writer", agentName: " researcher ", sessionMode: "new" },
        AGENTS,
      ),
    ).toEqual({
      agentSource: "name",
      agentId: undefined,
      agentName: "researcher",
      sessionMode: "new",
    });
  });
});

describe("stepAgentFields", () => {
  it("sends the id alone for an instance binding", () => {
    expect(
      stepAgentFields({ agentSource: "instance", agentId: "id-research", agentName: "researcher" }),
    ).toEqual({ agent_id: "id-research", agent_name: undefined, session_mode: undefined });
  });

  it("sends the name alone for a name binding, even with a stale id present", () => {
    expect(
      stepAgentFields({ agentSource: "name", agentId: "id-research", agentName: "researcher" }),
    ).toEqual({ agent_id: undefined, agent_name: "researcher", session_mode: undefined });
  });

  it("treats a node with only a name as a name binding", () => {
    // Nodes saved before the source was recorded, and every node hydrated
    // from an `agent_name` step, arrive without `agentSource`.
    expect(stepAgentFields({ agentName: "researcher" })).toMatchObject({
      agent_name: "researcher",
    });
  });

  it("carries the session override and omits it when unset", () => {
    expect(stepAgentFields({ agentId: "id-research", sessionMode: "new" }).session_mode).toBe("new");
    expect(stepAgentFields({ agentId: "id-research" }).session_mode).toBeUndefined();
  });
});

describe("isStepBound", () => {
  it("counts a name-only binding as bound", () => {
    expect(isStepBound({ agentName: "researcher" })).toBe(true);
  });
  it("does not count whitespace as a binding", () => {
    expect(isStepBound({ agentName: "   " })).toBe(false);
    expect(isStepBound({})).toBe(false);
  });
});

describe("switchAgentSource", () => {
  it("is a no-op when the source does not change", () => {
    const value: StepAgentBindingValue = { ...EMPTY, agentId: "id-writer" };
    expect(switchAgentSource(value, "instance", AGENTS)).toBe(value);
  });

  it("falls back to no agent when the typed name matches nothing live", () => {
    const value: StepAgentBindingValue = { ...EMPTY, source: "name", agentName: "not-spawned-yet" };
    expect(switchAgentSource(value, "instance", AGENTS)).toEqual({
      source: "instance",
      agentId: "",
      agentName: "not-spawned-yet",
      sessionMode: "",
    });
  });
});

describe("StepAgentBinding", () => {
  it("switches from a concrete agent to a name and back without losing the binding", async () => {
    const user = userEvent.setup();
    const seen: StepAgentBindingValue[] = [];
    render(<Harness initial={EMPTY} onValue={v => seen.push(v)} />);

    // 1. Bind to a concrete running agent.
    await user.selectOptions(screen.getByLabelText("canvas.assign_agent"), "id-research");
    expect(last(seen)).toMatchObject({ source: "instance", agentId: "id-research" });

    // 2. Switch to the name binding — the selected agent's name carries over.
    await user.selectOptions(screen.getByLabelText("canvas.agent_source_label"), "name");
    expect(last(seen)).toMatchObject({ source: "name", agentName: "researcher" });
    expect(screen.getByLabelText("canvas.agent_name_label")).toHaveValue("researcher");
    expect(screen.queryByLabelText("canvas.assign_agent")).toBeNull();

    // 3. Switch back. This is the direction the one-way door blocked: the
    //    concrete-agent select must reappear already reporting the agent.
    await user.selectOptions(screen.getByLabelText("canvas.agent_source_label"), "instance");
    expect(last(seen)).toMatchObject({ source: "instance", agentId: "id-research" });
    expect(screen.getByLabelText("canvas.assign_agent")).toHaveValue("id-research");

    // 4. And it is still a live control — a different agent can be chosen.
    await user.selectOptions(screen.getByLabelText("canvas.assign_agent"), "id-writer");
    expect(bindingToNodeData(last(seen), AGENTS)).toMatchObject({
      agentId: "id-writer",
      agentName: "writer",
    });
  });

  it("keeps a hand-typed name that no running agent answers to", async () => {
    const user = userEvent.setup();
    const seen: StepAgentBindingValue[] = [];
    render(<Harness initial={{ ...EMPTY, source: "name" }} onValue={v => seen.push(v)} />);

    await user.type(screen.getByLabelText("canvas.agent_name_label"), "nightly-auditor");
    expect(last(seen)).toMatchObject({ agentName: "nightly-auditor" });

    // Bouncing through the instance source leaves the typed name intact, so
    // the operator can go back to it rather than retyping.
    await user.selectOptions(screen.getByLabelText("canvas.agent_source_label"), "instance");
    expect(screen.getByLabelText("canvas.assign_agent")).toHaveValue("");
    await user.selectOptions(screen.getByLabelText("canvas.agent_source_label"), "name");
    expect(screen.getByLabelText("canvas.agent_name_label")).toHaveValue("nightly-auditor");
  });

  it("selects the per-run session mode and clears it again", async () => {
    const user = userEvent.setup();
    const seen: StepAgentBindingValue[] = [];
    render(<Harness initial={{ ...EMPTY, agentId: "id-research" }} onValue={v => seen.push(v)} />);

    const sessionSelect = screen.getByLabelText("canvas.session_mode_label");
    expect(sessionSelect).toHaveValue("");

    await user.selectOptions(sessionSelect, "new");
    expect(bindingToNodeData(last(seen), AGENTS).sessionMode).toBe("new");

    await user.selectOptions(sessionSelect, "persistent");
    expect(bindingToNodeData(last(seen), AGENTS).sessionMode).toBe("persistent");

    // Back to the manifest default: the field must be omitted, not sent as
    // an empty string the API would log and discard.
    await user.selectOptions(sessionSelect, "");
    expect(bindingToNodeData(last(seen), AGENTS).sessionMode).toBeUndefined();
  });

  it("offers every known agent name as a datalist suggestion", () => {
    render(<Harness initial={{ ...EMPTY, source: "name" }} onValue={vi.fn()} />);
    const input = screen.getByLabelText("canvas.agent_name_label");
    const list = document.getElementById(input.getAttribute("list")!);
    expect([...list!.querySelectorAll("option")].map(o => o.getAttribute("value"))).toEqual([
      "researcher",
      "writer",
    ]);
  });
});
