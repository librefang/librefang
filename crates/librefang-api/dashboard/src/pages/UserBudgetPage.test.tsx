// Tests for the per-user budget page (refs #3853 — pages/ test gap).
//
// Mocks at the queries/mutations hook layer per the dashboard data-layer rule:
// pages MUST go through `lib/queries` / `lib/mutations`, never `fetch()`. We
// therefore mock those hooks here and assert the page wires the correct
// mutation calls — matching the convention established in OverviewPage.test.tsx.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { UserBudgetPage } from "./UserBudgetPage";
import { useUserBudget } from "../lib/queries/userBudget";
import {
  useUpdateUserBudget,
  useDeleteUserBudget,
} from "../lib/mutations/userBudget";

vi.mock("../lib/queries/userBudget", () => ({
  useUserBudget: vi.fn(),
}));

vi.mock("../lib/mutations/userBudget", () => ({
  useUpdateUserBudget: vi.fn(),
  useDeleteUserBudget: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      // Echo the fallback default if provided, else the key itself.
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

const useUserBudgetMock = useUserBudget as unknown as ReturnType<typeof vi.fn>;
const useUpdateUserBudgetMock = useUpdateUserBudget as unknown as ReturnType<
  typeof vi.fn
>;
const useDeleteUserBudgetMock = useDeleteUserBudget as unknown as ReturnType<
  typeof vi.fn
>;

const HAPPY_DATA = {
  enforced: true,
  alert_breach: false,
  alert_threshold: 0.8,
  hourly: { spend: 0.1234, limit: 1.0, pct: 0.1234 },
  daily: { spend: 2.5, limit: 10.0, pct: 0.25 },
  monthly: { spend: 12.345, limit: 100.0, pct: 0.12345 },
};

function renderPage(): void {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <UserBudgetPage />
    </QueryClientProvider>,
  );
}

describe("UserBudgetPage", () => {
  let updateMutateAsync: ReturnType<typeof vi.fn>;
  let deleteMutateAsync: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    updateMutateAsync = vi.fn().mockResolvedValue(undefined);
    deleteMutateAsync = vi.fn().mockResolvedValue(undefined);
    useUpdateUserBudgetMock.mockReturnValue({
      mutateAsync: updateMutateAsync,
      isPending: false,
    });
    useDeleteUserBudgetMock.mockReturnValue({
      mutateAsync: deleteMutateAsync,
      isPending: false,
    });
  });

  it("renders loading state while the budget query is pending", () => {
    useUserBudgetMock.mockReturnValue({
      data: undefined,
      isLoading: true,
      error: null,
    });

    renderPage();

    expect(screen.getByText("Loading…")).toBeInTheDocument();
  });

  it("renders error state when the budget query fails", () => {
    useUserBudgetMock.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: new Error("network down"),
    });

    renderPage();

    expect(screen.getByText("Failed to load budget")).toBeInTheDocument();
    expect(screen.getByText(/network down/)).toBeInTheDocument();
  });

  it("renders the three spend windows and seeds the form with server limits", () => {
    useUserBudgetMock.mockReturnValue({
      data: HAPPY_DATA,
      isLoading: false,
      error: null,
    });

    renderPage();

    // Spend amounts (4-decimal-place, monospace) for each window.
    expect(screen.getByText("$0.1234")).toBeInTheDocument();
    expect(screen.getByText("$2.5000")).toBeInTheDocument();
    expect(screen.getByText("$12.3450")).toBeInTheDocument();

    // Form seeded from server (limit values stringified).
    const inputs = screen
      .getAllByRole("spinbutton")
      .map((el) => (el as HTMLInputElement).value);
    expect(inputs).toEqual(["1", "10", "100", "0.8"]);

    // Enforced badge shows when not in alert breach.
    expect(screen.getByText("enforced")).toBeInTheDocument();
  });

  it("calls useUpdateUserBudget with parsed payload when Save is clicked", async () => {
    useUserBudgetMock.mockReturnValue({
      data: HAPPY_DATA,
      isLoading: false,
      error: null,
    });

    renderPage();

    const inputs = screen.getAllByRole("spinbutton") as HTMLInputElement[];
    // Edit hourly cap to mark the form dirty so Save un-disables.
    fireEvent.change(inputs[0], { target: { value: "2.5" } });

    const saveBtn = screen.getByRole("button", { name: /Save/ });
    expect(saveBtn).not.toBeDisabled();
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(updateMutateAsync).toHaveBeenCalledTimes(1);
    });
    expect(updateMutateAsync).toHaveBeenCalledWith({
      name: "alice",
      payload: {
        max_hourly_usd: 2.5,
        max_daily_usd: 10,
        max_monthly_usd: 100,
        alert_threshold: 0.8,
      },
    });
  });

  it("rejects an alert_threshold above 1 without firing the mutation", async () => {
    useUserBudgetMock.mockReturnValue({
      data: HAPPY_DATA,
      isLoading: false,
      error: null,
    });

    renderPage();

    const inputs = screen.getAllByRole("spinbutton") as HTMLInputElement[];
    // alert_threshold input is the 4th field.
    fireEvent.change(inputs[3], { target: { value: "1.5" } });

    fireEvent.click(screen.getByRole("button", { name: /Save/ }));

    expect(
      await screen.findByText("alert_threshold must be in 0.0..=1.0"),
    ).toBeInTheDocument();
    expect(updateMutateAsync).not.toHaveBeenCalled();
  });

  it("calls useDeleteUserBudget with the user name when Clear cap is clicked", async () => {
    useUserBudgetMock.mockReturnValue({
      data: HAPPY_DATA,
      isLoading: false,
      error: null,
    });

    renderPage();

    fireEvent.click(screen.getByRole("button", { name: /Clear cap/ }));

    await waitFor(() => {
      expect(deleteMutateAsync).toHaveBeenCalledTimes(1);
    });
    expect(deleteMutateAsync).toHaveBeenCalledWith("alice");
  });

  it("shows the alert-breach badge when the server flags a breach", () => {
    useUserBudgetMock.mockReturnValue({
      data: { ...HAPPY_DATA, alert_breach: true },
      isLoading: false,
      error: null,
    });

    renderPage();

    expect(screen.getByText("alert breach")).toBeInTheDocument();
  });
});
