import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import * as http from "../lib/http/client";
import type { AgentDetail, AgentModelRouting, ModelRouterProfiles } from "../api";
import { AgentModelRoutingPanel } from "./AgentModelRoutingPanel";

// Pass-through i18n stub that honours `defaultValue`, so the assertions below
// match the English the operator actually reads instead of key paths.
vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, defaultOrOpts?: unknown) => {
        if (
          defaultOrOpts &&
          typeof defaultOrOpts === "object" &&
          "defaultValue" in (defaultOrOpts as Record<string, unknown>)
        ) {
          return String(
            (defaultOrOpts as { defaultValue: string }).defaultValue,
          );
        }
        return typeof defaultOrOpts === "string" ? defaultOrOpts : key;
      },
    }),
  };
});

// The store's zustand `persist` middleware needs a storage backend jsdom does
// not give it; the panel's behaviour under test is what it renders, not toasts.
vi.mock("../lib/store", () => {
  const noop = () => {};
  return {
    useUIStore: (selector: (s: { addToast: typeof noop }) => unknown) =>
      selector({ addToast: noop }),
  };
});

vi.mock("../lib/http/client", () => ({
  listModelRouterProfiles: vi.fn(),
  getAgentModelRouting: vi.fn(),
  updateAgentModelRouting: vi.fn(),
}));

const agent: AgentDetail = {
  id: "00000000-0000-0000-0000-000000000001",
  name: "router-agent",
};

const catalog: ModelRouterProfiles = {
  enabled: true,
  profiles: [
    {
      name: "coder",
      tags: ["code"],
      provider: "anthropic",
      model: "claude-sonnet-4-5",
      cost_tier: "medium",
      priority: 10,
      max_complexity: 80,
    },
  ],
};

const OPT_OUT_BANNER =
  /opted out of routing \(fixed\) — the allowlist and budget below have no effect/i;

function withQueryClient(node: ReactNode) {
  const qc = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, structuralSharing: false },
    },
  });
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>);
}

function seed(routing: AgentModelRouting) {
  vi.mocked(http.listModelRouterProfiles).mockResolvedValue(catalog);
  vi.mocked(http.getAgentModelRouting).mockResolvedValue(routing);
}

describe("AgentModelRoutingPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // #7781 review: `fixed` is the per-agent router opt-out, and it now survives
  // a save instead of being cleared by one. This banner is the only place it
  // is visible to an operator — without it the panel shows a live-looking
  // allowlist and budget for a router that never runs on this agent.
  it("warns that the allowlist and budget do nothing while the agent is opted out", async () => {
    seed({ mode: "flexible", allowed_profiles: ["coder"], fixed: true });

    withQueryClient(<AgentModelRoutingPanel agent={agent} />);

    expect(await screen.findByText(OPT_OUT_BANNER)).toBeInTheDocument();
  });

  it("shows no opt-out warning for an agent the router is allowed to touch", async () => {
    seed({ mode: "flexible", allowed_profiles: ["coder"], fixed: false });

    withQueryClient(<AgentModelRoutingPanel agent={agent} />);

    // Wait for the loaded state before asserting an absence, otherwise the
    // spinner would satisfy the assertion on its own.
    expect(await screen.findByText("coder")).toBeInTheDocument();
    await waitFor(() => {
      expect(screen.queryByText(OPT_OUT_BANNER)).not.toBeInTheDocument();
    });
  });
});
