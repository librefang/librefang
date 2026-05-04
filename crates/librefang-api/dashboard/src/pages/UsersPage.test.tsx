import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { UsersPage } from "./UsersPage";
import { useDrawerStore } from "../lib/drawerStore";
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

// `vi.mocked(useUsers)` would preserve the TanStack Query
// `UseQueryResult<UserItem[], Error>` return type, which is a 15+ field
// union — partial mocks (data + isPending + a couple of flags) fail
// strict typecheck. Same for the `UseMutationResult` returned by each
// mutation hook. Cast to a generic vi.fn-compatible shape is the
// idiomatic escape hatch here; ChannelsPage.test.tsx uses the same
// pattern intentionally.
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
    // `UserItem` types `channel_bindings` as a required
    // `Record<string, string>`. UsersPage now defensively reads it as
    // `Object.keys(u.channel_bindings ?? {}).length` (UsersPage.tsx:307,
    // :312, :314, :1040), so an unset value no longer crashes — but the
    // fixture still supplies `{}` to satisfy the type.
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

// Renders the current global drawer body once into a stable host so tests
// can query the open drawer's content alongside the page. Mirrors the
// pattern in ChannelsPage.test.tsx — avoids the dual desktop+mobile mount
// that <PushDrawer /> does (which would yield duplicate matches for every
// query inside the drawer body).
function DrawerSlot(): React.ReactNode {
  const content = useDrawerStore(s => s.content);
  const isOpen = useDrawerStore(s => s.isOpen);
  if (!isOpen || !content) return null;
  return <div data-testid="drawer-slot">{content.body}</div>;
}

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <UsersPage />
      <DrawerSlot />
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
    // Drawer state is a global zustand store — reset between tests so a
    // drawer left open by one test doesn't bleed into the next.
    useDrawerStore.setState({ isOpen: false, content: null });
  });

  it("renders the loading skeleton while users are pending", () => {
    setUsers(undefined, { isPending: true });
    renderPage();
    // Page header is rendered alongside the skeleton — assert on the
    // page title to confirm the route mounted.
    expect(screen.getByText("Users & RBAC")).toBeInTheDocument();
    // Positive signal: UsersPage renders two <CardSkeleton/>s while
    // pending (UsersPage.tsx:240-242), each exposing role="status"
    // with aria-busy="true". EmptyState shares role="status" but
    // without aria-busy (EmptyState.tsx:12) — filter on aria-busy so
    // this assertion specifically pinpoints the loading placeholder
    // and won't false-positive on a future refactor that swaps the
    // skeleton for a static empty/error panel.
    const busy = screen
      .queryAllByRole("status")
      .filter(el => el.getAttribute("aria-busy") === "true");
    expect(busy.length).toBeGreaterThanOrEqual(2);
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

  it("falls back to the empty state when the users query errors", () => {
    // UsersPage does not render a dedicated error branch — it computes
    // `users = usersQuery.data ?? []` and falls through to the empty
    // state when data is undefined. Pinning this so a future refactor
    // that introduces a real error UI fails this test, prompting an
    // explicit error-branch assertion to be added.
    setUsers(undefined, { isError: true });
    renderPage();
    expect(screen.getByText("No users yet")).toBeInTheDocument();
    // Skeleton must NOT render — CardSkeleton sets aria-busy="true"
    // (Skeleton.tsx:15), EmptyState uses role=status without aria-busy
    // (EmptyState.tsx:12), so this filter pinpoints the loading
    // placeholder specifically.
    expect(
      screen
        .queryAllByRole("status")
        .filter(el => el.getAttribute("aria-busy") === "true").length,
    ).toBe(0);
  });

  it("opens the create wizard when the New user button is clicked", async () => {
    setUsers([]);
    renderPage();
    await userEvent.click(screen.getByRole("button", { name: "New user" }));
    // UserFormModal body renders a "Channel bindings" section header
    // (UsersPage.tsx:878) — unique to the open wizard, not present on
    // the page itself, so a positive match here proves the drawer
    // mounted via the global drawerStore.
    expect(await screen.findByText("Channel bindings")).toBeInTheDocument();
  });

  it("opens the bulk import drawer when the Bulk import button is clicked", async () => {
    setUsers([]);
    renderPage();
    await userEvent.click(
      screen.getByRole("button", { name: /bulk import/i }),
    );
    // BulkImportModal body has an "Or paste CSV" label
    // (UsersPage.tsx:1340) — unique to the open import drawer.
    expect(await screen.findByText("Or paste CSV")).toBeInTheDocument();
  });
});
