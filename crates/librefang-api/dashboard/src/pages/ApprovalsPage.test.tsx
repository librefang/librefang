import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ApprovalsPage } from "./ApprovalsPage";
import {
  useApprovals,
  useApprovalAudit,
  useTotpStatus,
} from "../lib/queries/approvals";
import {
  useApproveApproval,
  useRejectApproval,
  useModifyAndRetryApproval,
} from "../lib/mutations/approvals";
import type { ApprovalItem } from "../api";

vi.mock("../lib/queries/approvals", () => ({
  useApprovals: vi.fn(),
  useApprovalAudit: vi.fn(),
  useTotpStatus: vi.fn(),
}));

vi.mock("../lib/mutations/approvals", () => ({
  useApproveApproval: vi.fn(),
  useRejectApproval: vi.fn(),
  useModifyAndRetryApproval: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      // Echo the key so assertions can match on i18n keys directly. For
      // interpolated strings (count, ago, action), append the relevant value
      // so we can still assert on it.
      t: (key: string, opts?: Record<string, unknown>) => {
        if (opts && typeof opts === "object") {
          if ("count" in opts) return `${key}:${opts.count}`;
          if ("ago" in opts) return `${key}:${opts.ago}`;
          if ("action" in opts) return `${key}:${opts.action}`;
        }
        return key;
      },
    }),
  };
});

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, ...rest }: { children: React.ReactNode } & Record<string, unknown>) => (
    // eslint-disable-next-line jsx-a11y/anchor-is-valid
    <a {...(rest as Record<string, unknown>)}>{children}</a>
  ),
}));

const useApprovalsMock = useApprovals as unknown as ReturnType<typeof vi.fn>;
const useApprovalAuditMock = useApprovalAudit as unknown as ReturnType<typeof vi.fn>;
const useTotpStatusMock = useTotpStatus as unknown as ReturnType<typeof vi.fn>;
const useApproveApprovalMock = useApproveApproval as unknown as ReturnType<typeof vi.fn>;
const useRejectApprovalMock = useRejectApproval as unknown as ReturnType<typeof vi.fn>;
const useModifyAndRetryApprovalMock = useModifyAndRetryApproval as unknown as ReturnType<typeof vi.fn>;

function makeApproval(overrides: Partial<ApprovalItem> = {}): ApprovalItem {
  return {
    id: "appr-1",
    agent_id: "agent-alpha",
    agent_name: "alpha",
    tool_name: "shell.exec",
    action_summary: "rm -rf /tmp/cache",
    description: "Clear the cache directory",
    risk_level: "high",
    requested_at: new Date().toISOString(),
    status: "pending",
    ...overrides,
  };
}

function setApprovalsList(items: ApprovalItem[] | undefined, opts: {
  isLoading?: boolean;
  isError?: boolean;
} = {}) {
  useApprovalsMock.mockReturnValue({
    data: items,
    isLoading: opts.isLoading ?? false,
    isError: opts.isError ?? false,
    isFetching: false,
    refetch: vi.fn(),
  });
}

function setTotpEnforced(enforced: boolean) {
  useTotpStatusMock.mockReturnValue({
    data: { enforced },
    isLoading: false,
    isError: false,
  });
}

function setMutationDefaults() {
  useApproveApprovalMock.mockReturnValue({
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
  });
  useRejectApprovalMock.mockReturnValue({
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
  });
  useModifyAndRetryApprovalMock.mockReturnValue({
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
  });
}

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <ApprovalsPage />
    </QueryClientProvider>,
  );
}

// useListNav scrolls the focused option into view; jsdom does not implement
// Element.scrollIntoView, so stub it for the duration of these tests.
beforeEach(() => {
  if (!Element.prototype.scrollIntoView) {
    Element.prototype.scrollIntoView = function () {};
  }
});

describe("ApprovalsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setMutationDefaults();
    setTotpEnforced(false);
    // Audit hook is only consumed when the History tab is active; default to
    // an empty page so any incidental render does not blow up.
    useApprovalAuditMock.mockReturnValue({
      data: { entries: [], total: 0 },
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    });
  });

  it("renders the loading skeleton while approvals are loading", () => {
    setApprovalsList(undefined, { isLoading: true });
    renderPage();
    // Pending count chip uses the count-interpolated i18n key. It still
    // renders 0 while loading — the skeleton replaces the list area.
    expect(screen.getByText("approvals.pendingCount:0")).toBeInTheDocument();
    // ListSkeleton renders status role(s); guard by absence of the empty-state
    // copy — both empty and error states would show different keys.
    expect(screen.queryByText("approvals.queue_clear")).not.toBeInTheDocument();
    expect(screen.queryByText("approvals.loadError")).not.toBeInTheDocument();
  });

  it("renders the error state with a retry handler when the list query errors", async () => {
    const refetch = vi.fn();
    useApprovalsMock.mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: true,
      isFetching: false,
      refetch,
    });
    renderPage();
    expect(screen.getByText("approvals.loadError")).toBeInTheDocument();
  });

  it("renders the empty state when there are no pending approvals", () => {
    setApprovalsList([]);
    renderPage();
    expect(screen.getByText("approvals.queue_clear")).toBeInTheDocument();
    expect(screen.getByText("approvals.queue_clear_desc")).toBeInTheDocument();
  });

  it("renders the pending list with action summary and risk badge", () => {
    setApprovalsList([
      makeApproval({ id: "a1", action_summary: "delete user", risk_level: "high" }),
      makeApproval({ id: "a2", action_summary: "list files", risk_level: "low", agent_name: "beta" }),
    ]);
    renderPage();
    expect(screen.getByText("delete user")).toBeInTheDocument();
    expect(screen.getByText("list files")).toBeInTheDocument();
    // Pending count chip reflects the size of the list.
    expect(screen.getByText("approvals.pendingCount:2")).toBeInTheDocument();
    // Two listbox options visible.
    const list = screen.getByRole("listbox", { name: "approvals.tabPending" });
    expect(within(list).getAllByRole("option")).toHaveLength(2);
  });

  it("calls approve mutation directly with no totp_code when TOTP is not enforced", async () => {
    const mutateAsync = vi.fn().mockResolvedValue(undefined);
    useApproveApprovalMock.mockReturnValue({ mutateAsync, isPending: false });
    setApprovalsList([makeApproval({ id: "a1" })]);

    renderPage();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /approvals\.approve$/ }));

    expect(mutateAsync).toHaveBeenCalledTimes(1);
    expect(mutateAsync).toHaveBeenCalledWith({ id: "a1", totpCode: undefined });
    // No TOTP modal appeared.
    expect(screen.queryByText("approvals.totp.modalTitle")).not.toBeInTheDocument();
  });

  it("opens the TOTP modal and forwards the entered code on approve when TOTP is enforced", async () => {
    setTotpEnforced(true);
    const mutateAsync = vi.fn().mockResolvedValue(undefined);
    useApproveApprovalMock.mockReturnValue({ mutateAsync, isPending: false });
    setApprovalsList([makeApproval({ id: "a1" })]);

    renderPage();
    const user = userEvent.setup();
    // The button label switches to the TOTP variant.
    await user.click(screen.getByRole("button", { name: "approvals.approveWithTotp" }));

    // Modal opens; mutation has not been called yet — TOTP code is required first.
    expect(screen.getByText("approvals.totp.modalTitle")).toBeInTheDocument();
    expect(mutateAsync).not.toHaveBeenCalled();

    const otpInput = screen.getByLabelText("approvals.totpLabel");
    await user.type(otpInput, "123456");
    await user.click(screen.getByRole("button", { name: "approvals.totp.confirm" }));

    expect(mutateAsync).toHaveBeenCalledTimes(1);
    expect(mutateAsync).toHaveBeenCalledWith({ id: "a1", totpCode: "123456" });
  });

  it("calls the reject mutation with the bare approval id (no TOTP gate)", async () => {
    setTotpEnforced(true);
    const mutateAsync = vi.fn().mockResolvedValue(undefined);
    useRejectApprovalMock.mockReturnValue({ mutateAsync, isPending: false });
    setApprovalsList([makeApproval({ id: "a1" })]);

    renderPage();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "approvals.deny" }));

    expect(mutateAsync).toHaveBeenCalledTimes(1);
    // Reject takes the id string directly — no TOTP code is ever forwarded.
    expect(mutateAsync).toHaveBeenCalledWith("a1");
    // No TOTP modal opened on reject, even when TOTP is enforced.
    expect(screen.queryByText("approvals.totp.modalTitle")).not.toBeInTheDocument();
  });

  it("filters the pending list by the search query", async () => {
    setApprovalsList([
      makeApproval({ id: "a1", agent_name: "alpha", action_summary: "delete user" }),
      makeApproval({ id: "a2", agent_name: "beta",  action_summary: "list files" }),
    ]);
    renderPage();
    const user = userEvent.setup();
    // Open the filter input.
    await user.click(screen.getByRole("button", { name: /approvals\.filter/ }));
    const input = await screen.findByPlaceholderText("approvals.filterPlaceholder");
    await user.type(input, "beta");

    expect(screen.queryByText("delete user")).not.toBeInTheDocument();
    expect(screen.getByText("list files")).toBeInTheDocument();
  });
});
