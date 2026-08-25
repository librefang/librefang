import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AuthDialog } from "./App";
import {
  clearApiKey,
  dashboardLogin,
  setApiKey,
  verifyStoredAuth,
} from "./api";

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({ t: (key: string) => key }),
  };
});

vi.mock("motion/react", async () => {
  const React = await import("react");
  const MotionDiv = ({ children, ...props }: React.HTMLAttributes<HTMLDivElement>) =>
    React.createElement("div", props, children);
  return {
    AnimatePresence: ({ children }: { children: React.ReactNode }) => children,
    MotionConfig: ({ children }: { children: React.ReactNode }) => children,
    motion: { div: MotionDiv },
  };
});

vi.mock("./api", () => ({
  changePassword: vi.fn(),
  checkDashboardAuthMode: vi.fn(),
  clearApiKey: vi.fn(),
  dashboardLogin: vi.fn(),
  dashboardLogout: vi.fn(),
  getDashboardUsername: vi.fn(),
  getStatus: vi.fn(),
  getVersionInfo: vi.fn(),
  isPasskeySupported: vi.fn(() => false),
  loginWithPasskey: vi.fn(),
  setApiKey: vi.fn(),
  setOnUnauthorized: vi.fn(),
  verifyStoredAuth: vi.fn(),
}));

describe("AuthDialog", () => {
  beforeEach(() => {
    vi.mocked(clearApiKey).mockReset();
    vi.mocked(dashboardLogin).mockReset();
    vi.mocked(setApiKey).mockReset();
    vi.mocked(verifyStoredAuth).mockReset();
  });

  it("clears a submitted key when verification returns false", async () => {
    const user = userEvent.setup();
    const onAuthenticated = vi.fn();
    vi.mocked(verifyStoredAuth).mockResolvedValue(false);
    render(<AuthDialog mode="api_key" onAuthenticated={onAuthenticated} />);

    const submit = screen.getByRole("button", { name: "auth.submit" });
    expect(submit).toBeDisabled();
    await user.type(screen.getByPlaceholderText("auth.placeholder"), "secret");
    expect(submit).toBeEnabled();
    await user.click(submit);

    await waitFor(() => expect(clearApiKey).toHaveBeenCalledOnce());
    expect(setApiKey).toHaveBeenCalledWith("secret");
    expect(screen.getByText("auth.invalid_api_key")).toBeInTheDocument();
    expect(onAuthenticated).not.toHaveBeenCalled();
  });

  it("catches unexpected API-key verification failures", async () => {
    const user = userEvent.setup();
    vi.mocked(verifyStoredAuth).mockRejectedValue(new Error("offline"));
    render(<AuthDialog mode="api_key" onAuthenticated={() => {}} />);

    await user.type(screen.getByPlaceholderText("auth.placeholder"), "secret");
    await user.click(screen.getByRole("button", { name: "auth.submit" }));

    await waitFor(() =>
      expect(screen.getByText("auth.invalid")).toBeInTheDocument(),
    );
    expect(clearApiKey).toHaveBeenCalledOnce();
  });

  it("surfaces unavailable key storage without an unhandled rejection", async () => {
    const user = userEvent.setup();
    vi.mocked(setApiKey).mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    render(<AuthDialog mode="api_key" onAuthenticated={() => {}} />);

    await user.type(screen.getByPlaceholderText("auth.placeholder"), "secret");
    await user.click(screen.getByRole("button", { name: "auth.submit" }));

    await waitFor(() =>
      expect(screen.getByText("auth.invalid")).toBeInTheDocument(),
    );
    expect(clearApiKey).toHaveBeenCalledOnce();
    expect(verifyStoredAuth).not.toHaveBeenCalled();
  });

  it("catches unexpected credential-login failures", async () => {
    const user = userEvent.setup();
    const onAuthenticated = vi.fn();
    vi.mocked(dashboardLogin).mockRejectedValue(new Error("offline"));
    render(<AuthDialog mode="credentials" onAuthenticated={onAuthenticated} />);

    const submit = screen.getByRole("button", { name: "auth.submit" });
    expect(submit).toBeDisabled();
    await user.type(
      screen.getByPlaceholderText("auth.username_placeholder"),
      "operator",
    );
    expect(submit).toBeDisabled();
    await user.type(
      screen.getByPlaceholderText("auth.password_placeholder"),
      "password",
    );
    expect(submit).toBeEnabled();
    await user.click(submit);

    await waitFor(() =>
      expect(screen.getByText("auth.invalid")).toBeInTheDocument(),
    );
    expect(onAuthenticated).not.toHaveBeenCalled();
  });

  it("requires a complete TOTP code before verification", async () => {
    const user = userEvent.setup();
    vi.mocked(dashboardLogin).mockResolvedValue({
      ok: false,
      requires_totp: true,
    });
    render(<AuthDialog mode="credentials" onAuthenticated={() => {}} />);

    await user.type(
      screen.getByPlaceholderText("auth.username_placeholder"),
      "operator",
    );
    await user.type(
      screen.getByPlaceholderText("auth.password_placeholder"),
      "password",
    );
    await user.click(screen.getByRole("button", { name: "auth.submit" }));

    const totpSubmit = await screen.findByRole("button", {
      name: "auth.verify_totp",
    });
    expect(totpSubmit).toBeDisabled();
    await user.type(screen.getByPlaceholderText("000000"), "123456");
    expect(totpSubmit).toBeEnabled();
  });
});
