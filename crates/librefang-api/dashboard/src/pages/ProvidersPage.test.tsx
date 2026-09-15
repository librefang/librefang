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
import { useModels, useModelOverrides } from "../lib/queries/models";
import { useUpdateModelOverrides } from "../lib/mutations/models";
import {
  useTestProvider,
  useSetProviderKey,
  useDeleteProviderKey,
  useEnableProvider,
  useSetProviderUrl,
  useSetProviderDiscovery,
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
  // ProviderModelLimitsSection (#6209, #7774) calls this; default to no
  // override so the existing tests don't have to care about the limit editors.
  useModelOverrides: vi.fn(() => ({ data: undefined, isLoading: false })),
}));

vi.mock("../lib/mutations/models", () => ({
  useUpdateModelOverrides: vi.fn(),
}));

vi.mock("../lib/mutations/providers", () => ({
  useTestProvider: vi.fn(),
  useSetProviderKey: vi.fn(),
  useDeleteProviderKey: vi.fn(),
  useEnableProvider: vi.fn(),
  useSetProviderUrl: vi.fn(),
  useSetProviderDiscovery: vi.fn(),
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
const useModelOverridesMock = useModelOverrides as unknown as ReturnType<
  typeof vi.fn
>;
const useUpdateModelOverridesMock =
  useUpdateModelOverrides as unknown as ReturnType<typeof vi.fn>;
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
const useSetProviderDiscoveryMock =
  useSetProviderDiscovery as unknown as ReturnType<typeof vi.fn>;
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
  let enableProviderMutateAsync: ReturnType<typeof vi.fn>;
  let connectEveryApiMutateAsync: ReturnType<typeof vi.fn>;
  let setDiscoveryMutateAsync: ReturnType<typeof vi.fn>;
  let updateOverridesMutateAsync: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    // Drawer state is a global zustand store — reset between tests so a
    // drawer left open by one test doesn't bleed into the next.
    useDrawerStore.setState({ isOpen: false, content: null });
    testMutateAsync = vi.fn().mockResolvedValue({ status: "ok" });
    connectEveryApiMutateAsync = vi.fn().mockResolvedValue(undefined);
    setDiscoveryMutateAsync = vi.fn().mockResolvedValue(undefined);
    updateOverridesMutateAsync = vi.fn().mockResolvedValue({});

    useProviderStatusMock.mockReturnValue({
      data: { default_provider: "openai" },
      isFetching: false,
    });
    useModelsMock.mockReturnValue({ data: { models: [] }, isLoading: false });
    useModelOverridesMock.mockReturnValue({ data: undefined, isLoading: false });
    useUpdateModelOverridesMock.mockReturnValue({
      mutateAsync: updateOverridesMutateAsync,
      isPending: false,
    });

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
    enableProviderMutateAsync = vi.fn().mockResolvedValue(undefined);
    useEnableProviderMock.mockReturnValue(stubMutation(enableProviderMutateAsync));
    useSetProviderUrlMock.mockReturnValue(
      stubMutation(vi.fn().mockResolvedValue(undefined)),
    );
    useSetProviderDiscoveryMock.mockReturnValue(
      stubMutation(setDiscoveryMutateAsync),
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

  it("keeps provider actions outside implicit card button semantics", () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    const provider = screen.getByRole("group", { name: "OpenAI" });
    expect(provider).not.toHaveAttribute("tabindex");
    expect(
      within(provider).getByRole("button", {
        name: "providers.details: OpenAI",
      }),
    ).toBeInTheDocument();
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

  // ── A rejected key must not make the provider disappear ───────────────
  //
  // Saving a key the endpoint answers 401/403 to lands the entry on
  // `AuthStatus::InvalidKey`, which `is_available()` reports as false. While
  // the page partitioned on availability, that single save removed the
  // provider from the grid outright: the operator saw "Authentication failed
  // (HTTP 401/403)" and then an empty space where the provider had been, with
  // the only way back buried in the Add picker.

  const REJECTED: ProviderItem = {
    id: "deepseek",
    display_name: "DeepSeek",
    auth_status: "invalid_key",
    reachable: false,
    model_count: 0,
    key_required: true,
    base_url: "https://api.deepseek.com",
  };

  it("keeps a provider on the page after its key is rejected", async () => {
    useProvidersMock.mockReturnValue({
      data: [...PROVIDERS, REJECTED],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    // On the page, flagged as broken — not vanished.
    expect(screen.getByText("DeepSeek")).toBeInTheDocument();
    const card = screen.getByRole("group", { name: "DeepSeek" });
    expect(within(card).getByText("providers.key_rejected")).toBeInTheDocument();

    // And it must not double as an "unconfigured" entry in the Add picker.
    fireEvent.click(screen.getByRole("button", { name: /providers\.add/ }));
    const drawer = await screen.findByTestId("drawer-slot");
    expect(within(drawer).queryByText("DeepSeek")).not.toBeInTheDocument();
  });

  // ── Suppressed providers stay out of sight until asked for ────────────
  //
  // Suppression is the operator saying "I do not use this one". The daemon
  // keeps returning the entry — the registry recreates every built-in TOML on
  // boot by design — so the picker is the only place the choice can be
  // honoured, and the toggle is how it is reversed.

  const SUPPRESSED: ProviderItem = {
    id: "vertex-ai",
    display_name: "Vertex AI",
    auth_status: "missing",
    reachable: false,
    model_count: 0,
    key_required: true,
    suppressed: true,
  };

  // A local provider whose service went down reaches `local_offline`, which
  // `is_available()` also reports as false — the same vanish, triggered by a
  // restart of the box rather than by a bad key.
  it("keeps a local provider on the page while its service is down", () => {
    useProvidersMock.mockReturnValue({
      data: [...PROVIDERS, {
        id: "ollama",
        display_name: "Ollama",
        auth_status: "local_offline",
        reachable: false,
        model_count: 16,
        key_required: false,
        base_url: "http://127.0.0.1:11434",
      } satisfies ProviderItem],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    const card = screen.getByRole("group", { name: "Ollama" });
    expect(within(card).getByText("providers.offline")).toBeInTheDocument();
  });

  // The one measured on a live daemon: `vertex-ai` sat at `suppressed: true`
  // and an unrelated restart promoted it from `missing` to `configured` off a
  // stray credential env var, which put a provider the operator had removed
  // back on the page. Suppression has to cut across auth status.
  it("keeps a suppressed provider off the page even once it looks configured", async () => {
    const suppressedButConfigured: ProviderItem = {
      ...SUPPRESSED,
      auth_status: "configured",
    };
    useProvidersMock.mockReturnValue({
      data: [...PROVIDERS, suppressedButConfigured],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    expect(screen.queryByText("Vertex AI")).not.toBeInTheDocument();

    // And it is still recoverable through the same one toggle.
    fireEvent.click(screen.getByRole("button", { name: /providers\.add/ }));
    const drawer = await screen.findByTestId("drawer-slot");
    expect(within(drawer).queryByText("Vertex AI")).not.toBeInTheDocument();
    fireEvent.click(within(drawer).getByLabelText("providers.show_suppressed"));
    expect(within(drawer).getByText("Vertex AI")).toBeInTheDocument();
  });

  it("offers Re-enable for a suppressed provider that now reads as configured", async () => {
    const suppressedButConfigured: ProviderItem = { ...SUPPRESSED, auth_status: "configured" };
    useProvidersMock.mockReturnValue({
      data: [...PROVIDERS, suppressedButConfigured],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /providers\.add/ }));
    const drawer = await screen.findByTestId("drawer-slot");
    fireEvent.click(within(drawer).getByLabelText("providers.show_suppressed"));

    // The Re-enable branch keys on `suppressed`, not on auth status, so it
    // must still fire for an entry `detect_auth` has promoted.
    fireEvent.click(within(drawer).getByText("Vertex AI"));
    expect(enableProviderMutateAsync).toHaveBeenCalledWith("vertex-ai");
  });

  // Suppressing the only provider used to leave the page saying "No providers
  // configured yet", which reads as config loss to whoever just removed one.
  it("says providers are hidden, not absent, when suppression empties the page", () => {
    useProvidersMock.mockReturnValue({
      data: [{ ...SUPPRESSED, auth_status: "not_required", id: "ollama", display_name: "Ollama" }],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    expect(screen.getByText("providers.empty_all_suppressed_title")).toBeInTheDocument();
    expect(screen.queryByText("providers.empty_title")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "providers.empty_all_suppressed_cta" })).toBeInTheDocument();
  });

  it("excludes suppressed providers from the configured count", () => {
    useProvidersMock.mockReturnValue({
      // 2 configured + 1 suppressed-but-configured + 1 unconfigured = 4 total.
      data: [...PROVIDERS, { ...SUPPRESSED, auth_status: "configured" }],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    // The pill interpolates across text nodes, so match on the element's own
    // normalised text rather than a string literal.
    expect(
      screen.getByText((_, el) =>
        el?.textContent?.replace(/\s+/g, " ").trim() === "2 / 4 providers.configured"),
    ).toBeInTheDocument();
  });

  it("keeps suppressed providers out of the Add picker until asked for", async () => {
    useProvidersMock.mockReturnValue({
      data: [...PROVIDERS, SUPPRESSED],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /providers\.add/ }));
    const drawer = await screen.findByTestId("drawer-slot");

    // Unconfigured-but-not-suppressed still shows; the suppressed one does not.
    expect(within(drawer).getByText("Groq")).toBeInTheDocument();
    expect(within(drawer).queryByText("Vertex AI")).not.toBeInTheDocument();

    // The way back: one checkbox, no config file editing.
    fireEvent.click(within(drawer).getByLabelText("providers.show_suppressed"));
    expect(within(drawer).getByText("Vertex AI")).toBeInTheDocument();
  });

  // The toggle is the only route back to a suppressed provider, so it has to
  // appear whenever one exists — and the empty state must not simultaneously
  // claim there is nothing left to add.
  it("explains an Add picker emptied by suppression rather than calling it complete", async () => {
    useProvidersMock.mockReturnValue({
      // Every unconfigured provider is suppressed: Groq dropped, Vertex hidden.
      data: [PROVIDERS[0], PROVIDERS[1], SUPPRESSED],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /providers\.add/ }));
    const drawer = await screen.findByTestId("drawer-slot");

    expect(within(drawer).getByText("providers.all_suppressed")).toBeInTheDocument();
    expect(within(drawer).queryByText("providers.all_configured")).not.toBeInTheDocument();
    expect(within(drawer).getByLabelText("providers.show_suppressed")).toBeInTheDocument();
  });

  // The count drives the toggle's label, so it is scoped by the same search
  // term as the list it promises to reveal.
  it("does not offer to reveal suppressed providers the search already excludes", async () => {
    useProvidersMock.mockReturnValue({
      data: [...PROVIDERS, SUPPRESSED],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /providers\.add/ }));
    const drawer = await screen.findByTestId("drawer-slot");

    expect(within(drawer).getByLabelText("providers.show_suppressed")).toBeInTheDocument();
    fireEvent.change(within(drawer).getByPlaceholderText("common.search"), {
      target: { value: "groq" },
    });
    expect(within(drawer).queryByLabelText("providers.show_suppressed")).not.toBeInTheDocument();
    expect(within(drawer).getByText("Groq")).toBeInTheDocument();
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
  // ── Local providers behind auth (#6703) + model discovery (#6702) ──

  const VLLM: ProviderItem = {
    id: "vllm",
    display_name: "vLLM",
    auth_status: "not_required",
    reachable: true,
    model_count: 1,
    key_required: false,
    key_present: false,
    is_local: true,
    base_url: "http://gpu-box:8000/v1",
    api_key_env: "VLLM_API_KEY",
  };

  function openConfigureDrawer(provider: ProviderItem) {
    useProvidersMock.mockReturnValue({
      data: [provider],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });
    renderPage();
    fireEvent.click(screen.getAllByRole("button", { name: /common\.edit/ })[0]);
    return screen.findByTestId("drawer-slot");
  }

  it("offers the API key field for a key_required=false provider (#6703)", async () => {
    // The regression: the field was gated on `key_required !== false`, so a
    // self-hosted vLLM behind auth had no way to receive a key from the UI even
    // though the runtime sends whatever key is stored as a Bearer token.
    const drawer = await openConfigureDrawer(VLLM);

    const keyInput = within(drawer).getByPlaceholderText(
      "providers.key_placeholder",
    );
    expect(keyInput).toBeInTheDocument();
    // Labelled as optional, because the provider genuinely does not require one.
    expect(
      within(drawer).getByText("providers.api_key_optional_hint"),
    ).toBeInTheDocument();
  });

  it("treats key_present as a stored key for a keyless provider (#6703)", async () => {
    // `auth_status` is `not_required` whether or not a key is set, so without
    // `key_present` the drawer offered no way to replace or remove one.
    const drawer = await openConfigureDrawer({ ...VLLM, key_present: true });

    expect(
      within(drawer).getByPlaceholderText("providers.key_placeholder_existing"),
    ).toBeInTheDocument();
    expect(
      within(drawer).getByRole("button", { name: /providers\.remove_key/ }),
    ).toBeInTheDocument();
  });

  it("keeps the key field away from CLI passthrough providers (#6703)", async () => {
    // The counterweight to the un-gating above: claude-code & friends also
    // declare `key_required: false`, but they spawn a subprocess and carry no
    // base URL. Showing a key field there would plant a meaningless
    // CLAUDE_CODE_API_KEY in secrets.env, and there is no endpoint to send it to.
    const drawer = await openConfigureDrawer({
      id: "claude-code",
      display_name: "Claude Code",
      auth_status: "configured_cli",
      model_count: 2,
      key_required: false,
      base_url: "",
      api_key_env: "",
    });

    expect(
      within(drawer).queryByPlaceholderText("providers.key_placeholder"),
    ).toBeNull();
    expect(
      within(drawer).queryByText("providers.api_key_optional_hint"),
    ).toBeNull();
    // No endpoint to poll either, so the discovery control stays away too.
    expect(
      within(drawer).queryByRole("switch", {
        name: "providers.discover_models_label",
      }),
    ).toBeNull();
  });

  it("pins discovery on for built-in local providers (#6702)", async () => {
    const drawer = await openConfigureDrawer(VLLM);

    const toggle = within(drawer).getByRole("switch", {
      name: "providers.discover_models_label",
    });
    expect(toggle).toBeChecked();
    expect(toggle).toBeDisabled();
    expect(
      within(drawer).getByText("providers.discover_models_hint_builtin"),
    ).toBeInTheDocument();
  });

  it("lets a custom provider opt into model discovery (#6702)", async () => {
    const drawer = await openConfigureDrawer({
      id: "acme-vllm",
      display_name: "ACME vLLM",
      auth_status: "configured",
      model_count: 0,
      key_required: true,
      is_custom: true,
      discover_models: false,
      base_url: "http://gpu-box:4000/v1",
      api_key_env: "ACME_VLLM_API_KEY",
    });

    const toggle = within(drawer).getByRole("switch", {
      name: "providers.discover_models_label",
    });
    expect(toggle).not.toBeChecked();
    expect(toggle).not.toBeDisabled();

    fireEvent.click(toggle);
    expect(setDiscoveryMutateAsync).toHaveBeenCalledWith({
      id: "acme-vllm",
      discoverModels: true,
    });
  });
  // ── Model capacity limits (#7774) ──
  //
  // The context window used to be settable only in the creation wizard and was
  // overwritten by the next registry sync. It is now an entry in
  // `model_overrides.json`, so it must be editable here, seeded from the value
  // in force, and reverted against `limits_catalog` rather than against the
  // row's own (already effective) `context_window`.

  const LITELLM: ProviderItem = {
    id: "litellm",
    display_name: "LiteLLM",
    auth_status: "configured",
    model_count: 1,
    key_required: true,
    base_url: "http://gateway:4000/v1",
    api_key_env: "LITELLM_API_KEY",
  };

  /** One gateway model whose window discovery guessed at 131072. */
  function seedDiscoveredModel(overrides?: {
    context_window?: number;
    limitsCatalogWindow?: number;
  }): void {
    useModelsMock.mockReturnValue({
      data: {
        models: [
          {
            id: "sensor-model-generic-high",
            display_name: "sensor-model-generic-high",
            provider: "litellm",
            context_window: overrides?.context_window ?? 131072,
            max_output_tokens: 16384,
            limits_catalog: {
              context_window: overrides?.limitsCatalogWindow ?? 131072,
              max_output_tokens: 16384,
            },
          },
        ],
      },
      isLoading: false,
    });
  }

  it("saves a corrected context window as a model override (#7774)", async () => {
    seedDiscoveredModel();
    const drawer = await openConfigureDrawer(LITELLM);

    // The shared `ModelParamField`: a rung set, seeded from the value in force
    // so the selected rung is the real number rather than an empty box.
    const ladder = within(drawer).getByRole("group", { name: "providers.context_window" });
    expect(within(ladder).getByRole("button", { name: "128K" }))
      .toHaveAttribute("aria-pressed", "true");

    fireEvent.click(within(ladder).getByRole("button", { name: "32K" }));
    fireEvent.click(
      within(drawer).getByRole("button", {
        name: /providers\.context_window/,
      }),
    );

    expect(updateOverridesMutateAsync).toHaveBeenCalledWith({
      modelKey: "litellm:sensor-model-generic-high",
      overrides: { context_window: 32768 },
    });
  });

  it("keeps an active context-window override from deleting itself (#7774)", async () => {
    // The row's `context_window` is the *effective* value, so it equals the
    // override. Reverting against it instead of `limits_catalog` would make the
    // seeded field show a number the override no longer distinguishes.
    //
    // What this guards today is the seeding and the hint, not the write: the save
    // path no longer reads the revert target at all (see `modelOverrideDraft.ts`),
    // so the class of defect the assertion was written for is no longer
    // expressible here. Kept because those two halves still discriminate.
    seedDiscoveredModel({ context_window: 16384, limitsCatalogWindow: 131072 });
    useModelOverridesMock.mockReturnValue({
      data: { context_window: 16384 },
      isLoading: false,
    });
    const drawer = await openConfigureDrawer(LITELLM);

    // 16384 is not a context-window rung — the ladder starts 8K, 32K, 128K —
    // so an override of that size lives in the custom field, seeded with it.
    const ladder = within(drawer).getByRole("group", { name: "providers.context_window" });
    expect(within(ladder).getByRole("button", { name: "model_param.custom" }))
      .toHaveAttribute("aria-pressed", "true");
    expect(
      within(drawer).getByLabelText("providers.context_window — model_param.custom"),
    ).toHaveValue(16384);
    // Untouched field → nothing to save.
    expect(
      within(drawer).getByRole("button", { name: /providers\.context_window/ }),
    ).toBeDisabled();
    // The hint names the catalog value as the revert target, not the override.
    expect(
      within(drawer).getByText("providers.context_window_hint_override"),
    ).toBeInTheDocument();
  });

  it("leaves Save disabled while the max_tokens field is untouched", async () => {
    // The other half of the rule, and the one that catches an over-correction.
    //
    // The field seeds from a *display* value — with no override stored that is
    // the catalog figure, 16384 here — while the state it would be compared
    // against is "no override". Read as a difference, that makes simply opening
    // the drawer dirty, and one click writes `max_tokens: 16384` for every agent
    // that never set one, moving its request from the kernel default of 4096 to
    // 16384. An untouched field has nothing to save.
    seedDiscoveredModel();
    const drawer = await openConfigureDrawer(LITELLM);

    expect(updateOverridesMutateAsync).not.toHaveBeenCalled();
    expect(
      within(drawer).getByRole("button", { name: /providers\.max_tokens/ }),
    ).toBeDisabled();
  });

  it("saves a max_tokens that equals the model's catalog output capacity", async () => {
    // The reported defect. The catalog reports `max_output_tokens: 16384`, and
    // the old rule read a typed 16384 as "same as the default, so clear it".
    // It is not the default: an absent override falls through to
    // `DEFAULT_MODEL_MAX_TOKENS` (4096), so the number meant something else.
    //
    // Measured against the old rule, this fails with `Number of calls: 0` —
    // the field resolved to "not dirty", so Save never enabled and the value
    // could not be stored at all. The operator saw a box reading 16384 that
    // silently refused to save 16384.
    seedDiscoveredModel();
    const drawer = await openConfigureDrawer(LITELLM);

    const ladder = within(drawer).getByRole("group", { name: "providers.max_tokens" });
    fireEvent.click(within(ladder).getByRole("button", { name: "model_param.custom" }));
    fireEvent.change(
      within(drawer).getByLabelText("providers.max_tokens — model_param.custom"),
      { target: { value: "16384" } },
    );
    fireEvent.click(
      within(drawer).getByRole("button", {
        name: /providers\.max_tokens/,
      }),
    );

    expect(updateOverridesMutateAsync).toHaveBeenCalledWith({
      modelKey: "litellm:sensor-model-generic-high",
      overrides: { max_tokens: 16384 },
    });
  });

  it("clearing the field drops the context_window override (#7774)", async () => {
    seedDiscoveredModel({ context_window: 16384, limitsCatalogWindow: 131072 });
    useModelOverridesMock.mockReturnValue({
      data: { context_window: 16384, temperature: 0.3 },
      isLoading: false,
    });
    const drawer = await openConfigureDrawer(LITELLM);

    // Inherit is the rung that means "no override here" — the same thing the
    // cleared box used to mean.
    fireEvent.click(
      within(
        within(drawer).getByRole("group", { name: "providers.context_window" }),
      ).getByRole("button", { name: "model_param.inherit" }),
    );
    fireEvent.click(
      within(drawer).getByRole("button", {
        name: /providers\.context_window/,
      }),
    );

    // Only the limit is dropped — the unrelated inference parameter survives,
    // because PUT replaces the whole document.
    expect(updateOverridesMutateAsync).toHaveBeenCalledWith({
      modelKey: "litellm:sensor-model-generic-high",
      overrides: { temperature: 0.3 },
    });
  });

  it("says so in the interface when no context window is known (#7774)", async () => {
    // The runtime already logs "falling back to a conservative context window";
    // the operator has to be able to see it next to the model.
    seedDiscoveredModel({ context_window: 0, limitsCatalogWindow: 0 });
    const drawer = await openConfigureDrawer(LITELLM);
    expect(
      within(drawer).getByText("providers.context_window_unknown"),
    ).toBeInTheDocument();
    expect(
      within(
        within(drawer).getByRole("group", { name: "providers.context_window" }),
      ).getByRole("button", { name: "model_param.inherit" }),
    ).toHaveAttribute("aria-pressed", "true");
  });
});
