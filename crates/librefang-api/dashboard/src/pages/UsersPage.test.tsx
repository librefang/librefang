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
      // Echo the inline English default when present so assertions can
      // match on the literal copy. When the second arg is an interpolation
      // options object (no inline default), fall back to the i18n key
      // suffixed with the first interpolation value (count / ago /
      // action / message / n) — same shape as ApprovalsPage.test.tsx so
      // future branches that exercise rotate-result, wizard, or import
      // count don't silently match the raw `{{count}}` placeholder.
      t: (
        key: string,
        fallbackOrOpts?: string | Record<string, unknown>,
        opts?: Record<string, unknown>,
      ) => {
        if (typeof fallbackOrOpts === "string") {
          if (opts && typeof opts === "object") {
            for (const k of ["count", "ago", "action", "message", "n"]) {
              if (k in opts) return `${fallbackOrOpts}:${String(opts[k])}`;
            }
          }
          return fallbackOrOpts;
        }
        if (fallbackOrOpts && typeof fallbackOrOpts === "object") {
          for (const k of ["count", "ago", "action", "message", "n"]) {
            if (k in fallbackOrOpts) return `${key}:${String(fallbackOrOpts[k])}`;
          }
        }
        return key;
      },
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
  opts: {
    isPending?: boolean;
    isLoading?: boolean;
    isError?: boolean;
    isFetching?: boolean;
  } = {},
) {
  // UsersPage gates on `isPending` (UsersPage.tsx:238). `isLoading` is kept
  // independently settable so a future branch that differentiates pending
  // vs background-refetch can drive each knob without surprising the
  // existing tests. Defaults to mirroring `isPending` for the common case.
  const isPending = opts.isPending ?? false;
  useUsersMock.mockReturnValue({
    data: items,
    isPending,
    isLoading: opts.isLoading ?? isPending,
    isError: opts.isError ?? false,
    isFetching: opts.isFetching ?? false,
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
    // Positive signal: UsersPage renders two <CardSkeleton/>s while
    // pending (UsersPage.tsx:240-242), each exposing role="status".
    // Asserting on this catches a future refactor that drops the
    // skeleton entirely or replaces it with a non-status placeholder —
    // the absence-only checks below would silently pass in that case.
    expect(screen.getAllByRole("status").length).toBeGreaterThanOrEqual(2);
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
        channel_bindings: { discord: "bob#1234" },
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
