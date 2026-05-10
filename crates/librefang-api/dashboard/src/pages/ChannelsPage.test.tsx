import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ChannelsPage } from "./ChannelsPage";
import { useDrawerStore } from "../lib/drawerStore";
import { useChannels, useChannelInstances } from "../lib/queries/channels";
import {
  useConfigureChannel,
  useCreateChannelInstance,
  useDeleteChannelInstance,
  useReloadChannels,
  useTestChannel,
  useUpdateChannelInstance,
} from "../lib/mutations/channels";
import type { ChannelInstance, ChannelItem } from "../api";

vi.mock("../lib/queries/channels", () => ({
  useChannels: vi.fn(),
  useChannelInstances: vi.fn(),
}));

vi.mock("../lib/mutations/channels", () => ({
  useConfigureChannel: vi.fn(),
  useCreateChannelInstance: vi.fn(),
  useUpdateChannelInstance: vi.fn(),
  useDeleteChannelInstance: vi.fn(),
  useTestChannel: vi.fn(),
  useReloadChannels: vi.fn(),
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
            // Prefer i18n key for assertions; tests that need defaultValue can
            // match on the key itself.
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
const useChannelInstancesMock = useChannelInstances as unknown as ReturnType<
  typeof vi.fn
>;
const useConfigureChannelMock = useConfigureChannel as unknown as ReturnType<
  typeof vi.fn
>;
const useCreateChannelInstanceMock =
  useCreateChannelInstance as unknown as ReturnType<typeof vi.fn>;
const useUpdateChannelInstanceMock =
  useUpdateChannelInstance as unknown as ReturnType<typeof vi.fn>;
const useDeleteChannelInstanceMock =
  useDeleteChannelInstance as unknown as ReturnType<typeof vi.fn>;
const useTestChannelMock = useTestChannel as unknown as ReturnType<typeof vi.fn>;
const useReloadChannelsMock = useReloadChannels as unknown as ReturnType<
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
    category: "messaging",
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

function setMutationDefaults(): {
  configure: MutationStub;
  create: MutationStub;
  update: MutationStub;
  remove: MutationStub;
  test: MutationStub;
  reload: MutationStub;
} {
  const configure = makeMutation();
  const create = makeMutation();
  const update = makeMutation();
  const remove = makeMutation();
  const test = makeMutation();
  const reload = makeMutation();
  useConfigureChannelMock.mockReturnValue(configure);
  useCreateChannelInstanceMock.mockReturnValue(create);
  useUpdateChannelInstanceMock.mockReturnValue(update);
  useDeleteChannelInstanceMock.mockReturnValue(remove);
  useTestChannelMock.mockReturnValue(test);
  useReloadChannelsMock.mockReturnValue(reload);
  return { configure, create, update, remove, test, reload };
}

function setInstancesDefault(items: ChannelInstance[] = []): void {
  useChannelInstancesMock.mockReturnValue(
    makeQuery<{ channel: string; items: ChannelInstance[]; total: number }>({
      channel: "test",
      items,
      total: items.length,
    }),
  );
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
    setInstancesDefault();
    // Drawer state is a global zustand store — reset between tests so a
    // drawer left open by one test doesn't bleed into the next.
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

    // Title still mounts; configured-count chip is text "channels.configured_count:0"
    expect(screen.getByText("channels.title")).toBeInTheDocument();
    // No real channel cards mount during loading skeleton phase.
    expect(screen.queryByText("Slack")).not.toBeInTheDocument();
  });

  it("renders the empty-state CTA when no channels are configured yet", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack", configured: false }),
      ]),
    );

    renderPage();

    // Empty state shows the picker CTA and empty-state title key.
    expect(screen.getByText("channels.empty_title")).toBeInTheDocument();
    expect(screen.getByText("channels.connect_first")).toBeInTheDocument();
  });

  it("lists only configured channels in the main grid and excludes unconfigured ones", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack", display_name: "Slack", configured: true }),
        makeChannel({
          name: "discord",
          display_name: "Discord",
          configured: false,
        }),
      ]),
    );

    renderPage();

    expect(screen.getByText("Slack")).toBeInTheDocument();
    // Discord is unconfigured — must not appear in the configured grid.
    expect(screen.queryByText("Discord")).not.toBeInTheDocument();
  });

  it("filters configured channels by the search box (case-insensitive)", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack", display_name: "Slack", configured: true }),
        makeChannel({
          name: "telegram",
          display_name: "Telegram",
          configured: true,
        }),
      ]),
    );

    renderPage();

    const search = screen.getByPlaceholderText("common.search");
    fireEvent.change(search, { target: { value: "tele" } });

    expect(screen.queryByText("Slack")).not.toBeInTheDocument();
    expect(screen.getByText("Telegram")).toBeInTheDocument();
  });

  it("disables the Add button when every channel is already configured", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack", configured: true }),
        makeChannel({ name: "discord", configured: true }),
      ]),
    );

    renderPage();

    const addBtn = screen.getByText("channels.add").closest("button");
    expect(addBtn).toBeDisabled();
  });

  it("opens the picker drawer and lists only unconfigured channels when Add is clicked", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({ name: "slack", display_name: "Slack", configured: true }),
        makeChannel({
          name: "discord",
          display_name: "Discord",
          configured: false,
          category: "messaging",
        }),
        makeChannel({
          name: "email",
          display_name: "Email",
          configured: false,
          category: "mail",
        }),
      ]),
    );

    renderPage();

    fireEvent.click(screen.getByText("channels.add"));

    // The drawer renders unconfigured channels — Slack must NOT appear in
    // the picker (it's already configured).
    expect(screen.getByText("Discord")).toBeInTheDocument();
    expect(screen.getByText("Email")).toBeInTheDocument();
    // Slack now appears once on the page (the configured card) but not
    // in the picker; we assert presence count is exactly 1.
    expect(screen.getAllByText("Slack")).toHaveLength(1);
  });

  it("invokes the reload mutation when the Reload button is clicked", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([makeChannel({ configured: true })]),
    );
    const muts = setMutationDefaults();

    renderPage();

    fireEvent.click(screen.getByText("channels.reload"));
    expect(muts.reload.mutate).toHaveBeenCalledTimes(1);
  });

  it("opens the instances drawer with an Add CTA when the gear is clicked (#4837)", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({
          name: "slack",
          display_name: "Slack",
          configured: true,
          fields: [
            {
              key: "bot_token",
              label: "Bot Token",
              type: "secret",
              required: true,
            },
          ],
        }),
      ]),
    );

    renderPage();

    fireEvent.click(screen.getByLabelText("channels.config"));

    // The instance manager opens in "list" phase. With no instances seeded
    // the empty-state copy and the "Add instance" CTA must both render.
    const drawer = screen.getByTestId("drawer-slot");
    expect(within(drawer).getByText("channels.no_instances")).toBeInTheDocument();
    expect(within(drawer).getByText("channels.add_instance")).toBeInTheDocument();
    // The form fields must NOT render in the list phase — they only appear
    // after the user clicks "Add instance" / "Edit".
    expect(within(drawer).queryByText(/Bot Token/)).not.toBeInTheDocument();
  });

  it("clicking Add instance reveals the form fields", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({
          name: "slack",
          display_name: "Slack",
          configured: true,
          fields: [
            {
              key: "bot_token",
              label: "Bot Token",
              type: "secret",
              required: true,
            },
          ],
        }),
      ]),
    );

    renderPage();
    fireEvent.click(screen.getByLabelText("channels.config"));
    const drawer = screen.getByTestId("drawer-slot");
    fireEvent.click(within(drawer).getByText("channels.add_instance"));
    expect(within(drawer).getByText(/Bot Token/)).toBeInTheDocument();
  });

  it("submits createChannelInstance with only typed values when Save is clicked (#4837)", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({
          name: "slack",
          display_name: "Slack",
          configured: true,
          fields: [
            {
              key: "bot_token",
              label: "Bot Token",
              type: "secret",
              required: true,
            },
            {
              key: "workspace",
              label: "Workspace",
              type: "text",
              required: false,
            },
          ],
        }),
      ]),
    );
    const muts = setMutationDefaults();

    renderPage();
    fireEvent.click(screen.getByLabelText("channels.config"));
    const drawer = screen.getByTestId("drawer-slot");
    fireEvent.click(within(drawer).getByText("channels.add_instance"));

    const allInputs = drawer.querySelectorAll<HTMLInputElement>("input");
    expect(allInputs.length).toBeGreaterThanOrEqual(2);
    // Type a new bot token only — workspace stays blank, so payload should
    // omit it (the create handler skips empty values).
    fireEvent.change(allInputs[0], { target: { value: "xoxb-secret" } });

    fireEvent.click(within(drawer).getByText("common.create"));

    expect(muts.create.mutate).toHaveBeenCalledTimes(1);
    const [payload] = muts.create.mutate.mock.calls[0];
    expect(payload).toEqual({
      channelName: "slack",
      fields: { bot_token: "xoxb-secret" },
    });
  });

  it("disables the Create button while createChannelInstance is pending", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({
          name: "slack",
          display_name: "Slack",
          configured: true,
          fields: [{ key: "bot_token", label: "Bot Token", type: "text" }],
        }),
      ]),
    );
    setMutationDefaults();
    useCreateChannelInstanceMock.mockReturnValue(
      makeMutation({ isPending: true }),
    );

    renderPage();
    fireEvent.click(screen.getByLabelText("channels.config"));
    const drawer = screen.getByTestId("drawer-slot");
    fireEvent.click(within(drawer).getByText("channels.add_instance"));

    const save = within(drawer).getByText("common.saving").closest("button");
    expect(save).toBeDisabled();
  });

  it("lists seeded instances and routes Delete through useDeleteChannelInstance (#4837)", () => {
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([
        makeChannel({
          name: "telegram",
          display_name: "Telegram",
          configured: true,
          instance_count: 2,
          fields: [
            {
              key: "bot_token_env",
              label: "Bot Token",
              type: "secret",
              env_var: "TELEGRAM_BOT_TOKEN",
            },
          ],
        }),
      ]),
    );
    const muts = setMutationDefaults();
    useChannelInstancesMock.mockReturnValue(
      makeQuery({
        channel: "telegram",
        items: [
          {
            index: 0,
            fields: [],
            config: { bot_token_env: "TELEGRAM_BOT_TOKEN" },
            has_token: true,
          },
          {
            index: 1,
            fields: [],
            config: { bot_token_env: "TELEGRAM_BOT_TOKEN_2" },
            has_token: false,
          },
        ],
        total: 2,
      }),
    );

    renderPage();
    fireEvent.click(screen.getByLabelText("channels.config"));
    const drawer = screen.getByTestId("drawer-slot");

    // Both instances render with their pointed-at env var name as label.
    expect(within(drawer).getByText(/TELEGRAM_BOT_TOKEN(?!_)/)).toBeInTheDocument();
    expect(within(drawer).getByText(/TELEGRAM_BOT_TOKEN_2/)).toBeInTheDocument();

    // Delete the second instance — first click stages confirmation, second
    // click fires the mutation.
    const deleteButtons = within(drawer).getAllByLabelText("common.delete");
    expect(deleteButtons).toHaveLength(2);
    fireEvent.click(deleteButtons[1]);
    fireEvent.click(within(drawer).getByText("common.confirm"));

    expect(muts.remove.mutate).toHaveBeenCalledTimes(1);
    const [args] = muts.remove.mutate.mock.calls[0];
    expect(args).toEqual({ channelName: "telegram", index: 1 });
  });

  it("refetches channels when the header refresh action fires", () => {
    const refetch = vi.fn().mockResolvedValue(undefined);
    useChannelsMock.mockReturnValue(
      makeQuery<ChannelItem[]>([makeChannel({ configured: true })], { refetch }),
    );

    renderPage();

    fireEvent.click(screen.getByLabelText("common.refresh"));
    expect(refetch).toHaveBeenCalledTimes(1);
  });
});
