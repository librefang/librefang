import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ChatPage } from "./ChatPage";
import { useAgents } from "../lib/queries/agents";
import { useModels } from "../lib/queries/models";
import { useFullConfig } from "../lib/queries/config";

vi.mock("../lib/queries/agents", () => ({
  useAgents: vi.fn(),
  useAgentSessions: vi.fn(),
}));

vi.mock("../lib/queries/models", () => ({
  useModels: vi.fn(),
}));

vi.mock("../lib/queries/config", () => ({
  useFullConfig: vi.fn(),
}));

vi.mock("../lib/queries/approvals", () => ({
  usePendingApprovals: vi.fn(),
}));

vi.mock("../lib/queries/media", () => ({
  useMediaProviders: vi.fn(),
}));

vi.mock("../lib/queries/sessions", () => ({
  useSessionStream: vi.fn(),
}));

vi.mock("../lib/queries/hands", () => ({
  useActiveHandsWhen: vi.fn(),
}));

vi.mock("../lib/mutations/agents", () => ({
  useCreateAgentSession: vi.fn(),
  useDeleteAgentSession: vi.fn(),
  usePatchAgentConfig: vi.fn(),
  usePatchHandAgentRuntimeConfig: vi.fn(),
  useResolveApproval: vi.fn(),
  useStopAgent: vi.fn(),
  useUploadAgentFile: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  useSearch: () => ({}),
}));

vi.mock("@tanstack/react-query", async () => {
  const actual = await vi.importActual("@tanstack/react-query");
  return {
    ...actual,
    useQueryClient: () => ({
      invalidateQueries: vi.fn(),
    }),
  };
});

const createTestQueryClient = () =>
  new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        staleTime: 0,
      },
    },
  });

describe("ChatPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  const renderWithQueryClient = (ui: React.ReactElement) => {
    const queryClient = createTestQueryClient();
    return render(
      <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>
    );
  };

  it("renders loading state correctly", async () => {
    (useAgents as ReturnType<typeof vi.fn>).mockReturnValue({
      data: null,
      isLoading: true,
    });
    (useModels as ReturnType<typeof vi.fn>).mockReturnValue({
      data: null,
      isLoading: true,
    });
    (useFullConfig as ReturnType<typeof vi.fn>).mockReturnValue({
      data: null,
      isLoading: true,
    });

    renderWithQueryClient(<ChatPage />);

    await waitFor(() => {
      expect(screen.getByRole("main")).toBeInTheDocument();
    });
  });

  it("renders empty state when no agents exist", async () => {
    (useAgents as ReturnType<typeof vi.fn>).mockReturnValue({
      data: [],
      isLoading: false,
    });
    (useModels as ReturnType<typeof vi.fn>).mockReturnValue({
      data: [],
      isLoading: false,
    });
    (useFullConfig as ReturnType<typeof vi.fn>).mockReturnValue({
      data: {},
      isLoading: false,
    });

    renderWithQueryClient(<ChatPage />);

    await waitFor(() => {
      expect(screen.getByRole("main")).toBeInTheDocument();
    });
  });

  it("renders with agents data", async () => {
    const mockAgents = [
      { id: "agent-1", name: "Test Agent", status: "active" },
    ];

    (useAgents as ReturnType<typeof vi.fn>).mockReturnValue({
      data: mockAgents,
      isLoading: false,
    });
    (useModels as ReturnType<typeof vi.fn>).mockReturnValue({
      data: [],
      isLoading: false,
    });
    (useFullConfig as ReturnType<typeof vi.fn>).mockReturnValue({
      data: {},
      isLoading: false,
    });

    renderWithQueryClient(<ChatPage />);

    await waitFor(() => {
      expect(screen.getByRole("main")).toBeInTheDocument();
    });
  });
});