import { StrictMode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { useUIStore } from "./lib/store";
import {
  checkDashboardAuthMode,
  dashboardLogin,
  getStatus,
  getVersionInfo,
  getWhoami,
  verifyStoredAuth,
} from "./api";

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  // `t` and the object wrapping it must be referentially stable: `navGroups`
  // memoises on `t`, and a fresh identity each render re-runs the
  // `pruneCollapsedNavGroups` effect into an update loop.
  const translation = { t: (key: string) => key };
  return {
    ...actual,
    useTranslation: () => translation,
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

vi.mock("@tanstack/react-router", async () => {
  const React = await import("react");
  return {
    Link: ({
      children,
      to,
      ...rest
    }: { children: React.ReactNode; to?: string } & Record<string, unknown>) =>
      React.createElement("a", { href: to, ...rest }, children),
    Outlet: () => null,
    useNavigate: () => vi.fn(),
    useRouterState: () => ({ location: { pathname: "/overview" } }),
  };
});

// The sidebar/topbar chrome pulls in TanStack Query consumers that are not
// under test here; the bootstrap effect they surround is.
vi.mock("./components/NotificationCenter", () => ({
  NotificationCenter: () => null,
}));
vi.mock("./components/OfflineBanner", () => ({ OfflineBanner: () => null }));
vi.mock("./components/EveryApiPartnerLink", () => ({
  EveryApiPartnerLink: () => null,
}));
vi.mock("./components/ui/CommandPalette", () => ({
  CommandPalette: () => null,
  useCommandPalette: () => ({ isOpen: false, setIsOpen: vi.fn() }),
}));
vi.mock("./components/ui/PushDrawer", () => ({ PushDrawer: () => null }));

vi.mock("./api", () => ({
  changePassword: vi.fn(),
  checkDashboardAuthMode: vi.fn(),
  clearApiKey: vi.fn(),
  dashboardLogin: vi.fn(),
  dashboardLogout: vi.fn(),
  getDashboardUsername: vi.fn(),
  getStatus: vi.fn(),
  getVersionInfo: vi.fn(),
  getWhoami: vi.fn(),
  isPasskeySupported: vi.fn(() => false),
  loginWithPasskey: vi.fn(),
  setApiKey: vi.fn(),
  setOnUnauthorized: vi.fn(),
  verifyStoredAuth: vi.fn(),
}));

async function logIn() {
  const user = userEvent.setup();
  await user.type(
    await screen.findByPlaceholderText("auth.username_placeholder"),
    "operator",
  );
  await user.type(
    screen.getByPlaceholderText("auth.password_placeholder"),
    "password",
  );
  await user.click(screen.getByRole("button", { name: "auth.submit" }));
}

describe("DashboardApp authed bootstrap", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUIStore.setState({ terminalEnabled: null });
    window.history.pushState({}, "", "/overview");
    vi.mocked(checkDashboardAuthMode).mockResolvedValue("credentials");
    // Unauthenticated on mount, authenticated once the login succeeds.
    vi.mocked(verifyStoredAuth).mockResolvedValueOnce(false).mockResolvedValue(true);
    vi.mocked(dashboardLogin).mockResolvedValue({ ok: true });
    // `/api/version` never sends `hostname` in production (it's public and
    // deliberately omits it) — the real value comes from the authenticated
    // `/api/status`, so that's the only place this test supplies one.
    vi.mocked(getVersionInfo).mockResolvedValue({ version: "test" });
    vi.mocked(getStatus).mockResolvedValue({
      terminal_enabled: true,
      hostname: "myhost",
    } as Awaited<ReturnType<typeof getStatus>>);
    // `/api/auth/dashboard-check` never echoes the username to a caller
    // either — the daemon's identity comes from the authenticated
    // `/api/authz/whoami` instead.
    vi.mocked(getWhoami).mockResolvedValue({ name: "daemon-user", role: "admin" });
  });

  it("shows the daemon's username in the avatar after logging in", async () => {
    render(<App />);

    await logIn();

    // "DA" from the daemon's `daemon-user`, not "OP" from the form input and
    // not the "U" placeholder the empty state renders.
    await waitFor(() => expect(screen.getAllByText("DA").length).toBeGreaterThan(0));
    expect(screen.queryByText("OP")).not.toBeInTheDocument();
  });

  // The hostname is the third of the three values this fix is about, and the
  // only one nothing read back: `getStatus` was mocked with it and no test
  // opened the menu that renders it, so moving it back onto `/api/version` —
  // which never sends it — would not have failed anything.
  it("shows the daemon's hostname in the user menu after logging in", async () => {
    render(<App />);

    await logIn();

    // The sidebar row joins the caller's role and the hostname.
    // An empty hostname is dropped by the `.filter(Boolean)`, leaving the bare role.
    await waitFor(() =>
      expect(screen.getAllByText("admin · myhost").length).toBeGreaterThan(0),
    );
    expect(screen.queryByText("admin")).not.toBeInTheDocument();
  });

  // #8092: both panels used to build this line from the auth mode, so a `hybrid` deployment labelled every user `hybrid · myhost` — a fact about how the daemon accepts credentials, not about who is signed in.
  // The role is the RBAC level `/api/authz/whoami` resolved for this credential.
  it("shows the caller's role, not the auth mode, in both user panels", async () => {
    vi.mocked(checkDashboardAuthMode).mockResolvedValue("hybrid");
    render(<App />);

    await logIn();

    await waitFor(() =>
      expect(screen.getAllByText("admin · myhost")).toHaveLength(1),
    );

    // Open the topbar avatar's menu: its `UserMenuPanel` header must match the sidebar row rather than disagree with it.
    await userEvent.setup().click(screen.getByRole("button", { name: "nav.user_center" }));

    await waitFor(() =>
      expect(screen.getAllByText("admin · myhost")).toHaveLength(2),
    );
    expect(screen.queryByText(/hybrid/)).not.toBeInTheDocument();
  });

  // A rejected whoami leaves the role empty; the line must drop it rather than fall back to the auth mode.
  it("falls back to the hostname alone when whoami cannot answer", async () => {
    vi.mocked(checkDashboardAuthMode).mockResolvedValue("hybrid");
    vi.mocked(getWhoami).mockRejectedValue(new Error("401"));
    render(<App />);

    await logIn();

    await waitFor(() => expect(screen.getAllByText("myhost")).toHaveLength(1));
    await userEvent.setup().click(screen.getByRole("button", { name: "nav.user_center" }));
    await waitFor(() => expect(screen.getAllByText("myhost")).toHaveLength(2));
    expect(screen.queryByText(/hybrid/)).not.toBeInTheDocument();
  });

  it("does not re-run the auth probe after a login succeeds", async () => {
    render(<App />);

    await logIn();

    await waitFor(() => expect(screen.getAllByText("DA").length).toBeGreaterThan(0));

    // The probe ran once, on mount, to decide the login dialog needed to
    // show at all. Re-running it on every login (the bootstrap effect used
    // to depend on `authEpoch` directly) races a transient 401/500/timeout
    // from `verifyStoredAuth()` against a session that was just
    // established, and can bounce the user straight back to the dialog it
    // took real credentials to get past.
    expect(verifyStoredAuth).toHaveBeenCalledTimes(1);
    expect(
      screen.queryByPlaceholderText("auth.username_placeholder"),
    ).not.toBeInTheDocument();
  });

  it("applies the daemon's terminal policy after logging in", async () => {
    render(<App />);

    await logIn();

    await waitFor(() =>
      expect(screen.getByText("nav.console")).toBeInTheDocument(),
    );
  });

  // `main.tsx` wraps the app in `<React.StrictMode>`, which in development
  // mounts, unmounts and remounts every component once. The mounted-guard
  // ref survives that simulated remount — it is the same fiber — so a
  // cleanup that only ever sets it to `false` leaves it `false` for the rest
  // of the session, and every `fetchAuthedBootstrap` continuation returns
  // early. The avatar, the hostname and the terminal policy then stay on
  // their placeholders: exactly the symptom this PR fixes, reappearing under
  // `vite dev` while production (a single mount) looks fine.
  it("still fills the avatar under StrictMode's double mount", async () => {
    render(
      <StrictMode>
        <App />
      </StrictMode>,
    );

    await logIn();

    await waitFor(() => expect(screen.getAllByText("DA").length).toBeGreaterThan(0));
  });
});
