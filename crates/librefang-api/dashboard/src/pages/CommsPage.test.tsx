import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { CommsPage } from "./CommsPage";
import { useChannels, useCommsTopology, useCommsEvents } from "../lib/queries/channels";
import { useDashboardSnapshot } from "../lib/queries/overview";
import type { ChannelItem } from "../api";

// The Comms page's Channels tab renders the SAME `useChannels()` payload as
// the Channels page, and before #6606 it painted a green "Online" badge from
// `configured` alone. These tests pin the two halves of the corrected
// behaviour: a configured-but-dead sidecar must read as failed, and an
// unconfigured catalog row — which this tab lists and the Channels page does
// not — must keep reading as "Setup" rather than borrowing a liveness state.

vi.mock("../lib/queries/channels", () => ({
  useChannels: vi.fn(),
  useCommsTopology: vi.fn(),
  useCommsEvents: vi.fn(),
}));

vi.mock("../lib/queries/overview", () => ({
  useDashboardSnapshot: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) => {
        if (opts && typeof opts === "object" && "count" in opts) {
          return `${key}:${opts.count}`;
        }
        return key;
      },
    }),
  };
});

const useChannelsMock = useChannels as unknown as ReturnType<typeof vi.fn>;
const useCommsTopologyMock = useCommsTopology as unknown as ReturnType<typeof vi.fn>;
const useCommsEventsMock = useCommsEvents as unknown as ReturnType<typeof vi.fn>;
const useDashboardSnapshotMock = useDashboardSnapshot as unknown as ReturnType<
  typeof vi.fn
>;

function makeQuery<T>(data: T, overrides: Record<string, unknown> = {}) {
  return {
    data,
    isLoading: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn(),
    error: null,
    ...overrides,
  };
}

function makeChannel(overrides: Partial<ChannelItem>): ChannelItem {
  // No `display_name`: the tile falls back to `name`, so overriding `name`
  // alone is enough to tell two fixtures apart.
  return {
    name: "tg-alerts",
    category: "sidecar",
    channel_type: "telegram",
    configured: true,
    supervised: true,
    connected: true,
    started_at: "2030-01-01T00:00:00Z",
    last_message_at: null,
    messages_received: 0,
    messages_sent: 0,
    last_error: null,
    ...overrides,
  };
}

function renderPage(channels: ChannelItem[]) {
  useChannelsMock.mockReturnValue(makeQuery(channels));
  useCommsTopologyMock.mockReturnValue(makeQuery(null));
  useCommsEventsMock.mockReturnValue(makeQuery([]));
  useDashboardSnapshotMock.mockReturnValue(makeQuery(null));
  return render(<CommsPage />);
}

describe("CommsPage channel tiles", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("reports a dead sidecar as failed, not online", () => {
    // The reporter's own case: six Telegram bots, one dead for over a day,
    // with the type-level 24h aggregate still high because the siblings are
    // busy. The Comms tab used to show this row with a green ONLINE badge.
    renderPage([
      makeChannel({
        name: "tg-alerts",
        connected: false,
        last_error: "sidecar exited with status 1",
        messages_received: 34,
        messages_sent: 21,
        msgs_24h_channel_type: 812,
      }),
      makeChannel({ name: "tg-ops", messages_received: 12 }),
    ]);

    expect(screen.getByText("tg-alerts")).toBeInTheDocument();
    expect(screen.getByText("channels.liveness.failed")).toBeInTheDocument();
    expect(screen.getByText("channels.liveness.active")).toBeInTheDocument();
    expect(screen.queryByText("common.setup")).not.toBeInTheDocument();
  });

  it("keeps the setup badge on an unconfigured catalog row", () => {
    // An unconfigured row carries none of the supervisor fields, so running it
    // through `channelLiveness` would report "Not started" — broken-looking,
    // when the truth is it was never set up. This tab lists such rows.
    renderPage([
      {
        name: "discord",
        display_name: "Discord",
        category: "chat",
        configured: false,
      },
    ]);

    expect(screen.getByText("common.setup")).toBeInTheDocument();
    expect(
      screen.queryByText("channels.liveness.not_supervised"),
    ).not.toBeInTheDocument();
  });

  it("renders a configured-but-unsupervised row as not started", () => {
    renderPage([
      makeChannel({
        name: "tg-idle",
        supervised: false,
        connected: false,
        started_at: null,
      }),
    ]);

    expect(
      screen.getByText("channels.liveness.not_supervised"),
    ).toBeInTheDocument();
  });

  it("exposes the sticky error text on the badge tooltip", () => {
    renderPage([
      makeChannel({
        name: "tg-flaky",
        connected: true,
        last_error: "stdout closed",
        messages_received: 5,
      }),
    ]);

    const badge = screen.getByText("channels.liveness.degraded");
    expect(badge).toHaveAttribute("title", "stdout closed");
  });

  it("still renders the tile name and description", () => {
    const { container } = renderPage([
      makeChannel({ name: "tg-ops", description: "Out-of-process sidecar" }),
    ]);

    expect(within(container).getByText("tg-ops")).toBeInTheDocument();
    expect(
      within(container).getByText("Out-of-process sidecar"),
    ).toBeInTheDocument();
  });
});

describe("CommsPage degraded data", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useChannelsMock.mockReturnValue(makeQuery([]));
    useCommsTopologyMock.mockReturnValue(makeQuery(null));
    useCommsEventsMock.mockReturnValue(makeQuery([]));
    useDashboardSnapshotMock.mockReturnValue(makeQuery({ status: { uptime_seconds: 1 } }));
  });

  it("renders a partial snapshot without health data", () => {
    render(<CommsPage />);

    expect(screen.getAllByText("common.no_data")).not.toHaveLength(0);
  });

  it("fetches recent events once on channels and polls only on the events tab", () => {
    render(<CommsPage />);

    expect(useCommsEventsMock).toHaveBeenLastCalledWith(50, {
      enabled: true,
      refetchInterval: false,
    });

    fireEvent.click(screen.getByRole("tab", { name: "comms.events" }));
    expect(useCommsEventsMock).toHaveBeenLastCalledWith(50, {
      enabled: true,
      refetchInterval: 5_000,
    });

    fireEvent.click(screen.getByRole("tab", { name: "comms.topology" }));
    expect(useCommsEventsMock).toHaveBeenLastCalledWith(50, {
      enabled: false,
      refetchInterval: false,
    });
  });

  it("enables and refreshes only the active tab queries", () => {
    const channels = makeQuery([]);
    const snapshot = makeQuery({ status: { uptime_seconds: 1 } });
    const topology = makeQuery(null);
    const events = makeQuery([]);
    useChannelsMock.mockReturnValue(channels);
    useDashboardSnapshotMock.mockReturnValue(snapshot);
    useCommsTopologyMock.mockReturnValue(topology);
    useCommsEventsMock.mockReturnValue(events);
    render(<CommsPage />);

    expect(useChannelsMock).toHaveBeenLastCalledWith({ enabled: true });
    expect(useDashboardSnapshotMock).toHaveBeenLastCalledWith({ enabled: true });
    fireEvent.click(screen.getByRole("tab", { name: "comms.topology" }));
    expect(useChannelsMock).toHaveBeenLastCalledWith({ enabled: false });
    expect(useDashboardSnapshotMock).toHaveBeenLastCalledWith({ enabled: false });
    expect(useCommsTopologyMock).toHaveBeenLastCalledWith({ enabled: true });
    fireEvent.click(screen.getByRole("button", { name: "common.refresh" }));
    expect(topology.refetch).toHaveBeenCalledOnce();
    expect(snapshot.refetch).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("tab", { name: "comms.active_channels" }));
    fireEvent.click(screen.getByRole("button", { name: "common.refresh" }));
    expect(channels.refetch).toHaveBeenCalledOnce();
    expect(snapshot.refetch).toHaveBeenCalledOnce();
    expect(events.refetch).toHaveBeenCalledOnce();
  });

  it("counts only live configured channels as connected", () => {
    useChannelsMock.mockReturnValue(
      makeQuery([
        makeChannel({ name: "connected", connected: true }),
        makeChannel({ name: "disconnected", connected: false }),
      ]),
    );
    render(<CommsPage />);

    const label = screen.getByText("comms.connected");
    expect(
      within(label.parentElement?.parentElement as HTMLElement).getByText("1"),
    ).toBeInTheDocument();
  });

  it("distinguishes channel query failures from empty results", () => {
    useChannelsMock.mockReturnValue(
      makeQuery([], { isError: true, error: new Error("channels unavailable") }),
    );

    render(<CommsPage />);

    expect(screen.getAllByText("common.error")).toHaveLength(2);
    expect(screen.getByText("channels unavailable")).toBeInTheDocument();
    expect(screen.queryByText("common.online")).not.toBeInTheDocument();
  });

  it("does not report the events stream as online after a query failure", () => {
    useCommsEventsMock.mockReturnValue(
      makeQuery([], { isError: true, error: new Error("events unavailable") }),
    );
    render(<CommsPage />);

    fireEvent.click(screen.getByRole("tab", { name: "comms.events" }));

    expect(screen.getAllByText("common.error")).toHaveLength(3);
    expect(screen.getByText("events unavailable")).toBeInTheDocument();
    expect(screen.queryByText("common.online")).not.toBeInTheDocument();
  });
});
