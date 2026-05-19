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

// Regression — Codex P2 review on PR #5256.
//
// `format_schedule_mode` in `routes/agents.rs` returns:
//   ScheduleMode::Reactive             → "manual"
//   ScheduleMode::Periodic { cron }    → the cron string itself
//   ScheduleMode::Proactive { .. }     → "proactive"
//   ScheduleMode::Continuous { .. }    → "continuous · Ns"
//
// Pre-fix the panel collapsed periodic/proactive into the "manual" branch,
// which then offered a "Switch to continuous" button that would silently
// clobber the manifest-driven schedule. We now render the actual mode and
// hide the toggle to keep the manifest the source of truth for those modes.
describe("AgentSchedulePanel — non-continuous schedule modes", () => {
  beforeEach(() => {
    vi.mocked(http.listCronJobs).mockResolvedValue([]);
    vi.mocked(http.listTriggers).mockResolvedValue([]);
  });

  it("renders a periodic schedule with the cron expression, no switch-to-continuous offered", async () => {
    withQueryClient(
      <AgentSchedulePanel agent={{ ...agent, schedule: "0 9 * * *" }} />,
    );
    expect(await screen.findByText("Periodic (0 9 * * *)")).toBeInTheDocument();
    // The "manifest-controlled" marker replaces the toggle so users can't
    // accidentally overwrite a periodic schedule with a continuous one.
    expect(await screen.findByText("manifest-controlled")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Switch to continuous" }),
    ).not.toBeInTheDocument();
  });

  it("renders a proactive schedule with no switch-to-continuous offered", async () => {
    withQueryClient(
      <AgentSchedulePanel agent={{ ...agent, schedule: "proactive" }} />,
    );
    expect(await screen.findByText("Proactive")).toBeInTheDocument();
    expect(await screen.findByText("manifest-controlled")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Switch to continuous" }),
    ).not.toBeInTheDocument();
  });
});

// Regression — Codex P2 review on PR #5256.
//
// `TriggerPattern::AgentSpawned { name_pattern }` is a struct variant: serde
// rejects the bare string `"agent_spawned"` because the required
// `name_pattern` field is missing. The preset must therefore serialise the
// object shape `{"agent_spawned":{"name_pattern":"*"}}` so the backend
// `create_trigger` route accepts it.
//
// The preset list lives inside `AgentSchedulePanel.tsx` as a module-private
// const and the drawer that renders it is hoisted into a global slot
// (see the long comment at the end of this file), so we can't reach the
// `<option>` through the rendered DOM here. We pin the contract via the
// component source file instead: a fixture-style read of the file with a
// targeted regex catches both regressions on this preset.
//
// This is a static contract: if a future refactor swaps the preset value
// back to `'"agent_spawned"'` the JSON.parse below will produce the bare
// string and fail the deep-equal check below.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

describe("AgentSchedulePanel — trigger pattern preset wire shape", () => {
  it("encodes the agent_spawned preset as the struct-variant object shape, not a bare string", () => {
    const here = dirname(fileURLToPath(import.meta.url));
    const src = readFileSync(join(here, "AgentSchedulePanel.tsx"), "utf8");
    const presetMatch = src.match(
      /label:\s*"agent_spawned[^"]*",\s*\n?\s*value:\s*([^,\n]+)/,
    );
    expect(presetMatch, "agent_spawned preset not found").toBeTruthy();
    const valueLiteral = presetMatch![1].trim();
    // The value is a single-quoted JS string literal wrapping a JSON
    // payload; strip the outer JS quotes before parsing.
    const jsonStr = valueLiteral.replace(/^['"]/, "").replace(/['"]$/, "");
    const parsed = JSON.parse(jsonStr);
    expect(parsed).toEqual({ agent_spawned: { name_pattern: "*" } });
  });
});

// Regression — round-2 review on PR #5256.
//
// Round-1 left an open concern: the cron edit form's only schedule input
// is a single cron-expression field. If the user clicks the pencil on an
// `every` or `at` cron job, the form opens with that field cleared and a
// submit would silently rewrite the schedule to a `{kind: "cron"}` shape.
// The simplest fix is to refuse to open the edit form for non-cron
// schedule kinds. The button still renders (so the row layout is stable)
// but is disabled with a tooltip pointing the user at agent.toml.
describe("AgentSchedulePanel — non-cron schedule kinds are not editable in the UI", () => {
  beforeEach(() => {
    vi.mocked(http.listTriggers).mockResolvedValue([]);
  });

  it("disables the pencil button for `every` cron jobs", async () => {
    const job: CronJobItem = {
      id: "job-every-1",
      name: "ping-loop",
      enabled: true,
      agent_id: agent.id,
      schedule: { kind: "every", every_secs: 60 },
      action: { kind: "agent_turn", message: "tick" },
    };
    vi.mocked(http.listCronJobs).mockResolvedValue([job]);

    withQueryClient(<AgentSchedulePanel agent={agent} />);
    expect(await screen.findByText("ping-loop")).toBeInTheDocument();
    // The row renders three icon-only buttons (toggle is the badge, then
    // pencil, then trash). The pencil for an `every` schedule must be
    // disabled to prevent the silent kind conversion described above.
    const row = (await screen.findByText("ping-loop")).closest("div.flex");
    // Walk up to the row container.
    const card = row?.parentElement?.parentElement;
    expect(card).toBeTruthy();
    const buttons = card!.querySelectorAll("button");
    // Find the disabled pencil — it carries the tooltip with the
    // user-visible reason. Asserting on `title` keeps the test resilient
    // if the SVG icon mapping changes.
    const pencil = Array.from(buttons).find(
      (b) =>
        (b as HTMLButtonElement).title?.includes("every") ||
        (b as HTMLButtonElement).title?.includes("at"),
    ) as HTMLButtonElement | undefined;
    expect(pencil, "pencil button with non-cron tooltip not found").toBeTruthy();
    expect(pencil!.disabled).toBe(true);
  });

  it("disables the pencil button for `at` cron jobs", async () => {
    const job: CronJobItem = {
      id: "job-at-1",
      name: "one-shot",
      enabled: true,
      agent_id: agent.id,
      schedule: { kind: "at", at: "2030-01-01T00:00:00Z" },
      action: { kind: "agent_turn", message: "fire once" },
    };
    vi.mocked(http.listCronJobs).mockResolvedValue([job]);

    withQueryClient(<AgentSchedulePanel agent={agent} />);
    expect(await screen.findByText("one-shot")).toBeInTheDocument();
    const card = (await screen.findByText("one-shot")).closest("div.flex")
      ?.parentElement?.parentElement;
    const buttons = card!.querySelectorAll("button");
    const pencil = Array.from(buttons).find(
      (b) =>
        (b as HTMLButtonElement).title?.includes("every") ||
        (b as HTMLButtonElement).title?.includes("at"),
    ) as HTMLButtonElement | undefined;
    expect(pencil, "pencil button with non-cron tooltip not found").toBeTruthy();
    expect(pencil!.disabled).toBe(true);
  });

  it("keeps the pencil button enabled for `cron` cron jobs", async () => {
    const job: CronJobItem = {
      id: "job-cron-1",
      name: "daily-summary",
      enabled: true,
      agent_id: agent.id,
      schedule: { kind: "cron", expr: "0 9 * * *", tz: "UTC" },
      action: { kind: "agent_turn", message: "morning" },
    };
    vi.mocked(http.listCronJobs).mockResolvedValue([job]);

    withQueryClient(<AgentSchedulePanel agent={agent} />);
    expect(await screen.findByText("daily-summary")).toBeInTheDocument();
    const card = (await screen.findByText("daily-summary")).closest("div.flex")
      ?.parentElement?.parentElement;
    const buttons = card!.querySelectorAll("button");
    // The pencil for a `cron` row must NOT carry the non-cron tooltip
    // and must NOT be disabled.
    const disabledPencil = Array.from(buttons).find(
      (b) =>
        (b as HTMLButtonElement).title?.includes("every") ||
        (b as HTMLButtonElement).title?.includes("at"),
    );
    expect(disabledPencil).toBeFalsy();
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
