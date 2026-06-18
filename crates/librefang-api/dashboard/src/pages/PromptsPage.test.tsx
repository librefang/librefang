import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { PromptsPage } from "./PromptsPage";
import { usePromptsOverview } from "../lib/queries/prompts";
import { usePromptVersions } from "../lib/queries/agents";
import {
  useCreatePromptVersionForRepo,
  useDeletePromptVersionForRepo,
  useBindPromptVersionToAgent,
} from "../lib/mutations/prompts";
import type { PromptOverviewItem, PromptVersion } from "../api";

// The page reads the fleet overview through `usePromptsOverview`, drills into
// one agent's version history through `usePromptVersions`, and mutates through
// the three repository mutation hooks. Mock all five so each test can drive a
// specific UI state without a network or a real kernel.

vi.mock("../lib/queries/prompts", () => ({
  usePromptsOverview: vi.fn(),
}));

vi.mock("../lib/queries/agents", () => ({
  usePromptVersions: vi.fn(),
}));

vi.mock("../lib/mutations/prompts", () => ({
  useCreatePromptVersionForRepo: vi.fn(),
  useDeletePromptVersionForRepo: vi.fn(),
  useBindPromptVersionToAgent: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual =
    await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) => {
        if (opts && typeof opts === "object") {
          if ("defaultValue" in opts && typeof opts.defaultValue === "string") {
            return key;
          }
          if ("version" in opts) return `${key}:${opts.version}`;
          if ("count" in opts) return `${key}:${opts.count}`;
        }
        return key;
      },
    }),
  };
});

const useOverviewMock = usePromptsOverview as unknown as ReturnType<
  typeof vi.fn
>;
const useVersionsMock = usePromptVersions as unknown as ReturnType<typeof vi.fn>;
const useCreateMock = useCreatePromptVersionForRepo as unknown as ReturnType<
  typeof vi.fn
>;
const useDeleteMock = useDeletePromptVersionForRepo as unknown as ReturnType<
  typeof vi.fn
>;
const useBindMock = useBindPromptVersionToAgent as unknown as ReturnType<
  typeof vi.fn
>;

interface QueryShape<T> {
  data: T;
  isLoading: boolean;
  isFetching: boolean;
  isError: boolean;
  refetch: ReturnType<typeof vi.fn>;
}

function makeQuery<T>(
  data: T,
  overrides: Partial<QueryShape<T>> = {},
): QueryShape<T> {
  return {
    data,
    isLoading: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

interface MutationStub {
  mutate: ReturnType<typeof vi.fn>;
  mutateAsync: ReturnType<typeof vi.fn>;
  isPending: boolean;
}

function makeMutation(overrides: Partial<MutationStub> = {}): MutationStub {
  return {
    mutate: vi.fn(),
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
    ...overrides,
  };
}

function makeOverviewItem(
  overrides: Partial<PromptOverviewItem> = {},
): PromptOverviewItem {
  return {
    agent_id: "11111111-1111-1111-1111-111111111111",
    agent_name: "Researcher",
    version_count: 2,
    active_version: 2,
    active_version_id: "22222222-2222-2222-2222-222222222222",
    live_system_prompt: "You are a careful research assistant.",
    latest_version_at: "2026-06-17T00:00:00Z",
    ...overrides,
  };
}

function makeVersion(overrides: Partial<PromptVersion> = {}): PromptVersion {
  return {
    id: "22222222-2222-2222-2222-222222222222",
    agent_id: "11111111-1111-1111-1111-111111111111",
    version: 1,
    content_hash: "abc",
    system_prompt: "You are a careful research assistant.",
    tools: [],
    variables: [],
    created_at: "2026-06-17T00:00:00Z",
    created_by: "dashboard",
    is_active: false,
    description: undefined,
    ...overrides,
  };
}

let bindMut: MutationStub;
let createMut: MutationStub;
let deleteMut: MutationStub;

function renderPage(): void {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <PromptsPage />
    </QueryClientProvider>,
  );
}

describe("PromptsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    bindMut = makeMutation();
    createMut = makeMutation();
    deleteMut = makeMutation();
    useBindMock.mockReturnValue(bindMut);
    useCreateMock.mockReturnValue(createMut);
    useDeleteMock.mockReturnValue(deleteMut);
    // Default: no agent selected, so the version query is never consulted —
    // but the hook is always called (rules of hooks), so give it a default.
    useVersionsMock.mockReturnValue(makeQuery<PromptVersion[]>([]));
  });

  it("renders skeleton placeholders while the overview is loading", () => {
    useOverviewMock.mockReturnValue(
      makeQuery<PromptOverviewItem[] | undefined>(undefined, {
        isLoading: true,
      }),
    );
    renderPage();
    // CardSkeleton renders role="status" placeholders, not the empty state.
    expect(screen.getAllByRole("status").length).toBeGreaterThan(0);
    expect(screen.queryByText("prompts.empty_title")).not.toBeInTheDocument();
  });

  it("shows the empty state when there are no agents", () => {
    useOverviewMock.mockReturnValue(makeQuery<PromptOverviewItem[]>([]));
    renderPage();
    expect(screen.getByText("prompts.empty_title")).toBeInTheDocument();
  });

  it("shows the error state when the overview query fails", () => {
    useOverviewMock.mockReturnValue(
      makeQuery<PromptOverviewItem[] | undefined>(undefined, {
        isError: true,
      }),
    );
    renderPage();
    expect(screen.getByText("prompts.error_title")).toBeInTheDocument();
  });

  it("renders one card per agent with active-version badge and version count", () => {
    useOverviewMock.mockReturnValue(
      makeQuery<PromptOverviewItem[]>([
        makeOverviewItem(),
        makeOverviewItem({
          agent_id: "33333333-3333-3333-3333-333333333333",
          agent_name: "Coder",
          version_count: 0,
          active_version: null,
          active_version_id: null,
          live_system_prompt: "",
        }),
      ]),
    );
    renderPage();
    expect(screen.getByText("Researcher")).toBeInTheDocument();
    expect(screen.getByText("Coder")).toBeInTheDocument();
    // Active badge interpolates the version via the count/version opt mock.
    expect(screen.getByText("prompts.active_badge:2")).toBeInTheDocument();
    // The agent with no active version shows the no-active badge.
    expect(screen.getByText("prompts.no_active_badge")).toBeInTheDocument();
  });

  it("filters agents by the search box", () => {
    useOverviewMock.mockReturnValue(
      makeQuery<PromptOverviewItem[]>([
        makeOverviewItem({ agent_name: "Researcher" }),
        makeOverviewItem({
          agent_id: "33333333-3333-3333-3333-333333333333",
          agent_name: "Coder",
        }),
      ]),
    );
    renderPage();
    fireEvent.change(
      screen.getByLabelText("prompts.search_placeholder"),
      { target: { value: "cod" } },
    );
    expect(screen.queryByText("Researcher")).not.toBeInTheDocument();
    expect(screen.getByText("Coder")).toBeInTheDocument();
  });

  it("opens the version-history modal and binds an inactive version", () => {
    useOverviewMock.mockReturnValue(
      makeQuery<PromptOverviewItem[]>([makeOverviewItem()]),
    );
    const inactive = makeVersion({
      id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
      version: 1,
      is_active: false,
    });
    const active = makeVersion({
      id: "22222222-2222-2222-2222-222222222222",
      version: 2,
      is_active: true,
    });
    useVersionsMock.mockReturnValue(
      makeQuery<PromptVersion[]>([active, inactive]),
    );
    renderPage();

    // Open the modal by clicking the agent card.
    fireEvent.click(screen.getByText("Researcher"));

    // The bind button is only rendered for the inactive version.
    const bindButton = screen.getByRole("button", { name: /prompts\.bind/ });
    fireEvent.click(bindButton);

    expect(bindMut.mutate).toHaveBeenCalledTimes(1);
    const [arg] = bindMut.mutate.mock.calls[0];
    expect(arg).toMatchObject({
      agentId: "11111111-1111-1111-1111-111111111111",
      version: { id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa" },
    });
  });

  it("creates a new version from the modal form", () => {
    useOverviewMock.mockReturnValue(
      makeQuery<PromptOverviewItem[]>([makeOverviewItem()]),
    );
    useVersionsMock.mockReturnValue(makeQuery<PromptVersion[]>([]));
    renderPage();

    fireEvent.click(screen.getByText("Researcher"));
    // Reveal the create form.
    fireEvent.click(screen.getByRole("button", { name: /prompts\.new_version/ }));

    const textarea = screen.getByPlaceholderText(
      "prompts.system_prompt_placeholder",
    );
    fireEvent.change(textarea, { target: { value: "New system prompt body" } });

    // The submit button shares the "common.create" label; scope to the form.
    const createButton = screen.getByRole("button", { name: "common.create" });
    fireEvent.click(createButton);

    expect(createMut.mutate).toHaveBeenCalledTimes(1);
    const [arg] = createMut.mutate.mock.calls[0];
    expect(arg).toMatchObject({
      agentId: "11111111-1111-1111-1111-111111111111",
      version: { system_prompt: "New system prompt body" },
    });
  });

  it("deletes an inactive version from the modal", () => {
    useOverviewMock.mockReturnValue(
      makeQuery<PromptOverviewItem[]>([makeOverviewItem()]),
    );
    const inactive = makeVersion({
      id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
      version: 1,
      is_active: false,
    });
    useVersionsMock.mockReturnValue(makeQuery<PromptVersion[]>([inactive]));
    renderPage();

    fireEvent.click(screen.getByText("Researcher"));
    const deleteButton = screen.getByRole("button", {
      name: "prompts.delete",
    });
    fireEvent.click(deleteButton);

    expect(deleteMut.mutate).toHaveBeenCalledTimes(1);
    const [arg] = deleteMut.mutate.mock.calls[0];
    expect(arg).toMatchObject({
      versionId: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
      agentId: "11111111-1111-1111-1111-111111111111",
    });
  });

  it("offers a disabled delete and no bind on the active version", () => {
    useOverviewMock.mockReturnValue(
      makeQuery<PromptOverviewItem[]>([makeOverviewItem()]),
    );
    const active = makeVersion({ version: 2, is_active: true });
    useVersionsMock.mockReturnValue(makeQuery<PromptVersion[]>([active]));
    renderPage();

    fireEvent.click(screen.getByText("Researcher"));
    // Active version shows the "bound" indicator and no bind button.
    expect(screen.getByText("prompts.bound")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /prompts\.bind/ }),
    ).not.toBeInTheDocument();
    // Delete is rendered but disabled — active version cannot be deleted until another is activated.
    expect(screen.getByRole("button", { name: "prompts.delete" })).toBeDisabled();
  });

  it("deletes an inactive version when its delete button is clicked", () => {
    useOverviewMock.mockReturnValue(
      makeQuery<PromptOverviewItem[]>([makeOverviewItem()]),
    );
    const inactive = makeVersion({ version: 1, is_active: false });
    useVersionsMock.mockReturnValue(makeQuery<PromptVersion[]>([inactive]));
    renderPage();

    fireEvent.click(screen.getByText("Researcher"));
    const del = screen.getByRole("button", { name: "prompts.delete" });
    expect(del).not.toBeDisabled();
    fireEvent.click(del);
    expect(deleteMut.mutate).toHaveBeenCalledTimes(1);
    expect(deleteMut.mutate.mock.calls[0][0]).toMatchObject({ versionId: inactive.id });
  });
});
