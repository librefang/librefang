import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SettingsPage } from "./SettingsPage";
import { useTotpStatus } from "../lib/queries/approvals";
import {
  useTotpSetup,
  useTotpConfirm,
  useTotpRevoke,
} from "../lib/mutations/approvals";

// Tests for #3853 — TOTP second-factor surface in SettingsPage. The
// TotpSection lives inside SettingsPage and was completely uncovered.
// We mock at the queries/mutations hook layer (per dashboard AGENTS.md
// rule, never inline fetch / api.* in components or tests of them).

vi.mock("../lib/queries/approvals", () => ({
  useTotpStatus: vi.fn(),
}));

vi.mock("../lib/mutations/approvals", () => ({
  useTotpSetup: vi.fn(),
  useTotpConfirm: vi.fn(),
  useTotpRevoke: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, defaultOrOpts?: unknown) =>
        typeof defaultOrOpts === "string" ? defaultOrOpts : key,
    }),
  };
});

const useTotpStatusMock = useTotpStatus as unknown as ReturnType<typeof vi.fn>;
const useTotpSetupMock = useTotpSetup as unknown as ReturnType<typeof vi.fn>;
const useTotpConfirmMock = useTotpConfirm as unknown as ReturnType<typeof vi.fn>;
const useTotpRevokeMock = useTotpRevoke as unknown as ReturnType<typeof vi.fn>;

interface MutationStub {
  mutateAsync: ReturnType<typeof vi.fn>;
  isPending: boolean;
}

function makeMutation(impl?: (...args: unknown[]) => unknown): MutationStub {
  return {
    mutateAsync: vi.fn(impl ?? (async () => undefined)),
    isPending: false,
  };
}

function renderPage(): void {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={client}>
      <SettingsPage />
    </QueryClientProvider>,
  );
}

describe("SettingsPage TOTP section (#3853)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useTotpSetupMock.mockReturnValue(makeMutation());
    useTotpConfirmMock.mockReturnValue(makeMutation());
    useTotpRevokeMock.mockReturnValue(makeMutation());
  });

  it("shows the 'Not enrolled' badge and a Set up TOTP button when not enrolled", () => {
    useTotpStatusMock.mockReturnValue({
      data: {
        enrolled: false,
        confirmed: false,
        enforced: false,
        remaining_recovery_codes: 0,
      },
      isLoading: false,
      isError: false,
    });

    renderPage();

    expect(screen.getByText("Not enrolled")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Set up TOTP" }),
    ).toBeInTheDocument();
    // No "Revoke" button when not enrolled.
    expect(
      screen.queryByRole("button", { name: "Revoke TOTP" }),
    ).not.toBeInTheDocument();
  });

  it("shows the 'Enrolled' badge plus Reset and Revoke when enrolled", () => {
    useTotpStatusMock.mockReturnValue({
      data: {
        enrolled: true,
        confirmed: true,
        enforced: true,
        remaining_recovery_codes: 8,
      },
      isLoading: false,
      isError: false,
    });

    renderPage();

    expect(screen.getByText("Enrolled")).toBeInTheDocument();
    expect(screen.getByText("Enforced")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Reset TOTP" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Revoke TOTP" }),
    ).toBeInTheDocument();
  });

  it("warns when remaining recovery codes are low", () => {
    useTotpStatusMock.mockReturnValue({
      data: {
        enrolled: true,
        confirmed: true,
        enforced: false,
        remaining_recovery_codes: 0,
      },
      isLoading: false,
      isError: false,
    });

    renderPage();

    expect(
      screen.getByText(/No recovery codes remaining/i),
    ).toBeInTheDocument();
  });

  it("renders QR code, secret and recovery codes after Set up TOTP", async () => {
    const user = userEvent.setup();
    useTotpStatusMock.mockReturnValue({
      data: {
        enrolled: false,
        confirmed: false,
        enforced: false,
        remaining_recovery_codes: 0,
      },
      isLoading: false,
      isError: false,
    });
    const setupResponse = {
      otpauth_uri: "otpauth://totp/librefang:user?secret=ABC",
      secret: "ABCDEFGHIJKLMNOP",
      qr_code: "data:image/png;base64,fake",
      recovery_codes: ["aaaa-1111", "bbbb-2222"],
      message: "ok",
    };
    const setupMutation = makeMutation(async () => setupResponse);
    useTotpSetupMock.mockReturnValue(setupMutation);

    renderPage();

    await user.click(screen.getByRole("button", { name: "Set up TOTP" }));

    // setupTotp should be invoked with no current code (fresh enrollment).
    expect(setupMutation.mutateAsync).toHaveBeenCalledTimes(1);
    expect(setupMutation.mutateAsync).toHaveBeenCalledWith(undefined);

    await waitFor(() => {
      expect(screen.getByAltText("TOTP QR Code")).toBeInTheDocument();
    });
    expect(screen.getByText("ABCDEFGHIJKLMNOP")).toBeInTheDocument();
    expect(screen.getByText("aaaa-1111")).toBeInTheDocument();
    expect(screen.getByText("bbbb-2222")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Confirm" }),
    ).toBeInTheDocument();
  });

  it("submits the 6-digit code through useTotpConfirm and shows success", async () => {
    const user = userEvent.setup();
    useTotpStatusMock.mockReturnValue({
      data: {
        enrolled: false,
        confirmed: false,
        enforced: false,
        remaining_recovery_codes: 0,
      },
      isLoading: false,
      isError: false,
    });
    useTotpSetupMock.mockReturnValue(
      makeMutation(async () => ({
        otpauth_uri: "otpauth://x",
        secret: "SECRET",
        qr_code: null,
        recovery_codes: [],
        message: "ok",
      })),
    );
    const confirmMutation = makeMutation(async () => ({ ok: true }));
    useTotpConfirmMock.mockReturnValue(confirmMutation);

    renderPage();

    await user.click(screen.getByRole("button", { name: "Set up TOTP" }));

    const codeInput = await screen.findByPlaceholderText("000000");
    // Confirm button stays disabled until 6 digits are entered.
    const confirmBtn = screen.getByRole("button", { name: "Confirm" });
    expect(confirmBtn).toBeDisabled();
    await user.type(codeInput, "123456");
    expect(confirmBtn).toBeEnabled();
    await user.click(confirmBtn);

    expect(confirmMutation.mutateAsync).toHaveBeenCalledWith("123456");
    expect(
      await screen.findByText(/TOTP confirmed/i),
    ).toBeInTheDocument();
  });

  it("revoke flow requires a code before calling useTotpRevoke", async () => {
    const user = userEvent.setup();
    useTotpStatusMock.mockReturnValue({
      data: {
        enrolled: true,
        confirmed: true,
        enforced: false,
        remaining_recovery_codes: 8,
      },
      isLoading: false,
      isError: false,
    });
    const revokeMutation = makeMutation(async () => ({ ok: true }));
    useTotpRevokeMock.mockReturnValue(revokeMutation);

    renderPage();

    await user.click(screen.getByRole("button", { name: "Revoke TOTP" }));

    const revokeInput = await screen.findByPlaceholderText(
      "TOTP or recovery code",
    );
    const confirmRevoke = screen.getByRole("button", {
      name: "Confirm Revoke",
    });
    // No code typed yet — button is disabled, mutation not called.
    expect(confirmRevoke).toBeDisabled();
    expect(revokeMutation.mutateAsync).not.toHaveBeenCalled();

    await user.type(revokeInput, "999999");
    expect(confirmRevoke).toBeEnabled();
    await user.click(confirmRevoke);

    expect(revokeMutation.mutateAsync).toHaveBeenCalledWith("999999");
    expect(await screen.findByText(/TOTP revoked/i)).toBeInTheDocument();
  });

  it("surfaces a setup failure as an inline error message", async () => {
    const user = userEvent.setup();
    useTotpStatusMock.mockReturnValue({
      data: {
        enrolled: false,
        confirmed: false,
        enforced: false,
        remaining_recovery_codes: 0,
      },
      isLoading: false,
      isError: false,
    });
    useTotpSetupMock.mockReturnValue(
      makeMutation(async () => {
        throw new Error("network down");
      }),
    );

    renderPage();

    await user.click(screen.getByRole("button", { name: "Set up TOTP" }));

    expect(await screen.findByText("network down")).toBeInTheDocument();
    // Setup payload (QR / secret) must NOT render when setup failed.
    expect(screen.queryByAltText("TOTP QR Code")).not.toBeInTheDocument();
  });

  it("opens the reset prompt (current-code input) when Reset TOTP is pressed for an enrolled user", async () => {
    const user = userEvent.setup();
    useTotpStatusMock.mockReturnValue({
      data: {
        enrolled: true,
        confirmed: true,
        enforced: false,
        remaining_recovery_codes: 8,
      },
      isLoading: false,
      isError: false,
    });
    const setupMutation = makeMutation(async () => ({
      otpauth_uri: "otpauth://x",
      secret: "NEWSECRET",
      qr_code: null,
      recovery_codes: [],
      message: "ok",
    }));
    useTotpSetupMock.mockReturnValue(setupMutation);

    renderPage();

    await user.click(screen.getByRole("button", { name: "Reset TOTP" }));

    const resetInput = await screen.findByPlaceholderText(
      "Current TOTP or recovery code",
    );
    // Must verify with current code before issuing a new secret.
    expect(setupMutation.mutateAsync).not.toHaveBeenCalled();
    await user.type(resetInput, "654321");
    await user.click(
      screen.getByRole("button", { name: "Verify & Reset" }),
    );
    expect(setupMutation.mutateAsync).toHaveBeenCalledWith("654321");
  });
});
