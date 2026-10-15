import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SessionsPage } from "./SessionsPage";
import { useAgentAvatarUrl, useAgents } from "../lib/queries/agents";
import { useSessions } from "../lib/queries/sessions";
import { useDeleteAgentSession } from "../lib/mutations/agents";
import { useSetSessionLabel } from "../lib/mutations/sessions";

vi.mock("../lib/queries/agents", () => ({
  useAgents: vi.fn(),
  // Each session row renders `AgentAvatar`, which resolves the agent's
  // avatar image through this hook. Without it in the mock, rendering a row
  // with a resolvable agent throws.
  useAgentAvatarUrl: vi.fn(),
}));

vi.mock("../lib/queries/sessions", () => ({
  useSessions: vi.fn(),
}));

vi.mock("../lib/mutations/agents", () => ({
  useDeleteAgentSession: vi.fn(),
}));

vi.mock("../lib/mutations/sessions", () => ({
  useSetSessionLabel: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) =>
        (opts?.defaultValue as string | undefined) ?? key,
      i18n: { language: "en" },
    }),
  };
});

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
}));

vi.mock("../lib/store", () => ({
  useUIStore: (
    selector: (state: { addToast: (m: string, t?: string) => void }) => unknown,
  ) => selector({ addToast: vi.fn() }),
}));

const useAgentsMock = useAgents as unknown as ReturnType<typeof vi.fn>;
const mockAvatarUrl = useAgentAvatarUrl as unknown as ReturnType<typeof vi.fn>;
const useSessionsMock = useSessions as unknown as ReturnType<typeof vi.fn>;
const useDeleteAgentSessionMock =
  useDeleteAgentSession as unknown as ReturnType<typeof vi.fn>;
const useSetSessionLabelMock =
  useSetSessionLabel as unknown as ReturnType<typeof vi.fn>;

const HAND_AGENT_ID = "11111111-1111-1111-1111-111111111111";
const HAND_SESSION_ID = "22222222-2222-2222-2222-222222222222";

function renderPage() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <SessionsPage />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  // `undefined` is what the hook returns while the blob is in flight and when
  // the agent has no image at all — both by design.
  mockAvatarUrl.mockReset();
  mockAvatarUrl.mockReturnValue(undefined);
  useDeleteAgentSessionMock.mockReturnValue({
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
  });
  useSetSessionLabelMock.mockReturnValue({ mutate: vi.fn(), isPending: false });
  useSessionsMock.mockReturnValue({
    data: [
      {
        session_id: HAND_SESSION_ID,
        agent_id: HAND_AGENT_ID,
        created_at: "2026-06-17T00:00:00Z",
        active: false,
      },
    ],
    isLoading: false,
    isError: false,
    isFetching: false,
    refetch: vi.fn(),
    truncated: false,
  });
});

describe("SessionsPage hand agent name (#6156)", () => {
  it("requests the agent list with hand agents included", () => {
    useAgentsMock.mockReturnValue({ data: [], isLoading: false, isError: false });
    renderPage();
    // The bare `/api/agents` list excludes hand agents; the sessions view
    // must opt in so a hand-owned session can resolve its agent name.
    expect(useAgentsMock).toHaveBeenCalledWith({ includeHands: true });
  });

  it("renders the hand agent's real name for a hand-owned session", () => {
    useAgentsMock.mockReturnValue({
      data: [{ id: HAND_AGENT_ID, name: "My Hand Agent", is_hand: true }],
      isLoading: false,
      isError: false,
    });
    renderPage();
    expect(screen.getByText("My Hand Agent")).toBeInTheDocument();
    expect(screen.queryByText("sessions.unknown_agent")).not.toBeInTheDocument();
  });

  it("falls back to the unknown-agent label when the agent is missing", () => {
    // Regression guard: if the hand agent were filtered out (the pre-fix
    // behaviour), the lookup misses and the unknown label is shown.
    useAgentsMock.mockReturnValue({ data: [], isLoading: false, isError: false });
    renderPage();
    expect(screen.getByText("sessions.unknown_agent")).toBeInTheDocument();
  });
});

describe("SessionsPage agent avatar (#8339)", () => {
  const AVATAR = `/api/agents/${HAND_AGENT_ID}/avatar`;

  it("renders the agent's avatar, resolving its image through AgentAvatar", () => {
    useAgentsMock.mockReturnValue({
      data: [
        {
          id: HAND_AGENT_ID,
          name: "My Hand Agent",
          is_hand: true,
          identity: { avatar_url: AVATAR, emoji: "🤖" },
        },
      ],
      isLoading: false,
      isError: false,
    });
    mockAvatarUrl.mockReturnValue("blob:test/1");

    renderPage();

    // `Avatar` carries the agent's name as its accessible label, so the row
    // is no longer a bare initial: identity renders image → emoji → initials.
    const avatar = screen.getByRole("img", { name: "My Hand Agent" });
    expect(avatar.querySelector("img")).toHaveAttribute("src", "blob:test/1");
    // `identity.avatar_url` is present, so the image must actually be fetched.
    expect(mockAvatarUrl).toHaveBeenCalledWith(HAND_AGENT_ID, true);
  });

  it("shows the agent's emoji while it has no avatar image", () => {
    useAgentsMock.mockReturnValue({
      data: [
        {
          id: HAND_AGENT_ID,
          name: "My Hand Agent",
          is_hand: true,
          identity: { emoji: "🤖" },
        },
      ],
      isLoading: false,
      isError: false,
    });

    renderPage();

    expect(screen.getByRole("img", { name: "My Hand Agent" })).toHaveTextContent("🤖");
    // No avatar_url on the identity, so the row must not cost a request for one.
    expect(mockAvatarUrl).toHaveBeenCalledWith(HAND_AGENT_ID, false);
  });

  it("keeps the Users fallback when the session has no resolvable agent", () => {
    useAgentsMock.mockReturnValue({ data: [], isLoading: false, isError: false });

    const { container } = renderPage();

    expect(screen.queryByRole("img")).not.toBeInTheDocument();
    // `lucide-users` ties this to lucide's internal naming; the tile carries
    // no other stable marker to select the fallback icon by.
    expect(container.querySelector("svg.lucide-users")).toBeInTheDocument();
  });

  it("keeps the active dot pinned to the avatar's corner", () => {
    useAgentsMock.mockReturnValue({
      data: [{ id: HAND_AGENT_ID, name: "My Hand Agent", is_hand: true }],
      isLoading: false,
      isError: false,
    });
    useSessionsMock.mockReturnValue({
      data: [
        {
          session_id: HAND_SESSION_ID,
          agent_id: HAND_AGENT_ID,
          created_at: "2026-06-17T00:00:00Z",
          active: true,
        },
      ],
      isLoading: false,
      isError: false,
      isFetching: false,
      refetch: vi.fn(),
      truncated: false,
    });

    renderPage();

    // The dot lives on the wrapper around the avatar, the way `AgentsPage`
    // and the chat agent picker anchor it — not inside the avatar itself.
    const avatar = screen.getByRole("img", { name: "My Hand Agent" });
    const dot = avatar.parentElement!.querySelector("span.animate-pulse");
    expect(dot).toBeInTheDocument();
    expect(dot).toHaveClass("absolute", "-bottom-0.5", "-right-0.5", "bg-success");
  });
});
