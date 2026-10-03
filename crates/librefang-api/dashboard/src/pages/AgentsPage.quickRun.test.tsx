import { beforeEach, describe, it, expect, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import React from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AgentsPage } from "./AgentsPage";

/**
 * The UI half of `POST /api/agents/spawn-ephemeral`.
 *
 * #8384/#8385 repointed the agent-types Run control at the create flow and
 * deleted the Quick Run dialog with it, leaving the endpoint published in the
 * OpenAPI document but with no way to reach it from the dashboard. The modal's
 * own suite covers the dialog; this one covers the half a component test
 * cannot see — that the Agents page still renders a control to open it, and
 * that the control carries the agent whose ledger pays for the run.
 *
 * Deliberately a separate file from AgentsPage.test.tsx: that one tests pure
 * helpers with narrow per-module mocks, and widening those mocks to render the
 * whole page would put those tests at risk for no gain.
 */

// `vi.mock` factories are hoisted above the module body, so the fixtures they
// close over have to be hoisted too.
//
// Two agents, and the second is the one the tests select: with a single agent
// the dialog's "fall back to the first candidate" path answers the same id as
// "preselect the agent that was clicked", so the wiring could be severed
// (`initialParent={undefined}`) and every assertion below would still pass.
const { AGENTS, spawnEphemeralAsync } = vi.hoisted(() => ({
  AGENTS: [
    { id: "agent-a", name: "Alpha", is_hand: false, state: "running" },
    { id: "agent-b", name: "Bravo", is_hand: false, state: "running" },
  ],
  spawnEphemeralAsync: vi.fn(),
}));

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: new Proxy(
    {},
    {
      get: (_target: unknown, prop: string) =>
        ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
          React.createElement(prop, rest, children),
    },
  ),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: "en" } }),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  useSearch: () => ({}),
  Link: ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
    React.createElement("a", rest, children),
}));

// The Agents list is projected from the overview snapshot, not from
// `useAgents` — the page calls `useDashboardSnapshot()` for it.
vi.mock("../lib/queries/overview", () => ({
  useDashboardSnapshot: () => ({ data: { agents: AGENTS }, isLoading: false }),
}));
vi.mock("../lib/queries/sessions", () => ({
  useSessionDetails: () => ({ data: undefined, isLoading: false }),
  useSessions: () => ({ data: [], isLoading: false }),
}));
vi.mock("../lib/queries/memory", () => ({
  useAgentKvMemory: () => ({ data: undefined, isLoading: false }),
  useMemorySearchOrList: () => ({ data: undefined, isLoading: false }),
}));
vi.mock("../lib/queries/providers", () => ({
  useProviders: () => ({ data: [], isLoading: false }),
}));
vi.mock("../lib/queries/models", () => ({
  useModels: () => ({ data: [], isLoading: false }),
}));
vi.mock("../lib/queries/skills", () => ({
  useSkills: () => ({ data: [], isLoading: false }),
}));
vi.mock("../lib/queries/mcp", () => ({
  useMcpServers: () => ({ data: { configured: [] }, isLoading: false }),
}));

vi.mock("../lib/queries/agents", () => ({
  agentQueries: {
    detail: (id: string) => ({
      queryKey: ["agents", "detail", id],
      queryFn: () => Promise.resolve(AGENTS.find((a) => a.id === id) ?? AGENTS[0]),
    }),
  },
  // The dialog's own candidate list — an array, as `listAgents` returns.
  useAgents: () => ({ data: AGENTS, isLoading: false, isError: false }),
  useAgentEvents: () => ({ data: [], isLoading: false }),
  useAgentSessions: () => ({ data: [], isLoading: false }),
  useAgentStats: () => ({ data: undefined, isLoading: false }),
  useAgentTemplates: () => ({ data: [], isLoading: false }),
  useAgentTools: () => ({ data: [], isLoading: false }),
  useAgentSkills: () => ({ data: [], isLoading: false }),
  useAgentMcpServers: () => ({ data: [], isLoading: false }),
  usePromptVersions: () => ({ data: [], isLoading: false }),
  useTools: () => ({ data: [], isLoading: false }),
  // Same reason as the mutation block below: the agent-editor and avatar
  // branches each add an import to `AgentsPage`, and a `vi.mock` factory that
  // omits one fails the file with a message about the mock rather than about
  // the flow under test. Empty/undefined data is enough — none of these is
  // read by the Quick Run path.
  useAgentAvatarUrl: () => ({ data: undefined, isLoading: false }),
  useAgentChannels: () => ({ data: [], isLoading: false }),
  useAgentManifest: () => ({ data: undefined, isLoading: false }),
  useAgentManifestHistory: () => ({ data: [], isLoading: false }),
}));

vi.mock("../lib/mutations/agents", () => ({
  useSpawnEphemeral: () => ({ mutateAsync: spawnEphemeralAsync, isPending: false }),
  useSpawnAgent: () => ({ mutate: vi.fn(), isPending: false }),
  useCloneAgent: () => ({ mutate: vi.fn(), isPending: false }),
  useDeleteAgent: () => ({ mutate: vi.fn(), mutateAsync: vi.fn(), isPending: false }),
  usePatchAgent: () => ({ mutate: vi.fn(), isPending: false }),
  usePatchAgentRuntimeConfig: () => ({ mutate: vi.fn(), isPending: false }),
  useResetAgentSession: () => ({ mutate: vi.fn(), mutateAsync: vi.fn(), isPending: false }),
  useResumeAgent: () => ({ mutate: vi.fn(), mutateAsync: vi.fn(), isPending: false }),
  useSuspendAgent: () => ({ mutate: vi.fn(), mutateAsync: vi.fn(), isPending: false }),
  useUpdateAgentTools: () => ({ mutate: vi.fn(), isPending: false }),
  useSetAgentSkills: () => ({ mutate: vi.fn(), isPending: false }),
  useAgentTemplateToml: () => ({ mutate: vi.fn(), isPending: false }),
  // Five exports `main`'s `AgentsPage` does not import yet, and the agent-editor
  // and avatar branches each add one along with the import that uses it. A
  // `vi.mock` factory that omits any of them fails this whole file with
  // `No "<name>" export is defined on the mock` — a message about the mock
  // rather than about the Quick Run wiring this test is here to measure.
  // Listing them costs nothing: an export the page never imports is inert.
  useSetAgentMcpServers: () => ({ mutate: vi.fn(), isPending: false }),
  useSetAgentChannels: () => ({ mutate: vi.fn(), isPending: false }),
  useDeleteAgentAvatar: () => ({ mutate: vi.fn(), mutateAsync: vi.fn(), isPending: false }),
  useUpdateAgentIdentity: () => ({ mutate: vi.fn(), isPending: false }),
  useUploadAgentAvatar: () => ({ mutate: vi.fn(), isPending: false }),
}));
vi.mock("../lib/mutations/prompts", () => ({
  useBindPromptVersionToAgent: () => ({ mutate: vi.fn(), isPending: false }),
}));

const addToastMock = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: { addToast: typeof addToastMock }) => unknown) =>
    selector({ addToast: addToastMock }),
}));

function renderPage() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <AgentsPage />
    </QueryClientProvider>,
  );
}

function runControl() {
  return screen.findByRole("button", { name: "agents.quick_run" });
}

beforeEach(() => {
  vi.clearAllMocks();
  // The page auto-selects the first agent only on a wide viewport (the effect
  // bails under the 1000px breakpoint). jsdom has no layout, so this is the
  // switch that decides whether the detail panel — and with it the Run
  // control — renders at all.
  // The stub is `setupTests.ts`'s MockMediaQueryList, whose `matches` is a
  // plain field; the DOM type marks it readonly, hence the widening cast.
  (window.matchMedia("(min-width: 1000px)") as unknown as { matches: boolean }).matches = true;
  spawnEphemeralAsync.mockResolvedValue({
    name: "ephemeral-worker",
    response: "All done.",
    iterations: 1,
    cost_usd: 0,
    tools: [],
  });
});

describe("AgentsPage Quick Run entry point (#6699)", () => {
  it("renders a Quick Run control for the selected agent", async () => {
    renderPage();

    expect(await runControl()).toBeTruthy();
  });

  it("opens the ephemeral dialog on the agent whose row was selected, and runs it there", async () => {
    renderPage();

    // The page auto-selects the first agent (Alpha) on a wide viewport, so
    // picking Bravo is what makes this discriminating: the dialog's fallback
    // for "no viable parent given" answers Alpha, and only the agent that was
    // actually clicked answers Bravo.
    fireEvent.click(await screen.findByRole("button", { name: /Bravo/ }));

    fireEvent.click(await runControl());

    // The dialog, not a navigation: the agent-types control navigates to the
    // create flow, and this capability must not have been folded into it.
    const task = await screen.findByRole("textbox", { name: "agents.quick_run_task" });
    expect(screen.getByRole("combobox", { name: "agents.quick_run_parent" })).toHaveValue(
      "agent-b",
    );

    fireEvent.change(task, { target: { value: "Say hello" } });
    fireEvent.click(screen.getByRole("button", { name: "agents.quick_run_submit" }));

    await waitFor(() => expect(spawnEphemeralAsync).toHaveBeenCalledTimes(1));
    expect(spawnEphemeralAsync).toHaveBeenCalledWith({
      parent: "agent-b",
      message: "Say hello",
    });
    expect(await screen.findByText("All done.")).toBeTruthy();
  });
});
