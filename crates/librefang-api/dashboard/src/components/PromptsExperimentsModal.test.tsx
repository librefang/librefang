import { beforeEach, describe, it, expect, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React from "react";
import { PromptsExperimentsModal } from "./PromptsExperimentsModal";
import { usePromptVersions } from "../lib/queries/agents";
import { useDeletePromptVersion } from "../lib/mutations/agents";

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

/**
 * The variant cap, mocked deliberately low.
 *
 * What the cap test asserts is that selection stops at the cap and says so, which
 * is the same behaviour at three variants as at a hundred.
 * Driving the production value meant a hundred sequential clicks through a subtree
 * the motion mock remounts on every state change, and that is quadratic: two seconds
 * on an idle machine against vitest's five-second default, so the test failed
 * whenever the rest of the suite happened to be loading the same cores.
 */
const { TEST_VARIANT_CAP } = vi.hoisted(() => ({ TEST_VARIANT_CAP: 3 }));

vi.mock("./trafficSplit", () => ({
  buildEvenTrafficSplit: vi.fn().mockReturnValue([50, 50]),
  MAX_TRAFFIC_VARIANTS: TEST_VARIANT_CAP,
}));

describe("PromptsExperimentsModal", () => {
  beforeEach(() => {
    vi.mocked(usePromptVersions).mockReturnValue({
      data: [],
      isLoading: false,
    } as never);
    vi.mocked(useDeletePromptVersion).mockReturnValue({
      mutate: vi.fn(),
      mutateAsync: vi.fn(),
      isPending: false,
    } as never);
  });

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

  it("requires confirmation before deleting a prompt version", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    const mutateAsync = vi.fn().mockResolvedValue(undefined);
    vi.mocked(usePromptVersions).mockReturnValue({
      data: [
        {
          id: "version-1",
          version: 3,
          is_active: false,
          description: "candidate",
          system_prompt: "Be concise.",
          created_at: "2026-08-15T00:00:00Z",
        },
      ],
      isLoading: false,
    } as never);
    vi.mocked(useDeletePromptVersion).mockReturnValue({
      mutate: vi.fn(),
      mutateAsync,
      isPending: false,
    } as never);

    render(
      <PromptsExperimentsModal
        agentId="agent-1"
        agentName="Test Agent"
        onClose={onClose}
      />,
    );

    await user.click(screen.getByTitle("prompts.delete"));
    expect(mutateAsync).not.toHaveBeenCalled();
    expect(
      screen.getByText("agents.prompts_experiments.delete_version_title"),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "common.delete" }));
    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        versionId: "version-1",
        agentId: "agent-1",
      }),
    );
    expect(onClose).not.toHaveBeenCalled();
  });

  it("only marks a prompt preview when it is truncated", () => {
    vi.mocked(usePromptVersions).mockReturnValue({
      data: [
        {
          id: "short",
          version: 1,
          is_active: false,
          system_prompt: "Short prompt",
          created_at: "2026-08-15T00:00:00Z",
        },
        {
          id: "long",
          version: 2,
          is_active: false,
          system_prompt: "x".repeat(201),
          created_at: "2026-08-15T00:00:00Z",
        },
      ],
      isLoading: false,
    } as never);

    render(
      <PromptsExperimentsModal
        agentId="agent-1"
        agentName="Test Agent"
        onClose={() => {}}
      />,
    );

    expect(screen.getByText("Short prompt")).toBeInTheDocument();
    expect(screen.queryByText("Short prompt...")).toBeNull();
    expect(screen.getByText(`${"x".repeat(200)}...`)).toBeInTheDocument();
  });

  it("stops selection when every variant has a traffic bucket", async () => {
    const user = userEvent.setup();
    vi.mocked(usePromptVersions).mockReturnValue({
      data: Array.from({ length: TEST_VARIANT_CAP + 1 }, (_, index) => ({
        id: `version-${index}`,
        version: index + 1,
        is_active: false,
        system_prompt: `Prompt ${index + 1}`,
        created_at: "2026-08-15T00:00:00Z",
      })),
      isLoading: false,
    } as never);

    const { container } = render(
      <PromptsExperimentsModal
        agentId="agent-1"
        agentName="Test Agent"
        onClose={() => {}}
      />,
    );
    await user.click(
      screen.getByRole("tab", {
        name: "agents.prompts_experiments.experiments_tab",
      }),
    );
    await user.click(
      screen.getByRole("button", {
        name: "agents.prompts_experiments.new_experiment",
      }),
    );

    for (let index = 0; index < TEST_VARIANT_CAP; index += 1) {
      // The lightweight motion mock remounts its host subtree after state
      // changes, so read each current checkbox before clicking it.
      const checkbox = container.querySelectorAll<HTMLInputElement>(
        'input[type="checkbox"]',
      )[index];
      fireEvent.click(checkbox);
    }

    expect(
      container.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')[
        TEST_VARIANT_CAP
      ],
    ).toBeDisabled();
    expect(
      screen.getByText("agents.prompts_experiments.variant_limit"),
    ).toBeInTheDocument();
  });
});
