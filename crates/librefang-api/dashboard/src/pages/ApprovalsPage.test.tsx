import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  ApprovalsPage,
  isValidTotpOrRecovery,
  maxHistoryOffset,
  sanitizeTotpOrRecovery,
} from "./ApprovalsPage";
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
import { useFullConfig } from "../lib/queries/config";
import type { ApprovalAuditEntry, ApprovalItem } from "../api";

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

vi.mock("../lib/queries/config", () => ({
  useFullConfig: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      // Echo the key so assertions can match on i18n keys directly. For
      // interpolated strings (count, ago, action, label, value), append the
      // relevant value so we can still assert on it.
      t: (key: string, opts?: Record<string, unknown>) => {
        if (opts && typeof opts === "object") {
          if ("count" in opts) return `${key}:${opts.count}`;
          if ("ago" in opts) return `${key}:${opts.ago}`;
          if ("action" in opts) return `${key}:${opts.action}`;
          if ("label" in opts) return `${key}:${opts.label}`;
          if ("value" in opts) return `${key}:${opts.value}`;
        }
        return key;
      },
    }),
  };
});

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, ...rest }: { children: React.ReactNode } & Record<string, unknown>) => (
    <a {...(rest as Record<string, unknown>)}>{children}</a>
  ),
}));

const useApprovalsMock = useApprovals as unknown as ReturnType<typeof vi.fn>;
const useApprovalAuditMock = useApprovalAudit as unknown as ReturnType<typeof vi.fn>;
const useTotpStatusMock = useTotpStatus as unknown as ReturnType<typeof vi.fn>;
const useApproveApprovalMock = useApproveApproval as unknown as ReturnType<typeof vi.fn>;
const useRejectApprovalMock = useRejectApproval as unknown as ReturnType<typeof vi.fn>;
const useModifyAndRetryApprovalMock = useModifyAndRetryApproval as unknown as ReturnType<typeof vi.fn>;
const useFullConfigMock = useFullConfig as unknown as ReturnType<typeof vi.fn>;

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

function makeAuditEntry(overrides: Partial<ApprovalAuditEntry> = {}): ApprovalAuditEntry {
  return {
    id: "audit-1",
    request_id: "req-1",
    agent_id: "agent-alpha",
    tool_name: "shell.exec",
    description: "Clear the cache directory",
    action_summary: "rm -rf /tmp/cache",
    risk_level: "high",
    decision: "approved",
    decided_by: "admin",
    decided_at: "2026-07-28T10:00:00Z",
    requested_at: "2026-07-28T09:59:00Z",
    second_factor_used: false,
    ...overrides,
  };
}

function setAudit(entries: ApprovalAuditEntry[]) {
  useApprovalAuditMock.mockReturnValue({
    data: { items: entries, total: entries.length },
    isLoading: false,
    isError: false,
    refetch: vi.fn(),
  });
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

function setFullConfig(
  config: Record<string, unknown> | undefined,
  opts: { isLoading?: boolean; isError?: boolean } = {},
) {
  useFullConfigMock.mockReturnValue({
    data: config,
    isLoading: opts.isLoading ?? false,
    isError: opts.isError ?? false,
    refetch: vi.fn(),
  });
}

function setTotpEnforced(enforced: boolean) {
  useTotpStatusMock.mockReturnValue({
    data: { enforced },
    isSuccess: true,
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

describe("ApprovalsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setMutationDefaults();
    setTotpEnforced(false);
    // The trusted-senders card reads the full config on the Pending tab; default to an empty approval section so unrelated cases render it as the "nobody bypasses the gate" state.
    setFullConfig({ approval: { trusted_senders: [] } });
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

  it("fails closed while the TOTP status is unavailable", async () => {
    useTotpStatusMock.mockReturnValue({
      data: undefined,
      isSuccess: false,
      isLoading: false,
      isError: true,
    });
    const mutateAsync = vi.fn().mockResolvedValue(undefined);
    useApproveApprovalMock.mockReturnValue({ mutateAsync, isPending: false });
    setApprovalsList([makeApproval({ id: "a1" })]);

    renderPage();
    await userEvent
      .setup()
      .click(screen.getByRole("button", { name: "approvals.approveWithTotp" }));

    expect(screen.getByText("approvals.totp.modalTitle")).toBeInTheDocument();
    expect(mutateAsync).not.toHaveBeenCalled();
  });

  it("normalizes TOTP and recovery-code input to accepted shapes", () => {
    expect(sanitizeTotpOrRecovery("12a34--56-78")).toBe("1234-5678");
    expect(sanitizeTotpOrRecovery("------")).toBe("");
    expect(sanitizeTotpOrRecovery("123-456-")).toBe("123456");
    expect(isValidTotpOrRecovery("123456")).toBe(true);
    expect(isValidTotpOrRecovery("1234-5678")).toBe(true);
  });

  it("masks the complete recovery-code shape", async () => {
    setTotpEnforced(true);
    setApprovalsList([makeApproval({ id: "a1" })]);
    const user = userEvent.setup();

    renderPage();
    await user.click(
      screen.getByRole("button", { name: "approvals.approveWithTotp" }),
    );
    await user.type(screen.getByLabelText("approvals.totpLabel"), "1234-5678");

    expect(screen.getByText("•••••••••")).toBeInTheDocument();
    expect(screen.queryByText("••••-••••")).not.toBeInTheDocument();
  });

  it("locks every decision control while modify-and-retry is pending", async () => {
    const mutateAsync = vi.fn(() => new Promise<void>(() => {}));
    useModifyAndRetryApprovalMock.mockReturnValue({ mutateAsync, isPending: false });
    setApprovalsList([makeApproval({ id: "a1" })]);
    const user = userEvent.setup();

    renderPage();
    await user.click(screen.getByRole("button", { name: "approvals.editApprove" }));
    await user.type(
      screen.getByPlaceholderText("approvals.modifyPlaceholder"),
      "Use a safer command",
    );
    await user.click(
      screen.getByRole("button", { name: "approvals.editApproveSubmit" }),
    );

    expect(screen.getByRole("button", { name: /approvals\.approve$/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: "approvals.deny" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "approvals.editApproveSubmit" }),
    ).toBeDisabled();
  });

  it("clamps history offsets to the last populated page", () => {
    expect(maxHistoryOffset(0)).toBe(0);
    expect(maxHistoryOffset(50)).toBe(0);
    expect(maxHistoryOffset(51)).toBe(50);
    expect(maxHistoryOffset(149)).toBe(100);
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

  /* ---------------------------------------------------------------- */
  /*  History decision labels (#6607)                                 */
  /* ---------------------------------------------------------------- */

  describe("history decision labels", () => {
    const EDITED = "approvals.history.decisions.edited";

    async function openHistory() {
      const user = userEvent.setup();
      await user.click(screen.getByRole("tab", { name: "approvals.tabHistory" }));
    }

    // Every value the daemon is known to write to `approval_audit.decision`,
    // paired with the label it must render. `pending` and `timed_out` are the
    // regression from #6607: before the fix every non-approve/non-reject value
    // fell through to the "Edited" branch, so a request nobody answered was
    // presented as a completed operator edit.
    const cases: Array<[string, string]> = [
      ["approved", "approvals.history.decisions.approved"],
      ["approve", "approvals.history.decisions.approved"],
      ["denied", "approvals.history.decisions.denied"],
      ["rejected", "approvals.history.decisions.denied"],
      ["reject", "approvals.history.decisions.denied"],
      ["modify_and_retry", EDITED],
      ["timed_out", "approvals.history.decisions.timedOut"],
      ["pending", "approvals.history.decisions.pending"],
      ["skipped", "approvals.history.decisions.skipped"],
    ];

    it.each(cases)(
      "labels a %s audit entry with %s",
      async (decision, expectedKey) => {
        setApprovalsList([]);
        setAudit([makeAuditEntry({ decision })]);
        renderPage();
        await openHistory();

        expect(await screen.findByText(expectedKey)).toBeInTheDocument();
        if (expectedKey !== EDITED) {
          expect(screen.queryByText(EDITED)).not.toBeInTheDocument();
        }
      },
    );

    it("distinguishes timed_out from a real operator edit in one table", async () => {
      setApprovalsList([]);
      setAudit([
        makeAuditEntry({ id: "h1", request_id: "r1", decision: "timed_out" }),
        makeAuditEntry({ id: "h2", request_id: "r2", decision: "modify_and_retry" }),
      ]);
      renderPage();
      await openHistory();

      expect(
        await screen.findByText("approvals.history.decisions.timedOut"),
      ).toBeInTheDocument();
      expect(screen.getByText(EDITED)).toBeInTheDocument();
      // The two rows must not collapse onto the same label.
      expect(screen.getAllByText(EDITED)).toHaveLength(1);
    });

    it("labels a still-pending audit row as pending, not as an edit", async () => {
      setApprovalsList([]);
      setAudit([makeAuditEntry({ decision: "pending", decided_by: undefined })]);
      renderPage();
      await openHistory();

      expect(
        await screen.findByText("approvals.history.decisions.pending"),
      ).toBeInTheDocument();
      expect(screen.queryByText(EDITED)).not.toBeInTheDocument();
    });

    it("degrades an unrecognised decision to its raw value, not to Edited", async () => {
      setApprovalsList([]);
      setAudit([makeAuditEntry({ decision: "escalated_to_oncall" })]);
      renderPage();
      await openHistory();

      // Raw server value is shown verbatim so a new backend variant is visible
      // instead of masquerading as a completed edit.
      expect(await screen.findByText("escalated_to_oncall")).toBeInTheDocument();
      expect(screen.queryByText(EDITED)).not.toBeInTheDocument();
    });

    it("falls back to the explicit unknown label when the decision is empty", async () => {
      setApprovalsList([]);
      setAudit([makeAuditEntry({ decision: "" })]);
      renderPage();
      await openHistory();

      expect(
        await screen.findByText("approvals.history.decisions.unknown"),
      ).toBeInTheDocument();
      expect(screen.queryByText(EDITED)).not.toBeInTheDocument();
    });

    it("does not resolve a prototype key to a known decision", async () => {
      setApprovalsList([]);
      setAudit([makeAuditEntry({ decision: "constructor" })]);
      renderPage();
      await openHistory();

      expect(await screen.findByText("constructor")).toBeInTheDocument();
      expect(screen.queryByText(EDITED)).not.toBeInTheDocument();
    });

    it("pairs each decision with a text equivalent so colour is not the only cue", async () => {
      setApprovalsList([]);
      setAudit([makeAuditEntry({ decision: "timed_out" })]);
      renderPage();
      await openHistory();

      // aria-label carries the column context plus the decision label.
      expect(
        await screen.findByLabelText(
          "approvals.history.decisions.aria:approvals.history.decisions.timedOut",
        ),
      ).toBeInTheDocument();
    });

    it("names the raw value in the aria-label of an unrecognised decision", async () => {
      setApprovalsList([]);
      setAudit([makeAuditEntry({ decision: "escalated_to_oncall" })]);
      renderPage();
      await openHistory();

      expect(
        await screen.findByLabelText(
          "approvals.history.decisions.unknownAria:escalated_to_oncall",
        ),
      ).toBeInTheDocument();
    });
  });
  // -------------------------------------------------------------------
  // Trusted senders — approval-bypass audit surface (#6611)
  // -------------------------------------------------------------------

  describe("trusted senders", () => {
    it("lists every configured sender on the pending tab", () => {
      setApprovalsList([]);
      setFullConfig({
        approval: { trusted_senders: ["operator-1", "ops-oncall"] },
      });
      renderPage();

      const list = screen.getByRole("list", {
        name: "approvals.trustedSendersTitle",
      });
      expect(within(list).getByText("operator-1")).toBeInTheDocument();
      expect(within(list).getByText("ops-oncall")).toBeInTheDocument();
    });

    it("reports an empty list as every sender going through the gate", () => {
      setApprovalsList([]);
      setFullConfig({ approval: { trusted_senders: [] } });
      renderPage();

      expect(screen.getByText("approvals.trustedSendersEmpty")).toBeInTheDocument();
      expect(
        screen.queryByRole("list", { name: "approvals.trustedSendersTitle" }),
      ).not.toBeInTheDocument();
    });

    it("shows an error state when the config query fails", () => {
      setApprovalsList([]);
      setFullConfig(undefined, { isError: true });
      renderPage();

      expect(
        screen.getByText("approvals.trustedSendersLoadError"),
      ).toBeInTheDocument();
    });

    it("renders a busy placeholder while the config is loading", () => {
      setApprovalsList([]);
      setFullConfig(undefined, { isLoading: true });
      renderPage();

      const busy = screen
        .getAllByRole("status")
        .filter((el) => el.getAttribute("aria-busy") === "true");
      expect(busy.length).toBeGreaterThan(0);
      expect(
        screen.queryByText("approvals.trustedSendersEmpty"),
      ).not.toBeInTheDocument();
    });

    it("ignores a malformed approval section instead of crashing the page", () => {
      // `GET /api/config` is untyped on the wire, so a section of the wrong shape must degrade to "nothing configured" rather than throw and take the whole approvals queue down with it.
      setApprovalsList([]);
      setFullConfig({ approval: { trusted_senders: "operator-1" } });
      renderPage();

      expect(screen.getByText("approvals.trustedSendersEmpty")).toBeInTheDocument();
    });
  });
});
