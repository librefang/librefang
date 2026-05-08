import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React from "react";
import { PromptsExperimentsModal } from "./PromptsExperimentsModal";

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

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({ t: (key: string) => key }),
  };
});

vi.mock("../lib/queries/agents", () => ({
  usePromptVersions: vi.fn().mockReturnValue({ data: [], isLoading: false }),
  useExperiments: vi.fn().mockReturnValue({ data: [], isLoading: false }),
  useExperimentMetrics: vi.fn().mockReturnValue({ data: null }),
}));

vi.mock("../lib/mutations/agents", () => ({
  useCreatePromptVersion: vi.fn().mockReturnValue({
    mutate: vi.fn(),
    isPending: false,
  }),
  useCreateExperiment: vi.fn().mockReturnValue({
    mutate: vi.fn(),
    isPending: false,
  }),
  useActivatePromptVersion: vi.fn().mockReturnValue({ mutate: vi.fn() }),
  useStartExperiment: vi.fn().mockReturnValue({ mutate: vi.fn() }),
  usePauseExperiment: vi.fn().mockReturnValue({ mutate: vi.fn() }),
  useCompleteExperiment: vi.fn().mockReturnValue({ mutate: vi.fn() }),
  useDeletePromptVersion: vi.fn().mockReturnValue({ mutate: vi.fn() }),
}));

vi.mock("./trafficSplit", () => ({
  buildEvenTrafficSplit: vi.fn().mockReturnValue([50, 50]),
}));

describe("PromptsExperimentsModal", () => {
  it("renders a dialog with the agent name", () => {
    render(
      <PromptsExperimentsModal
        agentId="agent-1"
        agentName="Test Agent"
        onClose={() => {}}
      />,
    );

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByText("Test Agent")).toBeInTheDocument();
  });

  it("renders two tab buttons inside the tablist", () => {
    render(
      <PromptsExperimentsModal
        agentId="agent-1"
        agentName="Test Agent"
        onClose={() => {}}
      />,
    );

    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(2);
  });

  it("calls onClose when close button is clicked", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();

    render(
      <PromptsExperimentsModal
        agentId="agent-1"
        agentName="Test Agent"
        onClose={onClose}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: "common.close" }),
    );

    expect(onClose).toHaveBeenCalledOnce();
  });

  it("calls onClose when backdrop is clicked", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();

    const { container } = render(
      <PromptsExperimentsModal
        agentId="agent-1"
        agentName="Test Agent"
        onClose={onClose}
      />,
    );

    const backdrop = container.querySelector(".fixed.inset-0")!;
    await user.click(backdrop);

    expect(onClose).toHaveBeenCalledOnce();
  });
});
