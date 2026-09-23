import { beforeEach, describe, it, expect, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import React from "react";
import { QuickRunModal } from "./QuickRunModal";
import { useAgents, useAgentTemplates } from "../lib/queries/agents";
import { useSpawnEphemeral } from "../lib/mutations/agents";

// `motion/react` ships browser-only animation primitives that jsdom can't
// drive. Same shim as Modal.test — render children inline and turn `motion.foo`
// into the corresponding host tag.
vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => (
    <>{children}</>
  ),
  motion: new Proxy(
    {},
    {
      get: (_target: unknown, prop: string) =>
        ({
          children,
          ...rest
        }: { children?: React.ReactNode } & Record<string, unknown>) =>
          React.createElement(prop, rest, children),
    },
  ),
}));

// Return the key as the rendered string so the assertions below name the exact
// i18n keys this dialog reads. A key that gets renamed shows up as a diff here
// rather than as a silently-missing label at runtime. `{{name}}` is interpolated
// because the dialog title is the only place the subject is named and asserting
// on the key alone would not show *what* it named.
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { name?: string }) =>
      typeof opts?.name === "string" ? `${key}:${opts.name}` : key,
    i18n: { language: "en" },
  }),
}));

vi.mock("../lib/queries/agents", () => ({
  useAgents: vi.fn(),
  useAgentTemplates: vi.fn(),
}));
vi.mock("../lib/mutations/agents", () => ({ useSpawnEphemeral: vi.fn() }));

const addToastMock = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: { addToast: typeof addToastMock }) => unknown) =>
    selector({ addToast: addToastMock }),
}));

const useAgentsMock = useAgents as unknown as ReturnType<typeof vi.fn>;
const useAgentTemplatesMock = useAgentTemplates as unknown as ReturnType<typeof vi.fn>;
const useSpawnEphemeralMock = useSpawnEphemeral as unknown as ReturnType<typeof vi.fn>;

const mutateAsync = vi.fn();

const AGENTS = [
  { id: "agent-a", name: "Alpha", is_hand: false },
  { id: "agent-b", name: "Bravo", is_hand: false },
  { id: "hand-1", name: "Greeter", is_hand: true },
];

const TEMPLATES = [
  { name: "researcher", description: "", provider: "", model: "", source: "user", editable: true },
  { name: "coder", description: "", provider: "", model: "", source: "user", editable: true },
];

function setAgents(data: unknown[] = AGENTS, isLoading = false) {
  useAgentsMock.mockReturnValue({ data, isLoading });
}

function setTemplates(data: unknown[] = TEMPLATES, isLoading = false) {
  useAgentTemplatesMock.mockReturnValue({ data, isLoading });
}

function submitButton() {
  return screen.getByRole("button", { name: "agents.quick_run_submit" });
}

function taskField() {
  return screen.getByRole("textbox", { name: "agents.quick_run_task" });
}

beforeEach(() => {
  vi.clearAllMocks();
  mutateAsync.mockResolvedValue({
    name: "ephemeral-worker",
    response: "All done.",
    iterations: 3,
    cost_usd: 0.0125,
    tools: ["read_file", "write_file"],
  });
  useSpawnEphemeralMock.mockReturnValue({ mutateAsync, isPending: false });
  setAgents();
  setTemplates();
});

/**
 * The Quick Run dialog is the only way the dashboard reaches
 * `POST /api/agents/spawn-ephemeral`. #8384/#8385 deleted it along with the
 * control that opened it and left the endpoint live but unreachable from the
 * webui; these tests are what would have failed.
 */
describe("QuickRunModal", () => {
  it("preselects the agent it was opened from", () => {
    render(<QuickRunModal initialParent="agent-b" onClose={() => {}} />);

    expect(screen.getByRole("combobox", { name: "agents.quick_run_parent" })).toHaveValue(
      "agent-b",
    );
  });

  it("offers no hand as a parent, since a hand owns no budget to bill", () => {
    render(<QuickRunModal initialParent="agent-a" onClose={() => {}} />);

    const options = [
      ...screen
        .getByRole("combobox", { name: "agents.quick_run_parent" })
        .querySelectorAll("option"),
    ].map((o) => o.textContent);
    expect(options).toEqual(["Alpha", "Bravo"]);
  });

  // A stale id — the agent was turned into a hand, or deleted since the list
  // was fetched — must leave a usable select, not a blank one.
  it("falls back to the first candidate when the opening agent cannot be a parent", () => {
    render(<QuickRunModal initialParent="hand-1" onClose={() => {}} />);

    expect(screen.getByRole("combobox", { name: "agents.quick_run_parent" })).toHaveValue(
      "agent-a",
    );
  });

  it("runs the worker on the parent's behalf and shows what came back", async () => {
    render(<QuickRunModal initialParent="agent-b" onClose={() => {}} />);

    fireEvent.change(taskField(), { target: { value: "Summarise the release notes" } });
    fireEvent.click(submitButton());

    await waitFor(() => expect(mutateAsync).toHaveBeenCalledTimes(1));
    expect(mutateAsync).toHaveBeenCalledWith({
      parent: "agent-b",
      message: "Summarise the release notes",
    });

    expect(await screen.findByText("All done.")).toBeTruthy();
    expect(screen.getByText("ephemeral-worker")).toBeTruthy();
    expect(screen.getByText("agents.quick_run_ephemeral_note")).toBeTruthy();
  });

  // The type is what makes this "run this type once" rather than "run a copy of
  // the parent", which is the use case the feature was announced for. Without
  // `agent_type` the kernel builds the worker from `parent.manifest.clone()`.
  it("names the type on the worker when one is picked, and inherits without one", async () => {
    render(<QuickRunModal initialParent="agent-b" onClose={() => {}} />);

    fireEvent.change(taskField(), { target: { value: "Try the type out" } });
    fireEvent.change(screen.getByRole("combobox", { name: "agents.quick_run_type" }), {
      target: { value: "researcher" },
    });
    fireEvent.click(submitButton());

    await waitFor(() => expect(mutateAsync).toHaveBeenCalledTimes(1));
    expect(mutateAsync).toHaveBeenCalledWith({
      parent: "agent-b",
      message: "Try the type out",
      agent_type: "researcher",
      label: "researcher",
    });
  });

  it("sends no agent_type when the parent's manifest is what should run", async () => {
    render(<QuickRunModal initialParent="agent-b" onClose={() => {}} />);

    // The picker defaults to inherit, and the two fields the server treats as
    // optional stay out of the body rather than being sent as empty strings.
    expect(screen.getByRole("combobox", { name: "agents.quick_run_type" })).toHaveValue("");
    fireEvent.change(taskField(), { target: { value: "Say hello" } });
    fireEvent.click(submitButton());

    await waitFor(() => expect(mutateAsync).toHaveBeenCalledTimes(1));
    expect(mutateAsync.mock.calls[0][0]).toEqual({
      parent: "agent-b",
      message: "Say hello",
    });
  });

  it("offers the installed types, with inheriting the parent as the default", () => {
    render(<QuickRunModal initialParent="agent-a" onClose={() => {}} />);

    const options = screen
      .getByRole("combobox", { name: "agents.quick_run_type" })
      .querySelectorAll("option");
    // Sorted, and led by the "run as the parent" choice, which is a real
    // option rather than an empty one.
    expect([...options].map((o) => o.textContent)).toEqual([
      "agents.quick_run_type_none",
      "coder",
      "researcher",
    ]);
  });

  it("names the subject in the title, following the type once one is picked", () => {
    render(<QuickRunModal initialParent="agent-b" onClose={() => {}} />);

    expect(screen.getByRole("heading", { name: "agents.quick_run_title:Bravo" })).toBeTruthy();

    fireEvent.change(screen.getByRole("combobox", { name: "agents.quick_run_type" }), {
      target: { value: "researcher" },
    });

    expect(screen.getByRole("heading", { name: "agents.quick_run_title:researcher" })).toBeTruthy();
  });

  it("refuses to submit until a task is written", () => {
    render(<QuickRunModal initialParent="agent-a" onClose={() => {}} />);

    expect(submitButton()).toBeDisabled();

    fireEvent.change(taskField(), { target: { value: "   " } });
    expect(submitButton()).toBeDisabled();

    fireEvent.change(taskField(), { target: { value: "do the thing" } });
    expect(submitButton()).not.toBeDisabled();
  });

  it("surfaces a failed run as a toast instead of an empty result panel", async () => {
    // `toastErr` prefers the thrown message, so an empty one is the case that
    // reaches the fallback this dialog supplies — and pins the key it names.
    mutateAsync.mockRejectedValue("");
    render(<QuickRunModal initialParent="agent-a" onClose={() => {}} />);

    fireEvent.change(taskField(), { target: { value: "do the thing" } });
    fireEvent.click(submitButton());

    await waitFor(() => expect(addToastMock).toHaveBeenCalled());
    expect(addToastMock.mock.calls[0][0]).toBe("agents.quick_run_failed");
    expect(addToastMock.mock.calls[0][1]).toBe("error");
    expect(screen.queryByText("agents.quick_run_result")).toBeNull();
  });

  it("says so, rather than offering an empty picker, when no agent can be a parent", () => {
    setAgents([]);
    render(<QuickRunModal onClose={() => {}} />);

    expect(screen.getByText("agents.quick_run_no_agents")).toBeTruthy();
    expect(screen.queryByRole("combobox", { name: "agents.quick_run_parent" })).toBeNull();
  });
});
