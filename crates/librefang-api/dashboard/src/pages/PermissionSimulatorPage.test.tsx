import React from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  PermissionSimulatorPage,
  normalizeRole,
} from "./PermissionSimulatorPage";
import { useUsers } from "../lib/queries/users";
import { useEffectivePermissions } from "../lib/queries/authz";
import { ApiError } from "../lib/http/client";

vi.mock("../lib/queries/users", () => ({
  useUsers: vi.fn(),
}));

vi.mock("../lib/queries/authz", () => ({
  useEffectivePermissions: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, fallback?: string) => fallback ?? key,
    }),
  };
});

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: new Proxy(
    {},
    {
      get: (_target, prop: string) =>
        ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
          React.createElement(prop, rest, children),
    },
  ),
}));

const mock = <T,>(fn: T) => fn as unknown as ReturnType<typeof vi.fn>;
const useUsersMock = mock(useUsers);
const useEffectivePermissionsMock = mock(useEffectivePermissions);

const alice = { name: "alice", role: "admin" };
const bob = { name: "bob", role: "viewer" };

function setUsers(users: Array<{ name: string; role: string }>) {
  useUsersMock.mockReturnValue({ data: users });
}

function setEffective(overrides: Record<string, unknown> = {}) {
  useEffectivePermissionsMock.mockReturnValue({
    data: undefined,
    error: null,
    isError: false,
    isLoading: true,
    ...overrides,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  setUsers([alice, bob]);
  setEffective();
});

describe("PermissionSimulatorPage", () => {
  it("recognizes missing users through the structured 404 status", () => {
    setEffective({
      error: new ApiError(404, "user_missing", "localized response"),
      isError: true,
      isLoading: false,
    });
    render(<PermissionSimulatorPage />);

    expect(screen.getByText("User not found")).toBeInTheDocument();
    expect(
      screen.queryByText("Could not load effective permissions"),
    ).not.toBeInTheDocument();
  });

  it("does not classify free-text 404 messages as structured not-found errors", () => {
    setEffective({
      error: new Error("404 user not found"),
      isError: true,
      isLoading: false,
    });
    render(<PermissionSimulatorPage />);

    expect(screen.getByText("Could not load effective permissions")).toBeInTheDocument();
    expect(screen.queryByText("User not found")).not.toBeInTheDocument();
  });

  it("falls back to the user role for unknown API role values", () => {
    expect(normalizeRole("viewer")).toBe("viewer");
    expect(normalizeRole("owner")).toBe("owner");
    expect(normalizeRole("future-role")).toBe("user");
    expect(normalizeRole(undefined)).toBe("user");
  });

  it("synchronizes selection when the chosen user disappears", async () => {
    const view = render(<PermissionSimulatorPage />);
    const select = screen.getByLabelText("User") as HTMLSelectElement;
    fireEvent.change(select, { target: { value: "bob" } });
    await waitFor(() => expect(select.value).toBe("bob"));

    setUsers([alice]);
    view.rerender(<PermissionSimulatorPage />);
    await waitFor(() => expect(select.value).toBe("alice"));

    setUsers([alice, bob]);
    view.rerender(<PermissionSimulatorPage />);
    expect(select.value).toBe("alice");
    const lastCall = useEffectivePermissionsMock.mock.calls[
      useEffectivePermissionsMock.mock.calls.length - 1
    ];
    expect(lastCall?.[0]).toBe("alice");
  });
});
