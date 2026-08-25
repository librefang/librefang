/**
 * Regression cover for #7387 — what the Skills page shows when a marketplace is dead.
 *
 * The hub in the fixture answers the way the real Skillhub does today: `200` with an HTML page, which the daemon fails to parse and reports as an upstream error.
 * Three things have to be true and none of them were: the hub pill must not claim to be live before anyone has contacted it, selecting the dead hub must produce an offline surface instead of a generic load failure with the parser's complaint under it, and "All hubs" must say a hub dropped out rather than silently omitting it.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClientProvider } from "@tanstack/react-query";
import { createTestQueryClient } from "../lib/test/query-client";
import { ApiError } from "../lib/http/errors";
import { SkillsPage } from "./SkillsPage";
import * as httpClient from "../lib/http/client";

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
  };
});

// Render translation keys plus their interpolation values, so an assertion can
// tell "SkillHub is live" from "SkillHub has not been checked" — the whole
// point of the bug — without depending on the English copy.
vi.mock("react-i18next", async () => {
  const actual =
    await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) => {
        const params = Object.entries(opts ?? {}).filter(
          ([k]) => k !== "defaultValue",
        );
        return params.length
          ? `${key}(${params.map(([k, v]) => `${k}=${String(v)}`).join(",")})`
          : key;
      },
    }),
  };
});

/**
 * The error a dead Skillhub produces today, verbatim in shape.
 *
 * The URL matters: `isRateLimitError` matches the bare substring `"rate"`, which `cos.accelerate.…` contains, so a marketplace-unavailable branch placed after the rate-limit check would misreport this as throttling.
 */
const DEAD_HUB_ERROR = new ApiError(
  502,
  "HTTP_502",
  "Network error: Skillhub index parse error: https://skillhub-1388575217.cos.accelerate.myqcloud.com/skills.json — expected value at line 1 column 1",
);

function renderPage() {
  const queryClient = createTestQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <SkillsPage />
    </QueryClientProvider>,
  );
}

function skillHubPill() {
  return screen.getByRole("button", { name: /SkillHub/ });
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(httpClient.listSkills).mockResolvedValue([]);
  vi.mocked(httpClient.listHands).mockResolvedValue([]);
  vi.mocked(httpClient.listPendingCandidates).mockResolvedValue([]);
  vi.mocked(httpClient.fanghubListSkills).mockResolvedValue({ skills: [], total: 0 });
  vi.mocked(httpClient.clawhubSearch).mockResolvedValue({ items: [] });
  vi.mocked(httpClient.clawhubCnSearch).mockResolvedValue({ items: [] });
  vi.mocked(httpClient.skillhubBrowse).mockRejectedValue(DEAD_HUB_ERROR);
  vi.mocked(httpClient.skillhubSearch).mockRejectedValue(DEAD_HUB_ERROR);
});

describe("SkillsPage hub availability", () => {
  it("does not report an un-queried hub as live", async () => {
    renderPage();

    // The page lands on FangHub, so no Skillhub request has been made.
    await waitFor(() => expect(httpClient.fanghubListSkills).toHaveBeenCalled());
    expect(httpClient.skillhubBrowse).not.toHaveBeenCalled();

    const pill = skillHubPill();
    expect(
      within(pill).getByText(
        "skills.hub_status(hub=SkillHub,status=skills.hub_not_checked)",
      ),
    ).toBeInTheDocument();
    expect(
      within(pill).queryByText(
        "skills.hub_status(hub=SkillHub,status=skills.hub_live)",
      ),
    ).not.toBeInTheDocument();
  });

  it("renders the unavailable surface, not a raw load error, for a dead hub", async () => {
    const user = userEvent.setup();
    renderPage();

    await user.click(skillHubPill());

    await waitFor(() =>
      expect(
        screen.getByText("skills.hub_unavailable"),
      ).toBeInTheDocument(),
    );
    expect(
      screen.getByText("skills.hub_unavailable_desc(hub=SkillHub)"),
    ).toBeInTheDocument();
    expect(screen.queryByText("skills.load_error")).not.toBeInTheDocument();
    expect(screen.queryByText("skills.rate_limited")).not.toBeInTheDocument();
    expect(screen.queryByText(DEAD_HUB_ERROR.message)).not.toBeInTheDocument();

    expect(
      within(skillHubPill()).getByText(
        "skills.hub_status(hub=SkillHub,status=skills.hub_unreachable)",
      ),
    ).toBeInTheDocument();
  });

  it("names the dead hub under All hubs instead of dropping it silently", async () => {
    const user = userEvent.setup();
    renderPage();

    await user.click(screen.getByRole("button", { name: /skills.all_hubs/ }));

    await waitFor(() =>
      expect(
        screen.getByText("skills.hub_unavailable_all(hubs=SkillHub)"),
      ).toBeInTheDocument(),
    );
  });
});
