import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SessionsPage } from "./SessionsPage";
import { useAgents } from "../lib/queries/agents";
import { useSessions } from "../lib/queries/sessions";
import { useDeleteAgentSession } from "../lib/mutations/agents";
import { useSetSessionLabel } from "../lib/mutations/sessions";

vi.mock("../lib/queries/agents", () => ({
  useAgents: vi.fn(),
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
