import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { createTestQueryClient } from "../lib/test/query-client";
import { SkillsPage } from "./SkillsPage";
import { PushDrawer } from "../components/ui/PushDrawer";
import * as httpClient from "../lib/http/client";
import { useUIStore } from "../lib/store";
import type { SkillDetail, SkillHistoryEntry, SkillItem } from "../api";

/**
 * The version-history half of #8021: the Skills drawer renders the skill
 * version store's snapshots — not the legacy file-based `.evolution.json`
 * list — and restoring one overwrites `skill.toml` and `prompt_context.md`
 * on disk, so the action must reach a destructive confirmation that names
 * the version before the mutation fires.
 *
 * The page is rendered whole and the client layer is mocked per function so
 * the real `useRestoreSkillVersion` hook (and its invalidation) sits between
 * the click and the mocked endpoint.
 */

vi.mock("../lib/http/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/http/client")>();
  return {
    ...actual,
    listSkills: vi.fn(),
    listHands: vi.fn(),
    listPendingCandidates: vi.fn(),
    fanghubListSkills: vi.fn(),
    skillhubBrowse: vi.fn(),
    skillhubSearch: vi.fn(),
    clawhubSearch: vi.fn(),
    clawhubCnSearch: vi.fn(),
    getSkillDetail: vi.fn(),
    getSkillHistory: vi.fn(),
    restoreSkillVersion: vi.fn(),
  };
});

// motion/react drives DrawerPanel and ConfirmDialog through async animation
// hooks that don't settle in jsdom. Stub them so render is synchronous.
vi.mock("motion/react", async () => {
  const React = await import("react");
  return {
    AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
    motion: new Proxy(
      {},
      {
        get:
          (_target, prop: string) =>
          ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
            React.createElement(prop, rest, children),
      },
    ),
  };
});

// Resolve keys against the real `en.json` rather than echoing them back, so
// the assertions tie the rows to the shipped labels (the change-source badge
// uses `skills.change_source.*`; the dialog uses `skills.evo_restore_confirm`).
vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  const en = (await import("../locales/en.json")).default as Record<string, unknown>;
  const lookup = (key: string): unknown =>
    key
      .split(".")
      .reduce<unknown>(
        (node, part) =>
          node && typeof node === "object" ? (node as Record<string, unknown>)[part] : undefined,
        en,
      );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) => {
        const hit = lookup(key);
        const template =
          typeof hit === "string" ? hit : ((opts?.defaultValue as string | undefined) ?? key);
        return template.replace(/\{\{(\w+)\}\}/g, (_m, name: string) => String(opts?.[name] ?? ""));
      },
    }),
  };
});

const SKILL: SkillItem = {
  name: "csv-analysis",
  version: "0.2.0",
  description: "Parses CSVs",
  author: "operator",
};

const DETAIL: SkillDetail = {
  name: "csv-analysis",
  version: "0.2.0",
  description: "Parses CSVs",
  author: "operator",
  license: "MIT",
  tags: [],
  runtime: "native",
  tools: [],
  required_tools: [],
  required_capabilities: [],
  has_prompt_context: true,
  prompt_context_length: 12,
  prompt_context: "be helpful",
  source: null,
  enabled: true,
  path: "/home/operator/.librefang/skills/csv-analysis",
  linked_files: {},
  evolution: {
    versions: [
      {
        version: "0.1.0",
        timestamp: "2026-08-01T00:00:00Z",
        changelog: "Initial draft from the file-based evolution log",
        content_hash: "hash",
      },
    ],
    use_count: 3,
    evolution_count: 2,
    mutation_count: 1,
  },
};

// Stored naive-UTC in SQLite's `YYYY-MM-DD HH:MM:SS` shape, exactly as the
// history endpoint returns it.
const EVOLVE_VERSION: SkillHistoryEntry = {
  id: 42,
  skill_name: "csv-analysis",
  timestamp: "2026-10-14 22:07:53",
  manifest_toml: 'name = "csv-analysis"\nversion = "0.2.0"\n',
  prompt_context: "Evolved instructions",
  author: "dashboard",
  change_source: "evolve",
};

const FILE_VERSION: SkillHistoryEntry = {
  id: 41,
  skill_name: "csv-analysis",
  timestamp: "2026-10-13 08:01:02",
  manifest_toml: 'name = "csv-analysis"\nversion = "0.1.0"\n',
  prompt_context: "",
  author: "agent:main",
  change_source: "file-write",
};

/** The list label and the dialog both read the same stored timestamp. */
function stamp(version: SkillHistoryEntry): string {
  return new Date(`${version.timestamp.replace(" ", "T")}Z`).toLocaleString();
}

function renderPage() {
  const queryClient = createTestQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <SkillsPage />
      {/* DrawerPanel pushes its body into the global store; PushDrawer is
          App.tsx's slot host. Without it the drawer body would never mount. */}
      <PushDrawer />
    </QueryClientProvider>,
  );
}

async function openDrawer() {
  renderPage();
  fireEvent.click(await screen.findByText("csv-analysis"));
  await screen.findByText("Evolved");
}

beforeEach(() => {
  vi.clearAllMocks();
  useUIStore.setState({ toasts: [] });
  // The page reads `?tab=` on mount; the installed view is the one with the
  // drawer the history lives in.
  window.history.pushState({}, "", "/?tab=installed");
  vi.mocked(httpClient.listSkills).mockResolvedValue([SKILL]);
  vi.mocked(httpClient.listHands).mockResolvedValue([]);
  vi.mocked(httpClient.listPendingCandidates).mockResolvedValue([]);
  vi.mocked(httpClient.fanghubListSkills).mockResolvedValue({ skills: [], total: 0 });
  vi.mocked(httpClient.getSkillDetail).mockResolvedValue(DETAIL);
  vi.mocked(httpClient.getSkillHistory).mockResolvedValue([EVOLVE_VERSION, FILE_VERSION]);
  vi.mocked(httpClient.restoreSkillVersion).mockResolvedValue({
    status: "ok",
    skill_name: "csv-analysis",
    restored_version_id: EVOLVE_VERSION.id,
  });
});

describe("SkillsPage version history", () => {
  it("lists store versions with labelled change sources instead of the raw tokens", async () => {
    await openDrawer();

    const evolve = screen.getByText("Evolved");
    // The raw value stays reachable for anyone matching a row against the database or the API response.
    expect(evolve).toHaveAttribute("title", "evolve");
    expect(screen.getByText("File written")).toHaveAttribute("title", "file-write");
    expect(screen.queryByText("evolve")).toBeNull();
    expect(screen.queryByText("file-write")).toBeNull();
    // Author and timestamp stay visible on the row.
    expect(screen.getByText("agent:main")).toBeInTheDocument();
    expect(screen.getByText(stamp(FILE_VERSION))).toBeInTheDocument();
  });

  it("renders the store history rather than the legacy file-based evolution list", async () => {
    await openDrawer();

    // The `.evolution.json` changelog still travels in the detail payload, but
    // the drawer no longer renders it as a second, contentless version list;
    // the store snapshot is the canonical row (it carries the files and the
    // restore action), so two near-identical time-ordered lists would only
    // duplicate each other.
    expect(
      screen.queryByText("Initial draft from the file-based evolution log"),
    ).toBeNull();
    expect(screen.getByText("Evolved")).toBeInTheDocument();
  });

  it("expands a version to read the stored manifest and prompt context", async () => {
    await openDrawer();

    expect(screen.queryByText(/version = "0.2.0"/)).toBeNull();

    fireEvent.click(screen.getAllByRole("button", { name: "Expand" })[0]);

    expect(screen.getByText(/version = "0.2.0"/)).toBeInTheDocument();
    expect(screen.getByText("Evolved instructions")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Collapse" }));
    expect(screen.queryByText(/version = "0.2.0"/)).toBeNull();
  });

  it("restores only after the destructive confirmation names the version", async () => {
    await openDrawer();

    fireEvent.click(screen.getAllByRole("button", { name: "Restore" })[0]);
    expect(httpClient.restoreSkillVersion).not.toHaveBeenCalled();

    // The history list is a column of near-identical rows, so the dialog has
    // to say *which* version it is about to write over the skill's files.
    const message = screen.getByText(/Restore 'csv-analysis' to the version saved/);
    expect(message).toHaveTextContent(stamp(EVOLVE_VERSION));
    expect(message).toHaveTextContent("(Evolved)");
    expect(message).not.toHaveTextContent("(evolve)");

    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));

    await waitFor(() =>
      expect(httpClient.restoreSkillVersion).toHaveBeenCalledWith("csv-analysis", 42),
    );
    await waitFor(() =>
      expect(useUIStore.getState().toasts.map((toast) => toast.message)).toContain(
        "Version restored",
      ),
    );
  });

  it("writes nothing when the restore confirmation is cancelled", async () => {
    await openDrawer();

    fireEvent.click(screen.getAllByRole("button", { name: "Restore" })[0]);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(httpClient.restoreSkillVersion).not.toHaveBeenCalled();
    expect(screen.queryByText(/Restore 'csv-analysis' to the version saved/)).toBeNull();
    // The drawer itself stays open — cancelling the dialog is not cancelling the browse.
    expect(screen.getByText("Evolved")).toBeInTheDocument();
  });
});
