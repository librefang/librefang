import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import * as http from "../lib/http/client";
import type { AgentDetail, TriggerItem, CronJobItem } from "../api";
import { AgentSchedulePanel } from "./AgentSchedulePanel";

// Lightweight i18n stub — pass-through that honours `defaultValue` so the
// component's user-facing labels come out as readable English in queries
// rather than translation-key paths.
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

// useUIStore wraps its toast/theme state in zustand's `persist` middleware,
// which needs a working storage backend. Vitest's jsdom does not expose
// localStorage to the persist driver (the SerializeFn calls `setItem` on a
// stubbed object), and the resulting "storage.setItem is not a function"
// throw propagates as an unhandled rejection from a fire-and-forget toast.
// Replace the whole store with a no-op `addToast` so toasts disappear into
// the void — the component's behaviour under test is the HTTP / cache fan-
// out, not the toast surface.
vi.mock("../lib/store", () => {
  const noop = () => {};
  return {
    useUIStore: (selector: (s: { addToast: typeof noop }) => unknown) =>
      selector({ addToast: noop }),
  };
});

// Mock the entire HTTP surface — the component owns the React-Query
// subscriptions but every network call needs to resolve to a value
// shaped like the real API so the renderer doesn't blow up on undefined.
vi.mock("../lib/http/client", () => ({
  listCronJobs: vi.fn(),
  listTriggers: vi.fn(),
  createCronJob: vi.fn(),
  updateCronJob: vi.fn(),
  deleteCronJob: vi.fn(),
  toggleCronJob: vi.fn(),
  createTrigger: vi.fn(),
  updateTrigger: vi.fn(),
  deleteTrigger: vi.fn(),
  patchAgent: vi.fn(),
}));

const agent: AgentDetail = {
  id: "00000000-0000-0000-0000-000000000001",
  name: "test-agent",
  schedule: "manual",
};

function withQueryClient(node: ReactNode) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, structuralSharing: false } },
  });
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>);
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("AgentSchedulePanel — read state", () => {
  it("renders the manual mode card by default and shows empty-state hints", async () => {
    vi.mocked(http.listCronJobs).mockResolvedValue([]);
    vi.mocked(http.listTriggers).mockResolvedValue([]);

    withQueryClient(<AgentSchedulePanel agent={agent} />);

    expect(await screen.findByText("Manual")).toBeInTheDocument();
    expect(await screen.findByText("No cron jobs")).toBeInTheDocument();
    // Triggers section's empty state varies by schedule mode; with a
    // reactive agent we surface the "wakes on incoming messages only" hint.
    expect(
      await screen.findByText(
        "No triggers — agent wakes on incoming messages only",
      ),
    ).toBeInTheDocument();
  });

  it("renders the continuous mode label and interval from the agent.schedule string", async () => {
    vi.mocked(http.listCronJobs).mockResolvedValue([]);
    vi.mocked(http.listTriggers).mockResolvedValue([]);
    withQueryClient(
      <AgentSchedulePanel
        agent={{ ...agent, schedule: "continuous · 180s" }}
      />,
    );
    // The mode card shows "Continuous (180s)" — the parenthesised number
    // is parsed out of the human-readable summary the backend hands us
    // on AgentDetail.schedule.
    expect(await screen.findByText("Continuous (180s)")).toBeInTheDocument();
  });

  it("lists existing cron jobs with name + schedule expression", async () => {
    const job: CronJobItem = {
      id: "job-1",
      name: "daily-summary",
      enabled: true,
      agent_id: agent.id,
      schedule: { kind: "cron", expr: "0 9 * * *", tz: "UTC" },
      action: { kind: "agent_turn", message: "morning" },
    };
    vi.mocked(http.listCronJobs).mockResolvedValue([job]);
    vi.mocked(http.listTriggers).mockResolvedValue([]);

    withQueryClient(<AgentSchedulePanel agent={agent} />);
    expect(await screen.findByText("daily-summary")).toBeInTheDocument();
    // Schedule expression is rendered with the optional timezone suffix.
    expect(await screen.findByText("0 9 * * * UTC")).toBeInTheDocument();
  });

  it("lists existing triggers with the formatted event pattern", async () => {
    const trigger: TriggerItem = {
      id: "trig-1",
      agent_id: agent.id,
      pattern: "lifecycle",
      prompt_template: "Greet new arrivals.",
      enabled: true,
    };
    vi.mocked(http.listCronJobs).mockResolvedValue([]);
    vi.mocked(http.listTriggers).mockResolvedValue([trigger]);

    withQueryClient(<AgentSchedulePanel agent={agent} />);
    expect(await screen.findByText("lifecycle")).toBeInTheDocument();
    expect(await screen.findByText("Greet new arrivals.")).toBeInTheDocument();
  });
});

describe("AgentSchedulePanel — mode toggles", () => {
  it('switches to continuous mode via PATCH /api/agents/{id} { schedule: { continuous: ... } }', async () => {
    vi.mocked(http.listCronJobs).mockResolvedValue([]);
    vi.mocked(http.listTriggers).mockResolvedValue([]);
    vi.mocked(http.patchAgent).mockResolvedValue({ status: "ok" });

    withQueryClient(<AgentSchedulePanel agent={agent} />);

    const btn = await screen.findByRole("button", {
      name: "Switch to continuous",
    });
    await userEvent.click(btn);
    await waitFor(() => {
      expect(http.patchAgent).toHaveBeenCalledWith(agent.id, {
        schedule: { continuous: { check_interval_secs: 120 } },
      });
    });
  });

  it('switches back to manual via PATCH /api/agents/{id} { schedule: "reactive" }', async () => {
    vi.mocked(http.listCronJobs).mockResolvedValue([]);
    vi.mocked(http.listTriggers).mockResolvedValue([]);
    vi.mocked(http.patchAgent).mockResolvedValue({ status: "ok" });

    withQueryClient(
      <AgentSchedulePanel
        agent={{ ...agent, schedule: "continuous · 120s" }}
      />,
    );

    const btn = await screen.findByRole("button", { name: "Switch to manual" });
    await userEvent.click(btn);
    await waitFor(() => {
      expect(http.patchAgent).toHaveBeenCalledWith(agent.id, {
        schedule: "reactive",
      });
    });
  });
});

// Drawer-form CRUD (cron / trigger create + edit) is not exercised here.
// `DrawerPanel` pushes its body into a global drawer slot owned by
// `<PushDrawer>` rather than rendering into the local subtree, so the
// inputs aren't in the test DOM. The drawer host has its own dedicated
// test suite (`src/components/ui/PushDrawer.test.tsx`,
// `src/components/ui/DrawerPanel.test.tsx`); the wire-up here is verified
// indirectly via the mutation invalidation tests in
// `src/lib/mutations/schedules.test.tsx`. Live drawer flow is covered by
// the integration-level Playwright pass over the agent detail panel.
