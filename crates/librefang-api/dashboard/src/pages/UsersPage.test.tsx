import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { UsersPage } from "./UsersPage";
import { useUsers } from "../lib/queries/users";
import {
  useCreateUser,
  useUpdateUser,
  useDeleteUser,
  useImportUsers,
  useRotateUserKey,
} from "../lib/mutations/users";
import type { UserItem } from "../lib/http/client";

// ---------------------------------------------------------------------------
// Mocks (#3853 — UsersPage RBAC management page).
// ---------------------------------------------------------------------------

vi.mock("../lib/queries/users", () => ({
  useUsers: vi.fn(),
}));

vi.mock("../lib/mutations/users", () => ({
  useCreateUser: vi.fn(),
  useUpdateUser: vi.fn(),
  useDeleteUser: vi.fn(),
  useImportUsers: vi.fn(),
  useRotateUserKey: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      // Echo the i18n key so assertions can grep on it directly. Fall back
      // to the second positional argument (the inline default) so we can
      // also assert on the literal English copy.
      t: (key: string, fallback?: string) =>
        typeof fallback === "string" ? fallback : key,
    }),
  };
});

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    ...rest
  }: {
    children: React.ReactNode;
  } & Record<string, unknown>) => (
    // eslint-disable-next-line jsx-a11y/anchor-is-valid
    <a {...(rest as Record<string, unknown>)}>{children}</a>
  ),
}));

const useUsersMock = useUsers as unknown as ReturnType<typeof vi.fn>;
const useCreateUserMock = useCreateUser as unknown as ReturnType<typeof vi.fn>;
const useUpdateUserMock = useUpdateUser as unknown as ReturnType<typeof vi.fn>;
const useDeleteUserMock = useDeleteUser as unknown as ReturnType<typeof vi.fn>;
const useImportUsersMock = useImportUsers as unknown as ReturnType<typeof vi.fn>;
const useRotateUserKeyMock = useRotateUserKey as unknown as ReturnType<typeof vi.fn>;

function makeUser(overrides: Partial<UserItem> = {}): UserItem {
  return {
    name: "alice",
    role: "Operator",
    platform: "telegram",
    platform_id: "@alice",
    created_at: new Date().toISOString(),
    // UserItem requires `channel_bindings: Record<string, string>` —
    // UsersPage renders `Object.keys(u.channel_bindings).length`, so an
    // unset value would crash with "Cannot convert undefined or null to
    // object" before any assertion runs.
    channel_bindings: {},
    ...overrides,
  } as UserItem;
}

function setUsers(
  items: UserItem[] | undefined,
  opts: { isPending?: boolean; isError?: boolean } = {},
) {
  useUsersMock.mockReturnValue({
    data: items,
    isPending: opts.isPending ?? false,
    isLoading: opts.isPending ?? false,
    isError: opts.isError ?? false,
    isFetching: false,
    refetch: vi.fn(),
  });
}

function setMutationDefaults() {
  const idleMut = {
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
  };
  useCreateUserMock.mockReturnValue(idleMut);
  useUpdateUserMock.mockReturnValue(idleMut);
  useDeleteUserMock.mockReturnValue(idleMut);
  useImportUsersMock.mockReturnValue(idleMut);
  useRotateUserKeyMock.mockReturnValue({
    ...idleMut,
    mutateAsync: vi.fn().mockResolvedValue({ plaintext: "rot-key-xyz" }),
  });
}

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <UsersPage />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  if (!Element.prototype.scrollIntoView) {
    Element.prototype.scrollIntoView = function () {};
  }
});

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("UsersPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setMutationDefaults();
  });

  it("renders the loading skeleton while users are pending", () => {
    setUsers(undefined, { isPending: true });
    renderPage();
    // Page header is rendered alongside the skeleton — assert on the
    // page title to confirm the route mounted.
    expect(screen.getByText("Users & RBAC")).toBeInTheDocument();
    // While isPending, neither the empty-state title nor a real user row
    // should be present — a CardSkeleton replaces the list area.
    expect(screen.queryByText("No users yet")).not.toBeInTheDocument();
  });

  it("renders the empty state when no users are configured", () => {
    setUsers([]);
    renderPage();
    expect(screen.getByText("No users yet")).toBeInTheDocument();
  });

  it("renders configured users with name and role", () => {
    setUsers([
      makeUser({ name: "alice", role: "Admin" }),
      makeUser({
        name: "bob",
        role: "Viewer",
        platform: "discord",
        platform_id: "bob#1234",
      }),
    ]);
    renderPage();
    expect(screen.getByText("alice")).toBeInTheDocument();
    expect(screen.getByText("bob")).toBeInTheDocument();
    // Empty state must not render when the list is non-empty.
    expect(screen.queryByText("No users yet")).not.toBeInTheDocument();
  });

  it("exposes the New user and Bulk import (CSV) action buttons", () => {
    setUsers([]);
    renderPage();
    expect(
      screen.getByRole("button", { name: "New user" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /bulk import/i }),
    ).toBeInTheDocument();
  });
});
