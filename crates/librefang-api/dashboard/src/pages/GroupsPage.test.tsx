// Tests for the Groups page list states (#7745).
//
// Mocks at the queries/mutations hook layer per the dashboard data-layer rule.
// The point of these tests is that a failed list request and a search with no hits are not rendered as "No groups yet", which invites creating a duplicate.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { GroupsPage } from "./GroupsPage";
import { useGroups } from "../lib/queries/groups";
import { useUsers } from "../lib/queries/users";

vi.mock("../lib/queries/groups", () => ({
  useGroups: vi.fn(),
}));

vi.mock("../lib/queries/users", () => ({
  useUsers: vi.fn(() => ({ data: [] })),
}));

vi.mock("../lib/mutations/groups", () => {
  const mut = () => ({ mutateAsync: vi.fn(), isPending: false });
  return {
    useAddGroupMember: vi.fn(mut),
    useCreateGroup: vi.fn(mut),
    useDeleteGroup: vi.fn(mut),
    useRemoveGroupMember: vi.fn(mut),
    useUpdateGroup: vi.fn(mut),
  };
});

vi.mock("../lib/store", () => ({
  useUIStore: (selector: (state: { addToast: () => void }) => unknown) =>
    selector({ addToast: vi.fn() }),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, fallback?: unknown) => (typeof fallback === "string" ? fallback : key),
    }),
  };
});

const useGroupsMock = useGroups as unknown as ReturnType<typeof vi.fn>;
const useUsersMock = useUsers as unknown as ReturnType<typeof vi.fn>;

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <GroupsPage />
    </QueryClientProvider>,
  );
}

describe("GroupsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUsersMock.mockReturnValue({ data: [] });
  });

  it("shows an error with retry instead of 'No groups yet' when the list request fails", () => {
    const refetch = vi.fn().mockResolvedValue(undefined);
    useGroupsMock.mockReturnValue({
      data: undefined,
      isPending: false,
      isFetching: false,
      isError: true,
      error: new Error("groups boom"),
      refetch,
    });

    renderPage();

    expect(screen.queryByText("No groups yet")).not.toBeInTheDocument();
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("groups boom");
    fireEvent.click(within(alert).getByRole("button"));
    expect(refetch).toHaveBeenCalledTimes(1);
  });

  it("says no groups match when a search filters everything out", () => {
    useGroupsMock.mockReturnValue({
      data: [],
      isPending: false,
      isFetching: false,
      isError: false,
      refetch: vi.fn(),
    });

    renderPage();
    expect(screen.getByText("No groups yet")).toBeInTheDocument();

    fireEvent.change(screen.getByPlaceholderText("Group, description, member or role…"), {
      target: { value: "oncall" },
    });

    expect(screen.queryByText("No groups yet")).not.toBeInTheDocument();
    expect(screen.getByText("No matching groups")).toBeInTheDocument();
  });
});
