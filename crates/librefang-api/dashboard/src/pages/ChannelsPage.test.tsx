import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ChannelsPage } from "./ChannelsPage";
import { useDrawerStore } from "../lib/drawerStore";
import { useChannels } from "../lib/queries/channels";
import { useReloadChannels, useSaveSidecarConfig } from "../lib/mutations/channels";
import type { ChannelItem } from "../api";

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
}));

vi.mock("../lib/mutations/channels", () => ({
  useReloadChannels: vi.fn(),
  useSaveSidecarConfig: vi.fn(),
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
const useReloadChannelsMock = useReloadChannels as unknown as ReturnType<
  typeof vi.fn
>;
const useSaveSidecarConfigMock = useSaveSidecarConfig as unknown as ReturnType<
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
    configured: true,
    has_token: true,
    msgs_24h: 12,
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

function setMutationDefaults(): { reload: MutationStub; save: MutationStub } {
  const reload = makeMutation();
  const save = makeMutation();
  useReloadChannelsMock.mockReturnValue(reload);
  useSaveSidecarConfigMock.mockReturnValue(save);
  return { reload, save };
}

function renderPage(): void {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
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

  it("forwards the schema-driven values to useSaveSidecarConfig on Save", () => {
    const { save } = setMutationDefaults();
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
    drawer = screen.getByTestId("drawer-slot");
    const tokenInput = within(drawer).getByDisplayValue("");
    fireEvent.change(tokenInput, { target: { value: "abc-123" } });
    fireEvent.click(within(drawer).getByRole("button", { name: /common\.save/ }));
    expect(save.mutate).toHaveBeenCalledTimes(1);
    const [arg] = save.mutate.mock.calls[0];
    expect(arg).toMatchObject({
      name: "telegram",
      values: { TELEGRAM_BOT_TOKEN: "abc-123" },
    });
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
});
