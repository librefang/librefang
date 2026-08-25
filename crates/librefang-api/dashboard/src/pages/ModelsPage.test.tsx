import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  ModelsPage,
  capabilityOverrideLabel,
  modelPricingKind,
  numberInputValue,
  settingsStateEqual,
  settingsStateFromOverrides,
} from "./ModelsPage";
import { useModels, useModelOverrides } from "../lib/queries/models";
import {
  useAddCustomModel,
  useRemoveCustomModel,
  useUpdateModelOverrides,
  useDeleteModelOverrides,
} from "../lib/mutations/models";
import type { ModelItem } from "../api";
import { useUIStore } from "../lib/store";

vi.mock("../lib/queries/models", () => ({
  useModels: vi.fn(),
  useModelOverrides: vi.fn(),
}));

vi.mock("../lib/mutations/models", () => ({
  useAddCustomModel: vi.fn(),
  useRemoveCustomModel: vi.fn(),
  useUpdateModelOverrides: vi.fn(),
  useDeleteModelOverrides: vi.fn(),
}));

// DrawerPanel pushes its children into a global slot via Zustand instead of
// rendering inline, so jsdom queries for form fields inside the drawer would
// miss them. Replace it with a passthrough that renders children when open.
vi.mock("../components/ui/DrawerPanel", () => ({
  DrawerPanel: ({ isOpen, title, children }: { isOpen: boolean; title?: string; children: React.ReactNode }) =>
    isOpen ? (
      <div data-testid="drawer-panel">
        {title && <div data-testid="drawer-title">{title}</div>}
        {children}
      </div>
    ) : null,
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({ t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue ?? key }),
  };
});

const useModelsMock = useModels as unknown as ReturnType<typeof vi.fn>;
const useModelOverridesMock = useModelOverrides as unknown as ReturnType<typeof vi.fn>;
const useAddCustomModelMock = useAddCustomModel as unknown as ReturnType<typeof vi.fn>;
const useRemoveCustomModelMock = useRemoveCustomModel as unknown as ReturnType<typeof vi.fn>;
const useUpdateModelOverridesMock = useUpdateModelOverrides as unknown as ReturnType<typeof vi.fn>;
const useDeleteModelOverridesMock = useDeleteModelOverrides as unknown as ReturnType<typeof vi.fn>;

interface QueryShape<T> {
  data: T;
  isLoading: boolean;
  isFetching: boolean;
  isError: boolean;
  refetch: ReturnType<typeof vi.fn>;
}

function makeQuery<T>(data: T, overrides: Partial<QueryShape<T>> = {}): QueryShape<T> {
  return {
    data,
    isLoading: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

interface MutShape {
  mutate: ReturnType<typeof vi.fn>;
  mutateAsync: ReturnType<typeof vi.fn>;
  isPending: boolean;
  error: Error | null;
}

function makeMut(overrides: Partial<MutShape> = {}): MutShape {
  return {
    mutate: vi.fn(),
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
    error: null,
    ...overrides,
  };
}

const sampleModels: ModelItem[] = [
  {
    id: "gpt-4o",
    display_name: "GPT-4o",
    provider: "openai",
    tier: "frontier",
    context_window: 128000,
    input_cost_per_m: 2.5,
    output_cost_per_m: 10,
    supports_tools: true,
    supports_vision: true,
    supports_streaming: true,
    available: true,
  },
  {
    id: "claude-haiku",
    display_name: "Claude Haiku",
    provider: "anthropic",
    tier: "fast",
    context_window: 200000,
    input_cost_per_m: 1,
    output_cost_per_m: 5,
    supports_tools: true,
    available: true,
  },
  {
    id: "my-custom",
    display_name: "My Custom Model",
    provider: "openai",
    tier: "custom",
    context_window: 32000,
    input_cost_per_m: 0,
    output_cost_per_m: 0,
    available: false,
  },
];

function setLoaded(models: ModelItem[] = sampleModels): void {
  useModelsMock.mockReturnValue(
    makeQuery({ models, total: models.length, available: models.filter(m => m.available).length }),
  );
}

function setMutationDefaults(): {
  add: MutShape;
  remove: MutShape;
  update: MutShape;
  del: MutShape;
} {
  const add = makeMut();
  const remove = makeMut();
  const update = makeMut();
  const del = makeMut();
  useAddCustomModelMock.mockReturnValue(add);
  useRemoveCustomModelMock.mockReturnValue(remove);
  useUpdateModelOverridesMock.mockReturnValue(update);
  useDeleteModelOverridesMock.mockReturnValue(del);
  return { add, remove, update, del };
}

function renderPage() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <ModelsPage />
    </QueryClientProvider>,
  );
}

describe("ModelsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Reset persisted Zustand state that affects filtering visibility.
    useUIStore.setState({
      hiddenModelKeys: [],
      modelsAvailableOnly: false,
      toasts: [],
    });
    useModelOverridesMock.mockReturnValue(makeQuery({}));
    setMutationDefaults();
  });

  it("shows the load-error banner when the models query errors", () => {
    useModelsMock.mockReturnValue(makeQuery(undefined, { isError: true }));
    renderPage();
    expect(screen.getByText("models.load_error")).toBeInTheDocument();
  });

  it("shows ListSkeleton placeholder while models query is loading", () => {
    useModelsMock.mockReturnValue(makeQuery(undefined, { isLoading: true }));
    renderPage();
    // No model cards; empty-state copy not rendered either.
    expect(screen.queryByText("models.no_models")).not.toBeInTheDocument();
    expect(screen.queryByText("GPT-4o")).not.toBeInTheDocument();
  });

  it("shows the empty-state when the catalog is empty", () => {
    setLoaded([]);
    renderPage();
    expect(screen.getByText("models.no_models")).toBeInTheDocument();
  });

  it("renders model cards grouped by provider", () => {
    setLoaded();
    renderPage();
    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
    expect(screen.getByText("Claude Haiku")).toBeInTheDocument();
    // Provider headers render.
    // Provider header text appears as a section header (and may also appear
    // in card subtitles like "openai/gpt-4o" — getAllByText keeps the test
    // tolerant to that).
    expect(screen.getAllByText("openai").length).toBeGreaterThan(0);
    expect(screen.getAllByText("anthropic").length).toBeGreaterThan(0);
  });

  it("filters by search query across id/display_name/provider", () => {
    setLoaded();
    renderPage();
    const search = screen.getByPlaceholderText("models.search_placeholder");
    fireEvent.change(search, { target: { value: "haiku" } });
    expect(screen.getByText("Claude Haiku")).toBeInTheDocument();
    expect(screen.queryByText("GPT-4o")).not.toBeInTheDocument();
  });

  it("filters by provider via the provider <select>", () => {
    setLoaded();
    renderPage();
    // First select after search input is providerFilter.
    const selects = screen.getAllByRole("combobox");
    fireEvent.change(selects[0], { target: { value: "anthropic" } });
    expect(screen.getByText("Claude Haiku")).toBeInTheDocument();
    expect(screen.queryByText("GPT-4o")).not.toBeInTheDocument();
  });

  it("hides custom-tier model when availableOnly toggle is on (it has available=false)", () => {
    setLoaded();
    renderPage();
    // Custom model is available=false, so it should be visible by default
    // (toggle off in beforeEach), then disappear after toggling.
    expect(screen.getByText("My Custom Model")).toBeInTheDocument();
    fireEvent.click(screen.getByTitle("models.available_only"));
    expect(screen.queryByText("My Custom Model")).not.toBeInTheDocument();
    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
  });

  it("renders Free badge only when both costs are explicitly 0 (custom model)", () => {
    setLoaded();
    renderPage();
    // Custom model has both costs = 0, so Free badge appears.
    expect(screen.getByText("models.free")).toBeInTheDocument();
  });

  it("does not label unknown OpenRouter pricing as free", () => {
    setLoaded([
      {
        id: "openrouter/acme/unknown",
        display_name: "Unknown pricing",
        provider: "openrouter",
        tier: "balanced",
        context_window: 32_768,
        input_cost_per_m: 0,
        output_cost_per_m: 0,
        pricing_known: false,
        available: true,
      },
    ]);
    setMutationDefaults();
    renderPage();

    expect(screen.queryByText("models.free")).toBeNull();
    expect(screen.getByText("—")).toBeInTheDocument();
  });

  it("treats missing costs as unknown pricing", () => {
    const model: ModelItem = {
      id: "missing-costs",
      display_name: "Missing costs",
      provider: "custom",
      context_window: 32_768,
      available: true,
    };
    setLoaded([model]);
    renderPage();

    expect(modelPricingKind(model)).toBe("unknown");
    expect(screen.getByText("—")).toBeInTheDocument();
    expect(screen.queryByText(/\$—/)).not.toBeInTheDocument();
  });

  it("opens the Add Custom Model drawer when the header button is clicked", () => {
    setLoaded();
    renderPage();
    fireEvent.click(screen.getByTitle("models.add_model (n)"));
    expect(screen.getByText("models.add_custom_model")).toBeInTheDocument();
    // Required form fields render.
    expect(screen.getByPlaceholderText("models.model_id_placeholder")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("models.provider_placeholder")).toBeInTheDocument();
  });

  it("submits the Add form and calls useAddCustomModel.mutateAsync with trimmed payload", async () => {
    setLoaded();
    const muts = setMutationDefaults();
    renderPage();
    fireEvent.click(screen.getByTitle("models.add_model (n)"));

    fireEvent.change(screen.getByPlaceholderText("models.model_id_placeholder"), {
      target: { value: "  new-model  " },
    });
    fireEvent.change(screen.getByPlaceholderText("models.provider_placeholder"), {
      target: { value: " custom-provider " },
    });

    // Submit — the drawer contains a <form>, find it and dispatch submit.
    const form = screen.getByPlaceholderText("models.model_id_placeholder").closest("form");
    expect(form).not.toBeNull();
    fireEvent.submit(form!);

    expect(muts.add.mutateAsync).toHaveBeenCalledTimes(1);
    const payload = muts.add.mutateAsync.mock.calls[0][0];
    expect(payload.id).toBe("new-model");
    expect(payload.provider).toBe("custom-provider");
  });

  it("keeps cleared numeric fields empty and omits them on submit", () => {
    setLoaded();
    const muts = setMutationDefaults();
    renderPage();
    fireEvent.click(screen.getByTitle("models.add_model (n)"));

    fireEvent.change(screen.getByPlaceholderText("models.model_id_placeholder"), {
      target: { value: "minimal-model" },
    });
    fireEvent.change(screen.getByPlaceholderText("models.provider_placeholder"), {
      target: { value: "custom" },
    });
    for (const label of [
      "models.context_window",
      "models.max_output",
      "models.input_cost",
      "models.output_cost",
    ]) {
      const input = screen.getByLabelText(label) as HTMLInputElement;
      fireEvent.change(input, { target: { value: "" } });
      expect(input.value).toBe("");
    }

    const form = screen
      .getByPlaceholderText("models.model_id_placeholder")
      .closest("form");
    fireEvent.submit(form!);
    const payload = muts.add.mutateAsync.mock.calls[0][0];
    expect(payload).not.toHaveProperty("context_window");
    expect(payload).not.toHaveProperty("max_output_tokens");
    expect(payload).not.toHaveProperty("input_cost_per_m");
    expect(payload).not.toHaveProperty("output_cost_per_m");
  });

  it("does not apply add-model completion effects after unmount", async () => {
    setLoaded();
    let resolveAdd: () => void = () => undefined;
    const mutateAsync = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolveAdd = resolve;
        }),
    );
    useAddCustomModelMock.mockReturnValue(makeMut({ mutateAsync }));
    const view = renderPage();
    fireEvent.click(screen.getByTitle("models.add_model (n)"));
    fireEvent.change(screen.getByPlaceholderText("models.model_id_placeholder"), {
      target: { value: "deferred-model" },
    });
    fireEvent.change(screen.getByPlaceholderText("models.provider_placeholder"), {
      target: { value: "custom" },
    });
    fireEvent.submit(
      screen.getByPlaceholderText("models.model_id_placeholder").closest("form")!,
    );

    view.unmount();
    resolveAdd();
    await act(async () => {
      await Promise.resolve();
    });
    expect(useUIStore.getState().toasts).toEqual([]);
  });

  it("resyncs pristine settings but preserves dirty settings", async () => {
    let overrides = { temperature: 0.2 };
    useModelOverridesMock.mockImplementation(() => makeQuery(overrides));
    setLoaded();
    renderPage();
    fireEvent.click(screen.getAllByTitle("models.settings_title")[0]);

    const temperatureRange = () =>
      screen
        .getAllByLabelText("models.temperature")
        .find(
          (element): element is HTMLInputElement =>
            element instanceof HTMLInputElement && element.type === "range",
        )!;
    await waitFor(() => expect(temperatureRange().value).toBe("0.2"));

    overrides = { temperature: 0.5 };
    fireEvent.change(screen.getByPlaceholderText("models.search_placeholder"), {
      target: { value: "g" },
    });
    await waitFor(() => expect(temperatureRange().value).toBe("0.5"));

    fireEvent.change(temperatureRange(), { target: { value: "0.9" } });
    overrides = { temperature: 0.7 };
    fireEvent.change(screen.getByPlaceholderText("models.search_placeholder"), {
      target: { value: "gp" },
    });
    await waitFor(() => expect(temperatureRange().value).toBe("0.9"));
  });

  it("does not apply settings-save effects after the drawer unmounts", async () => {
    let resolveSave: () => void = () => undefined;
    const mutateAsync = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolveSave = resolve;
        }),
    );
    useUpdateModelOverridesMock.mockReturnValue(makeMut({ mutateAsync }));
    setLoaded();
    renderPage();
    fireEvent.click(screen.getAllByTitle("models.settings_title")[0]);
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));
    fireEvent.click(screen.getByRole("button", { name: "common.cancel" }));

    resolveSave();
    await act(async () => {
      await Promise.resolve();
    });
    expect(useUIStore.getState().toasts).toEqual([]);
  });

  it("requires double-click to delete a custom model (confirm-then-delete)", () => {
    setLoaded();
    const muts = setMutationDefaults();
    renderPage();

    // Find the delete button for the custom model. It only appears on the
    // custom card; query by its title on either button.
    const deleteBtn = screen.getByTitle("models.delete_model");
    fireEvent.click(deleteBtn);
    // First click is just confirmation arming — should NOT call mutateAsync.
    expect(muts.remove.mutateAsync).not.toHaveBeenCalled();

    fireEvent.click(deleteBtn);
    expect(muts.remove.mutateAsync).toHaveBeenCalledTimes(1);
    expect(muts.remove.mutateAsync).toHaveBeenCalledWith("my-custom");
  });

  it("does not render a delete button for a cli_config-sourced model", () => {
    // A live-detected CLI model is tier "custom" but source "cli_config": it has
    // no persisted custom entry, so a delete would 404. The control must be hidden.
    setLoaded([
      {
        id: "codex-cli/deepseek-chat",
        display_name: "deepseek-chat (Codex CLI)",
        provider: "codex-cli",
        tier: "custom",
        source: "cli_config",
        context_window: 0,
        input_cost_per_m: 0,
        output_cost_per_m: 0,
        available: true,
      },
    ]);
    setMutationDefaults();
    renderPage();
    expect(screen.queryByTitle("models.delete_model")).toBeNull();
  });

  it("invokes refetch when the header refresh button fires", () => {
    const refetch = vi.fn().mockResolvedValue(undefined);
    useModelsMock.mockReturnValue(
      makeQuery(
        { models: sampleModels, total: 3, available: 2 },
        { refetch },
      ),
    );
    renderPage();
    fireEvent.click(screen.getByLabelText("common.refresh"));
    expect(refetch).toHaveBeenCalledTimes(1);
  });

  it("toggles a model to hidden via the per-card hide button and updates the hidden filter chip", () => {
    setLoaded();
    renderPage();
    // The hide button is hover-revealed but still in the DOM.
    const hideBtns = screen.getAllByTitle("models.hide_model");
    expect(hideBtns.length).toBeGreaterThan(0);
    fireEvent.click(hideBtns[0]);
    // After hiding, the persisted store should record the key, and the
    // hidden-toggle chip with count appears.
    expect(useUIStore.getState().hiddenModelKeys.length).toBe(1);
    expect(screen.getByTitle("models.show_hidden")).toBeInTheDocument();
  });
});

describe("ModelsPage helpers", () => {
  it("parses empty number inputs without coercing them to zero", () => {
    expect(numberInputValue("")).toBe("");
    expect(numberInputValue("0")).toBe(0);
    expect(numberInputValue("12.5")).toBe(12.5);
  });

  it("builds complete override state and compares it by value", () => {
    const first = settingsStateFromOverrides({ temperature: 0.4 });
    const equivalent = settingsStateFromOverrides({ temperature: 0.4 });
    const changed = settingsStateFromOverrides({ temperature: 0.8 });
    expect(first.tempEnabled).toBe(true);
    expect(settingsStateEqual(first, equivalent)).toBe(true);
    expect(settingsStateEqual(first, changed)).toBe(false);
  });

  it("formats capability override labels with explicit branches", () => {
    const labels = {
      auto: "Auto",
      on: "On",
      off: "Off",
      forceOn: "Force on",
      forceOff: "Force off",
    };
    expect(capabilityOverrideLabel("default", true, labels)).toBe("Auto (On)");
    expect(capabilityOverrideLabel("default", false, labels)).toBe("Auto (Off)");
    expect(capabilityOverrideLabel("on", false, labels)).toBe("Force on");
    expect(capabilityOverrideLabel("off", true, labels)).toBe("Force off");
  });
});
