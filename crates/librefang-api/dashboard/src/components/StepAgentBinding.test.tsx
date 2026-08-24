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
  formatRequiredSkills,
  isStepBound,
  normalizeSessionMode,
  parseRequiredSkills,
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
      skills={SKILLS}
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

const SKILLS = ["browser-automation", "pdf-extract"];

const EMPTY: StepAgentBindingValue = {
  source: "instance",
  agentId: "",
  agentName: "",
  agentType: "",
  sessionMode: "",
  requiredSkills: "",
};

describe("bindingFromNodeData", () => {
  it("infers the instance source when the step carries an agent id", () => {
    expect(bindingFromNodeData({ agentId: "id-research", agentName: "researcher" })).toEqual({
      source: "instance",
      agentId: "id-research",
      agentName: "researcher",
      agentType: "",
      sessionMode: "",
      requiredSkills: "",
    });
  });

  it("infers the name source when only a name is set", () => {
    // This is how a workflow hydrated from `agent_name` steps arrives.
    expect(bindingFromNodeData({ agentName: "researcher" }).source).toBe("name");
  });

  it("infers the type source when only a type is set", () => {
    // A find-or-spawn step (#7712), or a template that authored one.
    expect(bindingFromNodeData({ agentType: "researcher" })).toEqual({
      source: "type",
      agentId: "",
      agentName: "",
      agentType: "researcher",
      sessionMode: "",
      requiredSkills: "",
    });
  });

  it("honours an explicitly stored source over the inference", () => {
    expect(
      bindingFromNodeData({ agentSource: "name", agentId: "id-research", agentName: "researcher" })
        .source,
    ).toBe("name");
    expect(
      bindingFromNodeData({ agentSource: "type", agentId: "id-research", agentType: "researcher" })
        .source,
    ).toBe("type");
  });

  it("ignores a stored source that is not one of the three", () => {
    // A layout written by a build that spelled the source differently.
    expect(
      bindingFromNodeData({ agentSource: "template", agentType: "researcher" } as unknown as CanvasNodeData)
        .source,
    ).toBe("type");
  });

  it("drops a session mode the API would not parse", () => {
    // A layout persisted by an older build, or hand-edited, can hold anything here.
    expect(bindingFromNodeData({ sessionMode: "bogus" } as unknown as CanvasNodeData).sessionMode).toBe("");
    expect(normalizeSessionMode(42)).toBe("");
    expect(normalizeSessionMode("new")).toBe("new");
  });
});

describe("bindingFromNodeData required skills", () => {
  it("seeds the box from a stored list and drops a malformed one", () => {
    expect(
      bindingFromNodeData({ requiredSkills: ["b", "a"] } as CanvasNodeData).requiredSkills,
    ).toBe("b, a");
    expect(
      bindingFromNodeData({ requiredSkills: "b,a" } as unknown as CanvasNodeData)
        .requiredSkills,
    ).toBe("");
  });
});

describe("bindingToNodeData", () => {
  it("emits only the id for an instance binding", () => {
    expect(
      bindingToNodeData(
        { ...EMPTY, agentId: "id-writer", agentName: "stale", agentType: "stale-type" },
        AGENTS,
      ),
    ).toEqual({
      agentSource: "instance",
      agentId: "id-writer",
      // Re-derived from the registry, so a stale echo cannot be saved.
      agentName: "writer",
      agentType: undefined,
      sessionMode: undefined,
    });
  });

  it("emits only the name for a name binding, dropping the stale id", () => {
    expect(
      bindingToNodeData(
        { ...EMPTY, source: "name", agentId: "id-writer", agentName: " researcher ", sessionMode: "new" },
        AGENTS,
      ),
    ).toEqual({
      agentSource: "name",
      agentId: undefined,
      agentName: "researcher",
      agentType: undefined,
      sessionMode: "new",
    });
  });

  it("emits only the type for a type binding, dropping the id and the name", () => {
    // The clearing is what makes a type binding reversible: nothing an
    // abandoned source wrote survives for `stepAgentPayload` to prefer.
    expect(
      bindingToNodeData(
        { ...EMPTY, source: "type", agentId: "id-writer", agentName: "writer", agentType: " researcher " },
        AGENTS,
      ),
    ).toEqual({
      agentSource: "type",
      agentId: undefined,
      agentName: undefined,
      agentType: "researcher",
      sessionMode: undefined,
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

  it("sends the type alone for a type binding", () => {
    expect(
      stepAgentFields({ agentSource: "type", agentType: "researcher" }),
    ).toEqual({ agent_type: "researcher", session_mode: undefined });
  });

  it("carries the session override and omits it when unset", () => {
    expect(stepAgentFields({ agentId: "id-research", sessionMode: "new" }).session_mode).toBe("new");
    expect(stepAgentFields({ agentId: "id-research" }).session_mode).toBeUndefined();
  });
});

describe("parseRequiredSkills", () => {
  it("trims, drops blanks and de-duplicates", () => {
    expect(parseRequiredSkills("  a , ,a,  b ")).toEqual(["a", "b"]);
  });

  it("reads an all-whitespace box as no requirement", () => {
    expect(parseRequiredSkills("  ,  , ")).toEqual([]);
  });
});

describe("formatRequiredSkills", () => {
  it("renders a stored list back into the editable box", () => {
    expect(formatRequiredSkills(["a", "b"])).toBe("a, b");
  });

  it("reads anything that is not a string array as no requirement", () => {
    expect(formatRequiredSkills(undefined)).toBe("");
    expect(formatRequiredSkills("a,b")).toBe("");
    expect(formatRequiredSkills([1, "", "  ", "a"])).toBe("a");
  });
});

describe("isStepBound", () => {
  it("counts a name-only binding as bound", () => {
    expect(isStepBound({ agentName: "researcher" })).toBe(true);
  });
  it("counts a type-only binding as bound", () => {
    // Find-or-spawn resolves at run time, so the step is not unassigned.
    expect(isStepBound({ agentSource: "type", agentType: "researcher" })).toBe(true);
  });
  it("does not count whitespace as a binding", () => {
    expect(isStepBound({ agentName: "   " })).toBe(false);
    expect(isStepBound({ agentType: "   " })).toBe(false);
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
      agentType: "",
      sessionMode: "",
      requiredSkills: "",
    });
  });

  it("carries the instance name into a type binding and back again", () => {
    const bound: StepAgentBindingValue = { ...EMPTY, agentId: "id-research", agentName: "researcher" };
    const asType = switchAgentSource(bound, "type", AGENTS);
    expect(asType).toMatchObject({ source: "type", agentType: "researcher" });
    // The return trip is what a two-valued source made unreachable.
    expect(switchAgentSource(asType, "instance", AGENTS)).toMatchObject({
      source: "instance",
      agentId: "id-research",
      agentName: "researcher",
    });
  });

  it("carries a template name into a name binding", () => {
    const asType: StepAgentBindingValue = { ...EMPTY, source: "type", agentType: "not-spawned-yet" };
    expect(switchAgentSource(asType, "name", AGENTS)).toMatchObject({
      source: "name",
      agentName: "not-spawned-yet",
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

  it("switches to an agent type and back out again", async () => {
    // The type binding (#7712) is the third door #7724 has to keep openable:
    // once `agentType` is the only field set, nothing but a source control
    // that can report "instance" again could clear it.
    const user = userEvent.setup();
    const seen: StepAgentBindingValue[] = [];
    render(<Harness initial={EMPTY} onValue={v => seen.push(v)} />);

    await user.selectOptions(screen.getByLabelText("canvas.assign_agent"), "id-research");
    await user.selectOptions(screen.getByLabelText("canvas.agent_source_label"), "type");
    expect(last(seen)).toMatchObject({ source: "type", agentType: "researcher" });
    expect(screen.getByLabelText("canvas.agent_type_label")).toHaveValue("researcher");
    expect(screen.queryByLabelText("canvas.assign_agent")).toBeNull();
    // Only the type reaches the API, so the abandoned id cannot bind the step.
    expect(bindingToNodeData(last(seen), AGENTS)).toMatchObject({
      agentSource: "type",
      agentId: undefined,
      agentName: undefined,
      agentType: "researcher",
    });

    await user.selectOptions(screen.getByLabelText("canvas.agent_source_label"), "instance");
    expect(screen.getByLabelText("canvas.assign_agent")).toHaveValue("id-research");
    expect(bindingToNodeData(last(seen), AGENTS)).toMatchObject({
      agentSource: "instance",
      agentId: "id-research",
      agentType: undefined,
    });
  });

  it("keeps a hand-typed type that no template answers to yet", async () => {
    const user = userEvent.setup();
    const seen: StepAgentBindingValue[] = [];
    render(<Harness initial={{ ...EMPTY, source: "type" }} onValue={v => seen.push(v)} />);

    await user.type(screen.getByLabelText("canvas.agent_type_label"), "nightly-auditor");
    expect(last(seen)).toMatchObject({ source: "type", agentType: "nightly-auditor" });

    // Bouncing out to the name source and back keeps the typed handle.
    await user.selectOptions(screen.getByLabelText("canvas.agent_source_label"), "name");
    expect(screen.getByLabelText("canvas.agent_name_label")).toHaveValue("nightly-auditor");
    await user.selectOptions(screen.getByLabelText("canvas.agent_source_label"), "type");
    expect(screen.getByLabelText("canvas.agent_type_label")).toHaveValue("nightly-auditor");
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

  // #7721 shipped `required_skills` on the kernel and the API with no editor surface, so a workflow author could only set it by hand-editing TOML or posting JSON.
  // These pin the whole path: typed text becomes the API array, a stored array comes back into the box, and the box survives an agent rebinding.
  it("sends the typed required skills as the API array", async () => {
    const user = userEvent.setup();
    const seen: StepAgentBindingValue[] = [];
    render(<Harness initial={{ ...EMPTY, agentId: "id-research" }} onValue={v => seen.push(v)} />);

    const input = screen.getByLabelText("canvas.required_skills_label");
    expect(input).toHaveValue("");

    await user.type(input, "browser-automation, pdf-extract");
    const data = bindingToNodeData(last(seen), AGENTS);
    expect(data.requiredSkills).toEqual(["browser-automation", "pdf-extract"]);
    expect(stepAgentFields(data).required_skills).toEqual([
      "browser-automation",
      "pdf-extract",
    ]);
  });

  it("omits the field entirely when the box is blank", async () => {
    const user = userEvent.setup();
    const seen: StepAgentBindingValue[] = [];
    render(
      <Harness
        initial={{ ...EMPTY, agentId: "id-research", requiredSkills: "pdf-extract" }}
        onValue={v => seen.push(v)}
      />,
    );

    // A stray comma must not become a blank requirement: the API rejects one with a 400 naming the step, so the workflow would stop saving at all.
    await user.clear(screen.getByLabelText("canvas.required_skills_label"));
    await user.type(screen.getByLabelText("canvas.required_skills_label"), " , ");
    const data = bindingToNodeData(last(seen), AGENTS);
    expect(data.requiredSkills).toBeUndefined();
    expect(stepAgentFields(data).required_skills).toBeUndefined();
  });

  it("keeps the required skills when the agent binding is switched", async () => {
    const user = userEvent.setup();
    const seen: StepAgentBindingValue[] = [];
    render(
      <Harness
        initial={{ ...EMPTY, agentId: "id-research", requiredSkills: "browser-automation" }}
        onValue={v => seen.push(v)}
      />,
    );

    await user.selectOptions(screen.getByLabelText("canvas.agent_source_label"), "type");
    expect(screen.getByLabelText("canvas.required_skills_label")).toHaveValue(
      "browser-automation",
    );
    expect(bindingToNodeData(last(seen), AGENTS).requiredSkills).toEqual([
      "browser-automation",
    ]);
  });

  it("offers the loaded skills as datalist suggestions", () => {
    render(<Harness initial={EMPTY} onValue={vi.fn()} />);
    const input = screen.getByLabelText("canvas.required_skills_label");
    const list = document.getElementById(input.getAttribute("list")!);
    expect([...list!.querySelectorAll("option")].map(o => o.getAttribute("value"))).toEqual(
      SKILLS,
    );
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
