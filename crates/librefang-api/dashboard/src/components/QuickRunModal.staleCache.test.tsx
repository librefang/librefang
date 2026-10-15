import { beforeEach, describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import React from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { QuickRunModal } from "./QuickRunModal";
import { agentKeys } from "../lib/queries/keys";
import { listAgents, listAgentTemplates } from "../lib/http/client";

/**
 * The stale-cache half of the parent preselection fix (#8385).
 *
 * `useSpawnAgent` invalidates `agentKeys.lists()` but only marks the query
 * stale — it refetches nothing. The Agents page itself never subscribes to
 * `useAgents` (its rows come from the overview snapshot), so the dialog can
 * mount over a cached list that predates the agent that was just created and
 * clicked, with a background refetch only starting now. This suite seeds that
 * exact cache and uses the real query hooks, so the guard is measured against
 * React Query's own stale/refetching state rather than a stubbed hook.
 */

// `motion/react` ships browser-only animation primitives that jsdom can't
// drive. Same shim as the modal's own suite.
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

// The real store pulls in the full i18n instance, which needs more of
// `react-i18next` than this suite stubs; the dialog only reads `addToast`.
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: { addToast: ReturnType<typeof vi.fn> }) => unknown) =>
    selector({ addToast: vi.fn() }),
}));

// Only the two reads this dialog issues are stubbed; everything else keeps the
// real client so the transitively imported hook modules resolve.
vi.mock("../lib/http/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/http/client")>();
  return {
    ...actual,
    listAgents: vi.fn(),
    listAgentTemplates: vi.fn(),
    spawnEphemeral: vi.fn(),
  };
});

const listAgentsMock = vi.mocked(listAgents);

const ALPHA = { id: "agent-a", name: "Alpha", is_hand: false };
const BRAVO = { id: "agent-b", name: "Bravo", is_hand: false };

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(listAgentTemplates).mockResolvedValue([]);
});

describe("QuickRunModal over a stale cached agent list (#8385)", () => {
  it("waits for the refetch instead of billing the first cached agent", async () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    // The cache exactly as the spawn mutation's invalidation leaves it: a list
    // fetched before agent-b existed, marked stale with no active subscriber to
    // refetch it.
    qc.setQueryData(agentKeys.list(), [ALPHA]);
    await qc.invalidateQueries({ queryKey: agentKeys.lists() });
    listAgentsMock.mockResolvedValue([ALPHA, BRAVO]);

    render(
      <QueryClientProvider client={qc}>
        <QuickRunModal initialParent={BRAVO.id} onClose={() => {}} />
      </QueryClientProvider>,
    );

    // The stale list has no Bravo; committing its first row would run the task
    // on Alpha's ledger. This holds both before and after the refetch lands.
    expect(screen.getByRole("combobox", { name: "agents.quick_run_parent" })).not.toHaveValue(
      ALPHA.id,
    );

    await waitFor(() =>
      expect(screen.getByRole("combobox", { name: "agents.quick_run_parent" })).toHaveValue(
        BRAVO.id,
      ),
    );
    expect(listAgentsMock).toHaveBeenCalledTimes(1);
  });
});
