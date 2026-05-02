import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { PluginsPage } from "./PluginsPage";
import { PushDrawer } from "../components/ui/PushDrawer";
import { usePlugins, usePluginRegistries } from "../lib/queries/plugins";
import {
  useInstallPlugin,
  useUninstallPlugin,
  useScaffoldPlugin,
  useInstallPluginDeps,
} from "../lib/mutations/plugins";
import type { PluginItem, RegistryEntry } from "../api";

vi.mock("../lib/queries/plugins", () => ({
  usePlugins: vi.fn(),
  usePluginRegistries: vi.fn(),
}));

vi.mock("../lib/mutations/plugins", () => ({
  useInstallPlugin: vi.fn(),
  useUninstallPlugin: vi.fn(),
  useScaffoldPlugin: vi.fn(),
  useInstallPluginDeps: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) => {
        if (opts && typeof opts === "object" && "defaultValue" in opts) {
          return key;
        }
        return key;
      },
    }),
  };
});

const usePluginsMock = usePlugins as unknown as ReturnType<typeof vi.fn>;
const usePluginRegistriesMock = usePluginRegistries as unknown as ReturnType<typeof vi.fn>;
const useInstallPluginMock = useInstallPlugin as unknown as ReturnType<typeof vi.fn>;
const useUninstallPluginMock = useUninstallPlugin as unknown as ReturnType<typeof vi.fn>;
const useScaffoldPluginMock = useScaffoldPlugin as unknown as ReturnType<typeof vi.fn>;
const useInstallPluginDepsMock = useInstallPluginDeps as unknown as ReturnType<typeof vi.fn>;

interface QueryShape<T> {
  data: T;
  isLoading: boolean;
  isFetching: boolean;
  isError: boolean;
  isSuccess: boolean;
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
    isSuccess: true,
    refetch: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

function setMutationDefaults() {
  useInstallPluginMock.mockReturnValue({
    mutate: vi.fn(),
    isPending: false,
    error: null,
  });
  useUninstallPluginMock.mockReturnValue({
    mutate: vi.fn(),
    isPending: false,
  });
  useScaffoldPluginMock.mockReturnValue({
    mutate: vi.fn(),
    isPending: false,
  });
  useInstallPluginDepsMock.mockReturnValue({
    mutate: vi.fn(),
    isPending: false,
  });
}

function samplePlugins(): PluginItem[] {
  return [
    {
      name: "echo-memory",
      version: "0.1.0",
      description: "Tracks recent messages",
      author: "alice",
      hooks_valid: true,
      size_bytes: 4096,
      hooks: { ingest: true },
    },
    {
      name: "broken-plugin",
      version: "0.0.1",
      description: "",
      hooks_valid: false,
      size_bytes: 1024,
    },
  ];
}

function sampleRegistries(): RegistryEntry[] {
  return [
    {
      name: "official",
      github_repo: "librefang/registry",
      plugins: [
        {
          name: "weather",
          installed: false,
          version: "1.0.0",
          description: "Weather lookup",
          hooks: ["ingest"],
        },
        {
          name: "calendar",
          installed: true,
          version: "0.5.0",
          description: "Calendar tool",
          hooks: [],
        },
      ],
    },
  ];
}

function renderPage(): void {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <PluginsPage />
      <PushDrawer />
    </QueryClientProvider>,
  );
}

describe("PluginsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setMutationDefaults();
    // Default: registries empty so the auto-tab-switch when plugins.length===0
    // doesn't surface unexpected content. Tests that exercise registry behavior
    // override this.
    usePluginRegistriesMock.mockReturnValue(makeQuery({ registries: [] }));
  });

  it("renders the loading skeleton while the installed plugins list is loading", () => {
    usePluginsMock.mockReturnValue(
      makeQuery(undefined, { isLoading: true, isFetching: true, isSuccess: false }),
    );

    renderPage();

    expect(screen.getByText("plugins.title")).toBeInTheDocument();
    // ListSkeleton renders animated placeholders; assert the page is in the
    // installed tab and not showing the empty state yet.
    expect(screen.queryByText("plugins.no_plugins")).not.toBeInTheDocument();
  });

  it("renders the empty state when no plugins are installed", async () => {
    usePluginsMock.mockReturnValue(
      makeQuery<PluginItem[]>([], { isSuccess: true }),
    );

    renderPage();

    // With zero installed plugins the page auto-switches to the registry tab,
    // and the registry empty state appears.
    expect(await screen.findByText("plugins.no_registries")).toBeInTheDocument();
  });

  it("renders each installed plugin and its hooks/invalid badges", () => {
    usePluginsMock.mockReturnValue(makeQuery(samplePlugins()));

    renderPage();

    expect(screen.getByText("echo-memory")).toBeInTheDocument();
    expect(screen.getByText("broken-plugin")).toBeInTheDocument();
    expect(screen.getByText("Tracks recent messages")).toBeInTheDocument();
    // ingest hook badge from echo-memory
    expect(screen.getByText("ingest")).toBeInTheDocument();
    // hooks_valid=false plugin gets the invalid badge
    expect(screen.getByText("invalid")).toBeInTheDocument();
  });

  it("requires a confirm click before uninstalling a plugin", async () => {
    const user = userEvent.setup();
    const uninstallMutate = vi.fn();
    useUninstallPluginMock.mockReturnValue({
      mutate: uninstallMutate,
      isPending: false,
    });
    usePluginsMock.mockReturnValue(makeQuery(samplePlugins()));

    renderPage();

    // First click on the trash icon arms the confirm; it must NOT call mutate.
    const deleteButtons = screen.getAllByLabelText("common.delete");
    await user.click(deleteButtons[0]);
    expect(uninstallMutate).not.toHaveBeenCalled();

    // Now the row shows confirm/cancel; clicking confirm fires the mutation.
    const confirmBtn = screen.getByText("common.confirm");
    await user.click(confirmBtn);
    expect(uninstallMutate).toHaveBeenCalledTimes(1);
    expect(uninstallMutate).toHaveBeenCalledWith(
      "echo-memory",
      expect.any(Object),
    );
  });

  it("invokes the deps mutation when 'Install deps' is clicked on an installed plugin", async () => {
    const user = userEvent.setup();
    const depsMutate = vi.fn();
    useInstallPluginDepsMock.mockReturnValue({
      mutate: depsMutate,
      isPending: false,
    });
    usePluginsMock.mockReturnValue(makeQuery(samplePlugins()));

    renderPage();

    // The first row has a deps button labeled "plugins.deps".
    const depsButtons = screen.getAllByText("plugins.deps");
    await user.click(depsButtons[0]);

    expect(depsMutate).toHaveBeenCalledTimes(1);
    expect(depsMutate).toHaveBeenCalledWith(
      "echo-memory",
      expect.any(Object),
    );
  });

  it("switches to the registry tab and renders registry plugin cards", async () => {
    const user = userEvent.setup();
    usePluginsMock.mockReturnValue(makeQuery(samplePlugins()));
    usePluginRegistriesMock.mockReturnValue(
      makeQuery({ registries: sampleRegistries() }),
    );

    renderPage();

    // Click the registry tab.
    await user.click(screen.getByText("plugins.registry_tab"));

    expect(await screen.findByText("weather")).toBeInTheDocument();
    expect(screen.getByText("calendar")).toBeInTheDocument();
    expect(screen.getByText("Weather lookup")).toBeInTheDocument();
    // calendar is installed -> shows installed badge instead of an install button.
    expect(screen.getAllByText("plugins.installed").length).toBeGreaterThan(0);
  });

  it("installs a registry plugin from its card with the registry source payload", async () => {
    const user = userEvent.setup();
    const installMutate = vi.fn();
    useInstallPluginMock.mockReturnValue({
      mutate: installMutate,
      isPending: false,
      error: null,
    });
    usePluginsMock.mockReturnValue(makeQuery(samplePlugins()));
    usePluginRegistriesMock.mockReturnValue(
      makeQuery({ registries: sampleRegistries() }),
    );

    renderPage();
    await user.click(screen.getByText("plugins.registry_tab"));

    // The non-installed "weather" card has the install button; "calendar" is
    // already installed so it has no button. Find the install button on the
    // weather card by walking up from its name heading.
    const weatherHeading = await screen.findByText("weather");
    const card = weatherHeading.closest("div[class*='cursor-pointer']");
    expect(card).not.toBeNull();
    const installButton = within(card as HTMLElement).getByText(
      "plugins.install",
    );
    await user.click(installButton);

    expect(installMutate).toHaveBeenCalledTimes(1);
    const [payload] = installMutate.mock.calls[0];
    expect(payload).toEqual({
      source: "registry",
      name: "weather",
      github_repo: "librefang/registry",
    });
  });

  it("opens the scaffold drawer and disables Create when name is empty", async () => {
    const user = userEvent.setup();
    usePluginsMock.mockReturnValue(makeQuery(samplePlugins()));

    renderPage();

    await user.click(screen.getByText("plugins.new_plugin"));

    // PushDrawer renders the body in both the desktop push slot and the
    // mobile overlay sheet, so labels/buttons each appear twice.
    const nameInputs = await screen.findAllByLabelText("plugins.plugin_name");
    expect(nameInputs.length).toBeGreaterThan(0);
    const createBtn = screen.getAllByText("plugins.create")[0].closest("button");
    expect(createBtn).toBeDisabled();
  });

  it("submits the scaffold form with the entered name, description and runtime", async () => {
    const user = userEvent.setup();
    const scaffoldMutate = vi.fn();
    useScaffoldPluginMock.mockReturnValue({
      mutate: scaffoldMutate,
      isPending: false,
    });
    usePluginsMock.mockReturnValue(makeQuery(samplePlugins()));

    renderPage();

    await user.click(screen.getByText("plugins.new_plugin"));

    const nameInputs = await screen.findAllByLabelText("plugins.plugin_name");
    await user.type(nameInputs[0], "my-plugin");
    const descInputs = screen.getAllByLabelText("plugins.description");
    await user.type(descInputs[0], "scaffold from test");

    const createBtn = screen.getAllByText("plugins.create")[0].closest("button")!;
    expect(createBtn).not.toBeDisabled();
    await user.click(createBtn);

    expect(scaffoldMutate).toHaveBeenCalledTimes(1);
    const [payload] = scaffoldMutate.mock.calls[0];
    expect(payload).toEqual({
      name: "my-plugin",
      desc: "scaffold from test",
      runtime: "python",
    });
  });
});
