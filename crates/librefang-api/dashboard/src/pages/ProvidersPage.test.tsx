// Tests for the LLM providers page (refs #3853 — pages/ test gap).
//
// Mocks at the queries/mutations hook layer per the dashboard data-layer rule
// (see crates/librefang-api/dashboard/AGENTS.md): pages route through
// `lib/queries` / `lib/mutations`, never raw `fetch()`. Render-side concerns
// (motion, modals, drawer, store toasts) are stubbed so we can exercise the
// core list/filter/tab/search wiring without dragging in animation timers.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ProviderItem } from "../api";
import { ProvidersPage } from "./ProvidersPage";
import { useDrawerStore } from "../lib/drawerStore";
import { useProviders, useProviderStatus } from "../lib/queries/providers";
import { useModels } from "../lib/queries/models";
import {
  useTestProvider,
  useSetProviderKey,
  useDeleteProviderKey,
  useEnableProvider,
  useSetProviderUrl,
  useSetDefaultProvider,
  useCreateRegistryContent,
  useConnectEveryApi,
} from "../lib/mutations/providers";

vi.mock("../lib/queries/providers", () => ({
  useProviders: vi.fn(),
  useProviderStatus: vi.fn(),
  // CredentialPoolsSection (#5459-era addition) calls this; default to the
  // empty/hidden state so the existing provider-list tests don't have to
  // care about the niche credential-pools feature. Tests that exercise it
  // can override via the exported mock.
  useCredentialPools: vi.fn(() => ({
    data: undefined,
    isLoading: false,
    error: null,
  })),
}));

vi.mock("../lib/queries/models", () => ({
  useModels: vi.fn(),
  // ProviderMaxTokensSection (#6209) calls this; default to no override so the
  // existing tests don't have to care about the max-tokens section.
  useModelOverrides: vi.fn(() => ({ data: undefined, isLoading: false })),
}));

vi.mock("../lib/mutations/providers", () => ({
  useTestProvider: vi.fn(),
  useSetProviderKey: vi.fn(),
  useDeleteProviderKey: vi.fn(),
  useEnableProvider: vi.fn(),
  useSetProviderUrl: vi.fn(),
  useSetDefaultProvider: vi.fn(),
  useCreateRegistryContent: vi.fn(),
  useConnectEveryApi: vi.fn(),
  // A value export rather than a hook, and the page reads it for the relay-key and gateway placeholders.
  // The literal has to match `EVERYAPI_PROVIDER` in the real module, which in turn mirrors the Rust-side constants — a mock drifting from it would make these tests assert placeholders the shipped UI never renders.
  EVERYAPI_PROVIDER: {
    id: "everyapi",
    displayName: "EveryAPI",
    apiKeyEnv: "EVERYAPI_API_KEY",
    defaultBaseUrl: "https://api.everyapi.ai/v1",
  },
}));

// Toast store — only `addToast` is consumed by ProvidersPage.
const addToastMock = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: { addToast: typeof addToastMock }) => unknown) =>
    selector({ addToast: addToastMock }),
}));

// Keyboard shortcut hook is fire-and-forget here.
vi.mock("../lib/useCreateShortcut", () => ({
  useCreateShortcut: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallbackOrOpts?: unknown) =>
      typeof fallbackOrOpts === "string" ? fallbackOrOpts : key,
  }),
}));

const useProvidersMock = useProviders as unknown as ReturnType<typeof vi.fn>;
const useProviderStatusMock = useProviderStatus as unknown as ReturnType<
  typeof vi.fn
>;
const useModelsMock = useModels as unknown as ReturnType<typeof vi.fn>;
const useTestProviderMock = useTestProvider as unknown as ReturnType<
  typeof vi.fn
>;
const useSetProviderKeyMock = useSetProviderKey as unknown as ReturnType<
  typeof vi.fn
>;
const useEnableProviderMock = useEnableProvider as unknown as ReturnType<
  typeof vi.fn
>;
const useDeleteProviderKeyMock = useDeleteProviderKey as unknown as ReturnType<
  typeof vi.fn
>;
const useSetProviderUrlMock = useSetProviderUrl as unknown as ReturnType<
  typeof vi.fn
>;
const useSetDefaultProviderMock = useSetDefaultProvider as unknown as ReturnType<
  typeof vi.fn
>;
const useCreateRegistryContentMock =
  useCreateRegistryContent as unknown as ReturnType<typeof vi.fn>;
const useConnectEveryApiMock =
  useConnectEveryApi as unknown as ReturnType<typeof vi.fn>;

const PROVIDERS: ProviderItem[] = [
  {
    id: "openai",
    display_name: "OpenAI",
    auth_status: "validated_key",
    reachable: true,
    model_count: 12,
    latency_ms: 120,
    key_required: true,
    base_url: "https://api.openai.com/v1",
  },
  {
    id: "anthropic",
    display_name: "Anthropic",
    auth_status: "configured",
    reachable: false,
    model_count: 5,
    latency_ms: 700,
    key_required: true,
    base_url: "https://api.anthropic.com",
  },
  {
    id: "groq",
    display_name: "Groq",
    auth_status: "missing",
    reachable: false,
    model_count: 0,
    key_required: true,
  },
];

function renderPage(): void {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <ProvidersPage />
      <DrawerSlot />
    </QueryClientProvider>,
  );
}

// Renders the current global drawer body once into a stable host so tests
// can query the drawer's content alongside the page. Mirrors the helper in
// ChannelsPage.test.tsx; <PushDrawer /> mounts twice (desktop + mobile) and
// breaks unique text queries.
function DrawerSlot(): React.ReactNode {
  const content = useDrawerStore((s) => s.content);
  const isOpen = useDrawerStore((s) => s.isOpen);
  if (!isOpen || !content) return null;
  return <div data-testid="drawer-slot">{content.body}</div>;
}

describe("ProvidersPage", () => {
  let testMutateAsync: ReturnType<typeof vi.fn>;
  let connectEveryApiMutateAsync: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    // Drawer state is a global zustand store — reset between tests so a
    // drawer left open by one test doesn't bleed into the next.
    useDrawerStore.setState({ isOpen: false, content: null });
    testMutateAsync = vi.fn().mockResolvedValue({ status: "ok" });
    connectEveryApiMutateAsync = vi.fn().mockResolvedValue(undefined);

    useProviderStatusMock.mockReturnValue({
      data: { default_provider: "openai" },
      isFetching: false,
    });
    useModelsMock.mockReturnValue({ data: { models: [] }, isLoading: false });

    const stubMutation = (mutateAsync: ReturnType<typeof vi.fn>) => ({
      mutateAsync,
      isPending: false,
    });

    useTestProviderMock.mockReturnValue(stubMutation(testMutateAsync));
    useSetProviderKeyMock.mockReturnValue(
      stubMutation(vi.fn().mockResolvedValue(undefined)),
    );
    useDeleteProviderKeyMock.mockReturnValue(
      stubMutation(vi.fn().mockResolvedValue(undefined)),
    );
    useEnableProviderMock.mockReturnValue(
      stubMutation(vi.fn().mockResolvedValue(undefined)),
    );
    useSetProviderUrlMock.mockReturnValue(
      stubMutation(vi.fn().mockResolvedValue(undefined)),
    );
    useSetDefaultProviderMock.mockReturnValue(
      stubMutation(vi.fn().mockResolvedValue(undefined)),
    );
    useCreateRegistryContentMock.mockReturnValue(
      stubMutation(vi.fn().mockResolvedValue(undefined)),
    );
    // The EveryAPI drawer reads more of the mutation surface than `stubMutation` provides: it calls `reset()` when the drawer closes and renders an inline alert off `isError` / `error`.
    useConnectEveryApiMock.mockReturnValue({
      mutateAsync: connectEveryApiMutateAsync,
      isPending: false,
      isError: false,
      error: null,
      reset: vi.fn(),
    });
  });

  it("shows skeleton placeholders while providers load", () => {
    useProvidersMock.mockReturnValue({
      data: undefined,
      isLoading: true,
      isFetching: true,
      refetch: vi.fn(),
    });

    renderPage();

    // CardSkeleton uses role="status" aria-busy="true" — six are emitted
    // while the providers query is pending.
    expect(screen.getAllByRole("status").length).toBeGreaterThanOrEqual(6);
  });

  it("renders empty state when the providers list is empty", () => {
    useProvidersMock.mockReturnValue({
      data: [],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    expect(screen.getByText("common.no_data")).toBeInTheDocument();
  });

  it("shows the configured/total count badge in the header", () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    // 2 of 3 providers in PROVIDERS are configured (openai, anthropic).
    expect(screen.getByText(/2 \/ 3/)).toBeInTheDocument();
  });

  it("renders configured providers by default and hides unconfigured ones", () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    expect(screen.getByText("OpenAI")).toBeInTheDocument();
    expect(screen.getByText("Anthropic")).toBeInTheDocument();
    // groq is `missing` → unconfigured tab only.
    expect(screen.queryByText("Groq")).not.toBeInTheDocument();
  });

  it("opens the Add picker drawer and lists only unconfigured providers", async () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    // Configured providers are visible on the page; unconfigured ones live
    // behind the picker (post-tab-removal: ProvidersPage now mirrors
    // ChannelsPage's add-via-picker pattern).
    expect(screen.queryByText("Groq")).not.toBeInTheDocument();

    // Header has the Add button — click it to open the picker drawer.
    fireEvent.click(screen.getByRole("button", { name: /providers\.add/ }));

    // Drawer renders the unconfigured catalog. Groq (auth_status: missing)
    // shows up; OpenAI/Anthropic don't, since they're already configured.
    const drawer = await screen.findByTestId("drawer-slot");
    expect(within(drawer).getByText("Groq")).toBeInTheDocument();
    expect(within(drawer).queryByText("OpenAI")).not.toBeInTheDocument();
    expect(within(drawer).queryByText("Anthropic")).not.toBeInTheDocument();
  });

  // ── EveryAPI connect action ───────────────────────────────────────────
  //
  // EveryAPI is not a built-in provider: until a registry entry exists it is absent from `GET /api/providers` altogether, not merely unconfigured, so the picker's catalog can never list it.
  // The footer action is the dashboard's only way in, which is what these cover.

  it("offers the EveryAPI connect action while no everyapi entry exists", async () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /providers\.add/ }));

    const drawer = await screen.findByTestId("drawer-slot");
    expect(
      within(drawer).getByText("providers.everyapi_connect_cta"),
    ).toBeInTheDocument();
  });

  it("hides the EveryAPI connect action once an everyapi entry is present", async () => {
    useProvidersMock.mockReturnValue({
      data: [
        ...PROVIDERS,
        {
          id: "everyapi",
          display_name: "EveryAPI",
          // Deliberately unconfigured: the action must key off the entry *existing*, not off it being usable.
          // An entry with a missing key is reachable through the normal configure flow, so re-offering "connect" would write over it.
          auth_status: "missing",
          reachable: false,
          key_required: true,
          base_url: "https://api.everyapi.ai",
        },
      ],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /providers\.add/ }));

    const drawer = await screen.findByTestId("drawer-slot");
    expect(
      within(drawer).queryByText("providers.everyapi_connect_cta"),
    ).not.toBeInTheDocument();
  });

  it("registers the gateway and stores the relay key on submit", async () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /providers\.add/ }));

    const picker = await screen.findByTestId("drawer-slot");
    fireEvent.click(
      within(picker).getByText("providers.everyapi_connect_cta"),
    );

    // The connect form replaces the picker in the shared drawer slot.
    const form = await screen.findByTestId("drawer-slot");
    const keyField = within(form).getByPlaceholderText("EVERYAPI_API_KEY");
    fireEvent.change(keyField, { target: { value: "  relay-abc  " } });
    fireEvent.click(
      within(form).getByRole("button", { name: /providers\.everyapi_connect_action/ }),
    );

    // The key is trimmed, and an omitted gateway URL is passed through as-is so the hook applies the documented default rather than the page inventing one.
    expect(connectEveryApiMutateAsync).toHaveBeenCalledTimes(1);
    expect(connectEveryApiMutateAsync).toHaveBeenCalledWith({
      relayKey: "relay-abc",
      baseUrl: "",
    });
  });

  it("filters configured providers by search term", () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    fireEvent.change(screen.getByPlaceholderText("common.search"), {
      target: { value: "anthr" },
    });

    expect(screen.getByText("Anthropic")).toBeInTheDocument();
    expect(screen.queryByText("OpenAI")).not.toBeInTheDocument();
  });

  it("shows a 'no results' empty state when search matches nothing", () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    fireEvent.change(screen.getByPlaceholderText("common.search"), {
      target: { value: "definitely-not-a-provider" },
    });

    expect(screen.getByText("providers.no_results")).toBeInTheDocument();
  });

  it("filters by reachability via the reachable chip", () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    // FilterChips renders a button per status; pick the "reachable" one.
    const reachableBtn = screen.getByRole("button", {
      name: /providers\.filter_reachable/,
    });
    fireEvent.click(reachableBtn);

    expect(screen.getByText("OpenAI")).toBeInTheDocument();
    // Anthropic is reachable: false — should be filtered out.
    expect(screen.queryByText("Anthropic")).not.toBeInTheDocument();
  });

  it("calls useTestProvider when the per-card Test action fires", async () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    // Find the OpenAI card by its display name, then click its Test button.
    const openaiCard = screen.getByText("OpenAI").closest("div");
    expect(openaiCard).toBeTruthy();
    // Search the whole document for any Test button — clicking the first
    // visible one is sufficient: it triggers the mutation regardless of
    // which card it belongs to.
    const testButtons = within(document.body).getAllByRole("button", {
      name: /providers\.test/,
    });
    expect(testButtons.length).toBeGreaterThan(0);
    fireEvent.click(testButtons[0]);

    // The handler is async — assert the mutation was kicked off.
    expect(testMutateAsync).toHaveBeenCalledTimes(1);
    expect(typeof testMutateAsync.mock.calls[0][0]).toBe("string");
  });

  it("keeps Save actionable after a passing Test in the configure drawer (#6144)", async () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    // Add → pick an unconfigured provider (Groq) to open the configure drawer, mirroring a user adding a fresh provider.
    fireEvent.click(screen.getByRole("button", { name: /providers\.add/ }));
    const picker = await screen.findByTestId("drawer-slot");
    fireEvent.click(within(picker).getByText("Groq"));

    // Configure drawer now owns the slot.
    // Enter an API key — Save is enabled at this point because the key input is dirty.
    let drawer = await screen.findByTestId("drawer-slot");
    fireEvent.change(
      within(drawer).getByPlaceholderText("providers.key_placeholder"),
      { target: { value: "sk-test-key" } },
    );
    expect(
      within(drawer).getByRole("button", { name: /common\.save/ }),
    ).not.toBeDisabled();

    // Click Test. `testKey` persists the key (which clears `keyInput`) then runs the test mutation, which resolves ok.
    fireEvent.click(
      within(drawer).getByRole("button", { name: /providers\.test/ }),
    );
    // Wait for the success banner so the test result has landed in state.
    drawer = await screen.findByTestId("drawer-slot");
    await within(drawer).findByText("providers.reachable");

    // The passing Test clears `keyInput`, making the form look "unchanged" — but the credential is already saved.
    // Save must stay actionable; otherwise the greyed-out button reads as "provider can't be added".
    expect(
      within(drawer).getByRole("button", { name: /common\.save/ }),
    ).not.toBeDisabled();
  });
});
