import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ChannelsPage } from "./ChannelsPage";
import { useDrawerStore } from "../lib/drawerStore";
import { useUIStore } from "../lib/store";
import { useChannels, useChannelQr } from "../lib/queries/channels";
import { useReloadChannels, useSaveSidecarConfig, useRemoveSidecarConfig } from "../lib/mutations/channels";
import type { ChannelItem, QrState } from "../api";

// The post-migration ChannelsPage routes every write through the
// surviving endpoints:
//   - `useChannels()`            → GET  /api/channels
//   - `useReloadChannels()`      → POST /api/channels/reload
//   - `useSaveSidecarConfig()`   → POST /api/channels/sidecar/{name}/configure
// The instance / test / configure / QR-login mutations that targeted the
// (deleted) `/api/channels/{name}/*` family are gone; this test file only
// covers what the page actually does.

vi.mock("../lib/queries/channels", () => ({
  useChannels: vi.fn(),
  useChannelQr: vi.fn(),
}));

// The `qrcode` package writes to <canvas>; jsdom's canvas is a no-op
// stub but `QRCode.toCanvas` throws if it can't find a 2d context.
// Replace with a spy so we can both prevent the throw and assert the
// dashboard called it exactly once per unique payload (render-once
// optimization in `ChannelQrSection`).
vi.mock("qrcode", () => ({
  default: { toCanvas: vi.fn(() => Promise.resolve()) },
}));

vi.mock("../lib/mutations/channels", () => ({
  useReloadChannels: vi.fn(),
  useSaveSidecarConfig: vi.fn(),
  useRemoveSidecarConfig: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) => {
        if (opts && typeof opts === "object") {
          if ("defaultValue" in opts && typeof opts.defaultValue === "string") {
            return key;
          }
          if ("count" in opts) return `${key}:${opts.count}`;
        }
        return key;
      },
    }),
  };
});

const useChannelsMock = useChannels as unknown as ReturnType<typeof vi.fn>;
const useChannelQrMock = useChannelQr as unknown as ReturnType<typeof vi.fn>;
const useReloadChannelsMock = useReloadChannels as unknown as ReturnType<
  typeof vi.fn
>;
const useSaveSidecarConfigMock = useSaveSidecarConfig as unknown as ReturnType<
  typeof vi.fn
>;
const useRemoveSidecarConfigMock = useRemoveSidecarConfig as unknown as ReturnType<
  typeof vi.fn
>;

interface QueryShape<T> {
  data: T;
  isLoading: boolean;
  isFetching: boolean;
  isError: boolean;
  refetch: ReturnType<typeof vi.fn>;
}

function makeQuery<T>(
  data: T,
  overrides: Partial<QueryShape<T>> = {},
): QueryShape<T> {
  return {
    data,
    isLoading: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

function makeChannel(overrides: Partial<ChannelItem> = {}): ChannelItem {
  return {
    name: "slack",
    display_name: "Slack",
    category: "sidecar",
    channel_type: "slack",
    configured: true,
    has_token: true,
    // Healthy default: supervised, connected, some per-instance traffic.
    supervised: true,
    connected: true,
    started_at: "2030-01-01T00:00:00Z",
    last_message_at: "2030-01-01T00:05:00Z",
    messages_received: 7,
    messages_sent: 5,
    last_error: null,
    msgs_24h_channel_type: 12,
    ...overrides,
  };
}

interface MutationStub {
  mutate: ReturnType<typeof vi.fn>;
  mutateAsync: ReturnType<typeof vi.fn>;
  isPending: boolean;
}

function makeMutation(overrides: Partial<MutationStub> = {}): MutationStub {
  return {
    mutate: vi.fn(),
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
    ...overrides,
  };
}

function setMutationDefaults(): {
  reload: MutationStub;
  save: MutationStub;
  remove: MutationStub;
} {
  const reload = makeMutation();
  const save = makeMutation();
  const remove = makeMutation();
  useReloadChannelsMock.mockReturnValue(reload);
  useSaveSidecarConfigMock.mockReturnValue(save);
  useRemoveSidecarConfigMock.mockReturnValue(remove);
  return { reload, save, remove };
}

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ChannelsPage />
      <DrawerSlot />
    </QueryClientProvider>,
  );
}

// Renders the current global drawer body once into a stable host so tests
// can query the drawer's content alongside the page. Avoids the dual mount
// that <PushDrawer /> does for desktop + mobile (which yields duplicate
// matches for every text query inside the drawer).
function DrawerSlot(): React.ReactNode {
  const content = useDrawerStore((s) => s.content);
  const isOpen = useDrawerStore((s) => s.isOpen);
  if (!isOpen || !content) return null;
  return <div data-testid="drawer-slot">{content.body}</div>;
}

describe("ChannelsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setMutationDefaults();
    useDrawerStore.setState({ isOpen: false, content: null });
    useUIStore.setState({ toasts: [] });
    // Default: no QR session. Individual tests override.
    useChannelQrMock.mockReturnValue(
      makeQuery<QrState | null>(null, { isLoading: false }),
    );
  });

  it("renders skeleton placeholders while channels query is loading", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[] | undefined>(undefined, {
        isLoading: true,
        isFetching: true,
      }),
    );
    renderPage();
    expect(screen.getByText("channels.title")).toBeInTheDocument();
    expect(screen.queryByText("Slack")).not.toBeInTheDocument();
  });

  it("renders the empty-state CTA when no channels are configured", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "discord", configured: false }),
      ]),
    );
    renderPage();
    expect(screen.getByText("channels.empty_title")).toBeInTheDocument();
    expect(screen.getByText("channels.connect_first")).toBeInTheDocument();
  });

  // The gear was gated on `category !== "sidecar"` while every channel reports
  // `category: "sidecar"`, so the test was never true and the button never
  // rendered — leaving `POST /api/channels/sidecar/{name}/configure` (#5252)
  // unreachable from the UI for an already-configured sidecar (#7892).
  // Assert on the rendered button rather than on the gate, so the same class of
  // dead condition cannot come back unnoticed.
  it("renders the configure gear for a sidecar channel", () => {
    useChannelsMock.mockReturnValue(makeQuery([makeChannel()]));
    renderPage();
    expect(screen.getByLabelText("channels.config")).toBeTruthy();
  });

  it("opens the sidecar configure drawer when the gear is clicked", async () => {
    useChannelsMock.mockReturnValue(makeQuery([makeChannel()]));
    renderPage();
    fireEvent.click(screen.getByLabelText("channels.config"));
    await waitFor(() => {
      expect(useDrawerStore.getState().isOpen).toBe(true);
    });
  });

  it("lists configured channels and hides unconfigured ones by default", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack", display_name: "Slack" }),
        makeChannel({
          name: "discord",
          display_name: "Discord",
          configured: false,
        }),
      ]),
    );
    renderPage();
    expect(screen.getByText("Slack")).toBeInTheDocument();
    // Unconfigured channels live behind the Add picker, not on the
    // page body.
    expect(screen.queryByText("Discord")).not.toBeInTheDocument();
  });

  it("filters configured channels by search query", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack", display_name: "Slack" }),
        makeChannel({ name: "telegram", display_name: "Telegram" }),
      ]),
    );
    renderPage();
    const search = screen.getByPlaceholderText("common.search");
    fireEvent.change(search, { target: { value: "tele" } });
    expect(screen.queryByText("Slack")).not.toBeInTheDocument();
    expect(screen.getByText("Telegram")).toBeInTheDocument();
  });

  it("prunes selections when a channel disappears after reload", async () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack", display_name: "Slack" }),
        makeChannel({ name: "telegram", display_name: "Telegram" }),
      ]),
    );
    const view = renderPage();
    fireEvent.click(screen.getAllByRole("button", { name: "common.select" })[0]);
    expect(screen.getByRole("button", { name: "common.deselect" })).toBeInTheDocument();

    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "telegram", display_name: "Telegram" }),
      ]),
    );
    view.rerender(
      <QueryClientProvider client={new QueryClient()}>
        <ChannelsPage />
        <DrawerSlot />
      </QueryClientProvider>,
    );

    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "common.deselect" })).not.toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "channels.select_all" }));
    expect(screen.getByRole("button", { name: "common.deselect" })).toBeInTheDocument();
  });

  it("toggles only visible selections when the channel list is filtered", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack", display_name: "Slack" }),
        makeChannel({ name: "telegram", display_name: "Telegram" }),
      ]),
    );
    renderPage();
    const slackCard = screen.getByText("Slack").closest("[role=button]");
    expect(slackCard).not.toBeNull();
    fireEvent.click(within(slackCard as HTMLElement).getByRole("button", { name: "common.select" }));
    fireEvent.change(screen.getByPlaceholderText("common.search"), {
      target: { value: "tele" },
    });
    expect(screen.queryByText("Slack")).not.toBeInTheDocument();
    expect(screen.getByText("Telegram")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "channels.select_all" }));

    fireEvent.click(screen.getByRole("button", { name: "common.clear_search" }));
    expect(screen.getAllByRole("button", { name: "common.deselect" })).toHaveLength(2);

    fireEvent.change(screen.getByPlaceholderText("common.search"), {
      target: { value: "slack" },
    });
    fireEvent.click(screen.getByRole("button", { name: "channels.select_all" }));
    fireEvent.click(screen.getByRole("button", { name: "common.clear_search" }));
    expect(screen.getAllByRole("button", { name: "common.deselect" })).toHaveLength(1);
  });

  it("opens the picker drawer with unconfigured channels", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack" }),
        makeChannel({
          name: "discord",
          display_name: "Discord",
          configured: false,
        }),
      ]),
    );
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /channels\.add/ }));
    const drawer = screen.getByTestId("drawer-slot");
    expect(within(drawer).getByText("Discord")).toBeInTheDocument();
  });

  it("opens the sidecar configure drawer when an unconfigured channel is picked", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack" }),
        makeChannel({
          name: "telegram",
          display_name: "Telegram",
          configured: false,
          fields: [
            {
              key: "TELEGRAM_BOT_TOKEN",
              label: "Bot token",
              type: "secret",
              required: true,
            },
          ],
        }),
      ]),
    );
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /channels\.add/ }));
    let drawer = screen.getByTestId("drawer-slot");
    fireEvent.click(within(drawer).getByText("Telegram"));
    // Picker → SidecarForm swap is a single React commit; the slot now
    // owns the configure body.
    drawer = screen.getByTestId("drawer-slot");
    expect(within(drawer).getByText("Telegram")).toBeInTheDocument();
    expect(within(drawer).getByText("Bot token")).toBeInTheDocument();
  });

  it("shows the SDK-missing reason and disables Save when the sidecar schema is unavailable", () => {
    const reason =
      "librefang-sdk is not installed in the Python interpreter resolved by 'python3'.";
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack" }),
        makeChannel({
          name: "wechat",
          display_name: "WeChat",
          configured: false,
          // describe failed at boot → no fields, but a reason rides along.
          fields: [],
          schema_error: reason,
        }),
      ]),
    );
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /channels\.add/ }));
    let drawer = screen.getByTestId("drawer-slot");
    fireEvent.click(within(drawer).getByText("WeChat"));
    drawer = screen.getByTestId("drawer-slot");
    // The actionable reason is surfaced verbatim instead of a blank form.
    expect(within(drawer).getByText(reason)).toBeInTheDocument();
    expect(
      within(drawer).getByText("channels.schema_unavailable_title"),
    ).toBeInTheDocument();
    // The reason-carrying copy, not the no-reason variant: the two hints differ
    // in what they tell the operator to do (fix the error below vs. edit
    // config.toml by hand), so the branch has to be pinned from both sides.
    expect(
      within(drawer).getByText("channels.schema_unavailable_hint"),
    ).toBeInTheDocument();
    expect(
      within(drawer).queryByText("channels.schema_unavailable_hint_no_reason"),
    ).not.toBeInTheDocument();
    // Save is dead — there is nothing to submit.
    expect(
      within(drawer).getByRole("button", { name: /common\.save/ }),
    ).toBeDisabled();
  });

  it("shows the SDK version the sidecar adapter reported, and nothing when it reported none", () => {
    // #7140: a Telegram sidecar ran a four-month-old librefang-sdk against a
    // current daemon and the only way to find that out was shelling into the
    // host. The configure drawer is where an operator is already looking at
    // that adapter.
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack" }),
        makeChannel({
          name: "telegram",
          display_name: "Telegram",
          configured: false,
          sdk_version: "2026.3.2201",
          fields: [
            {
              key: "TELEGRAM_BOT_TOKEN",
              label: "Bot token",
              type: "secret",
              required: true,
            },
          ],
        }),
        makeChannel({
          name: "wechat",
          display_name: "WeChat",
          configured: false,
          // An SDK too old to report a version is exactly the deployment this
          // line exists to expose, so it must read as "unknown" — an absent
          // line — rather than borrowing another adapter's number.
          fields: [
            {
              key: "WECHAT_BOT_TOKEN",
              label: "Bot token",
              type: "secret",
              required: true,
            },
          ],
        }),
      ]),
    );
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /channels\.add/ }));
    let drawer = screen.getByTestId("drawer-slot");
    fireEvent.click(within(drawer).getByText("Telegram"));
    drawer = screen.getByTestId("drawer-slot");
    expect(within(drawer).getByTestId("sidecar-sdk-version")).toHaveTextContent(
      "2026.3.2201",
    );

    fireEvent.click(screen.getByRole("button", { name: /common\.cancel/ }));
    fireEvent.click(screen.getByRole("button", { name: /channels\.add/ }));
    drawer = screen.getByTestId("drawer-slot");
    fireEvent.click(within(drawer).getByText("WeChat"));
    drawer = screen.getByTestId("drawer-slot");
    expect(
      within(drawer).queryByTestId("sidecar-sdk-version"),
    ).not.toBeInTheDocument();
  });

  it("forwards the schema-driven values to useSaveSidecarConfig on Save", () => {
    const { save } = setMutationDefaults();
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack" }),
        makeChannel({
          name: "telegram",
          display_name: "Telegram",
          configured: false,
          // Discovery rows never carry `channel_type` on the wire (only
          // configured rows do) — `makeChannel`'s default is a leftover
          // from its "slack" base, so it's overridden here to match a real
          // discovery row shape and get the right instance-name default.
          channel_type: undefined,
          fields: [
            {
              key: "TELEGRAM_BOT_TOKEN",
              label: "Bot token",
              type: "secret",
              required: true,
            },
          ],
        }),
      ]),
    );
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /channels\.add/ }));
    let drawer = screen.getByTestId("drawer-slot");
    fireEvent.click(within(drawer).getByText("Telegram"));
    drawer = screen.getByTestId("drawer-slot");
    const tokenInput = within(drawer).getByDisplayValue("");
    fireEvent.change(tokenInput, { target: { value: "abc-123" } });
    fireEvent.click(within(drawer).getByRole("button", { name: /common\.save/ }));
    expect(save.mutate).toHaveBeenCalledTimes(1);
    const [arg] = save.mutate.mock.calls[0];
    expect(arg).toMatchObject({
      name: "telegram",
      values: { TELEGRAM_BOT_TOKEN: "abc-123" },
      instanceName: "telegram",
    });
  });

  it("emits one informational toast when saved secrets are shadowed", () => {
    const { save } = setMutationDefaults();
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack" }),
        makeChannel({
          name: "telegram",
          display_name: "Telegram",
          configured: false,
          channel_type: undefined,
          fields: [{ key: "TOKEN", label: "Token", type: "secret" }],
        }),
      ]),
    );
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /channels\.add/ }));
    let drawer = screen.getByTestId("drawer-slot");
    fireEvent.click(within(drawer).getByText("Telegram"));
    drawer = screen.getByTestId("drawer-slot");
    fireEvent.change(within(drawer).getByDisplayValue(""), { target: { value: "secret" } });
    fireEvent.click(within(drawer).getByRole("button", { name: /common\.save/ }));
    const options = save.mutate.mock.calls[0][1];
    options.onSuccess({ restart_required: true, shadowed_secrets: ["TOKEN"] });

    expect(useUIStore.getState().toasts).toHaveLength(1);
    expect(useUIStore.getState().toasts[0]).toMatchObject({ type: "info" });
  });

  it("triggers useReloadChannels when the Reload header button is clicked", () => {
    const { reload } = setMutationDefaults();
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([makeChannel()]),
    );
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /channels\.reload/ }));
    expect(reload.mutate).toHaveBeenCalledTimes(1);
  });

  it("keeps removal confirmation open until the mutation succeeds", async () => {
    const { remove } = setMutationDefaults();
    let resolveRemove!: () => void;
    remove.mutateAsync.mockImplementation(() => new Promise<void>((resolve) => {
      resolveRemove = resolve;
    }));
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([makeChannel({ name: "telegram", configured: true })]),
    );
    renderPage();
    // The remove dialog only appears after the per-card Trash button is clicked.
    fireEvent.click(screen.getByRole("button", { name: "channels.remove" }));
    fireEvent.click(screen.getByRole("button", { name: "common.confirm" }));
    expect(remove.mutateAsync).toHaveBeenCalledTimes(1);
    expect(remove.mutateAsync.mock.calls[0][0]).toBe("telegram");
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    resolveRemove();

    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("pre-populates non-secret field values from the sidecar schema", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack" }),
        makeChannel({
          name: "ntfy",
          display_name: "ntfy",
          configured: false,
          fields: [
            {
              key: "NTFY_TOPIC",
              label: "Topic",
              type: "text",
              value: "alerts",
              has_value: true,
            },
          ],
        }),
      ]),
    );
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /channels\.add/ }));
    let drawer = screen.getByTestId("drawer-slot");
    fireEvent.click(within(drawer).getByText("ntfy"));
    drawer = screen.getByTestId("drawer-slot");
    expect(within(drawer).getByDisplayValue("alerts")).toBeInTheDocument();
  });

  it("uses a 'currently set' placeholder for secret fields with has_value", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack" }),
        makeChannel({
          name: "telegram",
          display_name: "Telegram",
          configured: false,
          fields: [
            {
              key: "TELEGRAM_BOT_TOKEN",
              label: "Bot token",
              type: "secret",
              required: true,
              has_value: true,
            },
          ],
        }),
      ]),
    );
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /channels\.add/ }));
    let drawer = screen.getByTestId("drawer-slot");
    fireEvent.click(within(drawer).getByText("Telegram"));
    drawer = screen.getByTestId("drawer-slot");
    // Secret field with has_value=true never echoes the value back —
    // surfaced via placeholder so the operator knows the slot is
    // filled. Empty submission preserves the stored secret.
    expect(
      within(drawer).getByPlaceholderText(/set — leave blank|channels\.secret_set_placeholder/i),
    ).toBeInTheDocument();
  });

  // #8063 fixtures: a configured Slack instance whose `[[sidecar_channels]]`
  // name (`slack-hr`) is NOT its adapter name (`slack`). Every save-path test
  // before this one drove a *discovery* row, where the two are the same string,
  // so nothing covered the pair of defects the issue reported — an edit form
  // with no fields in it, and a Save addressed by instance name.
  function slackHrFields(): ChannelItem["fields"] {
    return [
      {
        key: "SLACK_APP_TOKEN",
        label: "App Token (xapp-)",
        type: "secret",
        required: true,
        has_value: true,
      },
      {
        key: "SLACK_ALLOWED_CHANNELS",
        label: "Allowed Channel IDs",
        type: "text",
        value: "C0123",
        has_value: true,
      },
      {
        key: "SLACK_FILE_DOWNLOADS",
        label: "Forward uploaded files",
        type: "bool",
        value: "true",
        has_value: true,
      },
      {
        key: "SLACK_ACCOUNT_ID",
        label: "Account ID",
        type: "text",
        advanced: true,
      },
    ];
  }

  function slackHrChannel(overrides: Partial<ChannelItem> = {}): ChannelItem {
    return makeChannel({
      name: "slack-hr",
      display_name: "slack-hr",
      channel_type: "slack",
      configured: true,
      fields: slackHrFields(),
      ...overrides,
    });
  }

  it("renders the editable schema fields for a configured instance whose name differs from its channel type", () => {
    useChannelsMock.mockReturnValue(makeQuery<ChannelItem[]>([slackHrChannel()]));
    renderPage();
    fireEvent.click(screen.getByLabelText("channels.config"));
    const drawer = screen.getByTestId("drawer-slot");

    // The reported symptom was a drawer holding nothing but the
    // config_template toggle and Cancel/Save. Assert the non-secret fields
    // the issue names are actually rendered and carry their stored values.
    expect(within(drawer).getByText("Allowed Channel IDs")).toBeInTheDocument();
    expect(within(drawer).getByDisplayValue("C0123")).toBeInTheDocument();
    expect(within(drawer).getByText("Forward uploaded files")).toBeInTheDocument();
    expect(within(drawer).getByDisplayValue("true")).toBeInTheDocument();
    expect(within(drawer).getByText("App Token (xapp-)")).toBeInTheDocument();
    // Advanced fields stay behind the toggle, which must be present whenever
    // any exist — the issue suspected the whole form was gated this way.
    expect(within(drawer).queryByText("Account ID")).not.toBeInTheDocument();
    fireEvent.click(within(drawer).getByText("common.show_advanced"));
    expect(within(drawer).getByText("Account ID")).toBeInTheDocument();

    // Nothing inert: there is something to edit, so Save is live.
    expect(
      within(drawer).getByRole("button", { name: /common\.save/ }),
    ).toBeEnabled();
  });

  it("saves a configured instance under its adapter name, not its instance name", () => {
    const { save } = setMutationDefaults();
    useChannelsMock.mockReturnValue(makeQuery<ChannelItem[]>([slackHrChannel()]));
    renderPage();
    fireEvent.click(screen.getByLabelText("channels.config"));
    const drawer = screen.getByTestId("drawer-slot");
    fireEvent.change(within(drawer).getByDisplayValue("C0123"), {
      target: { value: "C0999" },
    });
    fireEvent.click(within(drawer).getByRole("button", { name: /common\.save/ }));

    expect(save.mutate).toHaveBeenCalledTimes(1);
    const [arg] = save.mutate.mock.calls[0];
    // `name` keys the endpoint path. Sending the instance name here is what
    // produced the issue's `404 no sidecar adapter named 'slack-hr'` — the
    // catalog only knows `slack`.
    expect(arg.name).toBe("slack");
    expect(arg.instanceName).toBe("slack-hr");
    expect(arg.values).toEqual({
      SLACK_ALLOWED_CHANNELS: "C0999",
      SLACK_FILE_DOWNLOADS: "true",
    });
    // A secret left blank stays out of the payload so the daemon keeps the
    // stored token instead of being asked to write an empty one.
    expect(arg.values).not.toHaveProperty("SLACK_APP_TOKEN");
  });

  it("disables Save for a configured instance with no schema fields even when the daemon sent no reason", () => {
    const { save } = setMutationDefaults();
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([slackHrChannel({ fields: [], schema_error: undefined })]),
    );
    renderPage();
    fireEvent.click(screen.getByLabelText("channels.config"));
    const drawer = screen.getByTestId("drawer-slot");
    const saveButton = within(drawer).getByRole("button", { name: /common\.save/ });
    // There is no cached schema behind this row, so the configure endpoint
    // could only answer 503 — the button must not pretend otherwise.
    expect(saveButton).toBeDisabled();
    fireEvent.click(saveButton);
    expect(save.mutate).not.toHaveBeenCalled();
    // A disabled button with no explanation is the same dead end the issue
    // reported, just quieter. `schema_error` is keyed per catalog adapter, so
    // a custom `[[sidecar_channels]]` type never has one — the drawer must
    // still say why it is empty and where to configure the instance instead.
    expect(
      within(drawer).getByText("channels.schema_unavailable_title"),
    ).toBeInTheDocument();
    expect(
      within(drawer).getByText("channels.schema_unavailable_hint_no_reason"),
    ).toBeInTheDocument();
  });

  it("closes the configure drawer on Cancel without saving", async () => {
    const { save } = setMutationDefaults();
    useChannelsMock.mockReturnValue(makeQuery<ChannelItem[]>([slackHrChannel()]));
    renderPage();
    fireEvent.click(screen.getByLabelText("channels.config"));
    let drawer = screen.getByTestId("drawer-slot");
    fireEvent.change(within(drawer).getByDisplayValue("C0123"), {
      target: { value: "C0999" },
    });
    drawer = screen.getByTestId("drawer-slot");
    fireEvent.click(within(drawer).getByRole("button", { name: /common\.cancel/ }));

    await waitFor(() => expect(useDrawerStore.getState().isOpen).toBe(false));
    expect(save.mutate).not.toHaveBeenCalled();
  });

  it("offers the copyable config_template snippet inside the SidecarForm drawer", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack" }),
        makeChannel({
          name: "ntfy",
          display_name: "ntfy",
          configured: false,
          config_template: '[[sidecar_channels]]\nname = "ntfy"\n',
          fields: [
            {
              key: "NTFY_TOPIC",
              label: "Topic",
              type: "text",
            },
          ],
        }),
      ]),
    );
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /channels\.add/ }));
    let drawer = screen.getByTestId("drawer-slot");
    fireEvent.click(within(drawer).getByText("ntfy"));
    drawer = screen.getByTestId("drawer-slot");
    // <details> renders the summary unconditionally; the snippet lives
    // inside the collapsed body and is still in the DOM (queryable via
    // getByText) regardless of the open/closed state.
    expect(
      within(drawer).getByText(/paste this into config\.toml|channels\.config_template_summary/i),
    ).toBeInTheDocument();
    expect(
      within(drawer).getByText(/\[\[sidecar_channels\]\]/),
    ).toBeInTheDocument();
  });

  // ── ChannelQrSection ──────────────────────────────────────────
  //
  // The section is embedded inside `DetailsModal` (read-only details
  // drawer that opens when the operator clicks a configured channel
  // card). It polls `useChannelQr` and either renders the QR canvas,
  // a success / failure card, or hides itself entirely depending on
  // the projection returned by `GET /api/channels/{name}/qr`.

  function openDetailsForWechat(qr: QrState | null, opts?: { isError?: boolean }) {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "wechat", display_name: "WeChat", configured: true }),
      ]),
    );
    useChannelQrMock.mockReturnValue(
      makeQuery<QrState | null>(qr, { isError: opts?.isError ?? false }),
    );
    renderPage();
    // Whole-card click opens DetailsModal — pick the card by its
    // unique display_name to avoid the chevron / settings buttons.
    fireEvent.click(screen.getByText("WeChat"));
  }

  it("renders the QR canvas while the lifecycle is `pending`", async () => {
    const qrcode = (await import("qrcode")).default;
    openDetailsForWechat({
      status: "pending",
      qr_code: "ilink-opaque-token",
      qr_url: "https://platform.example/login?code=ilink-opaque-token",
      message: "Scan within 5 minutes",
      updated_at: "2030-01-01T00:00:00Z",
    });
    expect(screen.getByText("channels.qr_login")).toBeInTheDocument();
    expect(screen.getByText("Scan within 5 minutes")).toBeInTheDocument();
    // Canvas is rendered with the `qr_url` (preferred over the raw
    // `qr_code`) — that's the platform-recognised deep-link form.
    await waitFor(() => {
      expect(qrcode.toCanvas).toHaveBeenCalledWith(
        expect.anything(),
        "https://platform.example/login?code=ilink-opaque-token",
        expect.objectContaining({ width: 256 }),
      );
    });
  });

  it("renders the success card on `confirmed` with the operator instruction message", () => {
    openDetailsForWechat({
      status: "confirmed",
      qr_code: "ilink-opaque-token",
      message:
        "Login successful. To skip QR on next restart, set WECHAT_BOT_TOKEN in ~/.librefang/secrets.env",
      updated_at: "2030-01-01T00:00:00Z",
    });
    expect(
      screen.getByText(/Login successful.*WECHAT_BOT_TOKEN.*secrets\.env/),
    ).toBeInTheDocument();
    // No Retry button on `confirmed` — the operator has succeeded.
    expect(screen.queryByText("common.retry")).not.toBeInTheDocument();
  });

  it("shows the Retry button on terminal `expired` state", () => {
    openDetailsForWechat({
      status: "expired",
      qr_code: "ilink-opaque-token",
      message: "QR code expired",
      updated_at: "2030-01-01T00:00:00Z",
    });
    expect(screen.getByText("QR code expired")).toBeInTheDocument();
    expect(screen.getByText("common.retry")).toBeInTheDocument();
  });

  it("re-enables QR polling when the open details modal switches channels", async () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "wechat", display_name: "WeChat" }),
        makeChannel({ name: "telegram", display_name: "Telegram" }),
      ]),
    );
    useChannelQrMock.mockImplementation((channelName: string) =>
      makeQuery<QrState>({
        status: channelName === "wechat" ? "expired" : "pending",
        qr_code: `${channelName}-qr`,
        updated_at: "2030-01-01T00:00:00Z",
      }));
    renderPage();
    fireEvent.click(screen.getByText("WeChat"));
    await waitFor(() =>
      expect(useChannelQrMock).toHaveBeenLastCalledWith(
        "wechat",
        expect.objectContaining({ refetchInterval: false }),
      ));

    fireEvent.click(screen.getByText("Telegram"));
    await waitFor(() =>
      expect(useChannelQrMock).toHaveBeenLastCalledWith(
        "telegram",
        expect.objectContaining({ enabled: true, refetchInterval: undefined }),
      ));
    expect(screen.getByLabelText("mobile_pairing.qr_aria_label")).toBeInTheDocument();
  });

  it("hides the section and disables polling when the daemon returns 204 / null", async () => {
    openDetailsForWechat(null);
    // Section heading absent → component returned null.
    expect(screen.queryByText("channels.qr_login")).not.toBeInTheDocument();
    await waitFor(() =>
      expect(useChannelQrMock).toHaveBeenLastCalledWith(
        "wechat",
        expect.objectContaining({ enabled: false }),
      ));
  });

  it("hides the section and disables polling when the QR endpoint errors", async () => {
    openDetailsForWechat(null, { isError: true });
    expect(screen.queryByText("channels.qr_login")).not.toBeInTheDocument();
    await waitFor(() =>
      expect(useChannelQrMock).toHaveBeenLastCalledWith(
        "wechat",
        expect.objectContaining({ enabled: false }),
      ));
  });

  it("does NOT expose `bot_token` in the QrState type surface", () => {
    // Type-level invariant: a future refactor must not add `bot_token`
    // back without re-reviewing the partial-save data-loss issue
    // documented in `protocol.qr_status` and `types.rs::QrState`.
    // `bot_token` was removed from `QrState` after the initial draft
    // exposed it; this test fails loudly if anyone re-adds the field.
    const sample: QrState = {
      status: "confirmed",
      qr_code: "x",
      updated_at: "2030-01-01T00:00:00Z",
    };
    // @ts-expect-error — bot_token is intentionally NOT a field.
    sample.bot_token = "leaked";
    expect(sample).toBeDefined();
  });

  // ── Status indicator (#6606) ──────────────────────────────────
  //
  // The card badge is driven by the supervisor's per-instance liveness, not
  // by traffic. These render-level cases pin the two readings that the
  // pre-fix `msgs_24h > 0 ? "running" : "idle"` rule got backwards.

  it("does NOT render a disconnected channel as healthy even with traffic recorded", () => {
    // The exact shape the issue reports: a bot that died after handling
    // messages. The old rule keyed off traffic and painted it green.
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({
          name: "tg-personal",
          display_name: "tg-personal",
          connected: false,
          started_at: "2030-01-01T00:00:00Z",
          messages_received: 34,
          messages_sent: 21,
          msgs_24h_channel_type: 35,
        }),
      ]),
    );
    renderPage();
    expect(screen.getByText("channels.liveness.stopped")).toBeInTheDocument();
    expect(screen.queryByText("channels.liveness.active")).not.toBeInTheDocument();
    expect(screen.queryByText("channels.liveness.connected")).not.toBeInTheDocument();
  });

  it("renders a healthy-but-quiet channel as connected rather than idle-grey", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({
          name: "tg-alerts",
          display_name: "tg-alerts",
          connected: true,
          messages_received: 0,
          messages_sent: 0,
          // A busy sibling of the same type inflates the per-type figure;
          // it must not influence this card's status.
          msgs_24h_channel_type: 35,
        }),
      ]),
    );
    renderPage();
    expect(screen.getByText("channels.liveness.connected")).toBeInTheDocument();
  });

  it("gives two same-type channels their own status from their own liveness", () => {
    // Six Telegram sidecars shared one `msgs_24h` value before the fix, so
    // every card turned green as soon as any one of them saw traffic.
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({
          name: "tg-live",
          display_name: "tg-live",
          channel_type: "telegram",
          connected: true,
          messages_received: 30,
          messages_sent: 25,
          msgs_24h_channel_type: 35,
        }),
        makeChannel({
          name: "tg-dead",
          display_name: "tg-dead",
          channel_type: "telegram",
          connected: false,
          started_at: "2030-01-01T00:00:00Z",
          messages_received: 5,
          messages_sent: 3,
          last_error: "sidecar exited with status 1",
          msgs_24h_channel_type: 35,
        }),
      ]),
    );
    renderPage();
    expect(screen.getByText("channels.liveness.active")).toBeInTheDocument();
    expect(screen.getByText("channels.liveness.failed")).toBeInTheDocument();
  });

  it("surfaces the sticky supervisor error in the details drawer", () => {
    const err = "Failed to spawn sidecar (last cause: No such file or directory)";
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({
          name: "tg-broken",
          display_name: "tg-broken",
          connected: true,
          last_error: err,
        }),
      ]),
    );
    renderPage();
    // Connected + error reads as degraded, never as healthy.
    expect(screen.getByText("channels.liveness.degraded")).toBeInTheDocument();
    fireEvent.click(screen.getByText("tg-broken"));
    expect(screen.getByText(err)).toBeInTheDocument();
    expect(screen.getByText("channels.last_error_sticky_hint")).toBeInTheDocument();
  });

  it("labels the 24h figure as covering every channel of the type", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({
          name: "tg-personal",
          display_name: "tg-personal",
          channel_type: "telegram",
          msgs_24h_channel_type: 35,
        }),
      ]),
    );
    renderPage();
    fireEvent.click(screen.getByText("tg-personal"));
    // The drawer row carries the scope in its own label; the card no longer
    // shows a number that could be read as this bot's traffic.
    expect(screen.getByText("channels.msgs_24h_by_type")).toBeInTheDocument();
    expect(screen.getByText("35")).toBeInTheDocument();
  });

  it("tells the operator when a configured channel has no live adapter", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({
          name: "tg-unstarted",
          display_name: "tg-unstarted",
          supervised: false,
          connected: false,
          started_at: null,
          last_message_at: null,
          messages_received: 0,
          messages_sent: 0,
        }),
      ]),
    );
    renderPage();
    expect(screen.getByText("channels.liveness.not_supervised")).toBeInTheDocument();
    fireEvent.click(screen.getByText("tg-unstarted"));
    expect(screen.getByText("channels.not_supervised_hint")).toBeInTheDocument();
  });

  it("shows an error, not the clean-install empty state, when the fetch fails", () => {
    // A dead daemon yields `data: []` plus `isError`. Falling through to the
    // "no channels yet" CTA would tell an operator their channels are gone
    // when the truth is the page could not ask.
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([], { isError: true }),
    );
    renderPage();
    expect(screen.getByText("channels.load_error")).toBeInTheDocument();
    expect(screen.queryByText("channels.empty_title")).not.toBeInTheDocument();
  });

  it("keeps rendering cached channels when a background refetch fails", () => {
    // The query polls every 30s and retains its last good `data` on failure,
    // so one transient blip must not blank a working list.
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([makeChannel({ name: "tg-ops", display_name: "tg-ops" })], {
        isError: true,
      }),
    );
    renderPage();
    expect(screen.getByText("tg-ops")).toBeInTheDocument();
    expect(screen.queryByText("channels.load_error")).not.toBeInTheDocument();
  });
});
