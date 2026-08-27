import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuditPage } from "./AuditPage";
import { useAuditQuery } from "../lib/queries/audit";
import { PushDrawer } from "../components/ui/PushDrawer";
import { useDrawerStore } from "../lib/drawerStore";

const routerState = vi.hoisted(() => ({
  search: { user: "first" } as Record<string, unknown>,
  navigate: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useSearch: () => routerState.search,
  useNavigate: () => routerState.navigate,
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
  }),
}));

vi.mock("../lib/queries/audit", () => ({
  useAuditQuery: vi.fn(),
}));

vi.mock("../lib/queries/channels", () => ({
  useChannels: () => ({ data: [] }),
}));

vi.mock("../lib/store", () => ({
  useUIStore: (selector: (state: { addToast: ReturnType<typeof vi.fn> }) => unknown) =>
    selector({ addToast: vi.fn() }),
}));

const useAuditQueryMock = useAuditQuery as unknown as ReturnType<typeof vi.fn>;

describe("AuditPage route synchronization", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useDrawerStore.getState().close();
    routerState.search = { user: "first" };
    useAuditQueryMock.mockReturnValue({
      data: { entries: [], count: 0 },
      error: null,
      isError: false,
      isFetching: false,
      isLoading: false,
      refetch: vi.fn(),
    });
  });

  it("applies an in-place route search change without remounting", async () => {
    const view = render(<AuditPage />);
    expect(useAuditQueryMock).toHaveBeenCalledWith(
      expect.objectContaining({ user: "first", limit: 200 }),
    );

    routerState.search = { user: "second" };
    view.rerender(<AuditPage />);

    await waitFor(() =>
      expect(useAuditQueryMock).toHaveBeenLastCalledWith(
        expect.objectContaining({ user: "second", limit: 200 }),
      ),
    );
  });

  it("opens the custom-channel input when the current channel is blank", async () => {
    routerState.search = {};
    render(
      <>
        <AuditPage />
        <PushDrawer />
      </>,
    );
    const user = userEvent.setup();

    await user.click(screen.getByRole("button", { name: "audit.filters" }));
    const [channelSelect] = await screen.findAllByLabelText("audit.f_channel");
    await user.selectOptions(
      channelSelect,
      "__custom__",
    );

    expect(
      screen.getAllByPlaceholderText("audit.f_channel_placeholder").length,
    ).toBeGreaterThan(0);
  });
});
