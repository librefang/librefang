// Tests for the per-user permission policy page (refs #3853 — pages/ test gap).
//
// Mocks at the queries/mutations hook layer per the dashboard data-layer rule:
// pages MUST go through `lib/queries` / `lib/mutations`, never `fetch()`. We
// therefore mock those hooks here and assert the page wires the correct
// mutation calls — matching the convention established in UserBudgetPage.test.tsx.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { UserPolicyPage } from "./UserPolicyPage";
import { usePermissionPolicy } from "../lib/queries/permissionPolicy";
import { useUpdateUserPolicy } from "../lib/mutations/users";

vi.mock("../lib/queries/permissionPolicy", () => ({
  usePermissionPolicy: vi.fn(),
}));

vi.mock("../lib/mutations/users", () => ({
  useUpdateUserPolicy: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, fallbackOrOpts?: unknown, maybeOpts?: unknown) => {
        if (typeof fallbackOrOpts === "string") {
          const opts = maybeOpts as Record<string, unknown> | undefined;
          if (opts && typeof opts === "object") {
            return Object.entries(opts).reduce<string>(
              (acc, [k, v]) => acc.replace(`{{${k}}}`, String(v)),
              fallbackOrOpts,
            );
          }
          return fallbackOrOpts;
        }
        return key;
      },
    }),
  };
});

vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ name: "alice" }),
  Link: ({ children, ...rest }: { children: React.ReactNode }) => (
    <a {...rest}>{children}</a>
  ),
}));

const usePermissionPolicyMock =
  usePermissionPolicy as unknown as ReturnType<typeof vi.fn>;
const useUpdateUserPolicyMock =
  useUpdateUserPolicy as unknown as ReturnType<typeof vi.fn>;

const HAPPY_POLICY = {
  tool_policy: { allowed_tools: ["read_*"], denied_tools: ["delete_*"] },
  tool_categories: null,
  memory_access: {
    readable_namespaces: ["public", "alice"],
    writable_namespaces: ["alice"],
    pii_access: false,
    export_allowed: false,
    delete_allowed: false,
  },
  channel_tool_rules: {
    telegram: { allowed_tools: ["chat_*"], denied_tools: [] },
  },
};

function renderPage(): void {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <UserPolicyPage />
    </QueryClientProvider>,
  );
}

describe("UserPolicyPage", () => {
  let updateMutateAsync: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    updateMutateAsync = vi.fn().mockResolvedValue(undefined);
    useUpdateUserPolicyMock.mockReturnValue({
      mutateAsync: updateMutateAsync,
      isPending: false,
    });
  });

  it("renders loading skeleton while the policy query is pending", () => {
    usePermissionPolicyMock.mockReturnValue({
      data: undefined,
      isLoading: true,
      isError: false,
      error: null,
    });

    renderPage();

    // PageHeader renders the user name as subtitle even in loading state.
    expect(screen.getByText("alice")).toBeInTheDocument();
  });

  it("renders error state when the policy query fails", () => {
    usePermissionPolicyMock.mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: true,
      error: new Error("policy unavailable"),
    });

    renderPage();

    expect(screen.getByText("Failed to load policy")).toBeInTheDocument();
    expect(screen.getByText(/policy unavailable/)).toBeInTheDocument();
  });

  it("seeds the form from the loaded policy and shows configured slots", () => {
    usePermissionPolicyMock.mockReturnValue({
      data: HAPPY_POLICY,
      isLoading: false,
      isError: false,
      error: null,
    });

    renderPage();

    // Tool-policy section is enabled (has data) — its allowed/denied
    // textareas should be populated.
    const allowedTextarea = screen.getByDisplayValue("read_*");
    const deniedTextarea = screen.getByDisplayValue("delete_*");
    expect(allowedTextarea.tagName).toBe("TEXTAREA");
    expect(deniedTextarea.tagName).toBe("TEXTAREA");

    // Memory readable namespaces seeded as newline-joined string. RTL
    // collapses whitespace in display-value matches; check via DOM value.
    const textareas = Array.from(
      document.querySelectorAll<HTMLTextAreaElement>("textarea"),
    );
    expect(textareas.some(t => t.value === "public\nalice")).toBe(true);

    // Custom (non-default) channels are not in HAPPY_POLICY, so the count
    // badge equals 4 default channels (telegram already in defaults).
    expect(screen.getByText("4")).toBeInTheDocument();
  });

  it("submits the parsed payload through useUpdateUserPolicy on Save", async () => {
    usePermissionPolicyMock.mockReturnValue({
      data: HAPPY_POLICY,
      isLoading: false,
      isError: false,
      error: null,
    });

    renderPage();

    // Add a new allowed tool to mark the form dirty.
    const allowedTextarea = screen.getByDisplayValue("read_*") as HTMLTextAreaElement;
    fireEvent.change(allowedTextarea, {
      target: { value: "read_*\nlist_*" },
    });

    // Two Save buttons (header + sticky bar). Click the first.
    const saveButtons = screen.getAllByRole("button", { name: /Save/ });
    expect(saveButtons[0]).not.toBeDisabled();
    fireEvent.click(saveButtons[0]);

    await waitFor(() => {
      expect(updateMutateAsync).toHaveBeenCalledTimes(1);
    });
    const call = updateMutateAsync.mock.calls[0][0];
    expect(call.name).toBe("alice");
    expect(call.policy.tool_policy).toEqual({
      allowed_tools: ["read_*", "list_*"],
      denied_tools: ["delete_*"],
    });
    // tool_categories was disabled — payload nulls it out.
    expect(call.policy.tool_categories).toBeNull();
    // Channel rule for telegram preserved.
    expect(call.policy.channel_tool_rules.telegram).toEqual({
      allowed_tools: ["chat_*"],
      denied_tools: [],
    });
  });

  it("blocks save when writable namespace is not a subset of readable", () => {
    usePermissionPolicyMock.mockReturnValue({
      data: HAPPY_POLICY,
      isLoading: false,
      isError: false,
      error: null,
    });

    renderPage();

    // Replace writable list with a namespace not in readable.
    const writable = screen.getByDisplayValue("alice") as HTMLTextAreaElement;
    fireEvent.change(writable, { target: { value: "secret" } });

    // The validation banner appears and Save in the sticky bar disables.
    expect(
      screen.getByText(/is not in readable_namespaces/),
    ).toBeInTheDocument();
    const saveButtons = screen.getAllByRole("button", { name: /Save/ });
    // Sticky-bar Save is the last one — disabled when invalid.
    expect(saveButtons[saveButtons.length - 1]).toBeDisabled();
  });

  it("blocks save when allowed tools contains a duplicate entry", () => {
    usePermissionPolicyMock.mockReturnValue({
      data: HAPPY_POLICY,
      isLoading: false,
      isError: false,
      error: null,
    });

    renderPage();

    const allowed = screen.getByDisplayValue("read_*") as HTMLTextAreaElement;
    fireEvent.change(allowed, { target: { value: "read_*\nread_*" } });

    expect(
      screen.getByText(/contains duplicate entry 'read_\*'/),
    ).toBeInTheDocument();
  });

  it("adds a custom channel slot via the inline add-channel form", () => {
    usePermissionPolicyMock.mockReturnValue({
      data: HAPPY_POLICY,
      isLoading: false,
      isError: false,
      error: null,
    });

    renderPage();

    const input = screen.getByPlaceholderText(/wechat, matrix/);
    fireEvent.change(input, { target: { value: "matrix" } });

    const addBtn = screen.getByRole("button", { name: /^Add$/ });
    fireEvent.click(addBtn);

    // The new key surfaces uppercase in the row header.
    expect(screen.getByText("matrix")).toBeInTheDocument();
    // Badge count goes from 4 -> 5.
    expect(screen.getByText("5")).toBeInTheDocument();
  });

  it("rejects adding a duplicate channel and shows an inline error", () => {
    usePermissionPolicyMock.mockReturnValue({
      data: HAPPY_POLICY,
      isLoading: false,
      isError: false,
      error: null,
    });

    renderPage();

    const input = screen.getByPlaceholderText(/wechat, matrix/);
    // telegram is already in defaults.
    fireEvent.change(input, { target: { value: "telegram" } });
    fireEvent.click(screen.getByRole("button", { name: /^Add$/ }));

    expect(
      screen.getByText("Channel 'telegram' already has a rule slot."),
    ).toBeInTheDocument();
    expect(updateMutateAsync).not.toHaveBeenCalled();
  });

  it("disables Save in the sticky bar when the form has no changes", () => {
    usePermissionPolicyMock.mockReturnValue({
      data: HAPPY_POLICY,
      isLoading: false,
      isError: false,
      error: null,
    });

    renderPage();

    // Sticky bar shows the no-changes hint and disables Save + Discard.
    expect(screen.getByText("No unsaved changes")).toBeInTheDocument();
    const discardBtn = screen.getByRole("button", { name: /Discard changes/ });
    expect(discardBtn).toBeDisabled();
  });

  it("shows the saved confirmation card after a successful submit", async () => {
    usePermissionPolicyMock.mockReturnValue({
      data: HAPPY_POLICY,
      isLoading: false,
      isError: false,
      error: null,
    });

    renderPage();

    const allowed = screen.getByDisplayValue("read_*") as HTMLTextAreaElement;
    fireEvent.change(allowed, { target: { value: "read_*\nlist_*" } });

    fireEvent.click(screen.getAllByRole("button", { name: /Save/ })[0]);

    await waitFor(() => {
      expect(screen.getByText("Policy saved.")).toBeInTheDocument();
    });
  });

  it("surfaces a submit error when the mutation rejects", async () => {
    useUpdateUserPolicyMock.mockReturnValue({
      mutateAsync: vi.fn().mockRejectedValue(new Error("server says no")),
      isPending: false,
    });
    usePermissionPolicyMock.mockReturnValue({
      data: HAPPY_POLICY,
      isLoading: false,
      isError: false,
      error: null,
    });

    renderPage();

    const allowed = screen.getByDisplayValue("read_*") as HTMLTextAreaElement;
    fireEvent.change(allowed, { target: { value: "read_*\nlist_*" } });

    fireEvent.click(screen.getAllByRole("button", { name: /Save/ })[0]);

    await waitFor(() => {
      expect(screen.getByText(/server says no/)).toBeInTheDocument();
    });
  });
});

// Silence unused-import lint when `within` isn't used in any branch above.
void within;
